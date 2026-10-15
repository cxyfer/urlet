//! Shared application state handed to every handler.

use std::sync::Arc;

use sqlx::SqlitePool;
use time::OffsetDateTime;

use crate::config::Config;

#[derive(Clone)]
pub struct AppState {
    pub config: Arc<Config>,
    pub db: SqlitePool,
    pub started_at: OffsetDateTime,
}

impl AppState {
    pub fn new(config: Config, db: SqlitePool) -> Self {
        Self { config: Arc::new(config), db, started_at: OffsetDateTime::now_utc() }
    }

    pub fn uptime_seconds(&self) -> i64 {
        (OffsetDateTime::now_utc() - self.started_at).whole_seconds().max(0)
    }
}
