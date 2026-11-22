//! `login_attempts` table: forensic trail for authentication attempts.

use crate::db::Pool;

/// Reserve one attempt for an identifier, recorded as a failure.
///
/// The row is inserted only while fewer than `threshold` failures exist since
/// `since`; the count and the insert run in one write transaction, so
/// concurrent reservations cannot overshoot on either backend.
/// Returns the new row id, or `None` when the identifier is locked.
pub async fn reserve_failure(
    pool: &Pool,
    identifier_hash: &str,
    ip_hash: Option<&str>,
    created_at: &str,
    since: &str,
    threshold: i64,
) -> Result<Option<i64>, sqlx::Error> {
    // 計數與插入同屬一個寫入交易：PostgreSQL 不會序列化 autocommit 敘述，
    // 單一 `INSERT … SELECT … WHERE (SELECT COUNT(*) …)` 在並行時可能超額（FR-DB-6）。
    let mut tx = crate::db::begin_write(pool).await?;
    let id = sqlx::query_scalar(
        "INSERT INTO login_attempts (identifier_hash, ip_hash, success, created_at)
         SELECT $1, $2, 0, $3
          WHERE (SELECT COUNT(*) FROM login_attempts
                  WHERE identifier_hash = $4 AND success = 0 AND created_at >= $5) < $6
         RETURNING id",
    )
    .bind(identifier_hash)
    .bind(ip_hash)
    .bind(created_at)
    .bind(identifier_hash)
    .bind(since)
    .bind(threshold)
    .fetch_optional(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(id)
}

/// Mark a reserved attempt as successful, so it no longer counts as a failure.
pub async fn mark_success(pool: &Pool, id: i64) -> Result<(), sqlx::Error> {
    sqlx::query("UPDATE login_attempts SET success = 1 WHERE id = $1")
        .bind(id)
        .execute(pool)
        .await?;
    Ok(())
}

/// Delete attempts older than `before` (retention).
pub async fn delete_older_than(pool: &Pool, before: &str) -> Result<u64, sqlx::Error> {
    let result = sqlx::query("DELETE FROM login_attempts WHERE created_at < $1")
        .bind(before)
        .execute(pool)
        .await?;
    Ok(result.rows_affected())
}
