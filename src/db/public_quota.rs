//! 遊客自然週期配額的原子計數。

use sqlx::SqlitePool;

/// 嘗試扣抵一個名額；條件式 UPSERT 讓並行請求不會突破上限。
pub async fn reserve(
    pool: &SqlitePool,
    scope: &str,
    period: &str,
    start: i64,
    end: i64,
    limit: i64,
) -> Result<bool, sqlx::Error> {
    let used: Option<i64> = sqlx::query_scalar(
        "INSERT INTO public_creation_quota (scope, period, window_start, window_end, used)
         VALUES (?, ?, ?, ?, 1)
         ON CONFLICT (scope, period, window_start) DO UPDATE SET used = used + 1
         WHERE used < ?
         RETURNING used",
    )
    .bind(scope)
    .bind(period)
    .bind(start)
    .bind(end)
    .bind(limit)
    .fetch_optional(pool)
    .await?;
    Ok(used.is_some())
}

/// 只移除已結束的週期，未到期的使用量必須保留。
pub async fn delete_expired(pool: &SqlitePool, now: i64) -> Result<u64, sqlx::Error> {
    Ok(
        sqlx::query("DELETE FROM public_creation_quota WHERE window_end <= ?")
            .bind(now)
            .execute(pool)
            .await?
            .rows_affected(),
    )
}
