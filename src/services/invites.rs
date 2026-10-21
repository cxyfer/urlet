//! Invite issuance, shared by administrators and group members.
//!
//! Both paths use the same per-group calendar quota (`invite_quota`) and the
//! same issuance ledger, so an administrator-issued code and a member-issued
//! code draw from one budget. The difference is who may issue and which group
//! the code is pinned to:
//!
//!   * administrators choose any group (or none for the site-wide pool) and may
//!     also set an expiry;
//!   * members need `user_groups.allow_invites`, may only issue for their own
//!     group, and are capped at [`MAX_MEMBER_USES`] redemptions per code.
//!
//! Every code is stored twice: a hash for lookup and a sealed copy for the
//! "show it again" view (`services::invite_code`). The plaintext is handed to
//! the caller of [`Issuer::create`] and never written to the database as is.

use sqlx::SqlitePool;

use crate::db::user_groups::{self, UserGroup};
use crate::db::users::User;
use crate::db::{invite_issuance, invites};
use crate::error::{AppError, Result};
use crate::services::audit::{action, target, Audit};
use crate::services::invite_code;
use crate::services::invite_quota::InviteQuota;
use crate::services::settings::Settings;
use crate::services::{secret, time};

/// Longest accepted invite note.
pub const MAX_INVITE_NOTE: usize = 120;
/// Invite codes are 10 characters from an unambiguous alphabet.
const INVITE_CODE_LEN: usize = 10;
/// Administrators may create a code for up to 10 000 redemptions.
pub const MAX_ADMIN_USES: i64 = 10_000;
/// A member-issued code is meant for a small team, not bulk distribution.
pub const MAX_MEMBER_USES: i64 = 10;

/// A freshly created invite. The plain code is returned so the caller can show
/// it immediately; the sealed copy in `invite` keeps it available afterwards.
#[derive(Debug, Clone)]
pub struct IssuedInvite {
    pub invite: invites::Invite,
    pub plain_code: String,
}

/// Plaintext code of an invite, when its sealed copy can be opened.
///
/// `None` means the row predates sealed storage or the server secret changed;
/// such a code still redeems when someone knows it, but it cannot be displayed
/// again. Callers fall back to `code_prefix`.
pub fn reveal(secret: &[u8], invite: &invites::Invite) -> Option<String> {
    invite
        .code_sealed
        .as_deref()
        .and_then(|sealed| invite_code::open(secret, &invite.code_hash, sealed))
}

/// Public registration URL that carries `code` into the form.
///
/// Invite codes use only `A-Z` and `2-9`, so the value needs no percent
/// encoding. The canonical base URL is used so a link stays valid when the
/// issuer happens to be browsing another managed domain.
pub fn registration_link(base_url: &str, code: &str) -> String {
    format!(
        "{}/app/register?invite={code}",
        base_url.trim_end_matches('/')
    )
}

/// Who is issuing the code and under which policy.
pub struct Issuer<'a> {
    pub pool: &'a SqlitePool,
    pub settings: &'a Settings,
    pub actor: &'a User,
    pub ip_hash: Option<&'a str>,
    /// Server secret used to seal the recoverable copy of the code.
    pub secret: &'a [u8],
}

/// One issuance request. `max_uses_cap` distinguishes the admin and member caps.
pub struct Request<'a> {
    pub group_id: Option<i64>,
    pub note: Option<&'a str>,
    pub max_uses: Option<i64>,
    pub max_uses_cap: i64,
    pub expires_at: Option<&'a str>,
}

