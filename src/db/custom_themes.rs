//! `custom_themes` table: metadata for administrator-uploaded stylesheets.
//!
//! The CSS itself lives in `URLET_THEMES_DIR`; this table keeps the name, the
//! cache-busting content hash and the file size. A matching row in `themes`
//! makes the upload participate in the normal enabled/default catalogue, so a
//! custom theme behaves exactly like a compiled one once it is registered.

use crate::db::Pool;
use crate::services::theme::ThemeAppearance;
use crate::services::time::now_iso;

type ThemeTuple = (
    i64,
    String,
    String,
    String,
    String,
    String,
    i64,
    String,
    Option<i64>,
    String,
    String,
);

/// One row of the `custom_themes` table.
#[derive(Debug, Clone)]
pub struct CustomThemeRow {
    pub id: i64,
    pub key: String,
    pub name: String,
    pub description: String,
    pub appearance: ThemeAppearance,
    /// Content hash of the current stylesheet, used in its URL.
    pub css_version: String,
    pub css_size: i64,
    pub original_filename: String,
    pub created_by: Option<i64>,
    pub created_at: String,
    pub updated_at: String,
}

/// Fields required to register a new upload.
#[derive(Debug, Clone)]
pub struct NewCustomTheme {
    pub key: String,
    pub name: String,
    pub description: String,
    pub appearance: ThemeAppearance,
    pub css_version: String,
    pub css_size: i64,
    pub original_filename: String,
    pub created_by: Option<i64>,
}

/// Every uploaded theme, oldest first.
pub async fn all(pool: &Pool) -> Result<Vec<CustomThemeRow>, sqlx::Error> {
    let rows: Vec<ThemeTuple> = sqlx::query_as(
        "SELECT id, key, name, description, appearance, css_version, css_size,
                original_filename, created_by, created_at, updated_at
           FROM custom_themes
          ORDER BY id",
    )
    .fetch_all(pool)
    .await?;
    Ok(rows.into_iter().map(row_from).collect())
}

/// One upload by primary key.
pub async fn find(pool: &Pool, id: i64) -> Result<Option<CustomThemeRow>, sqlx::Error> {
    let row: Option<ThemeTuple> = sqlx::query_as(
        "SELECT id, key, name, description, appearance, css_version, css_size,
                original_filename, created_by, created_at, updated_at
           FROM custom_themes
          WHERE id = $1",
    )
    .bind(id)
    .fetch_optional(pool)
    .await?;
    Ok(row.map(row_from))
}

/// One upload by stable catalogue key.
pub async fn find_by_key(pool: &Pool, key: &str) -> Result<Option<CustomThemeRow>, sqlx::Error> {
    let row: Option<ThemeTuple> = sqlx::query_as(
        "SELECT id, key, name, description, appearance, css_version, css_size,
                original_filename, created_by, created_at, updated_at
           FROM custom_themes
          WHERE key = $1",
    )
    .bind(key)
    .fetch_optional(pool)
    .await?;
    Ok(row.map(row_from))
}

/// Number of uploads, used for the catalogue limit.
pub async fn count(pool: &Pool) -> Result<i64, sqlx::Error> {
    sqlx::query_scalar("SELECT COUNT(*) FROM custom_themes")
        .fetch_one(pool)
        .await
}

/// Stable keys of every upload, used by the startup reconciliation.
pub async fn keys(pool: &Pool) -> Result<Vec<String>, sqlx::Error> {
    sqlx::query_scalar("SELECT key FROM custom_themes ORDER BY id")
        .fetch_all(pool)
        .await
}

/// Register an upload and its `themes` catalogue row in one transaction.
pub async fn create(pool: &Pool, new: &NewCustomTheme) -> Result<i64, sqlx::Error> {
    let now = now_iso();
    let mut tx = crate::db::begin_write(pool).await?;

    sqlx::query(
        "INSERT INTO themes
             (key, is_enabled, is_default, sort_order, created_at, updated_at)
         VALUES ($1, 1, 0,
                 (SELECT COALESCE(MAX(sort_order), 0) + 10 FROM themes), $2, $3)",
    )
    .bind(&new.key)
    .bind(&now)
    .bind(&now)
    .execute(&mut *tx)
    .await?;

    let id = sqlx::query_scalar::<_, i64>(
        "INSERT INTO custom_themes
             (key, name, description, appearance, css_version, css_size,
              original_filename, created_by, created_at, updated_at)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10) RETURNING id",
    )
    .bind(&new.key)
    .bind(&new.name)
    .bind(&new.description)
    .bind(new.appearance.as_str())
    .bind(&new.css_version)
    .bind(new.css_size)
    .bind(&new.original_filename)
    .bind(new.created_by)
    .bind(&now)
    .bind(&now)
    .fetch_one(&mut *tx)
    .await?;

    tx.commit().await?;
    Ok(id)
}

/// Update the editable metadata and, when supplied, the replaced stylesheet in
/// one transaction.
///
/// The two halves have to land together: a row whose name changed but whose
/// `css_version` still names the old file would serve the previous stylesheet
/// under the new label until the next restart.
pub async fn update(
    pool: &Pool,
    id: i64,
    name: &str,
    description: &str,
    appearance: ThemeAppearance,
    replaced_css: Option<(&str, i64, &str)>,
) -> Result<(), sqlx::Error> {
    let mut tx = crate::db::begin_write(pool).await?;
    sqlx::query(
        "UPDATE custom_themes
            SET name = $1, description = $2, appearance = $3, updated_at = $4
          WHERE id = $5",
    )
    .bind(name)
    .bind(description)
    .bind(appearance.as_str())
    .bind(now_iso())
    .bind(id)
    .execute(&mut *tx)
    .await?;
    if let Some((css_version, css_size, original_filename)) = replaced_css {
        sqlx::query(
            "UPDATE custom_themes
                SET css_version = $1, css_size = $2, original_filename = $3, updated_at = $4
              WHERE id = $5",
        )
        .bind(css_version)
        .bind(css_size)
        .bind(original_filename)
        .bind(now_iso())
        .bind(id)
        .execute(&mut *tx)
        .await?;
    }
    tx.commit().await?;
    Ok(())
}

/// Delete the upload and its catalogue row in one transaction.
pub async fn delete(pool: &Pool, id: i64, key: &str) -> Result<(), sqlx::Error> {
    let mut tx = crate::db::begin_write(pool).await?;
    sqlx::query("DELETE FROM custom_themes WHERE id = $1")
        .bind(id)
        .execute(&mut *tx)
        .await?;
    sqlx::query("DELETE FROM themes WHERE key = $1")
        .bind(key)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    Ok(())
}

fn row_from(row: ThemeTuple) -> CustomThemeRow {
    CustomThemeRow {
        id: row.0,
        key: row.1,
        name: row.2,
        description: row.3,
        appearance: ThemeAppearance::parse(&row.4),
        css_version: row.5,
        css_size: row.6,
        original_filename: row.7,
        created_by: row.8,
        created_at: row.9,
        updated_at: row.10,
    }
}
