//! `urls` table.

use sqlx::{FromRow, SqlitePool};

use crate::services::time;

#[derive(Debug, Clone, FromRow)]
pub struct Url {
    pub id: i64,
    pub user_id: i64,
    pub domain_id: Option<i64>,
    pub slug: String,
    pub target_url: String,
    pub title: Option<String>,
    pub notes: Option<String>,
    pub password_hash: Option<String>,
    pub expires_at: Option<String>,
    pub max_clicks: Option<i64>,
    pub click_count: i64,
    pub unique_click_count: i64,
    pub redirect_type: i64,
    pub status: String,
    pub is_custom_slug: i64,
    pub last_clicked_at: Option<String>,
    pub created_at: String,
    pub updated_at: String,
    pub deleted_at: Option<String>,
}

/// Lifecycle state of a short link, derived from its columns.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UrlState {
    Active,
    Expired,
    LimitReached,
    Disabled,
    Deleted,
}

impl UrlState {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Active => "active",
            Self::Expired => "expired",
            Self::LimitReached => "limit_reached",
            Self::Disabled => "disabled",
            Self::Deleted => "deleted",
        }
    }

    /// Only `Active` links redirect.
    pub fn is_reachable(self) -> bool {
        matches!(self, Self::Active)
    }
}

impl Url {
    pub fn has_password(&self) -> bool {
        self.password_hash
            .as_deref()
            .map(|hash| !hash.is_empty())
            .unwrap_or(false)
    }

    pub fn is_custom(&self) -> bool {
        self.is_custom_slug == 1
    }

    /// Derive the current state at `now` (an ISO timestamp).
    pub fn state_at(&self, now: &str) -> UrlState {
        if self.deleted_at.is_some() {
            return UrlState::Deleted;
        }
        if self.status != "active" {
            return UrlState::Disabled;
        }
        if let Some(expires_at) = self.expires_at.as_deref() {
            if expires_at <= now {
                return UrlState::Expired;
            }
        }
        if let Some(max_clicks) = self.max_clicks {
            if self.click_count >= max_clicks {
                return UrlState::LimitReached;
            }
        }
        UrlState::Active
    }

    pub fn state(&self) -> UrlState {
        self.state_at(&time::now_iso())
    }

    pub fn is_expired(&self) -> bool {
        matches!(self.state(), UrlState::Expired | UrlState::LimitReached)
    }
}

/// Fields required to create a short link.
#[derive(Debug, Clone)]
pub struct NewUrl {
    pub user_id: i64,
    pub domain_id: Option<i64>,
    pub slug: String,
    pub target_url: String,
    pub title: Option<String>,
    pub notes: Option<String>,
    pub password_hash: Option<String>,
    pub expires_at: Option<String>,
    pub max_clicks: Option<i64>,
    pub redirect_type: i64,
    pub is_custom_slug: bool,
}

const SELECT: &str =
    "SELECT id, user_id, domain_id, slug, target_url, title, notes, password_hash, \
                      expires_at, max_clicks, click_count, unique_click_count, redirect_type, \
                      status, is_custom_slug, last_clicked_at, created_at, updated_at, deleted_at \
                      FROM urls";

/// Insert a short link.
pub async fn create(pool: &SqlitePool, new_url: &NewUrl) -> Result<i64, sqlx::Error> {
    let now = time::now_iso();
    let result = sqlx::query(
        "INSERT INTO urls (user_id, domain_id, slug, target_url, title, notes, password_hash,
                           expires_at, max_clicks, redirect_type, status, is_custom_slug,
                           created_at, updated_at)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, 'active', ?, ?, ?)",
    )
    .bind(new_url.user_id)
    .bind(new_url.domain_id)
    .bind(&new_url.slug)
    .bind(&new_url.target_url)
    .bind(new_url.title.as_deref())
    .bind(new_url.notes.as_deref())
    .bind(new_url.password_hash.as_deref())
    .bind(new_url.expires_at.as_deref())
    .bind(new_url.max_clicks)
    .bind(new_url.redirect_type)
    .bind(i64::from(new_url.is_custom_slug))
    .bind(&now)
    .bind(&now)
    .execute(pool)
    .await?;
    Ok(result.last_insert_rowid())
}

/// Find a live link by id (any owner). Soft deleted links are not returned.
pub async fn find_by_id(pool: &SqlitePool, id: i64) -> Result<Option<Url>, sqlx::Error> {
    sqlx::query_as::<_, Url>(&format!("{SELECT} WHERE id = ? AND deleted_at IS NULL"))
        .bind(id)
        .fetch_optional(pool)
        .await
}

