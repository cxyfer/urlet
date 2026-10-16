//! Bot protection: Cloudflare Turnstile and Google reCAPTCHA.
//!
//! The service is deliberately self contained so any future flow (for example an
//! anonymous "create a short link" form) can be protected by building a
//! [`CaptchaGuard`] for it and calling [`CaptchaGuard::verify`].
//!
//! Failure policy is *fail closed*: a missing token, a provider that says
//! `success != true`, a `hostname` that belongs to somebody else, a non 2xx
//! response, a connection error, a timeout and an unparsable body all reject the
//! request. A disabled guard is a no-op.
//!
//! The provider call is blocking and therefore runs on
//! [`tokio::task::spawn_blocking`], bounded by [`MAX_CONCURRENT_VERIFICATIONS`];
//! it uses `ureq` with the pure Rust `rustls` backend and the `webpki-roots`
//! trust store so the musl container needs no OpenSSL.
//!
//! Known limitation: [`VERIFY_TIMEOUT`] is the deadline of the HTTP call, and
//! `ureq`'s own documentation notes that DNS resolution cannot be interrupted
//! with the APIs it uses, so a stalled resolver can exceed it. The concurrency
//! limit keeps such a call from blocking the rest of the process.
//!
//! # Which hostnames are accepted
//!
//! A site key is public, so when a provider relaxes its domain restriction
//! (Cloudflare "Any Hostname", a wildcard, reCAPTCHA with domain validation off)
//! anybody could mint tokens with our key on their own domain. Every answer that
//! carries a `hostname` is therefore checked against an explicit allowlist:
//! `Config::base_host` plus whatever the administrator listed in
//! `captcha_allowed_hostnames`. The `Host` header of the request is deliberately
//! *not* a candidate — a client controls it, which is exactly the value an
//! attacker would forge to match the token it just obtained.
//!
//! Upgrade note: an instance that also serves a second hostname (a short domain
//! next to the application domain, or a staging alias) must add that hostname to
//! the allowlist, or verifications performed through it will be rejected. The
//! warning logged on a mismatch lists the hostname to add and the candidates
//! currently accepted.

use std::net::IpAddr;
use std::sync::OnceLock;
use std::time::Duration;

use crate::i18n::Lang;
use crate::services::secret::REDACTED;
use crate::services::settings::Settings;

/// `siteverify` endpoint of Cloudflare Turnstile.
pub const TURNSTILE_VERIFY_URL: &str = "https://challenges.cloudflare.com/turnstile/v0/siteverify";
/// `siteverify` endpoint of Google reCAPTCHA.
pub const RECAPTCHA_VERIFY_URL: &str = "https://www.google.com/recaptcha/api/siteverify";

/// Longest accepted site/secret key.
pub const MAX_KEY_LEN: usize = 200;

/// Most hostnames an administrator may add to the allowlist.
pub const MAX_ALLOWED_HOSTNAMES: usize = 10;
/// Longest accepted hostname (the DNS limit).
pub const MAX_HOSTNAME_LEN: usize = 253;

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
#[derive(Clone)]
pub struct CaptchaGuard {
    provider: CaptchaProvider,
    site_key: String,
    secret_key: String,
    verify_url: String,
    /// Hostname the instance is served under, from `Config::base_host`.
    base_host: String,
    /// Extra hostnames the provider may report, from the settings table.
    allowed_hostnames: Vec<String>,
    /// Whether the endpoint may be plain HTTP. Only ever true for the test
    /// override: `Config::captcha_verify_url` has no environment variable, so a
    /// deployment always talks HTTPS to the provider.
    allow_plaintext: bool,
}

/// Never print the secret, even through a stray `tracing::debug!(?guard)`.
impl std::fmt::Debug for CaptchaGuard {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CaptchaGuard")
            .field("provider", &self.provider)
            .field("site_key", &self.site_key)
            .field("secret_key", &REDACTED)
            .field("verify_url", &self.verify_url)
            .field("base_host", &self.base_host)
            .field("allowed_hostnames", &self.allowed_hostnames)
            .field("allow_plaintext", &self.allow_plaintext)
            .finish()
    }
}

