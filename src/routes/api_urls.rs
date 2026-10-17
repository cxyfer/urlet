//! `/api/v1/urls` — short link management for the authenticated user.

use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde::Deserialize;

use crate::db::urls::{AdminUrlFilter, UrlSort, UserUrlFilter};
use crate::db::{tags, urls};
use crate::error::{ApiError, AppError};
use crate::middleware::auth::{ClientInfo, CurrentUser};
use crate::routes::dto::{double_option, Page, Pagination, UrlDto};
use crate::routes::verify_captcha;
use crate::services::captcha::CaptchaFlow;
use crate::services::ratelimit::limits;
use crate::services::urls::{CreateInput, UpdateInput, UrlContext};
use crate::state::AppState;

/// Query parameters for `GET /api/v1/urls`.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct ListQuery {
    #[serde(default)]
    pub page: Option<i64>,
    #[serde(default)]
    pub per_page: Option<i64>,
    #[serde(default)]
    pub q: Option<String>,
    #[serde(default)]
    pub tag: Option<String>,
    #[serde(default)]
    pub status: Option<String>,
    #[serde(default)]
    pub sort: Option<String>,
}

impl ListQuery {
    fn pagination(&self) -> Pagination {
        Pagination {
            page: self.page,
            per_page: self.per_page,
        }
    }

    fn filter(&self) -> UserUrlFilter {
        UserUrlFilter {
            search: clean(&self.q),
            status: clean(&self.status),
            tag: clean(&self.tag),
            sort: UrlSort::parse(self.sort.as_deref()),
        }
    }
}

/// Body of `POST /api/v1/urls`.
#[derive(Debug, Clone, Deserialize)]
pub struct CreatePayload {
    pub target_url: String,
    #[serde(default)]
    pub slug: Option<String>,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub notes: Option<String>,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub password: Option<String>,
    #[serde(default)]
    pub expires_at: Option<String>,
    #[serde(default)]
    pub max_clicks: Option<i64>,
    #[serde(default)]
    pub redirect_type: Option<i64>,
    /// Bot protection token. Only needed when the instance protects link
    /// creation and the caller authenticates with a session cookie; a Bearer
    /// API token is exempt. Optional in the schema either way.
    #[serde(default)]
    pub captcha_token: Option<String>,
}

/// Body of `PATCH /api/v1/urls/{id}`. Absent fields stay unchanged; explicit
/// `null` clears the field.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct UpdatePayload {
    #[serde(default)]
    pub target_url: Option<String>,
    #[serde(default)]
    pub slug: Option<String>,
    #[serde(default, deserialize_with = "double_option")]
    pub title: Option<Option<String>>,
    #[serde(default, deserialize_with = "double_option")]
    pub notes: Option<Option<String>>,
    #[serde(default)]
    pub tags: Option<Vec<String>>,
    #[serde(default, deserialize_with = "double_option")]
    pub password: Option<Option<String>>,
    #[serde(default, deserialize_with = "double_option")]
    pub expires_at: Option<Option<String>>,
    #[serde(default, deserialize_with = "double_option")]
    pub max_clicks: Option<Option<i64>>,
    #[serde(default)]
    pub redirect_type: Option<i64>,
    #[serde(default)]
    pub status: Option<String>,
}

/// `GET /api/v1/urls`
pub async fn list(
    State(state): State<AppState>,
    CurrentUser { user, .. }: CurrentUser,
    Query(query): Query<ListQuery>,
) -> Result<Json<Page<UrlDto>>, ApiError> {
    let pagination = query.pagination();
    let filter = query.filter();
    let rows = urls::list_for_user(
        &state.db,
        user.id,
        &filter,
        pagination.per_page(),
        pagination.offset(),
    )
    .await?;
    let total = urls::count_for_user_filtered(&state.db, user.id, &filter).await?;

    let ids: Vec<i64> = rows.iter().map(|row| row.id).collect();
    let mut tag_map = tags::for_urls(&state.db, &ids).await?;
    let items = rows
        .iter()
        .map(|row| {
            let names = tag_map
                .remove(&row.id)
                .unwrap_or_default()
                .into_iter()
                .map(|tag| tag.name)
                .collect();
            UrlDto::new(row, &state.config.base_url, names)
        })
        .collect();

    Ok(Json(Page::new(
        items,
        pagination.page(),
        pagination.per_page(),
        total,
    )))
}

