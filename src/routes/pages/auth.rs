//! Login, registration and logout pages.

use askama::Template;
use axum::extract::rejection::FormRejection;
use axum::extract::{Query, State};
use axum::http::{header, HeaderValue, StatusCode};
use axum::response::{Html, IntoResponse, Redirect, Response};
use axum::Form;
use serde::Deserialize;

use crate::error::{AppError, Result};
use crate::middleware::auth::{ClientInfo, MaybeUser};
use crate::routes::pages::{redirect_ok, safe_next};
use crate::services::auth::{self, AuthContext};
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
    let notice = match query.ok.as_deref() {
        Some("logout") => Some("You have been signed out.".to_string()),
        Some("registered") => Some("Account created. Please sign in.".to_string()),
        Some("password") => Some("Password changed. Please sign in again.".to_string()),
        _ => None,
    };
    let error = match query.err.as_deref() {
        Some("login") => Some("Invalid email or password.".to_string()),
        Some("rate_limited") => Some("Too many attempts. Please wait a moment.".to_string()),
        Some("denied") => Some("That account is suspended.".to_string()),
        Some("session") => Some("Your session expired. Please sign in again.".to_string()),
        _ => None,
    };

    Ok(render(LoginPage {
        site_name: state.settings.site_name(),
        error,
        notice,
        next: safe_next(query.next.as_deref()).unwrap_or_default(),
        email: String::new(),
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
            error: Some("Please fill in every field.".to_string()),
            notice: None,
            next: String::new(),
            email: String::new(),
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
            error: Some("Invalid email or password.".to_string()),
            notice: None,
            next: safe_next(payload.next.as_deref()).unwrap_or_default(),
            email: payload.email,
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
    let error = match query.err.as_deref() {
        Some("invite") => Some("That invite code is not valid.".to_string()),
        Some("registration") => Some("Registration is currently closed.".to_string()),
        Some("conflict") => Some("An account with that email already exists.".to_string()),
        Some("invalid") => Some("Please check the form and try again.".to_string()),
        Some("rate_limited") => Some("Too many attempts. Please wait a moment.".to_string()),
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
                    return Ok(render(RegisterPage {
                        site_name: state.settings.site_name(),
                        error: Some(message),
                        mode: mode.as_str(),
                        invite_required: mode == RegistrationMode::Invite,
                        registration_open: mode != RegistrationMode::Closed,
                        next: safe_next(payload.next.as_deref()).unwrap_or_default(),
                        email: payload.email,
                        display_name: payload.display_name.unwrap_or_default(),
                        invite_code: payload.invite_code.unwrap_or_default(),
                    }))
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
