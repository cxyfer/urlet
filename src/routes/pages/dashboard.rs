//! Dashboard page: account overview plus the link list.

use askama::Template;
use axum::extract::{Query, State};
use axum::http::StatusCode;
use axum::response::{Html, IntoResponse, Response};
use serde::Deserialize;

use crate::db::urls::{UrlSort, UserUrlFilter};
use crate::db::{clicks, tags, urls, user_groups};
use crate::error::AppError;
use crate::i18n::{self, Lang};
use crate::middleware::auth::MaybeUser;
use crate::routes::dto::{DailyPointDto, DimensionDto, OverviewDto, TagDto, TopLinkDto, UrlDto};
use crate::routes::pages::{
    flash_from_query, page_links, redirect_to_login, Chrome, Flash, PageLink, UrlRow,
};
use crate::services::captcha::CaptchaFlow;
use crate::services::domains::DomainOption;
use crate::services::{chart, limits, time};
use crate::state::AppState;

#[derive(Debug, Template)]
#[template(path = "dashboard.html")]
pub struct DashboardPage {
    pub site_name: String,
    pub chrome: Chrome,
    pub overview: OverviewDto,
    pub chart: String,
    pub chart_max: i64,
    pub urls: Vec<UrlRow>,
    pub tags: Vec<TagDto>,
    pub links: Vec<PageLink>,
    pub page: i64,
    pub total_pages: i64,
    pub total: i64,
    pub q: String,
    pub tag: String,
    pub status: String,
    pub sort: String,
    pub quota: i64,
    pub used: i64,
    /// Name of the user's group, empty when the account is ungrouped.
    pub group_name: String,
    pub allow_custom_slug: bool,
    pub allow_password: bool,
    pub allow_expiry: bool,
    pub allow_max_clicks: bool,
    pub allow_tags: bool,
    pub allow_domain_selection: bool,
    pub default_redirect_type: i64,
    /// Domains selectable in the create form.
    pub domains: Vec<DomainOption>,
    /// Provider widget for the create form, or `None` when it is not protected.
    pub captcha: Option<crate::services::captcha::CaptchaWidget>,
    /// Localised "3 of 10 links used" line.
    pub usage: String,
    /// Localised "Page 2 of 5 (30 links)" line.
    pub pagination_note: String,
    pub lang: Lang,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct DashboardQuery {
    #[serde(default)]
    pub page: Option<i64>,
    #[serde(default)]
    pub q: Option<String>,
    #[serde(default)]
    pub tag: Option<String>,
    #[serde(default)]
    pub status: Option<String>,
    #[serde(default)]
    pub sort: Option<String>,
    #[serde(default)]
    pub ok: Option<String>,
    #[serde(default)]
    pub err: Option<String>,
}

/// `GET /app`
pub async fn dashboard(
    State(state): State<AppState>,
    MaybeUser(current): MaybeUser,
    Query(query): Query<DashboardQuery>,
) -> Result<Response, AppError> {
    let Some(current) = current else {
        return Ok(redirect_to_login(None).into_response());
    };
    let user = current.user;
    let flash = flash_from_query(query.ok.as_deref(), query.err.as_deref());

    let page = query.page.unwrap_or(1).max(1);
    let per_page = 20i64;
    let filter = UserUrlFilter {
        search: clean(&query.q),
        status: clean(&query.status),
        tag: clean(&query.tag),
        sort: UrlSort::parse(query.sort.as_deref()),
    };

    let rows = urls::list_for_user(&state.db, user.id, &filter, per_page, (page - 1) * per_page)
        .await
        .map_err(AppError::from)?;
    let total = urls::count_for_user_filtered(&state.db, user.id, &filter)
        .await
        .map_err(AppError::from)?;
    let ids: Vec<i64> = rows.iter().map(|row| row.id).collect();
    let mut tag_map = tags::for_urls(&state.db, &ids)
        .await
        .map_err(AppError::from)?;

    let url_rows: Vec<UrlRow> = rows
        .iter()
        .map(|row| {
            let names = tag_map
                .remove(&row.id)
                .unwrap_or_default()
                .into_iter()
                .map(|tag| tag.name)
                .collect();
            UrlRow::new(&UrlDto::new(
                row,
                &state.domain_hostname(row.domain_id),
                &state.config.base_scheme,
                names,
            ))
        })
        .collect();

    let overview = build_overview(&state, user.id).await?;
    let chart_svg = chart::svg_bars(&overview.daily);
    let total_pages = ((total + per_page - 1) / per_page).max(1);

    let mut params: Vec<(&str, String)> = Vec::new();
    if let Some(value) = clean(&query.q) {
        params.push(("q", value));
    }
    if let Some(value) = clean(&query.tag) {
        params.push(("tag", value));
    }
    if let Some(value) = clean(&query.status) {
        params.push(("status", value));
    }
    if let Some(value) = clean(&query.sort) {
        params.push(("sort", value));
    }

    let tag_list = tags::list_for_user(&state.db, user.id)
        .await
        .map_err(AppError::from)?;

    let lang = i18n::current();
    let effective = limits::effective(&state.db, &state.settings, &user).await?;
    let quota = effective.max_urls;
    let group_name = match user.group_id {
        Some(group_id) => user_groups::find_by_id(&state.db, group_id)
            .await
            .map_err(AppError::from)?
            .map(|group| group.name)
            .unwrap_or_default(),
        None => String::new(),
    };
    let used = urls::count_for_user(&state.db, user.id)
        .await
        .map_err(AppError::from)?;

    let page_render = DashboardPage {
        site_name: state.settings.site_name(),
        chrome: Chrome::new(&state, &user, "dashboard", flash),
        overview,
        chart: chart_svg,
        chart_max: 1,
        urls: url_rows,
        tags: tag_list.iter().map(TagDto::from).collect(),
        links: page_links("/app", &params, page, total_pages),
        page,
        total_pages,
        total,
        q: clean(&query.q).unwrap_or_default(),
        tag: clean(&query.tag).unwrap_or_default(),
        status: clean(&query.status).unwrap_or_default(),
        sort: clean(&query.sort).unwrap_or_default(),
        quota,
        used,
        group_name,
        allow_custom_slug: effective.allow_custom_slug,
        allow_password: effective.allow_password,
        allow_expiry: effective.allow_expiry,
        allow_max_clicks: effective.allow_max_clicks,
        allow_tags: effective.allow_tags,
        allow_domain_selection: effective.allow_domain_selection,
        default_redirect_type: state.settings.default_redirect_type(),
        domains: state.domains.active_options(),
        captcha: crate::routes::pages::captcha_widget(&state, CaptchaFlow::Create),
        usage: i18n::format(
            lang,
            "dashboard.usage",
            &[
                ("used", used.to_string()),
                (
                    "quota",
                    if quota > 0 {
                        quota.to_string()
                    } else {
                        lang.t("common.unlimited").to_string()
                    },
                ),
            ],
        ),
        pagination_note: i18n::format(
            lang,
            "dashboard.pagination_note",
            &[
                ("page", page.to_string()),
                ("total", total_pages.to_string()),
                ("count", total.to_string()),
            ],
        ),
        lang,
    };

    match page_render.render() {
        Ok(html) => Ok((StatusCode::OK, Html(html)).into_response()),
        Err(err) => Err(AppError::internal(format!("template error: {err}"))),
    }
}

/// Aggregate the numbers shown on the dashboard cards.
pub async fn build_overview(state: &AppState, user_id: i64) -> Result<OverviewDto, AppError> {
    let links = urls::count_for_user(&state.db, user_id)
        .await
        .map_err(AppError::from)?;
    let expired = urls::count_expired_for_user(&state.db, user_id, &time::now_iso())
        .await
        .map_err(AppError::from)?;
    let summary = clicks::user_summary(&state.db, user_id)
        .await
        .map_err(AppError::from)?;
    let clicks_7d = clicks::user_clicks_since(&state.db, user_id, &time::iso_days_ago(7))
        .await
        .map_err(AppError::from)?;
    let clicks_30d = clicks::user_clicks_since(&state.db, user_id, &time::iso_days_ago(30))
        .await
        .map_err(AppError::from)?;
    let series = clicks::user_daily_series(&state.db, user_id, &time::day_days_ago(13))
        .await
        .map_err(AppError::from)?;
    let top = clicks::top_urls_for_user(&state.db, user_id, 5)
        .await
        .map_err(AppError::from)?;

    let bot_share = if summary.total_clicks > 0 {
        ((summary.bot_clicks as f64 / summary.total_clicks as f64) * 100.0).round() as i64
    } else {
        0
    };

    Ok(OverviewDto {
        links,
        active_links: (links - expired).max(0),
        expired_links: expired,
        total_clicks: summary.total_clicks,
        unique_clicks: summary.unique_clicks,
        bot_clicks: summary.bot_clicks,
        bot_share,
        clicks_7d,
        clicks_30d,
        daily: chart::fill_days(&series, 14),
        top_links: top
            .into_iter()
            .map(|(id, slug, target_url, click_count)| TopLinkDto {
                id,
                slug,
                target_url,
                click_count,
            })
            .collect(),
    })
}

/// Convert a breakdown into template friendly bar rows.
pub fn bar_rows(items: Vec<DimensionDto>) -> Vec<BarRow> {
    let max = chart::max_count(&items);
    items
        .into_iter()
        .map(|item| BarRow {
            percent: chart::bar_percent(item.count, max),
            label: item.label,
            count: item.count,
        })
        .collect()
}

/// One labelled bar in a breakdown list.
#[derive(Debug, Clone)]
pub struct BarRow {
    pub label: String,
    pub count: i64,
    pub percent: i64,
}

/// Convenience for the chart legend.
pub fn total_of(points: &[DailyPointDto]) -> i64 {
    points.iter().map(|point| point.clicks).sum()
}

/// Trim a query parameter and drop it when empty.
pub fn clean(value: &Option<String>) -> Option<String> {
    value
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

/// Shared flash helper for pages that only need a banner.
pub fn flash(ok: Option<&str>, err: Option<&str>) -> Option<Flash> {
    flash_from_query(ok, err)
}
