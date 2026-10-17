//! Admin pages: overview, users, links, invites, settings and the audit log.

use askama::Template;
use axum::extract::rejection::FormRejection;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::{Html, IntoResponse, Response};
use axum::Form;
use serde::Deserialize;

use crate::db::audit::AuditFilter;
use crate::db::domains::DomainWithCount;
use crate::db::invites::Invite;
use crate::db::urls::{self, AdminUrlFilter, UrlSort};
use crate::db::users::{User, UserFilter};
use crate::db::{tags, users};
use crate::error::AppError;
use crate::i18n::{self, Lang};
use crate::middleware::auth::{ClientInfo, MaybeUser};
use crate::routes::dto::UrlDto;
use crate::routes::pages::dashboard::clean;
use crate::routes::pages::{
    flash_from_query, page_links, redirect_err, redirect_ok, redirect_to_login, Chrome, PageLink,
    UrlRow,
};
use crate::services::admin::{self, AdminContext};
use crate::services::captcha::CaptchaProvider;
use crate::services::domains::DomainContext;
use crate::services::settings::Settings;
use crate::services::time;
use crate::state::AppState;

const PER_PAGE: i64 = 25;

/// Admin overview page.
#[derive(Debug, Template)]
#[template(path = "admin/overview.html")]
pub struct OverviewPage {
    pub site_name: String,
    pub section: &'static str,
    pub chrome: Chrome,
    pub stats: OverviewStats,
    pub lang: Lang,
}

/// Numbers rendered on the overview page.
#[derive(Debug, Clone, Default)]
pub struct OverviewStats {
    pub version: &'static str,
    pub uptime: String,
    pub users: i64,
    pub admins: i64,
    pub links: i64,
    pub expired_links: i64,
    pub total_clicks: i64,
    pub unique_clicks: i64,
    pub bot_clicks: i64,
    pub active_invites: i64,
    pub database_size: String,
    pub registration_mode: String,
}

/// Users page.
#[derive(Debug, Template)]
#[template(path = "admin/users.html")]
pub struct UsersPage {
    pub site_name: String,
    pub section: &'static str,
    pub chrome: Chrome,
    pub users: Vec<AdminUserRow>,
    pub links: Vec<PageLink>,
    pub page: i64,
    pub total_pages: i64,
    pub total: i64,
    pub q: String,
    pub role: String,
    pub status: String,
    /// Localised "{count} accounts" line.
    pub count_label: String,
    /// Localised "Page 2 of 5" line.
    pub pagination_note: String,
    pub lang: Lang,
}

/// One row of the user table.
#[derive(Debug, Clone)]
pub struct AdminUserRow {
    pub id: i64,
    pub email: String,
    pub display_name: Option<String>,
    pub role: String,
    pub status: String,
    pub link_count: i64,
    pub created: String,
    pub last_login: String,
    pub is_self: bool,
}

/// Links page.
#[derive(Debug, Template)]
#[template(path = "admin/urls.html")]
pub struct AdminUrlsPage {
    pub site_name: String,
    pub section: &'static str,
    pub chrome: Chrome,
    pub urls: Vec<AdminUrlRow>,
    pub links: Vec<PageLink>,
    pub page: i64,
    pub total_pages: i64,
    pub total: i64,
    pub q: String,
    pub status: String,
    pub user_id: String,
    /// Localised "{count} links" line.
    pub count_label: String,
    /// Localised "Page 2 of 5" line.
    pub pagination_note: String,
    pub lang: Lang,
}

/// One row of the admin link table.
#[derive(Debug, Clone)]
pub struct AdminUrlRow {
    pub url: UrlRow,
    pub owner: String,
    pub owner_id: i64,
}

/// Domains page.
#[derive(Debug, Template)]
#[template(path = "admin/domains.html")]
pub struct DomainsPage {
    pub site_name: String,
    pub section: &'static str,
    pub chrome: Chrome,
    pub domains: Vec<DomainRow>,
    pub lang: Lang,
}

/// One row of the domain table.
#[derive(Debug, Clone)]
pub struct DomainRow {
    pub id: i64,
    pub hostname: String,
    pub status: String,
    pub status_label: String,
    pub status_class: &'static str,
    pub is_default: bool,
    pub source_label: String,
    pub source_note: Option<&'static str>,
    pub link_count: i64,
    pub created: String,
    pub can_activate: bool,
    pub can_disable: bool,
    pub can_default: bool,
    pub can_delete: bool,
}

impl DomainRow {
    fn new(domain: &DomainWithCount, lang: Lang) -> Self {
        let (status_key, status_class) = if domain.is_active() {
            ("admin.domains.status.active", "badge-ok")
        } else if domain.is_pending() {
            ("admin.domains.status.pending", "badge-warn")
        } else {
            ("admin.domains.status.disabled", "badge-muted")
        };
        let source_key = if domain.is_system() {
            "admin.domains.source.system"
        } else if domain.is_env() {
            "admin.domains.source.env"
        } else {
            "admin.domains.source.ui"
        };
        let source_note = if domain.is_system() {
            Some("admin.domains.note.system")
        } else if domain.is_env() {
            Some("admin.domains.note.env")
        } else {
            None
        };
        Self {
            id: domain.id,
            hostname: domain.hostname.clone(),
            status: domain.status.clone(),
            status_label: lang.t(status_key).to_string(),
            status_class,
            is_default: domain.is_default(),
            source_label: lang.t(source_key).to_string(),
            source_note,
            link_count: domain.link_count,
            created: time::humanize_lang(lang, &domain.created_at),
            can_activate: !domain.is_active(),
            can_disable: domain.is_active() && !domain.is_system() && !domain.is_default(),
            can_default: domain.is_active() && !domain.is_default(),
            can_delete: domain.is_ui() && !domain.is_default() && domain.link_count == 0,
        }
    }
}

