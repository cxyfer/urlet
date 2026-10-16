//! Data transfer objects shared by the JSON API and the HTML views.

use serde::{Deserialize, Deserializer, Serialize};

use crate::db::sessions::Session;
use crate::db::tags::TagWithCount;
use crate::db::urls::Url;
use crate::db::users::User;
use crate::services::time;

/// Deserialize `Option<Option<T>>` so that a missing field and an explicit
/// `null` mean different things in `PATCH` payloads.
///
/// * field absent  → `None`          (leave unchanged)
/// * field `null`  → `Some(None)`    (clear the value)
/// * field present → `Some(Some(v))` (set the value)
pub fn double_option<'de, D, T>(deserializer: D) -> Result<Option<Option<T>>, D::Error>
where
    D: Deserializer<'de>,
    T: Deserialize<'de>,
{
    Option::<T>::deserialize(deserializer).map(Some)
}

/// Public representation of a user. Never contains a password hash.
#[derive(Debug, Clone, Serialize)]
pub struct UserDto {
    pub id: i64,
    pub email: String,
    pub display_name: Option<String>,
    pub role: String,
    pub status: String,
    pub created_at: String,
    pub last_login_at: Option<String>,
}

impl From<&User> for UserDto {
    fn from(user: &User) -> Self {
        Self {
            id: user.id,
            email: user.email.clone(),
            display_name: user.display_name.clone(),
            role: user.role.clone(),
            status: user.status.clone(),
            created_at: user.created_at.clone(),
            last_login_at: user.last_login_at.clone(),
        }
    }
}

/// Public representation of a session.
#[derive(Debug, Clone, Serialize)]
pub struct SessionDto {
    pub id: i64,
    pub user_agent: Option<String>,
    pub created_at: String,
    pub last_seen_at: String,
    pub expires_at: String,
    pub current: bool,
    pub revoked: bool,
}

impl SessionDto {
    pub fn new(session: &Session, current_id: Option<i64>) -> Self {
        Self {
            id: session.id,
            user_agent: session.user_agent.clone(),
            created_at: session.created_at.clone(),
            last_seen_at: session.last_seen_at.clone(),
            expires_at: session.expires_at.clone(),
            current: current_id == Some(session.id),
            revoked: session.revoked_at.is_some(),
        }
    }
}

/// Standard pagination envelope.
#[derive(Debug, Clone, Serialize)]
pub struct Page<T> {
    pub items: Vec<T>,
    pub page: i64,
    pub per_page: i64,
    pub total: i64,
    pub total_pages: i64,
}

impl<T> Page<T> {
    pub fn new(items: Vec<T>, page: i64, per_page: i64, total: i64) -> Self {
        let total_pages = if per_page <= 0 { 1 } else { (total + per_page - 1) / per_page };
        Self { items, page, per_page, total, total_pages: total_pages.max(1) }
    }
}

/// Query string pagination parameters.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct Pagination {
    pub page: Option<i64>,
    pub per_page: Option<i64>,
}

impl Pagination {
    pub const MAX_PER_PAGE: i64 = 100;
    pub const DEFAULT_PER_PAGE: i64 = 20;

    pub fn page(&self) -> i64 {
        self.page.unwrap_or(1).max(1)
    }

    pub fn per_page(&self) -> i64 {
        self.per_page.unwrap_or(Self::DEFAULT_PER_PAGE).clamp(1, Self::MAX_PER_PAGE)
    }

    pub fn offset(&self) -> i64 {
        (self.page() - 1) * self.per_page()
    }
}

/// Simple success envelope for mutations that return nothing meaningful.
#[derive(Debug, Clone, Serialize)]
pub struct OkResponse {
    pub ok: bool,
}

impl Default for OkResponse {
    fn default() -> Self {
        Self { ok: true }
    }
}

