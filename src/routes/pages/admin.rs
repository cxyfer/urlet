//! Admin pages: overview, users, groups, links, invites, domains, themes,
//! settings and the audit log.

use std::collections::HashMap;

use askama::Template;
use axum::extract::multipart::{Multipart, MultipartRejection};
use axum::extract::rejection::FormRejection;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::{Html, IntoResponse, Response};
use axum::Form;
use serde::Deserialize;

use crate::db::audit::AuditFilter;
use crate::db::custom_themes as custom_db;
use crate::db::domains::DomainWithCount;
use crate::db::invites::Invite;
use crate::db::urls::{self, AdminUrlFilter, UrlSort};
use crate::db::users::{User, UserFilter};
use crate::db::{tags, user_groups, users};
use crate::error::AppError;
use crate::i18n::{self, Lang};
use crate::middleware::auth::{ClientInfo, MaybeUser};
use crate::routes::dto::UrlDto;
use crate::routes::no_store;
use crate::routes::pages::dashboard::clean;
use crate::routes::pages::themed::ThemedContext;
use crate::routes::pages::{
    flash_from_query, page_links, redirect_err, redirect_ok, redirect_to_login, Chrome, PageLink,
    UrlRow,
};
use crate::services::admin::{self, AdminContext};
use crate::services::captcha::CaptchaProvider;
use crate::services::custom_theme;
use crate::services::domains::{DomainContext, DomainOption, DomainRegistry};
use crate::services::groups::{self as group_service, GroupDraft, GroupOverview};
use crate::services::invite_quota::{usage_for, usage_map, InvitePeriod, InviteUsage};
use crate::services::invites::{registration_link, reveal};
use crate::services::redirect_mode::RedirectModePolicy;
use crate::services::settings::{Settings, ThemeSwitching};
use crate::services::theme::{self as theme_service, AdminThemeRow};
use crate::services::time;
use crate::state::AppState;
use crate::themed_page;

const PER_PAGE: i64 = 25;

/// Admin overview page.
#[derive(Debug)]
pub struct OverviewPage {
    pub site_name: String,
    pub section: &'static str,
    pub chrome: Chrome,
    pub stats: OverviewStats,
    pub lang: Lang,
}

themed_page!(overview_templates, OverviewPageView, OverviewPage,
    Verdant => "admin/overview.html",
    Aurora => "themes/aurora/admin/overview.html",
    Paper => "themes/paper/admin/overview.html",
    Signal => "themes/signal/admin/overview.html",
    Gilded => "themes/gilded/admin/overview.html",
    Swiss => "themes/swiss/admin/overview.html",
);

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
#[derive(Debug)]
pub struct UsersPage {
    pub site_name: String,
    pub section: &'static str,
    pub chrome: Chrome,
    pub users: Vec<AdminUserRow>,
    pub groups: Vec<GroupOption>,
    pub links: Vec<PageLink>,
    pub page: i64,
    pub total_pages: i64,
    pub total: i64,
    pub q: String,
    pub role: String,
    pub status: String,
    pub group_id: String,
    /// Localised "{count} accounts" line.
    pub count_label: String,
    /// Localised "Page 2 of 5" line.
    pub pagination_note: String,
    pub lang: Lang,
}

themed_page!(users_templates, UsersPageView, UsersPage,
    Verdant => "admin/users.html",
    Aurora => "themes/aurora/admin/users.html",
    Paper => "themes/paper/admin/users.html",
    Signal => "themes/signal/admin/users.html",
    Gilded => "themes/gilded/admin/users.html",
    Swiss => "themes/swiss/admin/users.html",
);

/// One group as a filter option or assignment target.
#[derive(Debug, Clone)]
pub struct GroupOption {
    pub id: i64,
    pub name: String,
}

/// A group option on the invite form, annotated with its current quota.
#[derive(Debug, Clone)]
pub struct InviteGroupOption {
    pub id: i64,
    pub name: String,
    /// `2 of 5 per month` style summary; empty when the group is unlimited.
    pub quota_label: String,
}

/// One row of the user table.
#[derive(Debug, Clone)]
pub struct AdminUserRow {
    pub id: i64,
    pub email: String,
    pub display_name: Option<String>,
    pub role: String,
    pub status: String,
    /// Empty string when the user has no group.
    pub group_id: String,
    pub group_name: String,
    pub link_count: i64,
    pub created: String,
    pub last_login: String,
    pub is_self: bool,
}

/// Links page.
#[derive(Debug)]
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

themed_page!(admin_urls_templates, AdminUrlsPageView, AdminUrlsPage,
    Verdant => "admin/urls.html",
    Aurora => "themes/aurora/admin/urls.html",
    Paper => "themes/paper/admin/urls.html",
    Signal => "themes/signal/admin/urls.html",
    Gilded => "themes/gilded/admin/urls.html",
    Swiss => "themes/swiss/admin/urls.html",
);

/// One row of the admin link table.
#[derive(Debug, Clone)]
pub struct AdminUrlRow {
    pub url: UrlRow,
    pub owner: String,
    pub owner_id: i64,
}

/// Domains page.
#[derive(Debug)]
pub struct DomainsPage {
    pub site_name: String,
    pub section: &'static str,
    pub chrome: Chrome,
    pub domains: Vec<DomainRow>,
    pub lang: Lang,
}

themed_page!(domains_templates, DomainsPageView, DomainsPage,
    Verdant => "admin/domains.html",
    Aurora => "themes/aurora/admin/domains.html",
    Paper => "themes/paper/admin/domains.html",
    Signal => "themes/signal/admin/domains.html",
    Gilded => "themes/gilded/admin/domains.html",
    Swiss => "themes/swiss/admin/domains.html",
);

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
#[derive(Debug)]
pub struct InvitesPage {
    pub site_name: String,
    pub section: &'static str,
    pub chrome: Chrome,
    pub invites: Vec<InviteRow>,
    pub groups: Vec<InviteGroupOption>,
    /// Quota of invites that are not pinned to a group, when it is limited.
    pub site_quota_label: String,
    pub new_code: Option<String>,
    /// Registration link for `new_code`.
    pub new_link: Option<String>,
    /// At least one row cannot show its code again (legacy row or rotated
    /// secret); the page explains why.
    pub has_legacy: bool,
    /// Raw token (`open` / `invite` / `closed`), used for the conditional class.
    pub registration_mode: String,
    /// Localised label of the same mode, used for display.
    pub registration_mode_label: String,
    pub lang: Lang,
}

