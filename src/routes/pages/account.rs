//! Account page: profile, password, sessions and API tokens.

use askama::Template;
use axum::extract::rejection::FormRejection;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::{Html, IntoResponse, Response};
use axum::Form;
use serde::Deserialize;

use crate::db::{sessions, user_groups, users};
use crate::error::AppError;
use crate::i18n::{self, Lang};
use crate::middleware::auth::{ClientInfo, MaybeUser};
use crate::routes::dto::{SessionDto, UserDto};
use crate::routes::pages::{
    flash_from_query, redirect_err, redirect_ok, redirect_to_login, Chrome,
};
use crate::services::audit::{action, target, Audit};
use crate::services::auth::AuthContext;
use crate::services::ratelimit::limits;
use crate::services::{limits as effective_limits, time, tokens};
use crate::state::AppState;
use crate::themed_page;

#[derive(Debug)]
pub struct AccountPage {
    pub site_name: String,
    pub chrome: Chrome,
    pub user: UserDto,
    pub group_name: String,
    pub sessions: Vec<SessionRow>,
    pub tokens: Vec<TokenRow>,
    /// Localised "2 of 5 links used" line.
    pub links_usage: String,
    /// Localised "2 of 20 tokens used" line.
    pub tokens_usage: String,
    pub allow_api_tokens: bool,
    /// Plain value of a token created by the current request (shown once).
    pub new_token: Option<String>,
    pub max_name_len: usize,
    pub lang: Lang,
}

themed_page!(account_templates, AccountView, AccountPage,
    Verdant => "account.html",
    Aurora => "themes/aurora/account.html",
    Paper => "themes/paper/account.html",
    Signal => "themes/signal/account.html",
    Gilded => "themes/gilded/account.html",
    Swiss => "themes/swiss/account.html",
);

/// One row of the session table.
#[derive(Debug, Clone)]
pub struct SessionRow {
    pub id: i64,
    pub current: bool,
    pub agent: String,
    pub created: String,
    pub last_seen: String,
    pub expires: String,
}

