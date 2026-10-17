//! Domain validation, normalisation and the in-process domain registry.

use std::collections::HashMap;
use std::sync::{Arc, RwLock};

use sqlx::SqlitePool;

use crate::db::domains::{self as domain_db, source, status, Domain, DomainWithCount, NewDomain};
use crate::db::users::User;
use crate::error::{AppError, Result as AppResult};
use crate::services::audit::{action, target, Audit};

/// Why a hostname was rejected.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HostError {
    Empty,
    TooLong,
    InvalidCharacter,
    InvalidLabel,
    InvalidPort,
    Ipv6Unsupported,
}

impl HostError {
    /// User facing, already translated by the UI catalogue where needed.
    pub fn message(self) -> &'static str {
        match self {
            Self::Empty => "enter a domain such as links.example.com",
            Self::TooLong => "the domain is too long",
            Self::InvalidCharacter => {
                "use only the hostname, for example links.example.com (no scheme or path)"
            }
            Self::InvalidLabel => "the domain contains an invalid label",
            Self::InvalidPort => "the port must be between 1 and 65535",
            Self::Ipv6Unsupported => "IPv6 literals are not supported; use a hostname",
        }
    }
}

/// Normalise a request `Host` header or a configured hostname.
///
/// Rules:
///   * trim, lower-case, strip a trailing dot;
///   * strip `:80` and `:443`, keep any other port;
///   * accept ASCII DNS labels and IPv4 addresses;
///   * reject schemes, paths, userinfo, whitespace, control characters and
///     IPv6 literals (the deployment only needs DNS names and `localhost`).
pub fn normalize_host(raw: &str) -> Option<String> {
    parse_host(raw).ok()
}

/// Strict version used by the admin form and `URLET_DOMAINS`.
pub fn validate_domain_hostname(raw: &str) -> std::result::Result<String, HostError> {
    parse_host(raw)
}

fn parse_host(raw: &str) -> std::result::Result<String, HostError> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err(HostError::Empty);
    }
    if trimmed.len() > 300 {
        return Err(HostError::TooLong);
    }
    if trimmed.contains("://")
        || trimmed.contains('/')
        || trimmed.contains('\\')
        || trimmed.contains('@')
        || trimmed.chars().any(|c| c.is_control() || c.is_whitespace())
    {
        return Err(HostError::InvalidCharacter);
    }

    let lowered = trimmed.to_ascii_lowercase();
    if lowered.starts_with('[') {
        return Err(HostError::Ipv6Unsupported);
    }

    let (host, port) = match lowered.rsplit_once(':') {
        Some((host, port)) if !host.contains(':') => {
            if port.is_empty() || !port.chars().all(|c| c.is_ascii_digit()) {
                return Err(HostError::InvalidPort);
            }
            (host, Some(port))
        }
        Some(_) => return Err(HostError::Ipv6Unsupported),
        None => (lowered.as_str(), None),
    };

    let host = host.trim_end_matches('.');
    if host.is_empty() {
        return Err(HostError::Empty);
    }
    if host.len() > 253 {
        return Err(HostError::TooLong);
    }

    if host != "localhost" {
        if host.starts_with('.') || host.ends_with('.') || host.contains("..") {
            return Err(HostError::InvalidLabel);
        }
        for label in host.split('.') {
            if label.is_empty() || label.len() > 63 {
                return Err(HostError::InvalidLabel);
            }
            if label.starts_with('-') || label.ends_with('-') {
                return Err(HostError::InvalidLabel);
            }
            if !label.chars().all(|c| c.is_ascii_alphanumeric() || c == '-') {
                return Err(HostError::InvalidLabel);
            }
        }
    }

    let port = match port {
        None => None,
        Some(value) => {
            let parsed = value.parse::<u16>().map_err(|_| HostError::InvalidPort)?;
            if parsed == 0 {
                return Err(HostError::InvalidPort);
            }
            // The instance uses one scheme; the default ports are implied.
            if parsed == 80 || parsed == 443 {
                None
            } else {
                Some(parsed)
            }
        }
    };

    Ok(match port {
        Some(port) => format!("{host}:{port}"),
        None => host.to_string(),
    })
}

/// Domain management for the admin UI and API.
pub struct DomainContext<'a> {
    pub pool: &'a SqlitePool,
    pub actor: &'a User,
    pub ip_hash: Option<&'a str>,
}

