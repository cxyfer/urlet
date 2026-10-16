//! Security response headers.

use axum::extract::{Request, State};
use axum::http::{header, HeaderName, HeaderValue};
use axum::middleware::Next;
use axum::response::Response;

use crate::services::captcha::CaptchaProvider;
use crate::state::AppState;

/// Origins the CAPTCHA widget of `provider` loads from, as
/// `(scripts, frames)`. The provider script is fetched from its own origin and
/// injects a cross-origin iframe, so `script-src`, `connect-src` and `frame-src`
/// all have to name it. The lists stay empty while the feature is off, which
/// leaves the strict `'self'` policy of an instance without bot protection
/// untouched.
fn captcha_origins(
    provider: CaptchaProvider,
) -> (&'static [&'static str], &'static [&'static str]) {
    match provider {
        // One origin serves the script, the iframe and the challenge API.
        CaptchaProvider::Turnstile => (
            &["https://challenges.cloudflare.com"],
            &["https://challenges.cloudflare.com"],
        ),
        // `api.js` comes from google.com and pulls its assets from gstatic.com;
        // the checkbox is normally a google.com iframe, but reCAPTCHA may serve
        // the challenge from recaptcha.google.com instead — Google's CSP
        // guidance names both frame origins.
        CaptchaProvider::Recaptcha => (
            &["https://www.google.com", "https://www.gstatic.com"],
            &["https://www.google.com", "https://recaptcha.google.com"],
        ),
        CaptchaProvider::None => (&[], &[]),
    }
}

fn allow_list(origins: &[&str]) -> String {
    if origins.is_empty() {
        String::new()
    } else {
        format!(" {}", origins.join(" "))
    }
}

/// Content Security Policy. Everything is served from this origin; inline
/// styles are allowed because templates use a few `style` attributes, and
/// scripts are restricted to our own bundle plus the configured CAPTCHA
/// provider. Without those origins the browser refuses to run the widget
/// script, the visitor can never produce a token, and — because verification
/// fails closed — sign-in and registration stop working entirely.
fn content_security_policy(provider: CaptchaProvider) -> String {
    let (scripts, frames) = captcha_origins(provider);
    let scripts = allow_list(scripts);
    let frames = allow_list(frames);
    format!(
        "default-src 'self'; \
         img-src 'self' data:; \
         style-src 'self' 'unsafe-inline'; \
         script-src 'self'{scripts}; \
         connect-src 'self'{scripts}; \
         frame-src 'self'{frames}; \
         font-src 'self'; \
         base-uri 'none'; \
         form-action 'self'; \
         frame-ancestors 'none'"
    )
}

const PERMISSIONS_POLICY: &str = "geolocation=(), microphone=(), camera=(), payment=()";

pub async fn security_headers(
    State(state): State<AppState>,
    request: Request,
    next: Next,
) -> Response {
    let mut response = next.run(request).await;
    let headers = response.headers_mut();

    if let Ok(value) =
        HeaderValue::from_str(&content_security_policy(state.settings.captcha_provider()))
    {
        headers.insert(header::CONTENT_SECURITY_POLICY, value);
    }
    headers.insert(
        header::X_CONTENT_TYPE_OPTIONS,
        HeaderValue::from_static("nosniff"),
    );
    headers.insert(header::X_FRAME_OPTIONS, HeaderValue::from_static("DENY"));
    headers.insert(
        header::REFERRER_POLICY,
        HeaderValue::from_static("strict-origin-when-cross-origin"),
    );
    headers.insert(
        HeaderName::from_static("permissions-policy"),
        HeaderValue::from_static(PERMISSIONS_POLICY),
    );
    headers.insert(
        HeaderName::from_static("cross-origin-opener-policy"),
        HeaderValue::from_static("same-origin"),
    );

    if state.config.is_https() {
        headers.insert(
            header::STRICT_TRANSPORT_SECURITY,
            HeaderValue::from_static("max-age=31536000; includeSubDomains"),
        );
    }

    response
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_disabled_provider_keeps_the_script_policy_closed() {
        let csp = content_security_policy(CaptchaProvider::None);
        assert!(csp.contains("script-src 'self';"), "{csp}");
        assert!(csp.contains("frame-src 'self';"), "{csp}");
        assert!(!csp.contains("https://"), "{csp}");
    }

    #[test]
    fn the_turnstile_origin_is_allowed_where_the_widget_needs_it() {
        let csp = content_security_policy(CaptchaProvider::Turnstile);
        assert!(
            csp.contains("script-src 'self' https://challenges.cloudflare.com;"),
            "{csp}"
        );
        assert!(
            csp.contains("connect-src 'self' https://challenges.cloudflare.com;"),
            "{csp}"
        );
        assert!(
            csp.contains("frame-src 'self' https://challenges.cloudflare.com;"),
            "{csp}"
        );
        // Nothing unrelated is opened up.
        assert!(!csp.contains("gstatic"), "{csp}");
    }

    #[test]
    fn the_recaptcha_origins_are_allowed_where_the_widget_needs_them() {
        let csp = content_security_policy(CaptchaProvider::Recaptcha);
        // `api.js` is on google.com, its assets are on gstatic.com.
        assert!(
            csp.contains("script-src 'self' https://www.google.com https://www.gstatic.com;"),
            "{csp}"
        );
        assert!(
            csp.contains("connect-src 'self' https://www.google.com https://www.gstatic.com;"),
            "{csp}"
        );
        // The checkbox iframe may come from either google.com origin, as
        // Google's CSP guidance requires.
        assert!(
            csp.contains("frame-src 'self' https://www.google.com https://recaptcha.google.com;"),
            "{csp}"
        );
        assert!(!csp.contains("cloudflare"), "{csp}");
    }

    #[test]
    fn the_rest_of_the_policy_is_unchanged() {
        for provider in [
            CaptchaProvider::None,
            CaptchaProvider::Turnstile,
            CaptchaProvider::Recaptcha,
        ] {
            let csp = content_security_policy(provider);
            for directive in [
                "default-src 'self';",
                "img-src 'self' data:;",
                "style-src 'self' 'unsafe-inline';",
                "font-src 'self';",
                "base-uri 'none';",
                "form-action 'self';",
                "frame-ancestors 'none'",
            ] {
                assert!(
                    csp.contains(directive),
                    "{provider:?} lost {directive}: {csp}"
                );
            }
        }
    }
}
