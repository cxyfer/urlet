//! Bot protection: Cloudflare Turnstile and Google reCAPTCHA.
//!
//! The service is deliberately self contained so any future flow (for example an
//! anonymous "create a short link" form) can be protected by building a
//! [`CaptchaGuard`] for it and calling [`CaptchaGuard::verify`].
//!
//! Failure policy is *fail closed*: a missing token, a provider that says
//! `success != true`, a non 2xx response, a connection error, a timeout and an
//! unparsable body all reject the request. A disabled guard is a no-op.
//!
//! The provider call is blocking and therefore runs on
//! [`tokio::task::spawn_blocking`]; it uses `ureq` with the pure Rust `rustls`
//! backend and the `webpki-roots` trust store so the musl container needs no
//! OpenSSL.

use std::net::IpAddr;
use std::time::Duration;

use crate::i18n::Lang;
use crate::services::settings::Settings;

/// `siteverify` endpoint of Cloudflare Turnstile.
pub const TURNSTILE_VERIFY_URL: &str = "https://challenges.cloudflare.com/turnstile/v0/siteverify";
/// `siteverify` endpoint of Google reCAPTCHA.
pub const RECAPTCHA_VERIFY_URL: &str = "https://www.google.com/recaptcha/api/siteverify";

/// Longest accepted site/secret key.
pub const MAX_KEY_LEN: usize = 200;

/// How long a `siteverify` call may take before it counts as a failure.
pub const VERIFY_TIMEOUT: Duration = Duration::from_secs(5);

/// Message shared by the JSON API and the HTML error page. The HTML page maps
/// it onto the `error.captcha` catalogue entry; see `crate::i18n::message_key`.
pub const FAILURE_MESSAGE: &str = "The bot protection check failed. Please try again.";

/// A supported bot protection provider.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CaptchaProvider {
    /// Bot protection is off.
    None,
    Turnstile,
    Recaptcha,
}

impl CaptchaProvider {
    /// Parse a stored/configured provider name. Unknown values disable the
    /// feature rather than failing open.
    pub fn parse(value: &str) -> Self {
        match value.trim().to_ascii_lowercase().as_str() {
            "turnstile" => Self::Turnstile,
            "recaptcha" => Self::Recaptcha,
            _ => Self::None,
        }
    }

    /// True when `value` names a provider the settings form may store.
    pub fn is_known(value: &str) -> bool {
        matches!(
            value.trim().to_ascii_lowercase().as_str(),
            "none" | "turnstile" | "recaptcha"
        )
    }

    /// Canonical name, as stored in the settings table.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Turnstile => "turnstile",
            Self::Recaptcha => "recaptcha",
        }
    }

    /// Server side verification endpoint.
    pub fn verify_url(self) -> &'static str {
        match self {
            Self::None => "",
            Self::Turnstile => TURNSTILE_VERIFY_URL,
            Self::Recaptcha => RECAPTCHA_VERIFY_URL,
        }
    }

    /// Script that renders the widget, without the language parameter.
    pub fn script_url(self) -> &'static str {
        match self {
            Self::None => "",
            Self::Turnstile => "https://challenges.cloudflare.com/turnstile/v0/api.js",
            Self::Recaptcha => "https://www.google.com/recaptcha/api.js",
        }
    }

    /// `class` of the element the provider script fills in.
    pub fn widget_class(self) -> &'static str {
        match self {
            Self::None => "",
            Self::Turnstile => "cf-turnstile",
            Self::Recaptcha => "g-recaptcha",
        }
    }

    /// Name of the form field carrying the token.
    pub fn form_field(self) -> &'static str {
        match self {
            Self::None => "",
            Self::Turnstile => "cf-turnstile-response",
            Self::Recaptcha => "g-recaptcha-response",
        }
    }

    /// Value of the `hl` query parameter of the provider script.
    ///
    /// Both providers expect a region flavoured tag (`zh-TW`, `zh-CN`) rather
    /// than the script subtag the UI uses internally.
    pub fn provider_lang(self, lang: Lang) -> &'static str {
        match lang {
            Lang::En => "en",
            Lang::ZhHant => "zh-TW",
            Lang::ZhHans => "zh-CN",
        }
    }
}

