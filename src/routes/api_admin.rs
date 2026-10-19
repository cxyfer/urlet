//! `/api/v1/admin/**` — administrative JSON API.

use std::collections::HashMap;

use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde::{Deserialize, Deserializer, Serialize};

use crate::db::audit::{AuditFilter, AuditLog};
use crate::db::domains::{Domain, DomainWithCount};
use crate::db::invites::Invite;
use crate::db::urls::{AdminUrlFilter, UrlSort};
use crate::db::user_groups::UserGroup;
use crate::db::users::{User, UserFilter};
use crate::db::{tags, urls};
use crate::error::{ApiError, AppError};
use crate::middleware::auth::{ClientInfo, CurrentUser};
use crate::routes::dto::{Page, Pagination, UrlDto, UserDto};
use crate::services::admin::{self, AdminContext, SystemStats};
use crate::services::domains::DomainContext;
use crate::services::groups::{self as group_service, GroupDraft};
use crate::services::invite_quota::{InvitePeriod, InviteQuota, InviteUsage};
use crate::services::time;
use crate::state::AppState;

/// Query for the admin user listing.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct AdminUsersQuery {
    #[serde(default, deserialize_with = "crate::routes::dto::optional_i64")]
    pub page: Option<i64>,
    #[serde(default, deserialize_with = "crate::routes::dto::optional_i64")]
    pub per_page: Option<i64>,
    #[serde(default)]
    pub q: Option<String>,
    #[serde(default)]
    pub role: Option<String>,
    #[serde(default)]
    pub status: Option<String>,
    #[serde(default, deserialize_with = "crate::routes::dto::optional_i64")]
    pub group_id: Option<i64>,
}

/// Query for the admin link listing.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct AdminUrlsQuery {
    #[serde(default, deserialize_with = "crate::routes::dto::optional_i64")]
    pub page: Option<i64>,
    #[serde(default, deserialize_with = "crate::routes::dto::optional_i64")]
    pub per_page: Option<i64>,
    #[serde(default)]
    pub q: Option<String>,
    #[serde(default)]
    pub status: Option<String>,
    #[serde(default, deserialize_with = "crate::routes::dto::optional_i64")]
    pub user_id: Option<i64>,
    #[serde(default)]
    pub sort: Option<String>,
}

/// Query for the audit listing.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct AdminAuditQuery {
    #[serde(default, deserialize_with = "crate::routes::dto::optional_i64")]
    pub page: Option<i64>,
    #[serde(default, deserialize_with = "crate::routes::dto::optional_i64")]
    pub per_page: Option<i64>,
    #[serde(default)]
    pub action: Option<String>,
    #[serde(default)]
    pub target_type: Option<String>,
    #[serde(default, deserialize_with = "crate::routes::dto::optional_i64")]
    pub actor: Option<i64>,
}

/// Query for the admin group listing.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct AdminGroupsQuery {
    #[serde(default, deserialize_with = "crate::routes::dto::optional_i64")]
    pub page: Option<i64>,
    #[serde(default, deserialize_with = "crate::routes::dto::optional_i64")]
    pub per_page: Option<i64>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct UpdateUserPayload {
    #[serde(default)]
    pub role: Option<String>,
    #[serde(default)]
    pub status: Option<String>,
    #[serde(default)]
    pub display_name: Option<String>,
    /// Absent = unchanged, `null` = ungroup (site defaults apply), a value = assign.
    #[serde(default, deserialize_with = "deserialize_double_option")]
    pub group_id: Option<Option<i64>>,
}

/// Distinguish "field absent" (`None`) from an explicit JSON `null` (`Some(None)`).
fn deserialize_double_option<'de, D>(deserializer: D) -> Result<Option<Option<i64>>, D::Error>
where
    D: Deserializer<'de>,
{
    Option::<i64>::deserialize(deserializer).map(Some)
}