impl CaptchaGuard {
    /// Guard for `flow`, honouring the four enable conditions: a provider other
    /// than `none`, a site key, a secret key and the per-flow switch.
    ///
    /// `verify_url_override` replaces the provider endpoint; production passes
    /// `None` and tests point it at a local mock. `base_host` is the hostname
    /// the instance is served under, used to check the `hostname` a provider
    /// reports back.
    pub fn from_settings(
        settings: &Settings,
        flow: CaptchaFlow,
        verify_url_override: Option<&str>,
        base_host: &str,
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

        let (verify_url, allow_plaintext) = match verify_url_override {
            Some(url) => (url.to_string(), true),
            None => (provider.verify_url().to_string(), false),
        };
        Self {
            provider,
            site_key,
            secret_key,
            verify_url,
            base_host: base_host.to_string(),
            allowed_hostnames: settings.captcha_allowed_hostname_list(),
            allow_plaintext,
        }
    }

    /// Build a guard from explicit parts (tests, and future callers that have
    /// already resolved the keys).
    pub fn new(
        provider: CaptchaProvider,
        site_key: &str,
        secret_key: &str,
        verify_url: &str,
        base_host: &str,
        allowed_hostnames: &[&str],
    ) -> Self {
        Self {
            provider,
            site_key: site_key.to_string(),
            secret_key: secret_key.to_string(),
            verify_url: verify_url.to_string(),
            base_host: base_host.to_string(),
            allowed_hostnames: allowed_hostnames
                .iter()
                .map(|host| (*host).to_string())
                .collect(),
            // `new` is the test/advanced entry point: it may point anywhere.
            allow_plaintext: true,
        }
    }

    /// The inert guard: every check passes, nothing is rendered.
    pub fn disabled() -> Self {
        Self {
            provider: CaptchaProvider::None,
            site_key: String::new(),
            secret_key: String::new(),
            verify_url: String::new(),
            base_host: String::new(),
            allowed_hostnames: Vec::new(),
            allow_plaintext: true,
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

    /// Token of the configured provider, falling back to the other field so a
    /// stale value in the form cannot cause a needless rejection.
    pub fn submitted_token<'a>(
        &self,
        turnstile: Option<&'a str>,
        recaptcha: Option<&'a str>,
    ) -> Option<&'a str> {
        match self.provider {
            CaptchaProvider::Recaptcha => recaptcha.or(turnstile),
            _ => turnstile.or(recaptcha),
        }
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
    ///
    /// The `hostname` of the answer is checked against [`accepted_hostnames`],
    /// never against the request `Host`: a client controls that header, so
    /// trusting it would defeat the check. See the module documentation for the
    /// upgrade note about instances served under more than one name.
    ///
    /// [`accepted_hostnames`]: CaptchaGuard::accepted_hostnames
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
            allow_plaintext: self.allow_plaintext,
        };
        let provider = self.provider.as_str();

        // Each verification occupies a blocking thread, so the number in flight
        // is bounded. A saturated queue waits briefly rather than being rejected
        // at once — eight slow provider calls must not lock every legitimate
        // visitor out — but the wait is bounded: past the deadline the request
        // fails closed instead of piling up without limit.
        let Ok(Ok(_permit)) =
            tokio::time::timeout(VERIFY_QUEUE_TIMEOUT, VERIFY_PERMITS.acquire()).await
        else {
            tracing::warn!(provider, "captcha verification is saturated");
            return Err(CaptchaError::Unavailable);
        };

