//! `user_groups` table: quota and feature-toggle groups.

use crate::db::Pool;
use std::collections::HashMap;

use sqlx::FromRow;

use crate::services::time::now_iso;

/// One row of `user_groups`.
///
/// A NULL quota inherits the matching site-wide setting; `0` means unlimited.
/// The `allow_*` flags default to true, so a freshly created group changes no
/// behaviour until an administrator edits it.
#[derive(Debug, Clone, FromRow)]
pub struct UserGroup {
    pub id: i64,
    pub name: String,
    pub description: Option<String>,
    pub max_urls: Option<i64>,
    pub max_api_tokens: Option<i64>,
    /// Invite codes this group may issue per `invite_period`; NULL inherits the
    /// site-wide setting, 0 means unlimited.
    pub max_invites: Option<i64>,
    /// Calendar period of the invite counter: `day`, `week` or `month`.
    pub invite_period: String,
    /// Whether the group's members may issue invite codes themselves.
    #[sqlx(try_from = "crate::db::Flag")]
    pub allow_invites: bool,
    #[sqlx(try_from = "crate::db::Flag")]
    pub allow_custom_slug: bool,
    #[sqlx(try_from = "crate::db::Flag")]
    pub allow_password: bool,
    #[sqlx(try_from = "crate::db::Flag")]
    pub allow_expiry: bool,
    #[sqlx(try_from = "crate::db::Flag")]
    pub allow_max_clicks: bool,
    #[sqlx(try_from = "crate::db::Flag")]
    pub allow_tags: bool,
    #[sqlx(try_from = "crate::db::Flag")]
    pub allow_api_tokens: bool,
    #[sqlx(try_from = "crate::db::Flag")]
    pub allow_domain_selection: bool,
    /// Which redirect modes members may use: `direct`, `splash` or `choice`.
    pub redirect_mode_policy: String,
    pub default_domain_id: Option<i64>,
    #[sqlx(try_from = "crate::db::Flag")]
    pub restrict_domains: bool,
    #[sqlx(try_from = "crate::db::Flag")]
    pub is_default: bool,
    pub created_at: String,
    pub updated_at: String,
}

const SELECT: &str = "SELECT id, name, description, max_urls, max_api_tokens, max_invites, \
                      invite_period, allow_invites, allow_custom_slug, allow_password, allow_expiry, \
                      allow_max_clicks, allow_tags, allow_api_tokens, allow_domain_selection, \
                      redirect_mode_policy, default_domain_id, restrict_domains, is_default, \
                      created_at, updated_at \
                      FROM user_groups";

/// Every field an administrator can write, already validated and canonicalised.
#[derive(Debug, Clone, Copy)]
pub struct GroupValues<'a> {
    pub name: &'a str,
    pub description: Option<&'a str>,
    pub max_urls: Option<i64>,
    pub max_api_tokens: Option<i64>,
    pub max_invites: Option<i64>,
    pub invite_period: &'a str,
    pub allow_invites: bool,
    pub allow_custom_slug: bool,
    pub allow_password: bool,
    pub allow_expiry: bool,
    pub allow_max_clicks: bool,
    pub allow_tags: bool,
    pub allow_api_tokens: bool,
    pub allow_domain_selection: bool,
    /// Which redirect modes members may use: `direct`, `splash` or `choice`.
    pub redirect_mode_policy: &'a str,
    pub default_domain_id: Option<i64>,
    pub restrict_domains: bool,
}

/// Groups in display order: the default first, then by name.
pub async fn list(pool: &Pool) -> Result<Vec<UserGroup>, sqlx::Error> {
    sqlx::query_as::<_, UserGroup>(&format!(
        "{SELECT} ORDER BY is_default DESC, name COLLATE NOCASE ASC"
    ))
    .fetch_all(pool)
    .await
}

/// Find a group by id.
pub async fn find_by_id(pool: &Pool, id: i64) -> Result<Option<UserGroup>, sqlx::Error> {
    sqlx::query_as::<_, UserGroup>(&format!("{SELECT} WHERE id = $1"))
        .bind(id)
        .fetch_optional(pool)
        .await
}

/// Find a group by name. `name` uses `COLLATE NOCASE`, so the comparison is
/// case-insensitive without normalising the input.
pub async fn find_by_name(pool: &Pool, name: &str) -> Result<Option<UserGroup>, sqlx::Error> {
    sqlx::query_as::<_, UserGroup>(&format!("{SELECT} WHERE name = $1"))
        .bind(name)
        .fetch_optional(pool)
        .await
}