/// `POST /api/v1/urls`
///
/// Bot protection applies to the **browser session** path only. The endpoint
/// accepts two credentials, and they are not equally trusted:
///
/// * an `Authorization: Bearer ur_...` API token — exempt, because a script that
///   already holds a long lived credential issued from the account page is not
///   the threat a captcha guards against, and challenging it would break every
///   existing integration;
/// * a session cookie — challenged when `captcha_on_create` is on, exactly like
///   the HTML form. Without this, a signed-in robot could simply call the JSON
///   API with the cookie it already has and bypass the challenge on
///   `POST /app/urls`.
pub async fn create(
    State(state): State<AppState>,
    client: ClientInfo,
    CurrentUser { user, token_id, .. }: CurrentUser,
    Json(payload): Json<CreatePayload>,
) -> Result<Response, ApiError> {
    state
        .limiter
        .check(
            &format!("create-url:{}", user.id),
            limits::CREATE_URL_CAPACITY,
            limits::CREATE_URL_REFILL_PER_SEC,
        )
        .map_err(|retry| ApiError(AppError::rate_limited(retry)))?;

    // `token_id` is only set for Bearer authentication.
    if token_id.is_none() {
        verify_captcha(
            &state,
            &client,
            CaptchaFlow::Create,
            payload.captcha_token.as_deref(),
        )
        .await?;
    }

    let context = context(&state, &client, &user);
    let url = context
        .create(CreateInput {
            target_url: payload.target_url,
            slug: payload.slug,
            title: payload.title,
            notes: payload.notes,
            tags: payload.tags,
            password: payload.password,
            expires_at: payload.expires_at,
            max_clicks: payload.max_clicks,
            redirect_type: payload.redirect_type,
        })
        .await?;

    let dto = to_dto(&state, &url).await?;
    Ok((StatusCode::CREATED, Json(dto)).into_response())
}

/// `GET /api/v1/urls/{id}`
pub async fn get(
    State(state): State<AppState>,
    CurrentUser { user, .. }: CurrentUser,
    Path(id): Path<i64>,
) -> Result<Json<UrlDto>, ApiError> {
    let url = urls::find_owned(&state.db, id, user.id)
        .await?
        .ok_or_else(|| ApiError(AppError::not_found()))?;
    Ok(Json(to_dto(&state, &url).await?))
}

/// `PATCH /api/v1/urls/{id}`
pub async fn update(
    State(state): State<AppState>,
    client: ClientInfo,
    CurrentUser { user, .. }: CurrentUser,
    Path(id): Path<i64>,
    Json(payload): Json<UpdatePayload>,
) -> Result<Json<UrlDto>, ApiError> {
    let context = context(&state, &client, &user);
    let existing = context.find_owned(id).await?;
    let updated = context
        .update(
            &existing,
            UpdateInput {
                target_url: payload.target_url,
                slug: payload.slug,
                title: payload.title,
                notes: payload.notes,
                tags: payload.tags,
                password: payload.password,
                expires_at: payload.expires_at,
                max_clicks: payload.max_clicks,
                redirect_type: payload.redirect_type,
                status: payload.status,
            },
        )
        .await?;
    Ok(Json(to_dto(&state, &updated).await?))
}

/// `DELETE /api/v1/urls/{id}`
pub async fn delete(
    State(state): State<AppState>,
    client: ClientInfo,
    CurrentUser { user, .. }: CurrentUser,
    Path(id): Path<i64>,
) -> Result<Response, ApiError> {
    let context = context(&state, &client, &user);
    let existing = context.find_owned(id).await?;
    context.delete(&existing).await?;
    Ok(StatusCode::NO_CONTENT.into_response())
}

fn context<'a>(
    state: &'a AppState,
    client: &'a ClientInfo,
    user: &'a crate::db::users::User,
) -> UrlContext<'a> {
    UrlContext {
        pool: &state.db,
        settings: &state.settings,
        actor: user,
        ip_hash: client.ip_hash.as_deref(),
    }
}

async fn to_dto(state: &AppState, url: &urls::Url) -> Result<UrlDto, ApiError> {
    let names = tags::for_url(&state.db, url.id)
        .await?
        .into_iter()
        .map(|tag| tag.name)
        .collect();
    Ok(UrlDto::new(url, &state.config.base_url, names))
}

fn clean(value: &Option<String>) -> Option<String> {
    value
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

/// Filters used by the administrative listing (shared with the admin routes).
pub fn admin_filter(
    search: &Option<String>,
    status: &Option<String>,
    user_id: Option<i64>,
    sort: &Option<String>,
) -> AdminUrlFilter {
    AdminUrlFilter {
        search: clean(search),
        status: clean(status),
        user_id,
        sort: UrlSort::parse(sort.as_deref()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn patch_payload_distinguishes_absent_from_null() {
        let untouched: UpdatePayload = serde_json::from_value(json!({})).expect("empty patch");
        assert!(untouched.title.is_none(), "absent means unchanged");
        assert!(untouched.password.is_none());

        let cleared: UpdatePayload =
            serde_json::from_value(json!({ "title": null, "password": null }))
                .expect("clear patch");
        assert_eq!(cleared.title, Some(None), "null means clear");
        assert_eq!(cleared.password, Some(None));

        let set: UpdatePayload =
            serde_json::from_value(json!({ "title": "Hello", "max_clicks": 5 }))
                .expect("set patch");
        assert_eq!(set.title, Some(Some("Hello".to_string())));
        assert_eq!(set.max_clicks, Some(Some(5)));
    }

    #[test]
    fn list_query_cleans_blank_filters() {
        let query = ListQuery {
            q: Some("   ".to_string()),
            tag: None,
            ..Default::default()
        };
        assert!(query.filter().search.is_none());
        let query = ListQuery {
            q: Some(" promo ".to_string()),
            ..Default::default()
        };
        assert_eq!(query.filter().search.as_deref(), Some("promo"));
    }
}