impl DomainContext<'_> {
    pub async fn list(&self) -> AppResult<Vec<DomainWithCount>> {
        domain_db::list_with_counts(self.pool)
            .await
            .map_err(AppError::from)
    }

    /// Add a domain in the `pending` state.
    pub async fn create(&self, raw_hostname: &str) -> AppResult<Domain> {
        let hostname = validate_domain_hostname(raw_hostname)
            .map_err(|err| AppError::validation("domain", err.message()))?;
        if domain_db::find_by_hostname(self.pool, &hostname)
            .await
            .map_err(AppError::from)?
            .is_some()
        {
            return Err(AppError::conflict(format!(
                "the domain '{hostname}' is already configured"
            )));
        }

        let domain = domain_db::create(
            self.pool,
            &NewDomain {
                hostname: hostname.clone(),
                source: source::UI,
                status: status::PENDING,
                verified: false,
                created_by: Some(self.actor.id),
            },
        )
        .await
        .map_err(|err| {
            if is_unique_violation(&err) {
                AppError::conflict(format!("the domain '{hostname}' is already configured"))
            } else {
                AppError::from(err)
            }
        })?;

        self.audit(action::DOMAIN_CREATED, &domain, serde_json::json!({}))
            .await;
        Ok(domain)
    }

    /// Activate a pending or disabled domain after DNS/TLS is ready.
    pub async fn activate(&self, id: i64) -> AppResult<Domain> {
        let domain = self.get(id).await?;
        if domain.is_active() {
            return Ok(domain);
        }
        domain_db::activate(self.pool, id)
            .await
            .map_err(AppError::from)?;
        let domain = self.get(id).await?;
        self.audit(
            action::DOMAIN_UPDATED,
            &domain,
            serde_json::json!({ "status": status::ACTIVE }),
        )
        .await;
        Ok(domain)
    }

    /// Disable a domain. The canonical host and the current default are
    /// protected; the administrator has to choose another default first.
    pub async fn disable(&self, id: i64) -> AppResult<Domain> {
        let domain = self.get(id).await?;
        if domain.is_system() {
            return Err(AppError::conflict(
                "the canonical domain cannot be disabled",
            ));
        }
        if domain.is_default() {
            return Err(AppError::conflict(
                "choose another default domain before disabling this one",
            ));
        }
        if domain.is_disabled() {
            return Ok(domain);
        }

        let changed = domain_db::disable(self.pool, id)
            .await
            .map_err(AppError::from)?;
        if changed == 0 {
            return self.disabled_race_result(id).await;
        }

        let domain = self.get(id).await?;
        self.audit(
            action::DOMAIN_UPDATED,
            &domain,
            serde_json::json!({ "status": status::DISABLED }),
        )
        .await;
        Ok(domain)
    }

    /// Make an active domain the single default.
    pub async fn set_default(&self, id: i64) -> AppResult<Domain> {
        let domain = self.get(id).await?;
        if !domain.is_active() {
            return Err(AppError::validation(
                "domain",
                "only an active domain can be the default",
            ));
        }
        domain_db::set_default(self.pool, id)
            .await
            .map_err(AppError::from)?;
        let domain = self.get(id).await?;
        self.audit(
            action::DOMAIN_DEFAULT_CHANGED,
            &domain,
            serde_json::json!({ "is_default": true }),
        )
        .await;
        Ok(domain)
    }

    /// Delete a UI-owned domain that has no links. System and environment
    /// domains are re-seeded on boot, so they can only be disabled.
    pub async fn delete(&self, id: i64) -> AppResult<()> {
        let domain = self.get(id).await?;
        if domain.is_system() {
            return Err(AppError::conflict("the canonical domain cannot be deleted"));
        }
        if domain.is_env() {
            return Err(AppError::conflict(
                "this domain is managed by URLET_DOMAINS; remove it there instead",
            ));
        }
        if domain.is_default() {
            return Err(AppError::conflict(
                "choose another default domain before deleting this one",
            ));
        }
        let links = domain_db::link_count(self.pool, id)
            .await
            .map_err(AppError::from)?;
        if links > 0 {
            return Err(AppError::conflict(format!(
                "this domain still has {links} link(s); disable it instead"
            )));
        }

        let deleted = domain_db::delete(self.pool, id).await;
        match deleted {
            Ok(0) => self.deleted_race_result(id).await,
            Ok(_) => {
                self.audit(action::DOMAIN_DELETED, &domain, serde_json::json!({}))
                    .await;
                Ok(())
            }
            Err(err) if is_foreign_key_violation(&err) => Err(AppError::conflict(
                "this domain still has link(s); disable it instead",
            )),
            Err(err) => Err(AppError::from(err)),
        }
    }

    /// Explain a conditional disable that affected 0 rows.
    async fn disabled_race_result(&self, id: i64) -> AppResult<Domain> {
        let current = self.get(id).await?;
        if current.is_system() {
            return Err(AppError::conflict(
                "the canonical domain cannot be disabled",
            ));
        }
        if current.is_default() {
            return Err(AppError::conflict(
                "choose another default domain before disabling this one",
            ));
        }
        if current.is_disabled() {
            return Ok(current);
        }
        Err(AppError::conflict(
            "the domain changed while it was being disabled; try again",
        ))
    }

    /// Explain a conditional delete that affected 0 rows.
    async fn deleted_race_result(&self, id: i64) -> AppResult<()> {
        let current = self.get(id).await?;
        if current.is_system() {
            return Err(AppError::conflict("the canonical domain cannot be deleted"));
        }
        if current.is_env() {
            return Err(AppError::conflict(
                "this domain is managed by URLET_DOMAINS; remove it there instead",
            ));
        }
        if current.is_default() {
            return Err(AppError::conflict(
                "choose another default domain before deleting this one",
            ));
        }
        Err(AppError::conflict(
            "the domain changed while it was being deleted; try again",
        ))
    }

    async fn get(&self, id: i64) -> AppResult<Domain> {
        domain_db::find_by_id(self.pool, id)
            .await
            .map_err(AppError::from)?
            .ok_or_else(AppError::not_found)
    }

    async fn audit(&self, action: &str, domain: &Domain, metadata: serde_json::Value) {
        let mut metadata = metadata;
        if let Some(object) = metadata.as_object_mut() {
            object.insert(
                "hostname".to_string(),
                serde_json::Value::String(domain.hostname.clone()),
            );
        }
        Audit::new(action)
            .actor(self.actor.id, &self.actor.role)
            .target(target::DOMAIN, domain.id)
            .metadata(metadata)
            .ip_hash(self.ip_hash.map(str::to_string))
            .record(self.pool)
            .await;
    }
}

