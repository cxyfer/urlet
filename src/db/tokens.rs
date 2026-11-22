//! `api_tokens` table.

use crate::db::Pool;
use sqlx::FromRow;

#[derive(Debug, Clone, FromRow)]
pub struct ApiToken {
    pub id: i64,
    pub user_id: i64,
    pub name: String,
    pub token_prefix: String,
    pub token_hash: String,
    pub scopes: String,
    pub last_used_at: Option<String>,
    pub expires_at: Option<String>,
    pub created_at: String,
    pub revoked_at: Option<String>,
}

impl ApiToken {
    pub fn is_valid_at(&self, now: &str) -> bool {
        self.revoked_at.is_none()
            && self
                .expires_at
                .as_deref()
                .map(|exp| exp > now)
                .unwrap_or(true)
    }
}

const SELECT: &str = "SELECT id, user_id, name, token_prefix, token_hash, scopes, last_used_at, \
                      expires_at, created_at, revoked_at FROM api_tokens";

/// Look up a token by the hash of its secret.
pub async fn find_by_hash(pool: &Pool, hash: &str) -> Result<Option<ApiToken>, sqlx::Error> {
    sqlx::query_as::<_, ApiToken>(&format!("{SELECT} WHERE token_hash = $1"))
        .bind(hash)
        .fetch_optional(pool)
        .await
}

/// Fields required to create a token.
#[derive(Debug, Clone)]
pub struct NewToken<'a> {
    pub user_id: i64,
    pub name: &'a str,
    pub token_prefix: &'a str,
    pub token_hash: &'a str,
    pub scopes: &'a str,
    pub expires_at: Option<&'a str>,
}

/// Create a token row.
pub async fn create(
    pool: &Pool,
    token: &NewToken<'_>,
    created_at: &str,
) -> Result<i64, sqlx::Error> {
    let id = sqlx::query_scalar::<_, i64>(
        "INSERT INTO api_tokens (user_id, name, token_prefix, token_hash, scopes, expires_at,
                                 created_at)
         VALUES ($1, $2, $3, $4, $5, $6, $7) RETURNING id",
    )
    .bind(token.user_id)
    .bind(token.name)
    .bind(token.token_prefix)
    .bind(token.token_hash)
    .bind(token.scopes)
    .bind(token.expires_at)
    .bind(created_at)
    .fetch_one(pool)
    .await?;
    Ok(id)
}

/// Insert a token only while the user has fewer than `limit` usable tokens.
///
/// The count and the insert run in one write transaction, so concurrent
/// creations cannot both pass a check made before either insert. Returns `false` when the quota
/// is already full.
pub async fn create_within_quota(
    pool: &Pool,
    token: &NewToken<'_>,
    created_at: &str,
    now: &str,
    limit: i64,
) -> Result<bool, sqlx::Error> {
    // 計數與插入同屬一個寫入交易：PostgreSQL 不會序列化 autocommit 敘述，
    // 單一 `INSERT … SELECT … WHERE (SELECT COUNT(*) …)` 在並行時可能超額（FR-DB-6）。
    let mut tx = crate::db::begin_write(pool).await?;
    let result = sqlx::query(
        "INSERT INTO api_tokens (user_id, name, token_prefix, token_hash, scopes, expires_at,
                                 created_at)
         SELECT $1, $2, $3, $4, $5, $6, $7
          WHERE (SELECT COUNT(*) FROM api_tokens
                  WHERE user_id = $8 AND revoked_at IS NULL
                    AND (expires_at IS NULL OR expires_at > $9)) < $10",
    )
    .bind(token.user_id)
    .bind(token.name)
    .bind(token.token_prefix)
    .bind(token.token_hash)
    .bind(token.scopes)
    .bind(token.expires_at)
    .bind(created_at)
    .bind(token.user_id)
    .bind(now)
    .bind(limit)
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(result.rows_affected() > 0)
}

/// Record token usage (called on every authenticated API request).
pub async fn touch_last_used(pool: &Pool, id: i64, now: &str) -> Result<(), sqlx::Error> {
    sqlx::query("UPDATE api_tokens SET last_used_at = $1 WHERE id = $2")
        .bind(now)
        .bind(id)
        .execute(pool)
        .await?;
    Ok(())
}

/// Number of live tokens across the whole instance.
pub async fn count_active(pool: &Pool, now: &str) -> Result<i64, sqlx::Error> {
    sqlx::query_scalar(
        "SELECT COUNT(*) FROM api_tokens
         WHERE revoked_at IS NULL AND (expires_at IS NULL OR expires_at > $1)",
    )
    .bind(now)
    .fetch_one(pool)
    .await
}

/// Number of usable tokens of one user: not revoked and not expired.
pub async fn count_active_for_user(
    pool: &Pool,
    user_id: i64,
    now: &str,
) -> Result<i64, sqlx::Error> {
    sqlx::query_scalar(
        "SELECT COUNT(*) FROM api_tokens
         WHERE user_id = $1 AND revoked_at IS NULL
           AND (expires_at IS NULL OR expires_at > $2)",
    )
    .bind(user_id)
    .bind(now)
    .fetch_one(pool)
    .await
}

/// Tokens of one user, newest first.
pub async fn list_for_user(pool: &Pool, user_id: i64) -> Result<Vec<ApiToken>, sqlx::Error> {
    sqlx::query_as::<_, ApiToken>(&format!(
        "{SELECT} WHERE user_id = $1 ORDER BY created_at DESC LIMIT 100"
    ))
    .bind(user_id)
    .fetch_all(pool)
    .await
}

/// Revoke every live token of a user (used when credentials change).
pub async fn revoke_all_for_user(pool: &Pool, user_id: i64, now: &str) -> Result<u64, sqlx::Error> {
    let result = sqlx::query(
        "UPDATE api_tokens SET revoked_at = $1 WHERE user_id = $2 AND revoked_at IS NULL",
    )
    .bind(now)
    .bind(user_id)
    .execute(pool)
    .await?;
    Ok(result.rows_affected())
}

/// Revoke a token owned by `user_id`. Returns `true` when a row changed.
pub async fn revoke(pool: &Pool, id: i64, user_id: i64, now: &str) -> Result<bool, sqlx::Error> {
    let result = sqlx::query(
        "UPDATE api_tokens SET revoked_at = $1 WHERE id = $2 AND user_id = $3 AND revoked_at IS NULL",
    )
    .bind(now)
    .bind(id)
    .bind(user_id)
    .execute(pool)
    .await?;
    Ok(result.rows_affected() > 0)
}
