//! `custom_themes` table: metadata for administrator-uploaded stylesheets.
//!
//! The CSS itself lives in `URLET_THEMES_DIR`; this table keeps the name, the
//! cache-busting content hash and the file size. A matching row in `themes`
//! makes the upload participate in the normal enabled/default catalogue, so a
//! custom theme behaves exactly like a compiled one once it is registered.

use sqlx::SqlitePool;

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
pub async fn all(pool: &SqlitePool) -> Result<Vec<CustomThemeRow>, sqlx::Error> {
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
pub async fn find(pool: &SqlitePool, id: i64) -> Result<Option<CustomThemeRow>, sqlx::Error> {
    let row: Option<ThemeTuple> = sqlx::query_as(
        "SELECT id, key, name, description, appearance, css_version, css_size,
                original_filename, created_by, created_at, updated_at
           FROM custom_themes
          WHERE id = ?",
    )
    .bind(id)
    .fetch_optional(pool)
    .await?;
    Ok(row.map(row_from))
}

/// One upload by stable catalogue key.
pub async fn find_by_key(
    pool: &SqlitePool,
    key: &str,
) -> Result<Option<CustomThemeRow>, sqlx::Error> {
    let row: Option<ThemeTuple> = sqlx::query_as(
        "SELECT id, key, name, description, appearance, css_version, css_size,
                original_filename, created_by, created_at, updated_at
           FROM custom_themes
          WHERE key = ?",
    )
    .bind(key)
    .fetch_optional(pool)
    .await?;
    Ok(row.map(row_from))
}

/// Number of uploads, used for the catalogue limit.
pub async fn count(pool: &SqlitePool) -> Result<i64, sqlx::Error> {
    sqlx::query_scalar("SELECT COUNT(*) FROM custom_themes")
        .fetch_one(pool)
        .await
}

/// Stable keys of every upload, used by the startup reconciliation.
pub async fn keys(pool: &SqlitePool) -> Result<Vec<String>, sqlx::Error> {
    sqlx::query_scalar("SELECT key FROM custom_themes ORDER BY id")
        .fetch_all(pool)
        .await
}

/// Register an upload and its `themes` catalogue row in one transaction.
pub async fn create(pool: &SqlitePool, new: &NewCustomTheme) -> Result<i64, sqlx::Error> {
    let now = now_iso();
    let mut tx = pool.begin().await?;

    sqlx::query(
        "INSERT INTO themes
             (key, is_enabled, is_default, sort_order, created_at, updated_at)
         VALUES (?, 1, 0,
                 (SELECT COALESCE(MAX(sort_order), 0) + 10 FROM themes), ?, ?)",
    )
    .bind(&new.key)
    .bind(&now)
    .bind(&now)
    .execute(&mut *tx)
    .await?;

    let result = sqlx::query(
        "INSERT INTO custom_themes
             (key, name, description, appearance, css_version, css_size,
              original_filename, created_by, created_at, updated_at)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
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
    .execute(&mut *tx)
    .await?;

    tx.commit().await?;
    Ok(result.last_insert_rowid())
}

/// Update the editable name/description/dark-only flags.
pub async fn update_metadata(
    pool: &SqlitePool,
    id: i64,
    name: &str,
    description: &str,
    appearance: ThemeAppearance,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "UPDATE custom_themes
            SET name = ?, description = ?, appearance = ?, updated_at = ?
          WHERE id = ?",
    )
    .bind(name)
    .bind(description)
    .bind(appearance.as_str())
    .bind(now_iso())
    .bind(id)
    .execute(pool)
    .await?;
    Ok(())
}

/// Record a replaced stylesheet so its URL cache-busts immediately.
pub async fn update_css(
    pool: &SqlitePool,
    id: i64,
    css_version: &str,
    css_size: i64,
    original_filename: &str,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "UPDATE custom_themes
            SET css_version = ?, css_size = ?, original_filename = ?, updated_at = ?
          WHERE id = ?",
    )
    .bind(css_version)
    .bind(css_size)
    .bind(original_filename)
    .bind(now_iso())
    .bind(id)
    .execute(pool)
    .await?;
    Ok(())
}

/// Delete the upload and its catalogue row in one transaction.
pub async fn delete(pool: &SqlitePool, id: i64, key: &str) -> Result<(), sqlx::Error> {
    let mut tx = pool.begin().await?;
    sqlx::query("DELETE FROM custom_themes WHERE id = ?")
        .bind(id)
        .execute(&mut *tx)
        .await?;
    sqlx::query("DELETE FROM themes WHERE key = ?")
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
