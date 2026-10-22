//! `settings` table: simple key/value store for site wide configuration.

use std::collections::HashMap;

use sqlx::SqlitePool;

use crate::services::time::now_iso;

/// Every setting the application understands, with its default value.
pub const DEFAULTS: &[(&str, &str)] = &[
    ("site_name", "urlet"),
    ("registration_mode", "open"),
    ("default_slug_length", "7"),
    ("max_urls_per_user", "1000"),
    ("max_api_tokens_per_user", "20"),
    // Invites not pinned to a group consume this site-wide quota.
    ("max_invites_per_period", "0"),
    ("invite_period", "month"),
    ("default_redirect_type", "302"),
    ("analytics_enabled", "true"),
    ("url_cleaning_enabled", "false"),
    ("url_cleaning_rules", ""),
    ("click_retention_days", "0"),
    // Bot protection. Disabled until a provider, a site key and a secret key are
    // all set; see `services::captcha`.
    ("captcha_provider", "none"),
    ("captcha_site_key", ""),
    ("captcha_secret_key", ""),
    ("captcha_allowed_hostnames", ""),
    ("captcha_on_register", "false"),
    ("captcha_on_login", "false"),
    ("captcha_on_create", "false"),
    // Who may change their own theme: everyone ("all"), including a visitor
    // without a session, or administrators only ("admin"). Administrators can
    // always preview any enabled theme.
    ("theme_switching", "all"),
    // Public shortener. `user` keeps the historical behaviour (only signed-in
    // accounts create links); `guest` also serves the form on the landing page
    // and `admin` restricts creation to administrators. The rate limits count
    // creations per hour and 0 means unlimited; the field switches decide which
    // optional inputs the public form offers (the target URL is always
    // required, and the switches are enforced server side).
    ("public_create_access", "user"),
    ("guest_allowlist_enabled", "true"),
    (
        "guest_allowlist",
        crate::services::guest_allowlist::DEFAULT_RULES,
    ),
    ("public_captcha", "true"),
    ("public_rate_limit_per_ip", "20"),
    ("public_rate_limit_global", "500"),
    // How many links the reserved Guest account may own in total. Separate from
    // `max_urls_per_user` so a flood of anonymous links cannot eat the quota set
    // for real accounts; 0 means unlimited.
    ("public_max_urls", "1000"),
    ("public_allow_slug", "true"),
    ("public_allow_title", "true"),
    ("public_allow_tags", "false"),
    ("public_allow_password", "false"),
    ("public_allow_expiry", "false"),
    ("public_allow_max_clicks", "false"),
    ("public_allow_redirect", "false"),
    ("public_allow_domain", "false"),
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

/// Insert or update several settings inside one transaction.
///
/// Either every pair is stored or none is: a batch that fails halfway must not
/// leave the instance reconfigured with, say, a new captcha provider but the old
/// keys.
pub async fn set_many(
    pool: &SqlitePool,
    values: &[(String, String)],
    updated_by: Option<i64>,
) -> Result<(), sqlx::Error> {
    let now = now_iso();
    let mut tx = pool.begin().await?;
    for (key, value) in values {
        sqlx::query(
            "INSERT INTO settings (key, value, updated_at, updated_by)
             VALUES (?, ?, ?, ?)
             ON CONFLICT (key) DO UPDATE SET value = excluded.value,
                                            updated_at = excluded.updated_at,
                                            updated_by = excluded.updated_by",
        )
        .bind(key)
        .bind(value)
        .bind(&now)
        .bind(updated_by)
        .execute(&mut *tx)
        .await?;
    }
    tx.commit().await?;
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
