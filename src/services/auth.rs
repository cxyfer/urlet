//! Authentication flows: registration, login, sessions, password changes.

use sqlx::SqlitePool;

use crate::config::Config;
use crate::db::sessions::Session;
use crate::db::users::User;
use crate::db::{invites, login_attempts, sessions, users};
use crate::error::{AppError, Result};
use crate::services::audit::{action, target, Audit};
use crate::services::settings::{RegistrationMode, Settings};
use crate::services::{password, secret, time};

/// Refresh `last_seen_at` (and slide the expiry) at most once per minute.
const SESSION_TOUCH_INTERVAL_SECS: i64 = 60;
/// Window used to count failed sign-in attempts per identifier.
const LOCKOUT_WINDOW_MINUTES: i64 = 15;
/// Failed attempts in that window before the account is temporarily locked.
const LOCKOUT_THRESHOLD: i64 = 30;

/// Everything an authentication flow needs from the request context.
pub struct AuthContext<'a> {
    pub pool: &'a SqlitePool,
    pub config: &'a Config,
    pub settings: &'a Settings,
    pub user_agent: Option<&'a str>,
    pub ip_hash: Option<&'a str>,
}

/// A freshly minted session: the plain token is returned exactly once.
#[derive(Debug, Clone)]
pub struct SessionToken {
    pub session_id: i64,
    pub token: String,
    pub expires_at: String,
    pub max_age_secs: i64,
}

/// Normalise an email address for storage and lookup.
pub fn normalize_email(email: &str) -> String {
    email.trim().to_lowercase()
}

/// Minimal, deliberately permissive email validation.
///
/// Only ASCII addresses are accepted: the address is used as a login handle and
/// as an audit identifier, so a predictable canonical form matters more than
/// exotic local parts.
pub fn validate_email(email: &str) -> Result<()> {
    let email = email.trim();
    if email.is_empty() || email.len() > 254 {
        return Err(AppError::validation(
            "email",
            "a valid email address is required",
        ));
    }
    if email.contains(char::is_whitespace) {
        return Err(AppError::validation(
            "email",
            "email must not contain spaces",
        ));
    }
    if email.matches('@').count() != 1 {
        return Err(AppError::validation(
            "email",
            "email must contain exactly one @",
        ));
    }
    let Some((local, domain)) = email.split_once('@') else {
        return Err(AppError::validation("email", "email must contain @"));
    };
    if local.is_empty() || local.len() > 64 {
        return Err(AppError::validation(
            "email",
            "the part before @ is invalid",
        ));
    }
    if !local
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || "!#$%&'*+-/=?^_`{|}~.".contains(c))
    {
        return Err(AppError::validation(
            "email",
            "the part before @ is invalid",
        ));
    }
    if domain.is_empty() || !domain.contains('.') {
        return Err(AppError::validation(
            "email",
            "a valid email address is required",
        ));
    }
    if !domain
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '.')
    {
        return Err(AppError::validation("email", "the domain is invalid"));
    }
    if domain.starts_with('.')
        || domain.ends_with('.')
        || domain.contains("..")
        || domain
            .split('.')
            .any(|label| label.is_empty() || label.starts_with('-') || label.ends_with('-'))
    {
        return Err(AppError::validation(
            "email",
            "a valid email address is required",
        ));
    }
    Ok(())
}

/// Validate an optional display name.
pub fn validate_display_name(name: &str) -> Result<()> {
    let length = name.chars().count();
    if length > 60 {
        return Err(AppError::validation(
            "display_name",
            "display name must be at most 60 characters",
        ));
    }
    if name.contains('\n') || name.contains('\r') {
        return Err(AppError::validation(
            "display_name",
            "display name must be a single line",
        ));
    }
    Ok(())
}

