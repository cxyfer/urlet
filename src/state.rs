//! Shared application state handed to every handler.

use std::sync::Arc;

use sqlx::SqlitePool;
use time::OffsetDateTime;

use crate::config::Config;
use crate::services::click::ClickRecorder;
use crate::services::domains::DomainRegistry;
use crate::services::ratelimit::RateLimiter;
use crate::services::settings::Settings;

#[derive(Clone)]
pub struct AppState {
    pub config: Arc<Config>,
    pub db: SqlitePool,
    pub settings: Arc<Settings>,
    pub domains: Arc<DomainRegistry>,
    pub limiter: Arc<RateLimiter>,
    pub clicks: ClickRecorder,
    pub started_at: OffsetDateTime,
}

impl AppState {
    pub fn new(config: Config, db: SqlitePool, settings: Settings) -> Self {
        let config = Arc::new(config);
        let settings = Arc::new(settings);
        let domains = Arc::new(DomainRegistry::new(&config.base_host));
        let clicks = ClickRecorder::start(
            db.clone(),
            config.clone(),
            settings.clone(),
            domains.clone(),
            crate::services::click::QUEUE_CAPACITY,
        );
        Self {
            config,
            db,
            settings,
            domains,
            limiter: Arc::new(RateLimiter::new()),
            clicks,
            started_at: OffsetDateTime::now_utc(),
        }
    }

    /// Hostname of a link's domain, falling back to the canonical host for
    /// legacy rows that somehow carry an unknown id.
    pub fn domain_hostname(&self, domain_id: i64) -> String {
        self.domains
            .hostname(domain_id)
            .unwrap_or_else(|| self.config.base_host.clone())
    }

    pub fn uptime_seconds(&self) -> i64 {
        (OffsetDateTime::now_utc() - self.started_at)
            .whole_seconds()
            .max(0)
    }
}