fn is_unique_violation(err: &sqlx::Error) -> bool {
    matches!(err, sqlx::Error::Database(db) if db.code().as_deref() == Some("2067"))
}

fn is_foreign_key_violation(err: &sqlx::Error) -> bool {
    matches!(err, sqlx::Error::Database(db) if db.code().as_deref() == Some("787"))
}

/// One domain as exposed to the create/edit form.
#[derive(Debug, Clone)]
pub struct DomainOption {
    pub hostname: String,
    pub is_default: bool,
}

impl From<&Domain> for DomainOption {
    fn from(domain: &Domain) -> Self {
        Self {
            hostname: domain.hostname.clone(),
            is_default: domain.is_default(),
        }
    }
}

/// Immutable view of the domain table used on the hot path.
#[derive(Debug, Clone, Default)]
pub struct DomainSnapshot {
    by_id: HashMap<i64, Domain>,
    by_host: HashMap<String, Domain>,
    default_id: Option<i64>,
    active_hosts: Vec<String>,
    active_domains: Vec<Domain>,
}

impl DomainSnapshot {
    fn from_domains(mut domains: Vec<Domain>) -> Self {
        domains.sort_by(|a, b| {
            b.is_default()
                .cmp(&a.is_default())
                .then_with(|| a.hostname.cmp(&b.hostname))
        });

        let mut by_id = HashMap::with_capacity(domains.len());
        let mut by_host = HashMap::with_capacity(domains.len());
        let mut default_id = None;
        let mut active_hosts = Vec::new();
        let mut active_domains = Vec::new();

        for domain in domains {
            if domain.is_default() {
                default_id = Some(domain.id);
            }
            if domain.is_active() {
                active_hosts.push(domain.hostname.clone());
                active_domains.push(domain.clone());
            }
            by_host.insert(domain.hostname.clone(), domain.clone());
            by_id.insert(domain.id, domain);
        }
        active_hosts.sort();

        Self {
            by_id,
            by_host,
            default_id,
            active_hosts,
            active_domains,
        }
    }
}

/// In-process domain cache.
///
/// The deployment is single-instance SQLite, so a process-local snapshot is
/// enough. `reload` is called during bootstrap and after every admin mutation.
#[derive(Debug)]
pub struct DomainRegistry {
    canonical_host: String,
    snapshot: RwLock<Arc<DomainSnapshot>>,
}

impl DomainRegistry {
    pub fn new(canonical_host: &str) -> Self {
        let canonical_host = normalize_host(canonical_host).unwrap_or_else(|| {
            canonical_host
                .trim()
                .trim_end_matches('.')
                .to_ascii_lowercase()
        });
        Self {
            canonical_host,
            snapshot: RwLock::new(Arc::new(DomainSnapshot::default())),
        }
    }

    pub fn canonical_host(&self) -> &str {
        &self.canonical_host
    }