/// A flow that can be protected.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CaptchaFlow {
    Register,
    Login,
    Create,
}

/// Everything a template needs to render the provider widget.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CaptchaWidget {
    pub provider: &'static str,
    /// Script URL including the `hl` language parameter.
    pub script_url: String,
    pub widget_class: &'static str,
    pub site_key: String,
    pub field_name: &'static str,
}

impl CaptchaWidget {
    /// Widget for `provider`, localised with `lang`. `None` when disabled.
    pub fn new(provider: CaptchaProvider, site_key: &str, lang: Lang) -> Option<Self> {
        if provider == CaptchaProvider::None || site_key.trim().is_empty() {
            return None;
        }
        Some(Self {
            provider: provider.as_str(),
            script_url: format!(
                "{}?hl={}",
                provider.script_url(),
                provider.provider_lang(lang)
            ),
            widget_class: provider.widget_class(),
            site_key: site_key.trim().to_string(),
            field_name: provider.form_field(),
        })
    }
}

/// Why a token was rejected.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CaptchaError {
    /// No token was submitted at all.
    Missing,
    /// The provider answered `success: false`.
    Rejected,
    /// The provider could not be reached, timed out or answered nonsense.
    Unavailable,
}

impl CaptchaError {
    /// Stable, non leaking code for logs and tests.
    pub fn code(self) -> &'static str {
        match self {
            Self::Missing => "missing",
            Self::Rejected => "rejected",
            Self::Unavailable => "unavailable",
        }
    }
}

impl std::fmt::Display for CaptchaError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Missing => "no captcha token was submitted",
            Self::Rejected => "the captcha token was rejected",
            Self::Unavailable => "the captcha provider could not be reached",
        })
    }
}

/// A ready to use guard for one flow.
///
/// A guard that does not satisfy every enable condition is inert: [`verify`]
/// returns `Ok(())` and [`widget`] returns `None`, so an unprotected flow
/// behaves exactly as before the feature existed.
///
/// [`verify`]: CaptchaGuard::verify
/// [`widget`]: CaptchaGuard::widget
#[derive(Debug, Clone)]
pub struct CaptchaGuard {
    provider: CaptchaProvider,
    site_key: String,
    secret_key: String,
    verify_url: String,
}

impl CaptchaGuard {
    /// Guard for `flow`, honouring the four enable conditions: a provider other
    /// than `none`, a site key, a secret key and the per-flow switch.
    ///
    /// `verify_url_override` replaces the provider endpoint; production passes
    /// `None` and tests point it at a local mock.
    pub fn from_settings(
        settings: &Settings,
        flow: CaptchaFlow,
        verify_url_override: Option<&str>,
    ) -> Self {
        let enabled_for_flow = match flow {
            CaptchaFlow::Register => settings.captcha_on_register(),
            CaptchaFlow::Login => settings.captcha_on_login(),
            CaptchaFlow::Create => settings.captcha_on_create(),
        };
        let provider = settings.captcha_provider();
        let site_key = settings.captcha_site_key();
        let secret_key = settings.captcha_secret_key();

        if provider == CaptchaProvider::None
            || site_key.trim().is_empty()
            || secret_key.trim().is_empty()
            || !enabled_for_flow
        {
            return Self::disabled();
        }

        let verify_url = match verify_url_override {
            Some(url) => url.to_string(),
            None => provider.verify_url().to_string(),
        };
        Self {
            provider,
            site_key,
            secret_key,
            verify_url,
        }
    }

    /// Build a guard from explicit parts (tests, and future callers that have
    /// already resolved the keys).
    pub fn new(
        provider: CaptchaProvider,
        site_key: &str,
        secret_key: &str,
        verify_url: &str,
    ) -> Self {
        Self {
            provider,
            site_key: site_key.to_string(),
            secret_key: secret_key.to_string(),
            verify_url: verify_url.to_string(),
        }
    }