/// Find a live link by id, restricted to its owner.
pub async fn find_owned(
    pool: &SqlitePool,
    id: i64,
    user_id: i64,
) -> Result<Option<Url>, sqlx::Error> {
    sqlx::query_as::<_, Url>(&format!(
        "{SELECT} WHERE id = ? AND user_id = ? AND deleted_at IS NULL"
    ))
    .bind(id)
    .bind(user_id)
    .fetch_optional(pool)
    .await
}

/// Find a live link by slug. The column is `COLLATE NOCASE`, so this is
/// case-insensitive and still uses the unique index.
pub async fn find_by_slug(pool: &SqlitePool, slug: &str) -> Result<Option<Url>, sqlx::Error> {
    sqlx::query_as::<_, Url>(&format!("{SELECT} WHERE slug = ? AND deleted_at IS NULL"))
        .bind(slug)
        .fetch_optional(pool)
        .await
}

/// True when the slug is taken (including by soft deleted links, which keep
/// their slug reserved so old links cannot be hijacked).
pub async fn slug_taken(pool: &SqlitePool, slug: &str) -> Result<bool, sqlx::Error> {
    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM urls WHERE slug = ?")
        .bind(slug)
        .fetch_one(pool)
        .await?;
    Ok(count > 0)
}

/// Field changes applied by an update.
#[derive(Debug, Clone, Default)]
pub struct UrlUpdate {
    pub slug: Option<String>,
    pub target_url: Option<String>,
    pub title: Option<Option<String>>,
    pub notes: Option<Option<String>>,
    pub password_hash: Option<Option<String>>,
    pub expires_at: Option<Option<String>>,
    pub max_clicks: Option<Option<i64>>,
    pub redirect_type: Option<i64>,
    pub status: Option<String>,
    pub is_custom_slug: Option<bool>,
}

impl UrlUpdate {
    pub fn is_empty(&self) -> bool {
        self.slug.is_none()
            && self.target_url.is_none()
            && self.title.is_none()
            && self.notes.is_none()
            && self.password_hash.is_none()
            && self.expires_at.is_none()
            && self.max_clicks.is_none()
            && self.redirect_type.is_none()
            && self.status.is_none()
            && self.is_custom_slug.is_none()
    }
}

/// Apply a partial update. Returns the number of rows changed.
pub async fn update(
    pool: &SqlitePool,
    id: i64,
    user_id: Option<i64>,
    changes: &UrlUpdate,
) -> Result<u64, sqlx::Error> {
    let mut sql = String::from("UPDATE urls SET updated_at = ?");
    let mut binds: Vec<Bind> = vec![Bind::Text(time::now_iso())];

    if let Some(value) = &changes.slug {
        sql.push_str(", slug = ?");
        binds.push(Bind::Text(value.clone()));
    }
    if let Some(value) = &changes.target_url {
        sql.push_str(", target_url = ?");
        binds.push(Bind::Text(value.clone()));
    }
    if let Some(value) = &changes.title {
        sql.push_str(", title = ?");
        binds.push(Bind::OptText(value.clone()));
    }
    if let Some(value) = &changes.notes {
        sql.push_str(", notes = ?");
        binds.push(Bind::OptText(value.clone()));
    }
    if let Some(value) = &changes.password_hash {
        sql.push_str(", password_hash = ?");
        binds.push(Bind::OptText(value.clone()));
    }
    if let Some(value) = &changes.expires_at {
        sql.push_str(", expires_at = ?");
        binds.push(Bind::OptText(value.clone()));
    }
    if let Some(value) = &changes.max_clicks {
        sql.push_str(", max_clicks = ?");
        binds.push(Bind::OptInt(*value));
    }
    if let Some(value) = changes.redirect_type {
        sql.push_str(", redirect_type = ?");
        binds.push(Bind::Int(value));
    }
    if let Some(value) = &changes.status {
        sql.push_str(", status = ?");
        binds.push(Bind::Text(value.clone()));
    }
    if let Some(value) = changes.is_custom_slug {
        sql.push_str(", is_custom_slug = ?");
        binds.push(Bind::Int(i64::from(value)));
    }

    sql.push_str(" WHERE id = ?");
    binds.push(Bind::Int(id));
    if let Some(user_id) = user_id {
        sql.push_str(" AND user_id = ?");
        binds.push(Bind::Int(user_id));
    }
    sql.push_str(" AND deleted_at IS NULL");

    // Build the query with the final SQL text, then bind in placeholder order.
    let mut query = sqlx::query(&sql);
    for bind in &binds {
        query = match bind {
            Bind::Text(value) => query.bind(value),
            Bind::OptText(value) => query.bind(value),
            Bind::Int(value) => query.bind(value),
            Bind::OptInt(value) => query.bind(value),
        };
    }

    let result = query.execute(pool).await?;
    Ok(result.rows_affected())
}

