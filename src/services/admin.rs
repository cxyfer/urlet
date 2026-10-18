//! Administrative operations: user management, invites, settings and reports.
//!
//! Every mutation is audited and guarded by the invariants that keep an instance
//! administrable (never lock out the last active administrator).

use std::sync::Arc;

use sqlx::SqlitePool;

use crate::config::Config;
use crate::db::invites::Invite;
use crate::db::urls::{self, UrlUpdate};
use crate::db::user_groups::{self, UserGroup};
use crate::db::users::{self, User, UserFilter};
use crate::db::{
    audit, clicks, domains as domain_db, invites, sessions, settings as settings_db, tokens,
};
use crate::error::{AppError, Result};
use crate::services::audit::{action, target, Audit};
use crate::services::groups::GroupDraft;
use crate::services::settings::Settings;
use crate::services::{password, secret, time, tokens as token_service};

/// Longest accepted invite note.
pub const MAX_INVITE_NOTE: usize = 120;
/// Invite codes are 10 characters from an unambiguous alphabet.
const INVITE_CODE_LEN: usize = 10;

/// A freshly created invite: the plain code is returned exactly once.
#[derive(Debug, Clone)]
pub struct CreatedInvite {
    pub invite: Invite,
    pub plain_code: String,
}

/// Instance level statistics for the admin dashboard.
#[derive(Debug, Clone, serde::Serialize)]
pub struct SystemStats {
    pub version: &'static str,
    pub uptime_seconds: i64,
    pub users: i64,
    pub admins: i64,
    pub links: i64,
    pub total_clicks: i64,
    pub unique_clicks: i64,
    pub bot_clicks: i64,
    pub expired_links: i64,
    pub active_invites: i64,
    pub active_tokens: i64,
    pub database_bytes: Option<u64>,
    pub registration_mode: String,
}

/// Everything an admin action needs from the request context.
pub struct AdminContext<'a> {
    pub pool: &'a SqlitePool,
    pub config: &'a Arc<Config>,
    pub settings: &'a Settings,
    pub actor: &'a User,
    pub ip_hash: Option<&'a str>,
    /// Process start time, used for the uptime figure.
    pub started_at: time::OffsetDateTime,
}