/// Invites page.
#[derive(Debug, Template)]
#[template(path = "admin/invites.html")]
pub struct InvitesPage {
    pub site_name: String,
    pub section: &'static str,
    pub chrome: Chrome,
    pub invites: Vec<InviteRow>,
    pub new_code: Option<String>,
    /// Raw token (`open` / `invite` / `closed`), used for the conditional class.
    pub registration_mode: String,
    /// Localised label of the same mode, used for display.
    pub registration_mode_label: String,
    pub lang: Lang,
}

/// One row of the invite table.
#[derive(Debug, Clone)]
pub struct InviteRow {
    pub id: i64,
    pub prefix: String,
    pub note: String,
    pub uses: String,
    pub expires: String,
    pub status: String,
    pub usable: bool,
    pub created: String,
}

/// Settings page.
#[derive(Debug, Template)]
#[template(path = "admin/settings.html")]
pub struct SettingsPage {
    pub site_name: String,
    pub section: &'static str,
    pub chrome: Chrome,
    pub values: SettingsValues,
    pub lang: Lang,
}

/// Current values of every editable setting.
#[derive(Debug, Clone, Default)]
pub struct SettingsValues {
    pub site_name: String,
    pub registration_mode: String,
    pub default_slug_length: i64,
    pub max_urls_per_user: i64,
    pub default_redirect_type: i64,
    pub analytics_enabled: bool,
    pub click_retention_days: i64,
    pub captcha_provider: String,
    pub captcha_site_key: String,
    /// Whether a secret is stored. The secret itself is never rendered.
    pub captcha_secret_configured: bool,
    /// Comma separated hostnames the provider may report, `base_host` excluded.
    pub captcha_allowed_hostnames: String,
    pub captcha_on_register: bool,
    pub captcha_on_login: bool,
    pub captcha_on_create: bool,
    /// Localised note telling the administrator whether the configuration is
    /// actually in effect: a ticked box with a missing key does nothing.
    pub captcha_state: Option<String>,
}

/// Audit page.
#[derive(Debug, Template)]
#[template(path = "admin/audit.html")]
pub struct AuditPage {
    pub site_name: String,
    pub section: &'static str,
    pub chrome: Chrome,
    pub entries: Vec<AuditRow>,
    pub links: Vec<PageLink>,
    pub page: i64,
    pub total_pages: i64,
    pub total: i64,
    pub action: String,
    pub target_type: String,
    pub actions: Vec<String>,
    /// Localised "{count} entries" line.
    pub count_label: String,
    /// Localised "Page 2 of 5" line.
    pub pagination_note: String,
    pub lang: Lang,
}

