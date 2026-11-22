//! 遊客自然週期配額的原子計數。

use crate::db::Pool;

/// 與記憶體限流器相同的容量上限，包含所有來源與週期。
const MAX_ROWS: i64 = 50_000;

/// 嘗試扣抵一個名額；條件式 UPSERT 讓並行請求不會突破上限。
pub async fn reserve(
    pool: &Pool,
    scope: &str,
    period: &str,
    start: i64,
    end: i64,
    limit: i64,
    now: i64,
) -> Result<bool, sqlx::Error> {
    // 列數上限是跨列的彙總條件；PostgreSQL 不會序列化 autocommit 敘述，
    // 因此回收與扣抵在同一個寫入交易內完成（FR-DB-6）。
    let mut tx = crate::db::begin_write(pool).await?;
    // 只回收已到期計數；即使沒有執行每日維護，也能在下一週期恢復服務。
    sqlx::query("DELETE FROM public_creation_quota WHERE window_end <= $1")
        .bind(now)
        .execute(&mut *tx)
        .await?;
    let used: Option<i64> = sqlx::query_scalar(
        "INSERT INTO public_creation_quota (scope, period, window_start, window_end, used)
         SELECT $1, $2, $3, $4, 1
         WHERE EXISTS (
             SELECT 1 FROM public_creation_quota
             WHERE scope = $5 AND period = $6 AND window_start = $7
         ) OR (SELECT COUNT(*) FROM public_creation_quota) < $8
         ON CONFLICT (scope, period, window_start) DO UPDATE SET used = public_creation_quota.used + 1
         WHERE public_creation_quota.used < $9
         RETURNING used",
    )
    .bind(scope)
    .bind(period)
    .bind(start)
    .bind(end)
    .bind(scope)
    .bind(period)
    .bind(start)
    .bind(MAX_ROWS)
    .bind(limit)
    .fetch_optional(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(used.is_some())
}

/// 只移除已結束的週期，未到期的使用量必須保留。
pub async fn delete_expired(pool: &Pool, now: i64) -> Result<u64, sqlx::Error> {
    Ok(
        sqlx::query("DELETE FROM public_creation_quota WHERE window_end <= $1")
            .bind(now)
            .execute(pool)
            .await?
            .rows_affected(),
    )
}
