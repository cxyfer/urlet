//! `users` table.

use sqlx::{FromRow, SqlitePool};

use crate::services::time::now_iso;

/// Address of the reserved Guest account that owns anonymous links.
///
/// `.invalid` is reserved by RFC 2606 and the row is created by migration 15, so
/// the address can never belong to a person. It is only ever a label in the
/// admin UI; see [`User::is_system`].
pub const GUEST_EMAIL: &str = "guest@urlet.invalid";

#[derive(Debug, Clone, FromRow)]
pub struct User {
    pub id: i64,
    pub email: String,
    pub email_normalized: String,
    pub password_hash: String,
    pub display_name: Option<String>,
    pub role: String,
    pub status: String,
    pub group_id: Option<i64>,
    pub email_verified_at: Option<String>,
    pub last_login_at: Option<String>,
    pub created_at: String,
    pub updated_at: String,
    pub deleted_at: Option<String>,
    /// `true` for the reserved accounts the application manages itself. They are
    /// hidden from every listing and count, cannot sign in and cannot be edited
    /// by an administrator.
    pub is_system: bool,
}

impl User {
    pub fn is_admin(&self) -> bool {
        self.role == "admin"
    }

    pub fn is_active(&self) -> bool {
        self.status == "active" && self.deleted_at.is_none()
    }

    /// True for the reserved Guest account that owns anonymous links.
    pub fn is_guest(&self) -> bool {
        self.is_system
    }

    /// Name to show in the UI.
    pub fn label(&self) -> &str {
        self.display_name
            .as_deref()
            .filter(|name| !name.is_empty())
            .unwrap_or(&self.email)
    }
}

/// Fields required to create a user.
///
/// `group_id` is a *preferred* group: the insert resolves it against the current
/// table and falls back to the default group in the same statement, so a group
/// deleted between the invite lookup and the insert cannot fail registration.
#[derive(Debug, Clone)]
pub struct NewUser {
    pub email: String,
    pub email_normalized: String,
    pub password_hash: String,
    pub display_name: Option<String>,
    pub role: String,
    pub group_id: Option<i64>,
}

const SELECT: &str = "SELECT id, email, email_normalized, password_hash, display_name, role, \
                      status, group_id, email_verified_at, last_login_at, created_at, updated_at, \
                      deleted_at, is_system FROM users";

/// Insert a user and return its id.
pub async fn create(pool: &SqlitePool, new_user: &NewUser) -> Result<i64, sqlx::Error> {
    let now = now_iso();
    let result = sqlx::query(
        "INSERT INTO users (email, email_normalized, password_hash, display_name, role, status,
                            group_id, created_at, updated_at)
         VALUES (?, ?, ?, ?, ?, 'active',
                 COALESCE((SELECT id FROM user_groups WHERE id = ?),
                          (SELECT id FROM user_groups WHERE is_default = 1)),
                 ?, ?)",
    )
    .bind(&new_user.email)
    .bind(&new_user.email_normalized)
    .bind(&new_user.password_hash)
    .bind(new_user.display_name.as_deref())
    .bind(&new_user.role)
    .bind(new_user.group_id)
    .bind(&now)
    .bind(&now)
    .execute(pool)
    .await?;
    Ok(result.last_insert_rowid())
}

/// Find an active-or-suspended user by normalised email.
pub async fn find_by_email(
    pool: &SqlitePool,
    email_normalized: &str,
) -> Result<Option<User>, sqlx::Error> {
    sqlx::query_as::<_, User>(&format!("{SELECT} WHERE email_normalized = ?"))
        .bind(email_normalized)
        .fetch_optional(pool)
        .await
}

/// Find a user by id.
pub async fn find_by_id(pool: &SqlitePool, id: i64) -> Result<Option<User>, sqlx::Error> {
    sqlx::query_as::<_, User>(&format!("{SELECT} WHERE id = ?"))
        .bind(id)
        .fetch_optional(pool)
        .await
}

/// Total number of accounts a person registered (the reserved Guest account is
/// not a person and is never counted; see [`User::is_system`]).
pub async fn count(pool: &SqlitePool) -> Result<i64, sqlx::Error> {
    sqlx::query_scalar("SELECT COUNT(*) FROM users WHERE deleted_at IS NULL AND is_system = 0")
        .fetch_one(pool)
        .await
}

/// The reserved system account (the Guest owner of anonymous links), if the
/// migration created it.
pub async fn find_system(pool: &SqlitePool) -> Result<Option<User>, sqlx::Error> {
    sqlx::query_as::<_, User>(&format!(
        "{SELECT} WHERE is_system = 1 AND deleted_at IS NULL ORDER BY id LIMIT 1"
    ))
    .fetch_optional(pool)
    .await
}

