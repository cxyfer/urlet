//! Server rendered HTML pages.
//!
//! Every page is rendered by Askama at request time; there is no frontend build
//! step and no client side framework. Forms post to `POST` routes that redirect
//! back with a short status code in the query string (`?ok=created`), which the
//! page turns into a banner.

pub mod account;
pub mod admin;
pub mod auth;
pub mod dashboard;
pub mod home;
pub mod invites;
pub mod themed;
pub mod urls;

use axum::extract::{DefaultBodyLimit, Path, Query, State};
use axum::http::{header, HeaderMap, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Redirect, Response};
use axum::routing::{get, post};
use axum::Router;
use serde::Deserialize;

use crate::db::users::User;
use crate::error::AppError;
use crate::i18n::{self, Lang};
use crate::middleware::auth::{ClientInfo, MaybeUser};
use crate::services::captcha::{CaptchaFlow, CaptchaGuard, CaptchaWidget};
use crate::services::custom_theme;
use crate::services::invite_quota::InvitePeriod;
use crate::services::theme::{self, ThemeView};
use crate::state::AppState;

/// A banner shown at the top of a page.
#[derive(Debug, Clone)]
pub struct Flash {
    pub kind: &'static str,
    pub message: String,
}

/// Shared chrome (navigation, identity, flash) for authenticated pages.
#[derive(Debug, Clone)]
pub struct Chrome {
    pub site_name: String,
    pub email: String,
    pub role: String,
    pub is_admin: bool,
    pub active: &'static str,
    pub flash: Option<Flash>,
    pub theme: ThemeView,
}

impl Chrome {
    pub fn new(state: &AppState, user: &User, active: &'static str, flash: Option<Flash>) -> Self {
        Self {
            site_name: state.settings.site_name(),
            email: user.label().to_string(),
            role: user.role.clone(),
            is_admin: user.is_admin(),
            active,
            flash,
            theme: crate::services::theme::for_user(&state.settings, user),
        }
    }
}

/// Localised calendar period name, shared by the admin and member pages.
pub fn period_label(lang: Lang, period: InvitePeriod) -> String {
    match period {
        InvitePeriod::Day => lang.t("admin.groups.period.day").to_string(),
        InvitePeriod::Week => lang.t("admin.groups.period.week").to_string(),
        InvitePeriod::Month => lang.t("admin.groups.period.month").to_string(),
    }
}

/// One entry of the pagination control.
#[derive(Debug, Clone)]
pub struct PageLink {
    pub url: String,
    pub label: String,
    pub current: bool,
}

