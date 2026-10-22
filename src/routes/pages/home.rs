//! Landing pages: the canonical entry point and the small page shown on extra
//! (redirect-only) domains.
//!
//! The canonical page is rendered for anonymous and signed-in visitors alike,
//! so it carries the brand, the language switcher, the two entry points into
//! `/app` and — when the administrator allows it — the public shortener. The
//! page stays cheap: the domain list comes from the in-memory registry and the
//! form is only built when somebody is actually allowed to use it.

use askama::Template;
use axum::extract::{Extension, Query, State};
use axum::http::{header, HeaderValue, StatusCode};
use axum::response::{Html, IntoResponse, Response};
use serde::Deserialize;

use crate::db::domains;
use crate::db::urls::{self, UrlState};
use crate::error::AppError;
use crate::i18n::{self, Lang};
use crate::middleware::auth::MaybeUser;
use crate::middleware::domain::HostContext;
use crate::routes::pages::{captcha_widget, flash_from_query, Flash};
use crate::services::captcha::CaptchaWidget;
use crate::services::domains::{self as domain_service, DomainOption};
use crate::services::public_urls;
use crate::services::settings::PublicFields;
use crate::services::theme::ThemeView;
use crate::state::AppState;
use crate::themed_page;

/// Query string of `GET /`.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct HomeQuery {
    /// Slug of the link the public form just created.
    #[serde(default)]
    pub shortened: Option<String>,
    /// Domain the created link lives on; absent means the site default.
    #[serde(default)]
    pub domain: Option<String>,
    /// Flash codes, understood by [`flash_from_query`].
    #[serde(default)]
    pub ok: Option<String>,
    #[serde(default)]
    pub err: Option<String>,
}

/// The public shortener as the landing page renders it.
#[derive(Debug)]
pub struct ShortenView {
    /// Whether the form is rendered at all: false when the access level does
    /// not include this visitor.
    pub show: bool,
    /// The short URL of the link that was just created, if any.
    pub result: Option<String>,
    /// Optional inputs the administrator enabled.
    pub fields: PublicFields,
    /// Domains offered by the form; empty unless the domain switch is on.
    pub domains: Vec<DomainOption>,
}

/// `templates/landing.html`
#[derive(Debug)]
pub struct LandingPage {
    pub site_name: String,
    pub lang: Lang,
    /// `true` when the visitor already holds a session.
    pub signed_in: bool,
    /// Label of the signed-in account; empty for anonymous visitors.
    pub email: String,
    pub theme: ThemeView,
    /// Error or success banner of a public form submission.
    pub flash: Option<Flash>,
    /// Bot protection widget of the public form, when the flow is protected.
    pub captcha: Option<CaptchaWidget>,
    /// The public shortener (see [`ShortenView`]).
    pub shorten: ShortenView,
}

themed_page!(landing_templates, LandingView, LandingPage,
    Verdant => "landing.html",
    Aurora => "themes/aurora/landing.html",
    Paper => "themes/paper/landing.html",
    Signal => "themes/signal/landing.html",
    Gilded => "themes/gilded/landing.html",
    Swiss => "themes/swiss/landing.html",
);

/// `templates/short_landing.html`
#[derive(Debug)]
pub struct ShortLandingPage {
    pub site_name: String,
    pub canonical_url: String,
    pub lang: Lang,
    pub theme: ThemeView,
}

themed_page!(short_landing_templates, ShortLandingView, ShortLandingPage,
    Verdant => "short_landing.html",
    Aurora => "themes/aurora/short_landing.html",
    Paper => "themes/paper/short_landing.html",
    Signal => "themes/signal/short_landing.html",
    Gilded => "themes/gilded/short_landing.html",
    Swiss => "themes/swiss/short_landing.html",
);

/// `GET /` — the public landing page.
///
/// Always answers `200`: the top bar and the hero adapt to the session, so
/// there is no reason to bounce anyone to `/app` or the login screen. On an
/// extra domain it renders the redirect-only landing page instead. When the
/// access level allows the visitor to create a link, the page also carries the
/// public shortener and the result of the last submission.
pub async fn home(
    State(state): State<AppState>,
    MaybeUser(current): MaybeUser,
    Extension(host): Extension<HostContext>,
    Query(query): Query<HomeQuery>,
) -> Result<Response, AppError> {
    if !host.canonical {
        let page = ShortLandingPage {
            site_name: state.settings.site_name(),
            canonical_url: state.config.base_url.clone(),
            lang: i18n::current(),
            theme: crate::services::theme::current(),
        }
        .themed();
        let mut response = render(page)?;
        response
            .headers_mut()
            .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
        response
            .headers_mut()
            .insert("x-robots-tag", HeaderValue::from_static("noindex"));
        return Ok(response);
    }

    let (signed_in, email) = match &current {
        Some(current) => (true, current.user.label().to_string()),
        None => (false, String::new()),
    };
    let theme = match &current {
        Some(current) => crate::services::theme::for_user(&state.settings, &current.user),
        None => crate::services::theme::current(),
    };

    let anonymous = current.is_none();
    let allowed =
        public_urls::access(&state.settings).allows(current.as_ref().map(|current| &current.user));
    let fields = state.settings.public_fields();
    let domains = match (&current, fields.domain) {
        (_, false) => Vec::new(),
        // A signed-in account sees the domains its group allows; a visitor gets
        // the site-wide list, which is what the ungrouped Guest account sees.
        (Some(current), true) => {
            crate::services::domain_policy::effective(&state.db, &current.user)
                .await?
                .options(&state.domains)
        }
        (None, true) => state.domains.active_options(),
    };
    let captcha = if allowed {
        public_urls::guard_flow(&state.settings, anonymous)
            .and_then(|flow| captcha_widget(&state, flow))
    } else {
        None
    };

    let page = LandingPage {
        site_name: state.settings.site_name(),
        lang: i18n::current(),
        signed_in,
        email,
        theme,
        flash: flash_from_query(query.ok.as_deref(), query.err.as_deref()),
        captcha,
        shorten: ShortenView {
            show: allowed,
            result: created_short_url(&state, &query).await,
            fields,
            domains,
        },
    }
    .themed();
    render(page)
}

/// The short URL of the link the public form just created.
///
/// The query string only names the link; every value that is displayed is read
/// back from the database, so a hand-written `?shortened=` cannot make the page
/// print an arbitrary URL. The destination is never shown: the visitor has no
/// account, and the same page must not leak the target of a password-protected
/// link created by somebody else.
async fn created_short_url(state: &AppState, query: &HomeQuery) -> Option<String> {
    let slug = query.shortened.as_deref().map(str::trim)?;
    if slug.is_empty() {
        return None;
    }
    let domain = match query
        .domain
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        Some(raw) => domain_service::find_active_by_input(&state.db, raw)
            .await
            .ok()??,
        None => domains::active_default(&state.db).await.ok()??,
    };
    let url = urls::find_by_domain_slug(&state.db, domain.id, slug)
        .await
        .ok()??;
    if matches!(url.state(), UrlState::Deleted | UrlState::Disabled) {
        return None;
    }
    Some(format!(
        "{}://{}/{}",
        state.config.base_scheme, domain.hostname, url.slug
    ))
}

fn render<T: Template>(page: T) -> Result<Response, AppError> {
    match page.render() {
        Ok(html) => Ok((StatusCode::OK, Html(html)).into_response()),
        Err(err) => Err(AppError::internal(format!("template error: {err}"))),
    }
}
