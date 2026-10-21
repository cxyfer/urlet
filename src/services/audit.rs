//! Audit logging.
//!
//! Audit writes must never break a user visible action, so failures are logged
//! and swallowed.

use sqlx::SqlitePool;

use crate::db::audit::{AuditFilter, AuditLog, NewAuditLog};
use crate::services::time::now_iso;

/// Known audit actions.
pub mod action {
    pub const AUTH_REGISTER: &str = "auth.register";
    pub const AUTH_LOGIN: &str = "auth.login";
    pub const AUTH_LOGIN_FAILED: &str = "auth.login_failed";
    pub const AUTH_LOGOUT: &str = "auth.logout";
    pub const AUTH_PASSWORD_CHANGED: &str = "auth.password_changed";
    pub const AUTH_SESSION_REVOKED: &str = "auth.session_revoked";
    pub const PROFILE_UPDATED: &str = "user.profile_updated";
    pub const URL_CREATED: &str = "url.created";
    pub const URL_UPDATED: &str = "url.updated";
    pub const URL_DELETED: &str = "url.deleted";
    pub const TOKEN_CREATED: &str = "token.created";
    pub const TOKEN_REVOKED: &str = "token.revoked";
    pub const INVITE_CREATED: &str = "invite.created";
    pub const INVITE_REVOKED: &str = "invite.revoked";
    pub const INVITE_REDEEMED: &str = "invite.redeemed";
    pub const ADMIN_USER_UPDATED: &str = "admin.user_updated";
    pub const ADMIN_USER_DELETED: &str = "admin.user_deleted";
    pub const ADMIN_PASSWORD_RESET: &str = "admin.password_reset";
    pub const ADMIN_URL_UPDATED: &str = "admin.url_updated";
    pub const ADMIN_SETTINGS_UPDATED: &str = "admin.settings_updated";
    pub const DOMAIN_CREATED: &str = "domain.created";
    pub const DOMAIN_UPDATED: &str = "domain.updated";
    pub const DOMAIN_DELETED: &str = "domain.deleted";
    pub const DOMAIN_DEFAULT_CHANGED: &str = "domain.default_changed";
    pub const ADMIN_GROUP_CREATED: &str = "admin.group_created";
    pub const ADMIN_GROUP_UPDATED: &str = "admin.group_updated";
    pub const ADMIN_GROUP_DELETED: &str = "admin.group_deleted";
    pub const ADMIN_THEMES_UPDATED: &str = "admin.themes_updated";
}

/// Target types used in audit entries.
pub mod target {
    pub const USER: &str = "user";
    pub const URL: &str = "url";
    pub const TOKEN: &str = "api_token";
    pub const INVITE: &str = "invite";
    pub const SETTINGS: &str = "settings";
    pub const DOMAIN: &str = "domain";
    pub const GROUP: &str = "group";
    pub const THEMES: &str = "themes";
}

/// Builder for an audit entry.
#[derive(Debug, Default)]
pub struct Audit {
    entry: NewAuditLog,
}

impl Audit {
    pub fn new(action: &str) -> Self {
        Self {
            entry: NewAuditLog {
                action: action.to_string(),
                ..Default::default()
            },
        }
    }

    pub fn actor(mut self, user_id: i64, role: &str) -> Self {
        self.entry.actor_user_id = Some(user_id);
        self.entry.actor_role = Some(role.to_string());
        self
    }

    pub fn actor_opt(mut self, user_id: Option<i64>, role: Option<&str>) -> Self {
        self.entry.actor_user_id = user_id;
        self.entry.actor_role = role.map(str::to_string);
        self
    }

    pub fn target(mut self, target_type: &str, target_id: impl std::fmt::Display) -> Self {
        self.entry.target_type = Some(target_type.to_string());
        self.entry.target_id = Some(target_id.to_string());
        self
    }

    pub fn metadata(mut self, value: serde_json::Value) -> Self {
        self.entry.metadata = serde_json::to_string(&value).ok();
        self
    }

    pub fn ip_hash(mut self, ip_hash: Option<String>) -> Self {
        self.entry.ip_hash = ip_hash;
        self
    }

    /// Persist the entry. Errors are logged, never propagated.
    pub async fn record(self, pool: &SqlitePool) {
        if let Err(err) = crate::db::audit::insert(pool, &self.entry).await {
            tracing::warn!(error = %err, action = %self.entry.action, "audit write failed");
        }
    }
}

/// Read audit entries (admin UI).
pub async fn list(
    pool: &SqlitePool,
    filter: &AuditFilter,
    limit: i64,
    offset: i64,
) -> Result<Vec<AuditLog>, sqlx::Error> {
    crate::db::audit::list(pool, filter, limit, offset).await
}

/// Count audit entries (admin UI).
pub async fn count(pool: &SqlitePool, filter: &AuditFilter) -> Result<i64, sqlx::Error> {
    crate::db::audit::count(pool, filter).await
}

/// Retention: drop entries older than `days`.
pub async fn prune(pool: &SqlitePool, days: i64) -> Result<u64, sqlx::Error> {
    if days <= 0 {
        return Ok(0);
    }
    let before = crate::services::time::iso_days_ago(days);
    let deleted = crate::db::audit::delete_older_than(pool, &before).await?;
    if deleted > 0 {
        tracing::info!(deleted, "pruned audit log");
    }
    Ok(deleted)
}

/// Convenience: current timestamp for audit metadata.
pub fn now() -> String {
    now_iso()
}
