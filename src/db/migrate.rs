//! Minimal embedded migration runner.
//!
//! Migrations are compiled into the binary (`include_str!`) and applied in
//! order inside a transaction. Checksums are stored so that editing an already
//! applied migration is detected instead of silently diverging.

use sha2::{Digest, Sha256};
use sqlx::{Row, SqlitePool};

use crate::error::{AppError, Result};
use crate::services::time::now_iso;

/// Ordered list of migrations. Append only — never edit an applied file.
pub const MIGRATIONS: &[(&str, &str)] = &[
    ("0001_init", include_str!("../../migrations/0001_init.sql")),
    ("0002_clicks_indexes", include_str!("../../migrations/0002_clicks_indexes.sql")),
];

/// Apply all pending migrations.
pub async fn run(pool: &SqlitePool) -> Result<()> {
    sqlx::query(
        "CREATE TABLE IF NOT EXISTS schema_migrations (
             version    TEXT PRIMARY KEY,
             checksum   TEXT NOT NULL,
             applied_at TEXT NOT NULL
         )",
    )
    .execute(pool)
    .await?;

    let applied: Vec<(String, String)> =
        sqlx::query("SELECT version, checksum FROM schema_migrations")
            .fetch_all(pool)
            .await?
            .into_iter()
            .map(|row| (row.get::<String, _>("version"), row.get::<String, _>("checksum")))
            .collect();

    for (version, sql) in MIGRATIONS {
        let checksum = checksum(sql);
        match applied.iter().find(|(v, _)| v == version) {
            Some((_, stored)) if stored == &checksum => continue,
            Some(_) => {
                return Err(AppError::internal(format!(
                    "migration {version} was modified after being applied"
                )))
            }
            None => {
                tracing::info!(version, "applying migration");
                let mut tx = pool.begin().await?;
                sqlx::raw_sql(sql).execute(&mut *tx).await?;
                sqlx::query(
                    "INSERT INTO schema_migrations (version, checksum, applied_at)
                     VALUES (?, ?, ?)",
                )
                .bind(version)
                .bind(&checksum)
                .bind(now_iso())
                .execute(&mut *tx)
                .await?;
                tx.commit().await?;
            }
        }
    }

    Ok(())
}

fn checksum(sql: &str) -> String {
    let digest = Sha256::digest(sql.as_bytes());
    digest.iter().map(|byte| format!("{byte:02x}")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn applies_migrations_once_and_is_idempotent() {
        let dir = std::env::temp_dir().join(format!("uwurls-migrate-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("temp dir");
        let url = format!("sqlite://{}/migrate.db?mode=rwc", dir.display());
        let options: sqlx::sqlite::SqliteConnectOptions = url.parse().expect("valid url");
        let pool = SqlitePool::connect_with(options).await.expect("pool");

        run(&pool).await.expect("first run");
        run(&pool).await.expect("second run is a no-op");

        let tables: Vec<String> = sqlx::query("SELECT name FROM sqlite_master WHERE type = 'table'")
            .fetch_all(&pool)
            .await
            .expect("tables")
            .into_iter()
            .map(|row| row.get::<String, _>("name"))
            .collect();
        for expected in ["users", "urls", "clicks", "sessions", "settings", "audit_logs"] {
            assert!(tables.contains(&expected.to_string()), "missing table {expected}");
        }

        let seeded: i64 = sqlx::query("SELECT COUNT(*) AS c FROM settings")
            .fetch_one(&pool)
            .await
            .expect("settings")
            .get("c");
        assert!(seeded >= 7, "default settings seeded");

        pool.close().await;
        let _ = std::fs::remove_dir_all(&dir);
    }
}
