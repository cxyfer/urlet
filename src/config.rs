//! Process configuration, sourced entirely from environment variables.

use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use thiserror::Error;

use crate::services::secret;

#[derive(Debug, Error)]
pub enum ConfigError {
    #[error("invalid {name}: {value:?}")]
    Invalid { name: &'static str, value: String },
    #[error("io error on {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("URLET_SECRET must be at least 16 characters when set explicitly")]
    WeakSecret,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LogFormat {
    Compact,
    Json,
}

impl LogFormat {
    fn parse(value: &str) -> Self {
        match value.trim().to_ascii_lowercase().as_str() {
            "json" => LogFormat::Json,
            _ => LogFormat::Compact,
        }
    }
}

#[derive(Clone)]
pub struct Config {
    pub bind: SocketAddr,
    /// Public base URL without a trailing slash, e.g. `https://s.example.com`.
    pub base_url: String,
    pub base_scheme: String,
    pub base_host: String,
    /// Extra hostnames seeded from `URLET_DOMAINS`. Insert-only; the database
    /// stays authoritative for lifecycle state after the first boot.
    pub extra_domains: Vec<String>,
    /// Whether `URLET_DOMAINS` was present at all. An unset variable must not
    /// detach existing environment-managed domains during bootstrap.
    pub extra_domains_declared: bool,
    pub secure_cookies: bool,
    pub database_url: String,
    pub data_dir: PathBuf,
    pub secret: Arc<Vec<u8>>,
    pub session_ttl_hours: i64,
    pub unlock_ttl_minutes: i64,
    pub trust_proxy: bool,
    pub registration_override: Option<String>,
    pub slug_length_override: Option<usize>,
    /// Bot protection overrides. A non empty variable wins over the settings
    /// table, exactly like `registration_override`.
    pub captcha_provider_override: Option<String>,
    pub captcha_site_key_override: Option<String>,
    pub captcha_secret_key_override: Option<String>,
    /// Extra hostnames a captcha provider may report, comma separated.
    pub captcha_allowed_hostnames_override: Option<String>,
    pub captcha_on_register_override: Option<bool>,
    pub captcha_on_login_override: Option<bool>,
    pub captcha_on_create_override: Option<bool>,
    /// Test only: replace the provider `siteverify` endpoint. There is
    /// deliberately no environment variable for it, so a deployment can never be
    /// pointed at a host that would learn the secret key.
    pub captcha_verify_url: Option<String>,
    pub admin_email: Option<String>,
    pub admin_password: Option<String>,
    pub log_filter: String,
    pub log_format: LogFormat,
    /// Fallback slug length when the settings table is unavailable.
    pub default_slug_length: usize,
    /// Fallback per-user URL quota when the settings table is unavailable.
    pub max_urls_per_user: i64,
}

/// The HMAC secret, the bootstrap password and the captcha secret are printed as
/// [`secret::REDACTED`]: a stray `tracing::debug!(?config)` must never leak them.
impl std::fmt::Debug for Config {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let hidden = |present: bool| present.then_some(secret::REDACTED);
        f.debug_struct("Config")
            .field("bind", &self.bind)
            .field("base_url", &self.base_url)
            .field("base_scheme", &self.base_scheme)
            .field("base_host", &self.base_host)
            .field("extra_domains", &self.extra_domains)
            .field("extra_domains_declared", &self.extra_domains_declared)
            .field("secure_cookies", &self.secure_cookies)
            .field("database_url", &self.database_url)
            .field("data_dir", &self.data_dir)
            .field("secret", &secret::REDACTED)
            .field("session_ttl_hours", &self.session_ttl_hours)
            .field("unlock_ttl_minutes", &self.unlock_ttl_minutes)
            .field("trust_proxy", &self.trust_proxy)
            .field("registration_override", &self.registration_override)
            .field("slug_length_override", &self.slug_length_override)
            .field("captcha_provider_override", &self.captcha_provider_override)
            .field("captcha_site_key_override", &self.captcha_site_key_override)
            .field(
                "captcha_secret_key_override",
                &hidden(self.captcha_secret_key_override.is_some()),
            )
            .field(
                "captcha_allowed_hostnames_override",
                &self.captcha_allowed_hostnames_override,
            )
            .field(
                "captcha_on_register_override",
                &self.captcha_on_register_override,
            )
            .field("captcha_on_login_override", &self.captcha_on_login_override)
            .field(
                "captcha_on_create_override",
                &self.captcha_on_create_override,
            )
            .field("captcha_verify_url", &self.captcha_verify_url)
            .field("admin_email", &self.admin_email)
            .field("admin_password", &hidden(self.admin_password.is_some()))
            .field("log_filter", &self.log_filter)
            .field("log_format", &self.log_format)
            .field("default_slug_length", &self.default_slug_length)
            .field("max_urls_per_user", &self.max_urls_per_user)
            .finish()
    }
}

impl Config {
    /// Read the configuration from the process environment.
    pub fn from_env() -> Result<Self, ConfigError> {
        let base_url = env_str("URLET_BASE_URL").unwrap_or_else(|| "http://localhost:8080".into());
        let base_url = base_url.trim().trim_end_matches('/').to_string();
        let (base_scheme, base_host) = split_base_url(&base_url)?;
        let base_host = canonical_base_host(&base_host).ok_or_else(|| ConfigError::Invalid {
            name: "URLET_BASE_URL",
            value: base_url.clone(),
        })?;
        let extra_domains_raw = env_str("URLET_DOMAINS");
        let extra_domains_declared = extra_domains_raw.is_some();
        let extra_domains = parse_extra_domains(extra_domains_raw, &base_host)?;

        let database_url = env_str("URLET_DATABASE_URL")
            .unwrap_or_else(|| "sqlite:///data/urlet.db?mode=rwc".into());

        let data_dir = match env_str("URLET_DATA_DIR") {
            Some(dir) => PathBuf::from(dir),
            None => sqlite_dir(&database_url).unwrap_or_else(|| PathBuf::from("data")),
        };
        std::fs::create_dir_all(&data_dir).map_err(|source| ConfigError::Io {
            path: data_dir.clone(),
            source,
        })?;

        let explicit_secret = env_str("URLET_SECRET").filter(|s| !s.is_empty());
        let secret = Arc::new(secret::load_or_create(
            &data_dir,
            explicit_secret.as_deref(),
        )?);

        let secure_cookies = match env_str("URLET_SECURE_COOKIES") {
            Some(v) => parse_bool(&v),
            None => base_scheme == "https",
        };

        Ok(Config {
            bind: env_str("URLET_BIND")
                .unwrap_or_else(|| "0.0.0.0:8080".into())
                .parse()
                .map_err(|_| ConfigError::Invalid {
                    name: "URLET_BIND",
                    value: env_str("URLET_BIND").unwrap_or_default(),
                })?,
            base_url,
            base_scheme,
            base_host,
            extra_domains,
            extra_domains_declared,
            secure_cookies,
            database_url,
            data_dir,
            secret,
            session_ttl_hours: env_i64("URLET_SESSION_TTL_HOURS", 168),
            unlock_ttl_minutes: env_i64("URLET_UNLOCK_TTL_MINUTES", 30),
            trust_proxy: env_str("URLET_TRUST_PROXY")
                .map(|v| parse_bool(&v))
                .unwrap_or(false),
            registration_override: env_str("URLET_REGISTRATION")
                .filter(|v| !v.trim().is_empty())
                .map(|v| v.trim().to_ascii_lowercase()),
            slug_length_override: env_str("URLET_DEFAULT_SLUG_LENGTH")
                .and_then(|v| v.trim().parse::<usize>().ok())
                .filter(|v| (4..=16).contains(v)),
            captcha_provider_override: env_str("URLET_CAPTCHA_PROVIDER")
                .filter(|v| !v.trim().is_empty())
                .map(|v| v.trim().to_ascii_lowercase()),
            captcha_site_key_override: env_str("URLET_CAPTCHA_SITE_KEY")
                .filter(|v| !v.trim().is_empty()),
            captcha_secret_key_override: env_str("URLET_CAPTCHA_SECRET_KEY")
                .filter(|v| !v.trim().is_empty()),
            captcha_allowed_hostnames_override: env_str("URLET_CAPTCHA_ALLOWED_HOSTNAMES")
                .filter(|v| !v.trim().is_empty()),
            captcha_on_register_override: env_bool("URLET_CAPTCHA_ON_REGISTER"),
            captcha_on_login_override: env_bool("URLET_CAPTCHA_ON_LOGIN"),
            captcha_on_create_override: env_bool("URLET_CAPTCHA_ON_CREATE"),
            captcha_verify_url: None,
            admin_email: env_str("URLET_ADMIN_EMAIL").filter(|v| !v.trim().is_empty()),
            admin_password: env_str("URLET_ADMIN_PASSWORD").filter(|v| !v.is_empty()),
            log_filter: env_str("URLET_LOG").unwrap_or_else(|| "info".into()),
            log_format: LogFormat::parse(&env_str("URLET_LOG_FORMAT").unwrap_or_default()),
            default_slug_length: 7,
            max_urls_per_user: 1000,
        })
    }

