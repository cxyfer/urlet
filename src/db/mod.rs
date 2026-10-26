//! Database access layer.
//!
//! All SQL lives in this module tree; services and routes never build queries
//! inline. Queries use the runtime API (no compile time `DATABASE_URL` needed).

pub mod audit;
pub mod clicks;
pub mod custom_themes;
pub mod domains;
pub mod invite_issuance;
pub mod invites;
pub mod login_attempts;
pub mod migrate;
pub mod public_quota;
pub mod sessions;
pub mod settings;
pub mod tags;
pub mod themes;
pub mod tokens;
pub mod urls;
pub mod user_groups;
pub mod users;

use std::str::FromStr;
use std::time::Duration;

use sqlx::sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions, SqliteSynchronous};
use sqlx::{Sqlite, SqlitePool, Transaction};

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

/// Begin a transaction that writes.
///
/// sqlx opens transactions with a plain `BEGIN`, which SQLite defers: the write
/// lock is only taken at the first write. A transaction that reads before it
/// writes therefore establishes a WAL read snapshot first, and if another
/// connection commits before that write, the upgrade fails with
/// `SQLITE_BUSY_SNAPSHOT` (517). SQLite does not run the busy handler for that
/// error — the transaction can only be rolled back and retried — so
/// `busy_timeout` cannot absorb it and the caller sees a failed request. The
/// window is real in production: the click recorder and the maintenance task
/// write on their own connections while requests run, and several helpers
/// (`domains::detach_env`, `domains::ensure_canonical`, `domains::seed_env`)
/// read before they write.
///
/// `BEGIN IMMEDIATE` takes the write lock up front, which turns the lost
/// snapshot into ordinary lock contention that `busy_timeout` does wait out.
/// `migrate::apply_rebuild` uses the same statement for the same reason.
///
/// Read-only transactions should keep [`SqlitePool::begin`].
pub async fn begin_write(
    pool: &SqlitePool,
) -> std::result::Result<Transaction<'static, Sqlite>, sqlx::Error> {
    pool.begin_with("BEGIN IMMEDIATE").await
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

pub mod url_previews;

#[cfg(test)]
mod tests {
    use super::*;

    /// A two-connection pool on its own file database, so two transactions can
    /// actually contend. The short `busy_timeout` keeps a test that has to wait
    /// for the write lock quick.
    async fn contention_pool(name: &str) -> SqlitePool {
        let dir = std::env::temp_dir().join(format!("urlet-db-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("temp dir");
        let options = SqliteConnectOptions::new()
            .filename(dir.join("contention.db"))
            .create_if_missing(true)
            .journal_mode(SqliteJournalMode::Wal)
            .busy_timeout(Duration::from_millis(250));
        let pool = SqlitePoolOptions::new()
            .max_connections(2)
            .connect_with(options)
            .await
            .expect("pool");
        sqlx::query("CREATE TABLE probe (id INTEGER PRIMARY KEY, v INTEGER)")
            .execute(&pool)
            .await
            .expect("schema");
        pool
    }

    /// Extended SQLite result code, as `is_foreign_key_violation` reads it.
    fn error_code(err: &sqlx::Error) -> Option<String> {
        err.as_database_error()
            .and_then(|db| db.code())
            .map(|code| code.into_owned())
    }

    /// The failure `begin_write` exists to prevent, and the reason a longer
    /// `busy_timeout` would not have fixed it.
    ///
    /// A deferred transaction that reads first holds a WAL snapshot. Once
    /// another connection commits, the write cannot upgrade that snapshot, and
    /// SQLite answers `SQLITE_BUSY_SNAPSHOT` (517) *without running the busy
    /// handler* — the observed failure returns in tens of microseconds even
    /// though the timeout is 250 ms. Only `BEGIN IMMEDIATE`, or a retry after a
    /// rollback, can get past it.
    #[tokio::test]
    async fn a_deferred_transaction_that_reads_first_loses_its_snapshot() {
        let pool = contention_pool("deferred").await;
        let mut reader = pool.acquire().await.expect("reader");
        let mut writer = pool.acquire().await.expect("writer");

        sqlx::query("BEGIN")
            .execute(&mut *reader)
            .await
            .expect("begin");
        let _: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM probe")
            .fetch_one(&mut *reader)
            .await
            .expect("read snapshot");

        sqlx::query("INSERT INTO probe (v) VALUES (1)")
            .execute(&mut *writer)
            .await
            .expect("the competing write commits");

        let err = sqlx::query("INSERT INTO probe (v) VALUES (2)")
            .execute(&mut *reader)
            .await
            .expect_err("a stale snapshot cannot be upgraded");
        assert_eq!(error_code(&err).as_deref(), Some("517"), "{err}");
        let _ = sqlx::query("ROLLBACK").execute(&mut *reader).await;
    }

    /// The same read-then-write sequence inside a write transaction.
    #[tokio::test]
    async fn a_write_transaction_holds_the_write_lock_from_the_start() {
        let pool = contention_pool("immediate").await;
        let mut tx = begin_write(&pool).await.expect("write transaction");
        let _: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM probe")
            .fetch_one(&mut *tx)
            .await
            .expect("read snapshot");

        // The competing writer can no longer slip in between the read and the
        // write: it waits for this transaction, and the short timeout makes it
        // give up with plain `SQLITE_BUSY` instead of losing a snapshot.
        let mut writer = pool.acquire().await.expect("writer");
        let err = sqlx::query("INSERT INTO probe (v) VALUES (1)")
            .execute(&mut *writer)
            .await
            .expect_err("the competing write has to wait");
        assert_eq!(error_code(&err).as_deref(), Some("5"), "{err}");

        sqlx::query("INSERT INTO probe (v) VALUES (2)")
            .execute(&mut *tx)
            .await
            .expect("the write lock was already held");
        tx.commit().await.expect("commit");
    }
}