impl AdminContext<'_> {
    fn audit(&self, action: &str, target_type: &str, target_id: i64) -> Audit {
        Audit::new(action)
            .actor(self.actor.id, &self.actor.role)
            .target(target_type, target_id)
            .ip_hash(self.ip_hash.map(str::to_string))
    }

    /// Update role, status, group and display name of an account.
    pub async fn update_user(
        &self,
        user_id: i64,
        role: Option<&str>,
        status: Option<&str>,
        group_id: Option<Option<i64>>,
        display_name: Option<Option<&str>>,
    ) -> Result<User> {
        let user = users::find_by_id(self.pool, user_id)
            .await
            .map_err(AppError::from)?
            .ok_or_else(AppError::not_found)?;

        // Keep the request's own options: the write must not replay values that
        // were only read to resolve guard rails.
        let requested_role = role;
        let requested_status = status;
        let requested_group = group_id;

        let role = match role {
            Some(role) => {
                let role = role.trim().to_ascii_lowercase();
                if role != "user" && role != "admin" {
                    return Err(AppError::validation(
                        "role",
                        "role must be 'user' or 'admin'",
                    ));
                }
                role
            }
            None => user.role.clone(),
        };
        let status = match status {
            Some(status) => {
                let status = status.trim().to_ascii_lowercase();
                if status != "active" && status != "suspended" {
                    return Err(AppError::validation(
                        "status",
                        "status must be 'active' or 'suspended'",
                    ));
                }
                status
            }
            None => user.status.clone(),
        };
        let group_id = match group_id {
            Some(Some(group_id)) => {
                if user_groups::find_by_id(self.pool, group_id)
                    .await
                    .map_err(AppError::from)?
                    .is_none()
                {
                    return Err(AppError::validation(
                        "group_id",
                        "the selected group does not exist",
                    ));
                }
                Some(group_id)
            }
            Some(None) => None,
            None => user.group_id,
        };

        // Validate every submitted field before the first write: a rejected
        // display name must not leave the group or role partially changed.
        let display_name = match display_name {
            Some(name) => {
                let name = name.map(str::trim).filter(|value| !value.is_empty());
                if let Some(name) = name {
                    crate::services::auth::validate_display_name(name)?;
                }
                Some(name)
            }
            None => None,
        };

        // Guard rails: an administrator must not lock themselves out.
        let demoting_self = user.id == self.actor.id && (role != "admin" || status != "active");
        if demoting_self {
            return Err(AppError::validation(
                "role",
                "you cannot change your own role or status",
            ));
        }
        let losing_an_admin = user.role == "admin"
            && user.status == "active"
            && (role != "admin" || status != "active");
        if losing_an_admin {
            let others = users::count_active_admins(self.pool, Some(user.id))
                .await
                .map_err(AppError::from)?;
            if others == 0 {
                return Err(AppError::conflict(
                    "at least one active administrator is required",
                ));
            }
        }

        let update = users::AdminUserUpdate {
            // Keep omission (None) separate, but persist the canonical values:
            // " Admin " and "SUSPENDED" are valid input and must be stored as
            // "admin" / "suspended".
            role: requested_role.map(|_| role.as_str()),
            status: requested_status.map(|_| status.as_str()),
            group_id: requested_group,
            display_name,
        };
        let applied = users::apply_admin_update(self.pool, user.id, &update)
            .await
            .map_err(|err| {
                if crate::db::is_foreign_key_violation(&err) {
                    AppError::validation("group_id", "the selected group no longer exists")
                } else {
                    AppError::from(err)
                }
            })?;
        if !applied {
            // The row vanished, or the conditional update refused to remove the
            // last active administrator. Distinguish the two after the rollback.
            return match users::find_by_id(self.pool, user.id)
                .await
                .map_err(AppError::from)?
            {
                Some(_) => Err(AppError::conflict(
                    "at least one active administrator is required",
                )),
                None => Err(AppError::not_found()),
            };
        }

        self.audit(action::ADMIN_USER_UPDATED, target::USER, user.id)
            .metadata(serde_json::json!({
                "role": role,
                "status": status,
                "group_id": group_id,
                "previous_group_id": user.group_id
            }))
            .record(self.pool)
            .await;

        users::find_by_id(self.pool, user.id)
            .await
            .map_err(AppError::from)?
            .ok_or_else(AppError::not_found)
    }

    /// Reset another account's password.
    pub async fn reset_password(&self, user_id: i64, new_password: &str) -> Result<()> {
        let user = users::find_by_id(self.pool, user_id)
            .await
            .map_err(AppError::from)?
            .ok_or_else(AppError::not_found)?;
        let hash = password::hash_async(new_password.to_string()).await?;
        users::set_password(self.pool, user.id, &hash)
            .await
            .map_err(AppError::from)?;
        sessions::revoke_for_user(self.pool, user.id, &time::now_iso(), None)
            .await
            .map_err(AppError::from)?;
        // A reset must also invalidate every API token of that account.
        crate::db::tokens::revoke_all_for_user(self.pool, user.id, &time::now_iso())
            .await
            .map_err(AppError::from)?;

        self.audit(action::ADMIN_PASSWORD_RESET, target::USER, user.id)
            .record(self.pool)
            .await;
        Ok(())
    }

    /// Soft delete an account together with its links.
    pub async fn delete_user(&self, user_id: i64) -> Result<()> {
        let user = users::find_by_id(self.pool, user_id)
            .await
            .map_err(AppError::from)?
            .ok_or_else(AppError::not_found)?;
        if user.id == self.actor.id {
            return Err(AppError::validation(
                "id",
                "you cannot delete your own account",
            ));
        }
        if user.role == "admin" && user.status == "active" {
            let others = users::count_active_admins(self.pool, Some(user.id))
                .await
                .map_err(AppError::from)?;
            if others == 0 {
                return Err(AppError::conflict(
                    "at least one active administrator is required",
                ));
            }
        }

        let deleted = users::soft_delete(self.pool, user.id)
            .await
            .map_err(AppError::from)?;
        if !deleted {
            return match users::find_by_id(self.pool, user.id)
                .await
                .map_err(AppError::from)?
            {
                Some(_) => Err(AppError::conflict(
                    "at least one active administrator is required",
                )),
                // Another administrator already removed the account.
                None => Ok(()),
            };
        }
        self.audit(action::ADMIN_USER_DELETED, target::USER, user.id)
            .metadata(serde_json::json!({ "email": user.email }))
            .record(self.pool)
            .await;
        Ok(())
    }

    /// Force a status change on someone else's link.
    pub async fn update_url(
        &self,
        url_id: i64,
        status: Option<&str>,
        redirect_type: Option<i64>,
    ) -> Result<()> {
        let url = urls::find_by_id(self.pool, url_id)
            .await
            .map_err(AppError::from)?
            .ok_or_else(AppError::not_found)?;

        let mut changes = UrlUpdate::default();
        if let Some(status) = status {
            let status = status.trim().to_ascii_lowercase();
            if status != "active" && status != "disabled" {
                return Err(AppError::validation(
                    "status",
                    "status must be 'active' or 'disabled'",
                ));
            }
            changes.status = Some(status);
        }
        if let Some(code) = redirect_type {
            changes.redirect_type = Some(crate::services::urls::validate_redirect_type(code)?);
        }
        if changes.is_empty() {
            return Err(AppError::bad_request("no changes were submitted"));
        }

        let affected = urls::update(self.pool, url.id, None, &changes)
            .await
            .map_err(AppError::from)?;
        if affected == 0 {
            return Err(AppError::not_found());
        }

        self.audit(action::ADMIN_URL_UPDATED, target::URL, url.id)
            .metadata(serde_json::json!({
                "slug": url.slug,
                "user_id": url.user_id,
                "status": changes.status,
            }))
            .record(self.pool)
            .await;
        Ok(())
    }

    /// Delete any link.
    pub async fn delete_url(&self, url_id: i64) -> Result<()> {
        let url = urls::find_by_id(self.pool, url_id)
            .await
            .map_err(AppError::from)?
            .ok_or_else(AppError::not_found)?;
        urls::soft_delete(self.pool, url.id, None)
            .await
            .map_err(AppError::from)?;
        self.audit(action::URL_DELETED, target::URL, url.id)
            .metadata(serde_json::json!({ "slug": url.slug, "by_admin": true }))
            .record(self.pool)
            .await;
        Ok(())
    }

    /// Create an invite code. `group_id` optionally pins the redeemer's group.
    pub async fn create_invite(
        &self,
        note: Option<&str>,
        group_id: Option<i64>,
        max_uses: Option<i64>,
        expires_at: Option<&str>,
    ) -> Result<CreatedInvite> {
        let max_uses = match max_uses {
            Some(value) if !(1..=10_000).contains(&value) => {
                return Err(AppError::validation(
                    "max_uses",
                    "uses must be between 1 and 10000",
                ))
            }
            Some(value) => value,
            None => 1,
        };
        let group_id = match group_id {
            Some(group_id) => {
                if user_groups::find_by_id(self.pool, group_id)
                    .await
                    .map_err(AppError::from)?
                    .is_none()
                {
                    return Err(AppError::validation(
                        "group_id",
                        "the selected group does not exist",
                    ));
                }
                Some(group_id)
            }
            None => None,
        };
        let note = match note.map(str::trim).filter(|value| !value.is_empty()) {
            Some(note) => {
                if note.chars().count() > MAX_INVITE_NOTE {
                    return Err(AppError::validation(
                        "note",
                        format!("the note must be at most {MAX_INVITE_NOTE} characters"),
                    ));
                }
                Some(note.to_string())
            }
            None => None,
        };
        let expires_at = match expires_at.map(str::trim).filter(|value| !value.is_empty()) {
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
                Some(time::to_iso(parsed))
            }
            None => None,
        };

        let plain_code = secret::random_code(INVITE_CODE_LEN);
        let code_hash = secret::sha256_hex(&plain_code);
        let prefix: String = plain_code.chars().take(4).collect();
        let id = invites::create(
            self.pool,
            &crate::db::invites::NewInvite {
                code_hash: &code_hash,
                code_prefix: &prefix,
                note: note.as_deref(),
                group_id,
                created_by: Some(self.actor.id),
                max_uses,
                expires_at: expires_at.as_deref(),
            },
        )
        .await
        .map_err(|err| {
            // The selected group may have been deleted after the check above.
            if crate::db::is_foreign_key_violation(&err) {
                AppError::validation("group_id", "the selected group no longer exists")
            } else {
                AppError::from(err)
            }
        })?;

        let invite = invites::find_by_id(self.pool, id)
            .await
            .map_err(AppError::from)?
            .ok_or_else(|| AppError::internal("invite disappeared after creation"))?;

        self.audit(action::INVITE_CREATED, target::INVITE, id)
            .metadata(serde_json::json!({
                "max_uses": max_uses,
                "prefix": prefix,
                "group_id": group_id
            }))
            .record(self.pool)
            .await;

        Ok(CreatedInvite { invite, plain_code })
    }

    /// Revoke an invite code.
    pub async fn revoke_invite(&self, invite_id: i64) -> Result<()> {
        let revoked = invites::revoke(self.pool, invite_id)
            .await
            .map_err(AppError::from)?;
        if !revoked {
            return Err(AppError::not_found());
        }
        self.audit(action::INVITE_REVOKED, target::INVITE, invite_id)
            .record(self.pool)
            .await;
        Ok(())
    }

    /// Create a user group.
    async fn validate_group_domains(&self, draft: &GroupDraft) -> Result<()> {
        if let Some(id) = draft.default_domain_id {
            self.require_active_group_domain(id, "default_domain_id")
                .await?;
        }
        if let Some(ids) = &draft.allowed_domain_ids {
            for id in ids {
                self.require_active_group_domain(*id, "domain_ids").await?;
            }
        }
        Ok(())
    }

    async fn require_active_group_domain(&self, id: i64, field: &'static str) -> Result<()> {
        let domain = domain_db::find_by_id(self.pool, id)
            .await
            .map_err(AppError::from)?
            .ok_or_else(|| AppError::validation(field, "unknown domain"))?;
        if !domain.is_active() {
            return Err(AppError::validation(field, "the domain must be active"));
        }
        Ok(())
    }

    pub async fn create_group(&self, draft: GroupDraft) -> Result<UserGroup> {
        self.validate_group_domains(&draft).await?;
        if user_groups::find_by_name(self.pool, &draft.name)
            .await
            .map_err(AppError::from)?
            .is_some()
        {
            return Err(AppError::conflict("a group with that name already exists"));
        }
        if draft.make_default
            && user_groups::default_group(self.pool)
                .await
                .map_err(AppError::from)?
                .is_some()
        {
            return Err(AppError::conflict(
                "a default group already exists; edit an existing group to move the marker",
            ));
        }

        let allowed_domain_ids = draft.allowed_domain_ids.clone().unwrap_or_default();
        let id = user_groups::create(
            self.pool,
            &draft.values(),
            draft.make_default,
            &allowed_domain_ids,
        )
        .await
        .map_err(new_group_write_error)?;
        let group = user_groups::find_by_id(self.pool, id)
            .await
            .map_err(AppError::from)?
            .ok_or_else(|| AppError::internal("group disappeared after creation"))?;

        self.audit(action::ADMIN_GROUP_CREATED, target::GROUP, id)
            .metadata(serde_json::json!({
                "name": &group.name,
                "max_urls": group.max_urls,
                "max_api_tokens": group.max_api_tokens,
                "default_domain_id": group.default_domain_id,
                "restrict_domains": group.restrict_domains,
                "is_default": group.is_default
            }))
            .record(self.pool)
            .await;
        Ok(group)
    }

    /// Replace a group's settings and optionally move the default marker.
    pub async fn update_group(&self, id: i64, draft: GroupDraft) -> Result<UserGroup> {
        self.validate_group_domains(&draft).await?;
        let existing = user_groups::find_by_id(self.pool, id)
            .await
            .map_err(AppError::from)?
            .ok_or_else(AppError::not_found)?;
        if let Some(other) = user_groups::find_by_name(self.pool, &draft.name)
            .await
            .map_err(AppError::from)?
        {
            if other.id != id {
                return Err(AppError::conflict("a group with that name already exists"));
            }
        }

        let updated = user_groups::update(
            self.pool,
            id,
            &draft.values(),
            draft.make_default,
            draft.allowed_domain_ids.as_deref(),
        )
        .await
        .map_err(new_group_write_error)?;
        if !updated {
            // The row disappeared after the load above; do not touch the marker.
            return Err(AppError::not_found());
        }
        let group = user_groups::find_by_id(self.pool, id)
            .await
            .map_err(AppError::from)?
            .ok_or_else(|| AppError::internal("group disappeared after update"))?;

        self.audit(action::ADMIN_GROUP_UPDATED, target::GROUP, id)
            .metadata(serde_json::json!({
                "name": &group.name,
                "previous_name": &existing.name,
                "max_urls": group.max_urls,
                "max_api_tokens": group.max_api_tokens,
                "default_domain_id": group.default_domain_id,
                "restrict_domains": group.restrict_domains,
                "is_default": group.is_default
            }))
            .record(self.pool)
            .await;
        Ok(group)
    }

    /// Delete a non-default group after moving all members to `reassign_to`.
    pub async fn delete_group(&self, id: i64, reassign_to: Option<i64>) -> Result<()> {
        let group = user_groups::find_by_id(self.pool, id)
            .await
            .map_err(AppError::from)?
            .ok_or_else(AppError::not_found)?;
        if group.is_default {
            return Err(AppError::conflict("the default group cannot be deleted"));
        }
        let target = reassign_to
            .ok_or_else(|| AppError::conflict("choose a group to move the members to"))?;
        if target == id {
            return Err(AppError::conflict(
                "members cannot be moved to the group being deleted",
            ));
        }
        let target_group = user_groups::find_by_id(self.pool, target)
            .await
            .map_err(AppError::from)?
            .ok_or_else(|| AppError::conflict("the target group does not exist"))?;

        let deleted = user_groups::delete_with_reassign(self.pool, id, target)
            .await
            .map_err(|err| {
                // The reassignment target may have been deleted between the
                // lookup above and the transaction's write lock.
                if crate::db::is_foreign_key_violation(&err) {
                    AppError::conflict("the target group does not exist")
                } else {
                    AppError::from(err)
                }
            })?;
        if !deleted {
            // The marker moved after the service-level check (or another
            // administrator removed the group). The transaction rolled back.
            return match user_groups::find_by_id(self.pool, id)
                .await
                .map_err(AppError::from)?
            {
                Some(_) => Err(AppError::conflict("the default group cannot be deleted")),
                None => Err(AppError::not_found()),
            };
        }

        self.audit(action::ADMIN_GROUP_DELETED, target::GROUP, id)
            .metadata(serde_json::json!({
                "name": &group.name,
                "reassign_to": target,
                "target_name": &target_group.name
            }))
            .record(self.pool)
            .await;
        Ok(())
    }

    /// Update several settings at once.
    ///
    /// The batch is validated as a whole and stored in one transaction: an
    /// invalid value must not leave the instance half reconfigured.
    pub async fn update_settings(&self, values: &[(String, String)]) -> Result<()> {
        if values.is_empty() {
            return Err(AppError::bad_request("no settings were submitted"));
        }
        self.settings
            .update_many(self.pool, values, Some(self.actor.id))
            .await?;
        let keys: Vec<&str> = values.iter().map(|(key, _)| key.as_str()).collect();
        Audit::new(action::ADMIN_SETTINGS_UPDATED)
            .actor(self.actor.id, &self.actor.role)
            .target(target::SETTINGS, "global")
            .metadata(serde_json::json!({ "keys": keys }))
            .ip_hash(self.ip_hash.map(str::to_string))
            .record(self.pool)
            .await;
        Ok(())
    }

    /// Collect the numbers shown on the admin overview.
    pub async fn system_stats(&self) -> Result<SystemStats> {
        let users_total = users::count(self.pool).await.map_err(AppError::from)?;
        let admins = users::count_active_admins(self.pool, None)
            .await
            .map_err(AppError::from)?;
        let (links, _clicks_from_urls, expired) = urls::global_counts(self.pool)
            .await
            .map_err(AppError::from)?;
        let (total_clicks, unique_clicks, bot_clicks) = clicks::global_totals(self.pool)
            .await
            .map_err(AppError::from)?;
        let active_invites = invites::list(self.pool, 500, 0)
            .await
            .map_err(AppError::from)?
            .into_iter()
            .filter(|invite| invite.status == "active" && !invite.is_exhausted())
            .count() as i64;
        let active_tokens = tokens::count_active(self.pool, &time::now_iso())
            .await
            .map_err(AppError::from)?;

        let database_bytes = self.config.sqlite_path().map(|path| database_size(&path));

        Ok(SystemStats {
            version: crate::VERSION,
            uptime_seconds: (time::now() - self.started_at).whole_seconds().max(0),
            users: users_total,
            admins,
            links,
            total_clicks,
            unique_clicks,
            bot_clicks,
            expired_links: expired,
            active_invites,
            active_tokens,
            database_bytes,
            registration_mode: self.settings.registration_mode().as_str().to_string(),
        })
    }
}