impl Issuer<'_> {
    /// Create an invite code for `request.group_id` (`None` = site-wide pool).
    pub async fn create(&self, request: Request<'_>) -> Result<IssuedInvite> {
        let max_uses = match request.max_uses {
            Some(value) if !(1..=request.max_uses_cap).contains(&value) => {
                return Err(AppError::validation(
                    "max_uses",
                    format!("uses must be between 1 and {}", request.max_uses_cap),
                ))
            }
            Some(value) => value,
            None => 1,
        };
        let group = match request.group_id {
            Some(group_id) => match user_groups::find_by_id(self.pool, group_id)
                .await
                .map_err(AppError::from)?
            {
                Some(group) => Some(group),
                None => {
                    return Err(AppError::validation(
                        "group_id",
                        "the selected group does not exist",
                    ))
                }
            },
            None => None,
        };
        let group_id = group.as_ref().map(|group| group.id);
        let note = match request
            .note
            .map(str::trim)
            .filter(|value| !value.is_empty())
        {
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
        let expires_at = match request
            .expires_at
            .map(str::trim)
            .filter(|value| !value.is_empty())
        {
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
        let code_sealed = invite_code::seal(self.secret, &code_hash, &plain_code);

        // Reserve the slot and create the code in one transaction. The
        // conditional INSERT is the authoritative quota check, and committing
        // both rows together means a cancelled or crashed request cannot leave
        // an orphan reservation consuming the group's budget.
        let quota = InviteQuota::resolve(self.settings, group.as_ref());
        let mut tx = self.pool.begin().await.map_err(AppError::from)?;
        let issuance_id = quota
            .reserve(&mut tx, group_id, Some(self.actor.id), time::now())
            .await?
            .ok_or_else(|| quota.exceeded())?;
        let id = invites::create(
            &mut tx,
            &crate::db::invites::NewInvite {
                code_hash: &code_hash,
                code_prefix: &prefix,
                code_sealed: Some(&code_sealed),
                note: note.as_deref(),
                group_id,
                created_by: Some(self.actor.id),
                max_uses,
                expires_at: expires_at.as_deref(),
            },
        )
        .await
        .map_err(|err| {
            // The group may have been deleted between the lookup and the write;
            // dropping `tx` rolls the reservation back with it.
            if crate::db::is_foreign_key_violation(&err) {
                AppError::validation("group_id", "the selected group no longer exists")
            } else {
                AppError::from(err)
            }
        })?;
        invite_issuance::attach(&mut tx, issuance_id, id)
            .await
            .map_err(AppError::from)?;
        tx.commit().await.map_err(AppError::from)?;

        let invite = invites::find_by_id(self.pool, id)
            .await
            .map_err(AppError::from)?
            .ok_or_else(|| AppError::internal("invite disappeared after creation"))?;

        Audit::new(action::INVITE_CREATED)
            .actor(self.actor.id, &self.actor.role)
            .target(target::INVITE, id)
            .metadata(serde_json::json!({
                "max_uses": max_uses,
                "prefix": prefix,
                "group_id": group_id
            }))
            .ip_hash(self.ip_hash.map(str::to_string))
            .record(self.pool)
            .await;

        Ok(IssuedInvite { invite, plain_code })
    }

    /// Issue a code for the member's own group. The group admin toggle and the
    /// group's calendar quota both apply; no expiry is accepted here.
    pub async fn create_for_member(
        &self,
        note: Option<&str>,
        max_uses: Option<i64>,
    ) -> Result<IssuedInvite> {
        let group = member_group(self.pool, self.actor)
            .await?
            .ok_or_else(|| AppError::forbidden(crate::services::limits::blocked::NO_GROUP))?;
        if !group.allow_invites {
            return Err(AppError::forbidden(
                crate::services::limits::blocked::INVITES,
            ));
        }
        self.create(Request {
            group_id: Some(group.id),
            note,
            max_uses,
            max_uses_cap: MAX_MEMBER_USES,
            expires_at: None,
        })
        .await
    }

    /// Revoke one of the caller's own codes. Someone else's code is reported as
    /// missing so the endpoint cannot be used to probe for ids.
    pub async fn revoke_owned(&self, invite_id: i64) -> Result<()> {
        let invite = invites::find_by_id(self.pool, invite_id)
            .await
            .map_err(AppError::from)?
            .ok_or_else(AppError::not_found)?;
        if invite.created_by != Some(self.actor.id) {
            return Err(AppError::not_found());
        }
        let revoked = invites::revoke(self.pool, invite_id)
            .await
            .map_err(AppError::from)?;
        if !revoked {
            return Err(AppError::not_found());
        }
        Audit::new(action::INVITE_REVOKED)
            .actor(self.actor.id, &self.actor.role)
            .target(target::INVITE, invite_id)
            .ip_hash(self.ip_hash.map(str::to_string))
            .record(self.pool)
            .await;
        Ok(())
    }
}

/// The group a member issues codes for: their own, or the default group when the
/// account has none. `None` means the instance has no usable group.
pub async fn member_group(pool: &SqlitePool, user: &User) -> Result<Option<UserGroup>> {
    match user.group_id {
        Some(group_id) => user_groups::find_by_id(pool, group_id)
            .await
            .map_err(AppError::from),
        None => user_groups::default_group(pool)
            .await
            .map_err(AppError::from),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registration_links_prefill_the_code() {
        assert_eq!(
            registration_link("https://s.example.com", "K7M2QPX4RT"),
            "https://s.example.com/app/register?invite=K7M2QPX4RT"
        );
        assert_eq!(
            registration_link("https://s.example.com/", "K7M2QPX4RT"),
            "https://s.example.com/app/register?invite=K7M2QPX4RT",
            "a trailing slash must not double up"
        );
    }
}
