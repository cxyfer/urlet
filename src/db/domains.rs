//! `domains` table and domain-scoped lookups.
//!
//! A domain is either the canonical host (`source = system`), a host seeded
//! from `URLET_DOMAINS` (`source = env`), or one added in the admin UI
//! (`source = ui`). Only `active` domains resolve or can be selected.

use sqlx::{FromRow, SqlitePool};

use crate::services::time::now_iso;

/// Domain lifecycle values stored in `domains.status`.
pub mod status {
    pub const PENDING: &str = "pending";
    pub const ACTIVE: &str = "active";
    pub const DISABLED: &str = "disabled";
}

/// Where a domain came from; drives which admin actions are allowed.
pub mod source {
    pub const SYSTEM: &str = "system";
    pub const ENV: &str = "env";
    pub const UI: &str = "ui";
}

/// One row of the `domains` table.
#[derive(Debug, Clone, FromRow)]
pub struct Domain {
    pub id: i64,
    pub hostname: String,
    pub is_default: i64,
    pub is_verified: i64,
    pub created_by: Option<i64>,
    pub created_at: String,
    pub status: String,
    pub source: String,
    pub updated_at: Option<String>,
}

impl Domain {
    pub fn is_active(&self) -> bool {
        self.status == status::ACTIVE
    }

    pub fn is_pending(&self) -> bool {
        self.status == status::PENDING
    }

    pub fn is_disabled(&self) -> bool {
        self.status == status::DISABLED
    }

    pub fn is_default(&self) -> bool {
        self.is_default == 1
    }

    pub fn is_system(&self) -> bool {
        self.source == source::SYSTEM
    }

    pub fn is_env(&self) -> bool {
        self.source == source::ENV
    }

    pub fn is_ui(&self) -> bool {
        self.source == source::UI
    }

    /// System domains are re-asserted by boot and cannot be disabled or
    /// deleted. Environment domains are re-created if missing and cannot be
    /// deleted from the UI, but they can be disabled.
    pub fn is_deletable(&self) -> bool {
        self.is_ui() && !self.is_default()
    }
}

/// A domain plus its total number of links (including soft-deleted ones).
#[derive(Debug, Clone, FromRow)]
pub struct DomainWithCount {
    pub id: i64,
    pub hostname: String,
    pub is_default: i64,
    pub is_verified: i64,
    pub created_by: Option<i64>,
    pub created_at: String,
    pub status: String,
    pub source: String,
    pub updated_at: Option<String>,
    pub link_count: i64,
}

impl DomainWithCount {
    pub fn is_active(&self) -> bool {
        self.status == status::ACTIVE
    }

    pub fn is_pending(&self) -> bool {
        self.status == status::PENDING
    }

    pub fn is_disabled(&self) -> bool {
        self.status == status::DISABLED
    }

    pub fn is_default(&self) -> bool {
        self.is_default == 1
    }

    pub fn is_system(&self) -> bool {
        self.source == source::SYSTEM
    }

    pub fn is_env(&self) -> bool {
        self.source == source::ENV
    }

    pub fn is_ui(&self) -> bool {
        self.source == source::UI
    }
}

const SELECT: &str =
    "SELECT id, hostname, is_default, is_verified, created_by, created_at, status, source, \
            updated_at FROM domains";

/// Every domain, default first.
pub async fn all(pool: &SqlitePool) -> Result<Vec<Domain>, sqlx::Error> {
    sqlx::query_as::<_, Domain>(&format!("{SELECT} ORDER BY is_default DESC, hostname"))
        .fetch_all(pool)
        .await
}

/// Domains that may be selected for new links.
pub async fn active(pool: &SqlitePool) -> Result<Vec<Domain>, sqlx::Error> {
    sqlx::query_as::<_, Domain>(&format!(
        "{SELECT} WHERE status = 'active' ORDER BY is_default DESC, hostname"
    ))
    .fetch_all(pool)
    .await
}

