//! `sessions` table: server side login state.

use sqlx::{FromRow, SqlitePool};

#[derive(Debug, Clone, FromRow)]
pub struct Session {
    pub id: i64,
    pub user_id: i64,
    pub token_hash: String,
    pub user_agent: Option<String>,
    pub ip_hash: Option<String>,
    pub created_at: String,
    pub last_seen_at: String,
    pub expires_at: String,
    pub revoked_at: Option<String>,
}

impl Session {
    pub fn is_valid_at(&self, now: &str) -> bool {
        self.revoked_at.is_none() && self.expires_at.as_str() > now
    }
}

const SELECT: &str = "SELECT id, user_id, token_hash, user_agent, ip_hash, created_at, \
                      last_seen_at, expires_at, revoked_at FROM sessions";

/// Create a session row.
pub async fn create(
    pool: &SqlitePool,
    user_id: i64,
    token_hash: &str,
    user_agent: Option<&str>,
    ip_hash: Option<&str>,
    created_at: &str,
    expires_at: &str,
) -> Result<i64, sqlx::Error> {
    let result = sqlx::query(
        "INSERT INTO sessions (user_id, token_hash, user_agent, ip_hash, created_at, last_seen_at,
                               expires_at)
         VALUES (?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(user_id)
    .bind(token_hash)
    .bind(user_agent)
    .bind(ip_hash)
    .bind(created_at)
    .bind(created_at)
    .bind(expires_at)
    .execute(pool)
    .await?;
    Ok(result.last_insert_rowid())
}

/// Look up a session by the hash of its token.
pub async fn find_by_token_hash(
    pool: &SqlitePool,
    token_hash: &str,
) -> Result<Option<Session>, sqlx::Error> {
    sqlx::query_as::<_, Session>(&format!("{SELECT} WHERE token_hash = ?"))
        .bind(token_hash)
        .fetch_optional(pool)
        .await
}

/// Refresh `last_seen_at`, but only when it is older than `stale_before`
/// (keeps the write path off the hot read path).
pub async fn touch(
    pool: &SqlitePool,
    id: i64,
    stale_before: &str,
    now: &str,
) -> Result<(), sqlx::Error> {
    sqlx::query("UPDATE sessions SET last_seen_at = ? WHERE id = ? AND last_seen_at < ?")
        .bind(now)
        .bind(id)
        .bind(stale_before)
        .execute(pool)
        .await?;
    Ok(())
}

/// Revoke one session. Returns the number of rows changed.
pub async fn revoke(pool: &SqlitePool, id: i64, now: &str) -> Result<u64, sqlx::Error> {
    let result = sqlx::query("UPDATE sessions SET revoked_at = ? WHERE id = ? AND revoked_at IS NULL")
        .bind(now)
        .bind(id)
        .execute(pool)
        .await?;
    Ok(result.rows_affected())
}

/// Revoke every session of a user, optionally keeping one alive.
pub async fn revoke_for_user(
    pool: &SqlitePool,
    user_id: i64,
    now: &str,
    keep: Option<i64>,
) -> Result<u64, sqlx::Error> {
    let result = match keep {
        Some(id) => {
            sqlx::query(
                "UPDATE sessions SET revoked_at = ?
                 WHERE user_id = ? AND revoked_at IS NULL AND id <> ?",
            )
            .bind(now)
            .bind(user_id)
            .bind(id)
            .execute(pool)
            .await?
        }
        None => {
            sqlx::query("UPDATE sessions SET revoked_at = ? WHERE user_id = ? AND revoked_at IS NULL")
                .bind(now)
                .bind(user_id)
                .execute(pool)
                .await?
        }
    };
    Ok(result.rows_affected())
}

/// Revoke a session that belongs to `user_id`.
pub async fn revoke_owned(
    pool: &SqlitePool,
    id: i64,
    user_id: i64,
    now: &str,
) -> Result<bool, sqlx::Error> {
    let result = sqlx::query(
        "UPDATE sessions SET revoked_at = ? WHERE id = ? AND user_id = ? AND revoked_at IS NULL",
    )
    .bind(now)
    .bind(id)
    .bind(user_id)
    .execute(pool)
    .await?;
    Ok(result.rows_affected() > 0)
}

/// Sessions belonging to a user, newest first.
pub async fn list_for_user(pool: &SqlitePool, user_id: i64) -> Result<Vec<Session>, sqlx::Error> {
    sqlx::query_as::<_, Session>(&format!(
        "{SELECT} WHERE user_id = ? ORDER BY last_seen_at DESC LIMIT 50"
    ))
    .bind(user_id)
    .fetch_all(pool)
    .await
}

/// Delete sessions that expired or were revoked before `before`.
pub async fn delete_stale(pool: &SqlitePool, before: &str) -> Result<u64, sqlx::Error> {
    let result = sqlx::query("DELETE FROM sessions WHERE expires_at < ? OR revoked_at < ?")
        .bind(before)
        .bind(before)
        .execute(pool)
        .await?;
    Ok(result.rows_affected())
}