/// Map a concurrent name/default collision to the same conflict the pre-checks
/// report, instead of leaking an internal error.
fn new_group_write_error(err: sqlx::Error) -> AppError {
    if crate::db::is_foreign_key_violation(&err) {
        AppError::validation("domain_ids", "a selected domain no longer exists")
    } else {
        map_group_write_error(err)
    }
}

fn map_group_write_error(err: sqlx::Error) -> AppError {
    if crate::services::urls::is_unique_violation(&err) {
        let message = err.to_string();
        if message.contains("is_default") || message.contains("idx_user_groups_single_default") {
            AppError::conflict("a default group already exists")
        } else {
            AppError::conflict("a group with that name already exists")
        }
    } else {
        AppError::from(err)
    }
}

/// Size of the database including its write-ahead log (WAL mode keeps recent
/// writes out of the main file).
pub fn database_size(path: &std::path::Path) -> u64 {
    let mut total = std::fs::metadata(path).map(|meta| meta.len()).unwrap_or(0);
    for suffix in ["-wal", "-shm"] {
        let sidecar = std::path::PathBuf::from(format!("{}{suffix}", path.display()));
        total += std::fs::metadata(sidecar)
            .map(|meta| meta.len())
            .unwrap_or(0);
    }
    total
}

/// Users matching a filter, with their link counts.
pub async fn list_users(
    pool: &SqlitePool,
    filter: &UserFilter,
    limit: i64,
    offset: i64,
) -> Result<Vec<User>> {
    users::list(pool, filter, limit, offset)
        .await
        .map_err(AppError::from)
}