impl AuthContext<'_> {
    fn identifier_hash(&self, email_normalized: &str) -> String {
        secret::hmac_hex(&self.config.secret, email_normalized)
    }

    /// Create an account, honouring the registration mode and invite codes.
    pub async fn register(
        &self,
        email: &str,
        password: &str,
        invite_code: Option<&str>,
        display_name: Option<&str>,
    ) -> Result<User> {
        validate_email(email)?;
        password::validate(password)?;
        if let Some(name) = display_name {
            validate_display_name(name)?;
        }

        let email = email.trim().to_string();
        let email_normalized = normalize_email(&email);
        let existing = users::find_by_email(self.pool, &email_normalized)
            .await
            .map_err(AppError::from)?;
        if existing.is_some() {
            return Err(AppError::conflict(
                "an account with this email already exists",
            ));
        }

        // The very first account bootstraps the instance as administrator.
        let is_first_user = users::count(self.pool).await.map_err(AppError::from)? == 0;

        let mode = self.settings.registration_mode();
        if mode == RegistrationMode::Closed {
            return Err(AppError::forbidden(
                "registration is disabled on this instance",
            ));
        }

        // A code supplied in open mode is still validated: it is how an invite
        // can pin the new account to a group even when registration is open.
        let submitted_code = invite_code.map(str::trim).filter(|code| !code.is_empty());
        let invite = match submitted_code {
            Some(code) => {
                let code_hash = secret::sha256_hex(code);
                let invite = invites::find_active_by_hash(self.pool, &code_hash, &time::now_iso())
                    .await
                    .map_err(AppError::from)?;
                match invite {
                    Some(invite) => Some(invite),
                    None => return Err(AppError::forbidden("this invite code is not valid")),
                }
            }
            None if mode == RegistrationMode::Invite => {
                return Err(AppError::forbidden(
                    "an invite code is required to register",
                ))
            }
            None => None,
        };

        let password_hash = password::hash_async(password.to_string()).await?;
        let role = if is_first_user { "admin" } else { "user" };
        // An invite code may pin the group; otherwise the default group wins.
        let group_id = match invite.as_ref().and_then(|invite| invite.group_id) {
            Some(group_id) => Some(group_id),
            None => crate::db::user_groups::default_id(self.pool)
                .await
                .map_err(AppError::from)?,
        };
        let user_id = users::create(
            self.pool,
            &users::NewUser {
                email: email.clone(),
                email_normalized: email_normalized.clone(),
                password_hash,
                display_name: display_name
                    .map(str::to_string)
                    .filter(|name| !name.is_empty()),
                role: role.to_string(),
                group_id,
            },
        )
        .await
        .map_err(|err| {
            // The unique index is the last line of defence against a race.
            if is_unique_violation(&err) {
                AppError::conflict("an account with this email already exists")
            } else {
                AppError::from(err)
            }
        })?;

        if let Some(invite) = invite {
            let redeemed = invites::redeem(self.pool, invite.id, user_id, &time::now_iso())
                .await
                .map_err(AppError::from)?;
            if !redeemed {
                // Extremely unlikely: another request consumed the last use.
                let _ = users::soft_delete(self.pool, user_id).await;
                return Err(AppError::forbidden("this invite code is no longer valid"));
            }
            Audit::new(action::INVITE_REDEEMED)
                .actor(user_id, role)
                .target(target::INVITE, invite.id)
                .ip_hash(self.ip_hash.map(str::to_string))
                .record(self.pool)
                .await;
        }

        let user = users::find_by_id(self.pool, user_id)
            .await
            .map_err(AppError::from)?
            .ok_or_else(|| AppError::internal("newly created user disappeared"))?;

        Audit::new(action::AUTH_REGISTER)
            .actor(user.id, &user.role)
            .target(target::USER, user.id)
            .ip_hash(self.ip_hash.map(str::to_string))
            .record(self.pool)
            .await;

        if is_first_user {
            tracing::info!(
                user_id = user.id,
                "first account created and promoted to administrator"
            );
        }

        Ok(user)
    }

    /// Verify credentials and open a session.
    pub async fn login(&self, email: &str, password: &str) -> Result<(User, SessionToken)> {
        // Bound the work before hashing anything: the caller controls this
        // string and it feeds both the limiter key and the audit hash.
        if email.len() > 254 {
            return Err(AppError::unauthorized("invalid email or password"));
        }
        let email_normalized = normalize_email(email);
        let identifier_hash = self.identifier_hash(&email_normalized);

        let user = users::find_by_email(self.pool, &email_normalized)
            .await
            .map_err(AppError::from)?;

        // A distributed guessing attempt would otherwise walk through the whole
        // account space: per-IP limits do not help against a botnet, so failed
        // attempts for one identifier are also counted in the database.
        let failures = login_attempts::count_failures_since(
            self.pool,
            &identifier_hash,
            &time::iso_minutes_ago(LOCKOUT_WINDOW_MINUTES),
        )
        .await
        .unwrap_or(0);
        if failures >= LOCKOUT_THRESHOLD {
            tracing::warn!(
                failures,
                "account temporarily locked after repeated failures"
            );
            return Err(AppError::rate_limited((LOCKOUT_WINDOW_MINUTES * 60) as u64));
        }

        let Some(user) = user else {
            password::dummy_verify_async(password.to_string()).await;
            self.record_attempt(&identifier_hash, false).await;
            return Err(AppError::unauthorized("invalid email or password"));
        };

        if !password::verify_async(password.to_string(), user.password_hash.clone()).await {
            self.record_attempt(&identifier_hash, false).await;
            Audit::new(action::AUTH_LOGIN_FAILED)
                .actor(user.id, &user.role)
                .target(target::USER, user.id)
                .ip_hash(self.ip_hash.map(str::to_string))
                .record(self.pool)
                .await;
            return Err(AppError::unauthorized("invalid email or password"));
        }

        if user.deleted_at.is_some() {
            self.record_attempt(&identifier_hash, false).await;
            return Err(AppError::unauthorized("invalid email or password"));
        }
        if user.status != "active" {
            self.record_attempt(&identifier_hash, false).await;
            return Err(AppError::forbidden("this account is suspended"));
        }

        self.record_attempt(&identifier_hash, true).await;
        users::set_last_login(self.pool, user.id)
            .await
            .map_err(AppError::from)?;
        let token = self.open_session(user.id).await?;

        Audit::new(action::AUTH_LOGIN)
            .actor(user.id, &user.role)
            .target(target::USER, user.id)
            .ip_hash(self.ip_hash.map(str::to_string))
            .metadata(serde_json::json!({ "session_id": token.session_id }))
            .record(self.pool)
            .await;

        // `last_login_at` was just written; reload so the response is accurate.
        let user = users::find_by_id(self.pool, user.id)
            .await
            .map_err(AppError::from)?
            .unwrap_or(user);
        Ok((user, token))
    }

    /// Create a session for an already authenticated user.
    pub async fn open_session(&self, user_id: i64) -> Result<SessionToken> {
        let token = secret::random_token(32);
        let token_hash = secret::sha256_hex(&token);
        let created_at = time::now_iso();
        let expires_at = time::iso_in_hours(self.config.session_ttl_hours);
        let session_id = sessions::create(
            self.pool,
            user_id,
            &token_hash,
            self.user_agent,
            self.ip_hash,
            &created_at,
            &expires_at,
        )
        .await
        .map_err(AppError::from)?;

        Ok(SessionToken {
            session_id,
            token,
            expires_at,
            max_age_secs: self.config.session_ttl_hours * 3600,
        })
    }

    /// Revoke a single session.
    pub async fn logout(&self, session_id: i64, user: &User) -> Result<()> {
        sessions::revoke(self.pool, session_id, &time::now_iso())
            .await
            .map_err(AppError::from)?;
        Audit::new(action::AUTH_LOGOUT)
            .actor(user.id, &user.role)
            .target(target::USER, user.id)
            .metadata(serde_json::json!({ "session_id": session_id }))
            .ip_hash(self.ip_hash.map(str::to_string))
            .record(self.pool)
            .await;
        Ok(())
    }

    /// Change the password of an authenticated user.
    ///
    /// Every other session is revoked; the current one can be preserved.
    pub async fn change_password(
        &self,
        user: &User,
        current_password: &str,
        new_password: &str,
        keep_session: Option<i64>,
    ) -> Result<()> {
        if !password::verify_async(current_password.to_string(), user.password_hash.clone()).await {
            return Err(AppError::unauthorized("current password is incorrect"));
        }
        password::validate(new_password)?;
        if password::verify_async(new_password.to_string(), user.password_hash.clone()).await {
            return Err(AppError::validation(
                "new_password",
                "the new password must differ from the current one",
            ));
        }

        let hash = password::hash_async(new_password.to_string()).await?;
        users::set_password(self.pool, user.id, &hash)
            .await
            .map_err(AppError::from)?;
        let revoked = sessions::revoke_for_user(self.pool, user.id, &time::now_iso(), keep_session)
            .await
            .map_err(AppError::from)?;
        // API tokens are credentials too: changing the password must invalidate
        // them, otherwise an exfiltrated token keeps working.
        let revoked_tokens =
            crate::db::tokens::revoke_all_for_user(self.pool, user.id, &time::now_iso())
                .await
                .map_err(AppError::from)?;

        Audit::new(action::AUTH_PASSWORD_CHANGED)
            .actor(user.id, &user.role)
            .target(target::USER, user.id)
            .metadata(serde_json::json!({
                "revoked_sessions": revoked,
                "revoked_tokens": revoked_tokens,
            }))
            .ip_hash(self.ip_hash.map(str::to_string))
            .record(self.pool)
            .await;
        Ok(())
    }

    async fn record_attempt(&self, identifier_hash: &str, success: bool) {
        if let Err(err) = login_attempts::record(
            self.pool,
            identifier_hash,
            self.ip_hash,
            success,
            &time::now_iso(),
        )
        .await
        {
            tracing::warn!(error = %err, "cannot record login attempt");
        }
    }
}