/// Filters for the administrative user listing.
#[derive(Debug, Clone, Default)]
pub struct UserFilter {
    pub search: Option<String>,
    pub role: Option<String>,
    pub status: Option<String>,
    pub group_id: Option<i64>,
}

/// List users for the admin panel.
pub async fn list(
    pool: &SqlitePool,
    filter: &UserFilter,
    limit: i64,
    offset: i64,
) -> Result<Vec<User>, sqlx::Error> {
    let search = filter.search.as_deref().map(crate::db::urls::like_pattern);
    sqlx::query_as::<_, User>(&format!(
        "{SELECT}
         WHERE deleted_at IS NULL
           AND is_system = 0
           AND (?1 IS NULL OR role = ?1)
           AND (?2 IS NULL OR status = ?2)
           AND (?3 IS NULL OR email LIKE ?3 ESCAPE '\\'
                          OR IFNULL(display_name, '') LIKE ?3 ESCAPE '\\')
           AND (?4 IS NULL OR group_id = ?4)
         ORDER BY created_at DESC, id DESC
         LIMIT ?5 OFFSET ?6"
    ))
    .bind(filter.role.as_deref())
    .bind(filter.status.as_deref())
    .bind(search.as_deref())
    .bind(filter.group_id)
    .bind(limit)
    .bind(offset)
    .fetch_all(pool)
    .await
}

/// Count users matching a filter.
pub async fn count_filtered(pool: &SqlitePool, filter: &UserFilter) -> Result<i64, sqlx::Error> {
    let search = filter.search.as_deref().map(crate::db::urls::like_pattern);
    sqlx::query_scalar(
        "SELECT COUNT(*) FROM users
         WHERE deleted_at IS NULL
           AND is_system = 0
           AND (?1 IS NULL OR role = ?1)
           AND (?2 IS NULL OR status = ?2)
           AND (?3 IS NULL OR email LIKE ?3 ESCAPE '\\'
                          OR IFNULL(display_name, '') LIKE ?3 ESCAPE '\\')
           AND (?4 IS NULL OR group_id = ?4)",
    )
    .bind(filter.role.as_deref())
    .bind(filter.status.as_deref())
    .bind(search.as_deref())
    .bind(filter.group_id)
    .fetch_one(pool)
    .await
}

/// Number of active administrators, optionally ignoring one account.
pub async fn count_active_admins(
    pool: &SqlitePool,
    excluding: Option<i64>,
) -> Result<i64, sqlx::Error> {
    sqlx::query_scalar(
        "SELECT COUNT(*) FROM users
         WHERE role = 'admin' AND status = 'active' AND deleted_at IS NULL AND is_system = 0
           AND (?1 IS NULL OR id <> ?1)",
    )
    .bind(excluding)
    .fetch_one(pool)
    .await
}

/// Total links per user, for the admin user table.
pub async fn link_counts(pool: &SqlitePool) -> Result<Vec<(i64, i64)>, sqlx::Error> {
    sqlx::query_as("SELECT user_id, COUNT(*) FROM urls WHERE deleted_at IS NULL GROUP BY user_id")
        .fetch_all(pool)
        .await
}

/// Update the last successful login timestamp.
pub async fn set_last_login(pool: &SqlitePool, id: i64) -> Result<(), sqlx::Error> {
    sqlx::query("UPDATE users SET last_login_at = ?, updated_at = ? WHERE id = ?")
        .bind(now_iso())
        .bind(now_iso())
        .bind(id)
        .execute(pool)
        .await?;
    Ok(())
}

/// Replace the password hash.
///
/// Refuses system accounts: their unparsable sentinel is one of the defences
/// that keep the reserved Guest account from ever signing in.
pub async fn set_password(pool: &SqlitePool, id: i64, hash: &str) -> Result<bool, sqlx::Error> {
    let result = sqlx::query(
        "UPDATE users SET password_hash = ?, updated_at = ? WHERE id = ? AND is_system = 0",
    )
    .bind(hash)
    .bind(now_iso())
    .bind(id)
    .execute(pool)
    .await?;
    Ok(result.rows_affected() > 0)
}

/// Update the display name.
pub async fn set_display_name(
    pool: &SqlitePool,
    id: i64,
    display_name: Option<&str>,
) -> Result<(), sqlx::Error> {
    sqlx::query("UPDATE users SET display_name = ?, updated_at = ? WHERE id = ?")
        .bind(display_name)
        .bind(now_iso())
        .bind(id)
        .execute(pool)
        .await?;
    Ok(())
}

