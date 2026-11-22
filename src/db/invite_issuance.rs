//! `invite_issuance` ledger: the authoritative invite-quota counter.
//!
//! One row is written per issued invite code, in the same transaction as the
//! code itself, through a conditional `INSERT ... SELECT`. That single statement
//! is what makes the quota safe under concurrency: SQLite runs the subquery
//! while holding the write lock, so two administrators cannot both see the last
//! free slot. Because the code and its ledger row commit together, a cancelled
//! or crashed request cannot leave an orphan slot behind.
//!
//! A row stops counting when the code it paid for is revoked before anyone
//! redeemed it: [`release`] stamps `released_at` and the slot returns to the
//! group. A code that was already redeemed keeps its slot even after
//! revocation, so creating, redeeming and revoking codes cannot exceed the
//! per-period limit.

use crate::db::{Connection, Pool};

/// Try to claim one issuance slot.
///
/// `quota == 0` means unlimited. Returns the ledger row id, or `None` when the
/// group already used every slot of the current period. `group_id = None`
/// charges the site-wide quota.
pub async fn reserve(
    conn: &mut Connection,
    group_id: Option<i64>,
    actor_id: Option<i64>,
    quota: i64,
    window_start: &str,
    now: &str,
) -> Result<Option<i64>, sqlx::Error> {
    let id = sqlx::query_scalar::<_, i64>(
        "INSERT INTO invite_issuance (group_id, actor_id, created_at)
         SELECT $1, $2, $3
          WHERE $4 = 0
             OR (SELECT COUNT(*) FROM invite_issuance
                  WHERE group_id IS NOT DISTINCT FROM $5 AND created_at >= $6 AND released_at IS NULL) < $7 RETURNING id",
    )
    .bind(group_id)
    .bind(actor_id)
    .bind(now)
    .bind(quota)
    .bind(group_id)
    .bind(window_start)
    .bind(quota)
    .fetch_optional(&mut *conn)
    .await?;
    Ok(id)
}

/// Release the slot of an invite that was revoked before anyone redeemed it.
///
/// The caller decides whether the code qualifies (see [`crate::db::invites::revoke`]);
/// a row that was already released, or a legacy invite with no ledger row, is
/// left untouched. Returns whether a slot was actually released.
pub async fn release(
    conn: &mut Connection,
    invite_id: i64,
    now: &str,
) -> Result<bool, sqlx::Error> {
    let result = sqlx::query(
        "UPDATE invite_issuance SET released_at = $1
          WHERE invite_id = $2 AND released_at IS NULL",
    )
    .bind(now)
    .bind(invite_id)
    .execute(&mut *conn)
    .await?;
    Ok(result.rows_affected() > 0)
}

/// Link a reserved slot to the invite code it paid for.
pub async fn attach(
    conn: &mut Connection,
    issuance_id: i64,
    invite_id: i64,
) -> Result<(), sqlx::Error> {
    sqlx::query("UPDATE invite_issuance SET invite_id = $1 WHERE id = $2")
        .bind(invite_id)
        .bind(issuance_id)
        .execute(&mut *conn)
        .await?;
    Ok(())
}

/// Per-group usage since `window_start`, in one grouped query.
pub async fn counts_since(pool: &Pool, window_start: &str) -> Result<Vec<(i64, i64)>, sqlx::Error> {
    sqlx::query_as(
        "SELECT group_id, COUNT(*) FROM invite_issuance
          WHERE group_id IS NOT NULL AND created_at >= $1 AND released_at IS NULL
          GROUP BY group_id",
    )
    .bind(window_start)
    .fetch_all(pool)
    .await
}

/// Slots used by `group_id` since `window_start`. `None` counts the site-wide
/// (group-less) invites.
pub async fn used(
    pool: &Pool,
    group_id: Option<i64>,
    window_start: &str,
) -> Result<i64, sqlx::Error> {
    sqlx::query_scalar(
        "SELECT COUNT(*) FROM invite_issuance
          WHERE group_id IS NOT DISTINCT FROM $1 AND created_at >= $2 AND released_at IS NULL",
    )
    .bind(group_id)
    .bind(window_start)
    .fetch_one(pool)
    .await
}