#[derive(Debug, Clone, Deserialize)]
pub struct ResetPasswordPayload {
    pub new_password: String,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct UpdateUrlPayload {
    #[serde(default)]
    pub status: Option<String>,
    #[serde(default)]
    pub redirect_type: Option<i64>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct CreateInvitePayload {
    #[serde(default)]
    pub note: Option<String>,
    #[serde(default)]
    pub group_id: Option<i64>,
    #[serde(default)]
    pub max_uses: Option<i64>,
    #[serde(default)]
    pub expires_at: Option<String>,
}

/// User plus the number of links they own.
#[derive(Debug, Clone, Serialize)]
pub struct AdminUserDto {
    #[serde(flatten)]
    pub user: UserDto,
    pub link_count: i64,
    pub group_name: Option<String>,
}

/// Query for deleting a group.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct DeleteGroupQuery {
    #[serde(default, deserialize_with = "crate::routes::dto::optional_i64")]
    pub reassign_to: Option<i64>,
}

/// Group representation, including its live member count.
#[derive(Debug, Clone, Serialize)]
pub struct GroupDto {
    pub id: i64,
    pub name: String,
    pub description: Option<String>,
    pub max_urls: Option<i64>,
    pub max_api_tokens: Option<i64>,
    /// Invite codes per `invite_period`; `null` inherits the site setting and
    /// `0` means unlimited.
    pub max_invites: Option<i64>,
    pub invite_period: String,
    /// Whether the group's members may issue their own invite codes.
    pub allow_invites: bool,
    /// Issued invite codes in the current period.
    pub invites_used: i64,
    /// Slots left in the current period; `null` when unlimited.
    pub invites_remaining: Option<i64>,
    pub allow_custom_slug: bool,
    pub allow_password: bool,
    pub allow_expiry: bool,
    pub allow_max_clicks: bool,
    pub allow_tags: bool,
    pub allow_api_tokens: bool,
    pub allow_domain_selection: bool,
    pub default_domain_id: Option<i64>,
    pub default_domain: Option<String>,
    pub restrict_domains: bool,
    pub allowed_domain_ids: Vec<i64>,
    pub allowed_domains: Vec<String>,
    pub is_default: bool,
    pub members: i64,
    pub created_at: String,
    pub updated_at: String,
}

impl GroupDto {
    fn new(
        group: &UserGroup,
        members: i64,
        allowed_domain_ids: &[i64],
        domains: &crate::services::domains::DomainRegistry,
        invite_usage: InviteUsage,
    ) -> Self {
        Self {
            id: group.id,
            name: group.name.clone(),
            description: group.description.clone(),
            max_urls: group.max_urls,
            max_api_tokens: group.max_api_tokens,
            max_invites: group.max_invites,
            invite_period: group.invite_period.clone(),
            allow_invites: group.allow_invites,
            invites_used: invite_usage.used,
            invites_remaining: invite_usage.remaining(),
            allow_custom_slug: group.allow_custom_slug,
            allow_password: group.allow_password,
            allow_expiry: group.allow_expiry,
            allow_max_clicks: group.allow_max_clicks,
            allow_tags: group.allow_tags,
            allow_api_tokens: group.allow_api_tokens,
            allow_domain_selection: group.allow_domain_selection,
            default_domain_id: group.default_domain_id,
            default_domain: group.default_domain_id.and_then(|id| domains.hostname(id)),
            restrict_domains: group.restrict_domains,
            allowed_domain_ids: allowed_domain_ids.to_vec(),
            allowed_domains: allowed_domain_ids
                .iter()
                .filter_map(|id| domains.hostname(*id))
                .collect(),
            is_default: group.is_default,
            members,
            created_at: group.created_at.clone(),
            updated_at: group.updated_at.clone(),
        }
    }
}

/// Invite usage snapshot for one group, used by the group API.
async fn invite_usage_for(state: &AppState, group: &UserGroup) -> Result<InviteUsage, ApiError> {
    let quota = InviteQuota::resolve(&state.settings, Some(group));
    InviteUsage::load(quota, &state.db, Some(group.id), time::now())
        .await
        .map_err(ApiError::from)
}

/// Group fields. `name` is required; other omitted fields keep their current
/// value on `PUT` and take the create defaults on `POST`.
#[derive(Debug, Clone, Deserialize)]
pub struct GroupPayload {
    pub name: String,
    /// `null` clears the description; omitting it keeps the current one.
    #[serde(default, deserialize_with = "crate::routes::dto::double_option")]
    pub description: Option<Option<String>>,
    /// `null` inherits the site-wide quota; omitting it keeps the current value.
    #[serde(default, deserialize_with = "crate::routes::dto::double_option")]
    pub max_urls: Option<Option<i64>>,
    #[serde(default, deserialize_with = "crate::routes::dto::double_option")]
    pub max_api_tokens: Option<Option<i64>>,
    /// `null` inherits the site-wide invite quota; omitting it keeps the value.
    #[serde(default, deserialize_with = "crate::routes::dto::double_option")]
    pub max_invites: Option<Option<i64>>,
    /// `day`, `week` or `month`; omitted keeps the current period.
    #[serde(default)]
    pub invite_period: Option<String>,
    #[serde(default)]
    pub allow_invites: Option<bool>,
    #[serde(default)]
    pub allow_custom_slug: Option<bool>,
    #[serde(default)]
    pub allow_password: Option<bool>,
    #[serde(default)]
    pub allow_expiry: Option<bool>,
    #[serde(default)]
    pub allow_max_clicks: Option<bool>,
    #[serde(default)]
    pub allow_tags: Option<bool>,
    #[serde(default)]
    pub allow_api_tokens: Option<bool>,
    #[serde(default)]
    pub allow_domain_selection: Option<bool>,
    /// `null` means the site-wide default; omitting it keeps the current value.
    #[serde(default, deserialize_with = "crate::routes::dto::double_option")]
    pub default_domain_id: Option<Option<i64>>,
    #[serde(default)]
    pub restrict_domains: Option<bool>,
    /// `null` or `[]` removes every allowlist row; omitting it keeps the current list.
    #[serde(default, deserialize_with = "crate::routes::dto::double_option")]
    pub domain_ids: Option<Option<Vec<i64>>>,
    #[serde(default)]
    pub is_default: Option<bool>,
}

impl GroupPayload {
    /// Build a validated draft. `existing` is `None` on create (omitted fields
    /// take the create defaults) and `Some` on update (omitted fields keep the
    /// current value, so a PUT cannot silently re-enable a disabled toggle).
    fn into_draft(self, existing: Option<&UserGroup>) -> Result<GroupDraft, ApiError> {
        let current = |value: Option<bool>, get: fn(&UserGroup) -> bool| {
            value.unwrap_or_else(|| existing.map(get).unwrap_or(true))
        };
        Ok(GroupDraft {
            name: self.name,
            description: match self.description {
                Some(value) => value,
                None => existing.and_then(|group| group.description.clone()),
            },
            max_urls: match self.max_urls {
                Some(value) => value,
                None => existing.and_then(|group| group.max_urls),
            },
            max_api_tokens: match self.max_api_tokens {
                Some(value) => value,
                None => existing.and_then(|group| group.max_api_tokens),
            },
            max_invites: match self.max_invites {
                Some(value) => value,
                None => existing.and_then(|group| group.max_invites),
            },
            invite_period: match self.invite_period {
                Some(value) => InvitePeriod::parse(&value).ok_or_else(|| {
                    ApiError::from(AppError::validation(
                        "invite_period",
                        "invite period must be day, week or month",
                    ))
                })?,
                None => existing
                    .and_then(|group| InvitePeriod::parse(&group.invite_period))
                    .unwrap_or_default(),
            },
            // Inviting is opt-in: unlike the other toggles, the create default
            // is false so a new group cannot hand out codes by accident.
            allow_invites: self
                .allow_invites
                .or_else(|| existing.map(|group| group.allow_invites))
                .unwrap_or(false),
            allow_custom_slug: current(self.allow_custom_slug, |group| group.allow_custom_slug),
            allow_password: current(self.allow_password, |group| group.allow_password),
            allow_expiry: current(self.allow_expiry, |group| group.allow_expiry),
            allow_max_clicks: current(self.allow_max_clicks, |group| group.allow_max_clicks),
            allow_tags: current(self.allow_tags, |group| group.allow_tags),
            allow_api_tokens: current(self.allow_api_tokens, |group| group.allow_api_tokens),
            allow_domain_selection: current(self.allow_domain_selection, |group| {
                group.allow_domain_selection
            }),
            default_domain_id: match self.default_domain_id {
                Some(value) => value,
                None => existing.and_then(|group| group.default_domain_id),
            },
            restrict_domains: self
                .restrict_domains
                .or_else(|| existing.map(|group| group.restrict_domains))
                .unwrap_or(false),
            allowed_domain_ids: match self.domain_ids {
                None => None,
                Some(None) => Some(Vec::new()),
                Some(Some(ids)) => Some(ids),
            },
            make_default: self.is_default.unwrap_or(false),
        }
        .validate()?)
    }
}

/// Invite representation. The plain code is never stored.
#[derive(Debug, Clone, Serialize)]
pub struct InviteDto {
    pub id: i64,
    pub prefix: String,
    pub note: Option<String>,
    pub group_id: Option<i64>,
    pub group_name: Option<String>,
    pub created_by: Option<i64>,
    pub max_uses: i64,
    pub used_count: i64,
    pub remaining: i64,
    pub expires_at: Option<String>,
    pub status: String,
    pub created_at: String,
    pub usable: bool,
}

impl InviteDto {
    fn new(invite: &Invite, now: &str) -> Self {
        let expired = invite
            .expires_at
            .as_deref()
            .map(|value| value <= now)
            .unwrap_or(false);
        Self {
            id: invite.id,
            prefix: invite.code_prefix.clone(),
            note: invite.note.clone(),
            group_id: invite.group_id,
            group_name: None,
            created_by: invite.created_by,
            max_uses: invite.max_uses,
            used_count: invite.used_count,
            remaining: (invite.max_uses - invite.used_count).max(0),
            expires_at: invite.expires_at.clone(),
            status: invite.status.clone(),
            created_at: invite.created_at.clone(),
            usable: invite.status == "active" && !invite.is_exhausted() && !expired,
        }
    }