    /// Configuration suitable for tests: isolated data directory, fixed secret.
    pub fn for_tests(data_dir: &Path) -> Self {
        let secret = Arc::new(b"test-secret-test-secret-test-secret".to_vec());
        Self {
            bind: "127.0.0.1:0".parse().expect("valid socket addr"),
            base_url: "http://localhost:8080".into(),
            base_scheme: "http".into(),
            base_host: "localhost:8080".into(),
            extra_domains: Vec::new(),
            extra_domains_declared: false,
            secure_cookies: false,
            database_url: format!("sqlite://{}/test.db?mode=rwc", data_dir.display()),
            data_dir: data_dir.to_path_buf(),
            secret,
            session_ttl_hours: 168,
            unlock_ttl_minutes: 30,
            trust_proxy: false,
            registration_override: None,
            slug_length_override: None,
            captcha_provider_override: None,
            captcha_site_key_override: None,
            captcha_secret_key_override: None,
            captcha_allowed_hostnames_override: None,
            captcha_on_register_override: None,
            captcha_on_login_override: None,
            captcha_on_create_override: None,
            captcha_verify_url: None,
            admin_email: None,
            admin_password: None,
            log_filter: "warn".into(),
            log_format: LogFormat::Compact,
            default_slug_length: 7,
            max_urls_per_user: 1000,
        }
    }

