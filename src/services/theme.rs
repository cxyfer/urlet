//! Compiled-in themes, administrator uploads and the catalogue selection.
//!
//! A *compiled-in theme* is a complete visual identity: a CSS bundle plus a
//! directory of Askama overrides under `templates/themes/<key>/`. The base
//! templates in `templates/` are themselves the Verdant theme, and they double
//! as the fallback for any page a theme does not override.
//!
//! An *uploaded theme* is a stylesheet the administrator adds at runtime. It
//! reuses the Verdant template set and only replaces the stylesheet, because
//! Askama templates are compiled into the binary. The CSS file lives in
//! `URLET_THEMES_DIR`; its metadata and the catalogue state live in SQLite, so
//! a restart keeps both the upload and its enabled/default position.
//!
//! [`ThemeRegistry`] is a small in-memory cache of the database rows and the
//! upload metadata, reloaded after every write so a request never pays for a
//! query just to choose a stylesheet.

use std::future::Future;
use std::sync::RwLock;

use sqlx::SqlitePool;

use crate::db::custom_themes::{self as custom_db, CustomThemeRow};
use crate::db::themes::{self as theme_db, ThemeRow};
use crate::db::users::User;
use crate::error::{AppError, Result};
use crate::i18n::Lang;
use crate::services::settings::{Settings, ThemeSwitching};
use crate::services::time;

/// Cookie remembering an explicit theme choice.
pub const COOKIE_NAME: &str = "urlet_theme";

/// Administrator-only preview cookie. It may name a disabled theme and is only
/// honoured for a signed-in administrator.
pub const PREVIEW_COOKIE_NAME: &str = "urlet_theme_preview";

/// Preview is intentionally short-lived: an hour is enough to inspect a theme.
pub const PREVIEW_COOKIE_MAX_AGE: i64 = 3_600;

/// Lifetime of the theme cookie: one year.
pub const COOKIE_MAX_AGE: i64 = 31_536_000;

/// Prefix of every uploaded theme key. The rest is random, never a user-supplied
/// path, so a key is safe in a cookie, a URL and a file name.
pub const CUSTOM_KEY_PREFIX: &str = "custom-";

/// A theme compiled into this binary.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum ThemeKey {
    /// The design that shipped with v1.2; also the base template directory.
    #[default]
    Verdant,
    /// Dark glass, fixed sidebar (PR #7).
    Aurora,
    /// Editorial paper ledger (PR #5).
    Paper,
    /// Brutalist signal board (PR #6).
    Signal,
    /// Art Deco gilded circuit (PR #10).
    Gilded,
    /// Swiss telemetry grid (PR #9).
    Swiss,
}

/// Every compiled-in theme, in the order used by the migration and the
/// fallback picker.
pub const ALL: [ThemeKey; 6] = [
    ThemeKey::Verdant,
    ThemeKey::Aurora,
    ThemeKey::Paper,
    ThemeKey::Signal,
    ThemeKey::Gilded,
    ThemeKey::Swiss,
];

