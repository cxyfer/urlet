//! Minimal embedded migration runner.
//!
//! Migrations are compiled into the binary (`include_str!`) and applied in
//! order. Checksums are stored so that editing an already applied migration is
//! detected instead of silently diverging.
//!
//! Most migrations run in a normal transaction. A *rebuild* migration is one
//! that has to disable foreign key enforcement outside a transaction: SQLite's
//! `DROP TABLE` performs an implicit `DELETE` when foreign keys are enabled,
//! which can fire `ON DELETE CASCADE` on child tables. The per-domain slug
//! migration rebuilds `urls`, so it must run on a dedicated connection with
//! `PRAGMA foreign_keys = OFF` and restore the pragma before closing.

use sha2::{Digest, Sha256};
use sqlx::sqlite::SqliteConnectOptions;
use sqlx::{Connection, SqliteConnection, SqlitePool};

use crate::error::{AppError, Result};
use crate::services::time::now_iso;

/// How a migration must be executed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MigrationKind {
    /// Ordinary migration: one transaction on a pooled connection.
    Transactional,
    /// Table rebuild: dedicated connection, foreign keys disabled outside the
    /// transaction, `PRAGMA foreign_key_check` before commit.
    Rebuild,
}

/// One embedded migration.
#[derive(Debug)]
pub struct Migration {
    pub version: &'static str,
    pub sql: &'static str,
    pub kind: MigrationKind,
}

/// Ordered list of migrations. Append only — never edit an applied file.
pub const MIGRATIONS: &[Migration] = &[
    Migration {
        version: "0001_init",
        sql: include_str!("../../migrations/0001_init.sql"),
        kind: MigrationKind::Transactional,
    },
    Migration {
        version: "0002_clicks_indexes",
        sql: include_str!("../../migrations/0002_clicks_indexes.sql"),
        kind: MigrationKind::Transactional,
    },
    Migration {
        version: "0003_multi_domain",
        sql: include_str!("../../migrations/0003_multi_domain.sql"),
        kind: MigrationKind::Rebuild,
    },
    Migration {
        version: "0004_user_groups",
        sql: include_str!("../../migrations/0004_user_groups.sql"),
        kind: MigrationKind::Transactional,
    },
    Migration {
        version: "0005_group_domains",
        sql: include_str!("../../migrations/0005_group_domains.sql"),
        kind: MigrationKind::Transactional,
    },
    Migration {
        version: "0006_group_invite_quotas",
        sql: include_str!("../../migrations/0006_group_invite_quotas.sql"),
        kind: MigrationKind::Transactional,
    },
    Migration {
        version: "0007_member_invites",
        sql: include_str!("../../migrations/0007_member_invites.sql"),
        kind: MigrationKind::Transactional,
    },
    Migration {
        version: "0008_invite_creator_index",
        sql: include_str!("../../migrations/0008_invite_creator_index.sql"),
        kind: MigrationKind::Transactional,
    },
];

/// Apply all pending migrations.
pub async fn run(pool: &SqlitePool, options: &SqliteConnectOptions) -> Result<()> {
    run_inner(pool, options, None).await
}

/// Apply pending migrations up to and including `last_version`.
///
/// Used by the upgrade tests to build a realistic older schema before applying
/// the newest migration. Not part of the public API.
#[cfg_attr(not(test), allow(dead_code))]
async fn run_until(
    pool: &SqlitePool,
    options: &SqliteConnectOptions,
    last_version: &str,
) -> Result<()> {
    run_inner(pool, options, Some(last_version)).await
}

async fn run_inner(
    pool: &SqlitePool,
    options: &SqliteConnectOptions,
    last_version: Option<&str>,
) -> Result<()> {
    sqlx::query(
        "CREATE TABLE IF NOT EXISTS schema_migrations (
             version    TEXT PRIMARY KEY,
             checksum   TEXT NOT NULL,
             applied_at TEXT NOT NULL
         )",
    )
    .execute(pool)
    .await?;

    for migration in MIGRATIONS {
        let checksum = checksum(migration.sql);
        if is_applied(pool, migration.version, &checksum).await? {
            if last_version == Some(migration.version) {
                break;
            }
            continue;
        }

        tracing::info!(version = migration.version, "applying migration");
        match migration.kind {
            MigrationKind::Transactional => apply_transactional(pool, migration, &checksum).await?,
            MigrationKind::Rebuild => apply_rebuild(options, migration, &checksum).await?,
        }

        if last_version == Some(migration.version) {
            break;
        }
    }

    Ok(())
}