/// Count users matching a filter.
pub async fn count_users(pool: &SqlitePool, filter: &UserFilter) -> Result<i64> {
    users::count_filtered(pool, filter)
        .await
        .map_err(AppError::from)
}

/// Links per user, as a lookup table.
pub async fn link_counts(pool: &SqlitePool) -> Result<std::collections::HashMap<i64, i64>> {
    Ok(users::link_counts(pool)
        .await
        .map_err(AppError::from)?
        .into_iter()
        .collect())
}

/// Audit log page plus total.
pub async fn audit_page(
    pool: &SqlitePool,
    filter: &audit::AuditFilter,
    limit: i64,
    offset: i64,
) -> Result<(Vec<audit::AuditLog>, i64)> {
    let rows = audit::list(pool, filter, limit, offset)
        .await
        .map_err(AppError::from)?;
    let total = audit::count(pool, filter).await.map_err(AppError::from)?;
    Ok((rows, total))
}

/// Current value of every setting.
pub async fn settings_snapshot(settings: &Settings) -> std::collections::HashMap<String, String> {
    settings.snapshot()
}

/// Checkpoint the write-ahead log so the database file stays compact.
pub async fn checkpoint_wal(pool: &SqlitePool) -> Result<()> {
    crate::db::checkpoint_wal(pool).await
}

