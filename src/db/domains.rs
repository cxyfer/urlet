//! `domains` table. Version 1 only ever uses the configured default domain;
//! the table exists so custom domains can be added without a migration.

use sqlx::SqlitePool;

use crate::services::time::now_iso;

/// Ensure `hostname` exists and is the single default domain.
pub async fn ensure_default(pool: &SqlitePool, hostname: &str) -> Result<i64, sqlx::Error> {
    let mut tx = pool.begin().await?;

    sqlx::query("UPDATE domains SET is_default = 0 WHERE is_default = 1 AND hostname <> ?")
        .bind(hostname)
        .execute(&mut *tx)
        .await?;

    sqlx::query(
        "INSERT INTO domains (hostname, is_default, is_verified, created_at)
         VALUES (?, 1, 1, ?)
         ON CONFLICT (hostname) DO UPDATE SET is_default = 1, is_verified = 1",
    )
    .bind(hostname)
    .bind(now_iso())
    .execute(&mut *tx)
    .await?;

    let id: i64 = sqlx::query_scalar("SELECT id FROM domains WHERE hostname = ?")
        .bind(hostname)
        .fetch_one(&mut *tx)
        .await?;

    tx.commit().await?;
    Ok(id)
}

/// Id of the default domain, if one has been registered.
pub async fn default_id(pool: &SqlitePool) -> Result<Option<i64>, sqlx::Error> {
    sqlx::query_scalar("SELECT id FROM domains WHERE is_default = 1 LIMIT 1").fetch_optional(pool).await
}

/// All known hostnames.
pub async fn all_hostnames(pool: &SqlitePool) -> Result<Vec<String>, sqlx::Error> {
    sqlx::query_scalar("SELECT hostname FROM domains ORDER BY hostname").fetch_all(pool).await
}
