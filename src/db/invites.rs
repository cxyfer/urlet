//! `invite_codes` and `invite_redemptions`.

use sqlx::{FromRow, SqliteConnection, SqlitePool};

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
    pool: &SqlitePool,
    code_hash: &str,
    now: &str,
) -> Result<Option<Invite>, sqlx::Error> {
    sqlx::query_as::<_, Invite>(&format!(
        "{SELECT} WHERE code_hash = ? AND status = 'active' AND used_count < max_uses
         AND (expires_at IS NULL OR expires_at > ?)"
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
pub async fn create(
    conn: &mut SqliteConnection,
    invite: &NewInvite<'_>,
) -> Result<i64, sqlx::Error> {
    let result = sqlx::query(
        "INSERT INTO invite_codes (code_hash, code_prefix, code_sealed, note, group_id, created_by,
                                   max_uses, used_count, expires_at, status, created_at)
         VALUES (?, ?, ?, ?, ?, ?, ?, 0, ?, 'active', ?)",
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
    .execute(&mut *conn)
    .await?;
    Ok(result.last_insert_rowid())
}

/// Atomically consume one use of an invite and record the redemption.
///
/// The conditional `UPDATE` is what makes this safe under concurrency: only one
/// transaction can move `used_count` past `max_uses`.
pub async fn redeem(
    pool: &SqlitePool,
    invite_id: i64,
    user_id: i64,
    now: &str,
) -> Result<bool, sqlx::Error> {
    let mut tx = pool.begin().await?;
    let updated = sqlx::query(
        "UPDATE invite_codes SET used_count = used_count + 1
         WHERE id = ? AND status = 'active' AND used_count < max_uses
           AND (expires_at IS NULL OR expires_at > ?)",
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
        "INSERT OR IGNORE INTO invite_redemptions (invite_id, user_id, redeemed_at)
         VALUES (?, ?, ?)",
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
pub async fn list(pool: &SqlitePool, limit: i64, offset: i64) -> Result<Vec<Invite>, sqlx::Error> {
    sqlx::query_as::<_, Invite>(&format!("{SELECT} ORDER BY id DESC LIMIT ? OFFSET ?"))
        .bind(limit)
        .bind(offset)
        .fetch_all(pool)
        .await
}

/// Invites created by one account, newest first (member self-service).
pub async fn list_by_creator(
    pool: &SqlitePool,
    creator_id: i64,
    limit: i64,
    offset: i64,
) -> Result<Vec<Invite>, sqlx::Error> {
    sqlx::query_as::<_, Invite>(&format!(
        "{SELECT} WHERE created_by = ? ORDER BY id DESC LIMIT ? OFFSET ?"
    ))
    .bind(creator_id)
    .bind(limit)
    .bind(offset)
    .fetch_all(pool)
    .await
}

/// Number of invites created by one account.
pub async fn count_by_creator(pool: &SqlitePool, creator_id: i64) -> Result<i64, sqlx::Error> {
    sqlx::query_scalar("SELECT COUNT(*) FROM invite_codes WHERE created_by = ?")
        .bind(creator_id)
        .fetch_one(pool)
        .await
}

/// Find one invite by id.
pub async fn find_by_id(pool: &SqlitePool, id: i64) -> Result<Option<Invite>, sqlx::Error> {
    sqlx::query_as::<_, Invite>(&format!("{SELECT} WHERE id = ?"))
        .bind(id)
        .fetch_optional(pool)
        .await
}

/// Count invites.
pub async fn count(pool: &SqlitePool) -> Result<i64, sqlx::Error> {
    sqlx::query_scalar("SELECT COUNT(*) FROM invite_codes")
        .fetch_one(pool)
        .await
}

/// Revoke an invite.
pub async fn revoke(pool: &SqlitePool, id: i64) -> Result<bool, sqlx::Error> {
    let result = sqlx::query(
        "UPDATE invite_codes SET status = 'revoked' WHERE id = ? AND status = 'active'",
    )
    .bind(id)
    .execute(pool)
    .await?;
    Ok(result.rows_affected() > 0)
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
