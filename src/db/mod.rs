//! Database access layer.
//!
//! All SQL lives in this module tree; services and routes never build queries
//! inline. Queries use the runtime API (no compile time `DATABASE_URL` needed).

pub mod audit;
pub mod clicks;
pub mod domains;
pub mod invites;
pub mod login_attempts;
pub mod migrate;
pub mod sessions;
pub mod settings;
pub mod tags;
pub mod tokens;
pub mod urls;
pub mod users;

use std::str::FromStr;
use std::time::Duration;

use sqlx::sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions, SqliteSynchronous};
use sqlx::SqlitePool;

use crate::config::Config;
use crate::error::{AppError, Result};

/// Open the pool with the pragmas required for a small, reliable deployment.
pub async fn connect(config: &Config) -> Result<SqlitePool> {
    let options = SqliteConnectOptions::from_str(&config.database_url)
        .map_err(|err| AppError::internal(format!("invalid database url: {err}")))?
        .create_if_missing(true)
        .journal_mode(SqliteJournalMode::Wal)
        .synchronous(SqliteSynchronous::Normal)
        .foreign_keys(true)
        .busy_timeout(Duration::from_secs(5))
        // Reclaim WAL disk space after checkpoints instead of keeping the
        // high-water mark forever (SQLite default is unlimited).
        .pragma("journal_size_limit", "8388608");

    SqlitePoolOptions::new()
        .max_connections(8)
        .acquire_timeout(Duration::from_secs(10))
        .connect_with(options)
        .await
        .map_err(|err| AppError::internal(format!("cannot open database: {err}")))
}

/// Connect and run migrations.
pub async fn init(config: &Config) -> Result<SqlitePool> {
    let pool = connect(config).await?;
    migrate::run(&pool).await?;
    Ok(pool)
}
