//! `login_attempts` table: forensic trail for authentication attempts.

use sqlx::SqlitePool;

/// Reserve one attempt for an identifier, recorded as a failure.
///
/// The row is inserted only while fewer than `threshold` failures exist since
/// `since`; the count and the insert are one statement, so SQLite evaluates
/// them under the write lock and concurrent reservations cannot overshoot.
/// Returns the new row id, or `None` when the identifier is locked.
pub async fn reserve_failure(
    pool: &SqlitePool,
    identifier_hash: &str,
    ip_hash: Option<&str>,
    created_at: &str,
    since: &str,
    threshold: i64,
) -> Result<Option<i64>, sqlx::Error> {
    sqlx::query_scalar(
        "INSERT INTO login_attempts (identifier_hash, ip_hash, success, created_at)
         SELECT ?, ?, 0, ?
          WHERE (SELECT COUNT(*) FROM login_attempts
                  WHERE identifier_hash = ? AND success = 0 AND created_at >= ?) < ?
         RETURNING id",
    )
    .bind(identifier_hash)
    .bind(ip_hash)
    .bind(created_at)
    .bind(identifier_hash)
    .bind(since)
    .bind(threshold)
    .fetch_optional(pool)
    .await
}

/// Mark a reserved attempt as successful, so it no longer counts as a failure.
pub async fn mark_success(pool: &SqlitePool, id: i64) -> Result<(), sqlx::Error> {
    sqlx::query("UPDATE login_attempts SET success = 1 WHERE id = ?")
        .bind(id)
        .execute(pool)
        .await?;
    Ok(())
}

/// Delete attempts older than `before` (retention).
pub async fn delete_older_than(pool: &SqlitePool, before: &str) -> Result<u64, sqlx::Error> {
    let result = sqlx::query("DELETE FROM login_attempts WHERE created_at < ?")
        .bind(before)
        .execute(pool)
        .await?;
    Ok(result.rows_affected())
}
