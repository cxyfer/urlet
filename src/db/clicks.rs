//! `clicks` and `url_daily_stats` tables.

use sqlx::{FromRow, SqlitePool};

#[derive(Debug, Clone, FromRow)]
pub struct Click {
    pub id: i64,
    pub url_id: i64,
    pub clicked_at: String,
    pub ip_hash: Option<String>,
    pub visitor_hash: Option<String>,
    pub user_agent: Option<String>,
    pub referrer: Option<String>,
    pub referrer_host: Option<String>,
    pub country_code: Option<String>,
    pub device_type: Option<String>,
    pub browser: Option<String>,
    pub os: Option<String>,
    pub is_bot: i64,
    pub is_unique: i64,
    pub password_protected: i64,
}

/// A click about to be stored.
#[derive(Debug, Clone)]
pub struct NewClick {
    pub url_id: i64,
    pub clicked_at: String,
    pub ip_hash: Option<String>,
    pub visitor_hash: Option<String>,
    pub user_agent: Option<String>,
    pub referrer: Option<String>,
    pub referrer_host: Option<String>,
    pub device_type: String,
    pub browser: String,
    pub os: String,
    pub is_bot: bool,
    pub is_unique: bool,
    pub password_protected: bool,
}

/// Dimensions that can be grouped in the statistics endpoints.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Dimension {
    Referrer,
    Device,
    Browser,
    Os,
}

impl Dimension {
    /// Column name. Never derived from user input, so no injection risk.
    pub fn column(self) -> &'static str {
        match self {
            Self::Referrer => "IFNULL(referrer_host, 'direct')",
            Self::Device => "IFNULL(device_type, 'Unknown')",
            Self::Browser => "IFNULL(browser, 'Other')",
            Self::Os => "IFNULL(os, 'Unknown')",
        }
    }
}

/// Daily rollup row.
#[derive(Debug, Clone, FromRow)]
pub struct DailyStat {
    pub day: String,
    pub clicks: i64,
    pub unique_clicks: i64,
    pub bot_clicks: i64,
}

/// Aggregate numbers for one link.
#[derive(Debug, Clone, Default, FromRow)]
pub struct ClickSummary {
    pub total_clicks: i64,
    pub unique_clicks: i64,
    pub bot_clicks: i64,
    pub first_click_at: Option<String>,
    pub last_click_at: Option<String>,
}

impl ClickSummary {
    pub fn human_clicks(&self) -> i64 {
        (self.total_clicks - self.bot_clicks).max(0)
    }
}

/// Label/count pair used by the breakdown charts.
#[derive(Debug, Clone, FromRow)]
pub struct DimensionCount {
    pub label: String,
    pub count: i64,
}

/// Has this visitor already clicked this link?
pub async fn visitor_seen(
    pool: &SqlitePool,
    url_id: i64,
    visitor_hash: &str,
) -> Result<bool, sqlx::Error> {
    let found: Option<i64> =
        sqlx::query_scalar("SELECT 1 FROM clicks WHERE url_id = ? AND visitor_hash = ? LIMIT 1")
            .bind(url_id)
            .bind(visitor_hash)
            .fetch_optional(pool)
            .await?;
    Ok(found.is_some())
}

/// Insert one click row.
pub async fn insert(pool: &SqlitePool, click: &NewClick) -> Result<i64, sqlx::Error> {
    let result = sqlx::query(
        "INSERT INTO clicks (url_id, clicked_at, ip_hash, visitor_hash, user_agent, referrer,
                             referrer_host, device_type, browser, os, is_bot, is_unique,
                             password_protected)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(click.url_id)
    .bind(&click.clicked_at)
    .bind(click.ip_hash.as_deref())
    .bind(click.visitor_hash.as_deref())
    .bind(click.user_agent.as_deref())
    .bind(click.referrer.as_deref())
    .bind(click.referrer_host.as_deref())
    .bind(&click.device_type)
    .bind(&click.browser)
    .bind(&click.os)
    .bind(i64::from(click.is_bot))
    .bind(i64::from(click.is_unique))
    .bind(i64::from(click.password_protected))
    .execute(pool)
    .await?;
    Ok(result.last_insert_rowid())
}

