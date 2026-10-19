//! Member invite page: the signed-in user's own invite codes.
//!
//! A member may issue codes for their own group when the group enables
//! `allow_invites`. The page is always reachable so the quota and the reason for
//! being blocked stay discoverable; the form is only rendered when allowed.

use askama::Template;
use axum::extract::rejection::FormRejection;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::{Html, IntoResponse, Response};
use axum::Form;
use serde::Deserialize;

use crate::db::invites::Invite;
use crate::error::AppError;
use crate::i18n::{self, Lang};
use crate::middleware::auth::{ClientInfo, MaybeUser};
use crate::routes::pages::{
    flash_from_query, redirect_err, redirect_ok, redirect_to_login, Chrome, Flash,
};
use crate::services::invite_quota::{InvitePeriod, InviteQuota, InviteUsage};
use crate::services::invites::{self as invite_service, Issuer, MAX_MEMBER_USES};
use crate::services::time;
use crate::state::AppState;

#[derive(Debug, Template)]
#[template(path = "invites.html")]
pub struct InvitesPage {
    pub site_name: String,
    pub chrome: Chrome,
    pub lang: Lang,
    /// Whether this account may issue codes right now.
    pub allowed: bool,
    pub group_name: String,
    /// Localised "2 of 5 per month", or "Unlimited".
    pub quota_label: String,
    pub remaining: Option<i64>,
    pub items: Vec<InviteRow>,
    /// Plaintext code created by the current request (shown once).
    pub new_code: Option<String>,
    pub max_uses_cap: i64,
}

#[derive(Debug, Clone)]
pub struct InviteRow {
    pub id: i64,
    pub prefix: String,
    pub note: String,
    pub uses: String,
    pub status: String,
    pub usable: bool,
    pub expires: String,
    pub created: String,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct InvitesQuery {
    #[serde(default)]
    pub ok: Option<String>,
    #[serde(default)]
    pub err: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct CreateInviteForm {
    #[serde(default)]
    pub note: Option<String>,
    #[serde(default)]
    pub max_uses: Option<String>,
}

fn period_label(lang: Lang, period: InvitePeriod) -> String {
    match period {
        InvitePeriod::Day => lang.t("invites.period.day").to_string(),
        InvitePeriod::Week => lang.t("invites.period.week").to_string(),
        InvitePeriod::Month => lang.t("invites.period.month").to_string(),
    }
}

/// `GET /app/invites`
pub async fn page(
    State(state): State<AppState>,
    MaybeUser(current): MaybeUser,
    Query(query): Query<InvitesQuery>,
) -> Result<Response, AppError> {
    let Some(current) = current else {
        return Ok(redirect_to_login(Some("/app/invites")).into_response());
    };
    render(
        &state,
        &current.user,
        flash_from_query(query.ok.as_deref(), query.err.as_deref()),
        None,
    )
    .await
}

/// `POST /app/invites`
pub async fn create(
    State(state): State<AppState>,
    client: ClientInfo,
    MaybeUser(current): MaybeUser,
    form: std::result::Result<Form<CreateInviteForm>, FormRejection>,
) -> Result<Response, AppError> {
    let Some(current) = current else {
        return Ok(redirect_to_login(Some("/app/invites")).into_response());
    };
    let Ok(Form(payload)) = form else {
        return Ok(redirect_err("/app/invites", "invalid").into_response());
    };
    let max_uses = match payload
        .max_uses
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        None => None,
        Some(value) => match value.parse::<i64>() {
            Ok(value) => Some(value),
            Err(_) => return Ok(redirect_err("/app/invites", "invalid").into_response()),
        },
    };

    let outcome = Issuer {
        pool: &state.db,
        settings: &state.settings,
        actor: &current.user,
        ip_hash: client.ip_hash.as_deref(),
    }
    .create_for_member(payload.note.as_deref(), max_uses)
    .await;

    match outcome {
        Ok(created) => {
            render(
                &state,
                &current.user,
                Some(Flash {
                    kind: "ok",
                    message: i18n::current().t("invites.created_flash").to_string(),
                }),
                Some(created.plain_code),
            )
            .await
        }
        Err(err) => Ok(redirect_err("/app/invites", super::urls::error_code(&err)).into_response()),
    }
}

/// `POST /app/invites/{id}/delete`
pub async fn revoke(
    State(state): State<AppState>,
    client: ClientInfo,
    MaybeUser(current): MaybeUser,
    Path(id): Path<i64>,
) -> Result<Response, AppError> {
    let Some(current) = current else {
        return Ok(redirect_to_login(Some("/app/invites")).into_response());
    };
    let outcome = Issuer {
        pool: &state.db,
        settings: &state.settings,
        actor: &current.user,
        ip_hash: client.ip_hash.as_deref(),
    }
    .revoke_owned(id)
    .await;
    Ok(match outcome {
        Ok(()) => redirect_ok("/app/invites", "invite_revoked").into_response(),
        Err(err) => redirect_err("/app/invites", super::urls::error_code(&err)).into_response(),
    })
}

async fn render(
    state: &AppState,
    user: &crate::db::users::User,
    flash: Option<Flash>,
    new_code: Option<String>,
) -> Result<Response, AppError> {
    let lang = i18n::current();
    let now = time::now_iso();
    let group = invite_service::member_group(&state.db, user).await?;
    let allowed = group
        .as_ref()
        .map(|group| group.allow_invites)
        .unwrap_or(false);
    let quota = InviteQuota::resolve(&state.settings, group.as_ref());
    let usage = InviteUsage::load(
        quota,
        &state.db,
        group.as_ref().map(|group| group.id),
        time::now(),
    )
    .await?;
    let quota_label = if usage.unlimited() {
        lang.t("invites.unlimited").to_string()
    } else {
        i18n::format(
            lang,
            "invites.quota",
            &[
                ("used", usage.used.to_string()),
                ("limit", usage.quota.limit.to_string()),
                ("period", period_label(lang, usage.quota.period)),
            ],
        )
    };

    let rows = crate::db::invites::list_by_creator(&state.db, user.id, 100)
        .await
        .map_err(AppError::from)?;
    let invites = rows
        .iter()
        .map(|invite: &Invite| {
            let expired = invite
                .expires_at
                .as_deref()
                .map(|value| value <= now.as_str())
                .unwrap_or(false);
            InviteRow {
                id: invite.id,
                prefix: invite.code_prefix.clone(),
                note: invite.note.clone().unwrap_or_default(),
                uses: format!("{} / {}", invite.used_count, invite.max_uses),
                status: if invite.status == "active" {
                    lang.t("admin.invites.status.active").to_string()
                } else {
                    lang.t("admin.invites.status.revoked").to_string()
                },
                usable: invite.status == "active" && !invite.is_exhausted() && !expired,
                expires: invite
                    .expires_at
                    .as_deref()
                    .map(|value| time::humanize_lang(lang, value))
                    .unwrap_or_else(|| lang.t("common.never").to_string()),
                created: time::humanize_lang(lang, &invite.created_at),
            }
        })
        .collect();

    let html = InvitesPage {
        site_name: state.settings.site_name(),
        chrome: Chrome::new(state, user, "invites", flash),
        lang,
        allowed,
        group_name: group
            .as_ref()
            .map(|group| group.name.clone())
            .unwrap_or_default(),
        quota_label,
        remaining: usage.remaining(),
        items: invites,
        new_code,
        max_uses_cap: MAX_MEMBER_USES,
    }
    .render()
    .map_err(|err| AppError::internal(format!("template error: {err}")))?;
    Ok((StatusCode::OK, Html(html)).into_response())
}
