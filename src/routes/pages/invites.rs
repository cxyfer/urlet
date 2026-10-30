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
use crate::routes::no_store;
use crate::routes::pages::{
    flash_from_query, page_links, redirect_err, redirect_ok, redirect_to_login, Chrome, Flash,
    PageLink,
};
use crate::services::invite_quota::usage_for;
use crate::services::invites::{self as invite_service, Issuer, MAX_MEMBER_USES};
use crate::services::ratelimit::limits;
use crate::services::time;
use crate::state::AppState;
use crate::themed_page;

#[derive(Debug)]
pub struct InvitesPage {
    pub site_name: String,
    pub chrome: Chrome,
    pub lang: Lang,
    /// Whether this account may issue codes right now.
    pub allowed: bool,
    pub group_name: String,
    /// Localised "2 of 5 per month", or "Unlimited".
    pub quota_label: String,
    pub items: Vec<InviteRow>,
    /// Plaintext code created by the current request, so the issuer can copy
    /// it straight away.
    pub new_code: Option<String>,
    /// Registration link for `new_code`.
    pub new_link: Option<String>,
    /// At least one row cannot show its code again (legacy row or rotated
    /// secret); the page explains why.
    pub has_legacy: bool,
    pub max_uses_cap: i64,
    pub page: i64,
    pub total_pages: i64,
    pub total: i64,
    pub links: Vec<PageLink>,
    /// Localised "Page 1 of 3".
    pub page_note: String,
}

themed_page!(invites_templates, InvitesView, InvitesPage,
    Verdant => "invites.html",
    Aurora => "themes/aurora/invites.html",
    Paper => "themes/paper/invites.html",
    Signal => "themes/signal/invites.html",
    Gilded => "themes/gilded/invites.html",
    Swiss => "themes/swiss/invites.html",
    Cosmos => "themes/cosmos/invites.html",
);

#[derive(Debug, Clone)]
pub struct InviteRow {
    pub id: i64,
    pub prefix: String,
    /// Recoverable plaintext, or `None` for a legacy/sealed-unreadable row.
    pub code: Option<String>,
    /// Registration link when `code` is available.
    pub link: Option<String>,
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
    #[serde(default, deserialize_with = "crate::routes::dto::optional_i64")]
    pub page: Option<i64>,
}

#[derive(Debug, Deserialize)]
pub struct CreateInviteForm {
    #[serde(default)]
    pub note: Option<String>,
    #[serde(default)]
    pub max_uses: Option<String>,
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
        query.page.unwrap_or(1).max(1),
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

    if state
        .limiter
        .check(
            &format!("create-invite:{}", current.user.id),
            limits::CREATE_INVITE_CAPACITY,
            limits::CREATE_INVITE_REFILL_PER_SEC,
        )
        .is_err()
    {
        return Ok(redirect_err("/app/invites", "rate_limited").into_response());
    }

    let outcome = Issuer {
        pool: &state.db,
        settings: &state.settings,
        actor: &current.user,
        ip_hash: client.ip_hash.as_deref(),
        secret: state.config.secret.as_slice(),
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
                1,
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
        secret: state.config.secret.as_slice(),
    }
    .revoke_owned(id)
    .await;
    Ok(match outcome {
        Ok(()) => redirect_ok("/app/invites", "invite_revoked").into_response(),
        Err(err) => redirect_err("/app/invites", super::urls::error_code(&err)).into_response(),
    })
}

const PER_PAGE: i64 = 20;

async fn render(
    state: &AppState,
    user: &crate::db::users::User,
    flash: Option<Flash>,
    new_code: Option<String>,
    page: i64,
) -> Result<Response, AppError> {
    let lang = i18n::current();
    let now = time::now_iso();
    let group = invite_service::member_group(&state.db, user).await?;
    let allowed = group
        .as_ref()
        .map(|group| group.allow_invites)
        .unwrap_or(false);
    let usage = usage_for(&state.db, &state.settings, group.as_ref(), time::now()).await?;
    let quota_label = if usage.unlimited() {
        lang.t("invites.unlimited").to_string()
    } else {
        i18n::format(
            lang,
            "invites.quota",
            &[
                ("used", usage.used.to_string()),
                ("limit", usage.quota.limit.to_string()),
                (
                    "period",
                    crate::routes::pages::period_label(lang, usage.quota.period),
                ),
            ],
        )
    };

    let total = crate::db::invites::count_by_creator(&state.db, user.id)
        .await
        .map_err(AppError::from)?;
    let total_pages = ((total + PER_PAGE - 1) / PER_PAGE).max(1);
    let rows =
        crate::db::invites::list_by_creator(&state.db, user.id, PER_PAGE, (page - 1) * PER_PAGE)
            .await
            .map_err(AppError::from)?;
    let mut has_legacy = false;
    let invites: Vec<InviteRow> = rows
        .iter()
        .map(|invite: &Invite| {
            let usable = invite.is_usable(&now);
            let code = invite_service::reveal(state.config.secret.as_slice(), invite);
            if code.is_none() {
                has_legacy = true;
            }
            // Only a usable code gets a link; the table and the create banner
            // must both hide a URL that redemption would reject.
            let link = if usable {
                code.as_deref()
                    .map(|code| invite_service::registration_link(&state.config.base_url, code))
            } else {
                None
            };
            InviteRow {
                id: invite.id,
                prefix: invite.code_prefix.clone(),
                code,
                link,
                note: invite.note.clone().unwrap_or_default(),
                uses: format!("{} / {}", invite.used_count, invite.max_uses),
                status: if invite.status == "active" {
                    lang.t("admin.invites.status.active").to_string()
                } else {
                    lang.t("admin.invites.status.revoked").to_string()
                },
                usable,
                expires: invite
                    .expires_at
                    .as_deref()
                    .map(|value| time::humanize_lang(lang, value))
                    .unwrap_or_else(|| lang.t("common.never").to_string()),
                created: time::humanize_lang(lang, &invite.created_at),
            }
        })
        .collect();

    let new_link = new_code.as_deref().and_then(|code| {
        invites
            .iter()
            .find(|row| row.code.as_deref() == Some(code))
            .and_then(|row| row.link.clone())
    });

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
        items: invites,
        new_code,
        new_link,
        has_legacy,
        max_uses_cap: MAX_MEMBER_USES,
        page,
        total_pages,
        total,
        links: page_links("/app/invites", &[], page, total_pages),
        page_note: i18n::format(
            lang,
            "common.page_of",
            &[
                ("page", page.to_string()),
                ("total", total_pages.to_string()),
            ],
        ),
    }
    .themed()
    .render()
    .map_err(|err| AppError::internal(format!("template error: {err}")))?;
    Ok(no_store((StatusCode::OK, Html(html)).into_response()))
}