enum Bind {
    Text(String),
    OptText(Option<String>),
    Int(i64),
    OptInt(Option<i64>),
}

/// Soft delete a link. The slug stays reserved.
pub async fn soft_delete(
    pool: &SqlitePool,
    id: i64,
    user_id: Option<i64>,
) -> Result<bool, sqlx::Error> {
    let now = time::now_iso();
    let result = match user_id {
        Some(user_id) => {
            sqlx::query(
                "UPDATE urls SET deleted_at = ?, updated_at = ?
                 WHERE id = ? AND user_id = ? AND deleted_at IS NULL",
            )
            .bind(&now)
            .bind(&now)
            .bind(id)
            .bind(user_id)
            .execute(pool)
            .await?
        }
        None => sqlx::query(
            "UPDATE urls SET deleted_at = ?, updated_at = ? WHERE id = ? AND deleted_at IS NULL",
        )
        .bind(&now)
        .bind(&now)
        .bind(id)
        .execute(pool)
        .await?,
    };
    Ok(result.rows_affected() > 0)
}

/// Number of live links owned by a user.
pub async fn count_for_user(pool: &SqlitePool, user_id: i64) -> Result<i64, sqlx::Error> {
    sqlx::query_scalar("SELECT COUNT(*) FROM urls WHERE user_id = ? AND deleted_at IS NULL")
        .bind(user_id)
        .fetch_one(pool)
        .await
}

/// Live links of a user that are expired or have reached their click limit.
pub async fn count_expired_for_user(
    pool: &SqlitePool,
    user_id: i64,
    now: &str,
) -> Result<i64, sqlx::Error> {
    sqlx::query_scalar(
        "SELECT COUNT(*) FROM urls
         WHERE user_id = ? AND deleted_at IS NULL
           AND ((expires_at IS NOT NULL AND expires_at <= ?)
                OR (max_clicks IS NOT NULL AND click_count >= max_clicks))",
    )
    .bind(user_id)
    .bind(now)
    .fetch_one(pool)
    .await
}

/// Sort orders accepted by the listing endpoints.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum UrlSort {
    #[default]
    CreatedDesc,
    ClicksDesc,
    SlugAsc,
    UpdatedDesc,
    ExpiresAsc,
}

impl UrlSort {
    pub fn parse(value: Option<&str>) -> Self {
        match value.map(str::trim) {
            Some("clicks") | Some("clicks_desc") => Self::ClicksDesc,
            Some("slug") | Some("slug_asc") => Self::SlugAsc,
            Some("updated") | Some("updated_desc") => Self::UpdatedDesc,
            Some("expires") | Some("expires_asc") => Self::ExpiresAsc,
            _ => Self::CreatedDesc,
        }
    }

    fn as_int(self) -> i64 {
        match self {
            Self::CreatedDesc => 0,
            Self::ClicksDesc => 1,
            Self::SlugAsc => 2,
            Self::UpdatedDesc => 3,
            Self::ExpiresAsc => 4,
        }
    }
}

/// Filters for the owner facing listing.
#[derive(Debug, Clone, Default)]
pub struct UserUrlFilter {
    pub search: Option<String>,
    pub status: Option<String>,
    pub tag: Option<String>,
    pub sort: UrlSort,
}

/// Filters for the administrative listing.
#[derive(Debug, Clone, Default)]
pub struct AdminUrlFilter {
    pub search: Option<String>,
    pub status: Option<String>,
    pub user_id: Option<i64>,
    pub sort: UrlSort,
}

/// Escape LIKE wildcards so a search for `50%` is literal.
pub fn like_pattern(term: &str) -> String {
    let escaped = term
        .replace('\\', "\\\\")
        .replace('%', "\\%")
        .replace('_', "\\_");
    format!("%{escaped}%")
}

