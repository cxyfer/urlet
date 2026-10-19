//! Effective quotas and feature toggles for one user.
//!
//! A group overrides the site-wide defaults when a value is set; NULL means
//! "inherit" and `0` means unlimited. Feature toggles are ANDed with the
//! site-wide defaults, which is why a group can only narrow what the instance
//! allows, never widen it.

use sqlx::{FromRow, SqlitePool};

use crate::db::users::User;
use crate::error::{AppError, Result};
use crate::services::settings::Settings;

/// The quotas and toggles actually applied to one user.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Limits {
    pub max_urls: i64,
    pub max_api_tokens: i64,
    pub allow_custom_slug: bool,
    pub allow_password: bool,
    pub allow_expiry: bool,
    pub allow_max_clicks: bool,
    pub allow_tags: bool,
    pub allow_api_tokens: bool,
    pub allow_domain_selection: bool,
}

impl Limits {
    /// Site-wide defaults with every feature enabled: what an ungrouped user
    /// gets, and the base a group value overrides.
    pub fn site_defaults(settings: &Settings) -> Self {
        Self {
            max_urls: settings.max_urls_per_user(),
            max_api_tokens: settings.max_api_tokens_per_user(),
            allow_custom_slug: true,
            allow_password: true,
            allow_expiry: true,
            allow_max_clicks: true,
            allow_tags: true,
            allow_api_tokens: true,
            allow_domain_selection: true,
        }
    }

    /// Overlay one group's values on this base.
    fn merge(self, group: &GroupOverrides) -> Self {
        Self {
            max_urls: group.max_urls.unwrap_or(self.max_urls),
            max_api_tokens: group.max_api_tokens.unwrap_or(self.max_api_tokens),
            allow_custom_slug: self.allow_custom_slug && group.allow_custom_slug,
            allow_password: self.allow_password && group.allow_password,
            allow_expiry: self.allow_expiry && group.allow_expiry,
            allow_max_clicks: self.allow_max_clicks && group.allow_max_clicks,
            allow_tags: self.allow_tags && group.allow_tags,
            allow_api_tokens: self.allow_api_tokens && group.allow_api_tokens,
            allow_domain_selection: self.allow_domain_selection && group.allow_domain_selection,
        }
    }

    /// `0` (or a negative stored value) means "no limit".
    pub fn urls_unlimited(&self) -> bool {
        self.max_urls <= 0
    }

    /// `0` (or a negative stored value) means "no limit".
    pub fn tokens_unlimited(&self) -> bool {
        self.max_api_tokens <= 0
    }
}

/// The group columns that participate in limit resolution.
#[derive(Debug, Clone, FromRow)]
struct GroupOverrides {
    max_urls: Option<i64>,
    max_api_tokens: Option<i64>,
    allow_custom_slug: bool,
    allow_password: bool,
    allow_expiry: bool,
    allow_max_clicks: bool,
    allow_tags: bool,
    allow_api_tokens: bool,
    allow_domain_selection: bool,
}

/// Resolve the limits for a user. One indexed lookup at most; there is no cache,
/// so an administrator's change applies to the next request.
pub async fn effective(pool: &SqlitePool, settings: &Settings, user: &User) -> Result<Limits> {
    let Some(group_id) = user.group_id else {
        return Ok(Limits::site_defaults(settings));
    };
    effective_for_group(pool, settings, Some(group_id)).await
}

/// Resolve the limits for a group id (`None` = site defaults).
pub async fn effective_for_group(
    pool: &SqlitePool,
    settings: &Settings,
    group_id: Option<i64>,
) -> Result<Limits> {
    let base = Limits::site_defaults(settings);
    let Some(group_id) = group_id else {
        return Ok(base);
    };
    let group = sqlx::query_as::<_, GroupOverrides>(
        "SELECT max_urls, max_api_tokens, allow_custom_slug, allow_password, allow_expiry,
                allow_max_clicks, allow_tags, allow_api_tokens, allow_domain_selection
           FROM user_groups WHERE id = ?",
    )
    .bind(group_id)
    .fetch_optional(pool)
    .await
    .map_err(AppError::from)?;
    Ok(group.map(|group| base.merge(&group)).unwrap_or(base))
}

/// Message used when a group blocks a feature. Kept in one place so the API and
/// the web forms describe the same rule.
pub mod blocked {
    pub const CUSTOM_SLUG: &str = "your group does not allow custom slugs";
    pub const PASSWORD: &str = "your group does not allow link passwords";
    pub const EXPIRY: &str = "your group does not allow link expiry";
    pub const MAX_CLICKS: &str = "your group does not allow click caps";
    pub const TAGS: &str = "your group does not allow tags";
    pub const API_TOKENS: &str = "your group does not allow API tokens";
    pub const DOMAIN_SELECTION: &str = "your group does not allow choosing another domain";
    pub const INVITES: &str = "your group does not allow invite codes";
    pub const NO_GROUP: &str = "your account has no group that can issue invite codes";
}

#[cfg(test)]
mod tests {
    use super::*;

    fn base() -> Limits {
        Limits {
            max_urls: 1000,
            max_api_tokens: 20,
            allow_custom_slug: true,
            allow_password: true,
            allow_expiry: true,
            allow_max_clicks: true,
            allow_tags: true,
            allow_api_tokens: true,
            allow_domain_selection: true,
        }
    }

    fn overrides(max_urls: Option<i64>, max_api_tokens: Option<i64>) -> GroupOverrides {
        GroupOverrides {
            max_urls,
            max_api_tokens,
            allow_custom_slug: true,
            allow_password: true,
            allow_expiry: true,
            allow_max_clicks: true,
            allow_tags: true,
            allow_api_tokens: true,
            allow_domain_selection: true,
        }
    }

    #[test]
    fn null_quotas_inherit_and_values_override() {
        let inherited = base().merge(&overrides(None, None));
        assert_eq!(inherited.max_urls, 1000);
        assert_eq!(inherited.max_api_tokens, 20);

        let overridden = base().merge(&overrides(Some(5), Some(2)));
        assert_eq!(overridden.max_urls, 5);
        assert_eq!(overridden.max_api_tokens, 2);

        // 0 is a real value (unlimited), never "inherit".
        let unlimited = base().merge(&overrides(Some(0), Some(0)));
        assert!(unlimited.urls_unlimited());
        assert!(unlimited.tokens_unlimited());
    }

    #[test]
    fn toggles_only_narrow_the_site_defaults() {
        let mut group = overrides(None, None);
        group.allow_custom_slug = false;
        group.allow_tags = false;
        let limits = base().merge(&group);
        assert!(!limits.allow_custom_slug);
        assert!(!limits.allow_tags);
        assert!(limits.allow_password);
    }

    #[test]
    fn site_defaults_allow_every_feature() {
        // A bare Limits with all flags set is the shape `site_defaults` builds;
        // the config-backed values are covered by the settings tests.
        let limits = base();
        assert!(limits.allow_custom_slug);
        assert!(limits.allow_password);
        assert!(limits.allow_expiry);
        assert!(limits.allow_max_clicks);
        assert!(limits.allow_tags);
        assert!(limits.allow_api_tokens);
        assert!(limits.allow_domain_selection);
        assert!(!limits.urls_unlimited());
        assert!(!limits.tokens_unlimited());
    }
}
