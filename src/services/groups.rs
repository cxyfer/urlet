//! User group validation and listing helpers.
//!
//! The database layer stores one row per group; this module owns the rules that
//! a group must satisfy before it is written, and the member counts the admin
//! page and API show next to it.

use std::collections::HashMap;

use sqlx::SqlitePool;

use crate::db::user_groups::{self, GroupValues, UserGroup};
use crate::error::{AppError, Result};
use crate::services::invite_quota::InvitePeriod;

/// Longest accepted group name.
pub const MAX_NAME: usize = 40;
/// Longest accepted group description.
pub const MAX_DESCRIPTION: usize = 200;
/// Largest accepted quota value (0 still means unlimited).
pub const MAX_QUOTA: i64 = 1_000_000;

/// A validated, canonical group edit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GroupDraft {
    pub name: String,
    pub description: Option<String>,
    pub max_urls: Option<i64>,
    pub max_api_tokens: Option<i64>,
    /// Invite codes this group may issue per `invite_period`; `None` inherits.
    pub max_invites: Option<i64>,
    pub invite_period: InvitePeriod,
    /// Whether group members may issue their own invite codes.
    pub allow_invites: bool,
    pub allow_custom_slug: bool,
    pub allow_password: bool,
    pub allow_expiry: bool,
    pub allow_max_clicks: bool,
    pub allow_tags: bool,
    pub allow_api_tokens: bool,
    pub allow_domain_selection: bool,
    /// Group default domain; `None` means the site-wide default.
    pub default_domain_id: Option<i64>,
    /// When true, only `allowed_domain_ids` (plus the default) may be selected.
    pub restrict_domains: bool,
    /// `None` keeps the stored allowlist on update; `Some([])` removes every row.
    /// On create `None` and `Some([])` both mean "no restriction rows".
    pub allowed_domain_ids: Option<Vec<i64>>,
    /// Whether this edit should move the single default marker here.
    pub make_default: bool,
}

impl GroupDraft {
    /// Validate and canonicalise raw form or JSON input.
    pub fn validate(self) -> Result<Self> {
        let GroupDraft {
            name,
            description,
            max_urls,
            max_api_tokens,
            max_invites,
            invite_period,
            allow_invites,
            allow_custom_slug,
            allow_password,
            allow_expiry,
            allow_max_clicks,
            allow_tags,
            allow_api_tokens,
            allow_domain_selection,
            default_domain_id,
            restrict_domains,
            allowed_domain_ids,
            make_default,
        } = self;

        let name = name.trim();
        if name.is_empty() || name.chars().count() > MAX_NAME {
            return Err(AppError::validation(
                "name",
                format!("the group name must be 1-{MAX_NAME} characters"),
            ));
        }
        if name.chars().any(char::is_control) {
            return Err(AppError::validation(
                "name",
                "the group name must not contain control characters",
            ));
        }

        let description = match description.as_deref().map(str::trim) {
            None | Some("") => None,
            Some(value) => {
                if value.chars().count() > MAX_DESCRIPTION {
                    return Err(AppError::validation(
                        "description",
                        format!("the description must be at most {MAX_DESCRIPTION} characters"),
                    ));
                }
                if value.chars().any(char::is_control) {
                    return Err(AppError::validation(
                        "description",
                        "the description must not contain control characters",
                    ));
                }
                Some(value.to_string())
            }
        };

        Ok(Self {
            name: name.to_string(),
            description,
            max_urls: validate_quota("max_urls", max_urls)?,
            max_api_tokens: validate_quota("max_api_tokens", max_api_tokens)?,
            max_invites: validate_quota("max_invites", max_invites)?,
            invite_period,
            allow_invites,
            allow_custom_slug,
            allow_password,
            allow_expiry,
            allow_max_clicks,
            allow_tags,
            allow_api_tokens,
            allow_domain_selection,
            default_domain_id,
            restrict_domains,
            allowed_domain_ids: allowed_domain_ids.map(|mut ids| {
                ids.sort_unstable();
                ids.dedup();
                ids
            }),
            make_default,
        })
    }