themed_page!(admin_invites_templates, InvitesPageView, InvitesPage,
    Verdant => "admin/invites.html",
    Aurora => "themes/aurora/admin/invites.html",
    Paper => "themes/paper/admin/invites.html",
    Signal => "themes/signal/admin/invites.html",
    Gilded => "themes/gilded/admin/invites.html",
    Swiss => "themes/swiss/admin/invites.html",
);

/// One row of the invite table.
#[derive(Debug, Clone)]
pub struct InviteRow {
    pub id: i64,
    pub prefix: String,
    /// Recoverable plaintext, or `None` for a legacy/sealed-unreadable row.
    pub code: Option<String>,
    /// Registration link when `code` is available.
    pub link: Option<String>,
    pub note: String,
    pub group_name: String,
    pub uses: String,
    pub expires: String,
    pub status: String,
    pub usable: bool,
    pub created: String,
}

/// User groups page.
#[derive(Debug)]
pub struct GroupsPage {
    pub site_name: String,
    pub section: &'static str,
    pub chrome: Chrome,
    pub groups: Vec<GroupRow>,
    /// Every active domain, for the default-domain select and the allowlist.
    pub domains: Vec<crate::services::domains::DomainOption>,
    pub lang: Lang,
}

themed_page!(groups_templates, GroupsPageView, GroupsPage,
    Verdant => "admin/groups.html",
    Aurora => "themes/aurora/admin/groups.html",
    Paper => "themes/paper/admin/groups.html",
    Signal => "themes/signal/admin/groups.html",
    Gilded => "themes/gilded/admin/groups.html",
    Swiss => "themes/swiss/admin/groups.html",
);

/// One row of the group table, carrying both display and form values.
#[derive(Debug, Clone)]
pub struct GroupRow {
    pub id: i64,
    pub name: String,
    pub description: String,
    pub max_urls_input: String,
    pub max_urls_display: String,
    pub max_api_tokens_input: String,
    pub max_api_tokens_display: String,
    pub max_invites_input: String,
    pub max_invites_display: String,
    /// Effective period of the invite counter (`day` / `week` / `month`).
    pub invite_period: String,
    /// Localised period label, used in the table and the form select.
    pub invite_period_label: String,
    /// `used / limit per period`, or the localised "Unlimited" when `0`.
    pub invite_usage_display: String,
    /// True when the current period is used up; the table marks it.
    pub invite_quota_exhausted: bool,
    pub members: i64,
    pub is_default: bool,
    pub allow_invites: bool,
    pub allow_custom_slug: bool,
    pub allow_password: bool,
    pub allow_expiry: bool,
    pub allow_max_clicks: bool,
    pub allow_tags: bool,
    pub allow_api_tokens: bool,
    pub allow_domain_selection: bool,
    /// Stored policy value (`direct` / `splash` / `choice`), for the radio group.
    pub redirect_mode_policy: String,
    /// Localised label of the policy, for the table.
    pub redirect_mode_policy_label: String,
    pub default_domain_id: Option<i64>,
    pub restrict_domains: bool,
    pub allowed_domain_ids: Vec<i64>,
    /// Stored allowlist domains that are currently disabled; rendered as
    /// checked options so they can be kept or removed explicitly.
    pub disabled_domains: Vec<DomainOption>,
    /// Stored group default that is currently disabled.
    pub default_domain_disabled_id: Option<i64>,
    pub default_domain_disabled_name: Option<String>,
    pub features_label: String,
    pub created: String,
}

impl GroupRow {
    fn new(
        settings: &Settings,
        lang: Lang,
        overview: &GroupOverview,
        active_domain_ids: &[i64],
        domains: &DomainRegistry,
        invite_usage: InviteUsage,
    ) -> Self {
        let group = &overview.group;
        let default_domain_disabled_id = group
            .default_domain_id
            .filter(|id| !active_domain_ids.contains(id));
        let default_domain_disabled_name =
            default_domain_disabled_id.and_then(|id| domains.hostname(id));
        let disabled_domains = overview
            .allowed_domain_ids
            .iter()
            .filter(|id| !active_domain_ids.contains(id))
            .filter_map(|id| {
                domains.hostname(*id).map(|hostname| DomainOption {
                    id: *id,
                    hostname,
                    is_default: false,
                })
            })
            .collect();
        let allowed_features = [
            group.allow_invites,
            group.allow_custom_slug,
            group.allow_password,
            group.allow_expiry,
            group.allow_max_clicks,
            group.allow_tags,
            group.allow_api_tokens,
            group.allow_domain_selection,
        ]
        .into_iter()
        .filter(|allowed| *allowed)
        .count() as i64;
        let invite_period = InvitePeriod::parse(&group.invite_period).unwrap_or_default();
        let invite_period_label = super::period_label(lang, invite_period);
        let redirect_mode_policy = RedirectModePolicy::from_db(&group.redirect_mode_policy);
        let invite_usage_display = if invite_usage.unlimited() {
            lang.t("admin.groups.unlimited").to_string()
        } else {
            i18n::format(
                lang,
                "admin.groups.invite_usage",
                &[
                    ("used", invite_usage.used.to_string()),
                    ("limit", invite_usage.quota.limit.to_string()),
                    ("period", invite_period_label.clone()),
                ],
            )
        };

        Self {
            id: group.id,
            name: group.name.clone(),
            description: group.description.clone().unwrap_or_default(),
            max_urls_input: group
                .max_urls
                .map(|value| value.to_string())
                .unwrap_or_default(),
            max_urls_display: quota_display(group.max_urls, settings.max_urls_per_user(), lang),
            max_api_tokens_input: group
                .max_api_tokens
                .map(|value| value.to_string())
                .unwrap_or_default(),
            max_api_tokens_display: quota_display(
                group.max_api_tokens,
                settings.max_api_tokens_per_user(),
                lang,
            ),
            max_invites_input: group
                .max_invites
                .map(|value| value.to_string())
                .unwrap_or_default(),
            max_invites_display: quota_display(
                group.max_invites,
                settings.max_invites_per_period(),
                lang,
            ),
            invite_period: invite_period.as_str().to_string(),
            invite_period_label,
            invite_usage_display,
            invite_quota_exhausted: invite_usage.exhausted(),
            members: overview.members,
            is_default: group.is_default,
            allow_invites: group.allow_invites,
            allow_custom_slug: group.allow_custom_slug,
            allow_password: group.allow_password,
            allow_expiry: group.allow_expiry,
            allow_max_clicks: group.allow_max_clicks,
            allow_tags: group.allow_tags,
            allow_api_tokens: group.allow_api_tokens,
            allow_domain_selection: group.allow_domain_selection,
            redirect_mode_policy: redirect_mode_policy.as_str().to_string(),
            redirect_mode_policy_label: lang.t(redirect_mode_policy.label_key()).to_string(),
            default_domain_id: group.default_domain_id,
            restrict_domains: group.restrict_domains,
            allowed_domain_ids: overview.allowed_domain_ids.clone(),
            disabled_domains,
            default_domain_disabled_id,
            default_domain_disabled_name,
            features_label: i18n::format(
                lang,
                "admin.groups.features_count",
                &[("count", allowed_features.to_string())],
            ),
            created: time::humanize_lang(lang, &group.created_at),
        }
    }
}