/// Translate `?ok=` / `?err=` codes into a banner. Unknown codes are ignored so
/// the query string can never inject arbitrary text into a page.
pub fn flash_from_query(ok: Option<&str>, err: Option<&str>) -> Option<Flash> {
    let lang = i18n::current();
    if let Some(code) = ok {
        let key = match code {
            "created" => "flash.ok.created",
            "updated" => "flash.ok.updated",
            "deleted" => "flash.ok.deleted",
            "profile" => "flash.ok.profile",
            "password" => "flash.ok.password",
            "session" => "flash.ok.session",
            "token_created" => "flash.ok.token_created",
            "token_revoked" => "flash.ok.token_revoked",
            "saved" => "flash.ok.saved",
            "invite" => "flash.ok.invite",
            "invite_revoked" => "flash.ok.invite_revoked",
            "user" => "flash.ok.user",
            "user_deleted" => "flash.ok.user_deleted",
            "domain_created" => "flash.ok.domain_created",
            "domain_activated" => "flash.ok.domain_activated",
            "domain_disabled" => "flash.ok.domain_disabled",
            "domain_default" => "flash.ok.domain_default",
            "domain_deleted" => "flash.ok.domain_deleted",
            "group" => "flash.ok.group",
            "group_deleted" => "flash.ok.group_deleted",
            "theme_created" => "flash.ok.theme_created",
            "theme_updated" => "flash.ok.theme_updated",
            "theme_deleted" => "flash.ok.theme_deleted",
            _ => return None,
        };
        return Some(Flash {
            kind: "ok",
            message: lang.t(key).to_string(),
        });
    }

    if let Some(code) = err {
        let key = match code {
            "invalid" => "flash.err.invalid",
            "conflict" => "flash.err.conflict",
            "not_found" => "flash.err.not_found",
            "rate_limited" => "flash.err.rate_limited",
            "denied" => "flash.err.denied",
            "mode_denied" => "flash.err.mode_denied",
            "quota" => "flash.err.quota",
            "invite_quota" => "flash.err.invite_quota",
            "invites_disabled" => "flash.err.invites_disabled",
            "token_quota" => "flash.err.token_quota",
            "password" => "flash.err.password",
            "login" => "flash.err.login",
            "registration" => "flash.err.registration",
            "invite" => "flash.err.invite",
            "csrf" => "flash.err.csrf",
            "expiry" => "flash.err.expiry",
            "slug" => "flash.err.slug",
            "target" => "flash.err.target",
            "password_rules" => "flash.err.password_rules",
            "tags" => "flash.err.tags",
            "captcha" => "flash.err.captcha",
            "theme" => "flash.err.theme",
            "theme_upload" => "flash.err.theme_upload",
            "theme_failed" => "flash.err.theme_failed",
            "theme_name" => "flash.err.theme_name",
            "theme_css" => "flash.err.theme_css",
            "theme_quota" => "flash.err.theme_quota",
            "theme_default" => "flash.err.theme_default",
            "domain" => "flash.err.domain",
            "domain_invalid" => "flash.err.domain_invalid",
            "domain_conflict" => "flash.err.domain_conflict",
            "domain_in_use" => "flash.err.domain_in_use",
            "domain_system" => "flash.err.domain_system",
            "domain_env" => "flash.err.domain_env",
            "domain_default" => "flash.err.domain_default",
            _ => return None,
        };
        return Some(Flash {
            kind: "error",
            message: lang.t(key).to_string(),
        });
    }

    None
}

/// Redirect back to a page with a success code.
pub fn redirect_ok(path: &str, code: &str) -> Redirect {
    Redirect::to(&with_param(path, "ok", code))
}

/// Redirect back to a page with an error code.
pub fn redirect_err(path: &str, code: &str) -> Redirect {
    Redirect::to(&with_param(path, "err", code))
}

/// Widget a browser form must render for `flow`, or `None` when the flow is not
/// protected. The guard is inert unless a provider, a site key, a secret key and
/// the per-flow switch are all set.
pub fn captcha_widget(state: &AppState, flow: CaptchaFlow) -> Option<CaptchaWidget> {
    captcha_guard(state, flow).widget(i18n::current())
}

/// Guard for a browser flow, built from the effective settings.
pub fn captcha_guard(state: &AppState, flow: CaptchaFlow) -> CaptchaGuard {
    CaptchaGuard::from_settings(
        &state.settings,
        flow,
        state.config.captcha_verify_url.as_deref(),
        &state.config.base_host,
    )
}

/// Verify the bot protection token of a browser flow.
///
/// Returns `true` when the submission must be rejected with `?err=captcha`; an
/// unguarded flow always returns `false`, so it behaves as before. The field of
/// the configured provider wins; a stale value in the other field is only a
/// fallback.
pub async fn captcha_rejected(
    state: &AppState,
    client: &ClientInfo,
    flow: CaptchaFlow,
    turnstile_token: Option<&str>,
    recaptcha_token: Option<&str>,
) -> bool {
    let guard = captcha_guard(state, flow);
    if !guard.enabled() {
        return false;
    }
    let token = guard.submitted_token(turnstile_token, recaptcha_token);
    match guard.verify(token, client.ip).await {
        Ok(()) => false,
        Err(err) => {
            // Only the reason code is logged; the token never is.
            tracing::debug!(reason = err.code(), "web captcha check rejected a request");
            true
        }
    }
}

/// Redirect to the login screen, remembering where the visitor wanted to go.
pub fn redirect_to_login(next: Option<&str>) -> Redirect {
    match safe_next(next) {
        Some(next) => Redirect::to(&format!("/app/login?next={}", encode(&next))),
        None => Redirect::to("/app/login"),
    }
}

