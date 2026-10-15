//! `audit_logs` table.

use sqlx::{FromRow, SqlitePool};

#[derive(Debug, Clone, FromRow)]
pub struct AuditLog {
    pub id: i64,
    pub actor_user_id: Option<i64>,
    pub actor_role: Option<String>,
    pub action: String,
    pub target_type: Option<String>,
    pub target_id: Option<String>,
    pub metadata: Option<String>,
    pub ip_hash: Option<String>,
    pub created_at: String,
}

/// Row to insert.
#[derive(Debug, Clone, Default)]
pub struct NewAuditLog {
    pub actor_user_id: Option<i64>,
    pub actor_role: Option<String>,
    pub action: String,
    pub target_type: Option<String>,
    pub target_id: Option<String>,
    pub metadata: Option<String>,
    pub ip_hash: Option<String>,
}

/// Append an audit entry.
pub async fn insert(pool: &SqlitePool, entry: &NewAuditLog) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT INTO audit_logs (actor_user_id, actor_role, action, target_type, target_id,
                                 metadata, ip_hash, created_at)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(entry.actor_user_id)
    .bind(entry.actor_role.as_deref())
    .bind(&entry.action)
    .bind(entry.target_type.as_deref())
    .bind(entry.target_id.as_deref())
    .bind(entry.metadata.as_deref())
    .bind(entry.ip_hash.as_deref())
    .bind(crate::services::time::now_iso())
    .execute(pool)
    .await?;
    Ok(())
}

/// Filters for the audit log listing.
#[derive(Debug, Clone, Default)]
pub struct AuditFilter {
    pub action: Option<String>,
    pub target_type: Option<String>,
    pub actor_user_id: Option<i64>,
}

/// List audit entries, newest first.
pub async fn list(
    pool: &SqlitePool,
    filter: &AuditFilter,
    limit: i64,
    offset: i64,
) -> Result<Vec<AuditLog>, sqlx::Error> {
    sqlx::query_as::<_, AuditLog>(
        "SELECT id, actor_user_id, actor_role, action, target_type, target_id, metadata, ip_hash,
                created_at
         FROM audit_logs
         WHERE (?1 IS NULL OR action = ?1)
           AND (?2 IS NULL OR target_type = ?2)
           AND (?3 IS NULL OR actor_user_id = ?3)
         ORDER BY id DESC
         LIMIT ?4 OFFSET ?5",
    )
    .bind(filter.action.as_deref())
    .bind(filter.target_type.as_deref())
    .bind(filter.actor_user_id)
    .bind(limit)
    .bind(offset)
    .fetch_all(pool)
    .await
}

/// Count audit entries matching a filter.
pub async fn count(pool: &SqlitePool, filter: &AuditFilter) -> Result<i64, sqlx::Error> {
    sqlx::query_scalar(
        "SELECT COUNT(*) FROM audit_logs
         WHERE (?1 IS NULL OR action = ?1)
           AND (?2 IS NULL OR target_type = ?2)
           AND (?3 IS NULL OR actor_user_id = ?3)",
    )
    .bind(filter.action.as_deref())
    .bind(filter.target_type.as_deref())
    .bind(filter.actor_user_id)
    .fetch_one(pool)
    .await
}

/// Delete entries older than `before`.
pub async fn delete_older_than(pool: &SqlitePool, before: &str) -> Result<u64, sqlx::Error> {
    let result = sqlx::query("DELETE FROM audit_logs WHERE created_at < ?")
        .bind(before)
        .execute(pool)
        .await?;
    Ok(result.rows_affected())
}