impl GroupRow {
    /// Whether this group uses `domain_id` as its default.
    pub fn uses_default_domain(&self, domain_id: &i64) -> bool {
        self.default_domain_id == Some(*domain_id)
    }

    /// Whether `domain_id` is in the explicit allowlist.
    pub fn allows_domain(&self, domain_id: &i64) -> bool {
        self.allowed_domain_ids.contains(domain_id)
    }

    pub fn has_disabled_default_domain(&self) -> bool {
        self.default_domain_disabled_id.is_some()
    }

    pub fn disabled_default_domain_id(&self) -> i64 {
        self.default_domain_disabled_id.unwrap_or(0)
    }

    pub fn disabled_default_domain_name(&self) -> &str {
        self.default_domain_disabled_name.as_deref().unwrap_or("")
    }

    /// Which `domain_policy` radio the form should preselect. Selection off
    /// wins over the allowlist, because a member who cannot choose never
    /// reaches the list.
    pub fn domain_policy(&self) -> &'static str {
        match (self.allow_domain_selection, self.restrict_domains) {
            (false, _) => "default",
            (true, true) => "list",
            (true, false) => "all",
        }
    }
}

/// `2 of 5 per month` (plus a used-up suffix) for the invite form. Empty when
/// the group has unlimited invites, so the option stays uncluttered.
fn invite_usage_label(lang: Lang, usage: InviteUsage) -> String {
    if usage.unlimited() {
        return String::new();
    }
    let values = [
        ("used", usage.used.to_string()),
        ("limit", usage.quota.limit.to_string()),
        ("period", super::period_label(lang, usage.quota.period)),
    ];
    if usage.exhausted() {
        i18n::format(lang, "admin.invites.quota_option_exhausted", &values)
    } else {
        i18n::format(lang, "admin.invites.quota_option", &values)
    }
}

/// A quota as shown in the group table.
fn quota_display(value: Option<i64>, site_default: i64, lang: Lang) -> String {
    match value {
        // An inherited unlimited quota must not read as the number zero.
        None if site_default <= 0 => lang.t("admin.groups.unlimited_site_default").to_string(),
        None => i18n::format(
            lang,
            "admin.groups.inherit_value",
            &[("value", site_default.to_string())],
        ),
        Some(0) => lang.t("admin.groups.unlimited").to_string(),
        Some(value) => value.to_string(),
    }
}

/// Settings page.
#[derive(Debug)]
pub struct SettingsPage {
    pub site_name: String,
    pub section: &'static str,
    pub chrome: Chrome,
    pub values: SettingsValues,
    pub lang: Lang,
}

themed_page!(settings_templates, SettingsPageView, SettingsPage,
    Verdant => "admin/settings.html",
    Aurora => "themes/aurora/admin/settings.html",
    Paper => "themes/paper/admin/settings.html",
    Signal => "themes/signal/admin/settings.html",
    Gilded => "themes/gilded/admin/settings.html",
    Swiss => "themes/swiss/admin/settings.html",
);

