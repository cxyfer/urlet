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

use std::borrow::Cow;
use std::str::FromStr;
use std::time::Duration;

use sqlx::any::{AnyConnectOptions, AnyPoolOptions, AnyTypeInfo, AnyTypeInfoKind, AnyValueRef};
use sqlx::sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions, SqliteSynchronous};
use sqlx::{Any, AnyConnection, AnyPool, SqlitePool, ValueRef};

use crate::config::Config;
use crate::error::{AppError, Result};

/// The application's connection pool, backed by SQLite or PostgreSQL
/// depending on the scheme of `URLET_DATABASE_URL` (FR-DB-1).
pub type Pool = AnyPool;

/// One connection checked out of [`Pool`].
pub type Connection = AnyConnection;

/// A transaction on [`Pool`].
pub type Transaction = sqlx::Transaction<'static, Any>;

/// Advisory-lock key that serialises PostgreSQL write transactions
/// (FR-DB-6). Any stable 64-bit value works; this one spells `urlet-wr`.
const WRITE_LOCK_KEY: i64 = 0x75726c65_742d7772;

/// The few places where SQL differs between the two backends.
///
/// Everything else is written once under the portable rules of SPEC §6.6.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Dialect {
    Sqlite,
    Postgres,
}

impl Dialect {
    /// Backend named by a database URL, or `None` for an unsupported scheme.
    pub fn from_url(url: &str) -> Option<Self> {
        let scheme = url.split_once(':')?.0.to_ascii_lowercase();
        match scheme.as_str() {
            "sqlite" => Some(Self::Sqlite),
            "postgres" | "postgresql" => Some(Self::Postgres),
            _ => None,
        }
    }

    /// Backend behind a pool.
    pub fn of(pool: &Pool) -> Self {
        Self::from_url(pool.connect_options().database_url.as_str()).unwrap_or(Self::Sqlite)
    }

    /// Backend behind a single connection.
    pub fn of_connection(connection: &Connection) -> Self {
        if connection.backend_name().eq_ignore_ascii_case("postgresql") {
            Self::Postgres
        } else {
            Self::Sqlite
        }
    }

    /// ASCII-case-insensitive `LIKE` on `column` with a `\`-escaped pattern.
    ///
    /// SQLite's `LIKE` already folds ASCII case. PostgreSQL's does not, and
    /// pattern matching is unavailable on the nondeterministic `nocase`
    /// collation, so it compares under `"C"` with `ILIKE` (SPEC §6.6).
    pub fn ci_like(self, column: &str, pattern: &str) -> String {
        match self {
            Self::Sqlite => format!("{column} LIKE {pattern} ESCAPE '\\'"),
            Self::Postgres => format!("{column} COLLATE \"C\" ILIKE {pattern} ESCAPE '\\'"),
        }
    }
}

/// A 0/1 flag column or a boolean expression, decoded from either an integer
/// or a boolean.
///
/// The `Any` driver decodes `bool` only from a boolean value, while SQLite
/// reports `INTEGER` flags and boolean expressions as integers and PostgreSQL
/// reports expressions such as `EXISTS` as booleans. Use it with
/// `#[sqlx(try_from = "crate::db::Flag")]` on `bool` fields, or directly as a
/// scalar type.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Flag(pub bool);

impl From<Flag> for bool {
    fn from(flag: Flag) -> Self {
        flag.0
    }
}

impl sqlx::Type<Any> for Flag {
    fn type_info() -> AnyTypeInfo {
        <i64 as sqlx::Type<Any>>::type_info()
    }

    fn compatible(ty: &AnyTypeInfo) -> bool {
        ty.kind().is_integer() || ty.kind() == AnyTypeInfoKind::Bool
    }
}

impl<'r> sqlx::Decode<'r, Any> for Flag {
    fn decode(value: AnyValueRef<'r>) -> std::result::Result<Self, sqlx::error::BoxDynError> {
        if value.type_info().kind() == AnyTypeInfoKind::Bool {
            return <bool as sqlx::Decode<Any>>::decode(value).map(Flag);
        }
        <i64 as sqlx::Decode<Any>>::decode(value).map(|v| Flag(v != 0))
    }
}