/// Resolve a session token to a session and its user.
///
/// `session_ttl_hours` is used to slide the expiry forward on activity.
pub async fn resolve_session(
    pool: &SqlitePool,
    token: &str,
    session_ttl_hours: i64,
) -> Result<Option<(Session, User)>> {
    let token_hash = secret::sha256_hex(token);
    let Some(session) = sessions::find_by_token_hash(pool, &token_hash)
        .await
        .map_err(AppError::from)?
    else {
        return Ok(None);
    };
    let now = time::now_iso();
    if !session.is_valid_at(&now) {
        return Ok(None);
    }
    let Some(user) = users::find_by_id(pool, session.user_id)
        .await
        .map_err(AppError::from)?
    else {
        return Ok(None);
    };
    if !user.is_active() {
        return Ok(None);
    }

    // Sliding session: refresh `last_seen_at` and push `expires_at` forward,
    // written at most once per minute.
    let stale_before = time::iso_seconds_ago(SESSION_TOUCH_INTERVAL_SECS);
    if session.last_seen_at < stale_before {
        let expires_at = time::iso_in_hours(session_ttl_hours);
        let _ = sessions::touch(pool, session.id, &stale_before, &now, &expires_at).await;
    }

    Ok(Some((session, user)))
}

/// Resolve an API token to its record and owner.
pub async fn resolve_api_token(
    pool: &SqlitePool,
    token: &str,
) -> Result<Option<(crate::db::tokens::ApiToken, User)>> {
    let token_hash = secret::sha256_hex(token);
    let Some(record) = crate::db::tokens::find_by_hash(pool, &token_hash)
        .await
        .map_err(AppError::from)?
    else {
        return Ok(None);
    };
    if !record.is_valid_at(&time::now_iso()) {
        return Ok(None);
    }
    let Some(user) = users::find_by_id(pool, record.user_id)
        .await
        .map_err(AppError::from)?
    else {
        return Ok(None);
    };
    if !user.is_active() {
        return Ok(None);
    }
    let _ = crate::db::tokens::touch_last_used(pool, record.id, &time::now_iso()).await;
    Ok(Some((record, user)))
}

/// SQLite reports constraint violations as a database error with code 2067.
fn is_unique_violation(err: &sqlx::Error) -> bool {
    matches!(err, sqlx::Error::Database(db) if db.code().as_deref() == Some("2067"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalises_and_validates_emails() {
        assert_eq!(normalize_email("  User@Example.COM "), "user@example.com");
        assert!(validate_email("user@example.com").is_ok());
        assert!(validate_email("user@example").is_err());
        assert!(validate_email("no-at-sign").is_err());
        assert!(validate_email("two@@example.com").is_err());
        assert!(validate_email("spaces in@example.com").is_err());
        assert!(validate_email(&format!("{}@example.com", "a".repeat(260))).is_err());
    }

    #[test]
    fn validates_display_names() {
        assert!(validate_display_name("Fine Name").is_ok());
        assert!(validate_display_name("").is_ok());
        assert!(validate_display_name("line\nbreak").is_err());
        assert!(validate_display_name(&"x".repeat(61)).is_err());
    }
}