impl ThemeKey {
    /// Stable database and cookie key.
    pub const fn key(self) -> &'static str {
        match self {
            Self::Verdant => "verdant",
            Self::Aurora => "aurora",
            Self::Paper => "paper",
            Self::Signal => "signal",
            Self::Gilded => "gilded",
            Self::Swiss => "swiss",
        }
    }

    /// Parse a compiled-in key. Unknown values are rejected rather than
    /// silently mapped to the default, so a stale cookie cannot pin a deleted
    /// theme. Uploaded keys are resolved by [`ThemeRegistry`], not here.
    pub fn parse(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "verdant" => Some(Self::Verdant),
            "aurora" => Some(Self::Aurora),
            "paper" => Some(Self::Paper),
            "signal" => Some(Self::Signal),
            "gilded" => Some(Self::Gilded),
            "swiss" => Some(Self::Swiss),
            _ => None,
        }
    }

    /// Catalogue key of the human readable name.
    pub const fn name_key(self) -> &'static str {
        match self {
            Self::Verdant => "theme.name.verdant",
            Self::Aurora => "theme.name.aurora",
            Self::Paper => "theme.name.paper",
            Self::Signal => "theme.name.signal",
            Self::Gilded => "theme.name.gilded",
            Self::Swiss => "theme.name.swiss",
        }
    }

    /// Catalogue key of the one-line description shown in the admin list.
    pub const fn description_key(self) -> &'static str {
        match self {
            Self::Verdant => "theme.description.verdant",
            Self::Aurora => "theme.description.aurora",
            Self::Paper => "theme.description.paper",
            Self::Signal => "theme.description.signal",
            Self::Gilded => "theme.description.gilded",
            Self::Swiss => "theme.description.swiss",
        }
    }

    /// Stylesheet served for this theme.
    pub const fn css_url(self) -> &'static str {
        match self {
            // The base design keeps the historical asset URL.
            Self::Verdant => "/assets/app.css",
            Self::Aurora => "/assets/themes/aurora",
            Self::Paper => "/assets/themes/paper",
            Self::Signal => "/assets/themes/signal",
            Self::Gilded => "/assets/themes/gilded",
            Self::Swiss => "/assets/themes/swiss",
        }
    }

    /// Aurora is dark-only by design; the other themes follow the operating
    /// system (and the optional light/dark control).
    pub const fn is_dark_only(self) -> bool {
        matches!(self, Self::Aurora)
    }
}

/// A theme the registry can resolve: compiled in or uploaded.
#[derive(Debug, Clone)]
pub struct ThemeDef {
    pub key: String,
    /// Template set used by [`crate::themed_page!`]. Uploaded themes reuse
    /// Verdant because their templates are not part of the binary.
    pub builtin: ThemeKey,
    pub name: String,
    pub description: String,
    pub css_url: String,
    pub is_dark_only: bool,
    pub is_custom: bool,
    pub custom_id: Option<i64>,
}

/// One enabled theme as seen by a page.
#[derive(Debug, Clone)]
pub struct ThemeOptionView {
    pub key: String,
    pub name: String,
    pub current: bool,
}

/// Everything a template needs to render the current theme and the picker.
#[derive(Debug, Clone)]
pub struct ThemeView {
    pub key: String,
    pub name: String,
    pub description: String,
    pub css_url: String,
    pub is_dark_only: bool,
    /// Template set selected by `themed_page!`. Not exposed to templates.
    pub builtin: ThemeKey,
    pub options: Vec<ThemeOptionView>,
    /// Whether the current account may change the theme.
    pub can_switch: bool,
    /// Whether no explicit (or only a stale) cookie selected this look.
    pub using_default: bool,
    /// Whether this view was selected through the administrator preview cookie.
    pub preview: bool,
}

impl ThemeView {
    fn from_parts(
        selected: &ThemeDef,
        enabled: &[ThemeDef],
        can_switch: bool,
        using_default: bool,
    ) -> Self {
        let mut options: Vec<ThemeOptionView> = enabled
            .iter()
            .map(|theme| ThemeOptionView {
                key: theme.key.clone(),
                name: theme.name.clone(),
                current: theme.key == selected.key,
            })
            .collect();
        // A stale cookie can point at a theme that has just been disabled, and
        // an administrator can preview a theme that is not enabled. Keep the
        // current selection in the picker either way.
        if !options.iter().any(|option| option.key == selected.key) {
            options.push(ThemeOptionView {
                key: selected.key.clone(),
                name: selected.name.clone(),
                current: true,
            });
        }
        Self {
            key: selected.key.clone(),
            name: selected.name.clone(),
            description: selected.description.clone(),
            css_url: selected.css_url.clone(),
            is_dark_only: selected.is_dark_only,
            builtin: selected.builtin,
            options,
            can_switch,
            using_default,
            preview: false,
        }
    }

