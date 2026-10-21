//! Typed, cached access to the `settings` table.
//!
//! The map is kept in memory and refreshed on write, so request handling never
//! pays for a settings query.

use std::collections::HashMap;
use std::sync::RwLock;

use sqlx::SqlitePool;

use crate::config::Config;
use crate::db;
use crate::error::{AppError, Result};
use crate::services::captcha::{self, CaptchaProvider};
use crate::services::invite_quota::InvitePeriod;

/// Who may register.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RegistrationMode {
    Open,
    Invite,
    Closed,
}

impl RegistrationMode {
    pub fn parse(value: &str) -> Self {
        match value.trim().to_ascii_lowercase().as_str() {
            "invite" | "invite_only" => Self::Invite,
            "closed" | "disabled" | "none" => Self::Closed,
            _ => Self::Open,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Open => "open",
            Self::Invite => "invite",
            Self::Closed => "closed",
        }
    }

    pub fn allows_public_registration(self) -> bool {
        self == Self::Open
    }
}

/// Settings keys that administrators may change through the API.
pub const EDITABLE_KEYS: &[&str] = &[
    "site_name",
    "registration_mode",
    "default_slug_length",
    "max_urls_per_user",
    "max_api_tokens_per_user",
    "max_invites_per_period",
    "invite_period",
    "default_redirect_type",
    "analytics_enabled",
    "click_retention_days",
    "captcha_provider",
    "captcha_site_key",
    "captcha_secret_key",
    "captcha_allowed_hostnames",
    "captcha_on_register",
    "captcha_on_login",
    "captcha_on_create",
    "theme_switching",
];

/// Who may change their own theme.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ThemeSwitching {
    All,
    AdminOnly,
}

impl ThemeSwitching {
    pub fn parse(value: &str) -> Self {
        match value.trim().to_ascii_lowercase().as_str() {
            // An absent value means the default; an unknown value is treated as
            // the restrictive mode rather than silently granting switching.
            "" | "all" | "everyone" => Self::All,
            "admin" | "admins" | "admin_only" | "admin-only" => Self::AdminOnly,
            _ => Self::AdminOnly,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::All => "all",
            Self::AdminOnly => "admin",
        }
    }
}

/// Settings that are write only: stored, used, but never handed back out.
pub const SECRET_KEYS: &[&str] = &["captcha_secret_key"];

pub struct Settings {
    values: RwLock<HashMap<String, String>>,
    /// Serialises writes to the stored settings and the cache. Theme updates
    /// also write `theme_switching`, so they take this guard across their own
    /// transaction; that keeps a concurrent settings update from applying its
    /// cache entry after a newer theme transaction committed.
    update_lock: tokio::sync::Mutex<()>,
    registration_override: Option<RegistrationMode>,
    slug_length_override: Option<usize>,
    captcha_provider_override: Option<CaptchaProvider>,
    captcha_site_key_override: Option<String>,
    captcha_secret_key_override: Option<String>,
    captcha_allowed_hostnames_override: Option<String>,
    captcha_on_register_override: Option<bool>,
    captcha_on_login_override: Option<bool>,
    captcha_on_create_override: Option<bool>,
}

/// Only the *names* of the stored settings are printed, never their values: the
/// map holds the captcha secret. An override that carries a secret is redacted.
impl std::fmt::Debug for Settings {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let mut keys: Vec<String> = self
            .values
            .read()
            .map(|values| values.keys().cloned().collect())
            .unwrap_or_default();
        keys.sort_unstable();

        f.debug_struct("Settings")
            .field("keys", &keys)
            .field("registration_override", &self.registration_override)
            .field("slug_length_override", &self.slug_length_override)
            .field("captcha_provider_override", &self.captcha_provider_override)
            .field("captcha_site_key_override", &self.captcha_site_key_override)
            .field(
                "captcha_secret_key_override",
                &self
                    .captcha_secret_key_override
                    .as_ref()
                    .map(|_| crate::services::secret::REDACTED),
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
            .finish()
    }
}

