//! Landing page: the public entry point of an instance.
//!
//! Unlike the rest of the interface this page is rendered for anonymous and
//! signed-in visitors alike, so it only carries the brand, the language
//! switcher and the two entry points into `/app`. Nothing here touches the
//! database: it is cheap enough to serve on every visit to the root.

use askama::Template;
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{Html, IntoResponse, Response};

use crate::error::AppError;
use crate::i18n::{self, Lang};
use crate::middleware::auth::MaybeUser;
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

/// `GET /` — the public landing page.
///
/// Always answers `200`: the top bar and the hero adapt to the session, so
/// there is no reason to bounce anyone to `/app` or the login screen.
pub async fn home(
    State(state): State<AppState>,
    MaybeUser(current): MaybeUser,
) -> Result<Response, AppError> {
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

    match page.render() {
        Ok(html) => Ok((StatusCode::OK, Html(html)).into_response()),
        Err(err) => Err(AppError::internal(format!("template error: {err}"))),
    }
}
