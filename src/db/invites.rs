//! `invite_codes` and `invite_redemptions`.

use crate::db::{Connection, Pool};
use sqlx::FromRow;

use crate::db::invite_issuance;

#[derive(Debug, Clone, FromRow)]
pub struct Invite {
    pub id: i64,
    pub code_hash: String,
    pub code_prefix: String,
    /// Encrypted copy of the code, or `None` for codes issued before sealed
    /// storage (or when a secret rotation made the copy unreadable).
    pub code_sealed: Option<String>,
    pub note: Option<String>,
    pub group_id: Option<i64>,
    pub created_by: Option<i64>,
    pub max_uses: i64,
    pub used_count: i64,
    pub expires_at: Option<String>,
    pub status: String,
    pub created_at: String,
}

impl Invite {
    pub fn is_exhausted(&self) -> bool {
        self.used_count >= self.max_uses
    }

    /// Whether the invite can still be redeemed at `now`.
    ///
    /// This mirrors the conditions in [`find_active_by_hash`] and [`redeem`]:
    /// an expiry equal to `now` is already too late.
    pub fn is_usable(&self, now: &str) -> bool {
        self.status == "active"
            && !self.is_exhausted()
            && self
                .expires_at
                .as_deref()
                .map(|value| value > now)
                .unwrap_or(true)
    }
}

const SELECT: &str = "SELECT id, code_hash, code_prefix, code_sealed, note, group_id, created_by, \
                      max_uses, used_count, expires_at, status, created_at FROM invite_codes";

/// Find a usable invite by the hash of the submitted code.
pub async fn find_active_by_hash(
    pool: &Pool,
    code_hash: &str,
    now: &str,
) -> Result<Option<Invite>, sqlx::Error> {
    sqlx::query_as::<_, Invite>(&format!(
        "{SELECT} WHERE code_hash = $1 AND status = 'active' AND used_count < max_uses
         AND (expires_at IS NULL OR expires_at > $2)"
    ))
    .bind(code_hash)
    .bind(now)
    .fetch_optional(pool)
    .await
}

/// Fields required to create an invite.
#[derive(Debug, Clone)]
pub struct NewInvite<'a> {
    pub code_hash: &'a str,
    pub code_prefix: &'a str,
    /// Encrypted copy of the code for the "show again" view. Callers that seed
    /// legacy rows directly may pass `None`.
    pub code_sealed: Option<&'a str>,
    pub note: Option<&'a str>,
    pub group_id: Option<i64>,
    pub created_by: Option<i64>,
    pub max_uses: i64,
    pub expires_at: Option<&'a str>,
}

/// Create an invite on an explicit connection so the caller can commit it in the
/// same transaction as its quota reservation. `group_id` optionally pins the
/// redeemer's group.
pub async fn create(conn: &mut Connection, invite: &NewInvite<'_>) -> Result<i64, sqlx::Error> {
    let id = sqlx::query_scalar::<_, i64>(
        "INSERT INTO invite_codes (code_hash, code_prefix, code_sealed, note, group_id, created_by,
                                   max_uses, used_count, expires_at, status, created_at)
         VALUES ($1, $2, $3, $4, $5, $6, $7, 0, $8, 'active', $9) RETURNING id",
    )
    .bind(invite.code_hash)
    .bind(invite.code_prefix)
    .bind(invite.code_sealed)
    .bind(invite.note)
    .bind(invite.group_id)
    .bind(invite.created_by)
    .bind(invite.max_uses)
    .bind(invite.expires_at)
    .bind(crate::services::time::now_iso())
    .fetch_one(&mut *conn)
    .await?;
    Ok(id)
}

/// Atomically consume one use of an invite and record the redemption.
///
/// The conditional `UPDATE` is what makes this safe under concurrency: only one
/// transaction can move `used_count` past `max_uses`.
pub async fn redeem(
    pool: &Pool,
    invite_id: i64,
    user_id: i64,
    now: &str,
) -> Result<bool, sqlx::Error> {
    let mut tx = crate::db::begin_write(pool).await?;
    let updated = sqlx::query(
        "UPDATE invite_codes SET used_count = used_count + 1
         WHERE id = $1 AND status = 'active' AND used_count < max_uses
           AND (expires_at IS NULL OR expires_at > $2)",
    )
    .bind(invite_id)
    .bind(now)
    .execute(&mut *tx)
    .await?;

    if updated.rows_affected() == 0 {
        tx.rollback().await?;
        return Ok(false);
    }

    sqlx::query(
        "INSERT INTO invite_redemptions (invite_id, user_id, redeemed_at)
         VALUES ($1, $2, $3)
         ON CONFLICT DO NOTHING",
    )
    .bind(invite_id)
    .bind(user_id)
    .bind(now)
    .execute(&mut *tx)
    .await?;

    tx.commit().await?;
    Ok(true)
}