    /// The inert guard: every check passes, nothing is rendered.
    pub fn disabled() -> Self {
        Self {
            provider: CaptchaProvider::None,
            site_key: String::new(),
            secret_key: String::new(),
            verify_url: String::new(),
        }
    }

    /// True when this guard actually verifies tokens.
    pub fn enabled(&self) -> bool {
        self.provider != CaptchaProvider::None
            && !self.site_key.trim().is_empty()
            && !self.secret_key.trim().is_empty()
            && !self.verify_url.trim().is_empty()
    }

    pub fn provider(&self) -> CaptchaProvider {
        self.provider
    }

    pub fn site_key(&self) -> &str {
        &self.site_key
    }

    /// Widget markup data, or `None` when the flow is not protected.
    pub fn widget(&self, lang: Lang) -> Option<CaptchaWidget> {
        if !self.enabled() {
            return None;
        }
        CaptchaWidget::new(self.provider, &self.site_key, lang)
    }

    /// Verify a submitted token. A disabled guard always succeeds.
    ///
    /// `client_ip` is forwarded as `remoteip` when known; the provider API
    /// accepts requests without it.
    pub async fn verify(
        &self,
        token: Option<&str>,
        client_ip: Option<IpAddr>,
    ) -> Result<(), CaptchaError> {
        if !self.enabled() {
            return Ok(());
        }
        let token = token
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .ok_or(CaptchaError::Missing)?;

        let request = SiteverifyRequest {
            url: self.verify_url.clone(),
            secret: self.secret_key.clone(),
            response: token.to_string(),
            remote_ip: client_ip.map(|ip| ip.to_string()),
        };
        let provider = self.provider.as_str();

        // `ureq` is blocking: keep it off the async runtime.
        match tokio::task::spawn_blocking(move || request.send()).await {
            Ok(Ok(true)) => Ok(()),
            Ok(Ok(false)) => {
                tracing::warn!(provider, "captcha token rejected by the provider");
                Err(CaptchaError::Rejected)
            }
            // The message never contains the token or the secret.
            Ok(Err(reason)) => {
                tracing::warn!(provider, reason = %reason, "captcha verification failed");
                Err(CaptchaError::Unavailable)
            }
            Err(err) => {
                tracing::warn!(provider, error = %err, "captcha verification task failed");
                Err(CaptchaError::Unavailable)
            }
        }
    }
}

/// Pick the token out of a submitted form, whichever provider field carries it.
pub fn submitted_token<'a>(
    turnstile: Option<&'a str>,
    recaptcha: Option<&'a str>,
) -> Option<&'a str> {
    turnstile.or(recaptcha)
}

/// One blocking `siteverify` call.
#[derive(Debug)]
struct SiteverifyRequest {
    url: String,
    secret: String,
    response: String,
    remote_ip: Option<String>,
}

impl SiteverifyRequest {
    /// `Ok(true)` when the provider confirmed the token, `Ok(false)` when it
    /// explicitly rejected it, `Err(reason)` when the answer is unusable.
    fn send(&self) -> Result<bool, String> {
        let agent = ureq::builder()
            .timeout(VERIFY_TIMEOUT)
            .timeout_connect(VERIFY_TIMEOUT)
            .build();

        let mut form: Vec<(&str, &str)> = vec![
            ("secret", self.secret.as_str()),
            ("response", self.response.as_str()),
        ];
        if let Some(ip) = self.remote_ip.as_deref() {
            form.push(("remoteip", ip));
        }

        let response = match agent.post(&self.url).send_form(&form) {
            Ok(response) => response,
            Err(ureq::Error::Status(code, _)) => {
                return Err(format!("siteverify returned HTTP {code}"))
            }
            Err(ureq::Error::Transport(err)) => {
                return Err(format!("siteverify request failed: {err}"))
            }
        };

        // `ureq` only reports `>= 400` as an error, so a redirect that could not
        // be followed would otherwise slip through as a success.
        let status = response.status();
        if !(200..300).contains(&status) {
            return Err(format!("siteverify returned HTTP {status}"));
        }

        let body = response
            .into_string()
            .map_err(|err| format!("cannot read the siteverify response: {err}"))?;
        let parsed: serde_json::Value =
            serde_json::from_str(&body).map_err(|err| format!("invalid siteverify JSON: {err}"))?;
        Ok(parsed
            .get("success")
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(false))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};
    use std::net::TcpListener;