/// Bind value for a 0/1 flag column. PostgreSQL will not store a boolean
/// parameter in an integer column, so flags are always bound as integers.
pub fn flag(value: bool) -> i64 {
    i64::from(value)
}

/// Connection options for SQLite, shared by the migration runner. Rebuild
/// migrations need the same path, WAL settings and `foreign_keys(true)`
/// default as every other connection.
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

/// The pragmas [`connect_options`] sets, for connections opened through the
/// `Any` driver, which only knows the URL. sqlx's defaults already cover
/// `foreign_keys` and `busy_timeout`; they are repeated so the two paths are
/// visibly the same.
const SQLITE_PRAGMAS: &[&str] = &[
    "PRAGMA journal_mode = WAL",
    "PRAGMA synchronous = NORMAL",
    "PRAGMA foreign_keys = ON",
    "PRAGMA busy_timeout = 5000",
    "PRAGMA journal_size_limit = 8388608",
];

async fn apply_sqlite_pragmas(
    connection: &mut AnyConnection,
) -> std::result::Result<(), sqlx::Error> {
    for pragma in SQLITE_PRAGMAS {
        sqlx::query(pragma).execute(&mut *connection).await?;
    }
    Ok(())
}

/// The URL the `Any` driver opens. A SQLite URL without `mode` gets
/// `mode=rwc`, so the file is still created on first start as it is through
/// [`connect_options`].
fn any_url(config: &Config) -> String {
    let url = config.database_url.as_str();
    if Dialect::from_url(url) != Some(Dialect::Sqlite) {
        return url.to_string();
    }
    let has_mode = url
        .split_once('?')
        .is_some_and(|(_, query)| query.split('&').any(|pair| pair.starts_with("mode=")));
    if has_mode {
        url.to_string()
    } else if url.contains('?') {
        format!("{url}&mode=rwc")
    } else {
        format!("{url}?mode=rwc")
    }
}

/// Open the application pool for whichever backend the URL names.
pub async fn connect(config: &Config) -> Result<Pool> {
    sqlx::any::install_default_drivers();
    let dialect = Dialect::from_url(&config.database_url)
        .ok_or_else(|| AppError::internal("unsupported database url scheme"))?;
    let options = AnyConnectOptions::from_str(&any_url(config))
        .map_err(|err| AppError::internal(format!("invalid database url: {err}")))?;
    let mut pool = AnyPoolOptions::new()
        .max_connections(8)
        .acquire_timeout(Duration::from_secs(10));
    if dialect == Dialect::Sqlite {
        pool = pool.after_connect(|connection, _| Box::pin(apply_sqlite_pragmas(connection)));
    }
    pool.connect_with(options)
        .await
        .map_err(|err| AppError::internal(format!("cannot open database: {err}")))
}

/// Native SQLite pool for the migration runner.
async fn open_sqlite_pool(options: SqliteConnectOptions) -> Result<SqlitePool> {
    SqlitePoolOptions::new()
        .max_connections(8)
        .acquire_timeout(Duration::from_secs(10))
        .connect_with(options)
        .await
        .map_err(|err| AppError::internal(format!("cannot open database: {err}")))
}

