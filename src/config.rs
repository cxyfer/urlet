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
    #[error("UWURLS_SECRET must be at least 16 characters when set explicitly")]
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

#[derive(Debug, Clone)]
pub struct Config {
    pub bind: SocketAddr,
    /// Public base URL without a trailing slash, e.g. `https://s.example.com`.
    pub base_url: String,
    pub base_scheme: String,
    pub base_host: String,
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

impl Config {
    /// Read the configuration from the process environment.
    pub fn from_env() -> Result<Self, ConfigError> {
        let base_url = env_str("UWURLS_BASE_URL").unwrap_or_else(|| "http://localhost:8080".into());
        let base_url = base_url.trim().trim_end_matches('/').to_string();
        let (base_scheme, base_host) = split_base_url(&base_url)?;

        let database_url = env_str("UWURLS_DATABASE_URL")
            .unwrap_or_else(|| "sqlite:///data/uwurls.db?mode=rwc".into());

        let data_dir = match env_str("UWURLS_DATA_DIR") {
            Some(dir) => PathBuf::from(dir),
            None => sqlite_dir(&database_url).unwrap_or_else(|| PathBuf::from("data")),
        };
        std::fs::create_dir_all(&data_dir).map_err(|source| ConfigError::Io {
            path: data_dir.clone(),
            source,
        })?;

        let explicit_secret = env_str("UWURLS_SECRET").filter(|s| !s.is_empty());
        let secret = Arc::new(secret::load_or_create(
            &data_dir,
            explicit_secret.as_deref(),
        )?);

        let secure_cookies = match env_str("UWURLS_SECURE_COOKIES") {
            Some(v) => parse_bool(&v),
            None => base_scheme == "https",
        };

        Ok(Config {
            bind: env_str("UWURLS_BIND")
                .unwrap_or_else(|| "0.0.0.0:8080".into())
                .parse()
                .map_err(|_| ConfigError::Invalid {
                    name: "UWURLS_BIND",
                    value: env_str("UWURLS_BIND").unwrap_or_default(),
                })?,
            base_url,
            base_scheme,
            base_host,
            secure_cookies,
            database_url,
            data_dir,
            secret,
            session_ttl_hours: env_i64("UWURLS_SESSION_TTL_HOURS", 168),
            unlock_ttl_minutes: env_i64("UWURLS_UNLOCK_TTL_MINUTES", 30),
            trust_proxy: env_str("UWURLS_TRUST_PROXY")
                .map(|v| parse_bool(&v))
                .unwrap_or(false),
            registration_override: env_str("UWURLS_REGISTRATION")
                .filter(|v| !v.trim().is_empty())
                .map(|v| v.trim().to_ascii_lowercase()),
            slug_length_override: env_str("UWURLS_DEFAULT_SLUG_LENGTH")
                .and_then(|v| v.trim().parse::<usize>().ok())
                .filter(|v| (4..=16).contains(v)),
            captcha_provider_override: env_str("UWURLS_CAPTCHA_PROVIDER")
                .filter(|v| !v.trim().is_empty())
                .map(|v| v.trim().to_ascii_lowercase()),
            captcha_site_key_override: env_str("UWURLS_CAPTCHA_SITE_KEY")
                .filter(|v| !v.trim().is_empty()),
            captcha_secret_key_override: env_str("UWURLS_CAPTCHA_SECRET_KEY")
                .filter(|v| !v.trim().is_empty()),
            captcha_on_register_override: env_bool("UWURLS_CAPTCHA_ON_REGISTER"),
            captcha_on_login_override: env_bool("UWURLS_CAPTCHA_ON_LOGIN"),
            captcha_on_create_override: env_bool("UWURLS_CAPTCHA_ON_CREATE"),
            captcha_verify_url: None,
            admin_email: env_str("UWURLS_ADMIN_EMAIL").filter(|v| !v.trim().is_empty()),
            admin_password: env_str("UWURLS_ADMIN_PASSWORD").filter(|v| !v.is_empty()),
            log_filter: env_str("UWURLS_LOG").unwrap_or_else(|| "info".into()),
            log_format: LogFormat::parse(&env_str("UWURLS_LOG_FORMAT").unwrap_or_default()),
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

    /// Absolute URL for a slug, e.g. `https://s.example.com/abc1234`.
    pub fn short_url(&self, slug: &str) -> String {
        format!("{}/{}", self.base_url, slug)
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

/// Split a base URL into `(scheme, host)` without pulling in a URL parser.
fn split_base_url(base_url: &str) -> Result<(String, String), ConfigError> {
    let invalid = || ConfigError::Invalid {
        name: "UWURLS_BASE_URL",
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
            sqlite_path("sqlite:///data/uwurls.db?mode=rwc"),
            Some(PathBuf::from("/data/uwurls.db"))
        );
        assert_eq!(sqlite_path("sqlite:"), None);
    }

    #[test]
    fn derives_sqlite_directory() {
        assert_eq!(
            sqlite_dir("sqlite:///data/uwurls.db?mode=rwc"),
            Some(PathBuf::from("/data"))
        );
        assert_eq!(sqlite_dir("sqlite:uwurls.db"), None);
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
}
