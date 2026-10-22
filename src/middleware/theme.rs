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
use crate::middleware::auth::{authenticate, Authenticated};
use crate::services::cookies;
use crate::services::settings::ThemeSwitching;
use crate::services::theme::{self, COOKIE_NAME, PREVIEW_COOKIE_NAME};
use crate::state::AppState;

/// Does this path need the account-aware view?
///
/// `/` and `/app` (with its subpaths) render the signed-in shell or the landing
/// page and need the policy check. The redirect surface (`/{slug}`,
/// `/{slug}/unlock`) and health endpoints do not: authenticating there would add
/// two queries to the hot redirect path and break the NFR-PERF-1 budget.
///
/// The match is on the path boundary, not the bare prefix. `app` is reserved as
/// a slug, but a slug that merely begins with it (`/apple`) is an ordinary link:
/// matching `starts_with("/app")` would give such a link page the session lookup
/// the redirect surface avoids, and — under the `all` policy — the picker that
/// only the pages carrying site chrome may show.
fn needs_account(path: &str) -> bool {
    path == "/" || path == "/app" || path.starts_with("/app/")
}

/// Install the request theme for the rest of the stack.
pub async fn theme(State(state): State<AppState>, request: Request, next: Next) -> Response {
    // JSON and static assets never render a theme. Skipping the task local keeps
    // the hot asset path free of the registry lookup and keeps the API contract
    // (English JSON) independent from the cosmetic cookie.
    let path = request.uri().path();
    if path.starts_with("/assets/") || path.starts_with("/api/") {
        return next.run(request).await;
    }

    let lang = i18n::current();
    let switching = state.settings.theme_switching();

    // The redirect surface only needs the cookie. Under `admin` it deliberately
    // ignores the cookie, matching the anonymous default, but it never resolves
    // a session: the redirect hot path must stay a single indexed lookup. It also
    // never offers the picker, even under `all`: a link page is about the link,
    // and the visitor can switch from the landing page it points back to.
    if !needs_account(path) {
        let requested = if switching == ThemeSwitching::AdminOnly {
            None
        } else {
            cookies::get(request.headers(), COOKIE_NAME)
        };
        let view = state
            .themes
            .view(lang, requested.as_deref())
            .with_can_switch(false);
        return theme::scope(view, next.run(request)).await;
    }

    // The theme policy needs the account. Resolve authentication once and put
    // it in the request extensions so the page extractors reuse it instead of
    // querying the session/token a second time.
    let (mut parts, body) = request.into_parts();
    let current = authenticate(&parts, &state).await;
    parts.extensions.insert(Authenticated(current.clone()));
    let request = Request::from_parts(parts, body);

    let is_admin = current
        .as_ref()
        .map(|current| current.is_admin())
        .unwrap_or(false);

    // Preview is reserved for administrators and may name a disabled theme,
    // including an uploaded one.
    let preview = if is_admin {
        cookies::get(request.headers(), PREVIEW_COOKIE_NAME)
            .and_then(|value| state.themes.preview_view(lang, &value))
    } else {
        None
    };

    let view = if let Some(preview) = preview {
        preview
    } else {
        // Under `admin` policy the cookie is ignored for non-admin accounts, so
        // the permission is enforced at render time and not only on the switch
        // endpoint. A stale cookie from a previous `all` policy also falls back
        // to the default.
        let requested = if switching == ThemeSwitching::AdminOnly && !is_admin {
            None
        } else {
            cookies::get(request.headers(), COOKIE_NAME)
        };
        let mut view = state.themes.view(lang, requested.as_deref());
        // No session: the site-wide policy decides on its own. `all` is
        // literally everyone, so a visitor gets the same picker as a member.
        view.can_switch = match current.as_ref() {
            Some(current) => theme::can_switch(&state.settings, &current.user),
            None => theme::anonymous_can_switch(&state.settings),
        };
        view
    };

    // HTML responses depend on the Cookie header, but only in the same way the
    // language does. `locale` already merges `Cookie` into `Vary` for HTML.
    theme::scope(view, next.run(request)).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_redirect_surface_skips_authentication() {
        assert!(needs_account("/"));
        assert!(needs_account("/app"));
        assert!(needs_account("/app/"));
        assert!(needs_account("/app/admin/themes"));
        assert!(!needs_account("/promo"));
        assert!(!needs_account("/promo/unlock"));
        assert!(!needs_account("/health"));
        // A slug that merely begins with `app` is a link, not the app shell.
        assert!(!needs_account("/apple"));
        assert!(!needs_account("/apple/unlock"));
        assert!(!needs_account("/application"));
    }
}
