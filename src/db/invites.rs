//! `invite_codes` and `invite_redemptions`.

use sqlx::{FromRow, SqliteConnection, SqlitePool};

#[derive(Debug, Clone, FromRow)]
pub struct Invite {
    pub id: i64,
    pub code_hash: String,
    pub code_prefix: String,
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
}

const SELECT: &str = "SELECT id, code_hash, code_prefix, note, group_id, created_by, max_uses, \
                      used_count, expires_at, status, created_at FROM invite_codes";

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
        "INSERT INTO invite_codes (code_hash, code_prefix, note, group_id, created_by, max_uses,
                                   used_count, expires_at, status, created_at)
         VALUES (?, ?, ?, ?, ?, ?, 0, ?, 'active', ?)",
    )
    .bind(invite.code_hash)
    .bind(invite.code_prefix)
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