/// Upsert the daily rollup row for a click.
pub async fn bump_daily(
    pool: &SqlitePool,
    url_id: i64,
    day: &str,
    is_unique: bool,
    is_bot: bool,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT INTO url_daily_stats (url_id, day, clicks, unique_clicks, bot_clicks)
         VALUES (?, ?, 1, ?, ?)
         ON CONFLICT (url_id, day) DO UPDATE SET
             clicks = clicks + 1,
             unique_clicks = unique_clicks + excluded.unique_clicks,
             bot_clicks = bot_clicks + excluded.bot_clicks",
    )
    .bind(url_id)
    .bind(day)
    .bind(i64::from(is_unique))
    .bind(i64::from(is_bot))
    .execute(pool)
    .await?;
    Ok(())
}

/// Update the denormalised counters on the link itself.
pub async fn bump_url_counters(
    pool: &SqlitePool,
    url_id: i64,
    clicked_at: &str,
    is_unique: bool,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "UPDATE urls SET click_count = click_count + 1,
                         unique_click_count = unique_click_count + ?,
                         last_clicked_at = ?
         WHERE id = ?",
    )
    .bind(i64::from(is_unique))
    .bind(clicked_at)
    .bind(url_id)
    .execute(pool)
    .await?;
    Ok(())
}

/// Recent clicks of one link, newest first.
pub async fn recent_for_url(
    pool: &SqlitePool,
    url_id: i64,
    limit: i64,
    offset: i64,
) -> Result<Vec<Click>, sqlx::Error> {
    sqlx::query_as::<_, Click>(
        "SELECT id, url_id, clicked_at, ip_hash, visitor_hash, user_agent, referrer, referrer_host,
                country_code, device_type, browser, os, is_bot, is_unique, password_protected
         FROM clicks WHERE url_id = ?
         ORDER BY clicked_at DESC, id DESC
         LIMIT ? OFFSET ?",
    )
    .bind(url_id)
    .bind(limit)
    .bind(offset)
    .fetch_all(pool)
    .await
}

/// Number of click rows for a link.
pub async fn count_for_url(pool: &SqlitePool, url_id: i64) -> Result<i64, sqlx::Error> {
    sqlx::query_scalar("SELECT COUNT(*) FROM clicks WHERE url_id = ?")
        .bind(url_id)
        .fetch_one(pool)
        .await
}

/// Aggregate summary for one link.
pub async fn summary_for_url(pool: &SqlitePool, url_id: i64) -> Result<ClickSummary, sqlx::Error> {
    sqlx::query_as::<_, ClickSummary>(
        "SELECT COUNT(*) AS total_clicks,
                IFNULL(SUM(is_unique), 0) AS unique_clicks,
                IFNULL(SUM(is_bot), 0) AS bot_clicks,
                MIN(clicked_at) AS first_click_at,
                MAX(clicked_at) AS last_click_at
         FROM clicks WHERE url_id = ?",
    )
    .bind(url_id)
    .fetch_one(pool)
    .await
}

/// Daily series for one link, oldest first.
pub async fn daily_series(
    pool: &SqlitePool,
    url_id: i64,
    from_day: &str,
) -> Result<Vec<DailyStat>, sqlx::Error> {
    sqlx::query_as::<_, DailyStat>(
        "SELECT day, clicks, unique_clicks, bot_clicks
         FROM url_daily_stats
         WHERE url_id = ? AND day >= ?
         ORDER BY day ASC",
    )
    .bind(url_id)
    .bind(from_day)
    .fetch_all(pool)
    .await
}

/// Grouped counts for one dimension of a single link.
pub async fn breakdown_for_url(
    pool: &SqlitePool,
    url_id: i64,
    dimension: Dimension,
    limit: i64,
) -> Result<Vec<DimensionCount>, sqlx::Error> {
    let sql = format!(
        "SELECT {} AS label, COUNT(*) AS count
         FROM clicks WHERE url_id = ?
         GROUP BY label ORDER BY count DESC, label ASC LIMIT ?",
        dimension.column()
    );
    sqlx::query_as::<_, DimensionCount>(&sql)
        .bind(url_id)
        .bind(limit)
        .fetch_all(pool)
        .await
}