/// List invites, newest first.
pub async fn list(pool: &Pool, limit: i64, offset: i64) -> Result<Vec<Invite>, sqlx::Error> {
    sqlx::query_as::<_, Invite>(&format!("{SELECT} ORDER BY id DESC LIMIT $1 OFFSET $2"))
        .bind(limit)
        .bind(offset)
        .fetch_all(pool)
        .await
}

/// Invites created by one account, newest first (member self-service).
pub async fn list_by_creator(
    pool: &Pool,
    creator_id: i64,
    limit: i64,
    offset: i64,
) -> Result<Vec<Invite>, sqlx::Error> {
    sqlx::query_as::<_, Invite>(&format!(
        "{SELECT} WHERE created_by = $1 ORDER BY id DESC LIMIT $2 OFFSET $3"
    ))
    .bind(creator_id)
    .bind(limit)
    .bind(offset)
    .fetch_all(pool)
    .await
}

/// Number of invites created by one account.
pub async fn count_by_creator(pool: &Pool, creator_id: i64) -> Result<i64, sqlx::Error> {
    sqlx::query_scalar("SELECT COUNT(*) FROM invite_codes WHERE created_by = $1")
        .bind(creator_id)
        .fetch_one(pool)
        .await
}

/// Find one invite by id.
pub async fn find_by_id(pool: &Pool, id: i64) -> Result<Option<Invite>, sqlx::Error> {
    sqlx::query_as::<_, Invite>(&format!("{SELECT} WHERE id = $1"))
        .bind(id)
        .fetch_optional(pool)
        .await
}

/// Count invites.
pub async fn count(pool: &Pool) -> Result<i64, sqlx::Error> {
    sqlx::query_scalar("SELECT COUNT(*) FROM invite_codes")
        .fetch_one(pool)
        .await
}

/// Revoke an active invite and, when nobody redeemed it, release its quota slot.
///
/// The status change and the ledger release commit together. The redemption
/// count is read after the write lock is taken, so a concurrent redemption
/// either commits first (the slot stays charged) or fails because the code is
/// no longer active. Returns `None` when the invite was missing or already
/// revoked, or `Some(released)` where `released` reports whether the quota slot
/// was actually returned.
pub async fn revoke(pool: &Pool, id: i64) -> Result<Option<bool>, sqlx::Error> {
    let mut tx = crate::db::begin_write(pool).await?;
    let result = sqlx::query(
        "UPDATE invite_codes SET status = 'revoked' WHERE id = $1 AND status = 'active'",
    )
    .bind(id)
    .execute(&mut *tx)
    .await?;
    if result.rows_affected() == 0 {
        tx.rollback().await?;
        return Ok(None);
    }
    let used_count: i64 = sqlx::query_scalar("SELECT used_count FROM invite_codes WHERE id = $1")
        .bind(id)
        .fetch_one(&mut *tx)
        .await?;
    // A redeemed code did real work: revocation must not hand the slot back,
    // otherwise code creation + redemption + revocation could bypass the
    // per-period quota.
    let released = if used_count == 0 {
        invite_issuance::release(&mut tx, id, &crate::services::time::now_iso()).await?
    } else {
        false
    };
    tx.commit().await?;
    Ok(Some(released))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn invite(status: &str, used: i64, max: i64, expires: Option<&str>) -> Invite {
        Invite {
            id: 1,
            code_hash: "hash".into(),
            code_prefix: "ABCD".into(),
            code_sealed: None,
            note: None,
            group_id: None,
            created_by: None,
            max_uses: max,
            used_count: used,
            expires_at: expires.map(str::to_string),
            status: status.into(),
            created_at: "2026-01-01T00:00:00.000Z".into(),
        }
    }

    #[test]
    fn usability_matches_redemption_conditions() {
        let now = "2026-01-01T00:00:00.000Z";
        assert!(invite("active", 0, 1, None).is_usable(now));
        assert!(!invite("active", 1, 1, None).is_usable(now), "exhausted");
        assert!(!invite("revoked", 0, 1, None).is_usable(now), "revoked");
        assert!(invite("active", 0, 1, Some("2026-01-01T00:00:01.000Z")).is_usable(now));
        assert!(
            !invite("active", 0, 1, Some(now)).is_usable(now),
            "an expiry equal to now is already too late"
        );
        assert!(!invite("active", 0, 1, Some("2025-12-31T23:59:59.000Z")).is_usable(now));
    }
}