    #[test]
    fn parses_known_providers_only() {
        assert_eq!(
            CaptchaProvider::parse("turnstile"),
            CaptchaProvider::Turnstile
        );
        assert_eq!(
            CaptchaProvider::parse(" reCAPTCHA "),
            CaptchaProvider::Recaptcha
        );
        assert_eq!(CaptchaProvider::parse("none"), CaptchaProvider::None);
        // Anything unknown disables the feature instead of failing open.
        assert_eq!(CaptchaProvider::parse("hcaptcha"), CaptchaProvider::None);
        assert_eq!(CaptchaProvider::parse(""), CaptchaProvider::None);
        assert!(CaptchaProvider::is_known("recaptcha"));
        assert!(!CaptchaProvider::is_known("hcaptcha"));
    }

    #[test]
    fn provider_metadata_is_stable() {
        assert_eq!(
            CaptchaProvider::Turnstile.form_field(),
            "cf-turnstile-response"
        );
        assert_eq!(
            CaptchaProvider::Recaptcha.form_field(),
            "g-recaptcha-response"
        );
        assert_eq!(CaptchaProvider::Turnstile.widget_class(), "cf-turnstile");
        assert_eq!(CaptchaProvider::Recaptcha.widget_class(), "g-recaptcha");
        assert!(CaptchaProvider::Turnstile
            .verify_url()
            .starts_with("https://challenges.cloudflare.com/"));
        assert!(CaptchaProvider::Recaptcha
            .verify_url()
            .starts_with("https://www.google.com/"));
    }

    #[test]
    fn widget_carries_the_language_and_the_site_key() {
        let widget = CaptchaWidget::new(CaptchaProvider::Turnstile, "site-key", Lang::ZhHant)
            .expect("widget");
        assert_eq!(
            widget.script_url,
            "https://challenges.cloudflare.com/turnstile/v0/api.js?hl=zh-TW"
        );
        assert_eq!(widget.site_key, "site-key");
        assert_eq!(widget.field_name, "cf-turnstile-response");

        let widget =
            CaptchaWidget::new(CaptchaProvider::Recaptcha, "k", Lang::ZhHans).expect("widget");
        assert_eq!(
            widget.script_url,
            "https://www.google.com/recaptcha/api.js?hl=zh-CN"
        );
        assert_eq!(widget.field_name, "g-recaptcha-response");

        assert!(CaptchaWidget::new(CaptchaProvider::None, "k", Lang::En).is_none());
        assert!(CaptchaWidget::new(CaptchaProvider::Turnstile, "  ", Lang::En).is_none());
    }

    #[test]
    fn a_guard_needs_a_provider_and_both_keys() {
        let complete = CaptchaGuard::new(
            CaptchaProvider::Turnstile,
            "site",
            "secret",
            TURNSTILE_VERIFY_URL,
        );
        assert!(complete.enabled());

        for incomplete in [
            CaptchaGuard::new(
                CaptchaProvider::None,
                "site",
                "secret",
                TURNSTILE_VERIFY_URL,
            ),
            CaptchaGuard::new(
                CaptchaProvider::Turnstile,
                "",
                "secret",
                TURNSTILE_VERIFY_URL,
            ),
            CaptchaGuard::new(CaptchaProvider::Turnstile, "site", "", TURNSTILE_VERIFY_URL),
            CaptchaGuard::new(CaptchaProvider::Turnstile, "site", "secret", ""),
        ] {
            assert!(!incomplete.enabled());
        }
        assert!(!CaptchaGuard::disabled().enabled());
        assert!(CaptchaGuard::disabled().widget(Lang::En).is_none());
    }

    #[tokio::test]
    async fn a_disabled_guard_accepts_anything() {
        let guard = CaptchaGuard::disabled();
        assert!(guard.verify(None, None).await.is_ok());
        assert!(guard.verify(Some("garbage"), None).await.is_ok());
    }

