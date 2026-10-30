//! Link creation, detail (with statistics) and editing pages.

use askama::Template;
use axum::extract::rejection::FormRejection;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::{Html, IntoResponse, Response};
use axum::Form;
use serde::Deserialize;

use crate::db::clicks::{self, Click, Dimension};
use crate::db::url_previews::{Changes, Preview};
use crate::db::users::User;
use crate::db::{tags, urls};
use crate::error::AppError;
use crate::i18n::{self, Lang};
use crate::middleware::auth::{ClientInfo, MaybeUser};
use crate::routes::api_stats::{click_dto, summary_dto};
use crate::routes::dto::{StatsSummaryDto, UrlDto};
use crate::routes::pages::dashboard::{bar_rows, BarRow};
use crate::routes::pages::link_form::{LinkForm, LinkSubmission, Rejected, Submission};
use crate::routes::pages::{
    captcha_rejected, flash_from_query, redirect_err, redirect_ok, redirect_to_login, safe_next,
    Chrome, UrlRow,
};
use crate::routes::previews::{self, PreviewFields};
use crate::services::captcha::CaptchaFlow;
use crate::services::domain_policy;
use crate::services::domains::DomainOption;
use crate::services::ratelimit::limits;
use crate::services::urls::{CreateInput, UpdateInput, UrlContext};
use crate::services::{chart, time};
use crate::state::AppState;
use crate::themed_page;

#[derive(Debug)]
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
    pub days: i64,
    /// The quick "extend" actions follow the group's switches.
    pub allow_expiry: bool,
    pub allow_max_clicks: bool,
    /// Whether the link has a custom SNS preview.
    pub has_preview: bool,
    /// Localised "30 days ago" label of the chart axis.
    pub days_label: String,
    /// Localised "42 clicks in this window; grey segments are bots" legend.
    pub chart_legend: String,
    pub lang: Lang,
}

themed_page!(url_detail_templates, UrlDetailView, UrlDetailPage,
    Verdant => "url_detail.html",
    Aurora => "themes/aurora/url_detail.html",
    Paper => "themes/paper/url_detail.html",
    Signal => "themes/signal/url_detail.html",
    Gilded => "themes/gilded/url_detail.html",
    Swiss => "themes/swiss/url_detail.html",
);

/// The link editor: the dialog loads this page, and it is the whole editor
/// when JavaScript is unavailable.
#[derive(Debug)]
pub struct UrlEditPage {
    pub site_name: String,
    pub chrome: Chrome,
    pub url: UrlRow,
    pub link_form: LinkForm,
    /// Always `None`: the shared form only shows bot protection when creating.
    pub captcha: Option<crate::services::captcha::CaptchaWidget>,
    pub lang: Lang,
}