    fn with_group_name(mut self, group_name: Option<String>) -> Self {
        self.group_name = group_name;
        self
    }
}

/// Domain representation for the administrative API.
#[derive(Debug, Clone, Serialize)]
pub struct AdminDomainDto {
    pub id: i64,
    pub hostname: String,
    pub status: String,
    pub source: String,
    pub is_default: bool,
    pub is_verified: bool,
    pub link_count: i64,
    pub created_at: String,
    pub updated_at: Option<String>,
    pub can_activate: bool,
    pub can_disable: bool,
    pub can_delete: bool,
}

impl AdminDomainDto {
    fn from_domain(domain: &Domain, link_count: i64) -> Self {
        Self {
            id: domain.id,
            hostname: domain.hostname.clone(),
            status: domain.status.clone(),
            source: domain.source.clone(),
            is_default: domain.is_default(),
            is_verified: domain.is_verified == 1,
            link_count,
            created_at: domain.created_at.clone(),
            updated_at: domain.updated_at.clone(),
            can_activate: !domain.is_active(),
            can_disable: domain.is_active() && !domain.is_system() && !domain.is_default(),
            can_delete: domain.is_ui() && !domain.is_default() && link_count == 0,
        }
    }

    fn from_count(domain: &DomainWithCount) -> Self {
        Self {
            id: domain.id,
            hostname: domain.hostname.clone(),
            status: domain.status.clone(),
            source: domain.source.clone(),
            is_default: domain.is_default(),
            is_verified: domain.is_verified == 1,
            link_count: domain.link_count,
            created_at: domain.created_at.clone(),
            updated_at: domain.updated_at.clone(),
            can_activate: !domain.is_active(),
            can_disable: domain.is_active() && !domain.is_system() && !domain.is_default(),
            can_delete: domain.is_ui() && !domain.is_default() && domain.link_count == 0,
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct CreateDomainPayload {
    pub hostname: String,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct UpdateDomainPayload {
    #[serde(default)]
    pub status: Option<String>,
    #[serde(default)]
    pub is_default: Option<bool>,
}

/// Audit entry with parsed metadata.
#[derive(Debug, Clone, Serialize)]
pub struct AuditDto {
    pub id: i64,
    pub actor_user_id: Option<i64>,
    pub actor_role: Option<String>,
    pub action: String,
    pub target_type: Option<String>,
    pub target_id: Option<String>,
    pub metadata: Option<serde_json::Value>,
    pub created_at: String,
}

impl From<&AuditLog> for AuditDto {
    fn from(row: &AuditLog) -> Self {
        Self {
            id: row.id,
            actor_user_id: row.actor_user_id,
            actor_role: row.actor_role.clone(),
            action: row.action.clone(),
            target_type: row.target_type.clone(),
            target_id: row.target_id.clone(),
            metadata: row
                .metadata
                .as_deref()
                .and_then(|raw| serde_json::from_str(raw).ok()),
            created_at: row.created_at.clone(),
        }
    }
}

#[derive(Debug, Serialize)]
pub struct InviteList {
    pub items: Vec<InviteDto>,
}

#[derive(Debug, Serialize)]
pub struct CreatedInviteResponse {
    /// Plain invite code — returned exactly once.
    pub code: String,
    pub invite: InviteDto,
}

/// `GET /api/v1/admin/domains`
pub async fn list_domains(
    State(state): State<AppState>,
    CurrentUser { user, .. }: CurrentUser,
) -> Result<Json<Vec<AdminDomainDto>>, ApiError> {
    require_admin(&user)?;
    let rows = DomainContext {
        pool: &state.db,
        actor: &user,
        ip_hash: None,
    }
    .list()
    .await?;
    Ok(Json(rows.iter().map(AdminDomainDto::from_count).collect()))
}

/// `POST /api/v1/admin/domains`
pub async fn create_domain(
    State(state): State<AppState>,
    client: ClientInfo,
    CurrentUser { user, .. }: CurrentUser,
    Json(payload): Json<CreateDomainPayload>,
) -> Result<Response, ApiError> {
    require_admin(&user)?;
    let domain = DomainContext {
        pool: &state.db,
        actor: &user,
        ip_hash: client.ip_hash.as_deref(),
    }
    .create(&payload.hostname)
    .await?;
    reload_domains(&state).await?;
    Ok((
        StatusCode::CREATED,
        Json(AdminDomainDto::from_domain(&domain, 0)),
    )
        .into_response())
}

/// `PATCH /api/v1/admin/domains/{id}`
pub async fn update_domain(
    State(state): State<AppState>,
    client: ClientInfo,
    CurrentUser { user, .. }: CurrentUser,
    Path(id): Path<i64>,
    Json(payload): Json<UpdateDomainPayload>,
) -> Result<Json<AdminDomainDto>, ApiError> {
    require_admin(&user)?;

    if payload.is_default == Some(true) && payload.status.as_deref() == Some("disabled") {
        return Err(ApiError(AppError::validation(
            "is_default",
            "a domain cannot be disabled and made default in one request",
        )));
    }
    if payload.is_default == Some(false) {
        return Err(ApiError(AppError::validation(
            "is_default",
            "choose another domain to replace the default",
        )));
    }

    let context = DomainContext {
        pool: &state.db,
        actor: &user,
        ip_hash: client.ip_hash.as_deref(),
    };

    let mut domain = match payload.status.as_deref() {
        Some("active") => context.activate(id).await?,
        Some("disabled") => context.disable(id).await?,
        Some(other) => {
            return Err(ApiError(AppError::validation(
                "status",
                format!("unsupported status '{other}'"),
            )))
        }
        None => crate::db::domains::find_by_id(&state.db, id)
            .await?
            .ok_or_else(|| ApiError(AppError::not_found()))?,
    };

    if payload.is_default == Some(true) {
        domain = context.set_default(id).await?;
    }

    reload_domains(&state).await?;
    let links = crate::db::domains::link_count(&state.db, id).await?;
    Ok(Json(AdminDomainDto::from_domain(&domain, links)))
}

/// `DELETE /api/v1/admin/domains/{id}`
pub async fn delete_domain(
    State(state): State<AppState>,
    client: ClientInfo,
    CurrentUser { user, .. }: CurrentUser,
    Path(id): Path<i64>,
) -> Result<Response, ApiError> {
    require_admin(&user)?;
    DomainContext {
        pool: &state.db,
        actor: &user,
        ip_hash: client.ip_hash.as_deref(),
    }
    .delete(id)
    .await?;
    reload_domains(&state).await?;
    Ok(StatusCode::NO_CONTENT.into_response())
}

async fn reload_domains(state: &AppState) -> Result<(), ApiError> {
    state
        .domains
        .reload(&state.db)
        .await
        .map_err(|err| ApiError(AppError::internal(format!("cannot reload domains: {err}"))))
}

/// `GET /api/v1/admin/users`
pub async fn list_users(
    State(state): State<AppState>,
    CurrentUser { user, .. }: CurrentUser,
    Query(query): Query<AdminUsersQuery>,
) -> Result<Json<Page<AdminUserDto>>, ApiError> {
    require_admin(&user)?;
    let pagination = Pagination {
        page: query.page,
        per_page: query.per_page,
    };
    let filter = UserFilter {
        search: clean(&query.q),
        role: clean(&query.role),
        status: clean(&query.status),
        group_id: query.group_id,
    };

    let rows = admin::list_users(
        &state.db,
        &filter,
        pagination.per_page(),
        pagination.offset(),
    )
    .await?;
    let total = admin::count_users(&state.db, &filter).await?;
    let counts = admin::link_counts(&state.db).await?;
    let group_names = group_service::name_map(&state.db).await?;

    let items = rows
        .iter()
        .map(|row| AdminUserDto {
            user: UserDto::from(row),
            link_count: counts.get(&row.id).copied().unwrap_or(0),
            group_name: row
                .group_id
                .and_then(|group_id| group_names.get(&group_id).cloned()),
        })
        .collect();

    Ok(Json(Page::new(
        items,
        pagination.page(),
        pagination.per_page(),
        total,
    )))
}

/// `PATCH /api/v1/admin/users/{id}`
pub async fn update_user(
    State(state): State<AppState>,
    client: ClientInfo,
    CurrentUser { user, .. }: CurrentUser,
    Path(id): Path<i64>,
    Json(payload): Json<UpdateUserPayload>,
) -> Result<Json<UserDto>, ApiError> {
    require_admin(&user)?;
    let context = context(&state, &client, &user);
    let updated = context
        .update_user(
            id,
            payload.role.as_deref(),
            payload.status.as_deref(),
            payload.group_id,
            payload.display_name.as_deref().map(Some),
        )
        .await?;
    Ok(Json(UserDto::from(&updated)))
}

/// `POST /api/v1/admin/users/{id}/password`
pub async fn reset_password(
    State(state): State<AppState>,
    client: ClientInfo,
    CurrentUser { user, .. }: CurrentUser,
    Path(id): Path<i64>,
    Json(payload): Json<ResetPasswordPayload>,
) -> Result<Response, ApiError> {
    require_admin(&user)?;
    context(&state, &client, &user)
        .reset_password(id, &payload.new_password)
        .await?;
    Ok(StatusCode::NO_CONTENT.into_response())
}

/// `DELETE /api/v1/admin/users/{id}`
pub async fn delete_user(
    State(state): State<AppState>,
    client: ClientInfo,
    CurrentUser { user, .. }: CurrentUser,
    Path(id): Path<i64>,
) -> Result<Response, ApiError> {
    require_admin(&user)?;
    context(&state, &client, &user).delete_user(id).await?;
    Ok(StatusCode::NO_CONTENT.into_response())
}

/// `GET /api/v1/admin/urls`
pub async fn list_urls(
    State(state): State<AppState>,
    CurrentUser { user, .. }: CurrentUser,
    Query(query): Query<AdminUrlsQuery>,
) -> Result<Json<Page<UrlDto>>, ApiError> {
    require_admin(&user)?;
    let pagination = Pagination {
        page: query.page,
        per_page: query.per_page,
    };
    let filter = AdminUrlFilter {
        search: clean(&query.q),
        status: clean(&query.status),
        user_id: query.user_id,
        sort: UrlSort::parse(query.sort.as_deref()),
    };

    let rows = urls::list_all(
        &state.db,
        &filter,
        pagination.per_page(),
        pagination.offset(),
    )
    .await?;
    let total = urls::count_all(&state.db, &filter).await?;
    let ids: Vec<i64> = rows.iter().map(|row| row.id).collect();
    let mut tag_map = tags::for_urls(&state.db, &ids).await?;

    let items = rows
        .iter()
        .map(|row| {
            let names = tag_map
                .remove(&row.id)
                .unwrap_or_default()
                .into_iter()
                .map(|tag| tag.name)
                .collect();
            UrlDto::new(
                row,
                &state.domain_hostname(row.domain_id),
                &state.config.base_scheme,
                names,
            )
        })
        .collect();

    Ok(Json(Page::new(
        items,
        pagination.page(),
        pagination.per_page(),
        total,
    )))
}

/// `PATCH /api/v1/admin/urls/{id}`
pub async fn update_url(
    State(state): State<AppState>,
    client: ClientInfo,
    CurrentUser { user, .. }: CurrentUser,
    Path(id): Path<i64>,
    Json(payload): Json<UpdateUrlPayload>,
) -> Result<Json<UrlDto>, ApiError> {
    require_admin(&user)?;
    context(&state, &client, &user)
        .update_url(id, payload.status.as_deref(), payload.redirect_type)
        .await?;
    let url = urls::find_by_id(&state.db, id)
        .await?
        .ok_or_else(|| ApiError(AppError::not_found()))?;
    let names = tags::for_url(&state.db, url.id)
        .await?
        .into_iter()
        .map(|tag| tag.name)
        .collect();
    Ok(Json(UrlDto::new(
        &url,
        &state.domain_hostname(url.domain_id),
        &state.config.base_scheme,
        names,
    )))
}

/// `DELETE /api/v1/admin/urls/{id}`
pub async fn delete_url(
    State(state): State<AppState>,
    client: ClientInfo,
    CurrentUser { user, .. }: CurrentUser,
    Path(id): Path<i64>,
) -> Result<Response, ApiError> {
    require_admin(&user)?;
    context(&state, &client, &user).delete_url(id).await?;
    Ok(StatusCode::NO_CONTENT.into_response())
}

/// `GET /api/v1/admin/invites`
pub async fn list_invites(
    State(state): State<AppState>,
    CurrentUser { user, .. }: CurrentUser,
) -> Result<Json<InviteList>, ApiError> {
    require_admin(&user)?;
    let now = crate::services::time::now_iso();
    let rows = crate::db::invites::list(&state.db, 200, 0).await?;
    let group_names = group_service::name_map(&state.db).await?;
    Ok(Json(InviteList {
        items: rows
            .iter()
            .map(|row| {
                let group_name = row
                    .group_id
                    .and_then(|group_id| group_names.get(&group_id).cloned());
                InviteDto::new(row, &now).with_group_name(group_name)
            })
            .collect(),
    }))
}

/// `POST /api/v1/admin/invites`
pub async fn create_invite(
    State(state): State<AppState>,
    client: ClientInfo,
    CurrentUser { user, .. }: CurrentUser,
    Json(payload): Json<CreateInvitePayload>,
) -> Result<Response, ApiError> {
    require_admin(&user)?;
    let created = context(&state, &client, &user)
        .create_invite(
            payload.note.as_deref(),
            payload.group_id,
            payload.max_uses,
            payload.expires_at.as_deref(),
        )
        .await?;
    let now = crate::services::time::now_iso();
    let group_name = match created.invite.group_id {
        Some(group_id) => group_service::name_map(&state.db).await?.remove(&group_id),
        None => None,
    };
    let body = CreatedInviteResponse {
        code: created.plain_code,
        invite: InviteDto::new(&created.invite, &now).with_group_name(group_name),
    };
    Ok((StatusCode::CREATED, Json(body)).into_response())
}

/// `DELETE /api/v1/admin/invites/{id}`
pub async fn revoke_invite(
    State(state): State<AppState>,
    client: ClientInfo,
    CurrentUser { user, .. }: CurrentUser,
    Path(id): Path<i64>,
) -> Result<Response, ApiError> {
    require_admin(&user)?;
    context(&state, &client, &user).revoke_invite(id).await?;
    Ok(StatusCode::NO_CONTENT.into_response())
}

/// `GET /api/v1/admin/groups`
pub async fn list_groups(
    State(state): State<AppState>,
    CurrentUser { user, .. }: CurrentUser,
    Query(query): Query<AdminGroupsQuery>,
) -> Result<Json<Page<GroupDto>>, ApiError> {
    require_admin(&user)?;
    let pagination = Pagination {
        page: query.page,
        per_page: query.per_page,
    };
    let rows = group_service::overview(&state.db).await?;
    let total = rows.len() as i64;
    let offset = usize::try_from(pagination.offset().max(0)).unwrap_or(usize::MAX);
    let limit = usize::try_from(pagination.per_page().max(0)).unwrap_or(usize::MAX);
    let items = {
        let mut usage = HashMap::new();
        for row in &rows {
            usage.insert(row.group.id, invite_usage_for(&state, &row.group).await?);
        }
        rows.iter()
            .skip(offset)
            .take(limit)
            .map(|row| {
                GroupDto::new(
                    &row.group,
                    row.members,
                    &row.allowed_domain_ids,
                    &state.domains,
                    usage[&row.group.id],
                )
            })
            .collect()
    };
    Ok(Json(Page::new(
        items,
        pagination.page(),
        pagination.per_page(),
        total,
    )))
}

/// `POST /api/v1/admin/groups`
pub async fn create_group(
    State(state): State<AppState>,
    client: ClientInfo,
    CurrentUser { user, .. }: CurrentUser,
    Json(payload): Json<GroupPayload>,
) -> Result<Response, ApiError> {
    require_admin(&user)?;
    let draft = payload.into_draft(None)?;
    let group = context(&state, &client, &user).create_group(draft).await?;
    let members = crate::db::user_groups::count_members(&state.db, group.id).await?;
    let allowed_domain_ids = crate::db::user_groups::domain_ids(&state.db, group.id).await?;
    let usage = invite_usage_for(&state, &group).await?;
    Ok((
        StatusCode::CREATED,
        Json(GroupDto::new(
            &group,
            members,
            &allowed_domain_ids,
            &state.domains,
            usage,
        )),
    )
        .into_response())
}

/// `PUT /api/v1/admin/groups/{id}`
pub async fn update_group(
    State(state): State<AppState>,
    client: ClientInfo,
    CurrentUser { user, .. }: CurrentUser,
    Path(id): Path<i64>,
    Json(payload): Json<GroupPayload>,
) -> Result<Json<GroupDto>, ApiError> {
    require_admin(&user)?;
    let existing = crate::db::user_groups::find_by_id(&state.db, id)
        .await?
        .ok_or_else(AppError::not_found)?;
    let draft = payload.into_draft(Some(&existing))?;
    let group = context(&state, &client, &user)
        .update_group(id, draft)
        .await?;
    let members = crate::db::user_groups::count_members(&state.db, group.id).await?;
    let allowed_domain_ids = crate::db::user_groups::domain_ids(&state.db, group.id).await?;
    let usage = invite_usage_for(&state, &group).await?;
    Ok(Json(GroupDto::new(
        &group,
        members,
        &allowed_domain_ids,
        &state.domains,
        usage,
    )))
}

/// `DELETE /api/v1/admin/groups/{id}?reassign_to={id}`
pub async fn delete_group(
    State(state): State<AppState>,
    client: ClientInfo,
    CurrentUser { user, .. }: CurrentUser,
    Path(id): Path<i64>,
    Query(query): Query<DeleteGroupQuery>,
) -> Result<Response, ApiError> {
    require_admin(&user)?;
    context(&state, &client, &user)
        .delete_group(id, query.reassign_to)
        .await?;
    Ok(StatusCode::NO_CONTENT.into_response())
}

/// `GET /api/v1/admin/settings`
pub async fn get_settings(
    State(state): State<AppState>,
    CurrentUser { user, .. }: CurrentUser,
) -> Result<Json<HashMap<String, String>>, ApiError> {
    require_admin(&user)?;
    Ok(Json(state.settings.snapshot()))
}

/// `PUT /api/v1/admin/settings`
pub async fn put_settings(
    State(state): State<AppState>,
    client: ClientInfo,
    CurrentUser { user, .. }: CurrentUser,
    Json(payload): Json<HashMap<String, String>>,
) -> Result<Json<HashMap<String, String>>, ApiError> {
    require_admin(&user)?;
    let values: Vec<(String, String)> = payload.into_iter().collect();
    context(&state, &client, &user)
        .update_settings(&values)
        .await?;
    Ok(Json(state.settings.snapshot()))
}

/// `GET /api/v1/admin/audit`
pub async fn audit(
    State(state): State<AppState>,
    CurrentUser { user, .. }: CurrentUser,
    Query(query): Query<AdminAuditQuery>,
) -> Result<Json<Page<AuditDto>>, ApiError> {
    require_admin(&user)?;
    let pagination = Pagination {
        page: query.page,
        per_page: query.per_page,
    };
    let filter = AuditFilter {
        action: clean(&query.action),
        target_type: clean(&query.target_type),
        actor_user_id: query.actor,
    };
    let (rows, total) = admin::audit_page(
        &state.db,
        &filter,
        pagination.per_page(),
        pagination.offset(),
    )
    .await?;
    let items = rows.iter().map(AuditDto::from).collect();
    Ok(Json(Page::new(
        items,
        pagination.page(),
        pagination.per_page(),
        total,
    )))
}

/// `GET /api/v1/admin/stats`
pub async fn stats(
    State(state): State<AppState>,
    client: ClientInfo,
    CurrentUser { user, .. }: CurrentUser,
) -> Result<Json<SystemStats>, ApiError> {
    require_admin(&user)?;
    Ok(Json(context(&state, &client, &user).system_stats().await?))
}

fn context<'a>(state: &'a AppState, client: &'a ClientInfo, user: &'a User) -> AdminContext<'a> {
    AdminContext {
        pool: &state.db,
        config: &state.config,
        settings: &state.settings,
        actor: user,
        ip_hash: client.ip_hash.as_deref(),
        started_at: state.started_at,
    }
}

fn require_admin(user: &User) -> Result<(), ApiError> {
    if user.is_admin() {
        Ok(())
    } else {
        Err(ApiError(AppError::forbidden(
            "administrator privileges are required",
        )))
    }
}

fn clean(value: &Option<String>) -> Option<String> {
    value
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn invite_dto_computes_usability() {
        let invite = Invite {
            id: 1,
            code_hash: "hash".into(),
            code_prefix: "ABCD".into(),
            note: None,
            group_id: None,
            created_by: Some(1),
            max_uses: 2,
            used_count: 2,
            expires_at: None,
            status: "active".into(),
            created_at: "2026-01-01T00:00:00.000Z".into(),
        };
        let dto = InviteDto::new(&invite, "2026-01-01T00:00:00.000Z");
        assert_eq!(dto.remaining, 0);
        assert!(!dto.usable, "an exhausted invite is not usable");

        let mut fresh = invite.clone();
        fresh.used_count = 0;
        assert!(InviteDto::new(&fresh, "2026-01-01T00:00:00.000Z").usable);

        let mut expired = invite.clone();
        expired.used_count = 0;
        expired.expires_at = Some("2000-01-01T00:00:00.000Z".into());
        assert!(!InviteDto::new(&expired, "2026-01-01T00:00:00.000Z").usable);
    }
}
