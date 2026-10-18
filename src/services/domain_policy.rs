//! Effective domain policy for one user.
//!
//! A group may set its own default domain and, optionally, restrict selection
//! to an explicit allowlist. The site-wide default remains the fallback when the
//! group has none or the selected domain was disabled or deleted.

use sqlx::{FromRow, SqlitePool};

use crate::db::domains as domain_db;
use crate::db::users::User;
use crate::error::{AppError, Result};
use crate::services::domains::{DomainOption, DomainRegistry};

/// The effective default domain and selection rules for one user.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DomainPolicy {
    /// Active default domain for new links.
    pub default_domain_id: i64,
    /// Whether a group member may choose a domain other than the default.
    pub allow_selection: bool,
    /// When true, only `allowed_domain_ids` (plus the default) may be selected.
    pub restrict_domains: bool,
    /// Active explicit allowlist; only meaningful when `restrict_domains` is true.
    pub allowed_domain_ids: Vec<i64>,
}

impl DomainPolicy {
    /// Whether a link may be created on (or moved to) this domain.
    ///
    /// The default domain is always allowed; keeping a link on its current
    /// domain is checked separately by the caller so existing links never break.
    pub fn allows(&self, domain_id: i64) -> bool {
        if domain_id == self.default_domain_id {
            return true;
        }
        if !self.allow_selection {
            return false;
        }
        if !self.restrict_domains {
            return true;
        }
        self.allowed_domain_ids.contains(&domain_id)
    }

    /// Active domains to offer in a create/edit form, with `is_default` set to
    /// the *group* default rather than the site-wide one.
    pub fn options(&self, registry: &DomainRegistry) -> Vec<DomainOption> {
        let mut options = registry.active_options();
        for option in &mut options {
            option.is_default = option.id == self.default_domain_id;
        }
        options.retain(|option| self.allows(option.id));
        if !options
            .iter()
            .any(|option| option.id == self.default_domain_id)
        {
            if let Some(hostname) = registry.hostname(self.default_domain_id) {
                options.insert(
                    0,
                    DomainOption {
                        id: self.default_domain_id,
                        hostname,
                        is_default: true,
                    },
                );
            }
        }
        options
    }

    pub fn default_hostname(&self, registry: &DomainRegistry) -> Option<String> {
        registry.hostname(self.default_domain_id)
    }

    /// Hostnames a client may use, useful for `GET /api/v1/auth/me`.
    pub fn allowed_hostnames(&self, registry: &DomainRegistry) -> Vec<String> {
        self.options(registry)
            .into_iter()
            .map(|option| option.hostname)
            .collect()
    }
}

#[derive(Debug, FromRow)]
struct GroupDomainRow {
    default_domain_id: Option<i64>,
    allow_domain_selection: bool,
    restrict_domains: bool,
}

/// Resolve the policy for `user`; ungrouped users get the site default, every
/// active domain and selection enabled.
pub async fn effective(pool: &SqlitePool, user: &User) -> Result<DomainPolicy> {
    let global = domain_db::active_default(pool)
        .await
        .map_err(AppError::from)?
        .ok_or_else(|| AppError::internal("no active default domain"))?;
    let defaults = DomainPolicy {
        default_domain_id: global.id,
        allow_selection: true,
        restrict_domains: false,
        allowed_domain_ids: Vec::new(),
    };

    let Some(group_id) = user.group_id else {
        return Ok(defaults);
    };
    let group = sqlx::query_as::<_, GroupDomainRow>(
        "SELECT default_domain_id, allow_domain_selection, restrict_domains
           FROM user_groups WHERE id = ?",
    )
    .bind(group_id)
    .fetch_optional(pool)
    .await
    .map_err(AppError::from)?;
    let Some(group) = group else {
        // The group vanished after the user row was loaded; keep the request
        // useful with the site defaults rather than failing.
        return Ok(defaults);
    };

    let default_domain_id = match group.default_domain_id {
        Some(id) => domain_db::find_by_id(pool, id)
            .await
            .map_err(AppError::from)?
            .filter(|domain| domain.is_active())
            .map(|domain| domain.id),
        None => None,
    }
    .unwrap_or(global.id);

    let allowed_domain_ids = if group.restrict_domains {
        sqlx::query_scalar(
            "SELECT gd.domain_id
               FROM group_domains gd
               JOIN domains d ON d.id = gd.domain_id
              WHERE gd.group_id = ? AND d.status = 'active'
              ORDER BY gd.domain_id",
        )
        .bind(group_id)
        .fetch_all(pool)
        .await
        .map_err(AppError::from)?
    } else {
        Vec::new()
    };

    Ok(DomainPolicy {
        default_domain_id,
        allow_selection: group.allow_domain_selection,
        restrict_domains: group.restrict_domains,
        allowed_domain_ids,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn policy(allow_selection: bool, restrict_domains: bool) -> DomainPolicy {
        DomainPolicy {
            default_domain_id: 1,
            allow_selection,
            restrict_domains,
            allowed_domain_ids: vec![2, 3],
        }
    }

    #[test]
    fn the_default_domain_is_always_allowed() {
        let locked = policy(false, true);
        assert!(locked.allows(1));
        assert!(!locked.allows(2));
    }

    #[test]
    fn restriction_only_narrows_when_enabled() {
        let open = policy(true, false);
        assert!(open.allows(2));
        assert!(open.allows(99));

        let restricted = policy(true, true);
        assert!(restricted.allows(2));
        assert!(restricted.allows(3));
        assert!(!restricted.allows(99));
    }

    #[test]
    fn an_empty_restricted_list_still_allows_only_the_default() {
        let restricted = DomainPolicy {
            default_domain_id: 1,
            allow_selection: true,
            restrict_domains: true,
            allowed_domain_ids: vec![],
        };
        assert!(restricted.allows(1));
        assert!(!restricted.allows(2));
    }
}