/// Public representation of a short link.
#[derive(Debug, Clone, Serialize)]
pub struct UrlDto {
    pub id: i64,
    pub slug: String,
    pub short_url: String,
    pub target_url: String,
    pub title: Option<String>,
    pub notes: Option<String>,
    pub tags: Vec<String>,
    pub has_password: bool,
    pub expires_at: Option<String>,
    pub expires_in: Option<String>,
    pub max_clicks: Option<i64>,
    pub click_count: i64,
    pub unique_click_count: i64,
    pub redirect_type: i64,
    pub status: String,
    /// `active`, `expired`, `limit_reached`, `disabled` or `deleted`.
    pub state: String,
    pub is_custom_slug: bool,
    pub last_clicked_at: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

impl UrlDto {
    pub fn new(url: &Url, base_url: &str, tags: Vec<String>) -> Self {
        let state = url.state();
        Self {
            id: url.id,
            slug: url.slug.clone(),
            short_url: format!("{}/{}", base_url.trim_end_matches('/'), url.slug),
            target_url: url.target_url.clone(),
            title: url.title.clone(),
            notes: url.notes.clone(),
            tags,
            has_password: url.has_password(),
            expires_at: url.expires_at.clone(),
            expires_in: url.expires_at.as_deref().map(time::humanize),
            max_clicks: url.max_clicks,
            click_count: url.click_count,
            unique_click_count: url.unique_click_count,
            redirect_type: url.redirect_type,
            status: url.status.clone(),
            state: state.as_str().to_string(),
            is_custom_slug: url.is_custom(),
            last_clicked_at: url.last_clicked_at.clone(),
            created_at: url.created_at.clone(),
            updated_at: url.updated_at.clone(),
        }
    }
}

/// Public representation of a tag.
#[derive(Debug, Clone, Serialize)]
pub struct TagDto {
    pub id: i64,
    pub name: String,
    pub color: Option<String>,
    pub url_count: i64,
}

impl From<&TagWithCount> for TagDto {
    fn from(tag: &TagWithCount) -> Self {
        Self { id: tag.id, name: tag.name.clone(), color: tag.color.clone(), url_count: tag.url_count }
    }
}

/// Summary numbers for one link or one account.
#[derive(Debug, Clone, Serialize)]
pub struct StatsSummaryDto {
    pub total_clicks: i64,
    /// Clicks that were not classified as automated traffic.
    pub human_clicks: i64,
    pub unique_clicks: i64,
    pub bot_clicks: i64,
    pub first_click_at: Option<String>,
    pub last_click_at: Option<String>,
}

/// One day of the trend chart.
#[derive(Debug, Clone, Serialize)]
pub struct DailyPointDto {
    pub day: String,
    pub clicks: i64,
    pub unique_clicks: i64,
    pub bot_clicks: i64,
}

impl DailyPointDto {
    pub fn human_clicks(&self) -> i64 {
        (self.clicks - self.bot_clicks).max(0)
    }
}

/// A label/count pair (referrer, device, browser, OS).
#[derive(Debug, Clone, Serialize)]
pub struct DimensionDto {
    pub label: String,
    pub count: i64,
}

/// One recorded click. Deliberately free of IP and visitor hashes.
#[derive(Debug, Clone, Serialize)]
pub struct ClickDto {
    pub clicked_at: String,
    pub referrer_host: Option<String>,
    pub country_code: Option<String>,
    pub device_type: Option<String>,
    pub browser: Option<String>,
    pub os: Option<String>,
    pub is_bot: bool,
    pub is_unique: bool,
    pub password_protected: bool,
}

/// Full statistics payload for one link.
#[derive(Debug, Clone, Serialize)]
pub struct UrlStatsDto {
    pub url: UrlDto,
    pub summary: StatsSummaryDto,
    pub daily: Vec<DailyPointDto>,
    pub referrers: Vec<DimensionDto>,
    pub devices: Vec<DimensionDto>,
    pub browsers: Vec<DimensionDto>,
    pub operating_systems: Vec<DimensionDto>,
}

/// Account level overview used by the dashboard.
#[derive(Debug, Clone, Serialize)]
pub struct OverviewDto {
    pub links: i64,
    pub active_links: i64,
    pub expired_links: i64,
    pub total_clicks: i64,
    pub unique_clicks: i64,
    pub bot_clicks: i64,
    pub clicks_7d: i64,
    pub clicks_30d: i64,
    pub daily: Vec<DailyPointDto>,
    pub top_links: Vec<TopLinkDto>,
}

/// Best performing link.
#[derive(Debug, Clone, Serialize)]
pub struct TopLinkDto {
    pub id: i64,
    pub slug: String,
    pub target_url: String,
    pub click_count: i64,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pagination_is_clamped() {
        let pagination = Pagination { page: Some(0), per_page: Some(1000) };
        assert_eq!(pagination.page(), 1);
        assert_eq!(pagination.per_page(), 100);
        assert_eq!(pagination.offset(), 0);

        let pagination = Pagination { page: Some(3), per_page: Some(10) };
        assert_eq!(pagination.offset(), 20);
    }

    #[test]
    fn page_computes_total_pages() {
        let page = Page::new(vec![1, 2], 1, 20, 41);
        assert_eq!(page.total_pages, 3);
        let empty = Page::<i32>::new(vec![], 1, 20, 0);
        assert_eq!(empty.total_pages, 1);
    }
}