/// Current values of every editable setting.
#[derive(Debug, Clone, Default)]
pub struct SettingsValues {
    pub site_name: String,
    pub registration_mode: String,
    pub default_slug_length: i64,
    pub max_urls_per_user: i64,
    pub max_api_tokens_per_user: i64,
    pub max_invites_per_period: i64,
    pub invite_period: String,
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

/// Themes page: enable/disable the compiled-in designs and choose the default.
#[derive(Debug)]
pub struct ThemesPage {
    pub site_name: String,
    pub section: &'static str,
    pub chrome: Chrome,
    pub themes: Vec<AdminThemeRow>,
    /// `all` or `admin`; the raw token drives the select.
    pub switching: String,
    pub custom_count: usize,
    pub max_custom: i64,
    pub lang: Lang,
}

themed_page!(themes_templates, ThemesPageView, ThemesPage,
    Verdant => "admin/themes.html",
    Aurora => "themes/aurora/admin/themes.html",
    Paper => "themes/paper/admin/themes.html",
    Signal => "themes/signal/admin/themes.html",
    Gilded => "themes/gilded/admin/themes.html",
    Swiss => "themes/swiss/admin/themes.html",
);

/// One uploaded theme on its edit page.
#[derive(Debug, Clone)]
pub struct CustomThemeEdit {
    pub id: i64,
    pub key: String,
    pub name: String,
    pub description: String,
    pub appearance: String,
    pub size: String,
    pub created: String,
    pub filename: String,
    pub css_url: String,
}

impl CustomThemeEdit {
    fn new(row: &crate::db::custom_themes::CustomThemeRow, lang: Lang) -> Self {
        Self {
            id: row.id,
            key: row.key.clone(),
            name: row.name.clone(),
            description: row.description.clone(),
            appearance: row.appearance.as_str().to_string(),
            size: format!("{} B", row.css_size.max(0)),
            created: time::humanize_lang(lang, &row.created_at),
            filename: row.original_filename.clone(),
            css_url: theme_service::custom_css_url(&row.key, &row.css_version),
        }
    }
}

/// Edit page for an uploaded theme.
#[derive(Debug)]
pub struct CustomThemePage {
    pub site_name: String,
    pub section: &'static str,
    pub chrome: Chrome,
    pub custom_theme: CustomThemeEdit,
    pub lang: Lang,
}

themed_page!(custom_theme_templates, CustomThemePageView, CustomThemePage,
    Verdant => "admin/theme_edit.html",
    Aurora => "themes/aurora/admin/theme_edit.html",
    Paper => "themes/paper/admin/theme_edit.html",
    Signal => "themes/signal/admin/theme_edit.html",
    Gilded => "themes/gilded/admin/theme_edit.html",
    Swiss => "themes/swiss/admin/theme_edit.html",
);

/// Audit page.
#[derive(Debug)]
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

themed_page!(audit_templates, AuditPageView, AuditPage,
    Verdant => "admin/audit.html",
    Aurora => "themes/aurora/admin/audit.html",
    Paper => "themes/paper/admin/audit.html",
    Signal => "themes/signal/admin/audit.html",
    Gilded => "themes/gilded/admin/audit.html",
    Swiss => "themes/swiss/admin/audit.html",
);

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
    #[serde(default, deserialize_with = "crate::routes::dto::optional_i64")]
    pub page: Option<i64>,
    #[serde(default)]
    pub q: Option<String>,
    #[serde(default)]
    pub role: Option<String>,
    #[serde(default)]
    pub status: Option<String>,
    #[serde(default, deserialize_with = "crate::routes::dto::optional_i64")]
    pub group_id: Option<i64>,
    #[serde(default, deserialize_with = "crate::routes::dto::optional_i64")]
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
    #[serde(default)]
    pub group_id: Option<String>,
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
    pub group_id: Option<String>,
    #[serde(default)]
    pub max_uses: Option<String>,
    #[serde(default)]
    pub expires_at: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct GroupForm {
    pub name: String,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub max_urls: Option<String>,
    #[serde(default)]
    pub max_api_tokens: Option<String>,
    #[serde(default)]
    pub max_invites: Option<String>,
    /// `day`, `week` or `month`; empty keeps the month default.
    #[serde(default)]
    pub invite_period: Option<String>,
    #[serde(default)]
    pub allow_invites: Option<String>,
    #[serde(default)]
    pub allow_custom_slug: Option<String>,
    #[serde(default)]
    pub allow_password: Option<String>,
    #[serde(default)]
    pub allow_expiry: Option<String>,
    #[serde(default)]
    pub allow_max_clicks: Option<String>,
    #[serde(default)]
    pub allow_tags: Option<String>,
    #[serde(default)]
    pub allow_api_tokens: Option<String>,
    #[serde(default)]
    pub allow_domain_selection: Option<String>,
    /// `direct`, `splash` or `choice`; missing falls back to `choice`.
    #[serde(default)]
    pub redirect_mode_policy: Option<String>,
    #[serde(default)]
    pub default_domain_id: Option<String>,
    #[serde(default)]
    pub restrict_domains: Option<String>,
    /// `all`, `default` or `list`; the radio that replaces the old
    /// `allow_domain_selection` + `restrict_domains` checkbox pair in the form.
    #[serde(default)]
    pub domain_policy: Option<String>,
    /// Dynamic `domain_<id>` checkboxes; serde cannot model them statically.
    #[serde(flatten)]
    pub extra: std::collections::HashMap<String, String>,
    #[serde(default)]
    pub is_default: Option<String>,
}

impl GroupForm {
    /// Resolve the domain controls into the two booleans the database keeps.
    ///
    /// The form posts one `domain_policy` radio: `all` lets members pick any
    /// active domain, `default` pins them to the group default, and `list`
    /// narrows the choice to the checked `domain_<id>` boxes. The older
    /// `allow_domain_selection` / `restrict_domains` pair is still accepted so
    /// API-shaped posts and existing scripts keep their meaning.
    fn domain_policy(&self) -> Result<(bool, bool), AppError> {
        match self.domain_policy.as_deref().map(str::trim) {
            Some("all") => Ok((true, false)),
            Some("default") => Ok((false, false)),
            Some("list") => Ok((true, true)),
            None | Some("") => Ok((
                self.allow_domain_selection.is_some(),
                self.restrict_domains.is_some(),
            )),
            Some(other) => Err(AppError::validation(
                "domain_policy",
                format!("domain policy must be all, default or list, not {other:?}"),
            )),
        }
    }

    fn into_draft(self, existing: Option<&user_groups::UserGroup>) -> Result<GroupDraft, AppError> {
        let (allow_domain_selection, restrict_domains) = self.domain_policy()?;
        GroupDraft {
            name: self.name,
            description: self.description,
            max_urls: parse_quota("max_urls", self.max_urls)?,
            max_api_tokens: parse_quota("max_api_tokens", self.max_api_tokens)?,
            max_invites: parse_quota("max_invites", self.max_invites)?,
            invite_period: match self.invite_period.as_deref().map(str::trim) {
                None | Some("") => InvitePeriod::default(),
                Some(value) => InvitePeriod::parse(value).ok_or_else(|| {
                    AppError::validation(
                        "invite_period",
                        "invite period must be day, week or month",
                    )
                })?,
            },
            // Browsers omit unchecked boxes entirely.
            allow_invites: self.allow_invites.is_some(),
            allow_custom_slug: self.allow_custom_slug.is_some(),
            allow_password: self.allow_password.is_some(),
            allow_expiry: self.allow_expiry.is_some(),
            allow_max_clicks: self.allow_max_clicks.is_some(),
            allow_tags: self.allow_tags.is_some(),
            allow_api_tokens: self.allow_api_tokens.is_some(),
            allow_domain_selection,
            redirect_mode_policy: match self
                .redirect_mode_policy
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty())
            {
                None => existing
                    .map(|group| RedirectModePolicy::from_db(&group.redirect_mode_policy))
                    .unwrap_or_default(),
                Some(value) => RedirectModePolicy::parse(value).ok_or_else(|| {
                    AppError::validation(
                        "redirect_mode_policy",
                        "redirect mode policy must be direct, splash or choice",
                    )
                })?,
            },
            default_domain_id: parse_optional_number(
                "default_domain_id",
                self.default_domain_id.as_deref(),
            )?,
            restrict_domains,
            allowed_domain_ids: Some(
                self.extra
                    .iter()
                    .filter_map(|(key, _)| {
                        key.strip_prefix("domain_")
                            .and_then(|id| id.parse::<i64>().ok())
                    })
                    .collect(),
            ),
            make_default: self.is_default.is_some(),
        }
        .validate()
    }
}

/// Empty means "inherit"; a number must parse.
fn parse_quota(field: &'static str, raw: Option<String>) -> Result<Option<i64>, AppError> {
    match raw
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        None => Ok(None),
        Some(value) => value
            .parse::<i64>()
            .map(Some)
            .map_err(|_| AppError::validation(field, format!("{field} must be a number"))),
    }
}

/// Optional number from a form field: empty means missing, garbage is rejected.
fn parse_optional_number(field: &'static str, raw: Option<&str>) -> Result<Option<i64>, AppError> {
    match raw.map(str::trim).filter(|value| !value.is_empty()) {
        None => Ok(None),
        Some(value) => value
            .parse::<i64>()
            .map(Some)
            .map_err(|_| AppError::validation(field, format!("{field} must be a number"))),
    }
}

