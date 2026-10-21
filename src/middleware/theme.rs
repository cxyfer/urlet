//! Theme middleware.
//!
//! Resolves the visitor's theme cookie once and installs the resulting
//! [`ThemeView`] as a task local for the rest of the request. Templates and
//! error pages can then ask for the stylesheet without threading a state handle
//! through every handler.
//!
//! The middleware is deliberately inside [`super::locale`]: theme names are
//! catalogue strings, so the language has to be known before the view is built.

use axum::extract::{Request, State};
use axum::middleware::Next;
use axum::response::Response;

use crate::i18n;
use crate::services::cookies;
use crate::services::theme::{self, COOKIE_NAME};
use crate::state::AppState;

/// Install the request theme for the rest of the stack.
pub async fn theme(State(state): State<AppState>, request: Request, next: Next) -> Response {
    // JSON and static assets never render a theme. Skipping the task local keeps
    // the hot asset path free of the registry lookup and keeps the API contract
    // (English JSON) independent from the cosmetic cookie.
    let path = request.uri().path();
    if path.starts_with("/assets/") || path.starts_with("/api/") {
        return next.run(request).await;
    }
    let requested = cookies::get(request.headers(), COOKIE_NAME);
    let view = state.themes.view(i18n::current(), requested.as_deref());

    // HTML responses depend on the Cookie header, but only in the same way the
    // language does. `locale` already merges `Cookie` into `Vary` for HTML.
    theme::scope(view, next.run(request)).await
}