/// One row of the API token table.
#[derive(Debug, Clone)]
pub struct TokenRow {
    pub id: i64,
    pub name: String,
    pub prefix: String,
    pub created: String,
    pub last_used: Option<String>,
    pub expires: Option<String>,
    pub active: bool,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct AccountQuery {
    #[serde(default)]
    pub ok: Option<String>,
    #[serde(default)]
    pub err: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct ProfileForm {
    #[serde(default)]
    pub display_name: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct PasswordForm {
    pub current_password: String,
    pub new_password: String,
}

#[derive(Debug, Deserialize)]
pub struct TokenForm {
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub expires_at: Option<String>,
}

/// `GET /app/account`
pub async fn page(
    State(state): State<AppState>,
    MaybeUser(current): MaybeUser,
    Query(query): Query<AccountQuery>,
) -> Result<Response, AppError> {
    let Some(current) = current else {
        return Ok(redirect_to_login(Some("/app/account")).into_response());
    };
    render_account(
        &state,
        &current.user,
        current.session_id,
        flash_from_query(query.ok.as_deref(), query.err.as_deref()),
        None,
    )
    .await
}

/// `POST /app/account` — update the display name.
pub async fn update_profile(
    State(state): State<AppState>,
    client: ClientInfo,
    MaybeUser(current): MaybeUser,
    form: std::result::Result<Form<ProfileForm>, FormRejection>,
) -> Result<Response, AppError> {
    let Some(current) = current else {
        return Ok(redirect_to_login(Some("/app/account")).into_response());
    };
    let Ok(Form(payload)) = form else {
        return Ok(redirect_err("/app/account", "invalid").into_response());
    };

    let display_name = payload
        .display_name
        .as_deref()
        .map(str::trim)
        .filter(|name| !name.is_empty());
    if let Some(name) = display_name {
        if let Err(err) = crate::services::auth::validate_display_name(name) {
            return Ok(redirect_err("/app/account", super::urls::error_code(&err)).into_response());
        }
    }

    users::set_display_name(&state.db, current.user.id, display_name)
        .await
        .map_err(AppError::from)?;
    Audit::new(action::PROFILE_UPDATED)
        .actor(current.user.id, &current.user.role)
        .target(target::USER, current.user.id)
        .ip_hash(client.ip_hash)
        .record(&state.db)
        .await;

    Ok(redirect_ok("/app/account", "profile").into_response())
}

/// `POST /app/account/password`
pub async fn change_password(
    State(state): State<AppState>,
    client: ClientInfo,
    MaybeUser(current): MaybeUser,
    form: std::result::Result<Form<PasswordForm>, FormRejection>,
) -> Result<Response, AppError> {
    let Some(current) = current else {
        return Ok(redirect_to_login(Some("/app/account")).into_response());
    };
    let Ok(Form(payload)) = form else {
        return Ok(redirect_err("/app/account", "invalid").into_response());
    };

    if state
        .limiter
        .check(
            &format!("password:{}", current.user.id),
            limits::PASSWORD_CAPACITY,
            limits::PASSWORD_REFILL_PER_SEC,
        )
        .is_err()
    {
        return Ok(redirect_err("/app/account", "rate_limited").into_response());
    }

    let context = AuthContext {
        pool: &state.db,
        config: &state.config,
        settings: &state.settings,
        user_agent: client.user_agent.as_deref(),
        ip_hash: client.ip_hash.as_deref(),
    };

    Ok(
        match context
            .change_password(
                &current.user,
                &payload.current_password,
                &payload.new_password,
                current.session_id,
            )
            .await
        {
            Ok(()) => redirect_ok("/app/account", "password").into_response(),
            Err(err) => redirect_err("/app/account", super::urls::error_code(&err)).into_response(),
        },
    )
}

/// `POST /app/account/sessions/{id}/delete`
pub async fn revoke_session(
    State(state): State<AppState>,
    client: ClientInfo,
    MaybeUser(current): MaybeUser,
    Path(id): Path<i64>,
) -> Result<Response, AppError> {
    let Some(current) = current else {
        return Ok(redirect_to_login(Some("/app/account")).into_response());
    };

    let revoked = sessions::revoke_owned(&state.db, id, current.user.id, &time::now_iso())
        .await
        .map_err(AppError::from)?;
    if !revoked {
        return Ok(redirect_err("/app/account", "not_found").into_response());
    }

    Audit::new(action::AUTH_SESSION_REVOKED)
        .actor(current.user.id, &current.user.role)
        .target(target::USER, current.user.id)
        .metadata(serde_json::json!({ "session_id": id }))
        .ip_hash(client.ip_hash)
        .record(&state.db)
        .await;

    // Signing out the current session returns to the login screen.
    if current.session_id == Some(id) {
        return Ok(redirect_ok("/app/login", "logout").into_response());
    }
    Ok(redirect_ok("/app/account", "session").into_response())
}

/// `POST /app/account/tokens`
pub async fn create_token(
    State(state): State<AppState>,
    client: ClientInfo,
    MaybeUser(current): MaybeUser,
    form: std::result::Result<Form<TokenForm>, FormRejection>,
) -> Result<Response, AppError> {
    let Some(current) = current else {
        return Ok(redirect_to_login(Some("/app/account")).into_response());
    };
    let Ok(Form(payload)) = form else {
        return Ok(redirect_err("/app/account", "invalid").into_response());
    };

    let name = payload.name.as_deref().unwrap_or("api token");
    match tokens::create(
        &state.db,
        &state.settings,
        &current.user,
        name,
        payload.expires_at.as_deref(),
        client.ip_hash.as_deref(),
    )
    .await
    {
        // Rendered inline so the plain token is shown exactly once.
        Ok(created) => {
            render_account(
                &state,
                &current.user,
                current.session_id,
                Some(crate::routes::pages::Flash {
                    kind: "ok",
                    message: i18n::current()
                        .t("account.tokens.created_flash")
                        .to_string(),
                }),
                Some(created.plain),
            )
            .await
        }
        Err(err) => Ok(redirect_err("/app/account", super::urls::error_code(&err)).into_response()),
    }
}

/// `POST /app/account/tokens/{id}/delete`
pub async fn revoke_token(
    State(state): State<AppState>,
    client: ClientInfo,
    MaybeUser(current): MaybeUser,
    Path(id): Path<i64>,
) -> Result<Response, AppError> {
    let Some(current) = current else {
        return Ok(redirect_to_login(Some("/app/account")).into_response());
    };

    match tokens::revoke(&state.db, &current.user, id, client.ip_hash.as_deref()).await {
        Ok(()) => Ok(redirect_ok("/app/account", "token_revoked").into_response()),
        Err(err) => Ok(redirect_err("/app/account", super::urls::error_code(&err)).into_response()),
    }
}

async fn render_account(
    state: &AppState,
    user: &crate::db::users::User,
    current_session: Option<i64>,
    flash: Option<crate::routes::pages::Flash>,
    new_token: Option<String>,
) -> Result<Response, AppError> {
    let session_rows = sessions::list_for_user(&state.db, user.id)
        .await
        .map_err(AppError::from)?;
    let token_rows = tokens::list(&state.db, user.id).await?;
    let fresh = users::find_by_id(&state.db, user.id)
        .await
        .map_err(AppError::from)?
        .unwrap_or_else(|| user.clone());

    let lang = i18n::current();
    let effective = effective_limits::effective(&state.db, &state.settings, &fresh).await?;
    let group_name = match fresh.group_id {
        Some(group_id) => user_groups::find_by_id(&state.db, group_id)
            .await
            .map_err(AppError::from)?
            .map(|group| group.name)
            .unwrap_or_default(),
        None => String::new(),
    };
    let now = time::now_iso();
    let link_used = crate::db::urls::count_for_user(&state.db, fresh.id)
        .await
        .map_err(AppError::from)?;
    let links_usage = i18n::format(
        lang,
        "dashboard.usage",
        &[
            ("used", link_used.to_string()),
            (
                "quota",
                if effective.max_urls > 0 {
                    effective.max_urls.to_string()
                } else {
                    lang.t("common.unlimited").to_string()
                },
            ),
        ],
    );
    // `tokens::list` is capped at 100 rows for the table; the usage meter must
    // count every usable token, not only the ones displayed.
    let token_used = crate::db::tokens::count_active_for_user(&state.db, fresh.id, &now)
        .await
        .map_err(AppError::from)?;
    let tokens_usage = i18n::format(
        lang,
        "account.tokens.usage",
        &[
            ("used", token_used.to_string()),
            (
                "quota",
                if effective.max_api_tokens > 0 {
                    effective.max_api_tokens.to_string()
                } else {
                    lang.t("common.unlimited").to_string()
                },
            ),
        ],
    );
    let page = AccountPage {
        site_name: state.settings.site_name(),
        chrome: Chrome::new(state, &fresh, "account", flash),
        user: UserDto::from(&fresh),
        group_name,
        links_usage,
        sessions: session_rows
            .iter()
            .map(|session| {
                let dto = SessionDto::new(session, current_session);
                SessionRow {
                    id: dto.id,
                    current: dto.current,
                    agent: dto
                        .user_agent
                        .as_deref()
                        .map(|agent| crate::routes::pages::truncate(agent, 80))
                        .unwrap_or_else(|| lang.t("account.sessions.unknown_client").to_string()),
                    created: time::humanize_lang(lang, &dto.created_at),
                    last_seen: time::humanize_lang(lang, &dto.last_seen_at),
                    expires: time::humanize_lang(lang, &dto.expires_at),
                }
            })
            .collect(),
        tokens: token_rows
            .iter()
            .map(|token| TokenRow {
                id: token.id,
                name: token.name.clone(),
                prefix: token.token_prefix.clone(),
                created: time::humanize_lang(lang, &token.created_at),
                last_used: token
                    .last_used_at
                    .as_deref()
                    .map(|value| time::humanize_lang(lang, value)),
                expires: token
                    .expires_at
                    .as_deref()
                    .map(|value| time::humanize_lang(lang, value)),
                active: token.is_valid_at(&time::now_iso()),
            })
            .collect(),
        tokens_usage,
        allow_api_tokens: effective.allow_api_tokens,
        new_token,
        max_name_len: tokens::MAX_NAME_LEN,
        lang,
    }
    .themed();

    match page.render() {
        Ok(html) => Ok((StatusCode::OK, Html(html)).into_response()),
        Err(err) => Err(AppError::internal(format!("template error: {err}"))),
    }
}