/// Only same-site relative paths are accepted as a post-login destination.
pub fn safe_next(next: Option<&str>) -> Option<String> {
    let next = next?.trim();
    // `//host` is a protocol-relative URL, but browsers treat a backslash as a
    // path separator for http(s) too, so `/\host` resolves to `//host` as well
    // (WHATWG URL). Both have to be rejected, and a backslash has no legitimate
    // place in any of our paths. Control characters are rejected because the
    // value is written straight into a `Location` header.
    if !next.starts_with('/')
        || next.starts_with("//")
        || next.contains('\\')
        || next.contains("://")
        || next.chars().any(char::is_control)
        || next.len() >= 512
    {
        return None;
    }
    Some(next.to_string())
}

/// Append a query parameter to a path, preserving existing ones.
pub fn with_param(path: &str, key: &str, value: &str) -> String {
    let separator = if path.contains('?') { '&' } else { '?' };
    format!("{path}{separator}{key}={}", encode(value))
}

/// Build a query string from key/value pairs (values are percent encoded).
pub fn query_string(params: &[(&str, String)]) -> String {
    params
        .iter()
        .filter(|(_, value)| !value.is_empty())
        .map(|(key, value)| format!("{}={}", encode(key), encode(value)))
        .collect::<Vec<_>>()
        .join("&")
}

/// Percent encode everything that is unsafe inside a query value.
pub fn encode(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for byte in value.as_bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(*byte as char)
            }
            _ => out.push_str(&format!("%{byte:02X}")),
        }
    }
    out
}

/// Percent encode a `Location` target.
///
/// `Location` is defined as a URI reference, so anything outside printable
/// ASCII is encoded: browsers cope with raw UTF-8, intermediaries need not.
/// The ASCII range — including the path and query separators — is left alone so
/// ordinary redirects stay readable (`/app/account`, not `%2Fapp%2Faccount`).
pub fn encode_location(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for byte in value.as_bytes() {
        if (0x21..=0x7e).contains(byte) {
            out.push(*byte as char);
        } else {
            out.push_str(&format!("%{byte:02X}"));
        }
    }
    out
}

/// Build the pagination control for a listing.
pub fn page_links(
    base: &str,
    params: &[(&str, String)],
    page: i64,
    total_pages: i64,
) -> Vec<PageLink> {
    let mut links = Vec::new();
    let first = (page - 2).max(1);
    let last = (page + 2).min(total_pages.max(1));

    let build = |target: i64| {
        let mut all = params.to_vec();
        all.push(("page", target.to_string()));
        let query = query_string(&all);
        if query.is_empty() {
            base.to_string()
        } else {
            format!("{base}?{query}")
        }
    };

    if page > 1 {
        links.push(PageLink {
            url: build(page - 1),
            label: "‹".to_string(),
            current: false,
        });
    }
    for target in first..=last {
        links.push(PageLink {
            url: build(target),
            label: target.to_string(),
            current: target == page,
        });
    }
    if page < total_pages {
        links.push(PageLink {
            url: build(page + 1),
            label: "›".to_string(),
            current: false,
        });
    }
    links
}

/// A short link prepared for rendering in a table or detail page.
#[derive(Debug, Clone)]
pub struct UrlRow {
    pub id: i64,
    pub slug: String,
    pub short_url: String,
    pub target_url: String,
    pub target_display: String,
    pub title: Option<String>,
    pub tags: Vec<String>,
    pub has_password: bool,
    pub click_count: i64,
    pub unique_click_count: i64,
    pub max_clicks: Option<i64>,
    pub expires_at: Option<String>,
    pub expires_human: Option<String>,
    pub created_human: String,
    pub redirect_type: i64,
    /// `direct` or `splash`.
    pub redirect_mode: String,
    /// True when the link renders the interstitial before redirecting.
    pub is_splash: bool,
    pub splash_delay_seconds: i64,
    pub splash_title: String,
    pub splash_message: String,
    pub state: String,
    pub state_class: &'static str,
    pub state_label: &'static str,
}