themed_page!(url_edit_templates, UrlEditView, UrlEditPage,
    Verdant => "url_edit.html",
    Aurora => "themes/aurora/url_edit.html",
    Paper => "themes/paper/url_edit.html",
    Signal => "themes/signal/url_edit.html",
    Gilded => "themes/gilded/url_edit.html",
    Swiss => "themes/swiss/url_edit.html",
);

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
    fn new(click: &Click, lang: Lang) -> Self {
        let dto = click_dto(click);
        let (kind, kind_class) = if dto.is_bot {
            (lang.t("click.kind.bot"), "badge-muted")
        } else if dto.is_unique {
            (lang.t("click.kind.unique"), "badge-accent")
        } else {
            (lang.t("click.kind.repeat"), "badge")
        };
        Self {
            when: time::humanize_lang(lang, &dto.clicked_at),
            // The same mapper as the breakdown charts, so a missing referrer or
            // an unparsable user agent reads the same in both places.
            referrer: dimension_label(
                Dimension::Referrer,
                dto.referrer_host.as_deref().unwrap_or("direct"),
                lang,
            ),
            device: dimension_label(
                Dimension::Device,
                dto.device_type.as_deref().unwrap_or("Unknown"),
                lang,
            ),
            browser: dimension_label(
                Dimension::Browser,
                dto.browser.as_deref().unwrap_or("Other"),
                lang,
            ),
            os: dimension_label(Dimension::Os, dto.os.as_deref().unwrap_or("Unknown"), lang),
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
    pub domain: Option<String>,
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
    #[serde(default)]
    pub redirect_mode: Option<String>,
    #[serde(default)]
    pub splash_delay_seconds: Option<String>,
    #[serde(default)]
    pub splash_title: Option<String>,
    #[serde(default)]
    pub splash_message: Option<String>,
    /// Cloudflare Turnstile token, submitted by the rendered widget.
    #[serde(default, rename = "cf-turnstile-response")]
    pub turnstile_token: Option<String>,
    /// Google reCAPTCHA token.
    #[serde(default, rename = "g-recaptcha-response")]
    pub recaptcha_token: Option<String>,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct EditQuery {
    #[serde(default)]
    pub next: Option<String>,
}

/// Quick actions offered next to an expired or capped link (spec FR-EXP-3).
#[derive(Debug, Deserialize)]
pub struct ExtendForm {
    #[serde(default)]
    pub days: Option<String>,
    #[serde(default)]
    pub clicks: Option<String>,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct UpdateForm {
    /// Page to return to after saving; see [`return_path`].
    #[serde(default)]
    pub next: Option<String>,
    #[serde(default)]
    pub target_url: Option<String>,
    #[serde(default)]
    pub domain: Option<String>,
    #[serde(default)]
    pub slug: Option<String>,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub notes: Option<String>,
    #[serde(default)]
    pub tags: Option<String>,
    #[serde(default)]
    pub clear_tags: Option<String>,
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
    pub redirect_mode: Option<String>,
    #[serde(default)]
    pub splash_delay_seconds: Option<String>,
    #[serde(default)]
    pub splash_title: Option<String>,
    #[serde(default)]
    pub splash_message: Option<String>,
    #[serde(default)]
    pub status: Option<String>,
}

/// Split a comma separated tag field. Shared with the public form.
pub(crate) fn split_tags(raw: &Option<String>) -> Vec<String> {
    raw.as_deref()
        .unwrap_or_default()
        .split(',')
        .map(str::trim)
        .filter(|tag| !tag.is_empty())
        .map(str::to_string)
        .collect()
}

/// Parse an optional numeric form field. Shared with the public form.
///
/// An absent or blank field means "not submitted" and yields `Ok(None)`.
/// Anything else must be a whole number: a typo is reported instead of being
/// dropped, because silently ignoring `max_clicks=abc` would hand back a link
/// without the cap the caller asked for — and, on the public form, would slip
/// past the field switch that refuses a disabled input.
pub(crate) fn number_field(
    raw: &Option<String>,
    field: &'static str,
) -> Result<Option<i64>, AppError> {
    match raw.as_deref().map(str::trim) {
        None | Some("") => Ok(None),
        Some(value) => value
            .parse()
            .map(Some)
            .map_err(|_| AppError::validation(field, format!("{field} must be a whole number"))),
    }
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
    Submission(submission): Submission<CreateForm>,
) -> Result<Response, AppError> {
    let Some(current) = current else {
        return Ok(redirect_to_login(Some("/app")).into_response());
    };
    let (payload, preview) = match submission {
        Ok(LinkSubmission { form, preview }) => (form, preview),
        // A multipart body only fails to read when it is over the size limit,
        // which means the preview image.
        Err(Rejected {
            multipart: true, ..
        }) => return Ok(redirect_err("/app", "preview_image").into_response()),
        Err(_) => return Ok(redirect_err("/app", "invalid").into_response()),
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

    // Bot protection runs before the link is created.
    if captcha_rejected(
        &state,
        &client,
        CaptchaFlow::Create,
        payload.turnstile_token.as_deref(),
        payload.recaptcha_token.as_deref(),
    )
    .await
    {
        return Ok(redirect_err("/app", "captcha").into_response());
    }

    // The preview is checked before the link exists, so a rejected image does
    // not leave a half-configured link behind.
    let preview_changes = match preview_changes(&state, &current.user, preview.as_ref(), None).await
    {
        Ok(changes) => changes,
        Err(err) => return Ok(redirect_err("/app", preview_error_code(&err)).into_response()),
    };

    let context = UrlContext {
        pool: &state.db,
        settings: &state.settings,
        actor: &current.user,
        ip_hash: client.ip_hash.as_deref(),
    };

    // A malformed number is reported, never dropped: the link would otherwise
    // come back without the click cap or countdown the form asked for.
    let (max_clicks, redirect_type, splash_delay_seconds) = match (
        number_field(&payload.max_clicks, "max_clicks"),
        number_field(&payload.redirect_type, "redirect_type"),
        number_field(&payload.splash_delay_seconds, "splash_delay_seconds"),
    ) {
        (Ok(max_clicks), Ok(redirect_type), Ok(splash_delay_seconds)) => {
            (max_clicks, redirect_type, splash_delay_seconds)
        }
        (Err(err), _, _) | (_, Err(err), _) | (_, _, Err(err)) => {
            return Ok(redirect_err("/app", error_code(&err)).into_response())
        }
    };

    let outcome = context
        .create(CreateInput {
            target_url: payload.target_url,
            domain: payload.domain,
            slug: payload.slug,
            title: payload.title,
            notes: None,
            tags: split_tags(&payload.tags),
            password: payload.password,
            expires_at: payload.expires_at,
            max_clicks,
            redirect_type,
            redirect_mode: payload.redirect_mode,
            splash_delay_seconds,
            splash_title: payload.splash_title,
            splash_message: payload.splash_message,
        })
        .await;

    let url = match outcome {
        Ok(url) => url,
        Err(err) => return Ok(redirect_err("/app", error_code(&err)).into_response()),
    };
    let detail_page = format!("/app/urls/{}", url.id);
    if let Some(changes) = preview_changes {
        // The preview was validated before the link was created, so this only
        // fails on a policy change racing the request or a database error.
        // The link exists by now; say so instead of reporting a plain failure.
        if previews::save(&state, &current, url.id, changes)
            .await
            .is_err()
        {
            return Ok(redirect_err(&detail_page, "preview_not_saved").into_response());
        }
    }
    Ok(redirect_ok(&detail_page, "created").into_response())
}

/// `GET /app/urls/{id}` — the link's statistics. Editing happens in the
/// dialog (or on [`edit`] without JavaScript).
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
    let row = url_row(&state, &url).await?;
    let lang = i18n::current();
    let effective = crate::services::limits::effective(&state.db, &state.settings, &user).await?;
    let daily_total: i64 = daily.iter().map(|point| point.clicks).sum();
    let has_preview = crate::db::url_previews::find(&state.db, url.id)
        .await?
        .is_some();
    let page = UrlDetailPage {
        site_name: state.settings.site_name(),
        chrome: Chrome::new(
            &state,
            &user,
            "dashboard",
            flash_from_query(query.ok.as_deref(), query.err.as_deref()),
        ),
        url: row,
        summary: summary_dto(&summary),
        chart: chart::svg_bars(&daily),
        daily_total,
        referrers: bar_rows(dimension_dtos(referrers, Dimension::Referrer, lang)),
        devices: bar_rows(dimension_dtos(devices, Dimension::Device, lang)),
        browsers: bar_rows(dimension_dtos(browsers, Dimension::Browser, lang)),
        operating_systems: bar_rows(dimension_dtos(operating_systems, Dimension::Os, lang)),
        clicks: recent
            .iter()
            .map(|click| ClickRow::new(click, lang))
            .collect(),
        days,
        allow_expiry: effective.allow_expiry,
        allow_max_clicks: effective.allow_max_clicks,
        has_preview,
        days_label: i18n::format(
            lang,
            "url_detail.chart.days_ago",
            &[("days", days.to_string())],
        ),
        chart_legend: i18n::format(
            lang,
            "url_detail.chart.legend",
            &[("clicks", daily_total.to_string())],
        ),
        lang,
    }
    .themed();

    match page.render() {
        Ok(html) => Ok((StatusCode::OK, Html(html)).into_response()),
        Err(err) => Err(AppError::internal(format!("template error: {err}"))),
    }
}

/// `GET /app/urls/{id}/edit`
pub async fn edit(
    State(state): State<AppState>,
    MaybeUser(current): MaybeUser,
    Path(id): Path<i64>,
    Query(query): Query<EditQuery>,
) -> Result<Response, AppError> {
    let Some(current) = current else {
        return Ok(redirect_to_login(Some(&format!("/app/urls/{id}/edit"))).into_response());
    };
    let next = return_path(query.next.as_deref()).unwrap_or_default();
    render_editor(&state, &current.user, id, StatusCode::OK, |form| {
        form.next = next;
    })
    .await
}

/// Render the link editor after a rejected submission. `adjust` puts the
/// submitted values and the error back into the form.
pub(crate) async fn render_edit(
    state: &AppState,
    user: &User,
    id: i64,
    adjust: impl FnOnce(&mut LinkForm),
) -> Result<Response, AppError> {
    render_editor(state, user, id, StatusCode::UNPROCESSABLE_ENTITY, adjust).await
}

async fn render_editor(
    state: &AppState,
    user: &User,
    id: i64,
    status: StatusCode,
    adjust: impl FnOnce(&mut LinkForm),
) -> Result<Response, AppError> {
    let Some(url) = urls::find_owned(&state.db, id, user.id)
        .await
        .map_err(AppError::from)?
    else {
        return Ok(redirect_err("/app", "not_found").into_response());
    };
    let row = url_row(state, &url).await?;
    let lang = i18n::current();
    let current_domain = state.domain_hostname(url.domain_id);
    let policy = domain_policy::effective(&state.db, user).await?;
    let mut domains = policy.options(&state.domains);
    if !domains
        .iter()
        .any(|domain| domain.hostname == current_domain)
    {
        // Keep a link on a disabled domain editable without moving it.
        domains.insert(
            0,
            DomainOption {
                id: url.domain_id,
                hostname: current_domain.clone(),
                is_default: false,
            },
        );
    }
    let effective = crate::services::limits::effective(&state.db, &state.settings, user).await?;
    let preview = crate::db::url_previews::find(&state.db, url.id).await?;
    let mut link_form = LinkForm::edit(
        state,
        &row,
        current_domain,
        domains,
        &effective,
        policy.allow_selection,
        preview,
        lang,
    );
    adjust(&mut link_form);
    let page = UrlEditPage {
        site_name: state.settings.site_name(),
        chrome: Chrome::new(state, user, "dashboard", None),
        url: row,
        link_form,
        captcha: None,
        lang,
    }
    .themed();
    match page.render() {
        Ok(html) => Ok((status, Html(html)).into_response()),
        Err(err) => Err(AppError::internal(format!("template error: {err}"))),
    }
}

/// The page row of one stored link, with its tags.
async fn url_row(state: &AppState, url: &urls::Url) -> Result<UrlRow, AppError> {
    let tag_names: Vec<String> = tags::for_url(&state.db, url.id)
        .await
        .map_err(AppError::from)?
        .into_iter()
        .map(|tag| tag.name)
        .collect();
    let dto = UrlDto::new(
        url,
        &state.domain_hostname(url.domain_id),
        &state.config.base_scheme,
        tag_names,
    );
    Ok(UrlRow::new(&dto))
}

/// Where a saved edit returns to: a page of the signed-in app, never another
/// site. Without one the editor returns to the link's statistics.
fn return_path(next: Option<&str>) -> Option<String> {
    // A fragment would swallow the `?ok=` flash appended after it.
    safe_next(next)
        .filter(|path| !path.contains('#'))
        .filter(|path| path == "/app" || path.starts_with("/app/") || path.starts_with("/app?"))
}

/// Validate the submitted preview section and drop it when it changes
/// nothing, so saving a link does not rewrite an untouched preview.
async fn preview_changes(
    state: &AppState,
    user: &User,
    fields: Option<&PreviewFields>,
    existing: Option<&Preview>,
) -> Result<Option<Changes>, AppError> {
    let Some(fields) = fields else {
        return Ok(None);
    };
    if fields.image_mode == "upload" {
        previews::upload_limit(state, user.id)?;
    }
    let changes = previews::form_changes(fields, state.settings.preview_image_policy()).await?;
    Ok((!previews::is_noop(&changes, existing)).then_some(changes))
}

/// Flash code of a rejected preview on the create form, which redirects
/// instead of re-rendering.
fn preview_error_code(err: &AppError) -> &'static str {
    match err {
        AppError::RateLimited { .. } => "rate_limited",
        err if previews::is_form_error(err) => match previews::error_key(err) {
            "preview.error_policy" => "preview_policy",
            "preview.error_title" => "preview_title",
            "preview.error_description" => "preview_description",
            "preview.error_url" => "preview_url",
            _ => "preview_image",
        },
        _ => "invalid",
    }
}

/// Put a rejected submission — the link values and the preview section —
/// back into the editor.
fn restore_submission(form: &mut LinkForm, payload: &UpdateForm, preview: Option<&PreviewFields>) {
    retain_submission(form, payload);
    if let (Some(view), Some(fields)) = (form.preview.as_mut(), preview) {
        view.retain(fields);
    }
}

/// Put the submitted link values back into the editor after a rejection.
fn retain_submission(form: &mut LinkForm, payload: &UpdateForm) {
    fn set(target: &mut String, value: &Option<String>) {
        if let Some(value) = value {
            *target = value.clone();
        }
    }
    set(&mut form.target_url, &payload.target_url);
    set(&mut form.domain, &payload.domain);
    set(&mut form.slug, &payload.slug);
    set(&mut form.title, &payload.title);
    set(&mut form.tags, &payload.tags);
    set(&mut form.expires_at, &payload.expires_at);
    set(&mut form.max_clicks, &payload.max_clicks);
    set(
        &mut form.splash_delay_seconds,
        &payload.splash_delay_seconds,
    );
    set(&mut form.splash_title, &payload.splash_title);
    set(&mut form.splash_message, &payload.splash_message);
    set(&mut form.status, &payload.status);
    if let Some(value) = payload
        .redirect_mode
        .as_deref()
        .filter(|value| !value.trim().is_empty())
    {
        form.redirect_mode = value.to_string();
    }
    if let Some(value) = payload
        .redirect_type
        .as_deref()
        .and_then(|value| value.trim().parse().ok())
    {
        form.redirect_type = value;
    }
    form.clear_password = checkbox(&payload.clear_password);
    form.clear_expiry = checkbox(&payload.clear_expiry);
    form.clear_max_clicks = checkbox(&payload.clear_max_clicks);
    form.clear_tags = checkbox(&payload.clear_tags);
    form.next = return_path(payload.next.as_deref()).unwrap_or_default();
}

/// `POST /app/urls/{id}`
pub async fn update(
    State(state): State<AppState>,
    client: ClientInfo,
    MaybeUser(current): MaybeUser,
    Path(id): Path<i64>,
    Submission(submission): Submission<UpdateForm>,
) -> Result<Response, AppError> {
    let Some(current) = current else {
        return Ok(redirect_to_login(Some(&format!("/app/urls/{id}"))).into_response());
    };
    let target_page = format!("/app/urls/{id}");
    // A multipart body only fails to read when it is over the size limit,
    // which means the preview image. The fields read before the failure (if
    // any) are shown again; the rest come from the stored link.
    let (payload, preview, cut_short) = match submission {
        Ok(LinkSubmission { form, preview }) => (form, preview, false),
        Err(Rejected {
            multipart: true,
            partial,
            ..
        }) => match partial.map(|partial| *partial) {
            Some(LinkSubmission { form, preview }) => (form, preview, true),
            None => (UpdateForm::default(), None, true),
        },
        Err(_) => return Ok(redirect_err(&target_page, "invalid").into_response()),
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

    // A rejected edit renders the editor again with the submitted values, so
    // nothing has to be typed twice; the dialog shows it in place.
    let reject_link = |code: &'static str| {
        let message = flash_from_query(None, Some(code))
            .map(|flash| flash.message)
            .unwrap_or_default();
        let advanced = matches!(code, "expiry" | "password_rules" | "mode_denied");
        let (payload, preview) = (&payload, preview.as_ref());
        move |form: &mut LinkForm| {
            restore_submission(form, payload, preview);
            form.error = message;
            form.advanced_open = advanced;
        }
    };
    let reject_preview = |key: &'static str| {
        let (payload, preview) = (&payload, preview.as_ref());
        move |form: &mut LinkForm| {
            restore_submission(form, payload, preview);
            if let Some(view) = form.preview.as_mut() {
                view.reject(key);
            }
            form.preview_open = true;
        }
    };

    if cut_short {
        return render_edit(
            &state,
            &current.user,
            id,
            reject_preview("preview.error_image"),
        )
        .await;
    }

    let stored_preview = match preview {
        Some(_) => crate::db::url_previews::find(&state.db, id).await?,
        None => None,
    };
    let preview_changes = match preview_changes(
        &state,
        &current.user,
        preview.as_ref(),
        stored_preview.as_ref(),
    )
    .await
    {
        Ok(changes) => changes,
        Err(err @ AppError::RateLimited { .. }) => {
            return render_edit(&state, &current.user, id, reject_link(error_code(&err))).await
        }
        Err(err) if previews::is_form_error(&err) => {
            return render_edit(
                &state,
                &current.user,
                id,
                reject_preview(previews::error_key(&err)),
            )
            .await
        }
        Err(err) => return Err(err),
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
        match number_field(&payload.max_clicks, "max_clicks") {
            Ok(value) => value.map(Some),
            Err(err) => {
                return render_edit(&state, &current.user, id, reject_link(error_code(&err))).await
            }
        }
    };

    // The tag field is hidden when the group disables tags; the clear checkbox
    // is the only way for a browser user to remove existing tags.
    let tags = if checkbox(&payload.clear_tags) {
        Some(Vec::new())
    } else {
        payload.tags.as_ref().map(|_| split_tags(&payload.tags))
    };

    let (redirect_type, splash_delay_seconds) = match (
        number_field(&payload.redirect_type, "redirect_type"),
        number_field(&payload.splash_delay_seconds, "splash_delay_seconds"),
    ) {
        (Ok(redirect_type), Ok(splash_delay_seconds)) => (redirect_type, splash_delay_seconds),
        (Err(err), _) | (_, Err(err)) => {
            return render_edit(&state, &current.user, id, reject_link(error_code(&err))).await
        }
    };

    let outcome = context
        .update(
            &existing,
            UpdateInput {
                target_url: payload.target_url.clone(),
                domain: payload.domain.clone(),
                slug: payload.slug.clone(),
                title: Some(payload.title.clone()),
                notes: payload.notes.as_ref().map(|_| payload.notes.clone()),
                tags,
                password,
                expires_at,
                max_clicks,
                redirect_type,
                redirect_mode: payload
                    .redirect_mode
                    .clone()
                    .filter(|value| !value.trim().is_empty()),
                splash_delay_seconds,
                splash_title: payload
                    .splash_title
                    .as_ref()
                    .map(|_| payload.splash_title.clone()),
                splash_message: payload
                    .splash_message
                    .as_ref()
                    .map(|_| payload.splash_message.clone()),
                status: payload.status.clone(),
            },
        )
        .await;
    if let Err(err) = outcome {
        return render_edit(&state, &current.user, id, reject_link(error_code(&err))).await;
    }

    if let Some(changes) = preview_changes {
        // The preview was validated before the link changed, so this only
        // fails on a policy change racing the request or a database error.
        // The link is saved by now: the editor says so, shows the stored link
        // and keeps the preview input, so a retry only applies the preview.
        if let Err(err) = previews::save(&state, &current, id, changes).await {
            let key = previews::is_form_error(&err).then(|| previews::error_key(&err));
            let message = flash_from_query(None, Some("preview_not_saved"))
                .map(|flash| flash.message)
                .unwrap_or_default();
            return render_edit(&state, &current.user, id, |form| {
                if let (Some(view), Some(fields)) = (form.preview.as_mut(), preview.as_ref()) {
                    view.retain(fields);
                    if let Some(key) = key {
                        view.reject(key);
                    }
                }
                form.next = return_path(payload.next.as_deref()).unwrap_or_default();
                form.error = message;
                form.preview_open = true;
            })
            .await;
        }
    }

    let back = return_path(payload.next.as_deref()).unwrap_or(target_page);
    Ok(redirect_ok(&back, "updated").into_response())
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
    // Both fields are optional; a malformed one is reported rather than ignored,
    // so a typo cannot silently extend the wrong thing.
    let (days, clicks) = match (
        number_field(&payload.days, "days"),
        number_field(&payload.clicks, "clicks"),
    ) {
        (Ok(days), Ok(clicks)) => (days, clicks),
        (Err(err), _) | (_, Err(err)) => {
            return Ok(redirect_err(&target_page, error_code(&err)).into_response())
        }
    };
    if let Some(days) = days.filter(|days| (1..=3650).contains(days)) {
        // Extend from the later of "now" and the current expiry.
        let base = url
            .expires_at
            .as_deref()
            .and_then(time::parse_iso)
            .filter(|instant| *instant > time::now())
            .unwrap_or_else(time::now);
        update.expires_at = Some(Some(time::to_iso(time::plus_days(base, days))));
    }
    if let Some(clicks) = clicks.filter(|clicks| (1..=1_000_000).contains(clicks)) {
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

/// Convert one breakdown into template rows, with localised labels.
///
/// The SQL rollup fills missing values with English literals (`direct`,
/// `Unknown`, `Other`) and the user agent parser stores device classes in
/// English (`Desktop`, `Mobile`, `Tablet`, `Bot`). They are translated here, on
/// the page path only: `/api/v1` keeps the raw labels so its payload stays
/// stable and machine readable. The English labels are returned unchanged, so
/// the English UI does not shift.
fn dimension_dtos(
    rows: Vec<clicks::DimensionCount>,
    dimension: Dimension,
    lang: Lang,
) -> Vec<crate::routes::dto::DimensionDto> {
    rows.into_iter()
        .map(|row| crate::routes::dto::DimensionDto {
            label: dimension_label(dimension, &row.label, lang),
            count: row.count,
        })
        .collect()
}

/// Localise one breakdown label.
///
/// Proper nouns (`Chrome`, `Windows`, `news.example.com`) pass through.
fn dimension_label(dimension: Dimension, label: &str, lang: Lang) -> String {
    match dimension {
        Dimension::Referrer => match label {
            "direct" => lang.t("common.direct").to_string(),
            _ => label.to_string(),
        },
        // Covers `Desktop`/`Mobile`/`Tablet`/`Bot` and the `Unknown` fallback.
        Dimension::Device => i18n::device_label(lang, label),
        Dimension::Browser => match label {
            "Other" => lang.t("common.other").to_string(),
            _ => label.to_string(),
        },
        Dimension::Os => match label {
            "Unknown" => lang.t("common.unknown").to_string(),
            _ => label.to_string(),
        },
    }
}

/// Map a service error onto a flash code the page understands.
pub fn error_code(err: &AppError) -> &'static str {
    match err {
        AppError::Conflict(_) => "conflict",
        AppError::NotFound => "not_found",
        AppError::Forbidden(message)
            if message.starts_with("you have reached your limit of ")
                && message.contains("API tokens") =>
        {
            "token_quota"
        }
        AppError::Forbidden(message) if message.starts_with("invite quota reached") => {
            "invite_quota"
        }
        AppError::Forbidden(message) if message.starts_with("your group does not allow invite") => {
            "invites_disabled"
        }
        AppError::Forbidden(message) if message.starts_with("your group only allows") => {
            "mode_denied"
        }
        AppError::Forbidden(message) if message.starts_with("you have reached your limit of ") => {
            "quota"
        }
        AppError::Forbidden(message) if message == crate::services::guest_allowlist::BLOCKED => {
            "guest_allowlist"
        }
        AppError::Forbidden(_) => "denied",
        AppError::RateLimited { .. } => "rate_limited",
        AppError::Validation { field, .. } => match *field {
            "expires_at" => "expiry",
            "slug" => "slug",
            "domain" => "domain",
            "target_url" => "target",
            "guest_allowlist" => "guest_allowlist_rules",
            "password" => "password_rules",
            "tags" => "tags",
            _ => "invalid",
        },
        AppError::BadRequest(_) => "invalid",
        AppError::CaptchaFailed(_) => "captcha",
        AppError::Unauthorized(_) => "password",
        AppError::Gone(_) | AppError::Internal(_) => "invalid",
    }
}