async fn apply_transactional(
    pool: &SqlitePool,
    migration: &Migration,
    checksum: &str,
) -> Result<()> {
    let mut tx = pool.begin().await?;
    if let Err(err) = sqlx::raw_sql(migration.sql).execute(&mut *tx).await {
        tx.rollback().await?;
        if is_applied(pool, migration.version, checksum).await? {
            return Ok(());
        }
        return Err(AppError::from(err));
    }
    if let Err(err) = insert_migration(&mut *tx, migration.version, checksum).await {
        tx.rollback().await?;
        if is_applied(pool, migration.version, checksum).await? {
            return Ok(());
        }
        return Err(err);
    }
    tx.commit().await?;
    Ok(())
}

/// Run a rebuild on its own connection so `PRAGMA foreign_keys = OFF` cannot
/// leak into the pool.
async fn apply_rebuild(
    options: &SqliteConnectOptions,
    migration: &Migration,
    checksum: &str,
) -> Result<()> {
    let mut connection = SqliteConnection::connect_with(options)
        .await
        .map_err(|err| AppError::internal(format!("cannot open migration connection: {err}")))?;

    if let Err(err) = sqlx::query("PRAGMA foreign_keys = OFF")
        .execute(&mut connection)
        .await
    {
        let _ = connection.close().await;
        return Err(AppError::internal(format!(
            "cannot disable foreign keys for {}: {err}",
            migration.version
        )));
    }

    if let Err(err) = sqlx::query("BEGIN IMMEDIATE")
        .execute(&mut connection)
        .await
    {
        let _ = sqlx::query("PRAGMA foreign_keys = ON")
            .execute(&mut connection)
            .await;
        let _ = connection.close().await;
        return Err(AppError::internal(format!(
            "cannot start rebuild for {}: {err}",
            migration.version
        )));
    }

    // Another process may have applied the rebuild while this connection waited
    // for the write lock. Re-check after BEGIN IMMEDIATE, before running DDL.
    let already =
        sqlx::query_scalar::<_, String>("SELECT checksum FROM schema_migrations WHERE version = ?")
            .bind(migration.version)
            .fetch_optional(&mut connection)
            .await;
    match already {
        Ok(Some(stored)) if stored == checksum => {
            let _ = sqlx::query("ROLLBACK").execute(&mut connection).await;
            let _ = sqlx::query("PRAGMA foreign_keys = ON")
                .execute(&mut connection)
                .await;
            let _ = connection.close().await;
            return Ok(());
        }
        Ok(Some(_)) => {
            let _ = sqlx::query("ROLLBACK").execute(&mut connection).await;
            let _ = sqlx::query("PRAGMA foreign_keys = ON")
                .execute(&mut connection)
                .await;
            let _ = connection.close().await;
            return Err(AppError::internal(format!(
                "migration {} was modified after being applied",
                migration.version
            )));
        }
        Ok(None) => {}
        Err(err) => {
            let _ = sqlx::query("ROLLBACK").execute(&mut connection).await;
            let _ = sqlx::query("PRAGMA foreign_keys = ON")
                .execute(&mut connection)
                .await;
            let _ = connection.close().await;
            return Err(AppError::from(err));
        }
    }

    let result = apply_rebuild_inner(&mut connection, migration, checksum).await;

    if result.is_err() {
        let _ = sqlx::query("ROLLBACK").execute(&mut connection).await;
    }

    // Restore enforcement even when the migration failed. The connection is
    // dedicated and closed immediately afterwards, but the invariant is worth
    // keeping explicit.
    if let Err(err) = sqlx::query("PRAGMA foreign_keys = ON")
        .execute(&mut connection)
        .await
    {
        tracing::error!(error = %err, "cannot restore foreign key enforcement");
    }
    let _ = connection.close().await;

    result
}

async fn apply_rebuild_inner(
    connection: &mut SqliteConnection,
    migration: &Migration,
    checksum: &str,
) -> Result<()> {
    sqlx::raw_sql(migration.sql)
        .execute(&mut *connection)
        .await?;

    let violations = sqlx::query("PRAGMA foreign_key_check")
        .fetch_all(&mut *connection)
        .await?;
    if !violations.is_empty() {
        return Err(AppError::internal(format!(
            "migration {} left {} foreign key violation(s)",
            migration.version,
            violations.len()
        )));
    }

    insert_migration(&mut *connection, migration.version, checksum).await?;
    sqlx::query("COMMIT").execute(&mut *connection).await?;
    Ok(())
}

async fn insert_migration(
    executor: impl sqlx::Executor<'_, Database = sqlx::Sqlite>,
    version: &str,
    checksum: &str,
) -> Result<()> {
    sqlx::query(
        "INSERT INTO schema_migrations (version, checksum, applied_at)
         VALUES (?, ?, ?)",
    )
    .bind(version)
    .bind(checksum)
    .bind(now_iso())
    .execute(executor)
    .await?;
    Ok(())
}