        // `ureq` is blocking: keep it off the async runtime.
        match tokio::task::spawn_blocking(move || request.send()).await {
            Ok(Ok(outcome)) if outcome.success => {
                if self.hostname_allowed(outcome.hostname.as_deref()) {
                    Ok(())
                } else {
                    Err(CaptchaError::Rejected)
                }
            }
            Ok(Ok(_)) => {
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

    /// Every hostname a provider may report: `base_host` plus the allowlist,
    /// normalised, sorted and de-duplicated.
    fn accepted_hostnames(&self) -> Vec<String> {
        let mut hosts: Vec<String> = std::iter::once(normalize_host(&self.base_host))
            .chain(
                self.allowed_hostnames
                    .iter()
                    .map(|host| normalize_host(host)),
            )
            .filter(|host| !host.is_empty())
            .collect();
        hosts.sort();
        hosts.dedup();
        hosts
    }

    /// Check the `hostname` a provider reports against the configured set.
    ///
    /// A missing or empty field passes: not every provider always returns it.
    /// A present value must equal one of [`accepted_hostnames`]. When that set is
    /// empty — only possible if `base_host` were empty, which cannot happen for a
    /// validated base URL — the check is skipped with a warning rather than
    /// locking every visitor out.
    ///
    /// [`accepted_hostnames`]: CaptchaGuard::accepted_hostnames
    fn hostname_allowed(&self, hostname: Option<&str>) -> bool {
        let provider = self.provider.as_str();
        let Some(reported) = hostname.map(normalize_host).filter(|host| !host.is_empty()) else {
            return true;
        };

        let accepted = self.accepted_hostnames();
        if accepted.is_empty() {
            tracing::warn!(
                provider,
                hostname = %reported,
                "cannot check the reported hostname: no hostname is configured"
            );
            return true;
        }
        if accepted.iter().any(|host| host == &reported) {
            return true;
        }
        tracing::warn!(
            provider,
            hostname = %reported,
            accepted = ?accepted,
            "captcha token was issued for a hostname that is not configured; add it to captcha_allowed_hostnames to accept it"
        );
        false
    }
}

/// Split a stored allowlist into normalised hostnames.
///
/// Comma separated, trimmed, lower case, empty entries dropped. No validation:
/// see [`validate_allowed_hostnames`] for the rules the settings form enforces.
pub fn parse_allowed_hostnames(value: &str) -> Vec<String> {
    value
        .split(',')
        .map(|host| host.trim().to_ascii_lowercase())
        .filter(|host| !host.is_empty())
        .collect()
}

/// Validate and canonicalise the `captcha_allowed_hostnames` setting.
///
/// Returns the normalised, comma separated list, or an English message suitable
/// for [`AppError::validation`](crate::error::AppError::validation).
pub fn validate_allowed_hostnames(value: &str) -> Result<String, String> {
    let hostnames = parse_allowed_hostnames(value);
    if hostnames.len() > MAX_ALLOWED_HOSTNAMES {
        return Err(format!(
            "at most {MAX_ALLOWED_HOSTNAMES} hostnames may be listed"
        ));
    }
    for hostname in &hostnames {
        if hostname.chars().count() > MAX_HOSTNAME_LEN {
            return Err(format!(
                "each hostname must be at most {MAX_HOSTNAME_LEN} characters"
            ));
        }
        if hostname
            .chars()
            .any(|c| c.is_whitespace() || matches!(c, '/' | ':' | '@'))
        {
            return Err("hostnames must be bare names without spaces, '/', ':' or '@'".to_string());
        }
    }
    Ok(hostnames.join(","))
}

/// Normalise a hostname for comparison: trimmed, lower case, no port and no
/// trailing dot. IPv6 literals keep their address without the brackets.
pub fn normalize_host(value: &str) -> String {
    let value = value.trim().to_ascii_lowercase();
    // `Host: [::1]:8080`
    let value = match value.strip_prefix('[') {
        Some(rest) => match rest.split_once(']') {
            Some((address, _)) => address.to_string(),
            None => value,
        },
        None => value,
    };
    // `Host: s.example.com:8443` — a bare IPv6 address keeps its colons.
    let value = match value.rsplit_once(':') {
        Some((host, port))
            if !host.contains(':')
                && !port.is_empty()
                && port.chars().all(|c| c.is_ascii_digit()) =>
        {
            host.to_string()
        }
        _ => value,
    };
    value.trim_end_matches('.').to_string()
}

/// What a `siteverify` answer told us.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct SiteverifyOutcome {
    /// Value of the `success` field. Anything but a JSON `true` is a rejection,
    /// including the string `"true"` some mocks like to send.
    success: bool,
    /// Value of the `hostname` field, when the provider reported one.
    hostname: Option<String>,
}

impl SiteverifyOutcome {
    fn parse(body: &str) -> Result<Self, String> {
        let parsed: serde_json::Value =
            serde_json::from_str(body).map_err(|err| format!("invalid siteverify JSON: {err}"))?;
        Ok(Self {
            success: parsed
                .get("success")
                .and_then(serde_json::Value::as_bool)
                .unwrap_or(false),
            hostname: parsed
                .get("hostname")
                .and_then(serde_json::Value::as_str)
                .map(str::to_string),
        })
    }
}

/// One blocking `siteverify` call.
struct SiteverifyRequest {
    url: String,
    secret: String,
    response: String,
    remote_ip: Option<String>,
    /// Test endpoints may be plain HTTP; see [`CaptchaGuard::allow_plaintext`].
    allow_plaintext: bool,
}

impl SiteverifyRequest {
    /// `Ok(outcome)` when the provider answered, `Err(reason)` when the answer
    /// is unusable.
    fn send(&self) -> Result<SiteverifyOutcome, String> {
        // One shared agent keeps the connection pool (and therefore TLS
        // handshakes) warm; see `agent`.
        let agent = if self.allow_plaintext {
            plaintext_agent()
        } else {
            agent()
        };

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

        // `redirects(0)` on the agent turns a 3xx into `Error::Status`, but a
        // 1xx/3xx without a body could still arrive here, so check the code.
        let status = response.status();
        if !(200..300).contains(&status) {
            return Err(format!("siteverify returned HTTP {status}"));
        }

        let body = response
            .into_string()
            .map_err(|err| format!("cannot read the siteverify response: {err}"))?;
        SiteverifyOutcome::parse(&body)
    }
}

/// Shared agent for the provider endpoints.
///
/// Reusing one agent keeps the connection pool — and therefore the TLS
/// handshake — warm between verifications.
fn agent() -> &'static ureq::Agent {
    static AGENT: OnceLock<ureq::Agent> = OnceLock::new();
    AGENT.get_or_init(|| build_agent(false))
}