impl UrlRow {
    pub fn new(dto: &crate::routes::dto::UrlDto) -> Self {
        let lang = i18n::current();
        let (state_class, state_key) = match dto.state.as_str() {
            "active" => ("badge-ok", "state.active"),
            "expired" => ("badge-warn", "state.expired"),
            "limit_reached" => ("badge-warn", "state.limit_reached"),
            "disabled" => ("badge-muted", "state.disabled"),
            _ => ("badge-muted", "state.deleted"),
        };
        Self {
            id: dto.id,
            slug: dto.slug.clone(),
            short_url: dto.short_url.clone(),
            target_url: dto.target_url.clone(),
            target_display: truncate(&dto.target_url, 72),
            title: dto.title.clone(),
            tags: dto.tags.clone(),
            has_password: dto.has_password,
            click_count: dto.click_count,
            unique_click_count: dto.unique_click_count,
            max_clicks: dto.max_clicks,
            expires_at: dto.expires_at.clone(),
            // The DTO keeps the English `expires_in` for `/api/v1`; the page
            // renders its own, localised, relative date.
            expires_human: dto
                .expires_at
                .as_deref()
                .map(|value| crate::services::time::humanize_lang(lang, value)),
            created_human: crate::services::time::humanize_lang(lang, &dto.created_at),
            redirect_type: dto.redirect_type,
            redirect_mode: dto.redirect_mode.clone(),
            is_splash: dto.redirect_mode == "splash",
            splash_delay_seconds: dto.splash_delay_seconds,
            splash_title: dto.splash_title.clone().unwrap_or_default(),
            splash_message: dto.splash_message.clone().unwrap_or_default(),
            state: dto.state.clone(),
            state_class,
            state_label: lang.t(state_key),
        }
    }
}

/// Truncate a string for display, appending an ellipsis when shortened.
pub fn truncate(value: &str, max: usize) -> String {
    if value.chars().count() <= max {
        return value.to_string();
    }
    let mut out: String = value.chars().take(max.saturating_sub(1)).collect();
    out.push('…');
    out
}

/// `/app` router.
pub fn router() -> Router<AppState> {
    Router::new()
        .route("/", get(dashboard::dashboard))
        .route("/login", get(auth::login_page).post(auth::login_submit))
        .route(
            "/register",
            get(auth::register_page).post(auth::register_submit),
        )
        .route("/logout", post(auth::logout))
        .route("/urls", post(urls::create))
        .route("/urls/{id}", get(urls::detail).post(urls::update))
        .route("/urls/{id}/delete", post(urls::delete))
        .route("/urls/{id}/extend", post(urls::extend))
        .route("/account", get(account::page).post(account::update_profile))
        .route("/account/password", post(account::change_password))
        .route(
            "/account/sessions/{id}/delete",
            post(account::revoke_session),
        )
        .route("/account/tokens", post(account::create_token))
        .route("/account/tokens/{id}/delete", post(account::revoke_token))
        // member invite codes
        .route("/invites", get(invites::page).post(invites::create))
        .route("/invites/{id}/delete", post(invites::revoke))
        // administration
        .route("/admin", get(admin::overview))
        .route("/admin/users", get(admin::users_page))
        .route("/admin/users/{id}", post(admin::update_user))
        .route("/admin/users/{id}/password", post(admin::reset_password))
        .route("/admin/users/{id}/delete", post(admin::delete_user))
        .route("/admin/urls", get(admin::urls_page))
        .route("/admin/urls/{id}", post(admin::update_url))
        .route("/admin/urls/{id}/delete", post(admin::delete_url))
        .route(
            "/admin/invites",
            get(admin::invites_page).post(admin::create_invite),
        )
        .route("/admin/invites/{id}/delete", post(admin::revoke_invite))
        .route(
            "/admin/domains",
            get(admin::domains_page).post(admin::create_domain),
        )
        .route("/admin/domains/{id}/activate", post(admin::activate_domain))
        .route("/admin/domains/{id}/disable", post(admin::disable_domain))
        .route("/admin/domains/{id}/default", post(admin::default_domain))
        .route("/admin/domains/{id}/delete", post(admin::delete_domain))
        .route(
            "/admin/groups",
            get(admin::groups_page).post(admin::create_group),
        )
        .route("/admin/groups/new", get(admin::new_group_page))
        .route("/admin/groups/{id}/edit", get(admin::edit_group_page))
        .route("/admin/groups/{id}", post(admin::update_group))
        .route("/admin/groups/{id}/delete", post(admin::delete_group))
        .route(
            "/admin/themes",
            get(admin::themes_page).post(admin::update_themes),
        )
        .route(
            "/admin/themes/custom",
            post(admin::create_custom_theme)
                .layer(DefaultBodyLimit::max(custom_theme::MAX_UPLOAD_BYTES)),
        )
        .route(
            "/admin/themes/custom/{id}",
            get(admin::custom_theme_page)
                .post(admin::update_custom_theme)
                .layer(DefaultBodyLimit::max(custom_theme::MAX_UPLOAD_BYTES)),
        )
        .route(
            "/admin/themes/custom/{id}/delete",
            post(admin::delete_custom_theme),
        )
        .route(
            "/admin/settings",
            get(admin::settings_page).post(admin::update_settings),
        )
        .route("/admin/audit", get(admin::audit_page))
        // language switcher
        .route("/lang/{code}", get(lang_switch))
        // theme picker
        .route("/theme/default", get(theme_default))
        .route("/theme/preview/stop", get(theme_preview_stop))
        .route("/theme/{key}", get(theme_switch))
}

