//! Click recording.
//!
//! Recording happens off the redirect path: the response is written first and
//! the work is queued for a single background writer.
//!
//! A single writer (instead of one task per click) matters for SQLite: writes
//! are serialised anyway, and concurrent writers starve the automatic WAL
//! checkpoint, which makes the `-wal` file grow without bound.

use std::sync::Arc;

use sqlx::SqlitePool;
use tokio::sync::mpsc;

use crate::config::Config;
use crate::db::clicks::{self, NewClick};
use crate::services::settings::Settings;
use crate::services::{secret, time, ua};

/// Maximum number of queued clicks before new ones are dropped.
///
/// Analytics are best effort: dropping under extreme load is preferable to
/// growing the queue without bound or slowing redirects down.
pub const QUEUE_CAPACITY: usize = 10_000;

/// Everything needed to record one click.
#[derive(Debug, Clone)]
pub struct ClickInput {
    pub url_id: i64,
    pub ip_hash: Option<String>,
    pub user_agent: Option<String>,
    pub referrer: Option<String>,
    pub password_protected: bool,
}

/// Handle to the background click writer.
#[derive(Debug, Clone)]
pub struct ClickRecorder {
    sender: mpsc::Sender<ClickInput>,
}

impl ClickRecorder {
    /// Start the writer task. It stops when every clone of the recorder is gone.
    pub fn start(
        pool: SqlitePool,
        config: Arc<Config>,
        settings: Arc<Settings>,
        capacity: usize,
    ) -> Self {
        let (sender, mut receiver) = mpsc::channel::<ClickInput>(capacity.max(1));
        tokio::spawn(async move {
            while let Some(input) = receiver.recv().await {
                if !settings.analytics_enabled() {
                    continue;
                }
                if let Err(err) = record_inner(&pool, &config, &input).await {
                    tracing::warn!(error = %err, url_id = input.url_id, "click recording failed");
                }
            }
        });
        Self { sender }
    }

    /// Queue a click. Never blocks and never fails the caller.
    pub fn record(&self, input: ClickInput) {
        match self.sender.try_send(input) {
            Ok(()) => {}
            Err(mpsc::error::TrySendError::Full(dropped)) => {
                tracing::warn!(
                    url_id = dropped.url_id,
                    "click queue is full, dropping click"
                );
            }
            Err(mpsc::error::TrySendError::Closed(_)) => {
                tracing::debug!("click writer stopped, dropping click");
            }
        }
    }
}

/// Record a click synchronously, swallowing errors.
///
/// Used by tests that want deterministic timing.
pub async fn record(
    pool: &SqlitePool,
    config: &Config,
    analytics_enabled: bool,
    input: ClickInput,
) {
    if !analytics_enabled {
        return;
    }
    if let Err(err) = record_inner(pool, config, &input).await {
        tracing::warn!(error = %err, url_id = input.url_id, "click recording failed");
    }
}

async fn record_inner(
    pool: &SqlitePool,
    config: &Config,
    input: &ClickInput,
) -> Result<(), sqlx::Error> {
    let now = time::now();
    let clicked_at = time::to_iso(now);
    let day = time::to_day(now);

    let agent = ua::parse(input.user_agent.as_deref());
    let referrer_host = ua::referrer_host(
        input.referrer.as_deref(),
        &[config.base_host.as_str(), "localhost", "127.0.0.1"],
    );

    // The visitor hash rotates daily and never contains the raw address.
    let visitor_hash = input.ip_hash.as_ref().map(|ip_hash| {
        let material = format!(
            "{ip_hash}|{}|{}",
            input.user_agent.as_deref().unwrap_or(""),
            day
        );
        secret::hmac_hex(&config.secret, &material)
    });

    let is_unique = match &visitor_hash {
        Some(hash) => !clicks::visitor_seen(pool, input.url_id, hash).await?,
        None => true,
    };

    clicks::insert(
        pool,
        &NewClick {
            url_id: input.url_id,
            clicked_at: clicked_at.clone(),
            ip_hash: input.ip_hash.clone(),
            visitor_hash,
            user_agent: input
                .user_agent
                .as_deref()
                .map(|agent| agent.chars().take(512).collect()),
            referrer: input
                .referrer
                .as_deref()
                .map(|value| value.chars().take(512).collect()),
            referrer_host,
            device_type: agent.device_type.to_string(),
            browser: agent.browser.to_string(),
            os: agent.os.to_string(),
            is_bot: agent.is_bot,
            is_unique,
            password_protected: input.password_protected,
        },
    )
    .await?;

    clicks::bump_daily(pool, input.url_id, &day, is_unique, agent.is_bot).await?;
    clicks::bump_url_counters(pool, input.url_id, &clicked_at, is_unique).await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn click_input_is_cloneable_for_spawning() {
        let input = ClickInput {
            url_id: 1,
            ip_hash: Some("hash".into()),
            user_agent: Some("curl/8".into()),
            referrer: None,
            password_protected: false,
        };
        let cloned = input.clone();
        assert_eq!(cloned.url_id, 1);
    }
}
