//! Link creation, detail (with statistics) and editing pages.

use askama::Template;
use axum::extract::rejection::FormRejection;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::{Html, IntoResponse, Response};
use axum::Form;
use serde::Deserialize;

use crate::db::clicks::{self, Click, Dimension};
use crate::db::{tags, urls};
use crate::error::AppError;
use crate::middleware::auth::{ClientInfo, MaybeUser};
use crate::routes::api_stats::{click_dto, summary_dto};
use crate::routes::dto::{StatsSummaryDto, UrlDto};
use crate::routes::pages::dashboard::{bar_rows, BarRow};
use crate::routes::pages::{
    flash_from_query, redirect_err, redirect_ok, redirect_to_login, Chrome, UrlRow,
};
use crate::services::ratelimit::limits;
use crate::services::urls::{CreateInput, UpdateInput, UrlContext};
use crate::services::{chart, time};
use crate::state::AppState;

#[derive(Debug, Template)]
#[template(path = "url_detail.html")]
pub struct UrlDetailPage {
    pub site_name: String,
    pub chrome: Chrome,
    pub url: UrlRow,
    pub summary: StatsSummaryDto,
    pub chart: String,
    pub daily_total: i64,
    pub referrers: Vec<BarRow>,
    pub devices: Vec<BarRow>,
    pub browsers: Vec<BarRow>,
    pub operating_systems: Vec<BarRow>,
    pub clicks: Vec<ClickRow>,
    pub tags_csv: String,
    pub days: i64,
}

/// One row of the recent click log.
#[derive(Debug, Clone)]
pub struct ClickRow {
    pub when: String,
    pub referrer: String,
    pub device: String,
    pub browser: String,
    pub os: String,
    pub kind: &'static str,
    pub kind_class: &'static str,
}