    /// Set the per-account switch permission after the task-local view was
    /// built without a user.
    pub fn with_can_switch(mut self, can_switch: bool) -> Self {
        self.can_switch = can_switch;
        self
    }

    /// Mark the view as the site default rather than an explicit cookie.
    pub fn with_using_default(mut self, using_default: bool) -> Self {
        self.using_default = using_default;
        self
    }

    /// Mark the view as an administrator preview.
    pub fn with_preview(mut self, preview: bool) -> Self {
        self.preview = preview;
        self
    }
}

/// Build the definition of a compiled-in theme in `lang`.
pub fn builtin_def(key: ThemeKey, lang: Lang) -> ThemeDef {
    ThemeDef {
        key: key.key().to_string(),
        builtin: key,
        name: lang.t(key.name_key()).to_string(),
        description: lang.t(key.description_key()).to_string(),
        css_url: key.css_url().to_string(),
        is_dark_only: key.is_dark_only(),
        is_custom: false,
        custom_id: None,
    }
}

/// Build the definition of an uploaded theme row.
pub fn custom_def(row: &CustomThemeRow) -> ThemeDef {
    ThemeDef {
        key: row.key.clone(),
        // Uploaded themes re-use the base template directory.
        builtin: ThemeKey::Verdant,
        name: row.name.clone(),
        description: row.description.clone(),
        css_url: custom_css_url(&row.key, &row.css_version),
        is_dark_only: row.is_dark_only,
        is_custom: true,
        custom_id: Some(row.id),
    }
}

/// Public URL of an uploaded stylesheet.
///
/// The content hash makes a replaced upload a new URL, so the one-hour asset
/// cache cannot keep serving the previous stylesheet.
pub fn custom_css_url(key: &str, version: &str) -> String {
    format!("/assets/custom-themes/{key}?v={version}")
}

/// The theme a request should render. Outside a request (unit tests, startup)
/// this is the Verdant fallback, so rendering never fails because middleware was
/// skipped.
pub fn fallback_view() -> ThemeView {
    let enabled: Vec<ThemeDef> = ALL.iter().map(|key| builtin_def(*key, Lang::En)).collect();
    ThemeView::from_parts(&enabled[0], &enabled, false, true)
}

tokio::task_local! {
    static CURRENT: ThemeView;
}

/// The theme of the request being handled.
pub fn current() -> ThemeView {
    CURRENT
        .try_with(Clone::clone)
        .unwrap_or_else(|_| fallback_view())
}

/// The compiled-in template set selected for the request, used by
/// [`crate::themed_page!`] to pick a template. An uploaded theme maps to
/// Verdant.
pub fn selected_key() -> ThemeKey {
    CURRENT.try_with(|view| view.builtin).unwrap_or_default()
}

/// Stylesheet URL of the request theme without cloning the whole view.
pub fn current_css_url() -> String {
    CURRENT
        .try_with(|view| view.css_url.clone())
        .unwrap_or_else(|_| ThemeKey::Verdant.css_url().to_string())
}

/// Catalogue key of the request theme without cloning the whole view.
pub fn current_key() -> String {
    CURRENT
        .try_with(|view| view.key.clone())
        .unwrap_or_else(|_| ThemeKey::Verdant.key().to_string())
}

/// Whether the request theme is dark-only, without cloning the whole view.
pub fn current_is_dark_only() -> bool {
    CURRENT.try_with(|view| view.is_dark_only).unwrap_or(false)
}

/// Run `fut` with `view` installed as the task-local theme.
pub async fn scope<F: Future>(view: ThemeView, fut: F) -> F::Output {
    CURRENT.scope(view, fut).await
}

/// Whether `user` may change their own theme.
///
/// Administrators are never restricted: they need to preview a theme before
/// making it the default. Everyone else is subject to the site-wide switch.
pub fn can_switch(settings: &Settings, user: &User) -> bool {
    user.is_admin() || settings.theme_switching() == ThemeSwitching::All
}