/// Whether `version` is recorded with exactly this checksum.
async fn is_applied(pool: &SqlitePool, version: &str, checksum: &str) -> Result<bool> {
    let stored: Option<String> =
        sqlx::query_scalar("SELECT checksum FROM schema_migrations WHERE version = ?")
            .bind(version)
            .fetch_optional(pool)
            .await
            .map_err(AppError::from)?;
    match stored {
        None => Ok(false),
        Some(stored) if stored == checksum => Ok(true),
        Some(_) => Err(AppError::internal(format!(
            "migration {version} was modified after being applied"
        ))),
    }
}

/// Number of migrations recorded as applied.
pub async fn applied_count(pool: &SqlitePool) -> Result<i64> {
    sqlx::query_scalar("SELECT COUNT(*) FROM schema_migrations")
        .fetch_one(pool)
        .await
        .map_err(AppError::from)
}

fn checksum(sql: &str) -> String {
    let digest = Sha256::digest(sql.as_bytes());
    digest.iter().map(|byte| format!("{byte:02x}")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use sqlx::sqlite::SqliteConnectOptions;
    use sqlx::Row;
    use std::str::FromStr;

    fn test_options(path: &str) -> SqliteConnectOptions {
        SqliteConnectOptions::from_str(&format!("sqlite://{path}?mode=rwc"))
            .expect("valid sqlite url")
            .create_if_missing(true)
            .foreign_keys(true)
    }

    async fn test_pool(options: &SqliteConnectOptions) -> SqlitePool {
        SqlitePool::connect_with(options.clone())
            .await
            .expect("pool")
    }

    #[tokio::test]
    async fn applies_migrations_once_and_is_idempotent() {
        let dir = std::env::temp_dir().join(format!("urlet-migrate-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("temp dir");
        let path = format!("{}/migrate.db", dir.display());
        let options = test_options(&path);
        let pool = test_pool(&options).await;

        run(&pool, &options).await.expect("first run");
        run(&pool, &options).await.expect("second run is a no-op");

        let tables: Vec<String> =
            sqlx::query("SELECT name FROM sqlite_master WHERE type = 'table'")
                .fetch_all(&pool)
                .await
                .expect("tables")
                .into_iter()
                .map(|row| row.get::<String, _>("name"))
                .collect();
        for expected in [
            "users",
            "user_groups",
            "group_domains",
            "urls",
            "clicks",
            "sessions",
            "settings",
            "audit_logs",
            "domains",
        ] {
            assert!(
                tables.contains(&expected.to_string()),
                "missing table {expected}"
            );
        }

        let seeded: i64 = sqlx::query("SELECT COUNT(*) AS c FROM settings")
            .fetch_one(&pool)
            .await
            .expect("settings")
            .get("c");
        assert!(seeded >= 7, "default settings seeded");

        let default_groups: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM user_groups WHERE is_default = 1")
                .fetch_one(&pool)
                .await
                .expect("default group");
        assert_eq!(default_groups, 1, "exactly one default group is seeded");
        let token_quota: String =
            sqlx::query_scalar("SELECT value FROM settings WHERE key = 'max_api_tokens_per_user'")
                .fetch_one(&pool)
                .await
                .expect("token quota");
        assert_eq!(token_quota, "20");

        pool.close().await;
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn rebuild_preserves_related_rows_and_scopes_slugs_per_domain() {
        let dir =
            std::env::temp_dir().join(format!("urlet-migrate-upgrade-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("temp dir");
        let path = format!("{}/upgrade.db", dir.display());
        let options = test_options(&path);
        let pool = test_pool(&options).await;

        run_until(&pool, &options, "0002_clicks_indexes")
            .await
            .expect("v2 schema");

        let now = now_iso();
        sqlx::query(
            "INSERT INTO users (email, email_normalized, password_hash, created_at, updated_at)
             VALUES ('a@example.com', 'a@example.com', 'hash', ?, ?)",
        )
        .bind(&now)
        .bind(&now)
        .execute(&pool)
        .await
        .expect("user");
        let user_id: i64 = sqlx::query_scalar("SELECT id FROM users WHERE email = 'a@example.com'")
            .fetch_one(&pool)
            .await
            .expect("user id");

        sqlx::query(
            "INSERT INTO domains (hostname, is_default, is_verified, created_at)
             VALUES ('s.example.com', 1, 1, ?)",
        )
        .bind(&now)
        .execute(&pool)
        .await
        .expect("domain");
        let default_domain: i64 =
            sqlx::query_scalar("SELECT id FROM domains WHERE hostname = 's.example.com'")
                .fetch_one(&pool)
                .await
                .expect("domain id");

        // A v2 URL may have a NULL domain_id; the upgrade must back-fill it.
        let url_result = sqlx::query(
            "INSERT INTO urls (user_id, domain_id, slug, target_url, created_at, updated_at)
             VALUES (?, NULL, 'promo', 'https://example.com/a', ?, ?)",
        )
        .bind(user_id)
        .bind(&now)
        .bind(&now)
        .execute(&pool)
        .await
        .expect("url");
        let url_id = url_result.last_insert_rowid();

        sqlx::query("INSERT INTO clicks (url_id, clicked_at) VALUES (?, ?)")
            .bind(url_id)
            .bind(&now)
            .execute(&pool)
            .await
            .expect("click");

        sqlx::query("INSERT INTO tags (user_id, name, created_at) VALUES (?, 'tag', ?)")
            .bind(user_id)
            .bind(&now)
            .execute(&pool)
            .await
            .expect("tag");
        let tag_id: i64 = sqlx::query_scalar("SELECT id FROM tags WHERE name = 'tag'")
            .fetch_one(&pool)
            .await
            .expect("tag id");
        sqlx::query("INSERT INTO url_tags (url_id, tag_id) VALUES (?, ?)")
            .bind(url_id)
            .bind(tag_id)
            .execute(&pool)
            .await
            .expect("url tag");

        sqlx::query(
            "INSERT INTO url_daily_stats (url_id, day, clicks) VALUES (?, '2026-01-01', 3)",
        )
        .bind(url_id)
        .execute(&pool)
        .await
        .expect("daily stats");

        run(&pool, &options).await.expect("upgrade");

        let backfilled: i64 = sqlx::query_scalar("SELECT domain_id FROM urls WHERE id = ?")
            .bind(url_id)
            .fetch_one(&pool)
            .await
            .expect("back-filled domain");
        assert_eq!(backfilled, default_domain, "NULL domain back-filled");

        for (table, expected) in [
            ("urls", 1_i64),
            ("clicks", 1_i64),
            ("url_tags", 1_i64),
            ("url_daily_stats", 1_i64),
        ] {
            let count: i64 = sqlx::query_scalar(&format!("SELECT COUNT(*) FROM {table}"))
                .fetch_one(&pool)
                .await
                .expect("count");
            assert_eq!(count, expected, "{table} rows preserved");
        }

        let violations = sqlx::query("PRAGMA foreign_key_check")
            .fetch_all(&pool)
            .await
            .expect("foreign key check");
        assert!(violations.is_empty(), "no foreign key violations");

        sqlx::query(
            "INSERT INTO domains (hostname, is_default, is_verified, status, source, created_at)
             VALUES ('b.example.com', 0, 1, 'active', 'ui', ?)",
        )
        .bind(&now)
        .execute(&pool)
        .await
        .expect("second domain");
        let second_domain: i64 =
            sqlx::query_scalar("SELECT id FROM domains WHERE hostname = 'b.example.com'")
                .fetch_one(&pool)
                .await
                .expect("second domain id");

        let second_url = sqlx::query(
            "INSERT INTO urls (user_id, domain_id, slug, target_url, created_at, updated_at)
             VALUES (?, ?, 'promo', 'https://example.com/b', ?, ?)",
        )
        .bind(user_id)
        .bind(second_domain)
        .bind(&now)
        .bind(&now)
        .execute(&pool)
        .await
        .expect("same slug on another domain");
        assert!(second_url.last_insert_rowid() > url_id, "sequence advanced");

        let duplicate = sqlx::query(
            "INSERT INTO urls (user_id, domain_id, slug, target_url, created_at, updated_at)
             VALUES (?, ?, 'promo', 'https://example.com/c', ?, ?)",
        )
        .bind(user_id)
        .bind(second_domain)
        .bind(&now)
        .bind(&now)
        .execute(&pool)
        .await;
        assert!(duplicate.is_err(), "slug is unique within a domain");

        pool.close().await;
        let _ = std::fs::remove_dir_all(&dir);
    }
    #[tokio::test]
    async fn normalizes_default_ports_in_migrated_domains() {
        let dir = std::env::temp_dir().join(format!(
            "urlet-migrate-default-ports-{}",
            std::process::id()
        ));
        std::fs::create_dir_all(&dir).expect("temp dir");
        let path = format!("{}/default-ports.db", dir.display());
        let options = test_options(&path);
        let pool = test_pool(&options).await;

        run_until(&pool, &options, "0002_clicks_indexes")
            .await
            .expect("v2 schema");

        let now = now_iso();
        sqlx::query(
            "INSERT INTO users (email, email_normalized, password_hash, created_at, updated_at)
             VALUES ('ports@example.com', 'ports@example.com', 'hash', ?, ?)",
        )
        .bind(&now)
        .bind(&now)
        .execute(&pool)
        .await
        .expect("user");
        let user_id: i64 =
            sqlx::query_scalar("SELECT id FROM users WHERE email = 'ports@example.com'")
                .fetch_one(&pool)
                .await
                .expect("user id");

        // v2 seeded the canonical row from URLET_BASE_URL without normalising
        // an explicit default port.
        sqlx::query(
            "INSERT INTO domains (hostname, is_default, is_verified, created_at)
             VALUES ('s.example.com.:0443', 1, 1, ?)",
        )
        .bind(&now)
        .execute(&pool)
        .await
        .expect("canonical domain");
        sqlx::query(
            "INSERT INTO domains (hostname, is_default, is_verified, created_at)
             VALUES ('b.example.com:080', 0, 1, ?)",
        )
        .bind(&now)
        .execute(&pool)
        .await
        .expect("extra domain");
        let canonical_old: i64 =
            sqlx::query_scalar("SELECT id FROM domains WHERE hostname = 's.example.com.:0443'")
                .fetch_one(&pool)
                .await
                .expect("old canonical id");
        let extra_old: i64 =
            sqlx::query_scalar("SELECT id FROM domains WHERE hostname = 'b.example.com:080'")
                .fetch_one(&pool)
                .await
                .expect("old extra id");
        sqlx::query(
            "INSERT INTO domains (hostname, is_default, is_verified, created_at)
             VALUES ('c.example.com:08080', 0, 1, ?)",
        )
        .bind(&now)
        .execute(&pool)
        .await
        .expect("zero-padded non-default port");

        sqlx::query(
            "INSERT INTO urls (user_id, domain_id, slug, target_url, created_at, updated_at)
             VALUES (?, NULL, 'promo', 'https://example.com/a', ?, ?)",
        )
        .bind(user_id)
        .bind(&now)
        .bind(&now)
        .execute(&pool)
        .await
        .expect("url with null domain");
        sqlx::query(
            "INSERT INTO urls (user_id, domain_id, slug, target_url, created_at, updated_at)
             VALUES (?, ?, 'other', 'https://example.com/b', ?, ?)",
        )
        .bind(user_id)
        .bind(extra_old)
        .bind(&now)
        .bind(&now)
        .execute(&pool)
        .await
        .expect("url on extra domain");

        run(&pool, &options).await.expect("upgrade");

        let canonical: i64 =
            sqlx::query_scalar("SELECT id FROM domains WHERE hostname = 's.example.com'")
                .fetch_one(&pool)
                .await
                .expect("normalised canonical");
        assert_eq!(
            canonical, canonical_old,
            "canonical row is renamed in place"
        );
        let extra: i64 =
            sqlx::query_scalar("SELECT id FROM domains WHERE hostname = 'b.example.com'")
                .fetch_one(&pool)
                .await
                .expect("normalised extra");
        assert_eq!(extra, extra_old, "extra row is renamed in place");
        let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM domains")
            .fetch_one(&pool)
            .await
            .expect("domain count");
        assert_eq!(count, 3, "no duplicate canonical row was inserted");
        let non_default: String =
            sqlx::query_scalar("SELECT hostname FROM domains WHERE hostname LIKE 'c.example.com%'")
                .fetch_one(&pool)
                .await
                .expect("non-default port");
        assert_eq!(non_default, "c.example.com:8080");

        let backfilled: i64 = sqlx::query_scalar("SELECT domain_id FROM urls WHERE slug = 'promo'")
            .fetch_one(&pool)
            .await
            .expect("back-filled domain");
        assert_eq!(
            backfilled, canonical,
            "NULL domain still points at the renamed row"
        );
        let extra_url: i64 = sqlx::query_scalar("SELECT domain_id FROM urls WHERE slug = 'other'")
            .fetch_one(&pool)
            .await
            .expect("extra url domain");
        assert_eq!(extra_url, extra, "extra URL keeps its renamed domain");

        pool.close().await;
        let _ = std::fs::remove_dir_all(&dir);
    }
    #[tokio::test]
    async fn merges_canonical_equivalent_domains_before_normalizing() {
        let dir = std::env::temp_dir().join(format!(
            "urlet-migrate-merge-domains-{}",
            std::process::id()
        ));
        std::fs::create_dir_all(&dir).expect("temp dir");
        let path = format!("{}/merge-domains.db", dir.display());
        let options = test_options(&path);
        let pool = test_pool(&options).await;

        run_until(&pool, &options, "0002_clicks_indexes")
            .await
            .expect("v2 schema");

        let now = now_iso();
        sqlx::query(
            "INSERT INTO users (email, email_normalized, password_hash, created_at, updated_at)
             VALUES ('merge@example.com', 'merge@example.com', 'hash', ?, ?)",
        )
        .bind(&now)
        .bind(&now)
        .execute(&pool)
        .await
        .expect("user");
        let user_id: i64 =
            sqlx::query_scalar("SELECT id FROM users WHERE email = 'merge@example.com'")
                .fetch_one(&pool)
                .await
                .expect("user id");

        // v2 could leave both rows after URLET_BASE_URL changed from
        // https://example.com:443 to https://example.com. The new row is the
        // default; the old row still owns links created before the change.
        sqlx::query(
            "INSERT INTO domains (hostname, is_default, is_verified, created_at)
             VALUES ('s.example.com:0443', 0, 1, ?)",
        )
        .bind(&now)
        .execute(&pool)
        .await
        .expect("old canonical");
        sqlx::query(
            "INSERT INTO domains (hostname, is_default, is_verified, created_at)
             VALUES ('s.example.com', 1, 1, ?)",
        )
        .bind(&now)
        .execute(&pool)
        .await
        .expect("new canonical");
        let old_id: i64 =
            sqlx::query_scalar("SELECT id FROM domains WHERE hostname = 's.example.com:0443'")
                .fetch_one(&pool)
                .await
                .expect("old id");
        let new_id: i64 =
            sqlx::query_scalar("SELECT id FROM domains WHERE hostname = 's.example.com'")
                .fetch_one(&pool)
                .await
                .expect("new id");

        let old_url = sqlx::query(
            "INSERT INTO urls (user_id, domain_id, slug, target_url, created_at, updated_at)
             VALUES (?, ?, 'old', 'https://example.com/old', ?, ?)",
        )
        .bind(user_id)
        .bind(old_id)
        .bind(&now)
        .bind(&now)
        .execute(&pool)
        .await
        .expect("old url")
        .last_insert_rowid();
        let new_url = sqlx::query(
            "INSERT INTO urls (user_id, domain_id, slug, target_url, created_at, updated_at)
             VALUES (?, ?, 'new', 'https://example.com/new', ?, ?)",
        )
        .bind(user_id)
        .bind(new_id)
        .bind(&now)
        .bind(&now)
        .execute(&pool)
        .await
        .expect("new url")
        .last_insert_rowid();

        run(&pool, &options).await.expect("upgrade");

        let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM domains")
            .fetch_one(&pool)
            .await
            .expect("domain count");
        assert_eq!(count, 1, "equivalent rows are merged");
        let hostname: String = sqlx::query_scalar("SELECT hostname FROM domains")
            .fetch_one(&pool)
            .await
            .expect("hostname");
        assert_eq!(hostname, "s.example.com");
        let kept: i64 = sqlx::query_scalar("SELECT id FROM domains")
            .fetch_one(&pool)
            .await
            .expect("kept id");
        assert_eq!(kept, new_id, "the default row is the survivor");
        let is_default: i64 = sqlx::query_scalar("SELECT is_default FROM domains")
            .fetch_one(&pool)
            .await
            .expect("default");
        assert_eq!(is_default, 1);

        for url_id in [old_url, new_url] {
            let domain_id: i64 = sqlx::query_scalar("SELECT domain_id FROM urls WHERE id = ?")
                .bind(url_id)
                .fetch_one(&pool)
                .await
                .expect("url domain");
            assert_eq!(domain_id, new_id, "URL domain_id remapped to the survivor");
        }

        let violations = sqlx::query("PRAGMA foreign_key_check")
            .fetch_all(&pool)
            .await
            .expect("foreign key check");
        assert!(violations.is_empty());

        pool.close().await;
        let _ = std::fs::remove_dir_all(&dir);
    }
    #[tokio::test]
    async fn normalizes_bracketed_ipv6_default_ports() {
        let dir =
            std::env::temp_dir().join(format!("urlet-migrate-ipv6-ports-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("temp dir");
        let path = format!("{}/ipv6-ports.db", dir.display());
        let options = test_options(&path);
        let pool = test_pool(&options).await;

        run_until(&pool, &options, "0002_clicks_indexes")
            .await
            .expect("v2 schema");

        let now = now_iso();
        sqlx::query(
            "INSERT INTO users (email, email_normalized, password_hash, created_at, updated_at)
             VALUES ('v6@example.com', 'v6@example.com', 'hash', ?, ?)",
        )
        .bind(&now)
        .bind(&now)
        .execute(&pool)
        .await
        .expect("user");
        let user_id: i64 =
            sqlx::query_scalar("SELECT id FROM users WHERE email = 'v6@example.com'")
                .fetch_one(&pool)
                .await
                .expect("user id");

        sqlx::query(
            "INSERT INTO domains (hostname, is_default, is_verified, created_at)
             VALUES ('[::1]:443', 1, 1, ?)",
        )
        .bind(&now)
        .execute(&pool)
        .await
        .expect("IPv6 default");
        sqlx::query(
            "INSERT INTO domains (hostname, is_default, is_verified, created_at)
             VALUES ('[::1]:080', 0, 1, ?)",
        )
        .bind(&now)
        .execute(&pool)
        .await
        .expect("IPv6 zero-padded default");
        let first_id: i64 =
            sqlx::query_scalar("SELECT id FROM domains WHERE hostname = '[::1]:443'")
                .fetch_one(&pool)
                .await
                .expect("first id");
        let second_id: i64 =
            sqlx::query_scalar("SELECT id FROM domains WHERE hostname = '[::1]:080'")
                .fetch_one(&pool)
                .await
                .expect("second id");

        for (slug, domain_id) in [("first", first_id), ("second", second_id)] {
            sqlx::query(
                "INSERT INTO urls (user_id, domain_id, slug, target_url, created_at, updated_at)
                 VALUES (?, ?, ?, 'https://example.com', ?, ?)",
            )
            .bind(user_id)
            .bind(domain_id)
            .bind(slug)
            .bind(&now)
            .bind(&now)
            .execute(&pool)
            .await
            .expect("url");
        }

        run(&pool, &options).await.expect("upgrade");

        let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM domains")
            .fetch_one(&pool)
            .await
            .expect("domain count");
        assert_eq!(count, 1, "IPv6 default-port rows are merged");
        let hostname: String = sqlx::query_scalar("SELECT hostname FROM domains")
            .fetch_one(&pool)
            .await
            .expect("hostname");
        assert_eq!(hostname, "[::1]");
        let kept: i64 = sqlx::query_scalar("SELECT id FROM domains")
            .fetch_one(&pool)
            .await
            .expect("kept id");
        let urls: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM urls WHERE domain_id = ?")
            .bind(kept)
            .fetch_one(&pool)
            .await
            .expect("urls on kept domain");
        assert_eq!(urls, 2, "both URLs point at the surviving domain");
        let default: i64 = sqlx::query_scalar("SELECT is_default FROM domains WHERE id = ?")
            .bind(kept)
            .fetch_one(&pool)
            .await
            .expect("default flag");
        assert_eq!(default, 1, "the default row is the survivor");
        let verified: i64 = sqlx::query_scalar("SELECT is_verified FROM domains WHERE id = ?")
            .bind(kept)
            .fetch_one(&pool)
            .await
            .expect("verified flag");
        assert_eq!(verified, 1);
        let violations = sqlx::query("PRAGMA foreign_key_check")
            .fetch_all(&pool)
            .await
            .expect("foreign key check");
        assert!(violations.is_empty());

        pool.close().await;
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn migrates_a_populated_v2_database() {
        let dir =
            std::env::temp_dir().join(format!("urlet-migrate-populated-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("temp dir");
        let path = format!("{}/populated.db", dir.display());
        let options = test_options(&path);
        let pool = test_pool(&options).await;

        run_until(&pool, &options, "0002_clicks_indexes")
            .await
            .expect("v2 schema");
        let now = now_iso();
        sqlx::query(
            "INSERT INTO users
                 (email, email_normalized, password_hash, role, status, created_at, updated_at,
                  deleted_at)
             VALUES
                 ('live@example.com', 'live@example.com', 'x', 'user', 'active', ?, ?, NULL),
                 ('gone@example.com', 'gone@example.com', 'x', 'user', 'active', ?, ?, ?)",
        )
        .bind(&now)
        .bind(&now)
        .bind(&now)
        .bind(&now)
        .bind(&now)
        .execute(&pool)
        .await
        .expect("users");
        sqlx::query(
            "INSERT INTO invite_codes
                 (code_hash, code_prefix, created_by, max_uses, used_count, status, created_at)
             VALUES ('hash', 'ABCD', 1, 1, 0, 'active', ?)",
        )
        .bind(&now)
        .execute(&pool)
        .await
        .expect("invite");

        run(&pool, &options).await.expect("upgrade");

        let group: (String, i64, Option<i64>, Option<i64>) =
            sqlx::query_as("SELECT name, is_default, max_urls, max_api_tokens FROM user_groups")
                .fetch_one(&pool)
                .await
                .expect("group");
        assert_eq!(group, ("Standard".to_string(), 1, None, None));

        let group_ids: Vec<Option<i64>> =
            sqlx::query_scalar("SELECT group_id FROM users ORDER BY id")
                .fetch_all(&pool)
                .await
                .expect("user groups");
        assert_eq!(
            group_ids,
            vec![Some(1), Some(1)],
            "live and soft-deleted users both backfill"
        );

        let invite_group: Option<i64> = sqlx::query_scalar("SELECT group_id FROM invite_codes")
            .fetch_one(&pool)
            .await
            .expect("invite group");
        assert_eq!(invite_group, None, "existing invites stay unpinned");

        let group_policy: (Option<i64>, i64) = sqlx::query_as(
            "SELECT default_domain_id, restrict_domains FROM user_groups WHERE is_default = 1",
        )
        .fetch_one(&pool)
        .await
        .expect("group domain policy");
        assert_eq!(
            group_policy,
            (None, 0),
            "existing groups keep the site default and no restriction"
        );
        let allowlist_rows: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM group_domains")
            .fetch_one(&pool)
            .await
            .expect("allowlist rows");
        assert_eq!(allowlist_rows, 0, "existing groups have no allowlist");

        let quota: String =
            sqlx::query_scalar("SELECT value FROM settings WHERE key = 'max_api_tokens_per_user'")
                .fetch_one(&pool)
                .await
                .expect("token quota");
        assert_eq!(quota, "20");

        let violations: Vec<String> = sqlx::query("PRAGMA foreign_key_check")
            .fetch_all(&pool)
            .await
            .expect("fk check")
            .into_iter()
            .map(|row| row.get::<String, _>("table"))
            .collect();
        assert!(
            violations.is_empty(),
            "foreign key violations: {violations:?}"
        );
        assert_eq!(
            applied_count(&pool).await.expect("count"),
            MIGRATIONS.len() as i64
        );

        pool.close().await;
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn upgrades_a_v4_group_without_a_domain_policy() {
        let dir = std::env::temp_dir().join(format!(
            "urlet-migrate-group-domains-{}",
            std::process::id()
        ));
        std::fs::create_dir_all(&dir).expect("temp dir");
        let path = format!("{}/group-domains.db", dir.display());
        let options = test_options(&path);
        let pool = test_pool(&options).await;

        run_until(&pool, &options, "0004_user_groups")
            .await
            .expect("v4 schema");
        let now = now_iso();
        sqlx::query(
            "INSERT INTO domains (hostname, is_default, is_verified, created_at)
             VALUES ('s.example.com', 1, 1, ?)",
        )
        .bind(&now)
        .execute(&pool)
        .await
        .expect("domain");
        sqlx::query(
            "INSERT INTO user_groups
                 (name, description, max_urls, max_api_tokens, allow_custom_slug, allow_password,
                  allow_expiry, allow_max_clicks, allow_tags, allow_api_tokens,
                  allow_domain_selection, is_default, created_at, updated_at)
             VALUES ('Legacy', NULL, NULL, NULL, 1, 1, 1, 1, 1, 1, 1, 0, ?, ?)",
        )
        .bind(&now)
        .bind(&now)
        .execute(&pool)
        .await
        .expect("legacy group");

        run(&pool, &options).await.expect("v5 upgrade");

        let upgraded: (Option<i64>, i64) = sqlx::query_as(
            "SELECT default_domain_id, restrict_domains FROM user_groups WHERE name = 'Legacy'",
        )
        .fetch_one(&pool)
        .await
        .expect("upgraded group");
        assert_eq!(upgraded, (None, 0));
        let domain_id: i64 =
            sqlx::query_scalar("SELECT id FROM domains WHERE hostname = 's.example.com'")
                .fetch_one(&pool)
                .await
                .expect("domain id");
        let group_id: i64 = sqlx::query_scalar("SELECT id FROM user_groups WHERE name = 'Legacy'")
            .fetch_one(&pool)
            .await
            .expect("group id");
        sqlx::query("INSERT INTO group_domains (group_id, domain_id) VALUES (?, ?)")
            .bind(group_id)
            .bind(domain_id)
            .execute(&pool)
            .await
            .expect("allowlist row");
        sqlx::query("DELETE FROM domains WHERE id = ?")
            .bind(domain_id)
            .execute(&pool)
            .await
            .expect("delete domain");
        let remaining: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM group_domains")
            .fetch_one(&pool)
            .await
            .expect("remaining rows");
        assert_eq!(remaining, 0, "deleting a domain cascades the allowlist row");
        assert_eq!(
            applied_count(&pool).await.expect("count"),
            MIGRATIONS.len() as i64
        );

        pool.close().await;
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn concurrent_runs_apply_each_migration_once() {
        use std::time::Duration;

        let dir =
            std::env::temp_dir().join(format!("urlet-migrate-concurrent-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("temp dir");
        let path = format!("{}/concurrent.db", dir.display());
        let options = test_options(&path).busy_timeout(Duration::from_secs(5));
        let pool = test_pool(&options).await;

        let (first, second, third) = tokio::join!(
            run(&pool, &options),
            run(&pool, &options),
            run(&pool, &options)
        );
        first.expect("first migrator");
        second.expect("second migrator");
        third.expect("third migrator");
        assert_eq!(
            applied_count(&pool).await.expect("count"),
            MIGRATIONS.len() as i64,
            "each migration is recorded exactly once"
        );
        let default_groups: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM user_groups WHERE is_default = 1")
                .fetch_one(&pool)
                .await
                .expect("default group");
        assert_eq!(default_groups, 1, "the seed is applied once");

        pool.close().await;
        let _ = std::fs::remove_dir_all(&dir);
    }
}