/// One row of the audit table.
#[derive(Debug, Clone)]
pub struct AuditRow {
    pub when: String,
    pub action: String,
    pub actor: String,
    pub target: String,
    pub metadata: String,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct AdminQuery {
    #[serde(default)]
    pub page: Option<i64>,
    #[serde(default)]
    pub q: Option<String>,
    #[serde(default)]
    pub role: Option<String>,
    #[serde(default)]
    pub status: Option<String>,
    #[serde(default)]
    pub user_id: Option<i64>,
    #[serde(default)]
    pub action: Option<String>,
    #[serde(default)]
    pub target_type: Option<String>,
    #[serde(default)]
    pub ok: Option<String>,
    #[serde(default)]
    pub err: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct UpdateUserForm {
    #[serde(default)]
    pub role: Option<String>,
    #[serde(default)]
    pub status: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct ResetPasswordForm {
    pub new_password: String,
}

#[derive(Debug, Deserialize)]
pub struct UpdateUrlForm {
    #[serde(default)]
    pub status: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct CreateInviteForm {
    #[serde(default)]
    pub note: Option<String>,
    #[serde(default)]
    pub max_uses: Option<String>,
    #[serde(default)]
    pub expires_at: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct SettingsForm {
    pub site_name: String,
    pub registration_mode: String,
    pub default_slug_length: String,
    pub max_urls_per_user: String,
    pub default_redirect_type: String,
    #[serde(default)]
    pub analytics_enabled: Option<String>,
    pub click_retention_days: String,
    // Bot protection. All optional so a form from an older page, or a partial
    // submission, never fails to deserialize.
    #[serde(default)]
    pub captcha_provider: Option<String>,
    #[serde(default)]
    pub captcha_site_key: Option<String>,
    /// Write only: an empty field keeps the stored secret.
    #[serde(default)]
    pub captcha_secret_key: Option<String>,
    /// Echoed back, so an empty field clears the allowlist.
    #[serde(default)]
    pub captcha_allowed_hostnames: Option<String>,
    #[serde(default)]
    pub captcha_on_register: Option<String>,
    #[serde(default)]
    pub captcha_on_login: Option<String>,
    #[serde(default)]
    pub captcha_on_create: Option<String>,
}

/// `GET /app/admin`
pub async fn overview(
    State(state): State<AppState>,
    MaybeUser(current): MaybeUser,
    Query(query): Query<AdminQuery>,
) -> Result<Response, AppError> {
    let Some(current) = admin_user(current)? else {
        return Ok(redirect_to_login(Some("/app/admin")).into_response());
    };

    let context = AdminContext {
        pool: &state.db,
        config: &state.config,
        settings: &state.settings,
        actor: &current.user,
        ip_hash: None,
        started_at: state.started_at,
    };
    let stats = context.system_stats().await?;
    let lang = i18n::current();

    render(
        OverviewPage {
            section: "overview",
            site_name: state.settings.site_name(),
            chrome: Chrome::new(
                &state,
                &current.user,
                "admin",
                flash_from_query(query.ok.as_deref(), query.err.as_deref()),
            ),
            stats: OverviewStats {
                version: stats.version,
                uptime: humanize_uptime(stats.uptime_seconds),
                users: stats.users,
                admins: stats.admins,
                links: stats.links,
                expired_links: stats.expired_links,
                total_clicks: stats.total_clicks,
                unique_clicks: stats.unique_clicks,
                bot_clicks: stats.bot_clicks,
                active_invites: stats.active_invites,
                database_size: stats
                    .database_bytes
                    .map(human_bytes)
                    .unwrap_or_else(|| lang.t("admin.overview.unknown").to_string()),
                // The raw token still drives `/api/v1`; the page shows a label.
                registration_mode: i18n::registration_mode_label(lang, &stats.registration_mode),
            },
            lang,
        },
        StatusCode::OK,
    )
}

/// `GET /app/admin/users`
pub async fn users_page(
    State(state): State<AppState>,
    MaybeUser(current): MaybeUser,
    Query(query): Query<AdminQuery>,
) -> Result<Response, AppError> {
    let Some(current) = admin_user(current)? else {
        return Ok(redirect_to_login(Some("/app/admin/users")).into_response());
    };

    let page = query.page.unwrap_or(1).max(1);
    let filter = UserFilter {
        search: clean(&query.q),
        role: clean(&query.role),
        status: clean(&query.status),
    };
    let rows = admin::list_users(&state.db, &filter, PER_PAGE, (page - 1) * PER_PAGE).await?;
    let total = admin::count_users(&state.db, &filter).await?;
    let counts = admin::link_counts(&state.db).await?;
    let total_pages = ((total + PER_PAGE - 1) / PER_PAGE).max(1);
    let lang = i18n::current();

    let mut params: Vec<(&str, String)> = Vec::new();
    if let Some(value) = clean(&query.q) {
        params.push(("q", value));
    }
    if let Some(value) = clean(&query.role) {
        params.push(("role", value));
    }
    if let Some(value) = clean(&query.status) {
        params.push(("status", value));
    }

    render(
        UsersPage {
            section: "users",
            site_name: state.settings.site_name(),
            chrome: Chrome::new(
                &state,
                &current.user,
                "admin",
                flash_from_query(query.ok.as_deref(), query.err.as_deref()),
            ),
            users: rows
                .iter()
                .map(|user| AdminUserRow {
                    id: user.id,
                    email: user.email.clone(),
                    display_name: user.display_name.clone(),
                    role: user.role.clone(),
                    status: user.status.clone(),
                    link_count: counts.get(&user.id).copied().unwrap_or(0),
                    created: time::humanize_lang(lang, &user.created_at),
                    last_login: user
                        .last_login_at
                        .as_deref()
                        .map(|value| time::humanize_lang(lang, value))
                        .unwrap_or_else(|| lang.t("common.never").to_string()),
                    is_self: user.id == current.user.id,
                })
                .collect(),
            links: page_links("/app/admin/users", &params, page, total_pages),
            page,
            total_pages,
            total,
            q: clean(&query.q).unwrap_or_default(),
            role: clean(&query.role).unwrap_or_default(),
            status: clean(&query.status).unwrap_or_default(),
            count_label: i18n::format(lang, "admin.users.count", &[("count", total.to_string())]),
            pagination_note: i18n::format(
                lang,
                "common.page_of",
                &[
                    ("page", page.to_string()),
                    ("total", total_pages.to_string()),
                ],
            ),
            lang,
        },
        StatusCode::OK,
    )
}

/// `GET /app/admin/urls`
pub async fn urls_page(
    State(state): State<AppState>,
    MaybeUser(current): MaybeUser,
    Query(query): Query<AdminQuery>,
) -> Result<Response, AppError> {
    let Some(current) = admin_user(current)? else {
        return Ok(redirect_to_login(Some("/app/admin/urls")).into_response());
    };

    let page = query.page.unwrap_or(1).max(1);
    let filter = AdminUrlFilter {
        search: clean(&query.q),
        status: clean(&query.status),
        user_id: query.user_id,
        sort: UrlSort::default(),
    };
    let rows = urls::list_all(&state.db, &filter, PER_PAGE, (page - 1) * PER_PAGE)
        .await
        .map_err(AppError::from)?;
    let total = urls::count_all(&state.db, &filter)
        .await
        .map_err(AppError::from)?;
    let ids: Vec<i64> = rows.iter().map(|row| row.id).collect();
    let mut tag_map = tags::for_urls(&state.db, &ids)
        .await
        .map_err(AppError::from)?;

    let lang = i18n::current();
    let mut owners: Vec<(i64, String)> = Vec::new();
    let mut url_rows = Vec::with_capacity(rows.len());
    for row in &rows {
        let owner = match owners.iter().find(|(id, _)| *id == row.user_id) {
            Some((_, email)) => email.clone(),
            None => {
                let email = users::find_by_id(&state.db, row.user_id)
                    .await
                    .map_err(AppError::from)?
                    .map(|user| user.email)
                    .unwrap_or_else(|| lang.t("admin.urls.deleted_user").to_string());
                owners.push((row.user_id, email.clone()));
                email
            }
        };
        let names = tag_map
            .remove(&row.id)
            .unwrap_or_default()
            .into_iter()
            .map(|tag| tag.name)
            .collect();
        url_rows.push(AdminUrlRow {
            url: UrlRow::new(&UrlDto::new(
                row,
                &state.domain_hostname(row.domain_id),
                &state.config.base_scheme,
                names,
            )),
            owner,
            owner_id: row.user_id,
        });
    }

    let total_pages = ((total + PER_PAGE - 1) / PER_PAGE).max(1);
    let mut params: Vec<(&str, String)> = Vec::new();
    if let Some(value) = clean(&query.q) {
        params.push(("q", value));
    }
    if let Some(value) = clean(&query.status) {
        params.push(("status", value));
    }
    if let Some(value) = query.user_id {
        params.push(("user_id", value.to_string()));
    }

    render(
        AdminUrlsPage {
            section: "urls",
            site_name: state.settings.site_name(),
            chrome: Chrome::new(
                &state,
                &current.user,
                "admin",
                flash_from_query(query.ok.as_deref(), query.err.as_deref()),
            ),
            urls: url_rows,
            links: page_links("/app/admin/urls", &params, page, total_pages),
            page,
            total_pages,
            total,
            q: clean(&query.q).unwrap_or_default(),
            status: clean(&query.status).unwrap_or_default(),
            user_id: query.user_id.map(|id| id.to_string()).unwrap_or_default(),
            count_label: i18n::format(lang, "admin.urls.count", &[("count", total.to_string())]),
            pagination_note: i18n::format(
                lang,
                "common.page_of",
                &[
                    ("page", page.to_string()),
                    ("total", total_pages.to_string()),
                ],
            ),
            lang,
        },
        StatusCode::OK,
    )
}

/// `GET /app/admin/invites`
pub async fn invites_page(
    State(state): State<AppState>,
    MaybeUser(current): MaybeUser,
    Query(query): Query<AdminQuery>,
) -> Result<Response, AppError> {
    let Some(current) = admin_user(current)? else {
        return Ok(redirect_to_login(Some("/app/admin/invites")).into_response());
    };
    render_invites(
        &state,
        &current.user,
        flash_from_query(query.ok.as_deref(), query.err.as_deref()),
        None,
    )
    .await
}

/// `POST /app/admin/invites`
pub async fn create_invite(
    State(state): State<AppState>,
    client: ClientInfo,
    MaybeUser(current): MaybeUser,
    form: std::result::Result<Form<CreateInviteForm>, FormRejection>,
) -> Result<Response, AppError> {
    let Some(current) = admin_user(current)? else {
        return Ok(redirect_to_login(Some("/app/admin/invites")).into_response());
    };
    let Ok(Form(payload)) = form else {
        return Ok(redirect_err("/app/admin/invites", "invalid").into_response());
    };

    let max_uses = payload
        .max_uses
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .and_then(|value| value.parse::<i64>().ok());

    let context = context(&state, &client, &current.user);
    match context
        .create_invite(
            payload.note.as_deref(),
            max_uses,
            payload.expires_at.as_deref(),
        )
        .await
    {
        Ok(created) => {
            render_invites(
                &state,
                &current.user,
                Some(crate::routes::pages::Flash {
                    kind: "ok",
                    message: i18n::current().t("admin.invites.created_flash").to_string(),
                }),
                Some(created.plain_code),
            )
            .await
        }
        Err(err) => {
            Ok(redirect_err("/app/admin/invites", super::urls::error_code(&err)).into_response())
        }
    }
}

/// `POST /app/admin/invites/{id}/delete`
pub async fn revoke_invite(
    State(state): State<AppState>,
    client: ClientInfo,
    MaybeUser(current): MaybeUser,
    Path(id): Path<i64>,
) -> Result<Response, AppError> {
    let Some(current) = admin_user(current)? else {
        return Ok(redirect_to_login(Some("/app/admin/invites")).into_response());
    };
    Ok(
        match context(&state, &client, &current.user)
            .revoke_invite(id)
            .await
        {
            Ok(()) => redirect_ok("/app/admin/invites", "invite_revoked").into_response(),
            Err(err) => {
                redirect_err("/app/admin/invites", super::urls::error_code(&err)).into_response()
            }
        },
    )
}

/// Form for adding a domain.
#[derive(Debug, Deserialize)]
pub struct DomainForm {
    pub hostname: String,
}

/// `GET /app/admin/domains`
pub async fn domains_page(
    State(state): State<AppState>,
    MaybeUser(current): MaybeUser,
    Query(query): Query<AdminQuery>,
) -> Result<Response, AppError> {
    let Some(current) = admin_user(current)? else {
        return Ok(redirect_to_login(Some("/app/admin/domains")).into_response());
    };
    render_domains(
        &state,
        &current.user,
        flash_from_query(query.ok.as_deref(), query.err.as_deref()),
    )
    .await
}

/// `POST /app/admin/domains`
pub async fn create_domain(
    State(state): State<AppState>,
    client: ClientInfo,
    MaybeUser(current): MaybeUser,
    form: std::result::Result<Form<DomainForm>, FormRejection>,
) -> Result<Response, AppError> {
    let Some(current) = admin_user(current)? else {
        return Ok(redirect_to_login(Some("/app/admin/domains")).into_response());
    };
    let Ok(Form(payload)) = form else {
        return Ok(redirect_err("/app/admin/domains", "domain_invalid").into_response());
    };

    let outcome = domain_context(&state, &client, &current.user)
        .create(&payload.hostname)
        .await;
    Ok(match outcome {
        Ok(_) => {
            reload_domain_cache(&state).await?;
            redirect_ok("/app/admin/domains", "domain_created").into_response()
        }
        Err(err) => redirect_err("/app/admin/domains", domain_error_code(&err)).into_response(),
    })
}

/// `POST /app/admin/domains/{id}/activate`
pub async fn activate_domain(
    State(state): State<AppState>,
    client: ClientInfo,
    MaybeUser(current): MaybeUser,
    Path(id): Path<i64>,
) -> Result<Response, AppError> {
    let Some(current) = admin_user(current)? else {
        return Ok(redirect_to_login(Some("/app/admin/domains")).into_response());
    };
    Ok(
        match domain_context(&state, &client, &current.user)
            .activate(id)
            .await
        {
            Ok(_) => {
                reload_domain_cache(&state).await?;
                redirect_ok("/app/admin/domains", "domain_activated").into_response()
            }
            Err(err) => redirect_err("/app/admin/domains", domain_error_code(&err)).into_response(),
        },
    )
}

/// `POST /app/admin/domains/{id}/disable`
pub async fn disable_domain(
    State(state): State<AppState>,
    client: ClientInfo,
    MaybeUser(current): MaybeUser,
    Path(id): Path<i64>,
) -> Result<Response, AppError> {
    let Some(current) = admin_user(current)? else {
        return Ok(redirect_to_login(Some("/app/admin/domains")).into_response());
    };
    Ok(
        match domain_context(&state, &client, &current.user)
            .disable(id)
            .await
        {
            Ok(_) => {
                reload_domain_cache(&state).await?;
                redirect_ok("/app/admin/domains", "domain_disabled").into_response()
            }
            Err(err) => redirect_err("/app/admin/domains", domain_error_code(&err)).into_response(),
        },
    )
}

/// `POST /app/admin/domains/{id}/default`
pub async fn default_domain(
    State(state): State<AppState>,
    client: ClientInfo,
    MaybeUser(current): MaybeUser,
    Path(id): Path<i64>,
) -> Result<Response, AppError> {
    let Some(current) = admin_user(current)? else {
        return Ok(redirect_to_login(Some("/app/admin/domains")).into_response());
    };
    Ok(
        match domain_context(&state, &client, &current.user)
            .set_default(id)
            .await
        {
            Ok(_) => {
                reload_domain_cache(&state).await?;
                redirect_ok("/app/admin/domains", "domain_default").into_response()
            }
            Err(err) => redirect_err("/app/admin/domains", domain_error_code(&err)).into_response(),
        },
    )
}

/// `POST /app/admin/domains/{id}/delete`
pub async fn delete_domain(
    State(state): State<AppState>,
    client: ClientInfo,
    MaybeUser(current): MaybeUser,
    Path(id): Path<i64>,
) -> Result<Response, AppError> {
    let Some(current) = admin_user(current)? else {
        return Ok(redirect_to_login(Some("/app/admin/domains")).into_response());
    };
    Ok(
        match domain_context(&state, &client, &current.user)
            .delete(id)
            .await
        {
            Ok(()) => {
                reload_domain_cache(&state).await?;
                redirect_ok("/app/admin/domains", "domain_deleted").into_response()
            }
            Err(err) => redirect_err("/app/admin/domains", domain_error_code(&err)).into_response(),
        },
    )
}

async fn render_domains(
    state: &AppState,
    user: &User,
    flash: Option<crate::routes::pages::Flash>,
) -> Result<Response, AppError> {
    let lang = i18n::current();
    let domains = crate::db::domains::list_with_counts(&state.db)
        .await
        .map_err(AppError::from)?
        .iter()
        .map(|domain| DomainRow::new(domain, lang))
        .collect();
    render(
        DomainsPage {
            section: "domains",
            site_name: state.settings.site_name(),
            chrome: Chrome::new(state, user, "admin", flash),
            domains,
            lang,
        },
        StatusCode::OK,
    )
}

fn domain_context<'a>(
    state: &'a AppState,
    client: &'a ClientInfo,
    user: &'a User,
) -> DomainContext<'a> {
    DomainContext {
        pool: &state.db,
        actor: user,
        ip_hash: client.ip_hash.as_deref(),
    }
}

async fn reload_domain_cache(state: &AppState) -> Result<(), AppError> {
    state
        .domains
        .reload(&state.db)
        .await
        .map_err(|err| AppError::internal(format!("cannot reload domains: {err}")))
}

fn domain_error_code(err: &AppError) -> &'static str {
    match err {
        AppError::Validation {
            field: "domain", ..
        } => "domain_invalid",
        AppError::NotFound => "not_found",
        AppError::Conflict(message) if message.contains("canonical") => "domain_system",
        AppError::Conflict(message) if message.contains("URLET_DOMAINS") => "domain_env",
        AppError::Conflict(message) if message.contains("default") => "domain_default",
        AppError::Conflict(message) if message.contains("link") => "domain_in_use",
        AppError::Conflict(_) => "domain_conflict",
        AppError::RateLimited { .. } => "rate_limited",
        _ => "invalid",
    }
}

/// `GET /app/admin/settings`
pub async fn settings_page(
    State(state): State<AppState>,
    MaybeUser(current): MaybeUser,
    Query(query): Query<AdminQuery>,
) -> Result<Response, AppError> {
    let Some(current) = admin_user(current)? else {
        return Ok(redirect_to_login(Some("/app/admin/settings")).into_response());
    };
    render_settings(
        &state,
        &current.user,
        flash_from_query(query.ok.as_deref(), query.err.as_deref()),
    )
}

/// `POST /app/admin/settings`
pub async fn update_settings(
    State(state): State<AppState>,
    client: ClientInfo,
    MaybeUser(current): MaybeUser,
    form: std::result::Result<Form<SettingsForm>, FormRejection>,
) -> Result<Response, AppError> {
    let Some(current) = admin_user(current)? else {
        return Ok(redirect_to_login(Some("/app/admin/settings")).into_response());
    };
    let Ok(Form(payload)) = form else {
        return Ok(redirect_err("/app/admin/settings", "invalid").into_response());
    };

    let mut values = vec![
        ("site_name".to_string(), payload.site_name),
        ("registration_mode".to_string(), payload.registration_mode),
        (
            "default_slug_length".to_string(),
            payload.default_slug_length,
        ),
        ("max_urls_per_user".to_string(), payload.max_urls_per_user),
        (
            "default_redirect_type".to_string(),
            payload.default_redirect_type,
        ),
        (
            "analytics_enabled".to_string(),
            if payload.analytics_enabled.is_some() {
                "true"
            } else {
                "false"
            }
            .to_string(),
        ),
        (
            "click_retention_days".to_string(),
            payload.click_retention_days,
        ),
    ];

    // Bot protection. The provider and the public site key are echoed back by
    // the form, so an empty value there is an explicit clear. The secret is
    // never echoed, so an empty value means "keep the stored one".
    if let Some(provider) = payload
        .captcha_provider
        .filter(|value| !value.trim().is_empty())
    {
        values.push(("captcha_provider".to_string(), provider));
    }
    if let Some(site_key) = payload.captcha_site_key {
        values.push(("captcha_site_key".to_string(), site_key));
    }
    if let Some(secret) = payload
        .captcha_secret_key
        .filter(|value| !value.trim().is_empty())
    {
        values.push(("captcha_secret_key".to_string(), secret));
    }
    if let Some(hostnames) = payload.captcha_allowed_hostnames {
        values.push(("captcha_allowed_hostnames".to_string(), hostnames));
    }
    // An unchecked box is simply absent from the body, so absence means "off".
    for (key, checked) in [
        ("captcha_on_register", payload.captcha_on_register.is_some()),
        ("captcha_on_login", payload.captcha_on_login.is_some()),
        ("captcha_on_create", payload.captcha_on_create.is_some()),
    ] {
        values.push((
            key.to_string(),
            if checked { "true" } else { "false" }.to_string(),
        ));
    }

    Ok(
        match context(&state, &client, &current.user)
            .update_settings(&values)
            .await
        {
            Ok(()) => redirect_ok("/app/admin/settings", "saved").into_response(),
            Err(err) => {
                redirect_err("/app/admin/settings", super::urls::error_code(&err)).into_response()
            }
        },
    )
}

/// `GET /app/admin/audit`
pub async fn audit_page(
    State(state): State<AppState>,
    MaybeUser(current): MaybeUser,
    Query(query): Query<AdminQuery>,
) -> Result<Response, AppError> {
    let Some(current) = admin_user(current)? else {
        return Ok(redirect_to_login(Some("/app/admin/audit")).into_response());
    };

    let page = query.page.unwrap_or(1).max(1);
    let filter = AuditFilter {
        action: clean(&query.action),
        target_type: clean(&query.target_type),
        actor_user_id: None,
    };
    let (rows, total) =
        admin::audit_page(&state.db, &filter, PER_PAGE, (page - 1) * PER_PAGE).await?;
    let total_pages = ((total + PER_PAGE - 1) / PER_PAGE).max(1);

    let mut params: Vec<(&str, String)> = Vec::new();
    if let Some(value) = clean(&query.action) {
        params.push(("action", value));
    }
    if let Some(value) = clean(&query.target_type) {
        params.push(("target_type", value));
    }

    let mut actions: Vec<String> = rows.iter().map(|row| row.action.clone()).collect();
    actions.sort();
    actions.dedup();
    let lang = i18n::current();

    render(
        AuditPage {
            section: "audit",
            site_name: state.settings.site_name(),
            chrome: Chrome::new(
                &state,
                &current.user,
                "admin",
                flash_from_query(query.ok.as_deref(), query.err.as_deref()),
            ),
            entries: rows
                .iter()
                .map(|row| AuditRow {
                    when: time::humanize_lang(lang, &row.created_at),
                    // The action and the metadata stay raw: they are the values
                    // the filter above searches by.
                    action: row.action.clone(),
                    actor: match (row.actor_user_id, row.actor_role.as_deref()) {
                        (Some(id), Some(role)) => {
                            format!("#{id} ({})", i18n::role_label(lang, role))
                        }
                        (Some(id), None) => format!("#{id}"),
                        _ => lang.t("admin.audit.system").to_string(),
                    },
                    target: match (&row.target_type, &row.target_id) {
                        (Some(kind), Some(id)) => format!("{kind}:{id}"),
                        (Some(kind), None) => kind.clone(),
                        _ => "-".to_string(),
                    },
                    metadata: row.metadata.clone().unwrap_or_default(),
                })
                .collect(),
            links: page_links("/app/admin/audit", &params, page, total_pages),
            page,
            total_pages,
            total,
            action: clean(&query.action).unwrap_or_default(),
            target_type: clean(&query.target_type).unwrap_or_default(),
            actions,
            count_label: i18n::format(lang, "admin.audit.count", &[("count", total.to_string())]),
            pagination_note: i18n::format(
                lang,
                "common.page_of",
                &[
                    ("page", page.to_string()),
                    ("total", total_pages.to_string()),
                ],
            ),
            lang,
        },
        StatusCode::OK,
    )
}

/// `POST /app/admin/users/{id}`
pub async fn update_user(
    State(state): State<AppState>,
    client: ClientInfo,
    MaybeUser(current): MaybeUser,
    Path(id): Path<i64>,
    form: std::result::Result<Form<UpdateUserForm>, FormRejection>,
) -> Result<Response, AppError> {
    let Some(current) = admin_user(current)? else {
        return Ok(redirect_to_login(Some("/app/admin/users")).into_response());
    };
    let Ok(Form(payload)) = form else {
        return Ok(redirect_err("/app/admin/users", "invalid").into_response());
    };

    Ok(
        match context(&state, &client, &current.user)
            .update_user(id, payload.role.as_deref(), payload.status.as_deref(), None)
            .await
        {
            Ok(_) => redirect_ok("/app/admin/users", "user").into_response(),
            Err(err) => {
                redirect_err("/app/admin/users", super::urls::error_code(&err)).into_response()
            }
        },
    )
}

/// `POST /app/admin/users/{id}/password`
pub async fn reset_password(
    State(state): State<AppState>,
    client: ClientInfo,
    MaybeUser(current): MaybeUser,
    Path(id): Path<i64>,
    form: std::result::Result<Form<ResetPasswordForm>, FormRejection>,
) -> Result<Response, AppError> {
    let Some(current) = admin_user(current)? else {
        return Ok(redirect_to_login(Some("/app/admin/users")).into_response());
    };
    let Ok(Form(payload)) = form else {
        return Ok(redirect_err("/app/admin/users", "invalid").into_response());
    };

    Ok(
        match context(&state, &client, &current.user)
            .reset_password(id, &payload.new_password)
            .await
        {
            Ok(()) => redirect_ok("/app/admin/users", "user").into_response(),
            Err(err) => {
                redirect_err("/app/admin/users", super::urls::error_code(&err)).into_response()
            }
        },
    )
}

/// `POST /app/admin/users/{id}/delete`
pub async fn delete_user(
    State(state): State<AppState>,
    client: ClientInfo,
    MaybeUser(current): MaybeUser,
    Path(id): Path<i64>,
) -> Result<Response, AppError> {
    let Some(current) = admin_user(current)? else {
        return Ok(redirect_to_login(Some("/app/admin/users")).into_response());
    };
    Ok(
        match context(&state, &client, &current.user)
            .delete_user(id)
            .await
        {
            Ok(()) => redirect_ok("/app/admin/users", "user_deleted").into_response(),
            Err(err) => {
                redirect_err("/app/admin/users", super::urls::error_code(&err)).into_response()
            }
        },
    )
}

/// `POST /app/admin/urls/{id}`
pub async fn update_url(
    State(state): State<AppState>,
    client: ClientInfo,
    MaybeUser(current): MaybeUser,
    Path(id): Path<i64>,
    form: std::result::Result<Form<UpdateUrlForm>, FormRejection>,
) -> Result<Response, AppError> {
    let Some(current) = admin_user(current)? else {
        return Ok(redirect_to_login(Some("/app/admin/urls")).into_response());
    };
    let Ok(Form(payload)) = form else {
        return Ok(redirect_err("/app/admin/urls", "invalid").into_response());
    };

    Ok(
        match context(&state, &client, &current.user)
            .update_url(id, payload.status.as_deref(), None)
            .await
        {
            Ok(()) => redirect_ok("/app/admin/urls", "updated").into_response(),
            Err(err) => {
                redirect_err("/app/admin/urls", super::urls::error_code(&err)).into_response()
            }
        },
    )
}

/// `POST /app/admin/urls/{id}/delete`
pub async fn delete_url(
    State(state): State<AppState>,
    client: ClientInfo,
    MaybeUser(current): MaybeUser,
    Path(id): Path<i64>,
) -> Result<Response, AppError> {
    let Some(current) = admin_user(current)? else {
        return Ok(redirect_to_login(Some("/app/admin/urls")).into_response());
    };
    Ok(
        match context(&state, &client, &current.user).delete_url(id).await {
            Ok(()) => redirect_ok("/app/admin/urls", "deleted").into_response(),
            Err(err) => {
                redirect_err("/app/admin/urls", super::urls::error_code(&err)).into_response()
            }
        },
    )
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

/// Require an authenticated administrator, returning `Ok(None)` when the visitor
/// should be sent to the login screen instead.
fn admin_user(
    current: Option<crate::middleware::auth::CurrentUser>,
) -> Result<Option<crate::middleware::auth::CurrentUser>, AppError> {
    match current {
        None => Ok(None),
        Some(current) if current.is_admin() => Ok(Some(current)),
        Some(_) => Err(AppError::forbidden("administrator privileges are required")),
    }
}

async fn render_invites(
    state: &AppState,
    user: &User,
    flash: Option<crate::routes::pages::Flash>,
    new_code: Option<String>,
) -> Result<Response, AppError> {
    let now = time::now_iso();
    let lang = i18n::current();
    let rows = crate::db::invites::list(&state.db, 200, 0)
        .await
        .map_err(AppError::from)?;
    render(
        InvitesPage {
            section: "invites",
            site_name: state.settings.site_name(),
            chrome: Chrome::new(state, user, "admin", flash),
            invites: rows
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
                        expires: invite
                            .expires_at
                            .as_deref()
                            .map(|value| time::humanize_lang(lang, value))
                            .unwrap_or_else(|| lang.t("common.never").to_string()),
                        status: if invite.status == "active" {
                            lang.t("admin.invites.status.active").to_string()
                        } else {
                            lang.t("admin.invites.status.revoked").to_string()
                        },
                        usable: invite.status == "active" && !invite.is_exhausted() && !expired,
                        created: time::humanize_lang(lang, &invite.created_at),
                    }
                })
                .collect(),
            new_code,
            registration_mode: state.settings.registration_mode().as_str().to_string(),
            registration_mode_label: i18n::registration_mode_label(
                lang,
                state.settings.registration_mode().as_str(),
            ),
            lang,
        },
        StatusCode::OK,
    )
}

