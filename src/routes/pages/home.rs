//! Landing pages: the canonical entry point and the small page shown on extra
//! (redirect-only) domains.
//!
//! The canonical page is rendered for anonymous and signed-in visitors alike,
//! so it only carries the brand, the language switcher and the two entry points
//! into `/app`. Nothing here touches the database: it is cheap enough to serve
//! on every visit to the root.

use askama::Template;
use axum::extract::{Extension, State};
use axum::http::{header, HeaderValue, StatusCode};
use axum::response::{Html, IntoResponse, Response};

use crate::error::AppError;
use crate::i18n::{self, Lang};
use crate::middleware::auth::MaybeUser;
use crate::middleware::domain::HostContext;
use crate::state::AppState;

/// `templates/landing.html`
#[derive(Debug, Template)]
#[template(path = "landing.html")]
pub struct LandingPage {
    pub site_name: String,
    pub lang: Lang,
    /// `true` when the visitor already holds a session.
    pub signed_in: bool,
    /// Label of the signed-in account; empty for anonymous visitors.
    pub email: String,
}

/// `templates/short_landing.html`
#[derive(Debug, Template)]
#[template(path = "short_landing.html")]
pub struct ShortLandingPage {
    pub site_name: String,
    pub canonical_url: String,
    pub lang: Lang,
}

/// `GET /` — the public landing page.
///
/// Always answers `200`: the top bar and the hero adapt to the session, so
/// there is no reason to bounce anyone to `/app` or the login screen. On an
/// extra domain it renders the redirect-only landing page instead.
pub async fn home(
    State(state): State<AppState>,
    MaybeUser(current): MaybeUser,
    Extension(host): Extension<HostContext>,
) -> Result<Response, AppError> {
    if !host.canonical {
        let page = ShortLandingPage {
            site_name: state.settings.site_name(),
            canonical_url: state.config.base_url.clone(),
            lang: i18n::current(),
        };
        let mut response = render(page)?;
        response
            .headers_mut()
            .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
        response
            .headers_mut()
            .insert("x-robots-tag", HeaderValue::from_static("noindex"));
        return Ok(response);
    }

    let (signed_in, email) = match current {
        Some(current) => (true, current.user.label().to_string()),
        None => (false, String::new()),
    };

    let page = LandingPage {
        site_name: state.settings.site_name(),
        lang: i18n::current(),
        signed_in,
        email,
    };
    render(page)
}

fn render<T: Template>(page: T) -> Result<Response, AppError> {
    match page.render() {
        Ok(html) => Ok((StatusCode::OK, Html(html)).into_response()),
        Err(err) => Err(AppError::internal(format!("template error: {err}"))),
    }
}