/// Query string of `GET /app/theme/{key}`.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct ThemeQuery {
    #[serde(default)]
    pub next: Option<String>,
    /// `1`/`true` requests the administrator-only preview cookie.
    #[serde(default)]
    pub preview: Option<String>,
}

fn wants_preview(value: Option<&str>) -> bool {
    matches!(value.map(str::trim), Some("1" | "true" | "yes" | "on"))
}

/// `GET /app/theme/{key}` — remember the choice in a cookie and go back.
///
/// Only a signed-in account may switch, and only when the site policy allows
/// it. The choice lives in a cookie so it applies before authentication too;
/// the server validates the key against the enabled themes on every request,
/// so a stale or hand-written value cannot load a disabled theme.
///
/// An administrator can pass `?preview=1` to set the separate preview cookie,
/// which may name a disabled theme. The theme middleware only honours that
/// cookie for administrators.
pub async fn theme_switch(
    State(state): State<AppState>,
    MaybeUser(current): MaybeUser,
    Path(key): Path<String>,
    Query(query): Query<ThemeQuery>,
    headers: HeaderMap,
) -> Response {
    let target = theme_target(&headers, &state.config.base_url, query.next.as_deref());
    let Some(current) = current else {
        return redirect_to_login(Some(&target)).into_response();
    };
    if !theme::can_switch(&state.settings, &current.user) {
        return AppError::forbidden("theme switching is disabled for this account").into_response();
    }
    let lang = crate::i18n::current();
    let Some(theme_key) = state.themes.known(&key, lang) else {
        return AppError::not_found().into_response();
    };

    let mut cookies = Vec::new();
    if wants_preview(query.preview.as_deref()) {
        if !current.is_admin() {
            return AppError::forbidden("theme preview is administrator only").into_response();
        }
        cookies.push(crate::services::cookies::build(
            crate::services::theme::PREVIEW_COOKIE_NAME,
            &theme_key.key,
            crate::services::theme::PREVIEW_COOKIE_MAX_AGE,
            state.config.secure_cookies,
        ));
    } else {
        if !state.themes.is_enabled(&theme_key.key) {
            return AppError::not_found().into_response();
        }
        cookies.push(crate::services::cookies::build(
            crate::services::theme::COOKIE_NAME,
            &theme_key.key,
            crate::services::theme::COOKIE_MAX_AGE,
            state.config.secure_cookies,
        ));
        // A normal picker click leaves preview mode.
        cookies.push(crate::services::cookies::clear(
            crate::services::theme::PREVIEW_COOKIE_NAME,
            state.config.secure_cookies,
        ));
    }
    theme_redirect(&target, &cookies)
}

/// `GET /app/theme/default` — clear the explicit choice and follow the site
/// default again.
pub async fn theme_default(
    State(state): State<AppState>,
    MaybeUser(current): MaybeUser,
    Query(query): Query<ThemeQuery>,
    headers: HeaderMap,
) -> Response {
    let target = theme_target(&headers, &state.config.base_url, query.next.as_deref());
    let Some(current) = current else {
        return redirect_to_login(Some(&target)).into_response();
    };
    if !theme::can_switch(&state.settings, &current.user) {
        return AppError::forbidden("theme switching is disabled for this account").into_response();
    }
    let cookies = vec![
        crate::services::cookies::clear(
            crate::services::theme::COOKIE_NAME,
            state.config.secure_cookies,
        ),
        crate::services::cookies::clear(
            crate::services::theme::PREVIEW_COOKIE_NAME,
            state.config.secure_cookies,
        ),
    ];
    theme_redirect(&target, &cookies)
}

