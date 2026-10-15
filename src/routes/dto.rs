//! Data transfer objects shared by the JSON API and the HTML views.

use serde::{Deserialize, Serialize};

use crate::db::sessions::Session;
use crate::db::users::User;

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
