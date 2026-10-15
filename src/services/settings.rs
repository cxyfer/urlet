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
    "default_redirect_type",
    "analytics_enabled",
    "click_retention_days",
];

#[derive(Debug)]
pub struct Settings {
    values: RwLock<HashMap<String, String>>,
    registration_override: Option<RegistrationMode>,
    slug_length_override: Option<usize>,
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
            registration_override: config
                .registration_override
                .as_deref()
                .map(RegistrationMode::parse),
            slug_length_override: config.slug_length_override,
        })
    }

    fn raw(&self, key: &str) -> Option<String> {
        self.values.read().ok().and_then(|values| values.get(key).cloned())
    }

    fn raw_i64(&self, key: &str, fallback: i64) -> i64 {
        self.raw(key).and_then(|value| value.trim().parse().ok()).unwrap_or(fallback)
    }

    fn raw_bool(&self, key: &str, fallback: bool) -> bool {
        self.raw(key)
            .map(|value| matches!(value.trim().to_ascii_lowercase().as_str(), "1" | "true" | "yes" | "on"))
            .unwrap_or(fallback)
    }

    pub fn site_name(&self) -> String {
        self.raw("site_name").filter(|value| !value.trim().is_empty()).unwrap_or_else(|| "uwurls".into())
    }

    pub fn registration_mode(&self) -> RegistrationMode {
        self.registration_override
            .unwrap_or_else(|| RegistrationMode::parse(&self.raw("registration_mode").unwrap_or_default()))
    }

    pub fn default_slug_length(&self) -> usize {
        self.slug_length_override
            .or_else(|| self.raw("default_slug_length").and_then(|value| value.trim().parse().ok()))
            .filter(|value| (4..=16).contains(value))
            .unwrap_or(7)
    }

    pub fn max_urls_per_user(&self) -> i64 {
        self.raw_i64("max_urls_per_user", 1000).max(0)
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

    pub fn click_retention_days(&self) -> i64 {
        self.raw_i64("click_retention_days", 0).max(0)
    }

    /// Snapshot of all settings, for the admin API.
    pub fn snapshot(&self) -> HashMap<String, String> {
        self.values.read().map(|values| values.clone()).unwrap_or_default()
    }

    /// Validate and persist one setting, then refresh the cache.
    pub async fn update(
        &self,
        pool: &SqlitePool,
        key: &str,
        value: &str,
        actor: Option<i64>,
    ) -> Result<()> {
        if !EDITABLE_KEYS.contains(&key) {
            return Err(AppError::validation("key", format!("unknown setting: {key}")));
        }
        let normalized = validate_value(key, value)?;
        db::settings::set(pool, key, &normalized, actor)
            .await
            .map_err(|err| AppError::internal(format!("cannot save setting: {err}")))?;
        if let Ok(mut values) = self.values.write() {
            values.insert(key.to_string(), normalized);
        }
        Ok(())
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
            mode @ (RegistrationMode::Open | RegistrationMode::Invite | RegistrationMode::Closed) => {
                Ok(mode.as_str().to_string())
            }
        },
        "default_slug_length" => {
            let parsed: usize = value.parse().map_err(|_| invalid("slug length must be a number"))?;
            if !(4..=16).contains(&parsed) {
                return Err(invalid("slug length must be between 4 and 16"));
            }
            Ok(parsed.to_string())
        }
        "max_urls_per_user" => {
            let parsed: i64 = value.parse().map_err(|_| invalid("quota must be a number"))?;
            if !(0..=1_000_000).contains(&parsed) {
                return Err(invalid("quota must be between 0 and 1000000 (0 = unlimited)"));
            }
            Ok(parsed.to_string())
        }
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
            let parsed: i64 = value.parse().map_err(|_| invalid("retention must be a number of days"))?;
            if !(0..=3650).contains(&parsed) {
                return Err(invalid("retention must be between 0 and 3650 days"));
            }
            Ok(parsed.to_string())
        }
        _ => Err(invalid("unknown setting")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_registration_modes() {
        assert_eq!(RegistrationMode::parse("open"), RegistrationMode::Open);
        assert_eq!(RegistrationMode::parse(" INVITE "), RegistrationMode::Invite);
        assert_eq!(RegistrationMode::parse("invite_only"), RegistrationMode::Invite);
        assert_eq!(RegistrationMode::parse("closed"), RegistrationMode::Closed);
        assert_eq!(RegistrationMode::parse("nonsense"), RegistrationMode::Open);
    }

    #[test]
    fn validates_editable_values() {
        assert_eq!(validate_value("default_slug_length", "9").expect("ok"), "9");
        assert!(validate_value("default_slug_length", "2").is_err());
        assert!(validate_value("default_redirect_type", "200").is_err());
        assert_eq!(validate_value("default_redirect_type", "307").expect("ok"), "307");
        assert_eq!(validate_value("analytics_enabled", "off").expect("ok"), "false");
        assert!(validate_value("unknown_key", "x").is_err());
        assert!(validate_value("site_name", "").is_err());
    }
}