/// Build the view for a signed-in account.
pub fn for_user(settings: &Settings, user: &User) -> ThemeView {
    current().with_can_switch(can_switch(settings, user))
}

/// Build the view for an anonymous visitor: the cookie still selects a look,
/// but the picker is hidden because there is no account to check.
pub fn for_anonymous() -> ThemeView {
    current().with_can_switch(false)
}

/// In-memory cache of the catalogue rows and the uploaded theme metadata.
#[derive(Debug)]
pub struct ThemeRegistry {
    rows: RwLock<Vec<ThemeRow>>,
    custom: RwLock<Vec<CustomThemeRow>>,
}

impl Default for ThemeRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl ThemeRegistry {
    /// An empty registry used while the process is bootstrapping and by tests.
    pub fn new() -> Self {
        Self {
            rows: RwLock::new(Vec::new()),
            custom: RwLock::new(Vec::new()),
        }
    }

    /// Load the current rows into the cache.
    pub async fn load(pool: &SqlitePool) -> Result<Self> {
        let registry = Self::new();
        registry.reload(pool).await?;
        Ok(registry)
    }

    /// Refresh the cache after an administrator write.
    pub async fn reload(&self, pool: &SqlitePool) -> Result<()> {
        let rows = theme_db::all(pool)
            .await
            .map_err(|err| AppError::internal(format!("cannot load themes: {err}")))?;
        let custom = custom_db::all(pool)
            .await
            .map_err(|err| AppError::internal(format!("cannot load custom themes: {err}")))?;
        if let Ok(mut guard) = self.rows.write() {
            *guard = rows;
        }
        if let Ok(mut guard) = self.custom.write() {
            *guard = custom;
        }
        Ok(())
    }

    /// Apply a committed enabled/default selection to the in-memory rows.
    ///
    /// The database transaction is the source of truth; this method only mirrors
    /// the values that transaction just committed. It does not touch the
    /// database, so it cannot fail after the commit and leave the caller with a
    /// stale cache and a successful audit.
    pub fn apply_state(&self, enabled: &[String], default_key: &str) {
        if let Ok(mut rows) = self.rows.write() {
            for row in rows.iter_mut() {
                row.is_enabled = enabled.iter().any(|key| key == &row.key);
                row.is_default = row.key == default_key;
            }
        }
    }

    /// Snapshot of every catalogue row, including disabled themes. Used by the
    /// admin page.
    pub fn rows(&self) -> Vec<ThemeRow> {
        self.rows
            .read()
            .map(|rows| rows.clone())
            .unwrap_or_default()
    }

    /// Snapshot of every uploaded theme.
    pub fn custom_rows(&self) -> Vec<CustomThemeRow> {
        self.custom
            .read()
            .map(|custom| custom.clone())
            .unwrap_or_default()
    }

    /// Enabled themes in display order, with names and stylesheet URLs.
    pub fn enabled_defs(&self, lang: Lang) -> Vec<ThemeDef> {
        let rows = self.rows();
        let custom = self.custom_rows();
        let mut out: Vec<ThemeDef> = Vec::new();
        for row in rows.iter().filter(|row| row.is_enabled) {
            if let Some(key) = ThemeKey::parse(&row.key) {
                if !out.iter().any(|theme| theme.key == key.key()) {
                    out.push(builtin_def(key, lang));
                }
            } else if let Some(upload) = custom.iter().find(|upload| upload.key == row.key) {
                out.push(custom_def(upload));
            }
        }
        if out.is_empty() {
            out.push(builtin_def(ThemeKey::Verdant, lang));
        }
        out
    }

    /// Enabled keys in display order.
    pub fn enabled_keys(&self) -> Vec<String> {
        self.enabled_defs(Lang::En)
            .into_iter()
            .map(|theme| theme.key)
            .collect()
    }