/// Administrative listing with link counts.
pub async fn list_with_counts(pool: &SqlitePool) -> Result<Vec<DomainWithCount>, sqlx::Error> {
    sqlx::query_as::<_, DomainWithCount>(
        "SELECT d.id, d.hostname, d.is_default, d.is_verified, d.created_by, d.created_at,
                d.status, d.source, d.updated_at,
                (SELECT COUNT(*) FROM urls u WHERE u.domain_id = d.id) AS link_count
           FROM domains d
          ORDER BY d.is_default DESC, d.hostname",
    )
    .fetch_all(pool)
    .await
}

pub async fn find_by_id(pool: &SqlitePool, id: i64) -> Result<Option<Domain>, sqlx::Error> {
    sqlx::query_as::<_, Domain>(&format!("{SELECT} WHERE id = ?"))
        .bind(id)
        .fetch_optional(pool)
        .await
}

pub async fn find_by_hostname(
    pool: &SqlitePool,
    hostname: &str,
) -> Result<Option<Domain>, sqlx::Error> {
    sqlx::query_as::<_, Domain>(&format!("{SELECT} WHERE hostname = ? COLLATE NOCASE"))
        .bind(hostname)
        .fetch_optional(pool)
        .await
}

/// The configured default domain, whatever its lifecycle state.
pub async fn default_domain(pool: &SqlitePool) -> Result<Option<Domain>, sqlx::Error> {
    sqlx::query_as::<_, Domain>(&format!("{SELECT} WHERE is_default = 1 LIMIT 1"))
        .fetch_optional(pool)
        .await
}

/// The default domain only when it is usable for new links.
pub async fn active_default(pool: &SqlitePool) -> Result<Option<Domain>, sqlx::Error> {
    sqlx::query_as::<_, Domain>(&format!(
        "{SELECT} WHERE is_default = 1 AND status = 'active' LIMIT 1"
    ))
    .fetch_optional(pool)
    .await
}

pub async fn all_hostnames(pool: &SqlitePool) -> Result<Vec<String>, sqlx::Error> {
    sqlx::query_scalar("SELECT hostname FROM domains ORDER BY hostname")
        .fetch_all(pool)
        .await
}

pub async fn active_hostnames(pool: &SqlitePool) -> Result<Vec<String>, sqlx::Error> {
    sqlx::query_scalar("SELECT hostname FROM domains WHERE status = 'active' ORDER BY hostname")
        .fetch_all(pool)
        .await
}

/// Fields required to create a domain.
#[derive(Debug, Clone)]
pub struct NewDomain {
    pub hostname: String,
    pub source: &'static str,
    pub status: &'static str,
    pub verified: bool,
    pub created_by: Option<i64>,
}

/// Insert a domain. Uniqueness is enforced by the database index.
pub async fn create(pool: &SqlitePool, new_domain: &NewDomain) -> Result<Domain, sqlx::Error> {
    let now = now_iso();
    let result = sqlx::query(
        "INSERT INTO domains
             (hostname, is_default, is_verified, created_by, created_at, status, source, updated_at)
         VALUES (?, 0, ?, ?, ?, ?, ?, ?)",
    )
    .bind(&new_domain.hostname)
    .bind(i64::from(new_domain.verified))
    .bind(new_domain.created_by)
    .bind(&now)
    .bind(new_domain.status)
    .bind(new_domain.source)
    .bind(&now)
    .execute(pool)
    .await?;

    find_by_id(pool, result.last_insert_rowid())
        .await?
        .ok_or(sqlx::Error::RowNotFound)
}

/// Activate a pending or disabled domain and mark it verified.
pub async fn activate(pool: &SqlitePool, id: i64) -> Result<u64, sqlx::Error> {
    let result = sqlx::query(
        "UPDATE domains
            SET status = 'active', is_verified = 1, updated_at = ?
          WHERE id = ?",
    )
    .bind(now_iso())
    .bind(id)
    .execute(pool)
    .await?;
    Ok(result.rows_affected())
}

/// Disable a domain; links on it stop resolving.
pub async fn disable(pool: &SqlitePool, id: i64) -> Result<u64, sqlx::Error> {
    // Conditional on purpose: a concurrent `set_default` must win rather than
    // leave a disabled default domain behind. The caller re-reads the row when
    // this affects 0 rows and reports the reason.
    let result = sqlx::query(
        "UPDATE domains
            SET status = 'disabled', updated_at = ?
          WHERE id = ? AND is_default = 0 AND source <> 'system'",
    )
    .bind(now_iso())
    .bind(id)
    .execute(pool)
    .await?;
    Ok(result.rows_affected())
}