    #[tokio::test]
    async fn a_missing_token_is_rejected_without_calling_the_provider() {
        let guard = CaptchaGuard::new(
            CaptchaProvider::Turnstile,
            "site",
            "secret",
            "http://127.0.0.1:1/siteverify",
        );
        assert_eq!(guard.verify(None, None).await, Err(CaptchaError::Missing));
        assert_eq!(
            guard.verify(Some("   "), None).await,
            Err(CaptchaError::Missing)
        );
    }

    /// Minimal raw HTTP server: no async runtime, no extra dependency.
    fn raw_server(status: &str, body: &'static str) -> (String, std::thread::JoinHandle<String>) {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
        let addr = listener.local_addr().expect("addr");
        let status = status.to_string();
        let handle = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().expect("accept");
            let request = read_request(&mut stream);
            let response = format!(
                "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
            let _ = stream.write_all(response.as_bytes());
            let _ = stream.flush();
            request
        });
        (format!("http://{addr}/siteverify"), handle)
    }

    /// Read one request: headers, then `Content-Length` bytes of body.
    fn read_request(stream: &mut std::net::TcpStream) -> String {
        let mut raw = Vec::new();
        let mut chunk = [0u8; 1024];
        loop {
            match stream.read(&mut chunk) {
                Ok(0) => break,
                Ok(read) => raw.extend_from_slice(&chunk[..read]),
                Err(_) => break,
            }
            let text = String::from_utf8_lossy(&raw).to_string();
            if let Some((head, body)) = text.split_once("\r\n\r\n") {
                let length = head
                    .lines()
                    .find_map(|line| {
                        let (name, value) = line.split_once(':')?;
                        name.eq_ignore_ascii_case("content-length")
                            .then(|| value.trim().parse::<usize>().ok())?
                    })
                    .unwrap_or(0);
                if body.len() >= length {
                    break;
                }
            }
        }
        String::from_utf8_lossy(&raw).to_string()
    }

    fn request(url: String) -> SiteverifyRequest {
        SiteverifyRequest {
            url,
            secret: "the-secret".into(),
            response: "the-token".into(),
            remote_ip: Some("203.0.113.9".into()),
        }
    }

    #[test]
    fn posts_the_token_and_reads_success() {
        let (url, server) = raw_server("200 OK", "{\"success\":true}");
        assert!(request(url).send().expect("verified"));

        let raw = server.join().expect("server thread");
        assert!(raw.contains("POST /siteverify"));
        assert!(raw.contains("the-secret"));
        assert!(raw.contains("the-token"));
        assert!(raw.contains("remoteip=203.0.113.9"));
    }

    #[test]
    fn omits_remoteip_when_the_client_is_unknown() {
        let (url, server) = raw_server("200 OK", "{\"success\":true}");
        let mut request = request(url);
        request.remote_ip = None;
        assert!(request.send().expect("verified"));
        assert!(!server.join().expect("server thread").contains("remoteip"));
    }

    #[test]
    fn a_false_success_is_not_verified() {
        let (url, server) = raw_server("200 OK", "{\"success\":false}");
        assert!(!request(url).send().expect("answered"));
        server.join().expect("server thread");
    }

    #[test]
    fn a_missing_success_field_is_not_verified() {
        let (url, server) = raw_server("200 OK", "{\"error-codes\":[\"nope\"]}");
        assert!(!request(url).send().expect("answered"));
        server.join().expect("server thread");
    }

    #[test]
    fn non_2xx_and_garbage_bodies_fail() {
        let (url, server) = raw_server("500 Internal Server Error", "{}");
        assert!(request(url).send().is_err());
        server.join().expect("server thread");

        let (url, server) = raw_server("200 OK", "not json at all");
        assert!(request(url).send().is_err());
        server.join().expect("server thread");
    }

    #[test]
    fn an_unreachable_provider_fails() {
        // Port 1 is not listening; the connection is refused immediately.
        let request = SiteverifyRequest {
            url: "http://127.0.0.1:1/siteverify".into(),
            secret: "s".into(),
            response: "t".into(),
            remote_ip: None,
        };
        assert!(request.send().is_err());
    }
}