    /// The definition of a known theme, whether enabled or not.
    pub fn known(&self, raw: &str, lang: Lang) -> Option<ThemeDef> {
        if let Some(key) = ThemeKey::parse(raw) {
            return Some(builtin_def(key, lang));
        }
        let key = raw.trim();
        self.custom_rows()
            .iter()
            .find(|row| row.key == key)
            .map(custom_def)
    }

    /// The configured default when it is enabled, otherwise the first enabled
    /// theme and finally Verdant.
    pub fn default_def(&self, lang: Lang) -> ThemeDef {
        let rows = self.rows();
        let custom = self.custom_rows();
        if let Some(row) = rows.iter().find(|row| row.is_default && row.is_enabled) {
            if let Some(key) = ThemeKey::parse(&row.key) {
                return builtin_def(key, lang);
            }
            if let Some(upload) = custom.iter().find(|upload| upload.key == row.key) {
                return custom_def(upload);
            }
        }
        self.enabled_defs(lang)
            .into_iter()
            .next()
            .unwrap_or_else(|| builtin_def(ThemeKey::Verdant, lang))
    }

    /// The key of the default theme.
    pub fn default_key(&self) -> String {
        self.default_def(Lang::En).key
    }

    /// Resolve a requested key (usually from the cookie) and report whether it
    /// was the implicit default. A disabled or unknown theme falls back to the
    /// configured default.
    pub fn resolve(&self, requested: Option<&str>, lang: Lang) -> (ThemeDef, bool) {
        if let Some(requested) = requested.map(str::trim).filter(|value| !value.is_empty()) {
            if let Some(theme) = self.known(requested, lang) {
                if self.is_enabled(&theme.key) {
                    return (theme, false);
                }
            }
        }
        (self.default_def(lang), true)
    }

    /// True when the theme is present and enabled.
    pub fn is_enabled(&self, raw: &str) -> bool {
        let key = ThemeKey::parse(raw)
            .map(|key| key.key().to_string())
            .unwrap_or_else(|| raw.trim().to_string());
        self.rows()
            .iter()
            .any(|row| row.key == key && row.is_enabled)
    }

    /// Build the request view for a cookie value.
    pub fn view(&self, lang: Lang, requested: Option<&str>) -> ThemeView {
        let (selected, using_default) = self.resolve(requested, lang);
        ThemeView::from_parts(&selected, &self.enabled_defs(lang), false, using_default)
    }

    /// Build a preview view for an administrator. The key may be disabled.
    pub fn preview_view(&self, lang: Lang, raw: &str) -> Option<ThemeView> {
        let selected = self.known(raw, lang)?;
        Some(
            ThemeView::from_parts(&selected, &self.enabled_defs(lang), true, false)
                .with_preview(true),
        )
    }
}

/// One row of the administrator's theme table.
#[derive(Debug, Clone)]
pub struct AdminThemeRow {
    pub key: String,
    pub name: String,
    pub description: String,
    pub is_enabled: bool,
    pub is_default: bool,
    pub is_dark_only: bool,
    pub css_url: String,
    pub is_custom: bool,
    pub custom_id: Option<i64>,
    /// Human readable size of an uploaded stylesheet; empty for compiled-in
    /// themes.
    pub size: String,
    /// Human readable upload time; empty for compiled-in themes.
    pub created: String,
    /// Edit/delete endpoints of an upload; empty for compiled-in themes.
    pub edit_url: String,
    pub delete_url: String,
}