/// Delete stale sessions and, when configured, old analytics data.
pub async fn run_maintenance(pool: &SqlitePool, settings: &Settings) -> Result<()> {
    let now = time::now_iso();
    let stale_before = time::iso_days_ago(1);
    let sessions_deleted = sessions::delete_stale(pool, &stale_before)
        .await
        .map_err(AppError::from)?;

    let attempts_before = time::iso_days_ago(30);
    let attempts_deleted = crate::db::login_attempts::delete_older_than(pool, &attempts_before)
        .await
        .map_err(AppError::from)?;

    let audit_before = time::iso_days_ago(180);
    let audit_deleted = audit::delete_older_than(pool, &audit_before)
        .await
        .map_err(AppError::from)?;

    let mut clicks_deleted = 0;
    let retention = settings.click_retention_days();
    if retention > 0 {
        let before = time::iso_days_ago(retention);
        clicks_deleted = clicks::delete_older_than(pool, &before)
            .await
            .map_err(AppError::from)?;
        let before_day = time::day_days_ago(retention);
        clicks::delete_daily_before(pool, &before_day)
            .await
            .map_err(AppError::from)?;
    }

    if let Err(err) = checkpoint_wal(pool).await {
        tracing::debug!(error = %err, "wal checkpoint skipped");
    }

    if sessions_deleted + attempts_deleted + audit_deleted + clicks_deleted > 0 {
        tracing::info!(
            sessions_deleted,
            attempts_deleted,
            audit_deleted,
            clicks_deleted,
            "maintenance pass complete"
        );
    }
    let _ = now;
    Ok(())
}

/// Re-export used by the settings page.
pub use crate::db::settings::DEFAULTS as SETTING_DEFAULTS;

/// Token prefix helper used by the admin token listing.
pub fn token_service_prefix() -> &'static str {
    token_service::TOKEN_PREFIX
}

/// Settings keys that administrators may edit.
pub use crate::services::settings::EDITABLE_KEYS;

/// Read the settings table directly (used by tests and the CLI).
pub async fn raw_setting(pool: &SqlitePool, key: &str) -> Result<Option<String>> {
    settings_db::get(pool, key).await.map_err(AppError::from)
}