#[derive(Debug, Deserialize)]
pub struct DeleteGroupForm {
    #[serde(default)]
    pub reassign_to: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct SettingsForm {
    pub site_name: String,
    pub registration_mode: String,
    pub default_slug_length: String,
    pub max_urls_per_user: String,
    /// Optional so a page cached before v1.2 still submits successfully.
    #[serde(default)]
    pub max_api_tokens_per_user: Option<String>,
    #[serde(default)]
    pub max_invites_per_period: Option<String>,
    #[serde(default)]
    pub invite_period: Option<String>,
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
        group_id: query.group_id,
    };
    let rows = admin::list_users(&state.db, &filter, PER_PAGE, (page - 1) * PER_PAGE).await?;
    let total = admin::count_users(&state.db, &filter).await?;
    let counts = admin::link_counts(&state.db).await?;
    let group_rows = user_groups::list(&state.db).await.map_err(AppError::from)?;
    let groups: Vec<GroupOption> = group_rows
        .iter()
        .map(|group| GroupOption {
            id: group.id,
            name: group.name.clone(),
        })
        .collect();
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
    if let Some(value) = query.group_id {
        params.push(("group_id", value.to_string()));
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
                    group_id: user
                        .group_id
                        .map(|group_id| group_id.to_string())
                        .unwrap_or_default(),
                    group_name: user
                        .group_id
                        .and_then(|group_id| {
                            group_rows
                                .iter()
                                .find(|group| group.id == group_id)
                                .map(|group| group.name.clone())
                        })
                        .unwrap_or_else(|| lang.t("admin.users.ungrouped").to_string()),
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
            groups,
            links: page_links("/app/admin/users", &params, page, total_pages),
            page,
            total_pages,
            total,
            q: clean(&query.q).unwrap_or_default(),
            role: clean(&query.role).unwrap_or_default(),
            status: clean(&query.status).unwrap_or_default(),
            group_id: query
                .group_id
                .map(|group_id| group_id.to_string())
                .unwrap_or_default(),
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

    let max_uses = match parse_optional_number("max_uses", payload.max_uses.as_deref()) {
        Ok(value) => value,
        Err(_) => return Ok(redirect_err("/app/admin/invites", "invalid").into_response()),
    };
    let group_id = match parse_optional_number("group_id", payload.group_id.as_deref()) {
        Ok(value) => value,
        Err(_) => return Ok(redirect_err("/app/admin/invites", "invalid").into_response()),
    };

    let context = context(&state, &client, &current.user);
    match context
        .create_invite(
            payload.note.as_deref(),
            group_id,
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

/// `GET /app/admin/themes`
pub async fn themes_page(
    State(state): State<AppState>,
    MaybeUser(current): MaybeUser,
    Query(query): Query<AdminQuery>,
) -> Result<Response, AppError> {
    let Some(current) = admin_user(current)? else {
        return Ok(redirect_to_login(Some("/app/admin/themes")).into_response());
    };
    render_themes(
        &state,
        &current.user,
        flash_from_query(query.ok.as_deref(), query.err.as_deref()),
    )
    .await
}

/// `POST /app/admin/themes`
pub async fn update_themes(
    State(state): State<AppState>,
    client: ClientInfo,
    MaybeUser(current): MaybeUser,
    form: std::result::Result<Form<HashMap<String, String>>, FormRejection>,
) -> Result<Response, AppError> {
    let Some(current) = admin_user(current)? else {
        return Ok(redirect_to_login(Some("/app/admin/themes")).into_response());
    };
    let Ok(Form(payload)) = form else {
        return Ok(redirect_err("/app/admin/themes", "invalid").into_response());
    };

    let Some(default_theme) = payload
        .get("default_theme")
        .map(String::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
    else {
        return Ok(redirect_err("/app/admin/themes", "theme").into_response());
    };
    let Some(raw_switching) = payload.get("theme_switching") else {
        return Ok(redirect_err("/app/admin/themes", "theme").into_response());
    };
    let switching = match raw_switching.trim().to_ascii_lowercase().as_str() {
        "all" => ThemeSwitching::All,
        "admin" | "admins" | "admin_only" | "admin-only" => ThemeSwitching::AdminOnly,
        _ => return Ok(redirect_err("/app/admin/themes", "theme").into_response()),
    };
    // Derive the enabled set from the submitted checkboxes instead of a fixed
    // field list, so a newly compiled theme is controllable without editing
    // this handler. The service validates every key against ThemeKey.
    let enabled: Vec<String> = payload
        .keys()
        .filter_map(|key| key.strip_prefix("enabled_"))
        .filter(|key| !key.is_empty())
        .map(str::to_string)
        .collect();
    let applied = match theme_service::update_selection(
        &state.db,
        &state.themes,
        &state.settings,
        Some(&enabled),
        Some(default_theme),
        Some(switching),
        Some(current.user.id),
    )
    .await
    {
        Ok(applied) => applied,
        Err(err) => {
            tracing::debug!(error = %err, "theme selection rejected");
            return Ok(redirect_err("/app/admin/themes", "theme").into_response());
        }
    };

    crate::services::audit::Audit::new(crate::services::audit::action::ADMIN_THEMES_UPDATED)
        .actor(current.user.id, &current.user.role)
        .target(crate::services::audit::target::THEMES, "catalogue")
        .metadata(serde_json::json!({
            "enabled": applied.enabled,
            "default": applied.default_key,
            "switching": applied.switching.as_str(),
        }))
        .ip_hash(client.ip_hash.clone())
        .record(&state.db)
        .await;

    Ok(redirect_ok("/app/admin/themes", "saved").into_response())
}

/// Fields of the custom theme upload/edit form.
#[derive(Debug, Default)]
struct ThemeUpload {
    name: String,
    description: String,
    appearance: theme_service::ThemeAppearance,
    /// Whether the form sent the new `appearance` field. Kept so an old cached
    /// form that only posts `is_dark_only` still maps to dark instead of auto.
    appearance_explicit: bool,
    css: Option<String>,
    filename: Option<String>,
}

/// Read a multipart theme form. `require_css` is true on upload, false on an
/// edit that only changes metadata.
async fn read_theme_upload(
    mut multipart: Multipart,
    require_css: bool,
) -> Result<ThemeUpload, AppError> {
    let mut form = ThemeUpload::default();
    loop {
        let field = multipart
            .next_field()
            .await
            .map_err(|err| AppError::bad_request(format!("invalid theme upload: {err}")))?;
        let Some(field) = field else {
            break;
        };
        let name = field.name().unwrap_or_default().to_string();
        match name.as_str() {
            "name" => {
                form.name = field
                    .text()
                    .await
                    .map_err(|err| AppError::bad_request(format!("invalid name field: {err}")))?;
            }
            "description" => {
                form.description = field.text().await.map_err(|err| {
                    AppError::bad_request(format!("invalid description field: {err}"))
                })?;
            }
            "appearance" => {
                let value = field.text().await.map_err(|err| {
                    AppError::bad_request(format!("invalid appearance field: {err}"))
                })?;
                form.appearance = theme_service::ThemeAppearance::parse(&value);
                form.appearance_explicit = true;
            }
            // Backwards compatibility for a form cached before the three-way
            // appearance control existed.
            "is_dark_only" => {
                let value = field.text().await.map_err(|err| {
                    AppError::bad_request(format!("invalid dark-only field: {err}"))
                })?;
                let enabled = matches!(
                    value.trim().to_ascii_lowercase().as_str(),
                    "1" | "on" | "true" | "yes"
                );
                if enabled && !form.appearance_explicit {
                    form.appearance = theme_service::ThemeAppearance::Dark;
                }
            }
            "css" => {
                let filename = field
                    .file_name()
                    .map(str::to_string)
                    .filter(|name| !name.trim().is_empty());
                let bytes = field
                    .bytes()
                    .await
                    .map_err(|err| AppError::bad_request(format!("invalid CSS file: {err}")))?;
                // Browsers submit an empty file part when the picker was left
                // untouched. Treat that as "keep the current stylesheet" on the
                // edit form instead of validating a zero-byte CSS file.
                if bytes.is_empty() && filename.is_none() {
                    continue;
                }
                form.css = Some(custom_theme::validate_css(&bytes)?);
                form.filename = filename;
            }
            _ => {
                // Drain unknown fields so the parser can reach the next part.
                let _ = field.bytes().await;
            }
        }
    }
    if require_css && form.css.is_none() {
        return Err(AppError::validation("css", "choose a CSS file"));
    }
    Ok(form)
}

/// Map a theme service error to a flash code the page understands.
fn theme_error_code(err: &AppError) -> &'static str {
    match err {
        AppError::Validation {
            field: "name" | "description",
            ..
        } => "theme_name",
        AppError::Validation { field: "css", .. } => "theme_css",
        AppError::Validation {
            field: "themes", ..
        } => "theme_quota",
        AppError::Conflict(_) => "theme_default",
        AppError::NotFound => "not_found",
        _ => "invalid",
    }
}

/// `POST /app/admin/themes/custom` — upload a stylesheet.
pub async fn create_custom_theme(
    State(state): State<AppState>,
    client: ClientInfo,
    MaybeUser(current): MaybeUser,
    multipart: std::result::Result<Multipart, MultipartRejection>,
) -> Result<Response, AppError> {
    let Some(current) = admin_user(current)? else {
        return Ok(redirect_to_login(Some("/app/admin/themes")).into_response());
    };
    let Ok(multipart) = multipart else {
        return Ok(redirect_err("/app/admin/themes", "theme_upload").into_response());
    };
    let form = match read_theme_upload(multipart, true).await {
        Ok(form) => form,
        Err(err) => {
            tracing::debug!(error = %err, "custom theme upload rejected");
            return Ok(redirect_err("/app/admin/themes", theme_error_code(&err)).into_response());
        }
    };
    let css = form.css.expect("required CSS was validated");
    let filename = form.filename.as_deref().unwrap_or("theme.css");

    let created = custom_theme::create(
        &state.db,
        &state.themes,
        &state.settings,
        &state.config.themes_dir,
        &form.name,
        &form.description,
        form.appearance,
        &css,
        filename,
        Some(current.user.id),
    )
    .await;
    match created {
        Ok(row) => {
            crate::services::audit::Audit::new(crate::services::audit::action::ADMIN_THEME_CREATED)
                .actor(current.user.id, &current.user.role)
                .target(crate::services::audit::target::THEMES, &row.key)
                .metadata(serde_json::json!({
                    "key": row.key,
                    "name": row.name,
                    "size": row.css_size,
                    "appearance": row.appearance.as_str(),
                }))
                .ip_hash(client.ip_hash.clone())
                .record(&state.db)
                .await;
            Ok(redirect_ok("/app/admin/themes", "theme_created").into_response())
        }
        Err(err) => {
            tracing::debug!(error = %err, "custom theme upload failed");
            Ok(redirect_err("/app/admin/themes", theme_error_code(&err)).into_response())
        }
    }
}

/// `GET /app/admin/themes/custom/{id}` — edit one upload.
pub async fn custom_theme_page(
    State(state): State<AppState>,
    MaybeUser(current): MaybeUser,
    Path(id): Path<i64>,
    Query(query): Query<AdminQuery>,
) -> Result<Response, AppError> {
    let Some(current) = admin_user(current)? else {
        return Ok(redirect_to_login(Some("/app/admin/themes")).into_response());
    };
    let Some(row) = custom_db::find(&state.db, id)
        .await
        .map_err(|err| AppError::internal(format!("cannot read custom theme: {err}")))?
    else {
        return Err(AppError::NotFound);
    };
    let lang = i18n::current();
    render(
        CustomThemePage {
            site_name: state.settings.site_name(),
            section: "themes",
            chrome: Chrome::new(
                &state,
                &current.user,
                "admin",
                flash_from_query(query.ok.as_deref(), query.err.as_deref()),
            ),
            custom_theme: CustomThemeEdit::new(&row, lang),
            lang,
        },
        StatusCode::OK,
    )
}

/// `POST /app/admin/themes/custom/{id}` — update metadata and/or replace CSS.
pub async fn update_custom_theme(
    State(state): State<AppState>,
    client: ClientInfo,
    MaybeUser(current): MaybeUser,
    Path(id): Path<i64>,
    multipart: std::result::Result<Multipart, MultipartRejection>,
) -> Result<Response, AppError> {
    let Some(current) = admin_user(current)? else {
        return Ok(redirect_to_login(Some("/app/admin/themes")).into_response());
    };
    let edit_path = format!("/app/admin/themes/custom/{id}");
    let Ok(multipart) = multipart else {
        return Ok(redirect_err(&edit_path, "theme_upload").into_response());
    };
    let form = match read_theme_upload(multipart, false).await {
        Ok(form) => form,
        Err(err) => {
            tracing::debug!(error = %err, "custom theme update rejected");
            return Ok(redirect_err(&edit_path, theme_error_code(&err)).into_response());
        }
    };

    match custom_theme::update(
        &state.db,
        &state.themes,
        &state.settings,
        &state.config.themes_dir,
        id,
        &form.name,
        &form.description,
        form.appearance,
        form.css.as_deref(),
        form.filename.as_deref(),
    )
    .await
    {
        Ok(row) => {
            crate::services::audit::Audit::new(crate::services::audit::action::ADMIN_THEME_UPDATED)
                .actor(current.user.id, &current.user.role)
                .target(crate::services::audit::target::THEMES, &row.key)
                .metadata(serde_json::json!({
                    "key": row.key,
                    "name": row.name,
                    "size": row.css_size,
                    "appearance": row.appearance.as_str(),
                }))
                .ip_hash(client.ip_hash.clone())
                .record(&state.db)
                .await;
            Ok(redirect_ok(&edit_path, "theme_updated").into_response())
        }
        Err(AppError::NotFound) => {
            Ok(redirect_err("/app/admin/themes", "not_found").into_response())
        }
        Err(err) => {
            tracing::debug!(error = %err, "custom theme update failed");
            Ok(redirect_err(&edit_path, theme_error_code(&err)).into_response())
        }
    }
}

/// `POST /app/admin/themes/custom/{id}/delete` — remove an upload and its CSS.
pub async fn delete_custom_theme(
    State(state): State<AppState>,
    client: ClientInfo,
    MaybeUser(current): MaybeUser,
    Path(id): Path<i64>,
) -> Result<Response, AppError> {
    let Some(current) = admin_user(current)? else {
        return Ok(redirect_to_login(Some("/app/admin/themes")).into_response());
    };
    let row = custom_db::find(&state.db, id)
        .await
        .map_err(|err| AppError::internal(format!("cannot read custom theme: {err}")))?;
    let Some(row) = row else {
        return Ok(redirect_err("/app/admin/themes", "not_found").into_response());
    };

    match custom_theme::delete(
        &state.db,
        &state.themes,
        &state.settings,
        &state.config.themes_dir,
        id,
    )
    .await
    {
        Ok(()) => {
            crate::services::audit::Audit::new(crate::services::audit::action::ADMIN_THEME_DELETED)
                .actor(current.user.id, &current.user.role)
                .target(crate::services::audit::target::THEMES, &row.key)
                .metadata(serde_json::json!({
                    "key": row.key,
                    "name": row.name,
                }))
                .ip_hash(client.ip_hash.clone())
                .record(&state.db)
                .await;
            Ok(redirect_ok("/app/admin/themes", "theme_deleted").into_response())
        }
        Err(AppError::NotFound) => {
            Ok(redirect_err("/app/admin/themes", "not_found").into_response())
        }
        Err(err) => {
            tracing::debug!(error = %err, "custom theme delete failed");
            Ok(redirect_err("/app/admin/themes", theme_error_code(&err)).into_response())
        }
    }
}

async fn render_themes(
    state: &AppState,
    user: &User,
    flash: Option<crate::routes::pages::Flash>,
) -> Result<Response, AppError> {
    let lang = i18n::current();
    render(
        ThemesPage {
            section: "themes",
            site_name: state.settings.site_name(),
            chrome: Chrome::new(state, user, "admin", flash),
            themes: theme_service::admin_rows(&state.themes, lang),
            switching: state.settings.theme_switching().as_str().to_string(),
            custom_count: state.themes.custom_rows().len(),
            max_custom: custom_theme::MAX_THEMES,
            lang,
        },
        StatusCode::OK,
    )
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

    if let Some(value) = payload.max_api_tokens_per_user {
        values.push(("max_api_tokens_per_user".to_string(), value));
    }
    if let Some(value) = payload.max_invites_per_period {
        values.push(("max_invites_per_period".to_string(), value));
    }
    if let Some(value) = payload
        .invite_period
        .filter(|value| !value.trim().is_empty())
    {
        values.push(("invite_period".to_string(), value));
    }

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

    let group_id = match payload.group_id.as_deref() {
        // The form always submits the select (including the empty "Ungrouped"
        // option), so an empty value is an explicit clear rather than "absent".
        Some(raw) => {
            let raw = raw.trim();
            if raw.is_empty() {
                Some(None)
            } else {
                Some(Some(raw.parse::<i64>().map_err(|_| {
                    AppError::validation("group_id", "the selected group is invalid")
                })?))
            }
        }
        // Older cached forms may not contain the field at all.
        None => None,
    };

    Ok(
        match context(&state, &client, &current.user)
            .update_user(
                id,
                payload.role.as_deref(),
                payload.status.as_deref(),
                group_id,
                None,
            )
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

/// `GET /app/admin/groups`
pub async fn groups_page(
    State(state): State<AppState>,
    MaybeUser(current): MaybeUser,
    Query(query): Query<AdminQuery>,
) -> Result<Response, AppError> {
    let Some(current) = admin_user(current)? else {
        return Ok(redirect_to_login(Some("/app/admin/groups")).into_response());
    };
    render_groups(
        &state,
        &current.user,
        flash_from_query(query.ok.as_deref(), query.err.as_deref()),
    )
    .await
}

/// `POST /app/admin/groups`
pub async fn create_group(
    State(state): State<AppState>,
    client: ClientInfo,
    MaybeUser(current): MaybeUser,
    form: std::result::Result<Form<GroupForm>, FormRejection>,
) -> Result<Response, AppError> {
    let Some(current) = admin_user(current)? else {
        return Ok(redirect_to_login(Some("/app/admin/groups")).into_response());
    };
    let Ok(Form(payload)) = form else {
        return Ok(redirect_err("/app/admin/groups", "invalid").into_response());
    };

    let outcome = match payload.into_draft(None) {
        Ok(draft) => {
            context(&state, &client, &current.user)
                .create_group(draft)
                .await
        }
        Err(err) => Err(err),
    };
    Ok(match outcome {
        Ok(_) => redirect_ok("/app/admin/groups", "group").into_response(),
        Err(err) => {
            redirect_err("/app/admin/groups", super::urls::error_code(&err)).into_response()
        }
    })
}

/// `POST /app/admin/groups/{id}`
pub async fn update_group(
    State(state): State<AppState>,
    client: ClientInfo,
    MaybeUser(current): MaybeUser,
    Path(id): Path<i64>,
    form: std::result::Result<Form<GroupForm>, FormRejection>,
) -> Result<Response, AppError> {
    let Some(current) = admin_user(current)? else {
        return Ok(redirect_to_login(Some("/app/admin/groups")).into_response());
    };
    let Ok(Form(payload)) = form else {
        return Ok(redirect_err("/app/admin/groups", "invalid").into_response());
    };

    // The form is a full representation, but a scripted or cached post may omit
    // the redirect-mode radios. Keep the stored policy in that case instead of
    // silently widening the group to `choice`.
    let existing = match user_groups::find_by_id(&state.db, id).await {
        Ok(Some(group)) => group,
        Ok(None) => return Ok(redirect_err("/app/admin/groups", "not_found").into_response()),
        Err(err) => return Err(AppError::from(err)),
    };
    let outcome = match payload.into_draft(Some(&existing)) {
        Ok(draft) => {
            context(&state, &client, &current.user)
                .update_group(id, draft)
                .await
        }
        Err(err) => Err(err),
    };
    Ok(match outcome {
        Ok(_) => redirect_ok("/app/admin/groups", "group").into_response(),
        Err(err) => {
            redirect_err("/app/admin/groups", super::urls::error_code(&err)).into_response()
        }
    })
}

/// `POST /app/admin/groups/{id}/delete`
pub async fn delete_group(
    State(state): State<AppState>,
    client: ClientInfo,
    MaybeUser(current): MaybeUser,
    Path(id): Path<i64>,
    form: std::result::Result<Form<DeleteGroupForm>, FormRejection>,
) -> Result<Response, AppError> {
    let Some(current) = admin_user(current)? else {
        return Ok(redirect_to_login(Some("/app/admin/groups")).into_response());
    };
    let Ok(Form(payload)) = form else {
        return Ok(redirect_err("/app/admin/groups", "invalid").into_response());
    };

    let target = match payload
        .reassign_to
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        Some(raw) => match raw.parse::<i64>() {
            Ok(value) => Some(value),
            Err(_) => return Ok(redirect_err("/app/admin/groups", "invalid").into_response()),
        },
        None => None,
    };

    Ok(
        match context(&state, &client, &current.user)
            .delete_group(id, target)
            .await
        {
            Ok(()) => redirect_ok("/app/admin/groups", "group_deleted").into_response(),
            Err(err) => {
                redirect_err("/app/admin/groups", super::urls::error_code(&err)).into_response()
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

/// The admin page lists every group because the delete and assignment selects
/// need all of them; only the JSON list endpoint is paginated.
async fn render_groups(
    state: &AppState,
    user: &User,
    flash: Option<crate::routes::pages::Flash>,
) -> Result<Response, AppError> {
    let overview = group_service::overview(&state.db).await?;
    let lang = i18n::current();
    let domains = state.domains.active_options();
    let active_domain_ids: Vec<i64> = domains.iter().map(|domain| domain.id).collect();
    let groups: Vec<crate::db::user_groups::UserGroup> =
        overview.iter().map(|row| row.group.clone()).collect();
    let usage = usage_map(&state.db, &state.settings, &groups, time::now()).await?;
    let mut rows = Vec::with_capacity(overview.len());
    for row in &overview {
        rows.push(GroupRow::new(
            &state.settings,
            lang,
            row,
            &active_domain_ids,
            &state.domains,
            usage[&row.group.id],
        ));
    }
    render(
        GroupsPage {
            section: "groups",
            site_name: state.settings.site_name(),
            chrome: Chrome::new(state, user, "admin", flash),
            groups: rows,
            domains,
            lang,
        },
        StatusCode::OK,
    )
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
    let group_rows = user_groups::list(&state.db).await.map_err(AppError::from)?;
    let usage = usage_map(&state.db, &state.settings, &group_rows, time::now()).await?;
    let groups: Vec<InviteGroupOption> = group_rows
        .iter()
        .map(|group| InviteGroupOption {
            id: group.id,
            name: group.name.clone(),
            quota_label: invite_usage_label(lang, usage[&group.id]),
        })
        .collect();
    let site_usage = usage_for(&state.db, &state.settings, None, time::now()).await?;
    let site_quota_label = invite_usage_label(lang, site_usage);
    let mut has_legacy = false;
    let invites: Vec<InviteRow> = rows
        .iter()
        .map(|invite: &Invite| {
            let usable = invite.is_usable(&now);
            let code = reveal(state.config.secret.as_slice(), invite);
            if code.is_none() {
                has_legacy = true;
            }
            // Only a usable code gets a link; the table and the create banner
            // must both hide a URL that redemption would reject.
            let link = if usable {
                code.as_deref()
                    .map(|code| registration_link(&state.config.base_url, code))
            } else {
                None
            };
            InviteRow {
                id: invite.id,
                prefix: invite.code_prefix.clone(),
                code,
                link,
                note: invite.note.clone().unwrap_or_default(),
                group_name: invite
                    .group_id
                    .and_then(|group_id| {
                        group_rows
                            .iter()
                            .find(|group| group.id == group_id)
                            .map(|group| group.name.clone())
                    })
                    .unwrap_or_else(|| lang.t("admin.invites.group_default").to_string()),
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
                usable,
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
    let response = render(
        InvitesPage {
            section: "invites",
            site_name: state.settings.site_name(),
            chrome: Chrome::new(state, user, "admin", flash),
            invites,
            groups,
            site_quota_label,
            new_code,
            new_link,
            has_legacy,
            registration_mode: state.settings.registration_mode().as_str().to_string(),
            registration_mode_label: i18n::registration_mode_label(
                lang,
                state.settings.registration_mode().as_str(),
            ),
            lang,
        },
        StatusCode::OK,
    )?;
    Ok(no_store(response))
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
        max_api_tokens_per_user: settings.max_api_tokens_per_user(),
        max_invites_per_period: settings.max_invites_per_period(),
        invite_period: settings.invite_period().as_str().to_string(),
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

fn render<T: ThemedContext>(template: T, status: StatusCode) -> Result<Response, AppError> {
    match template.themed().render() {
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
    fn shows_an_inherited_unlimited_quota_as_unlimited() {
        assert_eq!(quota_display(None, 0, Lang::En), "Unlimited (site default)");
        assert_eq!(quota_display(None, 1000, Lang::En), "Site default (1000)");
        assert_eq!(quota_display(Some(0), 1000, Lang::En), "Unlimited");
        assert_eq!(quota_display(Some(5), 1000, Lang::En), "5");
    }

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