/// Test only: an agent that also accepts plain HTTP, so the suite can point the
/// guard at a local mock. `Config::captcha_verify_url` is never read from the
/// environment, so production always uses [`agent`].
fn plaintext_agent() -> &'static ureq::Agent {
    static AGENT: OnceLock<ureq::Agent> = OnceLock::new();
    AGENT.get_or_init(|| build_agent(true))
}

fn build_agent(allow_plaintext: bool) -> ureq::Agent {
    ureq::builder()
        // Overall deadline; note that DNS resolution is not interruptible, see
        // the module documentation.
        .timeout(VERIFY_TIMEOUT)
        .timeout_connect(VERIFY_TIMEOUT)
        // Following a redirect would hand the secret to another host.
        .redirects(0)
        .https_only(!allow_plaintext)
        .build()
}

/// Maximum number of `siteverify` calls in flight at once.
///
/// Each one occupies a blocking thread, so the ceiling keeps a burst of
/// submissions from exhausting the blocking pool. The per-flow rate limiters
/// bound how quickly the queue can fill.
const MAX_CONCURRENT_VERIFICATIONS: usize = 8;

/// How long a verification waits for a free slot before failing closed.
const VERIFY_QUEUE_TIMEOUT: Duration = Duration::from_secs(2);

static VERIFY_PERMITS: tokio::sync::Semaphore =
    tokio::sync::Semaphore::const_new(MAX_CONCURRENT_VERIFICATIONS);

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
            "s.example.com",
            &["other.example.com"],
        );
        assert!(complete.enabled());

        for incomplete in [
            CaptchaGuard::new(
                CaptchaProvider::None,
                "site",
                "secret",
                TURNSTILE_VERIFY_URL,
                "s.example.com",
                &[],
            ),
            CaptchaGuard::new(
                CaptchaProvider::Turnstile,
                "",
                "secret",
                TURNSTILE_VERIFY_URL,
                "s.example.com",
                &[],
            ),
            CaptchaGuard::new(
                CaptchaProvider::Turnstile,
                "site",
                "",
                TURNSTILE_VERIFY_URL,
                "s.example.com",
                &[],
            ),
            CaptchaGuard::new(
                CaptchaProvider::Turnstile,
                "site",
                "secret",
                "",
                "s.example.com",
                &[],
            ),
        ] {
            assert!(!incomplete.enabled());
        }
        assert!(!CaptchaGuard::disabled().enabled());
        assert!(CaptchaGuard::disabled().widget(Lang::En).is_none());
    }

    #[test]
    fn the_debug_output_never_contains_the_secret() {
        let guard = CaptchaGuard::new(
            CaptchaProvider::Turnstile,
            "public-site-key",
            "super-secret-value",
            TURNSTILE_VERIFY_URL,
            "s.example.com",
            &[],
        );
        let printed = format!("{guard:?}");
        assert!(!printed.contains("super-secret-value"), "{printed}");
        assert!(printed.contains(REDACTED), "{printed}");
        assert!(printed.contains("public-site-key"), "{printed}");
    }

    #[test]
    fn the_token_field_of_the_configured_provider_wins() {
        let turnstile = CaptchaGuard::new(
            CaptchaProvider::Turnstile,
            "site",
            "secret",
            TURNSTILE_VERIFY_URL,
            "s.example.com",
            &[],
        );
        assert_eq!(
            turnstile.submitted_token(Some("cf"), Some("g")),
            Some("cf"),
            "the configured provider is tried first"
        );
        assert_eq!(turnstile.submitted_token(None, Some("g")), Some("g"));
        assert_eq!(turnstile.submitted_token(Some("cf"), None), Some("cf"));
        assert_eq!(turnstile.submitted_token(None, None), None);

        let recaptcha = CaptchaGuard::new(
            CaptchaProvider::Recaptcha,
            "site",
            "secret",
            RECAPTCHA_VERIFY_URL,
            "s.example.com",
            &[],
        );
        assert_eq!(recaptcha.submitted_token(Some("cf"), Some("g")), Some("g"));
        assert_eq!(recaptcha.submitted_token(Some("cf"), None), Some("cf"));
        assert_eq!(recaptcha.submitted_token(None, None), None);
    }

    #[test]
    fn normalises_hostnames() {
        assert_eq!(normalize_host(" S.Example.COM. "), "s.example.com");
        assert_eq!(normalize_host("s.example.com:8443"), "s.example.com");
        assert_eq!(normalize_host("[::1]:8080"), "::1");
        assert_eq!(normalize_host("::1"), "::1");
        assert_eq!(normalize_host("localhost:8080"), "localhost");
        assert_eq!(normalize_host(""), "");
    }

    #[test]
    fn checks_the_hostname_reported_by_the_provider() {
        let guard = CaptchaGuard::new(
            CaptchaProvider::Turnstile,
            "site",
            "secret",
            TURNSTILE_VERIFY_URL,
            "app.example.com",
            &["s.example.com", "S.EXAMPLE.COM."],
        );

        // A provider that does not report one is not rejected.
        assert!(guard.hostname_allowed(None));
        assert!(guard.hostname_allowed(Some("")));
        assert!(guard.hostname_allowed(Some("   ")));

        // `base_host` and every allowlisted name are accepted, normalised on
        // both sides (case, port, trailing dot).
        assert!(guard.hostname_allowed(Some("app.example.com")));
        assert!(guard.hostname_allowed(Some("APP.Example.com.")));
        assert!(guard.hostname_allowed(Some("app.example.com:8443")));
        assert!(guard.hostname_allowed(Some("s.example.com")));

        // Anything else is refused, including a lookalike suffix.
        assert!(!guard.hostname_allowed(Some("evil.example")));
        assert!(!guard.hostname_allowed(Some("s.example.com.evil.example")));
        assert!(!guard.hostname_allowed(Some("other.example.com")));

        // The candidate set is `base_host` plus the allowlist, de-duplicated.
        assert_eq!(
            guard.accepted_hostnames(),
            vec!["app.example.com".to_string(), "s.example.com".to_string()]
        );

        // With no candidate at all the check is skipped rather than locking
        // every visitor out.
        let homeless = CaptchaGuard::new(
            CaptchaProvider::Turnstile,
            "site",
            "secret",
            TURNSTILE_VERIFY_URL,
            "",
            &[],
        );
        assert!(homeless.hostname_allowed(Some("evil.example")));
    }

    #[test]
    fn validates_the_hostname_allowlist() {
        // Empty, or a list of separators, means "no extra hostnames".
        assert_eq!(validate_allowed_hostnames("").expect("ok"), "");
        assert_eq!(validate_allowed_hostnames(" , ,").expect("ok"), "");
        assert_eq!(
            validate_allowed_hostnames(" S.Example.COM , app.example.com ").expect("ok"),
            "s.example.com,app.example.com"
        );
        assert_eq!(
            parse_allowed_hostnames("a.example.com,,B.EXAMPLE.COM"),
            vec!["a.example.com", "b.example.com"]
        );

        // At most ten entries.
        let ten = (0..MAX_ALLOWED_HOSTNAMES)
            .map(|index| format!("h{index}.example.com"))
            .collect::<Vec<_>>()
            .join(",");
        assert_eq!(
            parse_allowed_hostnames(&validate_allowed_hostnames(&ten).expect("ok")).len(),
            MAX_ALLOWED_HOSTNAMES
        );
        let eleven = format!("{ten},extra.example.com");
        assert!(validate_allowed_hostnames(&eleven).is_err());

        // Each entry must be a bare hostname.
        for bad in [
            "https://s.example.com",
            "s.example.com/path",
            "s.example.com:8443",
            "user@s.example.com",
            "s example.com",
            "s\texample.com",
        ] {
            assert!(
                validate_allowed_hostnames(bad).is_err(),
                "{bad:?} must be refused"
            );
        }

        // Length is bounded by the DNS limit.
        assert!(validate_allowed_hostnames(&"x".repeat(MAX_HOSTNAME_LEN)).is_ok());
        assert!(validate_allowed_hostnames(&"x".repeat(MAX_HOSTNAME_LEN + 1)).is_err());
    }

    /// The queue is bounded, but it does wait: a slot is preferred over an
    /// instant rejection, and the deadline keeps the wait finite.
    ///
    /// Holds every permit for the duration, so no other test in this binary may
    /// call `verify` with a token while it runs (none does today).
    #[tokio::test]
    async fn a_saturated_queue_waits_briefly_then_fails_closed() {
        let held = VERIFY_PERMITS
            .acquire_many(MAX_CONCURRENT_VERIFICATIONS as u32)
            .await
            .expect("every permit is available");
        let guard = CaptchaGuard::new(
            CaptchaProvider::Turnstile,
            "site",
            "secret",
            "http://127.0.0.1:1/siteverify",
            "s.example.com",
            &[],
        );

        let started = std::time::Instant::now();
        assert_eq!(
            guard.verify(Some("token"), None).await,
            Err(CaptchaError::Unavailable)
        );
        let waited = started.elapsed();
        assert!(
            waited >= VERIFY_QUEUE_TIMEOUT,
            "a free slot is waited for, not skipped: {waited:?}"
        );
        assert!(
            waited < VERIFY_QUEUE_TIMEOUT * 4,
            "the wait is bounded: {waited:?}"
        );
        drop(held);
    }

    #[tokio::test]
    async fn a_missing_token_is_rejected_without_calling_the_provider() {
        let guard = CaptchaGuard::new(
            CaptchaProvider::Turnstile,
            "site",
            "secret",
            "http://127.0.0.1:1/siteverify",
            "s.example.com",
            &[],
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
            // The mock speaks plain HTTP on loopback.
            allow_plaintext: true,
        }
    }

    #[test]
    fn posts_the_token_and_reads_success() {
        let (url, server) = raw_server("200 OK", "{\"success\":true}");
        let outcome = request(url).send().expect("verified");
        assert!(outcome.success);
        assert_eq!(outcome.hostname, None);

        let raw = server.join().expect("server thread");
        assert!(raw.contains("POST /siteverify"));
        assert!(raw.contains("the-secret"));
        assert!(raw.contains("the-token"));
        assert!(raw.contains("remoteip=203.0.113.9"));
    }

    #[test]
    fn reads_the_reported_hostname() {
        let (url, server) = raw_server(
            "200 OK",
            "{\"success\":true,\"hostname\":\"s.example.com\"}",
        );
        let outcome = request(url).send().expect("verified");
        assert!(outcome.success);
        assert_eq!(outcome.hostname.as_deref(), Some("s.example.com"));
        server.join().expect("server thread");
    }

    #[test]
    fn omits_remoteip_when_the_client_is_unknown() {
        let (url, server) = raw_server("200 OK", "{\"success\":true}");
        let mut request = request(url);
        request.remote_ip = None;
        assert!(request.send().expect("verified").success);
        assert!(!server.join().expect("server thread").contains("remoteip"));
    }

    #[test]
    fn a_false_success_is_not_verified() {
        let (url, server) = raw_server("200 OK", "{\"success\":false}");
        assert!(!request(url).send().expect("answered").success);
        server.join().expect("server thread");
    }

    #[test]
    fn a_missing_success_field_is_not_verified() {
        let (url, server) = raw_server("200 OK", "{\"error-codes\":[\"nope\"]}");
        assert!(!request(url).send().expect("answered").success);
        server.join().expect("server thread");
    }

    #[test]
    fn a_non_boolean_success_is_not_verified() {
        // `"true"` is a string, not a boolean: only a JSON `true` counts.
        let (url, server) = raw_server("200 OK", "{\"success\":\"true\"}");
        assert!(!request(url).send().expect("answered").success);
        server.join().expect("server thread");

        let (url, server) = raw_server("200 OK", "{\"success\":1}");
        assert!(!request(url).send().expect("answered").success);
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
    fn a_redirect_is_not_followed() {
        // A redirect could send the secret to another host, so `redirects(0)`
        // turns it into a hard failure.
        let (url, server) = raw_server("302 Found", "");
        assert!(request(url).send().is_err());
        server.join().expect("server thread");
    }

    #[test]
    fn an_empty_body_fails() {
        let (url, server) = raw_server("204 No Content", "");
        assert!(request(url).send().is_err());
        server.join().expect("server thread");

        let (url, server) = raw_server("200 OK", "");
        assert!(request(url).send().is_err());
        server.join().expect("server thread");
    }

    #[test]
    fn the_production_agent_refuses_plain_http() {
        // `https_only` rejects the scheme before a connection is made, so the
        // accept thread is deliberately left detached instead of joined.
        let (url, _server) = raw_server("200 OK", "{\"success\":true}");
        let mut request = request(url);
        request.allow_plaintext = false;
        let error = request.send().expect_err("plain http must be refused");
        assert!(error.contains("siteverify request failed"), "{error}");
    }

    #[test]
    fn an_unreachable_provider_fails() {
        // Port 1 is not listening; the connection is refused immediately.
        let request = SiteverifyRequest {
            url: "http://127.0.0.1:1/siteverify".into(),
            secret: "s".into(),
            response: "t".into(),
            remote_ip: None,
            allow_plaintext: true,
        };
        assert!(request.send().is_err());
    }
}