/// `GET /app/theme/preview/stop` — leave administrator preview mode.
pub async fn theme_preview_stop(
    State(state): State<AppState>,
    MaybeUser(current): MaybeUser,
    Query(query): Query<ThemeQuery>,
    headers: HeaderMap,
) -> Response {
    let target = theme_target(&headers, &state.config.base_url, query.next.as_deref());
    if current.is_none() {
        return redirect_to_login(Some(&target)).into_response();
    }
    let cookies = vec![crate::services::cookies::clear(
        crate::services::theme::PREVIEW_COOKIE_NAME,
        state.config.secure_cookies,
    )];
    theme_redirect(&target, &cookies)
}

fn theme_target(headers: &HeaderMap, base_url: &str, next: Option<&str>) -> String {
    safe_next(next)
        .or_else(|| referrer_path(headers, base_url))
        .unwrap_or_else(|| "/app".to_string())
}

fn theme_redirect(target: &str, cookies: &[String]) -> Response {
    let mut response = StatusCode::FOUND.into_response();
    let location = HeaderValue::from_str(&encode_location(target))
        .unwrap_or_else(|_| HeaderValue::from_static("/app"));
    response.headers_mut().insert(header::LOCATION, location);
    response
        .headers_mut()
        .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    for cookie in cookies {
        if let Ok(value) = HeaderValue::from_str(cookie) {
            response.headers_mut().append(header::SET_COOKIE, value);
        }
    }
    response
}

/// Same-site path of a `Referer`, so a picker click can stay on the page the
/// visitor was reading without putting the path in every link.
fn referrer_path(headers: &HeaderMap, base_url: &str) -> Option<String> {
    let raw = headers.get(header::REFERER)?.to_str().ok()?;
    let base = base_url.trim_end_matches('/');
    let path = raw.strip_prefix(base).unwrap_or(raw);
    safe_next(Some(path))
}

/// Query string of `GET /app/lang/{code}`.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct LangQuery {
    #[serde(default)]
    pub next: Option<String>,
}