impl Settings {
    /// Load settings, applying environment overrides.
    pub async fn load(pool: &SqlitePool, config: &Config) -> Result<Self> {
        db::settings::ensure_defaults(pool)
            .await
            .map_err(|err| AppError::internal(format!("cannot seed settings: {err}")))?;
        let values = db::settings::all(pool)
            .await
            .map_err(|err| AppError::internal(format!("cannot load settings: {err}")))?;
        Ok(Self {
            values: RwLock::new(values),
            update_lock: tokio::sync::Mutex::new(()),
            registration_override: config
                .registration_override
                .as_deref()
                .map(RegistrationMode::parse),
            slug_length_override: config.slug_length_override,
            captcha_provider_override: config
                .captcha_provider_override
                .as_deref()
                .map(CaptchaProvider::parse),
            captcha_site_key_override: config.captcha_site_key_override.clone(),
            captcha_secret_key_override: config.captcha_secret_key_override.clone(),
            // An unusable value is reported once, here, and then ignored: a typo
            // in the environment must not reject every verification.
            captcha_allowed_hostnames_override: config
                .captcha_allowed_hostnames_override
                .as_deref()
                .and_then(|value| match captcha::validate_allowed_hostnames(value) {
                    Ok(canonical) => Some(canonical),
                    Err(message) => {
                        tracing::warn!(
                            message = %message,
                            "ignoring URLET_CAPTCHA_ALLOWED_HOSTNAMES"
                        );
                        None
                    }
                }),
            captcha_on_register_override: config.captcha_on_register_override,
            captcha_on_login_override: config.captcha_on_login_override,
            captcha_on_create_override: config.captcha_on_create_override,
        })
    }

    fn raw(&self, key: &str) -> Option<String> {
        self.values
            .read()
            .ok()
            .and_then(|values| values.get(key).cloned())
    }

    fn raw_i64(&self, key: &str, fallback: i64) -> i64 {
        self.raw(key)
            .and_then(|value| value.trim().parse().ok())
            .unwrap_or(fallback)
    }

    fn raw_bool(&self, key: &str, fallback: bool) -> bool {
        self.raw(key)
            .map(|value| {
                matches!(
                    value.trim().to_ascii_lowercase().as_str(),
                    "1" | "true" | "yes" | "on"
                )
            })
            .unwrap_or(fallback)
    }

    pub fn site_name(&self) -> String {
        self.raw("site_name")
            .filter(|value| !value.trim().is_empty())
            .unwrap_or_else(|| "urlet".into())
    }

    pub fn registration_mode(&self) -> RegistrationMode {
        self.registration_override.unwrap_or_else(|| {
            RegistrationMode::parse(&self.raw("registration_mode").unwrap_or_default())
        })
    }

    pub fn default_slug_length(&self) -> usize {
        self.slug_length_override
            .or_else(|| {
                self.raw("default_slug_length")
                    .and_then(|value| value.trim().parse().ok())
            })
            .filter(|value| (4..=16).contains(value))
            .unwrap_or(7)
    }

    pub fn max_urls_per_user(&self) -> i64 {
        self.raw_i64("max_urls_per_user", 1000).max(0)
    }

    /// Site-wide fallback for the number of usable API tokens per user.
    pub fn max_api_tokens_per_user(&self) -> i64 {
        self.raw_i64("max_api_tokens_per_user", 20).max(0)
    }

    /// Site-wide fallback for invite codes issued per period; 0 is unlimited.
    /// It also caps invites that are not pinned to a group.
    pub fn max_invites_per_period(&self) -> i64 {
        self.raw_i64("max_invites_per_period", 0).max(0)
    }

    /// Period of the site-wide invite quota (invites without a group).
    pub fn invite_period(&self) -> InvitePeriod {
        InvitePeriod::parse(&self.raw("invite_period").unwrap_or_default())
            .unwrap_or(InvitePeriod::Month)
    }

    pub fn default_redirect_type(&self) -> i64 {
        match self.raw_i64("default_redirect_type", 302) {
            301 | 302 | 307 | 308 => self.raw_i64("default_redirect_type", 302),
            _ => 302,
        }
    }

    pub fn analytics_enabled(&self) -> bool {
        self.raw_bool("analytics_enabled", true)
    }

    /// Who may switch to another theme. Administrators are never restricted.
    pub fn theme_switching(&self) -> ThemeSwitching {
        ThemeSwitching::parse(&self.raw("theme_switching").unwrap_or_default())
    }

    pub fn click_retention_days(&self) -> i64 {
        self.raw_i64("click_retention_days", 0).max(0)
    }

    /// Configured bot protection provider.
    pub fn captcha_provider(&self) -> CaptchaProvider {
        self.captcha_provider_override.unwrap_or_else(|| {
            CaptchaProvider::parse(&self.raw("captcha_provider").unwrap_or_default())
        })
    }

