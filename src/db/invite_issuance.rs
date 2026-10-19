//! `invite_issuance` ledger: the authoritative invite-quota counter.
//!
//! One row is written per issued invite code, in the same transaction as the
//! code itself, through a conditional `INSERT ... SELECT`. That single statement
//! is what makes the quota safe under concurrency: SQLite runs the subquery
//! while holding the write lock, so two administrators cannot both see the last
//! free slot. Because the code and its ledger row commit together, a cancelled
//! or crashed request cannot leave an orphan slot behind.
//!
//! Keeping a ledger instead of counting `invite_codes` means revoking a code
//! does not hand the slot back: the quota limits how many codes an
//! administrator may *issue* in a period, not how many are currently active.

use sqlx::{SqliteConnection, SqlitePool};

/// Try to claim one issuance slot.
///
/// `quota == 0` means unlimited. Returns the ledger row id, or `None` when the
/// group already used every slot of the current period. `group_id = None`
/// charges the site-wide quota.
pub async fn reserve(
    conn: &mut SqliteConnection,
    group_id: Option<i64>,
    actor_id: Option<i64>,
    quota: i64,
    window_start: &str,
    now: &str,
) -> Result<Option<i64>, sqlx::Error> {
    let result = sqlx::query(
        "INSERT INTO invite_issuance (group_id, actor_id, created_at)
         SELECT ?, ?, ?
          WHERE ? = 0
             OR (SELECT COUNT(*) FROM invite_issuance
                  WHERE group_id IS ? AND created_at >= ?) < ?",
    )
    .bind(group_id)
    .bind(actor_id)
    .bind(now)
    .bind(quota)
    .bind(group_id)
    .bind(window_start)
    .bind(quota)
    .execute(&mut *conn)
    .await?;
    if result.rows_affected() == 0 {
        Ok(None)
    } else {
        Ok(Some(result.last_insert_rowid()))
    }
}

/// Link a reserved slot to the invite code it paid for.
pub async fn attach(
    conn: &mut SqliteConnection,
    issuance_id: i64,
    invite_id: i64,
) -> Result<(), sqlx::Error> {
    sqlx::query("UPDATE invite_issuance SET invite_id = ? WHERE id = ?")
        .bind(invite_id)
        .bind(issuance_id)
        .execute(&mut *conn)
        .await?;
    Ok(())
}

/// Per-group usage since `window_start`, in one grouped query.
pub async fn counts_since(
    pool: &SqlitePool,
    window_start: &str,
) -> Result<Vec<(i64, i64)>, sqlx::Error> {
    sqlx::query_as(
        "SELECT group_id, COUNT(*) FROM invite_issuance
          WHERE group_id IS NOT NULL AND created_at >= ?
          GROUP BY group_id",
    )
    .bind(window_start)
    .fetch_all(pool)
    .await
}

/// Slots used by `group_id` since `window_start`. `None` counts the site-wide
/// (group-less) invites.
pub async fn used(
    pool: &SqlitePool,
    group_id: Option<i64>,
    window_start: &str,
) -> Result<i64, sqlx::Error> {
    sqlx::query_scalar(
        "SELECT COUNT(*) FROM invite_issuance WHERE group_id IS ? AND created_at >= ?",
    )
    .bind(group_id)
    .bind(window_start)
    .fetch_one(pool)
    .await
}
