//! `login_attempts` table: forensic trail for authentication attempts.

use sqlx::SqlitePool;

/// Record one attempt (successful or not).
pub async fn record(
    pool: &SqlitePool,
    identifier_hash: &str,
    ip_hash: Option<&str>,
    success: bool,
    created_at: &str,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT INTO login_attempts (identifier_hash, ip_hash, success, created_at)
         VALUES (?, ?, ?, ?)",
    )
    .bind(identifier_hash)
    .bind(ip_hash)
    .bind(i64::from(success))
    .bind(created_at)
    .execute(pool)
    .await?;
    Ok(())
}

/// Number of failed attempts for an identifier since `since`.
pub async fn count_failures_since(
    pool: &SqlitePool,
    identifier_hash: &str,
    since: &str,
) -> Result<i64, sqlx::Error> {
    sqlx::query_scalar(
        "SELECT COUNT(*) FROM login_attempts
         WHERE identifier_hash = ? AND success = 0 AND created_at >= ?",
    )
    .bind(identifier_hash)
    .bind(since)
    .fetch_one(pool)
    .await
}

/// Delete attempts older than `before` (retention).
pub async fn delete_older_than(pool: &SqlitePool, before: &str) -> Result<u64, sqlx::Error> {
    let result = sqlx::query("DELETE FROM login_attempts WHERE created_at < ?")
        .bind(before)
        .execute(pool)
        .await?;
    Ok(result.rows_affected())
}