/// Update role and/or status (administrative action).
pub async fn set_role_status(
    pool: &SqlitePool,
    id: i64,
    role: &str,
    status: &str,
) -> Result<(), sqlx::Error> {
    sqlx::query("UPDATE users SET role = ?, status = ?, updated_at = ? WHERE id = ?")
        .bind(role)
        .bind(status)
        .bind(now_iso())
        .bind(id)
        .execute(pool)
        .await?;
    Ok(())
}

/// The fields an administrative user update applies together.
///
/// `None` means "leave unchanged" for every field; the inner `None` of the
/// double options clears the value.
pub struct AdminUserUpdate<'a> {
    pub role: Option<&'a str>,
    pub status: Option<&'a str>,
    pub group_id: Option<Option<i64>>,
    pub display_name: Option<Option<&'a str>>,
}

/// Apply role, status, group, display name and the suspension revocation in one
/// transaction.
///
/// Only explicitly submitted fields are written: a group-only PATCH must not
/// restore a role or status that another administrator changed after the
/// service-level read. The group is validated by the foreign key *inside* this
/// transaction, so a concurrently deleted group rolls everything back instead of
/// leaving a partial update behind.
pub async fn apply_admin_update(
    pool: &SqlitePool,
    user_id: i64,
    update: &AdminUserUpdate<'_>,
) -> Result<bool, sqlx::Error> {
    let now = now_iso();
    let mut tx = crate::db::begin_write(pool).await?;
    let result = sqlx::query(
        "UPDATE users
            SET role = COALESCE(?1, role),
                status = COALESCE(?2, status),
                group_id = CASE WHEN ?3 = 1 THEN ?4 ELSE group_id END,
                updated_at = ?5
          WHERE id = ?6
            AND is_system = 0
            AND (
              NOT (role = 'admin' AND status = 'active'
                   AND (COALESCE(?1, role) <> 'admin' OR COALESCE(?2, status) <> 'active'))
              OR (SELECT COUNT(*) FROM users
                   WHERE role = 'admin' AND status = 'active'
                     AND deleted_at IS NULL AND id <> ?6) > 0
            )",
    )
    .bind(update.role)
    .bind(update.status)
    .bind(i64::from(update.group_id.is_some()))
    .bind(update.group_id.flatten())
    .bind(&now)
    .bind(user_id)
    .execute(&mut *tx)
    .await?;
    if result.rows_affected() == 0 {
        // Either the row vanished or this would remove the last active
        // administrator; the caller distinguishes the two after the rollback.
        tx.rollback().await?;
        return Ok(false);
    }
    if let Some(display_name) = update.display_name {
        sqlx::query("UPDATE users SET display_name = ?, updated_at = ? WHERE id = ?")
            .bind(display_name)
            .bind(&now)
            .bind(user_id)
            .execute(&mut *tx)
            .await?;
    }
    if update.status == Some("suspended") {
        sqlx::query("UPDATE sessions SET revoked_at = ? WHERE user_id = ? AND revoked_at IS NULL")
            .bind(&now)
            .bind(user_id)
            .execute(&mut *tx)
            .await?;
    }
    tx.commit().await?;
    Ok(true)
}

/// Soft delete a user together with their links.
///
/// The update is conditional on the account not being the last active
/// administrator, so two concurrent demotions/deletions cannot both pass a
/// pre-check and leave the instance without one.
pub async fn soft_delete(pool: &SqlitePool, id: i64) -> Result<bool, sqlx::Error> {
    let now = now_iso();
    let mut tx = crate::db::begin_write(pool).await?;
    let result = sqlx::query(
        "UPDATE users SET deleted_at = ?, updated_at = ?
          WHERE id = ?
            AND is_system = 0
            AND (
              NOT (role = 'admin' AND status = 'active')
              OR (SELECT COUNT(*) FROM users
                   WHERE role = 'admin' AND status = 'active'
                     AND deleted_at IS NULL AND id <> ?) > 0
            )",
    )
    .bind(&now)
    .bind(&now)
    .bind(id)
    .bind(id)
    .execute(&mut *tx)
    .await?;
    if result.rows_affected() == 0 {
        tx.rollback().await?;
        return Ok(false);
    }
    sqlx::query(
        "UPDATE urls SET deleted_at = ?, updated_at = ? WHERE user_id = ? AND deleted_at IS NULL",
    )
    .bind(&now)
    .bind(&now)
    .bind(id)
    .execute(&mut *tx)
    .await?;
    sqlx::query("UPDATE sessions SET revoked_at = ? WHERE user_id = ? AND revoked_at IS NULL")
        .bind(&now)
        .bind(id)
        .execute(&mut *tx)
        .await?;
    sqlx::query("UPDATE api_tokens SET revoked_at = ? WHERE user_id = ? AND revoked_at IS NULL")
        .bind(&now)
        .bind(id)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    Ok(true)
}
