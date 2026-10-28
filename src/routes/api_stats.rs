//! `/api/v1/urls/{id}/stats` and `/api/v1/urls/{id}/clicks`.

use crate::routes::extract::Json;
use axum::extract::{Path, Query, State};
use serde::{Deserialize, Serialize};

use crate::db::clicks::{self, Click, ClickSummary, Dimension};
use crate::db::{tags, urls};
use crate::error::{ApiError, AppError};
use crate::middleware::auth::CurrentUser;
use crate::routes::dto::{
    ClickDto, DailyPointDto, DimensionDto, Page, Pagination, StatsSummaryDto, UrlDto, UrlStatsDto,
};
use crate::services::chart;
use crate::state::AppState;

/// Query for the statistics endpoint.
#[derive(Debug, Clone, Deserialize)]
pub struct StatsQuery {
    #[serde(default)]
    pub days: Option<i64>,
}

/// Query for the click log endpoint.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct ClicksQuery {
    #[serde(default)]
    pub page: Option<i64>,
    #[serde(default)]
    pub per_page: Option<i64>,
}

#[derive(Debug, Serialize)]
pub struct ClickList {
    pub items: Vec<ClickDto>,
    pub page: i64,
    pub per_page: i64,
    pub total: i64,
    pub total_pages: i64,
}

/// `GET /api/v1/urls/{id}/stats`
pub async fn stats(
    State(state): State<AppState>,
    CurrentUser { user, .. }: CurrentUser,
    Path(id): Path<i64>,
    Query(query): Query<StatsQuery>,
) -> Result<Json<UrlStatsDto>, ApiError> {
    let url = urls::find_owned(&state.db, id, user.id)
        .await?
        .ok_or_else(|| ApiError(AppError::not_found()))?;

    let days = query.days.unwrap_or(30).clamp(1, 365);
    let from_day = crate::services::time::day_days_ago(days - 1);

    let summary = clicks::summary_for_url(&state.db, url.id).await?;
    let series = clicks::daily_series(&state.db, url.id, &from_day).await?;
    let referrers = clicks::breakdown_for_url(&state.db, url.id, Dimension::Referrer, 10).await?;
    let devices = clicks::breakdown_for_url(&state.db, url.id, Dimension::Device, 10).await?;
    let browsers = clicks::breakdown_for_url(&state.db, url.id, Dimension::Browser, 10).await?;
    let operating_systems = clicks::breakdown_for_url(&state.db, url.id, Dimension::Os, 10).await?;

    let tag_names = tags::for_url(&state.db, url.id)
        .await?
        .into_iter()
        .map(|tag| tag.name)
        .collect();

    Ok(Json(UrlStatsDto {
        url: UrlDto::new(
            &url,
            &state.domain_hostname(url.domain_id),
            &state.config.base_scheme,
            tag_names,
        ),
        summary: summary_dto(&summary),
        daily: chart::fill_days(&series, days),
        referrers: dimensions(referrers),
        devices: dimensions(devices),
        browsers: dimensions(browsers),
        operating_systems: dimensions(operating_systems),
    }))
}

/// `GET /api/v1/urls/{id}/clicks`
pub async fn clicks_log(
    State(state): State<AppState>,
    CurrentUser { user, .. }: CurrentUser,
    Path(id): Path<i64>,
    Query(query): Query<ClicksQuery>,
) -> Result<Json<ClickList>, ApiError> {
    let url = urls::find_owned(&state.db, id, user.id)
        .await?
        .ok_or_else(|| ApiError(AppError::not_found()))?;

    let pagination = Pagination {
        page: query.page,
        per_page: query.per_page,
    };
    let rows = clicks::recent_for_url(
        &state.db,
        url.id,
        pagination.per_page(),
        pagination.offset(),
    )
    .await?;
    let total = clicks::count_for_url(&state.db, url.id).await?;
    let page = Page::new(
        rows.iter().map(click_dto).collect::<Vec<_>>(),
        pagination.page(),
        pagination.per_page(),
        total,
    );

    Ok(Json(ClickList {
        items: page.items,
        page: page.page,
        per_page: page.per_page,
        total: page.total,
        total_pages: page.total_pages,
    }))
}

/// Convert an aggregate row into the public shape.
pub fn summary_dto(summary: &ClickSummary) -> StatsSummaryDto {
    StatsSummaryDto {
        total_clicks: summary.total_clicks,
        human_clicks: summary.human_clicks(),
        unique_clicks: summary.unique_clicks,
        bot_clicks: summary.bot_clicks,
        first_click_at: summary.first_click_at.clone(),
        last_click_at: summary.last_click_at.clone(),
    }
}

/// Convert click rows into the public shape.
pub fn click_dto(click: &Click) -> ClickDto {
    ClickDto {
        clicked_at: click.clicked_at.clone(),
        referrer_host: click.referrer_host.clone(),
        country_code: click.country_code.clone(),
        device_type: click.device_type.clone(),
        browser: click.browser.clone(),
        os: click.os.clone(),
        is_bot: click.is_bot == 1,
        is_unique: click.is_unique == 1,
        password_protected: click.password_protected == 1,
    }
}

/// Convert breakdown rows into the public shape.
pub fn dimensions(rows: Vec<clicks::DimensionCount>) -> Vec<DimensionDto> {
    rows.into_iter()
        .map(|row| DimensionDto {
            label: row.label,
            count: row.count,
        })
        .collect()
}

/// Daily points helper shared with the HTML dashboard.
pub fn daily_points(series: &[clicks::DailyStat], days: i64) -> Vec<DailyPointDto> {
    chart::fill_days(series, days)
}