fn render_settings(
    state: &AppState,
    user: &User,
    flash: Option<crate::routes::pages::Flash>,
) -> Result<Response, AppError> {
    let lang = i18n::current();
    let mut values = settings_values(&state.settings);
    values.captcha_state = captcha_state(&state.settings, lang);
    render(
        SettingsPage {
            section: "settings",
            site_name: state.settings.site_name(),
            chrome: Chrome::new(state, user, "admin", flash),
            values,
            lang,
        },
        StatusCode::OK,
    )
}

/// Localised note describing whether bot protection is really applied.
///
/// The enable conditions are the same four `CaptchaGuard` uses, so a ticked box
/// with a missing key is reported as "not applied" instead of silently doing
/// nothing.
fn captcha_state(settings: &Settings, lang: Lang) -> Option<String> {
    if settings.captcha_provider() == CaptchaProvider::None {
        return None;
    }
    let has_site_key = !settings.captcha_site_key().trim().is_empty();
    let has_secret = settings.captcha_secret_configured();
    let any_flow = settings.captcha_on_register()
        || settings.captcha_on_login()
        || settings.captcha_on_create();

    let key = match (has_site_key, has_secret) {
        (false, false) => "admin.settings.captcha_state.missing_keys",
        (false, true) => "admin.settings.captcha_state.missing_site_key",
        (true, false) => "admin.settings.captcha_state.missing_secret_key",
        (true, true) if !any_flow => "admin.settings.captcha_state.no_flow",
        (true, true) => "admin.settings.captcha_state.active",
    };
    Some(lang.t(key).to_string())
}

