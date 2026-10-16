//! Login, registration and logout pages.

use askama::Template;
use axum::extract::rejection::FormRejection;
use axum::extract::{Query, State};
use axum::http::{header, HeaderValue, StatusCode};
use axum::response::{Html, IntoResponse, Redirect, Response};
use axum::Form;
use serde::Deserialize;

use crate::error::{AppError, Result};
use crate::i18n::{self, Lang};
use crate::middleware::auth::{ClientInfo, MaybeUser};
use crate::routes::pages::{
    captcha_rejected, captcha_widget, redirect_err, redirect_ok, safe_next,
};
use crate::services::auth::{self, AuthContext};
use crate::services::captcha::{CaptchaFlow, CaptchaWidget};
use crate::services::ratelimit::limits;
use crate::services::settings::RegistrationMode;
use crate::state::AppState;

#[derive(Debug, Template)]
#[template(path = "login.html")]
pub struct LoginPage {
    pub site_name: String,
    pub error: Option<String>,
    pub notice: Option<String>,
    pub next: String,
    pub email: String,
    /// Provider widget to render, or `None` when sign in is not protected.
    pub captcha: Option<CaptchaWidget>,
    pub lang: Lang,
}

#[derive(Debug, Template)]
#[template(path = "register.html")]
pub struct RegisterPage {
    pub site_name: String,
    pub error: Option<String>,
    pub mode: &'static str,
    pub invite_required: bool,
    pub registration_open: bool,
    pub next: String,
    pub email: String,
    pub display_name: String,
    pub invite_code: String,
    /// Provider widget to render, or `None` when registration is not protected.
    pub captcha: Option<CaptchaWidget>,
    pub lang: Lang,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct LoginQuery {
    #[serde(default)]
    pub next: Option<String>,
    #[serde(default)]
    pub ok: Option<String>,
    #[serde(default)]
    pub err: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct LoginForm {
    pub email: String,
    pub password: String,
    #[serde(default)]
    pub next: Option<String>,
    /// Cloudflare Turnstile token, submitted by the rendered widget.
    #[serde(default, rename = "cf-turnstile-response")]
    pub turnstile_token: Option<String>,
    /// Google reCAPTCHA token.
    #[serde(default, rename = "g-recaptcha-response")]
    pub recaptcha_token: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct RegisterForm {
    pub email: String,
    pub password: String,
    #[serde(default)]
    pub display_name: Option<String>,
    #[serde(default)]
    pub invite_code: Option<String>,
    #[serde(default)]
    pub next: Option<String>,
    /// Cloudflare Turnstile token, submitted by the rendered widget.
    #[serde(default, rename = "cf-turnstile-response")]
    pub turnstile_token: Option<String>,
    /// Google reCAPTCHA token.
    #[serde(default, rename = "g-recaptcha-response")]
    pub recaptcha_token: Option<String>,
}

/// `GET /app/login`
pub async fn login_page(
    State(state): State<AppState>,
    MaybeUser(current): MaybeUser,
    Query(query): Query<LoginQuery>,
) -> Result<Response> {
    if current.is_some() {
        return Ok(Redirect::to("/app").into_response());
    }
    let lang = i18n::current();
    let notice = match query.ok.as_deref() {
        Some("logout") => Some(lang.t("auth.login.logged_out").to_string()),
        Some("registered") => Some(lang.t("auth.login.registered").to_string()),
        Some("password") => Some(lang.t("auth.login.password_changed").to_string()),
        _ => None,
    };
    let error = match query.err.as_deref() {
        Some("login") => Some(lang.t("auth.login.invalid").to_string()),
        Some("rate_limited") => Some(lang.t("auth.login.rate_limited").to_string()),
        Some("denied") => Some(lang.t("auth.login.suspended").to_string()),
        Some("session") => Some(lang.t("auth.login.session_expired").to_string()),
        Some("captcha") => Some(lang.t("flash.err.captcha").to_string()),
        _ => None,
    };

    Ok(render(LoginPage {
        site_name: state.settings.site_name(),
        error,
        notice,
        next: safe_next(query.next.as_deref()).unwrap_or_default(),
        email: String::new(),
        captcha: captcha_widget(&state, CaptchaFlow::Login),
        lang,
    }))
}

/// `POST /app/login`
pub async fn login_submit(
    State(state): State<AppState>,
    client: ClientInfo,
    form: std::result::Result<Form<LoginForm>, FormRejection>,
) -> Result<Response> {
    let Ok(Form(payload)) = form else {
        return Ok(render(LoginPage {
            site_name: state.settings.site_name(),
            error: Some(i18n::current().t("auth.login.missing_fields").to_string()),
            notice: None,
            next: String::new(),
            email: String::new(),
            captcha: captcha_widget(&state, CaptchaFlow::Login),
            lang: i18n::current(),
        }));
    };

    if state
        .limiter
        .check(
            &format!("login-ip:{}", client.limiter_key()),
            limits::LOGIN_IP_CAPACITY,
            limits::LOGIN_IP_REFILL_PER_SEC,
        )
        .is_err()
    {
        return Ok(Redirect::to("/app/login?err=rate_limited").into_response());
    }

    let email_normalized = auth::normalize_email(&payload.email);
    if state
        .limiter
        .check(
            &format!("login:{}:{}", client.limiter_key(), email_normalized),
            limits::LOGIN_CAPACITY,
            limits::LOGIN_REFILL_PER_SEC,
        )
        .is_err()
    {
        return Ok(Redirect::to("/app/login?err=rate_limited").into_response());
    }

    // Bot protection runs before any password hashing work.
    if captcha_rejected(
        &state,
        &client,
        CaptchaFlow::Login,
        payload.turnstile_token.as_deref(),
        payload.recaptcha_token.as_deref(),
    )
    .await
    {
        return Ok(redirect_err("/app/login", "captcha").into_response());
    }

    let context = AuthContext {
        pool: &state.db,
        config: &state.config,
        settings: &state.settings,
        user_agent: client.user_agent.as_deref(),
        ip_hash: client.ip_hash.as_deref(),
    };

    match context.login(&payload.email, &payload.password).await {
        Ok((_user, token)) => {
            let target =
                safe_next(payload.next.as_deref().or(Some(""))).unwrap_or_else(|| "/app".into());
            let mut response = Redirect::to(&target).into_response();
            let cookie = crate::routes::api_auth::session_cookie_value(&token, client.secure);
            if let Ok(value) = HeaderValue::from_str(&cookie) {
                response.headers_mut().append(header::SET_COOKIE, value);
            }
            Ok(response)
        }
        Err(AppError::Forbidden(_)) => Ok(Redirect::to("/app/login?err=denied").into_response()),
        Err(AppError::RateLimited { .. }) => {
            Ok(Redirect::to("/app/login?err=rate_limited").into_response())
        }
        Err(_) => Ok(render(LoginPage {
            site_name: state.settings.site_name(),
            error: Some(i18n::current().t("auth.login.invalid").to_string()),
            notice: None,
            next: safe_next(payload.next.as_deref()).unwrap_or_default(),
            email: payload.email,
            captcha: captcha_widget(&state, CaptchaFlow::Login),
            lang: i18n::current(),
        })),
    }
}

/// `GET /app/register`
pub async fn register_page(
    State(state): State<AppState>,
    MaybeUser(current): MaybeUser,
    Query(query): Query<LoginQuery>,
) -> Result<Response> {
    if current.is_some() {
        return Ok(Redirect::to("/app").into_response());
    }
    let mode = state.settings.registration_mode();
    let lang = i18n::current();
    let error = match query.err.as_deref() {
        Some("invite") => Some(lang.t("auth.register.err.invite").to_string()),
        Some("registration") => Some(lang.t("auth.register.err.closed").to_string()),
        Some("conflict") => Some(lang.t("auth.register.err.conflict").to_string()),
        Some("invalid") => Some(lang.t("auth.register.err.invalid").to_string()),
        Some("rate_limited") => Some(lang.t("auth.login.rate_limited").to_string()),
        Some("captcha") => Some(lang.t("flash.err.captcha").to_string()),
        _ => None,
    };

    Ok(render(RegisterPage {
        site_name: state.settings.site_name(),
        error,
        mode: mode.as_str(),
        invite_required: mode == RegistrationMode::Invite,
        registration_open: mode != RegistrationMode::Closed,
        next: safe_next(query.next.as_deref()).unwrap_or_default(),
        email: String::new(),
        display_name: String::new(),
        invite_code: String::new(),
        captcha: captcha_widget(&state, CaptchaFlow::Register),
        lang,
    }))
}

/// `POST /app/register`
pub async fn register_submit(
    State(state): State<AppState>,
    client: ClientInfo,
    form: std::result::Result<Form<RegisterForm>, FormRejection>,
) -> Result<Response> {
    let mode = state.settings.registration_mode();
    let Ok(Form(payload)) = form else {
        return Ok(Redirect::to("/app/register?err=invalid").into_response());
    };

    if state
        .limiter
        .check(
            &format!("register:{}", client.limiter_key()),
            limits::REGISTER_CAPACITY,
            limits::REGISTER_REFILL_PER_SEC,
        )
        .is_err()
    {
        return Ok(Redirect::to("/app/register?err=rate_limited").into_response());
    }

    // Bot protection runs before the account is created.
    if captcha_rejected(
        &state,
        &client,
        CaptchaFlow::Register,
        payload.turnstile_token.as_deref(),
        payload.recaptcha_token.as_deref(),
    )
    .await
    {
        return Ok(redirect_err("/app/register", "captcha").into_response());
    }

    let context = AuthContext {
        pool: &state.db,
        config: &state.config,
        settings: &state.settings,
        user_agent: client.user_agent.as_deref(),
        ip_hash: client.ip_hash.as_deref(),
    };

    let outcome = context
        .register(
            &payload.email,
            &payload.password,
            payload.invite_code.as_deref(),
            payload.display_name.as_deref(),
        )
        .await;

    match outcome {
        Ok(user) => {
            // Sign the new account in immediately.
            let token = context.open_session(user.id).await?;
            let target = safe_next(payload.next.as_deref()).unwrap_or_else(|| "/app".into());
            let mut response = Redirect::to(&target).into_response();
            let cookie = crate::routes::api_auth::session_cookie_value(&token, client.secure);
            if let Ok(value) = HeaderValue::from_str(&cookie) {
                response.headers_mut().append(header::SET_COOKIE, value);
            }
            Ok(response)
        }
        Err(err) => {
            let code = match err {
                AppError::Conflict(_) => "conflict",
                AppError::Forbidden(message) => {
                    if message.contains("invite") {
                        "invite"
                    } else if message.contains("disabled") {
                        "registration"
                    } else {
                        "denied"
                    }
                }
                AppError::RateLimited { .. } => "rate_limited",
                AppError::Validation { message, .. } => {
                    let lang = i18n::current();
                    return Ok(render(RegisterPage {
                        site_name: state.settings.site_name(),
                        // Validation text comes from the service layer in
                        // English; map the known messages onto the catalogue.
                        error: Some(i18n::message(lang, &message)),
                        mode: mode.as_str(),
                        invite_required: mode == RegistrationMode::Invite,
                        registration_open: mode != RegistrationMode::Closed,
                        next: safe_next(payload.next.as_deref()).unwrap_or_default(),
                        email: payload.email,
                        display_name: payload.display_name.unwrap_or_default(),
                        invite_code: payload.invite_code.unwrap_or_default(),
                        captcha: captcha_widget(&state, CaptchaFlow::Register),
                        lang,
                    }));
                }
                _ => "invalid",
            };
            Ok(Redirect::to(&format!("/app/register?err={code}")).into_response())
        }
    }
}

/// `POST /app/logout`
pub async fn logout(
    State(state): State<AppState>,
    client: ClientInfo,
    MaybeUser(current): MaybeUser,
) -> Result<Response> {
    if let Some(current) = current {
        if let Some(session_id) = current.session_id {
            let context = AuthContext {
                pool: &state.db,
                config: &state.config,
                settings: &state.settings,
                user_agent: client.user_agent.as_deref(),
                ip_hash: client.ip_hash.as_deref(),
            };
            let _ = context.logout(session_id, &current.user).await;
        }
    }
    let mut response = redirect_ok("/app/login", "logout").into_response();
    response.headers_mut().append(
        header::SET_COOKIE,
        HeaderValue::from_str(&crate::services::cookies::clear(
            crate::services::cookies::SESSION_COOKIE,
            client.secure,
        ))
        .unwrap_or_else(|_| HeaderValue::from_static("")),
    );
    Ok(response)
}

fn render<T: Template>(template: T) -> Response {
    match template.render() {
        Ok(html) => (StatusCode::OK, Html(html)).into_response(),
        Err(err) => AppError::internal(format!("template error: {err}")).into_response(),
    }
}