    /// Public site key. Not a secret, so it is safe to render.
    pub fn captcha_site_key(&self) -> String {
        match &self.captcha_site_key_override {
            Some(value) => value.trim().to_string(),
            None => self
                .raw("captcha_site_key")
                .unwrap_or_default()
                .trim()
                .to_string(),
        }
    }

    /// Provider secret. Never rendered and never returned by the admin API.
    pub fn captcha_secret_key(&self) -> String {
        match &self.captcha_secret_key_override {
            Some(value) => value.trim().to_string(),
            None => self
                .raw("captcha_secret_key")
                .unwrap_or_default()
                .trim()
                .to_string(),
        }
    }

    /// True when a secret is stored, without revealing it.
    pub fn captcha_secret_configured(&self) -> bool {
        !self.captcha_secret_key().is_empty()
    }

    /// Extra hostnames a provider may report, canonicalised and comma separated.
    pub fn captcha_allowed_hostnames(&self) -> String {
        match &self.captcha_allowed_hostnames_override {
            Some(value) => value.clone(),
            None => self
                .raw("captcha_allowed_hostnames")
                .unwrap_or_default()
                .trim()
                .to_string(),
        }
    }

    /// [`captcha_allowed_hostnames`] as a list.
    ///
    /// [`captcha_allowed_hostnames`]: Settings::captcha_allowed_hostnames
    pub fn captcha_allowed_hostname_list(&self) -> Vec<String> {
        captcha::parse_allowed_hostnames(&self.captcha_allowed_hostnames())
    }

    pub fn captcha_on_register(&self) -> bool {
        self.captcha_on_register_override
            .unwrap_or_else(|| self.raw_bool("captcha_on_register", false))
    }

    pub fn captcha_on_login(&self) -> bool {
        self.captcha_on_login_override
            .unwrap_or_else(|| self.raw_bool("captcha_on_login", false))
    }

    pub fn captcha_on_create(&self) -> bool {
        self.captcha_on_create_override
            .unwrap_or_else(|| self.raw_bool("captcha_on_create", false))
    }

    /// Snapshot of all settings, for the admin API.
    ///
    /// Write only settings are dropped: the secret key must never leave the
    /// process, not even to an authenticated administrator.
    pub fn snapshot(&self) -> HashMap<String, String> {
        let mut values = self
            .values
            .read()
            .map(|values| values.clone())
            .unwrap_or_default();
        for key in SECRET_KEYS {
            values.remove(*key);
        }
        values
    }

    /// Validate and persist one setting, then refresh the cache.
    pub async fn update(
        &self,
        pool: &SqlitePool,
        key: &str,
        value: &str,
        actor: Option<i64>,
    ) -> Result<()> {
        self.update_many(pool, &[(key.to_string(), value.to_string())], actor)
            .await
    }

    /// Validate and persist a batch of settings, then refresh the cache.
    ///
    /// Every value is validated *before* anything is written, and the writes run
    /// in one transaction: a rejected value must never leave the instance half
    /// reconfigured. That matters most for bot protection, where a new provider
    /// saved without its keys would lock every visitor out.
    pub async fn update_many(
        &self,
        pool: &SqlitePool,
        values: &[(String, String)],
        actor: Option<i64>,
    ) -> Result<()> {
        if values.is_empty() {
            return Err(AppError::bad_request("no settings were submitted"));
        }

        // Phase 1: nothing is stored until every pair is known to be valid.
        // The write lock also covers the cache update below, so two successful
        // writes cannot leave the cache reflecting the older one.
        let _guard = self.update_lock.lock().await;
        let mut normalized: Vec<(String, String)> = Vec::with_capacity(values.len());
        for (key, value) in values {
            if !EDITABLE_KEYS.contains(&key.as_str()) {
                return Err(AppError::validation(
                    "key",
                    format!("unknown setting: {key}"),
                ));
            }
            normalized.push((key.clone(), validate_value(key, value)?));
        }

        // Phase 2: all or nothing.
        db::settings::set_many(pool, &normalized, actor)
            .await
            .map_err(|err| AppError::internal(format!("cannot save settings: {err}")))?;
        if let Ok(mut cache) = self.values.write() {
            for (key, value) in normalized {
                cache.insert(key, value);
            }
        }
        Ok(())
    }