/// Read the typed settings used by the settings form.
pub fn settings_values(settings: &Settings) -> SettingsValues {
    SettingsValues {
        site_name: settings.site_name(),
        registration_mode: settings.registration_mode().as_str().to_string(),
        default_slug_length: settings.default_slug_length() as i64,
        max_urls_per_user: settings.max_urls_per_user(),
        default_redirect_type: settings.default_redirect_type(),
        analytics_enabled: settings.analytics_enabled(),
        click_retention_days: settings.click_retention_days(),
        captcha_provider: settings.captcha_provider().as_str().to_string(),
        captcha_site_key: settings.captcha_site_key(),
        captcha_secret_configured: settings.captcha_secret_configured(),
        captcha_allowed_hostnames: settings.captcha_allowed_hostnames(),
        captcha_on_register: settings.captcha_on_register(),
        captcha_on_login: settings.captcha_on_login(),
        captcha_on_create: settings.captcha_on_create(),
        // Filled in by `render_settings`, which knows the language.
        captcha_state: None,
    }
}

fn render<T: Template>(template: T, status: StatusCode) -> Result<Response, AppError> {
    match template.render() {
        Ok(html) => Ok((status, Html(html)).into_response()),
        Err(err) => Err(AppError::internal(format!("template error: {err}"))),
    }
}