/// Begin a transaction that writes.
///
/// **SQLite.** sqlx opens transactions with a plain `BEGIN`, which SQLite
/// defers: the write lock is only taken at the first write. A transaction that
/// reads before it writes therefore establishes a WAL read snapshot first, and
/// if another connection commits before that write, the upgrade fails with
/// `SQLITE_BUSY_SNAPSHOT` (517). SQLite does not run the busy handler for that
/// error — the transaction can only be rolled back and retried — so
/// `busy_timeout` cannot absorb it and the caller sees a failed request. The
/// window is real in production: the click recorder and the maintenance task
/// write on their own connections while requests run, and several helpers
/// (`domains::detach_env`, `domains::ensure_canonical`, `domains::seed_env`)
/// read before they write. `BEGIN IMMEDIATE` takes the write lock up front,
/// which turns the lost snapshot into ordinary lock contention that
/// `busy_timeout` does wait out. `migrate::apply_rebuild` uses the same
/// statement for the same reason.
///
/// **PostgreSQL.** `READ COMMITTED`, whatever the server default, plus a
/// transaction-scoped advisory lock, so write transactions run one at a time
/// exactly as they do on SQLite and every read-then-write check inside one
/// keeps its guarantee (FR-DB-6). `lock_timeout` gives up after five seconds,
/// like SQLite's `busy_timeout`, instead of letting waiters hold the pool.
///
/// Read-only transactions should keep [`Pool::begin`].
pub async fn begin_write(pool: &Pool) -> std::result::Result<Transaction, sqlx::Error> {
    match Dialect::of(pool) {
        Dialect::Sqlite => pool.begin_with("BEGIN IMMEDIATE").await,
        Dialect::Postgres => {
            let mut tx = pool
                .begin_with("BEGIN ISOLATION LEVEL READ COMMITTED")
                .await?;
            sqlx::query("SET LOCAL lock_timeout = '5s'")
                .execute(&mut *tx)
                .await?;
            sqlx::query("SELECT pg_advisory_xact_lock($1)")
                .bind(WRITE_LOCK_KEY)
                .execute(&mut *tx)
                .await?;
            Ok(tx)
        }
    }
}

/// Cheap liveness probe used by `/health`.
pub async fn ping(pool: &Pool) -> bool {
    sqlx::query_scalar::<_, i64>("SELECT 1")
        .fetch_one(pool)
        .await
        .is_ok()
}

/// Checkpoint the SQLite write-ahead log, truncating it back to zero bytes.
/// PostgreSQL has no equivalent to run; autovacuum maintains it.
pub async fn checkpoint_wal(pool: &Pool) -> Result<()> {
    if Dialect::of(pool) == Dialect::Postgres {
        return Ok(());
    }
    sqlx::query("PRAGMA wal_checkpoint(TRUNCATE)")
        .execute(pool)
        .await
        .map_err(|err| AppError::internal(format!("wal checkpoint failed: {err}")))?;
    Ok(())
}

/// Size of the database in bytes: the SQLite file plus its WAL and shared
/// memory files, or `pg_database_size` on PostgreSQL.
pub async fn database_size(pool: &Pool, config: &Config) -> Option<u64> {
    match Dialect::of(pool) {
        Dialect::Sqlite => config.sqlite_path().map(|path| sqlite_file_size(&path)),
        Dialect::Postgres => {
            sqlx::query_scalar::<_, i64>("SELECT pg_database_size(current_database())")
                .fetch_one(pool)
                .await
                .ok()
                .and_then(|size| u64::try_from(size).ok())
        }
    }
}

fn sqlite_file_size(path: &std::path::Path) -> u64 {
    let mut total = std::fs::metadata(path).map(|meta| meta.len()).unwrap_or(0);
    for suffix in ["-wal", "-shm"] {
        let sidecar = std::path::PathBuf::from(format!("{}{suffix}", path.display()));
        total += std::fs::metadata(sidecar)
            .map(|meta| meta.len())
            .unwrap_or(0);
    }
    total
}

/// True when the database rejected a write because a referenced row is
/// missing.
///
/// Used to turn races with a concurrent delete into a domain error instead of a
/// 500: the group a request selected may disappear between the lookup and the
/// write, and the foreign key is the authoritative re-check. The error kind
/// covers both drivers (FR-DB-7).
pub fn is_foreign_key_violation(err: &sqlx::Error) -> bool {
    matches!(err, sqlx::Error::Database(db) if db.is_foreign_key_violation())
}

/// True when the database rejected a write because of a unique constraint.
pub fn is_unique_violation(err: &sqlx::Error) -> bool {
    matches!(err, sqlx::Error::Database(db) if db.is_unique_violation())
}

/// Connect and run migrations.
pub async fn init(config: &Config) -> Result<Pool> {
    match Dialect::from_url(&config.database_url) {
        Some(Dialect::Sqlite) => {
            let options = connect_options(config)?;
            let pool = open_sqlite_pool(options.clone()).await?;
            let migrated = migrate::run(&pool, &options).await;
            pool.close().await;
            migrated?;
        }
        Some(Dialect::Postgres) => migrate::run_postgres(&config.database_url).await?,
        None => return Err(AppError::internal("unsupported database url scheme")),
    }
    connect(config).await
}

