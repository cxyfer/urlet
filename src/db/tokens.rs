//! `api_tokens` table.

use sqlx::{FromRow, SqlitePool};

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
pub async fn find_by_hash(pool: &SqlitePool, hash: &str) -> Result<Option<ApiToken>, sqlx::Error> {
    sqlx::query_as::<_, ApiToken>(&format!("{SELECT} WHERE token_hash = ?"))
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
    pool: &SqlitePool,
    token: &NewToken<'_>,
    created_at: &str,
) -> Result<i64, sqlx::Error> {
    let result = sqlx::query(
        "INSERT INTO api_tokens (user_id, name, token_prefix, token_hash, scopes, expires_at,
                                 created_at)
         VALUES (?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(token.user_id)
    .bind(token.name)
    .bind(token.token_prefix)
    .bind(token.token_hash)
    .bind(token.scopes)
    .bind(token.expires_at)
    .bind(created_at)
    .execute(pool)
    .await?;
    Ok(result.last_insert_rowid())
}

/// Insert a token only while the user has fewer than `limit` usable tokens.
///
/// The count and the insert are one statement, so concurrent creations cannot
/// both pass a check made before either insert. Returns `false` when the quota
/// is already full.
pub async fn create_within_quota(
    pool: &SqlitePool,
    token: &NewToken<'_>,
    created_at: &str,
    now: &str,
    limit: i64,
) -> Result<bool, sqlx::Error> {
    let result = sqlx::query(
        "INSERT INTO api_tokens (user_id, name, token_prefix, token_hash, scopes, expires_at,
                                 created_at)
         SELECT ?, ?, ?, ?, ?, ?, ?
          WHERE (SELECT COUNT(*) FROM api_tokens
                  WHERE user_id = ? AND revoked_at IS NULL
                    AND (expires_at IS NULL OR expires_at > ?)) < ?",
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
    .execute(pool)
    .await?;
    Ok(result.rows_affected() > 0)
}

/// Record token usage (called on every authenticated API request).
pub async fn touch_last_used(pool: &SqlitePool, id: i64, now: &str) -> Result<(), sqlx::Error> {
    sqlx::query("UPDATE api_tokens SET last_used_at = ? WHERE id = ?")
        .bind(now)
        .bind(id)
        .execute(pool)
        .await?;
    Ok(())
}

/// Number of live tokens across the whole instance.
pub async fn count_active(pool: &SqlitePool, now: &str) -> Result<i64, sqlx::Error> {
    sqlx::query_scalar(
        "SELECT COUNT(*) FROM api_tokens
         WHERE revoked_at IS NULL AND (expires_at IS NULL OR expires_at > ?)",
    )
    .bind(now)
    .fetch_one(pool)
    .await
}

/// Number of usable tokens of one user: not revoked and not expired.
pub async fn count_active_for_user(
    pool: &SqlitePool,
    user_id: i64,
    now: &str,
) -> Result<i64, sqlx::Error> {
    sqlx::query_scalar(
        "SELECT COUNT(*) FROM api_tokens
         WHERE user_id = ? AND revoked_at IS NULL
           AND (expires_at IS NULL OR expires_at > ?)",
    )
    .bind(user_id)
    .bind(now)
    .fetch_one(pool)
    .await
}

/// Tokens of one user, newest first.
pub async fn list_for_user(pool: &SqlitePool, user_id: i64) -> Result<Vec<ApiToken>, sqlx::Error> {
    sqlx::query_as::<_, ApiToken>(&format!(
        "{SELECT} WHERE user_id = ? ORDER BY created_at DESC LIMIT 100"
    ))
    .bind(user_id)
    .fetch_all(pool)
    .await
}

/// Revoke every live token of a user (used when credentials change).
pub async fn revoke_all_for_user(
    pool: &SqlitePool,
    user_id: i64,
    now: &str,
) -> Result<u64, sqlx::Error> {
    let result = sqlx::query(
        "UPDATE api_tokens SET revoked_at = ? WHERE user_id = ? AND revoked_at IS NULL",
    )
    .bind(now)
    .bind(user_id)
    .execute(pool)
    .await?;
    Ok(result.rows_affected())
}

/// Revoke a token owned by `user_id`. Returns `true` when a row changed.
pub async fn revoke(
    pool: &SqlitePool,
    id: i64,
    user_id: i64,
    now: &str,
) -> Result<bool, sqlx::Error> {
    let result = sqlx::query(
        "UPDATE api_tokens SET revoked_at = ? WHERE id = ? AND user_id = ? AND revoked_at IS NULL",
    )
    .bind(now)
    .bind(id)
    .bind(user_id)
    .execute(pool)
    .await?;
    Ok(result.rows_affected() > 0)
}
