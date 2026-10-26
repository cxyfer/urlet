//! `themes` table: which compiled-in themes are offered, and which one is the
//! default.
//!
//! The themes themselves live in code (`services::theme`) and in `templates/`;
//! this table only stores administrator choices. Keeping the selection in the
//! database means a restart or a theme upgrade never silently changes what
//! visitors see.

use sqlx::SqlitePool;

use crate::services::time::now_iso;

/// One row of the `themes` table.
#[derive(Debug, Clone)]
pub struct ThemeRow {
    pub key: String,
    pub is_enabled: bool,
    pub is_default: bool,
    pub sort_order: i64,
    pub created_at: String,
    pub updated_at: String,
}

/// Every theme, ordered for the admin list and the picker.
pub async fn all(pool: &SqlitePool) -> Result<Vec<ThemeRow>, sqlx::Error> {
    let rows: Vec<(String, i64, i64, i64, String, String)> = sqlx::query_as(
        "SELECT key, is_enabled, is_default, sort_order, created_at, updated_at
           FROM themes
          ORDER BY sort_order, key",
    )
    .fetch_all(pool)
    .await?;
    Ok(rows.into_iter().map(row_from).collect())
}

/// Only the themes an administrator has enabled, in picker order.
pub async fn enabled(pool: &SqlitePool) -> Result<Vec<ThemeRow>, sqlx::Error> {
    let rows: Vec<(String, i64, i64, i64, String, String)> = sqlx::query_as(
        "SELECT key, is_enabled, is_default, sort_order, created_at, updated_at
           FROM themes
          WHERE is_enabled = 1
          ORDER BY sort_order, key",
    )
    .fetch_all(pool)
    .await?;
    Ok(rows.into_iter().map(row_from).collect())
}

/// Read one theme row.
pub async fn find(pool: &SqlitePool, key: &str) -> Result<Option<ThemeRow>, sqlx::Error> {
    let row: Option<(String, i64, i64, i64, String, String)> = sqlx::query_as(
        "SELECT key, is_enabled, is_default, sort_order, created_at, updated_at
           FROM themes
          WHERE key = ?",
    )
    .bind(key)
    .fetch_optional(pool)
    .await?;
    Ok(row.map(row_from))
}

/// Insert any theme that is compiled in but missing from the table.
///
/// Migration 0009 seeds the initial six. This helper keeps a later release that
/// adds a theme usable without a migration rewrite, and an upgrade from an
/// older database gains the new row with its default enabled state.
pub async fn ensure_defaults(
    pool: &SqlitePool,
    keys: &[&str],
    default_key: &str,
) -> Result<(), sqlx::Error> {
    let existing: Vec<String> = sqlx::query_scalar("SELECT key FROM themes")
        .fetch_all(pool)
        .await?;
    let now = now_iso();
    for (index, key) in keys.iter().enumerate() {
        if existing.iter().any(|value| value == key) {
            continue;
        }
        let is_default = if *key == default_key { 1 } else { 0 };
        sqlx::query(
            "INSERT INTO themes (key, is_enabled, is_default, sort_order, created_at, updated_at)
             VALUES (?, 1, ?, ?, ?, ?)",
        )
        .bind(key)
        .bind(is_default)
        .bind(1000 + index as i64)
        .bind(&now)
        .bind(&now)
        .execute(pool)
        .await?;
    }
    Ok(())
}

/// Replace the enabled set, the default marker and the switching policy in one
/// transaction.
///
/// `enabled` must contain at least one key and `default_key` must be one of
/// them; callers validate that before reaching the database. The partial unique
/// index on `is_default` means the previous marker has to be cleared first.
///
/// The catalogue and `theme_switching` are written together on purpose: a
/// failure while saving the policy must not leave the enabled/default set
/// changed while the endpoint reports an error.
pub async fn set_state_and_setting(
    pool: &SqlitePool,
    enabled: &[String],
    default_key: &str,
    switching: &str,
    updated_by: Option<i64>,
) -> Result<(), sqlx::Error> {
    let now = now_iso();
    let mut tx = crate::db::begin_write(pool).await?;

    sqlx::query("UPDATE themes SET is_enabled = 0, updated_at = ?")
        .bind(&now)
        .execute(&mut *tx)
        .await?;

    for key in enabled {
        sqlx::query("UPDATE themes SET is_enabled = 1, updated_at = ? WHERE key = ?")
            .bind(&now)
            .bind(key)
            .execute(&mut *tx)
            .await?;
    }

    sqlx::query("UPDATE themes SET is_default = 0, updated_at = ? WHERE is_default = 1")
        .bind(&now)
        .execute(&mut *tx)
        .await?;
    sqlx::query("UPDATE themes SET is_default = 1, updated_at = ? WHERE key = ?")
        .bind(&now)
        .bind(default_key)
        .execute(&mut *tx)
        .await?;

    sqlx::query(
        "INSERT INTO settings (key, value, updated_at, updated_by)
         VALUES ('theme_switching', ?, ?, ?)
         ON CONFLICT (key) DO UPDATE SET value = excluded.value,
                                        updated_at = excluded.updated_at,
                                        updated_by = excluded.updated_by",
    )
    .bind(switching)
    .bind(&now)
    .bind(updated_by)
    .execute(&mut *tx)
    .await?;

    tx.commit().await?;
    Ok(())
}

fn row_from(row: (String, i64, i64, i64, String, String)) -> ThemeRow {
    ThemeRow {
        key: row.0,
        is_enabled: row.1 != 0,
        is_default: row.2 != 0,
        sort_order: row.3,
        created_at: row.4,
        updated_at: row.5,
    }
}