fn humanize_uptime(seconds: i64) -> String {
    let seconds = seconds.max(0);
    if seconds < 60 {
        format!("{seconds}s")
    } else if seconds < 3_600 {
        format!("{}m", seconds / 60)
    } else if seconds < 86_400 {
        format!("{}h {}m", seconds / 3_600, (seconds % 3_600) / 60)
    } else {
        format!("{}d {}h", seconds / 86_400, (seconds % 86_400) / 3_600)
    }
}

fn human_bytes(bytes: u64) -> String {
    const UNITS: [&str; 4] = ["B", "KiB", "MiB", "GiB"];
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1024.0 && unit < UNITS.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{bytes} B")
    } else {
        format!("{value:.1} {}", UNITS[unit])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_uptime() {
        assert_eq!(humanize_uptime(5), "5s");
        assert_eq!(humanize_uptime(90), "1m");
        assert_eq!(humanize_uptime(3_700), "1h 1m");
        assert_eq!(humanize_uptime(90_000), "1d 1h");
    }

    #[test]
    fn formats_bytes() {
        assert_eq!(human_bytes(512), "512 B");
        assert_eq!(human_bytes(2_048), "2.0 KiB");
        assert_eq!(human_bytes(5 * 1024 * 1024), "5.0 MiB");
    }
}