impl ClickRow {
    fn new(click: &Click) -> Self {
        let dto = click_dto(click);
        let (kind, kind_class) = if dto.is_bot {
            ("Bot", "badge-muted")
        } else if dto.is_unique {
            ("Unique", "badge-accent")
        } else {
            ("Repeat", "badge")
        };
        Self {
            when: time::humanize(&dto.clicked_at),
            referrer: dto.referrer_host.unwrap_or_else(|| "direct".to_string()),
            device: dto.device_type.unwrap_or_else(|| "Unknown".to_string()),
            browser: dto.browser.unwrap_or_else(|| "Other".to_string()),
            os: dto.os.unwrap_or_else(|| "Unknown".to_string()),
            kind,
            kind_class,
        }
    }
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct DetailQuery {
    #[serde(default)]
    pub days: Option<i64>,
    #[serde(default)]
    pub ok: Option<String>,
    #[serde(default)]
    pub err: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct CreateForm {
    pub target_url: String,
    #[serde(default)]
    pub slug: Option<String>,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub tags: Option<String>,
    #[serde(default)]
    pub password: Option<String>,
    #[serde(default)]
    pub expires_at: Option<String>,
    #[serde(default)]
    pub max_clicks: Option<String>,
    #[serde(default)]
    pub redirect_type: Option<String>,
}

/// Quick actions offered next to an expired or capped link (spec FR-EXP-3).
#[derive(Debug, Deserialize)]
pub struct ExtendForm {
    #[serde(default)]
    pub days: Option<String>,
    #[serde(default)]
    pub clicks: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct UpdateForm {
    #[serde(default)]
    pub target_url: Option<String>,
    #[serde(default)]
    pub slug: Option<String>,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub notes: Option<String>,
    #[serde(default)]
    pub tags: Option<String>,
    #[serde(default)]
    pub password: Option<String>,
    #[serde(default)]
    pub clear_password: Option<String>,
    #[serde(default)]
    pub expires_at: Option<String>,
    #[serde(default)]
    pub clear_expiry: Option<String>,
    #[serde(default)]
    pub max_clicks: Option<String>,
    #[serde(default)]
    pub clear_max_clicks: Option<String>,
    #[serde(default)]
    pub redirect_type: Option<String>,
    #[serde(default)]
    pub status: Option<String>,
}

/// Split a comma separated tag field.
fn split_tags(raw: &Option<String>) -> Vec<String> {
    raw.as_deref()
        .unwrap_or_default()
        .split(',')
        .map(str::trim)
        .filter(|tag| !tag.is_empty())
        .map(str::to_string)
        .collect()
}

fn optional_number(raw: &Option<String>) -> Option<i64> {
    raw.as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .and_then(|value| value.parse().ok())
}

fn checkbox(raw: &Option<String>) -> bool {
    matches!(
        raw.as_deref().map(str::trim),
        Some("on") | Some("1") | Some("true")
    )
}

/// `POST /app/urls`
pub async fn create(
    State(state): State<AppState>,
    client: ClientInfo,
    MaybeUser(current): MaybeUser,
    form: std::result::Result<Form<CreateForm>, FormRejection>,
) -> Result<Response, AppError> {
    let Some(current) = current else {
        return Ok(redirect_to_login(Some("/app")).into_response());
    };
    let Ok(Form(payload)) = form else {
        return Ok(redirect_err("/app", "invalid").into_response());
    };

    if state
        .limiter
        .check(
            &format!("create-url:{}", current.user.id),
            limits::CREATE_URL_CAPACITY,
            limits::CREATE_URL_REFILL_PER_SEC,
        )
        .is_err()
    {
        return Ok(redirect_err("/app", "rate_limited").into_response());
    }

    let context = UrlContext {
        pool: &state.db,
        settings: &state.settings,
        actor: &current.user,
        ip_hash: client.ip_hash.as_deref(),
    };

    let outcome = context
        .create(CreateInput {
            target_url: payload.target_url,
            slug: payload.slug,
            title: payload.title,
            notes: None,
            tags: split_tags(&payload.tags),
            password: payload.password,
            expires_at: payload.expires_at,
            max_clicks: optional_number(&payload.max_clicks),
            redirect_type: optional_number(&payload.redirect_type),
        })
        .await;

    Ok(match outcome {
        Ok(url) => redirect_ok(&format!("/app/urls/{}", url.id), "created").into_response(),
        Err(err) => redirect_err("/app", error_code(&err)).into_response(),
    })
}

/// `GET /app/urls/{id}`
pub async fn detail(
    State(state): State<AppState>,
    MaybeUser(current): MaybeUser,
    Path(id): Path<i64>,
    Query(query): Query<DetailQuery>,
) -> Result<Response, AppError> {
    let Some(current) = current else {
        return Ok(redirect_to_login(Some(&format!("/app/urls/{id}"))).into_response());
    };
    let user = current.user;

    let Some(url) = urls::find_owned(&state.db, id, user.id)
        .await
        .map_err(AppError::from)?
    else {
        return Ok(redirect_err("/app", "not_found").into_response());
    };

    let days = query.days.unwrap_or(30).clamp(1, 365);
    let from_day = time::day_days_ago(days - 1);
    let summary = clicks::summary_for_url(&state.db, url.id)
        .await
        .map_err(AppError::from)?;
    let series = clicks::daily_series(&state.db, url.id, &from_day)
        .await
        .map_err(AppError::from)?;
    let daily = chart::fill_days(&series, days);

    let referrers = clicks::breakdown_for_url(&state.db, url.id, Dimension::Referrer, 8)
        .await
        .map_err(AppError::from)?;
    let devices = clicks::breakdown_for_url(&state.db, url.id, Dimension::Device, 8)
        .await
        .map_err(AppError::from)?;
    let browsers = clicks::breakdown_for_url(&state.db, url.id, Dimension::Browser, 8)
        .await
        .map_err(AppError::from)?;
    let operating_systems = clicks::breakdown_for_url(&state.db, url.id, Dimension::Os, 8)
        .await
        .map_err(AppError::from)?;

    // Spec FR-STAT-2: the detail page shows the last 50 clicks.
    let recent = clicks::recent_for_url(&state.db, url.id, 50, 0)
        .await
        .map_err(AppError::from)?;
    let tag_names: Vec<String> = tags::for_url(&state.db, url.id)
        .await
        .map_err(AppError::from)?
        .into_iter()
        .map(|tag| tag.name)
        .collect();

    let dto = UrlDto::new(&url, &state.config.base_url, tag_names.clone());
    let page = UrlDetailPage {
        site_name: state.settings.site_name(),
        chrome: Chrome::new(
            &state,
            &user,
            "dashboard",
            flash_from_query(query.ok.as_deref(), query.err.as_deref()),
        ),
        url: UrlRow::new(&dto),
        summary: summary_dto(&summary),
        chart: chart::svg_bars(&daily),
        daily_total: daily.iter().map(|point| point.clicks).sum(),
        referrers: bar_rows(dimension_dtos(referrers)),
        devices: bar_rows(dimension_dtos(devices)),
        browsers: bar_rows(dimension_dtos(browsers)),
        operating_systems: bar_rows(dimension_dtos(operating_systems)),
        clicks: recent.iter().map(ClickRow::new).collect(),
        tags_csv: tag_names.join(", "),
        days,
    };

    match page.render() {
        Ok(html) => Ok((StatusCode::OK, Html(html)).into_response()),
        Err(err) => Err(AppError::internal(format!("template error: {err}"))),
    }
}

/// `POST /app/urls/{id}`
pub async fn update(
    State(state): State<AppState>,
    client: ClientInfo,
    MaybeUser(current): MaybeUser,
    Path(id): Path<i64>,
    form: std::result::Result<Form<UpdateForm>, FormRejection>,
) -> Result<Response, AppError> {
    let Some(current) = current else {
        return Ok(redirect_to_login(Some(&format!("/app/urls/{id}"))).into_response());
    };
    let target_page = format!("/app/urls/{id}");
    let Ok(Form(payload)) = form else {
        return Ok(redirect_err(&target_page, "invalid").into_response());
    };

    let context = UrlContext {
        pool: &state.db,
        settings: &state.settings,
        actor: &current.user,
        ip_hash: client.ip_hash.as_deref(),
    };

    let existing = match context.find_owned(id).await {
        Ok(url) => url,
        Err(_) => return Ok(redirect_err("/app", "not_found").into_response()),
    };

    // The password field is empty by default: only touch it when a value was
    // typed or the "remove password" box was ticked.
    let password = if checkbox(&payload.clear_password) {
        Some(None)
    } else {
        match payload.password.as_deref().map(str::trim) {
            Some("") | None => None,
            Some(value) => Some(Some(value.to_string())),
        }
    };

    let expires_at = if checkbox(&payload.clear_expiry) {
        Some(None)
    } else {
        match payload.expires_at.as_deref().map(str::trim) {
            Some("") | None => None,
            Some(value) => Some(Some(value.to_string())),
        }
    };

    let max_clicks = if checkbox(&payload.clear_max_clicks) {
        Some(None)
    } else {
        optional_number(&payload.max_clicks).map(Some)
    };

    let outcome = context
        .update(
            &existing,
            UpdateInput {
                target_url: payload.target_url,
                slug: payload.slug,
                title: Some(payload.title.clone()),
                notes: Some(payload.notes.clone()),
                tags: Some(split_tags(&payload.tags)),
                password,
                expires_at,
                max_clicks,
                redirect_type: optional_number(&payload.redirect_type),
                status: payload.status,
            },
        )
        .await;

    Ok(match outcome {
        Ok(_) => redirect_ok(&target_page, "updated").into_response(),
        Err(err) => redirect_err(&target_page, error_code(&err)).into_response(),
    })
}

/// `POST /app/urls/{id}/delete`
pub async fn delete(
    State(state): State<AppState>,
    client: ClientInfo,
    MaybeUser(current): MaybeUser,
    Path(id): Path<i64>,
) -> Result<Response, AppError> {
    let Some(current) = current else {
        return Ok(redirect_to_login(Some("/app")).into_response());
    };
    let context = UrlContext {
        pool: &state.db,
        settings: &state.settings,
        actor: &current.user,
        ip_hash: client.ip_hash.as_deref(),
    };

    Ok(match context.find_owned(id).await {
        Ok(url) => match context.delete(&url).await {
            Ok(()) => redirect_ok("/app", "deleted").into_response(),
            Err(err) => redirect_err("/app", error_code(&err)).into_response(),
        },
        Err(_) => redirect_err("/app", "not_found").into_response(),
    })
}

/// `POST /app/urls/{id}/extend` — push the expiry out or raise the click limit
/// without opening the full edit form.
pub async fn extend(
    State(state): State<AppState>,
    client: ClientInfo,
    MaybeUser(current): MaybeUser,
    Path(id): Path<i64>,
    form: std::result::Result<Form<ExtendForm>, FormRejection>,
) -> Result<Response, AppError> {
    let Some(current) = current else {
        return Ok(redirect_to_login(Some(&format!("/app/urls/{id}"))).into_response());
    };
    let target_page = format!("/app/urls/{id}");
    let Ok(Form(payload)) = form else {
        return Ok(redirect_err(&target_page, "invalid").into_response());
    };

    let context = UrlContext {
        pool: &state.db,
        settings: &state.settings,
        actor: &current.user,
        ip_hash: client.ip_hash.as_deref(),
    };
    let Ok(url) = context.find_owned(id).await else {
        return Ok(redirect_err("/app", "not_found").into_response());
    };

    let mut update = UpdateInput::default();
    if let Some(days) = optional_number(&payload.days).filter(|days| (1..=3650).contains(days)) {
        // Extend from the later of "now" and the current expiry.
        let base = url
            .expires_at
            .as_deref()
            .and_then(time::parse_iso)
            .filter(|instant| *instant > time::now())
            .unwrap_or_else(time::now);
        update.expires_at = Some(Some(time::to_iso(time::plus_days(base, days))));
    }
    if let Some(clicks) =
        optional_number(&payload.clicks).filter(|clicks| (1..=1_000_000).contains(clicks))
    {
        let base = url.max_clicks.unwrap_or(0).max(url.click_count);
        update.max_clicks = Some(Some(base + clicks));
    }

    if update.expires_at.is_none() && update.max_clicks.is_none() {
        return Ok(redirect_err(&target_page, "invalid").into_response());
    }

    Ok(match context.update(&url, update).await {
        Ok(_) => redirect_ok(&target_page, "updated").into_response(),
        Err(err) => redirect_err(&target_page, error_code(&err)).into_response(),
    })
}

fn dimension_dtos(rows: Vec<clicks::DimensionCount>) -> Vec<crate::routes::dto::DimensionDto> {
    rows.into_iter()
        .map(|row| crate::routes::dto::DimensionDto {
            label: row.label,
            count: row.count,
        })
        .collect()
}

/// Map a service error onto a flash code the page understands.
pub fn error_code(err: &AppError) -> &'static str {
    match err {
        AppError::Conflict(_) => "conflict",
        AppError::NotFound => "not_found",
        AppError::Forbidden(message) if message.contains("limit") => "quota",
        AppError::Forbidden(_) => "denied",
        AppError::RateLimited { .. } => "rate_limited",
        AppError::Validation { field, .. } => match *field {
            "expires_at" => "expiry",
            "slug" => "slug",
            "target_url" => "target",
            "password" => "password_rules",
            "tags" => "tags",
            _ => "invalid",
        },
        AppError::BadRequest(_) => "invalid",
        AppError::Unauthorized(_) => "password",
        AppError::Gone(_) | AppError::Internal(_) => "invalid",
    }
}