/// Build the admin list: every compiled-in theme plus every upload. Rows that
/// the registry does not know about (a stale database row for a theme removed
/// from the binary) are omitted; the selection update disables them.
pub fn admin_rows(registry: &ThemeRegistry, lang: Lang) -> Vec<AdminThemeRow> {
    let rows = registry.rows();
    let custom = registry.custom_rows();
    let mut out: Vec<AdminThemeRow> = ALL
        .iter()
        .map(|key| {
            let row = rows.iter().find(|row| row.key == key.key());
            AdminThemeRow {
                key: key.key().to_string(),
                name: lang.t(key.name_key()).to_string(),
                description: lang.t(key.description_key()).to_string(),
                is_enabled: row.map(|row| row.is_enabled).unwrap_or(true),
                is_default: row.map(|row| row.is_default).unwrap_or(false),
                is_dark_only: key.is_dark_only(),
                css_url: key.css_url().to_string(),
                is_custom: false,
                custom_id: None,
                size: String::new(),
                created: String::new(),
                edit_url: String::new(),
                delete_url: String::new(),
            }
        })
        .collect();
    for upload in &custom {
        let row = rows.iter().find(|row| row.key == upload.key);
        out.push(AdminThemeRow {
            key: upload.key.clone(),
            name: upload.name.clone(),
            description: upload.description.clone(),
            is_enabled: row.map(|row| row.is_enabled).unwrap_or(false),
            is_default: row.map(|row| row.is_default).unwrap_or(false),
            is_dark_only: upload.is_dark_only,
            css_url: custom_css_url(&upload.key, &upload.css_version),
            is_custom: true,
            custom_id: Some(upload.id),
            size: human_size(upload.css_size),
            created: time::humanize_lang(lang, &upload.created_at),
            edit_url: format!("/app/admin/themes/custom/{}", upload.id),
            delete_url: format!("/app/admin/themes/custom/{}/delete", upload.id),
        });
    }
    out.sort_by_key(|row| {
        rows.iter()
            .find(|entry| entry.key == row.key)
            .map(|entry| entry.sort_order)
            .or_else(|| {
                custom
                    .iter()
                    .find(|entry| entry.key == row.key)
                    .map(|entry| 1_000 + entry.id)
            })
            .unwrap_or(i64::MAX)
    });
    out
}

fn human_size(bytes: i64) -> String {
    let bytes = bytes.max(0) as f64;
    if bytes >= 1024.0 * 1024.0 {
        format!("{:.1} MB", bytes / (1024.0 * 1024.0))
    } else if bytes >= 1024.0 {
        format!("{:.1} KB", bytes / 1024.0)
    } else {
        format!("{bytes:.0} B")
    }
}

/// The values actually written by [`update_selection`].
#[derive(Debug, Clone)]
pub struct AppliedSelection {
    pub enabled: Vec<String>,
    pub default_key: String,
    pub switching: ThemeSwitching,
}