    pub fn is_canonical(&self, host: &str) -> bool {
        normalize_host(host)
            .as_deref()
            .map(|host| host == self.canonical_host)
            .unwrap_or(false)
    }

    fn current(&self) -> Arc<DomainSnapshot> {
        self.snapshot
            .read()
            .expect("domain registry lock poisoned")
            .clone()
    }

    /// Replace the snapshot. Public for tests and for bootstrap after seeding.
    pub fn replace(&self, domains: Vec<Domain>) {
        let snapshot = DomainSnapshot::from_domains(domains);
        *self
            .snapshot
            .write()
            .expect("domain registry lock poisoned") = Arc::new(snapshot);
    }

    /// Reload every domain from the database.
    pub async fn reload(&self, pool: &SqlitePool) -> Result<(), sqlx::Error> {
        let domains = domain_db::all(pool).await?;
        self.replace(domains);
        Ok(())
    }

    /// Lookup a hostname in any lifecycle state.
    pub fn lookup(&self, host: &str) -> Option<Domain> {
        self.current().by_host.get(host).cloned()
    }

    pub fn get(&self, id: i64) -> Option<Domain> {
        self.current().by_id.get(&id).cloned()
    }

    pub fn hostname(&self, id: i64) -> Option<String> {
        self.current()
            .by_id
            .get(&id)
            .map(|domain| domain.hostname.clone())
    }

    pub fn default_id(&self) -> Option<i64> {
        self.current().default_id
    }

    pub fn default_domain(&self) -> Option<Domain> {
        self.default_id().and_then(|id| self.get(id))
    }

    pub fn active_domains(&self) -> Vec<Domain> {
        self.current().active_domains.clone()
    }

    pub fn active_options(&self) -> Vec<DomainOption> {
        self.active_domains()
            .iter()
            .map(DomainOption::from)
            .collect()
    }

    /// Every active hostname, used for cookie/token binding and referrer
    /// self-exclusion.
    pub fn active_hosts(&self) -> Vec<String> {
        self.current().active_hosts.clone()
    }

    pub fn is_active_host(&self, host: &str) -> bool {
        self.lookup(host)
            .map(|domain| domain.is_active())
            .unwrap_or(false)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalises_hostnames() {
        assert_eq!(
            normalize_host("Example.COM.").as_deref(),
            Some("example.com")
        );
        assert_eq!(
            normalize_host("example.com:443").as_deref(),
            Some("example.com")
        );
        assert_eq!(
            normalize_host("example.com:80").as_deref(),
            Some("example.com")
        );
        assert_eq!(
            normalize_host("localhost:8080").as_deref(),
            Some("localhost:8080")
        );
        assert_eq!(
            normalize_host("  links.example.com  ").as_deref(),
            Some("links.example.com")
        );
    }

    #[test]
    fn rejects_bad_hostnames() {
        for bad in [
            "",
            "https://example.com",
            "example.com/path",
            "user@example.com",
            "bad host",
            "bad_host",
            "-bad.example.com",
            "bad-.example.com",
            "example.com:0",
            "example.com:70000",
            "example.com:",
            "[::1]:8080",
        ] {
            assert!(normalize_host(bad).is_none(), "{bad} should be rejected");
        }
    }

    #[test]
    fn validates_domains_for_the_admin_form() {
        assert_eq!(
            validate_domain_hostname("Links.Example.com").expect("valid"),
            "links.example.com"
        );
        assert_eq!(
            validate_domain_hostname("localhost:8080").expect("valid"),
            "localhost:8080"
        );
        assert_eq!(
            validate_domain_hostname("https://x.example").unwrap_err(),
            HostError::InvalidCharacter
        );
    }

    #[test]
    fn snapshot_lookup_and_default() {
        let mut domain = Domain {
            id: 1,
            hostname: "s.example.com".into(),
            is_default: 1,
            is_verified: 1,
            created_by: None,
            created_at: "2026-01-01T00:00:00.000Z".into(),
            status: status::ACTIVE.into(),
            source: source::SYSTEM.into(),
            updated_at: None,
        };
        let registry = DomainRegistry::new("s.example.com");
        registry.replace(vec![domain.clone()]);
        assert_eq!(registry.default_id(), Some(1));
        assert!(registry.is_canonical("S.EXAMPLE.COM."));
        assert!(registry.is_active_host("s.example.com"));

        domain.id = 2;
        domain.hostname = "b.example.com".into();
        domain.is_default = 0;
        registry.replace(vec![domain]);
        assert_eq!(registry.default_id(), None);
        assert_eq!(registry.active_hosts(), vec!["b.example.com"]);
    }
}