/// List links owned by one user.
pub async fn list_for_user(
    pool: &SqlitePool,
    user_id: i64,
    filter: &UserUrlFilter,
    limit: i64,
    offset: i64,
) -> Result<Vec<Url>, sqlx::Error> {
    let search = filter.search.as_deref().map(like_pattern);
    let status = filter.status.as_deref();
    let tag = filter.tag.as_deref();

    sqlx::query_as::<_, Url>(&format!(
        "{SELECT}
         WHERE deleted_at IS NULL AND user_id = ?1
           AND (?2 IS NULL OR status = ?2)
           AND (?3 IS NULL OR slug LIKE ?3 ESCAPE '\\' OR target_url LIKE ?3 ESCAPE '\\'
                          OR IFNULL(title, '') LIKE ?3 ESCAPE '\\')
           AND (?4 IS NULL OR id IN (SELECT ut.url_id FROM url_tags ut
                                     JOIN tags t ON t.id = ut.tag_id
                                     WHERE t.user_id = ?1 AND t.name = ?4))
         ORDER BY
           CASE WHEN ?5 = 1 THEN click_count END DESC,
           CASE WHEN ?5 = 2 THEN slug END ASC,
           CASE WHEN ?5 = 3 THEN updated_at END DESC,
           CASE WHEN ?5 = 4 THEN expires_at IS NULL END ASC,
           CASE WHEN ?5 = 4 THEN expires_at END ASC,
           created_at DESC, id DESC
         LIMIT ?6 OFFSET ?7"
    ))
    .bind(user_id)
    .bind(status)
    .bind(search.as_deref())
    .bind(tag)
    .bind(filter.sort.as_int())
    .bind(limit)
    .bind(offset)
    .fetch_all(pool)
    .await
}

/// Count links owned by one user.
pub async fn count_for_user_filtered(
    pool: &SqlitePool,
    user_id: i64,
    filter: &UserUrlFilter,
) -> Result<i64, sqlx::Error> {
    let search = filter.search.as_deref().map(like_pattern);
    sqlx::query_scalar(
        "SELECT COUNT(*) FROM urls
         WHERE deleted_at IS NULL AND user_id = ?1
           AND (?2 IS NULL OR status = ?2)
           AND (?3 IS NULL OR slug LIKE ?3 ESCAPE '\\' OR target_url LIKE ?3 ESCAPE '\\'
                          OR IFNULL(title, '') LIKE ?3 ESCAPE '\\')
           AND (?4 IS NULL OR id IN (SELECT ut.url_id FROM url_tags ut
                                     JOIN tags t ON t.id = ut.tag_id
                                     WHERE t.user_id = ?1 AND t.name = ?4))",
    )
    .bind(user_id)
    .bind(filter.status.as_deref())
    .bind(search.as_deref())
    .bind(filter.tag.as_deref())
    .fetch_one(pool)
    .await
}

/// List every link (administrative view).
pub async fn list_all(
    pool: &SqlitePool,
    filter: &AdminUrlFilter,
    limit: i64,
    offset: i64,
) -> Result<Vec<Url>, sqlx::Error> {
    let search = filter.search.as_deref().map(like_pattern);
    sqlx::query_as::<_, Url>(&format!(
        "{SELECT}
         WHERE deleted_at IS NULL
           AND (?1 IS NULL OR user_id = ?1)
           AND (?2 IS NULL OR status = ?2)
           AND (?3 IS NULL OR slug LIKE ?3 ESCAPE '\\' OR target_url LIKE ?3 ESCAPE '\\'
                          OR IFNULL(title, '') LIKE ?3 ESCAPE '\\')
         ORDER BY
           CASE WHEN ?4 = 1 THEN click_count END DESC,
           CASE WHEN ?4 = 2 THEN slug END ASC,
           created_at DESC, id DESC
         LIMIT ?5 OFFSET ?6"
    ))
    .bind(filter.user_id)
    .bind(filter.status.as_deref())
    .bind(search.as_deref())
    .bind(filter.sort.as_int())
    .bind(limit)
    .bind(offset)
    .fetch_all(pool)
    .await
}

/// Count every link (administrative view).
pub async fn count_all(pool: &SqlitePool, filter: &AdminUrlFilter) -> Result<i64, sqlx::Error> {
    let search = filter.search.as_deref().map(like_pattern);
    sqlx::query_scalar(
        "SELECT COUNT(*) FROM urls
         WHERE deleted_at IS NULL
           AND (?1 IS NULL OR user_id = ?1)
           AND (?2 IS NULL OR status = ?2)
           AND (?3 IS NULL OR slug LIKE ?3 ESCAPE '\\' OR target_url LIKE ?3 ESCAPE '\\'
                          OR IFNULL(title, '') LIKE ?3 ESCAPE '\\')",
    )
    .bind(filter.user_id)
    .bind(filter.status.as_deref())
    .bind(search.as_deref())
    .fetch_one(pool)
    .await
}

/// Global link statistics (administrative dashboard).
pub async fn global_counts(pool: &SqlitePool) -> Result<(i64, i64, i64), sqlx::Error> {
    let row: (i64, i64, i64) = sqlx::query_as(
        "SELECT COUNT(*), IFNULL(SUM(click_count), 0),
                IFNULL(SUM(CASE WHEN expires_at IS NOT NULL AND expires_at <= ?1 THEN 1 ELSE 0 END), 0)
         FROM urls WHERE deleted_at IS NULL",
    )
    .bind(time::now_iso())
    .fetch_one(pool)
    .await?;
    Ok(row)
}