/// Totals across every link of one user (dashboard cards).
pub async fn user_summary(pool: &SqlitePool, user_id: i64) -> Result<ClickSummary, sqlx::Error> {
    sqlx::query_as::<_, ClickSummary>(
        "SELECT COUNT(*) AS total_clicks,
                IFNULL(SUM(c.is_unique), 0) AS unique_clicks,
                IFNULL(SUM(c.is_bot), 0) AS bot_clicks,
                MIN(c.clicked_at) AS first_click_at,
                MAX(c.clicked_at) AS last_click_at
         FROM clicks c JOIN urls u ON u.id = c.url_id
         WHERE u.user_id = ? AND u.deleted_at IS NULL",
    )
    .bind(user_id)
    .fetch_one(pool)
    .await
}

/// Daily totals across every link of one user.
pub async fn user_daily_series(
    pool: &SqlitePool,
    user_id: i64,
    from_day: &str,
) -> Result<Vec<DailyStat>, sqlx::Error> {
    sqlx::query_as::<_, DailyStat>(
        "SELECT s.day AS day,
                SUM(s.clicks) AS clicks,
                SUM(s.unique_clicks) AS unique_clicks,
                SUM(s.bot_clicks) AS bot_clicks
         FROM url_daily_stats s JOIN urls u ON u.id = s.url_id
         WHERE u.user_id = ? AND u.deleted_at IS NULL AND s.day >= ?
         GROUP BY s.day
         ORDER BY s.day ASC",
    )
    .bind(user_id)
    .bind(from_day)
    .fetch_all(pool)
    .await
}

/// Clicks recorded since `from` for one user.
pub async fn user_clicks_since(
    pool: &SqlitePool,
    user_id: i64,
    from: &str,
) -> Result<i64, sqlx::Error> {
    sqlx::query_scalar(
        "SELECT COUNT(*) FROM clicks c JOIN urls u ON u.id = c.url_id
         WHERE u.user_id = ? AND u.deleted_at IS NULL AND c.clicked_at >= ?",
    )
    .bind(user_id)
    .bind(from)
    .fetch_one(pool)
    .await
}

/// Best performing links of one user.
pub async fn top_urls_for_user(
    pool: &SqlitePool,
    user_id: i64,
    limit: i64,
) -> Result<Vec<(i64, String, String, i64)>, sqlx::Error> {
    sqlx::query_as(
        "SELECT id, slug, target_url, click_count FROM urls
         WHERE user_id = ? AND deleted_at IS NULL
         ORDER BY click_count DESC, created_at DESC LIMIT ?",
    )
    .bind(user_id)
    .bind(limit)
    .fetch_all(pool)
    .await
}

/// Retention: delete clicks older than `before`.
pub async fn delete_older_than(pool: &SqlitePool, before: &str) -> Result<u64, sqlx::Error> {
    let result = sqlx::query("DELETE FROM clicks WHERE clicked_at < ?")
        .bind(before)
        .execute(pool)
        .await?;
    Ok(result.rows_affected())
}

/// Retention: drop rollup rows for days older than `before_day`.
pub async fn delete_daily_before(pool: &SqlitePool, before_day: &str) -> Result<u64, sqlx::Error> {
    let result = sqlx::query("DELETE FROM url_daily_stats WHERE day < ?")
        .bind(before_day)
        .execute(pool)
        .await?;
    Ok(result.rows_affected())
}

/// Global click totals (administrative dashboard).
pub async fn global_totals(pool: &SqlitePool) -> Result<(i64, i64, i64), sqlx::Error> {
    sqlx::query_as("SELECT COUNT(*), IFNULL(SUM(is_unique), 0), IFNULL(SUM(is_bot), 0) FROM clicks")
        .fetch_one(pool)
        .await
}