/// Make `id` the single default domain. The target must be active.
pub async fn set_default(pool: &SqlitePool, id: i64) -> Result<(), sqlx::Error> {
    let mut tx = crate::db::begin_write(pool).await?;

    let status: Option<String> = sqlx::query_scalar("SELECT status FROM domains WHERE id = ?")
        .bind(id)
        .fetch_optional(&mut *tx)
        .await?;
    if status.as_deref() != Some(status::ACTIVE) {
        return Err(sqlx::Error::RowNotFound);
    }

    sqlx::query("UPDATE domains SET is_default = 0 WHERE is_default = 1")
        .execute(&mut *tx)
        .await?;
    sqlx::query("UPDATE domains SET is_default = 1, updated_at = ? WHERE id = ?")
        .bind(now_iso())
        .bind(id)
        .execute(&mut *tx)
        .await?;

    tx.commit().await?;
    Ok(())
}

pub async fn delete(pool: &SqlitePool, id: i64) -> Result<u64, sqlx::Error> {
    // Conditional on purpose: a concurrent `set_default` must not be deleted
    // out from under the instance. The caller re-reads the row when this
    // affects 0 rows and reports the reason.
    let result =
        sqlx::query("DELETE FROM domains WHERE id = ? AND is_default = 0 AND source = 'ui'")
            .bind(id)
            .execute(pool)
            .await?;
    Ok(result.rows_affected())
}

/// Number of links on a domain, including soft-deleted rows.
pub async fn link_count(pool: &SqlitePool, id: i64) -> Result<i64, sqlx::Error> {
    sqlx::query_scalar("SELECT COUNT(*) FROM urls WHERE domain_id = ?")
        .bind(id)
        .fetch_one(pool)
        .await
}

/// Ensure the canonical host exists, is active and is a system domain.
///
/// The canonical domain is never forced to be the default: an administrator may
/// have selected another active domain. If the current default is not active,
/// it is cleared and the canonical domain is promoted.
pub async fn ensure_canonical(pool: &SqlitePool, hostname: &str) -> Result<i64, sqlx::Error> {
    let mut tx = crate::db::begin_write(pool).await?;
    let now = now_iso();

    let existing: Option<i64> =
        sqlx::query_scalar("SELECT id FROM domains WHERE hostname = ? COLLATE NOCASE")
            .bind(hostname)
            .fetch_optional(&mut *tx)
            .await?;

    // A URLET_BASE_URL change creates a new canonical row. If the previous
    // canonical row was the default, the default has to move with it; otherwise
    // new links keep pointing at the old hostname.
    let previous_system_default: Option<i64> = sqlx::query_scalar(
        "SELECT id FROM domains
          WHERE source = 'system' AND is_default = 1
            AND hostname <> ? COLLATE NOCASE
          LIMIT 1",
    )
    .bind(hostname)
    .fetch_optional(&mut *tx)
    .await?;

    let id = match existing {
        Some(id) => {
            sqlx::query(
                "UPDATE domains
                    SET hostname = ?, status = 'active', source = 'system',
                        is_verified = 1, updated_at = ?
                  WHERE id = ?",
            )
            .bind(hostname)
            .bind(&now)
            .bind(id)
            .execute(&mut *tx)
            .await?;
            id
        }
        None => {
            let has_default: i64 =
                sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM domains WHERE is_default = 1)")
                    .fetch_one(&mut *tx)
                    .await?;
            let result = sqlx::query(
                "INSERT INTO domains
                     (hostname, is_default, is_verified, created_by, created_at, status, source, updated_at)
                 VALUES (?, ?, 1, NULL, ?, 'active', 'system', ?)",
            )
            .bind(hostname)
            .bind(if has_default == 0 { 1 } else { 0 })
            .bind(&now)
            .bind(&now)
            .execute(&mut *tx)
            .await?;
            result.last_insert_rowid()
        }
    };

    if previous_system_default.is_some() {
        // Preserve an explicitly selected non-system default: in that case
        // `previous_system_default` is None and this branch is skipped.
        sqlx::query("UPDATE domains SET is_default = 0 WHERE is_default = 1")
            .execute(&mut *tx)
            .await?;
        sqlx::query("UPDATE domains SET is_default = 1 WHERE id = ?")
            .bind(id)
            .execute(&mut *tx)
            .await?;
    }

    // Old canonical rows stay active so their existing links keep working, but
    // they become ordinary UI domains that an administrator can disable or
    // delete. Only the current canonical host remains `source = 'system'`.
    let demoted = sqlx::query(
        "UPDATE domains
            SET source = 'ui', updated_at = ?
          WHERE source = 'system' AND hostname <> ? COLLATE NOCASE",
    )
    .bind(&now)
    .bind(hostname)
    .execute(&mut *tx)
    .await?
    .rows_affected();
    if demoted > 0 {
        tracing::info!(
            hostname = %hostname,
            demoted,
            "retired the previous canonical domain"
        );
    }

    // A default domain that was disabled manually must not block new links.
    sqlx::query("UPDATE domains SET is_default = 0 WHERE is_default = 1 AND status <> 'active'")
        .execute(&mut *tx)
        .await?;
    let has_default: i64 =
        sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM domains WHERE is_default = 1)")
            .fetch_one(&mut *tx)
            .await?;
    if has_default == 0 {
        sqlx::query("UPDATE domains SET is_default = 1 WHERE id = ?")
            .bind(id)
            .execute(&mut *tx)
            .await?;
    }

    tx.commit().await?;
    Ok(id)
}