/// `GET /app/lang/{code}` — remember the choice in a cookie and go back.
///
/// An unrecognised code is treated as English rather than failing the request.
pub async fn lang_switch(
    State(state): State<AppState>,
    Path(code): Path<String>,
    Query(query): Query<LangQuery>,
) -> Response {
    let lang = Lang::parse(&code).unwrap_or(Lang::En);
    let target = safe_next(query.next.as_deref()).unwrap_or_else(|| "/app".to_string());

    let mut response = StatusCode::FOUND.into_response();
    // The fallback keeps the response a usable redirect even if encoding ever
    // produced a value the header rejects.
    let location = HeaderValue::from_str(&encode_location(&target))
        .unwrap_or_else(|_| HeaderValue::from_static("/app"));
    response.headers_mut().insert(header::LOCATION, location);

    // Deliberately not `HttpOnly`: the value is read by CSS/JS if a theme ever
    // needs it, and it carries no secret.
    let mut cookie = format!(
        "{}={}; Path=/; Max-Age={}; SameSite=Lax",
        i18n::COOKIE_NAME,
        lang.code(),
        i18n::COOKIE_MAX_AGE
    );
    if state.config.secure_cookies {
        cookie.push_str("; Secure");
    }
    if let Ok(value) = HeaderValue::from_str(&cookie) {
        response.headers_mut().append(header::SET_COOKIE, value);
    }
    response
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn theme_switch_referrer_stays_on_site() {
        let mut headers = HeaderMap::new();
        headers.insert(
            header::REFERER,
            HeaderValue::from_static("http://localhost:8080/app/urls/7?days=30"),
        );
        assert_eq!(
            referrer_path(&headers, "http://localhost:8080").as_deref(),
            Some("/app/urls/7?days=30")
        );

        headers.insert(
            header::REFERER,
            HeaderValue::from_static("https://evil.example/app"),
        );
        assert_eq!(referrer_path(&headers, "http://localhost:8080"), None);

        headers.insert(
            header::REFERER,
            HeaderValue::from_static("http://localhost:8080.evil.example/app"),
        );
        assert_eq!(referrer_path(&headers, "http://localhost:8080"), None);

        headers.remove(header::REFERER);
        assert_eq!(referrer_path(&headers, "http://localhost:8080"), None);
    }

    #[test]
    fn only_relative_next_targets_are_allowed() {
        assert_eq!(
            safe_next(Some("/app/urls/1")).as_deref(),
            Some("/app/urls/1")
        );
        assert_eq!(safe_next(Some("//evil.example")), None);
        assert_eq!(safe_next(Some("https://evil.example")), None);
        assert_eq!(safe_next(Some("javascript:alert(1)")), None);
        assert_eq!(safe_next(None), None);
        assert_eq!(safe_next(Some(&"a".repeat(600))), None);
        // Browsers normalise a backslash to a slash for http(s), so `/\host`
        // and `\/host` are protocol-relative URLs in disguise.
        assert_eq!(safe_next(Some("/\\evil.example")), None);
        assert_eq!(safe_next(Some("\\/evil.example")), None);
        assert_eq!(safe_next(Some("/app\\..\\evil")), None);
        // Control characters would corrupt the `Location` header.
        assert_eq!(safe_next(Some("/app\r\nSet-Cookie: x=1")), None);
        assert_eq!(safe_next(Some("/app\u{0}")), None);
        // Ordinary deep links still work.
        assert_eq!(
            safe_next(Some("/app/admin/users?page=2")).as_deref(),
            Some("/app/admin/users?page=2")
        );
    }

    #[test]
    fn encodes_query_values() {
        assert_eq!(encode("promo & more"), "promo%20%26%20more");
        assert_eq!(encode("safe-._~"), "safe-._~");
        assert_eq!(
            query_string(&[("q", "a b".into()), ("empty", String::new())]),
            "q=a%20b"
        );
        assert_eq!(with_param("/app", "ok", "created"), "/app?ok=created");
        assert_eq!(
            with_param("/app?q=x", "ok", "created"),
            "/app?q=x&ok=created"
        );
    }

    #[test]
    fn location_targets_are_ascii_only() {
        // Ordinary paths and queries stay readable.
        assert_eq!(encode_location("/app/account"), "/app/account");
        assert_eq!(
            encode_location("/app/admin/users?page=2&q=a%20b"),
            "/app/admin/users?page=2&q=a%20b"
        );
        // Non-ASCII and spaces are encoded: `Location` is a URI reference.
        assert_eq!(encode_location("/日本"), "/%E6%97%A5%E6%9C%AC");
        assert_eq!(
            encode_location("/app/日本語"),
            "/app/%E6%97%A5%E6%9C%AC%E8%AA%9E"
        );
        assert_eq!(encode_location("/a b"), "/a%20b");
        assert!(encode_location("/日本").is_ascii());
    }

    #[test]
    fn builds_pagination_links() {
        let links = page_links("/app", &[("q", "term".to_string())], 2, 4);
        assert_eq!(links.first().expect("prev").label, "‹");
        assert_eq!(links.last().expect("next").label, "›");
        assert_eq!(links.first().expect("prev").url, "/app?q=term&page=1");
        assert!(links.iter().any(|link| link.current && link.label == "2"));
        // A single page listing shows no navigation.
        let single = page_links("/app", &[], 1, 1);
        assert_eq!(single.len(), 1);
        assert!(single[0].current);
    }

    #[test]
    fn maps_known_flash_codes_only() {
        assert_eq!(
            flash_from_query(Some("created"), None).expect("flash").kind,
            "ok"
        );
        assert_eq!(
            flash_from_query(None, Some("conflict"))
                .expect("flash")
                .kind,
            "error"
        );
        assert!(flash_from_query(Some("<script>"), None).is_none());
        assert_eq!(
            flash_from_query(None, Some("expiry")).expect("flash").kind,
            "error"
        );
        assert!(flash_from_query(None, Some("expiry"))
            .expect("flash")
            .message
            .contains("future"));
        let captcha = flash_from_query(None, Some("captcha")).expect("flash");
        assert_eq!(captcha.kind, "error");
        assert_eq!(captcha.message, Lang::En.t("flash.err.captcha"));
        assert!(flash_from_query(None, None).is_none());
    }
}
