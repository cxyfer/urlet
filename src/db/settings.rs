//! `settings` table: simple key/value store for site wide configuration.

use std::collections::HashMap;

use sqlx::SqlitePool;

use crate::services::time::now_iso;

/// Every setting the application understands, with its default value.
pub const DEFAULTS: &[(&str, &str)] = &[
    ("site_name", "uwurls"),
    ("registration_mode", "open"),
    ("default_slug_length", "7"),
    ("max_urls_per_user", "1000"),
    ("default_redirect_type", "302"),
    ("analytics_enabled", "true"),
    ("click_retention_days", "0"),
];

/// Load all settings.
pub async fn all(pool: &SqlitePool) -> Result<HashMap<String, String>, sqlx::Error> {
    let rows: Vec<(String, String)> = sqlx::query_as("SELECT key, value FROM settings")
        .fetch_all(pool)
        .await?;
    Ok(rows.into_iter().collect())
}

/// Read one setting.
pub async fn get(pool: &SqlitePool, key: &str) -> Result<Option<String>, sqlx::Error> {
    sqlx::query_scalar("SELECT value FROM settings WHERE key = ?")
        .bind(key)
        .fetch_optional(pool)
        .await
}

/// Insert or update one setting.
pub async fn set(
    pool: &SqlitePool,
    key: &str,
    value: &str,
    updated_by: Option<i64>,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT INTO settings (key, value, updated_at, updated_by)
         VALUES (?, ?, ?, ?)
         ON CONFLICT (key) DO UPDATE SET value = excluded.value,
                                        updated_at = excluded.updated_at,
                                        updated_by = excluded.updated_by",
    )
    .bind(key)
    .bind(value)
    .bind(now_iso())
    .bind(updated_by)
    .execute(pool)
    .await?;
    Ok(())
}

/// Ensure every known key exists (used at start-up).
pub async fn ensure_defaults(pool: &SqlitePool) -> Result<(), sqlx::Error> {
    for (key, value) in DEFAULTS {
        sqlx::query(
            "INSERT OR IGNORE INTO settings (key, value, updated_at, updated_by)
             VALUES (?, ?, ?, NULL)",
        )
        .bind(key)
        .bind(value)
        .bind(now_iso())
        .execute(pool)
        .await?;
    }
    Ok(())
}