/// Turn environment-managed domains that are no longer declared in
/// `URLET_DOMAINS` into ordinary UI domains.
///
/// The row, its links, status and default flag are preserved; only `source`
/// changes. This lets an administrator disable or delete a domain that was
/// removed from the deployment configuration instead of leaving it permanently
/// protected as `env`.
pub async fn detach_env(pool: &SqlitePool, declared: &[String]) -> Result<usize, sqlx::Error> {
    let mut tx = crate::db::begin_write(pool).await?;
    let env_hosts: Vec<String> =
        sqlx::query_scalar("SELECT hostname FROM domains WHERE source = 'env'")
            .fetch_all(&mut *tx)
            .await?;
    let now = now_iso();
    let mut detached = 0usize;

    for hostname in env_hosts {
        if declared
            .iter()
            .any(|declared| declared.eq_ignore_ascii_case(&hostname))
        {
            continue;
        }
        let result = sqlx::query(
            "UPDATE domains SET source = 'ui', updated_at = ?
              WHERE hostname = ? COLLATE NOCASE AND source = 'env'",
        )
        .bind(&now)
        .bind(&hostname)
        .execute(&mut *tx)
        .await?;
        detached += result.rows_affected() as usize;
    }

    tx.commit().await?;
    Ok(detached)
}

/// Insert domains declared through `URLET_DOMAINS`.
///
/// Insert-only by design: an existing row keeps its status, source, default and
/// verification. Returns how many rows were inserted.
pub async fn seed_env(pool: &SqlitePool, hostnames: &[String]) -> Result<usize, sqlx::Error> {
    let mut tx = crate::db::begin_write(pool).await?;
    let now = now_iso();
    let mut inserted = 0usize;

    for hostname in hostnames {
        let existing: Option<i64> =
            sqlx::query_scalar("SELECT id FROM domains WHERE hostname = ? COLLATE NOCASE")
                .bind(hostname)
                .fetch_optional(&mut *tx)
                .await?;
        if existing.is_some() {
            continue;
        }
        let result = sqlx::query(
            "INSERT INTO domains
                 (hostname, is_default, is_verified, created_by, created_at, status, source, updated_at)
             VALUES (?, 0, 1, NULL, ?, 'active', 'env', ?)",
        )
        .bind(hostname)
        .bind(&now)
        .bind(&now)
        .execute(&mut *tx)
        .await?;
        inserted += result.rows_affected() as usize;
    }

    tx.commit().await?;
    Ok(inserted)
}
