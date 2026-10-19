//! `/api/v1/invites` — invite codes issued by the signed-in member.
//!
//! Members may issue codes for their own group when the group has
//! `allow_invites` enabled. The group's calendar quota (`max_invites` /
//! `invite_period`) and the shared issuance ledger apply exactly as they do for
//! administrators, so both paths draw from the same budget.

use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde::{Deserialize, Serialize};

use crate::db::invites::Invite;
use crate::error::ApiError;
use crate::middleware::auth::{ClientInfo, CurrentUser};
use crate::routes::dto::Pagination;
use crate::services::invite_quota::{usage_for, InviteQuota};
use crate::services::invites::{self, Issuer};
use crate::services::ratelimit::limits;
use crate::services::time;
use crate::state::AppState;

/// One invite code, without the plaintext value.
#[derive(Debug, Clone, Serialize)]
pub struct MemberInviteDto {
    pub id: i64,
    pub prefix: String,
    pub note: Option<String>,
    pub group_id: Option<i64>,
    pub max_uses: i64,
    pub used_count: i64,
    pub remaining: i64,
    pub expires_at: Option<String>,
    pub status: String,
    pub usable: bool,
    pub created_at: String,
}

impl MemberInviteDto {
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
            max_uses: invite.max_uses,
            used_count: invite.used_count,
            remaining: (invite.max_uses - invite.used_count).max(0),
            expires_at: invite.expires_at.clone(),
            status: invite.status.clone(),
            usable: invite.status == "active" && !invite.is_exhausted() && !expired,
            created_at: invite.created_at.clone(),
        }
    }
}

/// The caller's effective invite quota for the current period.
#[derive(Debug, Clone, Serialize)]
pub struct MemberQuotaDto {
    pub group_id: Option<i64>,
    pub group_name: Option<String>,
    /// Whether this account may issue codes right now.
    pub allowed: bool,
    /// `0` means unlimited.
    pub limit: i64,
    pub period: String,
    pub used: i64,
    /// `null` when unlimited.
    pub remaining: Option<i64>,
}

#[derive(Debug, Serialize)]
pub struct MemberInviteList {
    pub items: Vec<MemberInviteDto>,
    pub page: i64,
    pub per_page: i64,
    pub total: i64,
    pub quota: MemberQuotaDto,
}

#[derive(Debug, Deserialize)]
pub struct CreateMemberInvitePayload {
    #[serde(default)]
    pub note: Option<String>,
    #[serde(default)]
    pub max_uses: Option<i64>,
}

#[derive(Debug, Serialize)]
pub struct CreatedMemberInvite {
    /// Plaintext code — returned exactly once.
    pub code: String,
    pub invite: MemberInviteDto,
    pub quota: MemberQuotaDto,
}

async fn quota_for(
    state: &AppState,
    user: &crate::db::users::User,
) -> Result<MemberQuotaDto, ApiError> {
    let group = invites::member_group(&state.db, user).await?;
    let quota = InviteQuota::resolve(&state.settings, group.as_ref());
    let usage = usage_for(&state.db, &state.settings, group.as_ref(), time::now()).await?;
    Ok(MemberQuotaDto {
        group_id: group.as_ref().map(|group| group.id),
        group_name: group.as_ref().map(|group| group.name.clone()),
        allowed: group
            .as_ref()
            .map(|group| group.allow_invites)
            .unwrap_or(false),
        limit: quota.limit,
        period: quota.period.as_str().to_string(),
        used: usage.used,
        remaining: usage.remaining(),
    })
}

/// `GET /api/v1/invites`
pub async fn list(
    State(state): State<AppState>,
    CurrentUser { user, .. }: CurrentUser,
    Query(pagination): Query<Pagination>,
) -> Result<Json<MemberInviteList>, ApiError> {
    let per_page = pagination.per_page();
    let rows =
        crate::db::invites::list_by_creator(&state.db, user.id, per_page, pagination.offset())
            .await?;
    let total = crate::db::invites::count_by_creator(&state.db, user.id).await?;
    let now = time::now_iso();
    Ok(Json(MemberInviteList {
        items: rows
            .iter()
            .map(|invite| MemberInviteDto::new(invite, &now))
            .collect(),
        page: pagination.page(),
        per_page,
        total,
        quota: quota_for(&state, &user).await?,
    }))
}

/// `POST /api/v1/invites`
pub async fn create(
    State(state): State<AppState>,
    client: ClientInfo,
    CurrentUser { user, .. }: CurrentUser,
    Json(payload): Json<CreateMemberInvitePayload>,
) -> Result<Response, ApiError> {
    state
        .limiter
        .check(
            &format!("create-invite:{}", user.id),
            limits::CREATE_INVITE_CAPACITY,
            limits::CREATE_INVITE_REFILL_PER_SEC,
        )
        .map_err(|retry| ApiError(crate::error::AppError::rate_limited(retry)))?;

    let created = Issuer {
        pool: &state.db,
        settings: &state.settings,
        actor: &user,
        ip_hash: client.ip_hash.as_deref(),
    }
    .create_for_member(payload.note.as_deref(), payload.max_uses)
    .await?;

    let body = CreatedMemberInvite {
        code: created.plain_code,
        invite: MemberInviteDto::new(&created.invite, &time::now_iso()),
        quota: quota_for(&state, &user).await?,
    };
    Ok((StatusCode::CREATED, Json(body)).into_response())
}

/// `DELETE /api/v1/invites/{id}` — revoke one of the caller's own codes.
pub async fn revoke(
    State(state): State<AppState>,
    client: ClientInfo,
    CurrentUser { user, .. }: CurrentUser,
    Path(id): Path<i64>,
) -> Result<Response, ApiError> {
    Issuer {
        pool: &state.db,
        settings: &state.settings,
        actor: &user,
        ip_hash: client.ip_hash.as_deref(),
    }
    .revoke_owned(id)
    .await?;
    Ok(StatusCode::NO_CONTENT.into_response())
}