    /// Borrow the values for the database layer.
    pub fn values(&self) -> GroupValues<'_> {
        GroupValues {
            name: &self.name,
            description: self.description.as_deref(),
            max_urls: self.max_urls,
            max_api_tokens: self.max_api_tokens,
            max_invites: self.max_invites,
            invite_period: self.invite_period.as_str(),
            allow_invites: self.allow_invites,
            allow_custom_slug: self.allow_custom_slug,
            allow_password: self.allow_password,
            allow_expiry: self.allow_expiry,
            allow_max_clicks: self.allow_max_clicks,
            allow_tags: self.allow_tags,
            allow_api_tokens: self.allow_api_tokens,
            allow_domain_selection: self.allow_domain_selection,
            default_domain_id: self.default_domain_id,
            restrict_domains: self.restrict_domains,
        }
    }
}

fn validate_quota(field: &'static str, value: Option<i64>) -> Result<Option<i64>> {
    match value {
        Some(value) if !(0..=MAX_QUOTA).contains(&value) => Err(AppError::validation(
            field,
            format!("{field} must be between 0 and {MAX_QUOTA} (0 = unlimited)"),
        )),
        other => Ok(other),
    }
}

/// A group together with its live member count.
#[derive(Debug, Clone)]
pub struct GroupOverview {
    pub group: UserGroup,
    pub members: i64,
    pub allowed_domain_ids: Vec<i64>,
}

/// Every group, default first, with member counts.
pub async fn overview(pool: &SqlitePool) -> Result<Vec<GroupOverview>> {
    let groups = user_groups::list(pool).await.map_err(AppError::from)?;
    let counts = user_groups::member_counts(pool)
        .await
        .map_err(AppError::from)?;
    let mut domain_ids = user_groups::domain_ids_by_group(pool)
        .await
        .map_err(AppError::from)?;
    Ok(groups
        .into_iter()
        .map(|group| GroupOverview {
            members: counts.get(&group.id).copied().unwrap_or(0),
            allowed_domain_ids: domain_ids.remove(&group.id).unwrap_or_default(),
            group,
        })
        .collect())
}

/// Id → name lookup, used by the admin user and invite lists.
pub async fn name_map(pool: &SqlitePool) -> Result<HashMap<i64, String>> {
    Ok(user_groups::list(pool)
        .await
        .map_err(AppError::from)?
        .into_iter()
        .map(|group| (group.id, group.name))
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn draft() -> GroupDraft {
        GroupDraft {
            name: "  Pro  ".into(),
            description: Some("  paid tier  ".into()),
            max_urls: Some(100),
            max_api_tokens: None,
            max_invites: Some(3),
            invite_period: InvitePeriod::Week,
            allow_invites: false,
            allow_custom_slug: true,
            allow_password: true,
            allow_expiry: true,
            allow_max_clicks: true,
            allow_tags: true,
            allow_api_tokens: true,
            allow_domain_selection: true,
            default_domain_id: None,
            restrict_domains: false,
            allowed_domain_ids: None,
            make_default: false,
        }
    }

    #[test]
    fn canonicalises_names_and_descriptions() {
        let draft = draft().validate().expect("valid");
        assert_eq!(draft.name, "Pro");
        assert_eq!(draft.description.as_deref(), Some("paid tier"));
    }

    #[test]
    fn rejects_names_outside_the_limits() {
        let mut too_long = draft();
        too_long.name = "x".repeat(MAX_NAME + 1);
        assert!(too_long.validate().is_err());

        let mut empty = draft();
        empty.name = "   ".into();
        assert!(empty.validate().is_err());

        let mut control = draft();
        control.name = "bad\u{7}name".into();
        assert!(control.validate().is_err());
    }

    #[test]
    fn accepts_null_and_zero_but_rejects_negative_quotas() {
        let mut free = draft();
        free.max_urls = None;
        free.max_api_tokens = Some(0);
        assert!(free.validate().is_ok());

        let mut negative = draft();
        negative.max_urls = Some(-1);
        assert!(negative.validate().is_err());

        let mut huge = draft();
        huge.max_api_tokens = Some(MAX_QUOTA + 1);
        assert!(huge.validate().is_err());

        let mut invites_negative = draft();
        invites_negative.max_invites = Some(-1);
        assert!(invites_negative.validate().is_err());

        let mut invites_free = draft();
        invites_free.max_invites = Some(0);
        invites_free.invite_period = InvitePeriod::Month;
        let validated = invites_free.validate().expect("valid");
        assert_eq!(validated.max_invites, Some(0));
        assert_eq!(validated.invite_period, InvitePeriod::Month);
    }

    #[test]
    fn empty_description_becomes_none() {
        let mut empty = draft();
        empty.description = Some("   ".into());
        assert!(empty.validate().expect("valid").description.is_none());
    }
}
