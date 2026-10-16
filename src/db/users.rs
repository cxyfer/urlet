//! `users` table.

use sqlx::{FromRow, SqlitePool};

use crate::services::time::now_iso;

#[derive(Debug, Clone, FromRow)]
pub struct User {
    pub id: i64,
    pub email: String,
    pub email_normalized: String,
    pub password_hash: String,
    pub display_name: Option<String>,
    pub role: String,
    pub status: String,
    pub email_verified_at: Option<String>,
    pub last_login_at: Option<String>,
    pub created_at: String,
    pub updated_at: String,
    pub deleted_at: Option<String>,
}

impl User {
    pub fn is_admin(&self) -> bool {
        self.role == "admin"
    }

    pub fn is_active(&self) -> bool {
        self.status == "active" && self.deleted_at.is_none()
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
#[derive(Debug, Clone)]
pub struct NewUser {
    pub email: String,
    pub email_normalized: String,
    pub password_hash: String,
    pub display_name: Option<String>,
    pub role: String,
}

const SELECT: &str = "SELECT id, email, email_normalized, password_hash, display_name, role, \
                      status, email_verified_at, last_login_at, created_at, updated_at, deleted_at \
                      FROM users";

/// Insert a user and return its id.
pub async fn create(pool: &SqlitePool, new_user: &NewUser) -> Result<i64, sqlx::Error> {
    let now = now_iso();
    let result = sqlx::query(
        "INSERT INTO users (email, email_normalized, password_hash, display_name, role, status,
                            created_at, updated_at)
         VALUES (?, ?, ?, ?, ?, 'active', ?, ?)",
    )
    .bind(&new_user.email)
    .bind(&new_user.email_normalized)
    .bind(&new_user.password_hash)
    .bind(new_user.display_name.as_deref())
    .bind(&new_user.role)
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

/// Total number of accounts that are not deleted.
pub async fn count(pool: &SqlitePool) -> Result<i64, sqlx::Error> {
    sqlx::query_scalar("SELECT COUNT(*) FROM users WHERE deleted_at IS NULL")
        .fetch_one(pool)
        .await
}

/// Filters for the administrative user listing.
#[derive(Debug, Clone, Default)]
pub struct UserFilter {
    pub search: Option<String>,
    pub role: Option<String>,
    pub status: Option<String>,
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
           AND (?1 IS NULL OR role = ?1)
           AND (?2 IS NULL OR status = ?2)
           AND (?3 IS NULL OR email LIKE ?3 ESCAPE '\\'
                          OR IFNULL(display_name, '') LIKE ?3 ESCAPE '\\')
         ORDER BY created_at DESC, id DESC
         LIMIT ?4 OFFSET ?5"
    ))
    .bind(filter.role.as_deref())
    .bind(filter.status.as_deref())
    .bind(search.as_deref())
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
           AND (?1 IS NULL OR role = ?1)
           AND (?2 IS NULL OR status = ?2)
           AND (?3 IS NULL OR email LIKE ?3 ESCAPE '\\'
                          OR IFNULL(display_name, '') LIKE ?3 ESCAPE '\\')",
    )
    .bind(filter.role.as_deref())
    .bind(filter.status.as_deref())
    .bind(search.as_deref())
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
         WHERE role = 'admin' AND status = 'active' AND deleted_at IS NULL
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
pub async fn set_password(pool: &SqlitePool, id: i64, hash: &str) -> Result<(), sqlx::Error> {
    sqlx::query("UPDATE users SET password_hash = ?, updated_at = ? WHERE id = ?")
        .bind(hash)
        .bind(now_iso())
        .bind(id)
        .execute(pool)
        .await?;
    Ok(())
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

/// Soft delete a user together with their links.
pub async fn soft_delete(pool: &SqlitePool, id: i64) -> Result<(), sqlx::Error> {
    let now = now_iso();
    let mut tx = pool.begin().await?;
    sqlx::query("UPDATE users SET deleted_at = ?, updated_at = ? WHERE id = ?")
        .bind(&now)
        .bind(&now)
        .bind(id)
        .execute(&mut *tx)
        .await?;
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
    tx.commit().await?;
    Ok(())
}
