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
pub mod user_groups;
pub mod users;

use std::str::FromStr;
use std::time::Duration;

use sqlx::sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions, SqliteSynchronous};
use sqlx::SqlitePool;

use crate::config::Config;
use crate::error::{AppError, Result};

/// Connection options shared by the pool and the dedicated migration
/// connection. Rebuild migrations need the same path, WAL settings and
/// `foreign_keys(true)` default as every other connection.
pub fn connect_options(config: &Config) -> Result<SqliteConnectOptions> {
    Ok(SqliteConnectOptions::from_str(&config.database_url)
        .map_err(|err| AppError::internal(format!("invalid database url: {err}")))?
        .create_if_missing(true)
        .journal_mode(SqliteJournalMode::Wal)
        .synchronous(SqliteSynchronous::Normal)
        .foreign_keys(true)
        .busy_timeout(Duration::from_secs(5))
        // Reclaim WAL disk space after checkpoints instead of keeping the
        // high-water mark forever (SQLite default is unlimited).
        .pragma("journal_size_limit", "8388608"))
}

/// Open the pool with the pragmas required for a small, reliable deployment.
pub async fn connect(config: &Config) -> Result<SqlitePool> {
    open_pool(connect_options(config)?).await
}

async fn open_pool(options: SqliteConnectOptions) -> Result<SqlitePool> {
    SqlitePoolOptions::new()
        .max_connections(8)
        .acquire_timeout(Duration::from_secs(10))
        .connect_with(options)
        .await
        .map_err(|err| AppError::internal(format!("cannot open database: {err}")))
}

/// Cheap liveness probe used by `/health`.
pub async fn ping(pool: &SqlitePool) -> bool {
    sqlx::query_scalar::<_, i64>("SELECT 1")
        .fetch_one(pool)
        .await
        .is_ok()
}

/// Checkpoint the write-ahead log, truncating it back to zero bytes.
pub async fn checkpoint_wal(pool: &SqlitePool) -> Result<()> {
    sqlx::query("PRAGMA wal_checkpoint(TRUNCATE)")
        .execute(pool)
        .await
        .map_err(|err| AppError::internal(format!("wal checkpoint failed: {err}")))?;
    Ok(())
}

/// True when SQLite rejected a write because a referenced row is missing.
///
/// Used to turn races with a concurrent delete into a domain error instead of a
/// 500: the group a request selected may disappear between the lookup and the
/// write, and the foreign key is the authoritative re-check.
pub fn is_foreign_key_violation(err: &sqlx::Error) -> bool {
    matches!(err, sqlx::Error::Database(db) if db.code().as_deref() == Some("787")
        || db.message().contains("FOREIGN KEY"))
}

/// Connect and run migrations.
pub async fn init(config: &Config) -> Result<SqlitePool> {
    let options = connect_options(config)?;
    let pool = open_pool(options.clone()).await?;
    migrate::run(&pool, &options).await?;
    Ok(pool)
}