/// Validate and apply an administrator selection.
///
/// The invariant is the same one the partial index protects: at least one
/// enabled theme, and the default must be enabled. Omitted fields are resolved
/// to the current values **after** taking the settings write lock, so a
/// concurrent catalogue update cannot be silently reverted by a request that
/// only changes the policy. The catalogue and the `theme_switching` policy are
/// committed in one database transaction, then the two in-memory caches are
/// updated from the values that transaction wrote.
pub async fn update_selection(
    pool: &SqlitePool,
    registry: &ThemeRegistry,
    settings: &Settings,
    enabled: Option<&[String]>,
    default_key: Option<&str>,
    switching: Option<ThemeSwitching>,
    actor: Option<i64>,
) -> Result<AppliedSelection> {
    // Serialise with settings writes: this guard covers the read of the
    // omitted fields, the transaction and the cache update. Uploads take the
    // same lock, so a theme cannot disappear between validation and commit.
    let _settings_guard = settings.lock_updates().await;

    let enabled = match enabled {
        Some(values) => values.to_vec(),
        None => registry
            .rows()
            .into_iter()
            .filter(|row| row.is_enabled)
            // A stale row for a theme that is no longer compiled in must not
            // survive a partial update; only known themes are carried over.
            .filter(|row| registry.known(&row.key, Lang::En).is_some())
            .map(|row| row.key)
            .collect(),
    };
    let requested_default = default_key
        .map(str::to_string)
        .unwrap_or_else(|| registry.default_key());
    let switching = switching.unwrap_or_else(|| settings.theme_switching());

    let default_theme = registry
        .known(&requested_default, Lang::En)
        .ok_or_else(|| AppError::validation("default_theme", "unknown theme"))?;

    let mut keys: Vec<String> = Vec::new();
    for raw in &enabled {
        let Some(theme) = registry.known(raw, Lang::En) else {
            return Err(AppError::validation("enabled_themes", "unknown theme"));
        };
        if !keys.contains(&theme.key) {
            keys.push(theme.key);
        }
    }
    if keys.is_empty() {
        return Err(AppError::validation(
            "enabled_themes",
            "at least one theme must stay enabled",
        ));
    }
    if !keys.contains(&default_theme.key) {
        return Err(AppError::validation(
            "default_theme",
            "the default theme must be enabled",
        ));
    }

    theme_db::set_state_and_setting(pool, &keys, &default_theme.key, switching.as_str(), actor)
        .await
        .map_err(|err| AppError::internal(format!("cannot update themes: {err}")))?;

    // Only after the commit: mirror the committed values into the caches. No
    // second database read (or settings validation) can fail at this point.
    registry.apply_state(&keys, &default_theme.key);
    settings.set_cached("theme_switching", switching.as_str());
    Ok(AppliedSelection {
        enabled: keys,
        default_key: default_theme.key,
        switching,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_round_trip_and_unknown_values_are_rejected() {
        for key in ALL {
            assert_eq!(ThemeKey::parse(key.key()), Some(key));
        }
        assert_eq!(ThemeKey::parse("AURORA"), Some(ThemeKey::Aurora));
        assert_eq!(ThemeKey::parse("nope"), None);
    }

    #[test]
    fn a_view_keeps_the_selected_theme_and_marks_it_current() {
        let selected = builtin_def(ThemeKey::Paper, Lang::En);
        let enabled = vec![
            builtin_def(ThemeKey::Verdant, Lang::En),
            builtin_def(ThemeKey::Paper, Lang::En),
        ];
        let view = ThemeView::from_parts(&selected, &enabled, true, false);
        assert_eq!(view.key, "paper");
        assert_eq!(view.builtin, ThemeKey::Paper);
        assert_eq!(view.css_url, "/assets/themes/paper");
        assert!(view.can_switch);
        assert_eq!(view.options.len(), 2);
        assert!(view
            .options
            .iter()
            .any(|option| option.current && option.key == "paper"));
    }

    #[test]
    fn an_uploaded_theme_uses_the_base_templates_and_a_hashed_url() {
        let row = CustomThemeRow {
            id: 7,
            key: "custom-abc".into(),
            name: "Mine".into(),
            description: "A test upload".into(),
            is_dark_only: true,
            css_version: "deadbeef".into(),
            css_size: 42,
            original_filename: "mine.css".into(),
            created_by: None,
            created_at: "2026-01-01T00:00:00.000Z".into(),
            updated_at: "2026-01-01T00:00:00.000Z".into(),
        };
        let def = custom_def(&row);
        assert_eq!(def.builtin, ThemeKey::Verdant);
        assert_eq!(def.css_url, "/assets/custom-themes/custom-abc?v=deadbeef");
        assert!(def.is_dark_only);
        assert!(def.is_custom);
        assert_eq!(def.custom_id, Some(7));
    }

    #[test]
    fn a_disabled_theme_is_not_offered() {
        // The registry only sees what the database exposes; `enabled_defs`
        // falls back to Verdant when every row is disabled.
        let registry = ThemeRegistry::new();
        assert_eq!(registry.enabled_keys(), vec!["verdant"]);
        assert_eq!(registry.default_key(), "verdant");
        assert_eq!(registry.resolve(Some("aurora"), Lang::En).0.key, "verdant");
    }
}
