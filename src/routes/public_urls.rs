//! The public shortener endpoints: the landing page form and the JSON API.
//!
//! Both accept the same payload and call the same service
//! ([`crate::services::public_urls::create`]), which enforces the access level,
//! the field switches and the rate limits. Only the presentation differs: the
//! form answers with a redirect back to the landing page, the API with JSON.
//!
//! Bot protection is verified here because it needs the submitted token: the
//! flow depends on the caller (anonymous visitors use `public_captcha`, a
//! signed-in session `captcha_on_create`) and a Bearer API token is exempt,
//! exactly like the owner-facing create endpoint.

use axum::extract::rejection::FormRejection;
use axum::extract::{Extension, State};
use axum::http::{header, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::{Form, Json};
use serde::{Deserialize, Serialize};

use crate::error::{ApiError, AppError};
use crate::middleware::auth::{ClientInfo, MaybeUser};
use crate::middleware::domain::HostContext;
use crate::routes::pages::urls::{number_field, split_tags};
use crate::routes::pages::{captcha_rejected, encode, redirect_err};
use crate::routes::verify_captcha;
use crate::services::public_urls::{self, PublicInput};
use crate::state::AppState;

/// Body of the landing page form (`POST /`).
#[derive(Debug, Clone, Default, Deserialize)]
pub struct PublicCreateForm {
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

impl PublicCreateForm {
    /// Convert the submitted form, rejecting a malformed number.
    ///
    /// A blank field means "not submitted"; `max_clicks=abc` must not be turned
    /// into "no click cap", which would both drop what the visitor asked for and
    /// slip past the field switch that refuses a disabled input.
    fn into_input(self) -> Result<PublicInput, AppError> {
        Ok(PublicInput {
            target_url: self.target_url,
            domain: self.domain,
            slug: self.slug,
            title: self.title,
            tags: split_tags(&self.tags),
            password: self.password,
            expires_at: self.expires_at,
            max_clicks: number_field(&self.max_clicks, "max_clicks")?,
            redirect_mode: self.redirect_mode,
            splash_delay_seconds: number_field(&self.splash_delay_seconds, "splash_delay_seconds")?,
            splash_title: self.splash_title,
            splash_message: self.splash_message,
        })
    }
}

/// Body of `POST /api/v1/public/urls`.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct PublicCreatePayload {
    pub target_url: String,
    #[serde(default)]
    pub domain: Option<String>,
    #[serde(default)]
    pub slug: Option<String>,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub password: Option<String>,
    #[serde(default)]
    pub expires_at: Option<String>,
    #[serde(default)]
    pub max_clicks: Option<i64>,
    #[serde(default)]
    pub redirect_mode: Option<String>,
    #[serde(default)]
    pub splash_delay_seconds: Option<i64>,
    #[serde(default)]
    pub splash_title: Option<String>,
    #[serde(default)]
    pub splash_message: Option<String>,
    /// Bot protection token; only needed for the flows that are protected.
    #[serde(default)]
    pub captcha_token: Option<String>,
}

impl PublicCreatePayload {
    fn into_input(self) -> PublicInput {
        PublicInput {
            target_url: self.target_url,
            domain: self.domain,
            slug: self.slug,
            title: self.title,
            tags: self.tags,
            password: self.password,
            expires_at: self.expires_at,
            max_clicks: self.max_clicks,
            redirect_mode: self.redirect_mode,
            splash_delay_seconds: self.splash_delay_seconds,
            splash_title: self.splash_title,
            splash_message: self.splash_message,
        }
    }
}

/// What the public API answers with.
///
/// Deliberately smaller than the owner-facing [`crate::routes::dto::UrlDto`]:
/// the caller has no account to manage the link with, and the identifier is not
/// theirs to keep.
#[derive(Debug, Clone, Serialize)]
pub struct PublicCreatedDto {
    pub domain: String,
    pub slug: String,
    pub short_url: String,
    pub target_url: String,
    pub created_at: String,
}

/// `POST /` — the shortener form on the landing page.
pub async fn create_form(
    State(state): State<AppState>,
    client: ClientInfo,
    MaybeUser(current): MaybeUser,
    Extension(host): Extension<HostContext>,
    form: std::result::Result<Form<PublicCreateForm>, FormRejection>,
) -> Result<Response, AppError> {
    // The public form only exists on the canonical host: an extra domain is
    // redirect-only and its `/` is the small "follow your link" page.
    if !host.canonical {
        return Err(AppError::NotFound);
    }
    let Ok(Form(mut payload)) = form else {
        return Ok(redirect_err("/", "invalid").into_response());
    };
    // The provider tokens are taken before the payload is consumed: the checks
    // that must run first (access level, fields, budgets) and the bot protection
    // call after them both need them.
    let turnstile_token = payload.turnstile_token.take();
    let recaptcha_token = payload.recaptcha_token.take();

    let input = match payload.into_input() {
        Ok(input) => input,
        Err(err) => {
            return Ok(
                redirect_err("/", crate::routes::pages::urls::error_code(&err)).into_response(),
            )
        }
    };
    // Access level, field switches and rate limits first: only a request that
    // passed them is allowed to spend a provider call.
    let authorized = match public_urls::authorize(&state, &client, current.as_ref(), &input) {
        Ok(authorized) => authorized,
        Err(err) => {
            return Ok(
                redirect_err("/", crate::routes::pages::urls::error_code(&err)).into_response(),
            )
        }
    };

    let anonymous = current.is_none();
    let via_token = current
        .as_ref()
        .map(|current| current.via_token())
        .unwrap_or(false);
    if !via_token {
        if let Some(flow) = public_urls::guard_flow(&state.settings, anonymous) {
            if captcha_rejected(
                &state,
                &client,
                flow,
                turnstile_token.as_deref(),
                recaptcha_token.as_deref(),
            )
            .await
            {
                return Ok(redirect_err("/", "captcha").into_response());
            }
        }
    }

    // The shared visitor budget is charged only now, so an attempt that failed
    // bot protection cannot drain it for everybody else.
    let authorized = match public_urls::admit(&state, current.as_ref(), authorized) {
        Ok(authorized) => authorized,
        Err(err) => {
            return Ok(
                redirect_err("/", crate::routes::pages::urls::error_code(&err)).into_response(),
            )
        }
    };

    match public_urls::create(&state, &client, current.as_ref(), input, authorized).await {
        Ok(url) => {
            let domain = state.domain_hostname(url.domain_id);
            // The landing page reads the link back from the database and prints
            // its short URL; nothing is taken from the query string verbatim.
            let target = format!(
                "/?shortened={}&domain={}",
                encode(&url.slug),
                encode(&domain)
            );
            let mut response = axum::response::Redirect::to(&target).into_response();
            response
                .headers_mut()
                .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
            Ok(response)
        }
        Err(err) => {
            Ok(redirect_err("/", crate::routes::pages::urls::error_code(&err)).into_response())
        }
    }
}

/// `POST /api/v1/public/urls`
pub async fn create_api(
    State(state): State<AppState>,
    client: ClientInfo,
    MaybeUser(current): MaybeUser,
    Json(mut payload): Json<PublicCreatePayload>,
) -> Result<Response, ApiError> {
    let captcha_token = payload.captcha_token.take();
    let input = payload.into_input();
    // Access level, field switches and rate limits first: only a request that
    // passed them is allowed to spend a provider call.
    let authorized = public_urls::authorize(&state, &client, current.as_ref(), &input)?;

    let anonymous = current.is_none();
    let via_token = current
        .as_ref()
        .map(|current| current.via_token())
        .unwrap_or(false);
    if !via_token {
        if let Some(flow) = public_urls::guard_flow(&state.settings, anonymous) {
            verify_captcha(&state, &client, flow, captcha_token.as_deref()).await?;
        }
    }

    // The shared visitor budget is charged only now, so an attempt that failed
    // bot protection cannot drain it for everybody else.
    let authorized = public_urls::admit(&state, current.as_ref(), authorized)?;

    let url = public_urls::create(&state, &client, current.as_ref(), input, authorized).await?;
    let domain = state.domain_hostname(url.domain_id);
    Ok((
        StatusCode::CREATED,
        Json(PublicCreatedDto {
            short_url: format!("{}://{domain}/{}", state.config.base_scheme, url.slug),
            domain,
            slug: url.slug,
            target_url: url.target_url,
            created_at: url.created_at,
        }),
    )
        .into_response())
}
