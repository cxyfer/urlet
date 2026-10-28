//! The API reference the instance serves about itself.
//!
//! The page and both contracts are embedded in the binary. A self-hosted
//! instance has to render its own reference with no outbound network and no
//! separate frontend build, which is the same constraint that already keeps the
//! stylesheets and the favicon under `src/assets/`.
//!
//! The page deliberately shares nothing with the themed site: Scalar brings its
//! own stylesheet and layout, and the requirement is a reference that looks
//! independent of whatever theme the instance runs.

use askama::Template;
use axum::extract::State;
use axum::http::{header, HeaderValue};
use axum::response::{IntoResponse, Response};

use crate::assets;
use crate::error::{AppError, Result};
use crate::i18n::{self, Lang};
use crate::state::AppState;

/// Path of the reference page.
///
/// Also listed in `services::slug::RESERVED`: the catch-all `/{slug}` route
/// would otherwise let a short link shadow the page.
pub const PATH: &str = "/docs";

/// Path of the English contract.
pub const SPEC_EN: &str = "/docs/openapi.json";

/// Path of the Traditional Chinese contract.
pub const SPEC_ZH_TW: &str = "/docs/openapi.zh-TW.json";

/// Contracts are embedded assets, so they keep the same one hour public cache
/// as the rest of `src/assets/`.
const CONTRACT_CACHE: &str = "public, max-age=3600";

/// Stands in for the instance's base URL in the authored contract.
///
/// The API only exists on the canonical host, while the reference is served from
/// every host its footer appears on — including the extra-domain short landing,
/// where `/api/v1` deliberately answers 404. Naming the canonical base URL is
/// what keeps the reference's "Test Request" pointed at the host that can serve
/// it. The value is deployment configuration rather than build input, so the
/// generator copies the token through and the response substitutes it. The
/// configured URL does not depend on the request, so the served body is the same
/// for every host and the cache stays correct.
const BASE_URL_TOKEN: &str = "__URLET_BASE_URL__";

/// The contract matching a request language.
///
/// The prose documentation is bilingual, so a Simplified Chinese visitor is
/// offered the English contract rather than a machine translation nobody
/// reviewed. The URL names the language explicitly, which keeps the rule that
/// `/api/v1` always answers in English intact.
pub fn spec_path(lang: Lang) -> &'static str {
    match lang {
        Lang::ZhHant => SPEC_ZH_TW,
        Lang::En | Lang::ZhHans => SPEC_EN,
    }
}

/// Tie an embedded URL to the release.
///
/// Contracts are cached for an hour, so an unversioned URL would keep serving
/// the previous contract after an upgrade. Every other embedded asset in the
/// project is versioned for the same reason.
fn versioned(path: &str) -> String {
    format!("{path}?v={}", crate::VERSION)
}

#[derive(Template)]
#[template(path = "docs.html")]
struct DocsPage {
    title: &'static str,
    lang_code: &'static str,
    spec_url: String,
    version: &'static str,
}

/// The reference page.
pub async fn page() -> Result<Response> {
    let lang = i18n::current();
    let page = DocsPage {
        title: lang.t("docs.title"),
        lang_code: lang.html_lang(),
        spec_url: versioned(spec_path(lang)),
        version: crate::VERSION,
    };
    let html = page.render().map_err(AppError::internal)?;
    // The page is small and names the release in its asset URLs, so it is
    // revalidated rather than cached: an upgrade takes effect immediately.
    Ok((
        [
            (
                header::CONTENT_TYPE,
                HeaderValue::from_static("text/html; charset=utf-8"),
            ),
            (header::CACHE_CONTROL, HeaderValue::from_static("no-cache")),
        ],
        html,
    )
        .into_response())
}

fn contract(body: &'static str, state: &AppState) -> Response {
    let body = if body.contains(BASE_URL_TOKEN) {
        body.replace(BASE_URL_TOKEN, &state.config.base_url)
    } else {
        body.to_string()
    };
    (
        [
            (
                header::CONTENT_TYPE,
                HeaderValue::from_static("application/json; charset=utf-8"),
            ),
            (
                header::CACHE_CONTROL,
                HeaderValue::from_static(CONTRACT_CACHE),
            ),
        ],
        body,
    )
        .into_response()
}

/// The English contract.
pub async fn spec_en(State(state): State<AppState>) -> Response {
    contract(assets::OPENAPI_EN, &state)
}

/// The Traditional Chinese contract.
pub async fn spec_zh_tw(State(state): State<AppState>) -> Response {
    contract(assets::OPENAPI_ZH_TW, &state)
}