    /// Serialise a caller that writes one of these settings itself (the theme
    /// catalogue writes `theme_switching` atomically with the theme rows).
    pub(crate) async fn lock_updates(&self) -> tokio::sync::MutexGuard<'_, ()> {
        self.update_lock.lock().await
    }

    /// Apply a value that was already validated and committed by another
    /// transaction, without re-reading the settings table.
    ///
    /// Used by the theme catalogue when it writes `theme_switching` atomically
    /// with the theme rows: the cache must follow the commit, but a second
    /// database read could fail after the commit and leave the endpoint
    /// reporting an error for a successful write.
    pub(crate) fn set_cached(&self, key: &str, value: &str) {
        if let Ok(mut cache) = self.values.write() {
            cache.insert(key.to_string(), value.to_string());
        }
    }
}

/// Validate and canonicalise a setting value.
fn validate_value(key: &str, value: &str) -> Result<String> {
    let value = value.trim();
    let invalid = |message: &str| AppError::validation("value", message.to_string());
    match key {
        "site_name" => {
            if value.is_empty() || value.chars().count() > 60 {
                return Err(invalid("site name must be 1-60 characters"));
            }
            Ok(value.to_string())
        }
        "registration_mode" => match RegistrationMode::parse(value) {
            mode @ (RegistrationMode::Open
            | RegistrationMode::Invite
            | RegistrationMode::Closed) => Ok(mode.as_str().to_string()),
        },
        "default_slug_length" => {
            let parsed: usize = value
                .parse()
                .map_err(|_| invalid("slug length must be a number"))?;
            if !(4..=16).contains(&parsed) {
                return Err(invalid("slug length must be between 4 and 16"));
            }
            Ok(parsed.to_string())
        }
        "max_urls_per_user" | "max_api_tokens_per_user" | "max_invites_per_period" => {
            let parsed: i64 = value
                .parse()
                .map_err(|_| invalid("quota must be a number"))?;
            if !(0..=1_000_000).contains(&parsed) {
                return Err(invalid(
                    "quota must be between 0 and 1000000 (0 = unlimited)",
                ));
            }
            Ok(parsed.to_string())
        }
        "invite_period" => match InvitePeriod::parse(value) {
            Some(period) => Ok(period.as_str().to_string()),
            None => Err(invalid("invite period must be day, week or month")),
        },
        "default_redirect_type" => match value.parse::<i64>() {
            Ok(code @ (301 | 302 | 307 | 308)) => Ok(code.to_string()),
            _ => Err(invalid("redirect type must be 301, 302, 307 or 308")),
        },
        "analytics_enabled" => Ok(matches!(
            value.to_ascii_lowercase().as_str(),
            "1" | "true" | "yes" | "on"
        )
        .to_string()),
        "click_retention_days" => {
            let parsed: i64 = value
                .parse()
                .map_err(|_| invalid("retention must be a number of days"))?;
            if !(0..=3650).contains(&parsed) {
                return Err(invalid("retention must be between 0 and 3650 days"));
            }
            Ok(parsed.to_string())
        }
        "captcha_provider" => {
            if !CaptchaProvider::is_known(value) {
                return Err(invalid("provider must be none, turnstile or recaptcha"));
            }
            Ok(CaptchaProvider::parse(value).as_str().to_string())
        }
        // The site key is public and may be cleared; the secret is write only
        // and is only stored when the administrator typed a new one.
        "captcha_site_key" | "captcha_secret_key" => {
            if value.chars().count() > crate::services::captcha::MAX_KEY_LEN {
                return Err(invalid("the key must be at most 200 characters"));
            }
            if value.chars().any(char::is_control) {
                return Err(invalid("the key must not contain control characters"));
            }
            Ok(value.to_string())
        }
        "captcha_allowed_hostnames" => {
            captcha::validate_allowed_hostnames(value).map_err(|message| invalid(&message))
        }
        "captcha_on_register" | "captcha_on_login" | "captcha_on_create" => Ok(matches!(
            value.to_ascii_lowercase().as_str(),
            "1" | "true" | "yes" | "on"
        )
        .to_string()),
        "theme_switching" => match value.to_ascii_lowercase().as_str() {
            "all" => Ok("all".to_string()),
            "admin" | "admins" | "admin_only" | "admin-only" => Ok("admin".to_string()),
            _ => Err(invalid("theme switching must be 'all' or 'admin'")),
        },
        _ => Err(invalid("unknown setting")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_theme_switching_modes() {
        assert_eq!(ThemeSwitching::parse("all"), ThemeSwitching::All);
        assert_eq!(ThemeSwitching::parse(" ADMIN "), ThemeSwitching::AdminOnly);
        assert_eq!(
            ThemeSwitching::parse("admin-only"),
            ThemeSwitching::AdminOnly
        );
        assert_eq!(ThemeSwitching::parse("nonsense"), ThemeSwitching::AdminOnly);
        assert_eq!(ThemeSwitching::All.as_str(), "all");
        assert_eq!(ThemeSwitching::AdminOnly.as_str(), "admin");
    }

    #[test]
    fn parses_registration_modes() {
        assert_eq!(RegistrationMode::parse("open"), RegistrationMode::Open);
        assert_eq!(
            RegistrationMode::parse(" INVITE "),
            RegistrationMode::Invite
        );
        assert_eq!(
            RegistrationMode::parse("invite_only"),
            RegistrationMode::Invite
        );
        assert_eq!(RegistrationMode::parse("closed"), RegistrationMode::Closed);
        assert_eq!(RegistrationMode::parse("nonsense"), RegistrationMode::Open);
    }

    #[test]
    fn validates_editable_values() {
        assert_eq!(validate_value("default_slug_length", "9").expect("ok"), "9");
        assert!(validate_value("default_slug_length", "2").is_err());
        assert_eq!(validate_value("max_urls_per_user", "50").expect("ok"), "50");
        assert_eq!(
            validate_value("max_api_tokens_per_user", "0").expect("ok"),
            "0"
        );
        assert!(validate_value("max_api_tokens_per_user", "-1").is_err());
        assert!(validate_value("max_api_tokens_per_user", "abc").is_err());
        assert!(validate_value("default_redirect_type", "200").is_err());
        assert_eq!(
            validate_value("default_redirect_type", "307").expect("ok"),
            "307"
        );
        assert_eq!(
            validate_value("analytics_enabled", "off").expect("ok"),
            "false"
        );
        assert!(validate_value("unknown_key", "x").is_err());
        assert!(validate_value("site_name", "").is_err());
        assert_eq!(validate_value("theme_switching", "all").expect("ok"), "all");
        assert_eq!(
            validate_value("theme_switching", "ADMIN").expect("ok"),
            "admin"
        );
        assert!(validate_value("theme_switching", "some").is_err());
    }

    #[test]
    fn validates_captcha_values() {
        assert_eq!(
            validate_value("captcha_provider", " TURNSTILE ").expect("ok"),
            "turnstile"
        );
        assert_eq!(
            validate_value("captcha_provider", "recaptcha").expect("ok"),
            "recaptcha"
        );
        assert_eq!(
            validate_value("captcha_provider", "none").expect("ok"),
            "none"
        );
        assert!(validate_value("captcha_provider", "hcaptcha").is_err());

        assert_eq!(
            validate_value("captcha_on_login", "on").expect("ok"),
            "true"
        );
        assert_eq!(
            validate_value("captcha_on_create", "no").expect("ok"),
            "false"
        );
        assert_eq!(
            validate_value("captcha_site_key", " 0x4AAAAAA ").expect("ok"),
            "0x4AAAAAA"
        );
        // The site key can be cleared; the secret is skipped by the route when
        // the administrator leaves the field empty.
        assert_eq!(validate_value("captcha_site_key", "").expect("ok"), "");
        assert!(validate_value("captcha_secret_key", &"x".repeat(201)).is_err());
        assert!(validate_value("captcha_secret_key", "bad\u{7}key").is_err());
        assert!(validate_value("captcha_secret_key", "fine").is_ok());

        // Hostname allowlist: canonicalised, bare names only, bounded in size.
        assert_eq!(
            validate_value(
                "captcha_allowed_hostnames",
                " a.example.com , B.example.com "
            )
            .expect("ok"),
            "a.example.com,b.example.com"
        );
        assert_eq!(
            validate_value("captcha_allowed_hostnames", "").expect("ok"),
            ""
        );
        assert_eq!(
            validate_value("captcha_allowed_hostnames", " , ").expect("ok"),
            ""
        );
        for bad in [
            "https://a.example.com",
            "a.example.com/path",
            "a.example.com:443",
            "user@a.example.com",
            "a b.example.com",
        ] {
            assert!(
                validate_value("captcha_allowed_hostnames", bad).is_err(),
                "{bad:?} must be refused"
            );
        }
        let eleven = (0..11)
            .map(|index| format!("h{index}.example.com"))
            .collect::<Vec<_>>()
            .join(",");
        assert!(validate_value("captcha_allowed_hostnames", &eleven).is_err());
    }

    #[test]
    fn secret_settings_are_editable_but_write_only() {
        assert!(!SECRET_KEYS.is_empty());
        for key in SECRET_KEYS {
            assert!(EDITABLE_KEYS.contains(key), "{key} must be editable");
        }
    }
}