/// A database URL with its password and query string removed, for logs and
/// debug output (FR-DB-9).
pub fn redacted_url(url: &str) -> Cow<'_, str> {
    // A SQLite URL is a file path and carries no credential.
    if Dialect::from_url(url) == Some(Dialect::Sqlite) {
        return Cow::Borrowed(url);
    }
    match url::Url::parse(url) {
        Ok(mut parsed) => {
            let _ = parsed.set_password(None);
            parsed.set_query(None);
            parsed.set_fragment(None);
            Cow::Owned(parsed.to_string())
        }
        Err(_) => Cow::Borrowed("<unparsable database url>"),
    }
}

pub mod url_previews;

#[cfg(test)]
mod tests {
    use super::*;

    /// A two-connection pool on its own file database, so two transactions can
    /// actually contend. The short `busy_timeout` keeps a test that has to wait
    /// for the write lock quick.
    async fn contention_pool(name: &str) -> Pool {
        sqlx::any::install_default_drivers();
        let dir = std::env::temp_dir().join(format!("urlet-db-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("temp dir");
        let url = format!("sqlite://{}?mode=rwc", dir.join("contention.db").display());
        let pool = AnyPoolOptions::new()
            .max_connections(2)
            .after_connect(|connection, _| {
                Box::pin(async move {
                    sqlx::query("PRAGMA journal_mode = WAL")
                        .execute(&mut *connection)
                        .await?;
                    sqlx::query("PRAGMA busy_timeout = 250")
                        .execute(&mut *connection)
                        .await?;
                    Ok(())
                })
            })
            .connect(&url)
            .await
            .expect("pool");
        sqlx::query("CREATE TABLE probe (id INTEGER PRIMARY KEY, v INTEGER)")
            .execute(&pool)
            .await
            .expect("schema");
        pool
    }

    #[test]
    fn the_url_scheme_selects_the_backend() {
        assert_eq!(
            Dialect::from_url("sqlite:///data/urlet.db?mode=rwc"),
            Some(Dialect::Sqlite)
        );
        assert_eq!(Dialect::from_url("sqlite::memory:"), Some(Dialect::Sqlite));
        assert_eq!(
            Dialect::from_url("postgres://u@h/db"),
            Some(Dialect::Postgres)
        );
        assert_eq!(
            Dialect::from_url("PostgreSQL://u@h/db"),
            Some(Dialect::Postgres)
        );
        assert_eq!(Dialect::from_url("mysql://u@h/db"), None);
        assert_eq!(Dialect::from_url("urlet.db"), None);
    }

    #[test]
    fn redacted_urls_keep_no_password_or_query() {
        let redacted = redacted_url(
            "postgres://urlet:s3cret@db.internal:5433/urlet?sslmode=verify-full&password=x",
        );
        assert_eq!(redacted, "postgres://urlet@db.internal:5433/urlet");
        assert!(!redacted_url("mysql://root:hunter2@h/db").contains("hunter2"));
        assert_eq!(
            redacted_url("sqlite:///data/urlet.db?mode=rwc"),
            "sqlite:///data/urlet.db?mode=rwc"
        );
        assert!(!redacted_url("postgres://u:p@[bad").contains(":p@"));
    }

    #[test]
    fn a_sqlite_url_without_a_mode_still_creates_the_file() {
        let dir = std::env::temp_dir().join(format!("urlet-any-url-{}", std::process::id()));
        let mut config = Config::for_tests(&dir);
        config.database_url = "sqlite:///tmp/a.db".into();
        assert_eq!(any_url(&config), "sqlite:///tmp/a.db?mode=rwc");
        config.database_url = "sqlite:///tmp/a.db?cache=shared".into();
        assert_eq!(any_url(&config), "sqlite:///tmp/a.db?cache=shared&mode=rwc");
        config.database_url = "sqlite:///tmp/a.db?mode=ro".into();
        assert_eq!(any_url(&config), "sqlite:///tmp/a.db?mode=ro");
        config.database_url = "postgres://u@h/db".into();
        assert_eq!(any_url(&config), "postgres://u@h/db");
        let _ = std::fs::remove_dir_all(&dir);
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