/// The group marked as default, if one exists.
pub async fn default_group(pool: &Pool) -> Result<Option<UserGroup>, sqlx::Error> {
    sqlx::query_as::<_, UserGroup>(&format!("{SELECT} WHERE is_default = 1"))
        .fetch_optional(pool)
        .await
}

/// Id of the default group, used when creating users and redeeming invites.
pub async fn default_id(pool: &Pool) -> Result<Option<i64>, sqlx::Error> {
    sqlx::query_scalar("SELECT id FROM user_groups WHERE is_default = 1")
        .fetch_optional(pool)
        .await
}

/// Live member counts keyed by group id.
pub async fn member_counts(pool: &Pool) -> Result<HashMap<i64, i64>, sqlx::Error> {
    let rows: Vec<(i64, i64)> = sqlx::query_as(
        "SELECT group_id, COUNT(*) FROM users
         WHERE group_id IS NOT NULL AND deleted_at IS NULL
         GROUP BY group_id",
    )
    .fetch_all(pool)
    .await?;
    Ok(rows.into_iter().collect())
}

/// Number of live members in one group.
pub async fn count_members(pool: &Pool, group_id: i64) -> Result<i64, sqlx::Error> {
    sqlx::query_scalar("SELECT COUNT(*) FROM users WHERE group_id = $1 AND deleted_at IS NULL")
        .bind(group_id)
        .fetch_one(pool)
        .await
}

/// Insert a group. The caller has already checked the name is free; `is_default`
/// is only honoured when no default group exists yet.
pub async fn create(
    pool: &Pool,
    values: &GroupValues<'_>,
    is_default: bool,
    allowed_domain_ids: &[i64],
) -> Result<i64, sqlx::Error> {
    let now = now_iso();
    let mut tx = crate::db::begin_write(pool).await?;
    let id = sqlx::query_scalar::<_, i64>(
        "INSERT INTO user_groups
            (name, description, max_urls, max_api_tokens, max_invites, invite_period,
             allow_invites, allow_custom_slug, allow_password, allow_expiry, allow_max_clicks,
             allow_tags, allow_api_tokens, allow_domain_selection, redirect_mode_policy,
             default_domain_id, restrict_domains, is_default, created_at, updated_at)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, $16, $17, $18, $19, $20) RETURNING id",
    )
    .bind(values.name)
    .bind(values.description)
    .bind(values.max_urls)
    .bind(values.max_api_tokens)
    .bind(values.max_invites)
    .bind(values.invite_period)
    .bind(crate::db::flag(values.allow_invites))
    .bind(crate::db::flag(values.allow_custom_slug))
    .bind(crate::db::flag(values.allow_password))
    .bind(crate::db::flag(values.allow_expiry))
    .bind(crate::db::flag(values.allow_max_clicks))
    .bind(crate::db::flag(values.allow_tags))
    .bind(crate::db::flag(values.allow_api_tokens))
    .bind(crate::db::flag(values.allow_domain_selection))
    .bind(values.redirect_mode_policy)
    .bind(values.default_domain_id)
    .bind(crate::db::flag(values.restrict_domains))
    .bind(crate::db::flag(is_default))
    .bind(&now)
    .bind(&now)
    .fetch_one(&mut *tx)
    .await?;
    for domain_id in allowed_domain_ids {
        sqlx::query("INSERT INTO group_domains (group_id, domain_id) VALUES ($1, $2)")
            .bind(id)
            .bind(domain_id)
            .execute(&mut *tx)
            .await?;
    }
    tx.commit().await?;
    Ok(id)
}