    /// Absolute URL for a slug on the canonical host.
    pub fn short_url(&self, slug: &str) -> String {
        format!("{}/{}", self.base_url, slug)
    }

    /// Absolute URL for a slug on any managed hostname.
    pub fn short_url_on(&self, hostname: &str, slug: &str) -> String {
        format!("{}://{}/{}", self.base_scheme, hostname, slug)
    }

    /// Whether the request scheme should be treated as secure.
    pub fn is_https(&self) -> bool {
        self.base_scheme == "https"
    }

    /// Filesystem path of the SQLite database, when the URL points at a file.
    pub fn sqlite_path(&self) -> Option<PathBuf> {
        sqlite_path(&self.database_url)
    }
}

fn env_str(name: &str) -> Option<String> {
    std::env::var(name).ok()
}

fn env_i64(name: &str, default: i64) -> i64 {
    env_str(name)
        .and_then(|v| v.trim().parse().ok())
        .unwrap_or(default)
}

/// Boolean override: `None` when the variable is unset or empty, so the
/// settings table keeps its value.
fn env_bool(name: &str) -> Option<bool> {
    env_str(name)
        .filter(|v| !v.trim().is_empty())
        .map(|v| parse_bool(&v))
}

fn parse_bool(value: &str) -> bool {
    matches!(
        value.trim().to_ascii_lowercase().as_str(),
        "1" | "true" | "yes" | "on"
    )
}

/// Canonical stored form of the `URLET_BASE_URL` host.
///
/// Normal hostnames use the same rules as request hosts. The canonical host may
/// also be an IPv6 literal or a Docker-style underscore service name; those are
/// accepted as an opaque ASCII key so the instance still starts. Unicode
/// hostnames are rejected instead of silently serving 404s: use their punycode
/// form in `URLET_BASE_URL`.
fn canonical_base_host(raw: &str) -> Option<String> {
    if let Some(host) = crate::services::domains::normalize_host(raw) {
        return Some(host);
    }
    let trimmed = raw.trim();
    if trimmed.is_empty()
        || !trimmed.is_ascii()
        || (!trimmed.starts_with('[') && !trimmed.contains('_'))
        || trimmed.chars().any(|c| {
            c.is_control() || c.is_whitespace() || matches!(c, '/' | '\\' | '@' | '#' | '?')
        })
    {
        return None;
    }
    Some(crate::services::domains::raw_host_key(trimmed))
}

/// Parse `URLET_DOMAINS`: comma separated hostnames, normalised, de-duplicated
/// and with the canonical host removed.
fn parse_extra_domains(raw: Option<String>, base_host: &str) -> Result<Vec<String>, ConfigError> {
    let Some(raw) = raw else {
        return Ok(Vec::new());
    };
    let mut result: Vec<String> = Vec::new();
    for part in raw.split(',') {
        let part = part.trim();
        if part.is_empty() {
            continue;
        }
        let host = crate::services::domains::validate_domain_hostname(part).map_err(|_| {
            ConfigError::Invalid {
                name: "URLET_DOMAINS",
                value: raw.clone(),
            }
        })?;
        if host.eq_ignore_ascii_case(base_host) {
            continue;
        }
        if !result
            .iter()
            .any(|existing| existing.eq_ignore_ascii_case(&host))
        {
            result.push(host);
        }
    }
    Ok(result)
}

/// Split a base URL into `(scheme, host)` without pulling in a URL parser.
fn split_base_url(base_url: &str) -> Result<(String, String), ConfigError> {
    let invalid = || ConfigError::Invalid {
        name: "URLET_BASE_URL",
        value: base_url.to_string(),
    };
    let (scheme, rest) = base_url.split_once("://").ok_or_else(invalid)?;
    if scheme.is_empty() || rest.is_empty() {
        return Err(invalid());
    }
    let host = rest.split('/').next().unwrap_or_default().to_string();
    if host.is_empty() {
        return Err(invalid());
    }
    Ok((scheme.to_ascii_lowercase(), host))
}

/// Filesystem path of the SQLite database file.
fn sqlite_path(database_url: &str) -> Option<PathBuf> {
    let path = database_url
        .strip_prefix("sqlite://")
        .or_else(|| database_url.strip_prefix("sqlite:"))?;
    let path = path.split('?').next().unwrap_or(path);
    if path.is_empty() {
        return None;
    }
    Some(PathBuf::from(path))
}

/// Directory that contains the SQLite file, used for the generated secret.
fn sqlite_dir(database_url: &str) -> Option<PathBuf> {
    let path = sqlite_path(database_url)?;
    path.parent()
        .filter(|p| !p.as_os_str().is_empty())
        .map(Path::to_path_buf)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splits_base_url() {
        let (scheme, host) = split_base_url("https://s.example.com/base").expect("valid");
        assert_eq!(scheme, "https");
        assert_eq!(host, "s.example.com");

        let (scheme, host) = split_base_url("http://localhost:8080").expect("valid");
        assert_eq!(scheme, "http");
        assert_eq!(host, "localhost:8080");
    }

    #[test]
    fn rejects_base_url_without_scheme() {
        assert!(split_base_url("localhost:8080").is_err());
        assert!(split_base_url("https://").is_err());
    }

    #[test]
    fn derives_sqlite_paths() {
        assert_eq!(
            sqlite_path("sqlite:///data/urlet.db?mode=rwc"),
            Some(PathBuf::from("/data/urlet.db"))
        );
        assert_eq!(sqlite_path("sqlite:"), None);
    }

    #[test]
    fn derives_sqlite_directory() {
        assert_eq!(
            sqlite_dir("sqlite:///data/urlet.db?mode=rwc"),
            Some(PathBuf::from("/data"))
        );
        assert_eq!(sqlite_dir("sqlite:urlet.db"), None);
    }

    #[test]
    fn parses_booleans() {
        for truthy in ["1", "true", "TRUE", "yes", "on"] {
            assert!(parse_bool(truthy), "{truthy}");
        }
        for falsy in ["0", "false", "no", "off", ""] {
            assert!(!parse_bool(falsy), "{falsy}");
        }
    }

    #[test]
    fn canonicalises_operator_supplied_base_hosts() {
        assert_eq!(
            canonical_base_host("Example.COM."),
            Some("example.com".into())
        );
        assert_eq!(canonical_base_host(""), None);
        assert_eq!(canonical_base_host("[::1]:8080"), Some("[::1]:8080".into()));
        assert_eq!(
            canonical_base_host("urlet_app:443"),
            Some("urlet_app".into())
        );
        assert_eq!(canonical_base_host("例え.jp"), None);
        assert_eq!(canonical_base_host("bad host"), None);
        assert_eq!(canonical_base_host("example.com:99999"), None);
    }

    #[test]
    fn parses_and_deduplicates_extra_domains() {
        let parsed = parse_extra_domains(
            Some("Go.Example.com, go.example.com, links.example.com".into()),
            "localhost:8080",
        )
        .expect("valid list");
        assert_eq!(parsed, vec!["go.example.com", "links.example.com"]);

        let parsed = parse_extra_domains(
            Some("localhost:8080, b.example.com".into()),
            "localhost:8080",
        )
        .expect("canonical host is filtered");
        assert_eq!(parsed, vec!["b.example.com"]);

        assert!(
            parse_extra_domains(Some("https://bad.example.com".into()), "localhost:8080").is_err()
        );
        assert!(parse_extra_domains(Some("".into()), "localhost:8080")
            .expect("empty")
            .is_empty());
    }

    #[test]
    fn debug_output_never_contains_a_secret() {
        let mut config = Config::for_tests(Path::new("."));
        config.admin_password = Some("bootstrap-password".into());
        config.captcha_secret_key_override = Some("captcha-secret".into());

        let printed = format!("{config:?}");
        assert!(!printed.contains("bootstrap-password"), "{printed}");
        assert!(!printed.contains("captcha-secret"), "{printed}");
        // The HMAC secret is never printable either, not even as bytes.
        assert!(!printed.contains("116"), "no byte dump of the secret");
        assert!(printed.contains(secret::REDACTED), "{printed}");
        // Non secret fields stay useful for debugging.
        assert!(printed.contains("localhost:8080"), "{printed}");
    }
}
