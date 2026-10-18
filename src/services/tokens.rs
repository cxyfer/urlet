//! API token issuance and lifecycle.

use sqlx::SqlitePool;

use crate::db::tokens::{self, ApiToken, NewToken};
use crate::db::users::User;
use crate::error::{AppError, Result};
use crate::services::audit::{action, target, Audit};
use crate::services::limits::{self, blocked};
use crate::services::settings::Settings;
use crate::services::{secret, time};

/// Prefix of every API token, so they are recognisable in logs and scanners.
pub const TOKEN_PREFIX: &str = "ur_";
/// Number of characters kept in clear for identification.
const DISPLAY_PREFIX_LEN: usize = 11;
/// Default scopes for a new token.
pub const DEFAULT_SCOPES: &str = r#"["urls:read","urls:write"]"#;
/// Longest accepted token name.
pub const MAX_NAME_LEN: usize = 60;

/// A freshly created token. The plain value is only available here.
#[derive(Debug, Clone)]
pub struct CreatedToken {
    pub record: ApiToken,
    pub plain: String,
}

/// Validate a token name.
pub fn validate_name(name: &str) -> Result<String> {
    let name = name.trim();
    if name.is_empty() {
        return Err(AppError::validation("name", "a token name is required"));
    }
    if name.chars().count() > MAX_NAME_LEN {
        return Err(AppError::validation(
            "name",
            format!("the token name must be at most {MAX_NAME_LEN} characters"),
        ));
    }
    if name.chars().any(char::is_control) {
        return Err(AppError::validation(
            "name",
            "the token name must not contain control characters",
        ));
    }
    Ok(name.to_string())
}

/// Validate an optional expiry timestamp.
pub fn validate_expiry(raw: Option<&str>) -> Result<Option<String>> {
    match raw.map(str::trim).filter(|value| !value.is_empty()) {
        None => Ok(None),
        Some(value) => {
            let parsed = time::parse_flexible(value).ok_or_else(|| {
                AppError::validation("expires_at", "expiry must look like 2030-01-31T12:00:00Z")
            })?;
            if parsed <= time::now() {
                return Err(AppError::validation(
                    "expires_at",
                    "expiry must be in the future",
                ));
            }
            Ok(Some(time::to_iso(parsed)))
        }
    }
}

/// Create a token for a user. The plain value is returned exactly once.
pub async fn create(
    pool: &SqlitePool,
    settings: &Settings,
    actor: &User,
    name: &str,
    expires_at: Option<&str>,
    ip_hash: Option<&str>,
) -> Result<CreatedToken> {
    let name = validate_name(name)?;
    let expires_at = validate_expiry(expires_at)?;

    let limits = limits::effective(pool, settings, actor).await?;
    if !limits.allow_api_tokens {
        return Err(AppError::forbidden(blocked::API_TOKENS));
    }

    let plain = format!("{TOKEN_PREFIX}{}", secret::random_token(32));
    let token_hash = secret::sha256_hex(&plain);
    let prefix: String = plain.chars().take(DISPLAY_PREFIX_LEN).collect();
    let now = time::now_iso();
    let new_token = NewToken {
        user_id: actor.id,
        name: &name,
        token_prefix: &prefix,
        token_hash: &token_hash,
        scopes: DEFAULT_SCOPES,
        expires_at: expires_at.as_deref(),
    };

    if limits.tokens_unlimited() {
        tokens::create(pool, &new_token, &now)
            .await
            .map_err(AppError::from)?;
    } else if !tokens::create_within_quota(pool, &new_token, &now, &now, limits.max_api_tokens)
        .await
        .map_err(AppError::from)?
    {
        // The insert and the count share one statement, so this is exact even
        // when several requests race for the last slot.
        return Err(AppError::forbidden(format!(
            "you have reached your limit of {} API tokens",
            limits.max_api_tokens
        )));
    }

    let record = tokens::find_by_hash(pool, &token_hash)
        .await
        .map_err(AppError::from)?
        .ok_or_else(|| AppError::internal("token disappeared after creation"))?;
    let id = record.id;

    Audit::new(action::TOKEN_CREATED)
        .actor(actor.id, &actor.role)
        .target(target::TOKEN, id)
        .metadata(serde_json::json!({ "name": name, "prefix": prefix }))
        .ip_hash(ip_hash.map(str::to_string))
        .record(pool)
        .await;

    Ok(CreatedToken { record, plain })
}

/// Tokens of a user, newest first.
pub async fn list(pool: &SqlitePool, user_id: i64) -> Result<Vec<ApiToken>> {
    tokens::list_for_user(pool, user_id)
        .await
        .map_err(AppError::from)
}

/// Revoke a token owned by a user.
pub async fn revoke(pool: &SqlitePool, actor: &User, id: i64, ip_hash: Option<&str>) -> Result<()> {
    let revoked = tokens::revoke(pool, id, actor.id, &time::now_iso())
        .await
        .map_err(AppError::from)?;
    if !revoked {
        return Err(AppError::not_found());
    }
    Audit::new(action::TOKEN_REVOKED)
        .actor(actor.id, &actor.role)
        .target(target::TOKEN, id)
        .ip_hash(ip_hash.map(str::to_string))
        .record(pool)
        .await;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validates_names() {
        assert_eq!(
            validate_name("  ci pipeline ").expect("valid"),
            "ci pipeline"
        );
        assert!(validate_name("").is_err());
        assert!(validate_name("   ").is_err());
        assert!(validate_name(&"x".repeat(MAX_NAME_LEN + 1)).is_err());
        assert!(validate_name("bad\nname").is_err());
    }

    #[test]
    fn validates_expiry() {
        assert_eq!(validate_expiry(None).expect("none"), None);
        assert_eq!(validate_expiry(Some("  ")).expect("blank"), None);
        assert!(validate_expiry(Some("2030-01-01"))
            .expect("future")
            .is_some());
        assert!(validate_expiry(Some("2000-01-01")).is_err());
        assert!(validate_expiry(Some("nope")).is_err());
    }
}