/// Replace every writable field of a group.
///
/// When `make_default` is set, the marker moves in the *same* transaction: an
/// update that fails halfway must not leave two defaults or none. Returns
/// `false` when the row no longer exists; the marker is then left untouched.
pub async fn update(
    pool: &Pool,
    id: i64,
    values: &GroupValues<'_>,
    make_default: bool,
    allowed_domain_ids: Option<&[i64]>,
) -> Result<bool, sqlx::Error> {
    let now = now_iso();
    let mut tx = crate::db::begin_write(pool).await?;
    let updated = sqlx::query(
        "UPDATE user_groups
            SET name = $1, description = $2, max_urls = $3, max_api_tokens = $4,
                max_invites = $5, invite_period = $6, allow_invites = $7,
                allow_custom_slug = $8, allow_password = $9, allow_expiry = $10,
                allow_max_clicks = $11, allow_tags = $12, allow_api_tokens = $13,
                allow_domain_selection = $14, redirect_mode_policy = $15, default_domain_id = $16,
                restrict_domains = $17, updated_at = $18
          WHERE id = $19",
    )
    .bind(values.name)
    .bind(values.description)
    .bind(values.max_urls)
    .bind(values.max_api_tokens)
    .bind(values.max_invites)
    .bind(values.invite_period)
    .bind(crate::db::flag(values.allow_invites))
    .bind(crate::db::flag(values.allow_custom_slug))
    .bind(crate::db::flag(values.allow_password))
    .bind(crate::db::flag(values.allow_expiry))
    .bind(crate::db::flag(values.allow_max_clicks))
    .bind(crate::db::flag(values.allow_tags))
    .bind(crate::db::flag(values.allow_api_tokens))
    .bind(crate::db::flag(values.allow_domain_selection))
    .bind(values.redirect_mode_policy)
    .bind(values.default_domain_id)
    .bind(crate::db::flag(values.restrict_domains))
    .bind(&now)
    .bind(id)
    .execute(&mut *tx)
    .await?;

    if updated.rows_affected() == 0 {
        // Another administrator deleted the row after the service loaded it.
        tx.rollback().await?;
        return Ok(false);
    }

    if let Some(domain_ids) = allowed_domain_ids {
        sqlx::query("DELETE FROM group_domains WHERE group_id = $1")
            .bind(id)
            .execute(&mut *tx)
            .await?;
        for domain_id in domain_ids {
            sqlx::query("INSERT INTO group_domains (group_id, domain_id) VALUES ($1, $2)")
                .bind(id)
                .bind(domain_id)
                .execute(&mut *tx)
                .await?;
        }
    }

    if make_default {
        sqlx::query(
            "UPDATE user_groups SET is_default = 0, updated_at = $1
              WHERE is_default = 1 AND id <> $2",
        )
        .bind(&now)
        .bind(id)
        .execute(&mut *tx)
        .await?;
        sqlx::query("UPDATE user_groups SET is_default = 1, updated_at = $1 WHERE id = $2")
            .bind(&now)
            .bind(id)
            .execute(&mut *tx)
            .await?;
    }

    tx.commit().await?;
    Ok(true)
}

/// The explicit domain allowlist of one group.
pub async fn domain_ids(pool: &Pool, group_id: i64) -> Result<Vec<i64>, sqlx::Error> {
    sqlx::query_scalar("SELECT domain_id FROM group_domains WHERE group_id = $1 ORDER BY domain_id")
        .bind(group_id)
        .fetch_all(pool)
        .await
}

/// Domain allowlists for every group, keyed by group id.
pub async fn domain_ids_by_group(pool: &Pool) -> Result<HashMap<i64, Vec<i64>>, sqlx::Error> {
    let rows: Vec<(i64, i64)> = sqlx::query_as(
        "SELECT group_id, domain_id FROM group_domains ORDER BY group_id, domain_id",
    )
    .fetch_all(pool)
    .await?;
    let mut map: HashMap<i64, Vec<i64>> = HashMap::new();
    for (group_id, domain_id) in rows {
        map.entry(group_id).or_default().push(domain_id);
    }
    Ok(map)
}

/// Move every member of `id` (including soft-deleted accounts) to `target`, then
/// delete the group. Invite codes pointing at it are set to NULL by the foreign
/// key, which means "fall back to the default group".
///
/// The `is_default = 0` predicate is checked inside the transaction: another
/// administrator may have moved the default marker here after the service-level
/// check, and deleting it then would leave the instance without a default. The
/// member reassignment is rolled back with it. Returns `false` when the group is
/// (or became) the default.
pub async fn delete_with_reassign(pool: &Pool, id: i64, target: i64) -> Result<bool, sqlx::Error> {
    let now = now_iso();
    let mut tx = crate::db::begin_write(pool).await?;
    sqlx::query("UPDATE users SET group_id = $1, updated_at = $2 WHERE group_id = $3")
        .bind(target)
        .bind(&now)
        .bind(id)
        .execute(&mut *tx)
        .await?;
    let deleted = sqlx::query("DELETE FROM user_groups WHERE id = $1 AND is_default = 0")
        .bind(id)
        .execute(&mut *tx)
        .await?;
    if deleted.rows_affected() == 0 {
        tx.rollback().await?;
        return Ok(false);
    }
    tx.commit().await?;
    Ok(true)
}
