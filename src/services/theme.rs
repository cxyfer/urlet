//! Compiled-in themes and the administrator-controlled selection.
//!
//! A *theme* is a complete visual identity: a CSS bundle plus a directory of
//! Askama overrides under `templates/themes/<key>/`. The base templates in
//! `templates/` are themselves the Verdant theme, and they double as the
//! fallback for any page a theme does not override.
//!
//! The database only stores the administrator's choices (enabled, default and
//! order); the list of themes is fixed at compile time. [`ThemeRegistry`] is a
//! small in-memory cache of those choices, reloaded after every write so a
//! request never pays for a query just to choose a stylesheet.

use std::future::Future;
use std::sync::RwLock;

use sqlx::SqlitePool;

use crate::db::themes::{self as theme_db, ThemeRow};
use crate::db::users::User;
use crate::error::{AppError, Result};
use crate::i18n::Lang;
use crate::services::settings::{Settings, ThemeSwitching};

/// Cookie remembering an explicit theme choice.
pub const COOKIE_NAME: &str = "urlet_theme";

/// Administrator-only preview cookie. It may name a disabled theme and is only
/// honoured for a signed-in administrator.
pub const PREVIEW_COOKIE_NAME: &str = "urlet_theme_preview";

/// Preview is intentionally short-lived: an hour is enough to inspect a theme.
pub const PREVIEW_COOKIE_MAX_AGE: i64 = 3_600;

/// Lifetime of the theme cookie: one year.
pub const COOKIE_MAX_AGE: i64 = 31_536_000;

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

    /// Parse a database/cookie key. Unknown values are rejected rather than
    /// silently mapped to the default, so a stale cookie cannot pin a deleted
    /// theme.
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

/// One enabled theme as seen by a page.
#[derive(Debug, Clone)]
pub struct ThemeOptionView {
    pub key: &'static str,
    pub name: String,
    pub current: bool,
}

/// Everything a template needs to render the current theme and the picker.
#[derive(Debug, Clone)]
pub struct ThemeView {
    pub key: ThemeKey,
    pub key_str: &'static str,
    pub name: String,
    pub description: String,
    pub css_url: &'static str,
    pub is_dark_only: bool,
    pub options: Vec<ThemeOptionView>,
    /// Whether the current account may change the theme.
    pub can_switch: bool,
    /// Whether no explicit (or only a stale) cookie selected this look.
    pub using_default: bool,
    /// Whether this view was selected through the administrator preview cookie.
    pub preview: bool,
}

impl ThemeView {
    fn from_parts(lang: Lang, selected: ThemeKey, enabled: &[ThemeKey], can_switch: bool) -> Self {
        let mut options: Vec<ThemeOptionView> = enabled
            .iter()
            .map(|key| ThemeOptionView {
                key: key.key(),
                name: lang.t(key.name_key()).to_string(),
                current: *key == selected,
            })
            .collect();
        // A stale cookie can point at a theme that has just been disabled, and
        // an administrator can preview a theme that is not enabled. Keep the
        // current selection in the picker either way.
        if !options.iter().any(|option| option.key == selected.key()) {
            options.push(ThemeOptionView {
                key: selected.key(),
                name: lang.t(selected.name_key()).to_string(),
                current: true,
            });
        }
        Self {
            key: selected,
            key_str: selected.key(),
            name: lang.t(selected.name_key()).to_string(),
            description: lang.t(selected.description_key()).to_string(),
            css_url: selected.css_url(),
            is_dark_only: selected.is_dark_only(),
            options,
            can_switch,
            using_default: false,
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

/// The theme a request should render. Outside a request (unit tests, startup)
/// this is the Verdant fallback, so rendering never fails because middleware was
/// skipped.
pub fn fallback_view() -> ThemeView {
    ThemeView::from_parts(Lang::En, ThemeKey::Verdant, &ALL, false)
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

/// The selected theme key, used by [`crate::themed_page!`] to pick a template.
pub fn selected_key() -> ThemeKey {
    current().key
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

/// In-memory cache of the administrator's theme choices.
#[derive(Debug)]
pub struct ThemeRegistry {
    rows: RwLock<Vec<ThemeRow>>,
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
        if let Ok(mut guard) = self.rows.write() {
            *guard = rows;
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

    /// Snapshot of every row, including disabled themes. Used by the admin page.
    pub fn rows(&self) -> Vec<ThemeRow> {
        self.rows
            .read()
            .map(|rows| rows.clone())
            .unwrap_or_default()
    }

    /// Enabled keys in display order.
    pub fn enabled_keys(&self) -> Vec<ThemeKey> {
        let mut rows: Vec<ThemeRow> = self
            .rows
            .read()
            .map(|rows| rows.iter().filter(|row| row.is_enabled).cloned().collect())
            .unwrap_or_default();
        rows.sort_by(|a, b| a.sort_order.cmp(&b.sort_order).then(a.key.cmp(&b.key)));
        let keys: Vec<ThemeKey> = rows
            .iter()
            .filter_map(|row| ThemeKey::parse(&row.key))
            .collect();
        if keys.is_empty() {
            vec![ThemeKey::Verdant]
        } else {
            keys
        }
    }

    /// The configured default when it is enabled, otherwise the first enabled
    /// theme and finally Verdant.
    pub fn default_key(&self) -> ThemeKey {
        let rows = self.rows();
        rows.iter()
            .find(|row| row.is_default && row.is_enabled)
            .and_then(|row| ThemeKey::parse(&row.key))
            .or_else(|| self.enabled_keys().first().copied())
            .unwrap_or(ThemeKey::Verdant)
    }

    /// Resolve a requested key (usually from the cookie). A disabled or unknown
    /// theme falls back to the configured default.
    pub fn resolve(&self, requested: Option<&str>) -> ThemeKey {
        let Some(requested) = requested.and_then(ThemeKey::parse) else {
            return self.default_key();
        };
        if self.is_enabled(requested) {
            requested
        } else {
            self.default_key()
        }
    }

    /// True when the theme is present and enabled.
    pub fn is_enabled(&self, key: ThemeKey) -> bool {
        self.rows()
            .iter()
            .any(|row| row.key == key.key() && row.is_enabled)
    }

    /// Build the request view for a cookie value.
    pub fn view(&self, lang: Lang, requested: Option<&str>) -> ThemeView {
        let requested_key = requested.and_then(ThemeKey::parse);
        let selected = self.resolve(requested);
        let using_default = match requested_key {
            Some(key) => !self.is_enabled(key) || key != selected,
            None => true,
        };
        ThemeView::from_parts(lang, selected, &self.enabled_keys(), false)
            .with_using_default(using_default)
    }

    /// Build a preview view for an administrator. The key may be disabled.
    pub fn preview_view(&self, lang: Lang, key: ThemeKey) -> ThemeView {
        ThemeView::from_parts(lang, key, &self.enabled_keys(), true).with_preview(true)
    }
}

/// One row of the administrator's theme table.
#[derive(Debug, Clone)]
pub struct AdminThemeRow {
    pub key: &'static str,
    pub name: String,
    pub description: String,
    pub is_enabled: bool,
    pub is_default: bool,
    pub is_dark_only: bool,
    pub css_url: &'static str,
}

/// Build the admin list, including themes the registry does not know about yet
/// (a stale database row for a theme removed from the binary).
pub fn admin_rows(registry: &ThemeRegistry, lang: Lang) -> Vec<AdminThemeRow> {
    let rows = registry.rows();
    let mut out: Vec<AdminThemeRow> = ALL
        .iter()
        .map(|key| {
            let row = rows.iter().find(|row| row.key == key.key());
            AdminThemeRow {
                key: key.key(),
                name: lang.t(key.name_key()).to_string(),
                description: lang.t(key.description_key()).to_string(),
                is_enabled: row.map(|row| row.is_enabled).unwrap_or(true),
                is_default: row.map(|row| row.is_default).unwrap_or(false),
                is_dark_only: key.is_dark_only(),
                css_url: key.css_url(),
            }
        })
        .collect();
    out.sort_by_key(|row| {
        rows.iter()
            .find(|entry| entry.key == row.key)
            .map(|entry| entry.sort_order)
            .unwrap_or(i64::MAX)
    });
    out
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
    // omitted fields, the transaction and the cache update.
    let _settings_guard = settings.lock_updates().await;

    let enabled = match enabled {
        Some(values) => values.to_vec(),
        None => registry
            .rows()
            .into_iter()
            .filter(|row| row.is_enabled)
            .filter_map(|row| ThemeKey::parse(&row.key).map(|key| key.key().to_string()))
            .collect(),
    };
    let requested_default = default_key
        .map(str::to_string)
        .unwrap_or_else(|| registry.default_key().key().to_string());
    let switching = switching.unwrap_or_else(|| settings.theme_switching());

    let default_key = ThemeKey::parse(&requested_default)
        .ok_or_else(|| AppError::validation("default_theme", "unknown theme"))?;

    let mut keys: Vec<ThemeKey> = Vec::new();
    for raw in &enabled {
        let Some(key) = ThemeKey::parse(raw) else {
            return Err(AppError::validation("enabled_themes", "unknown theme"));
        };
        if !keys.contains(&key) {
            keys.push(key);
        }
    }
    if keys.is_empty() {
        return Err(AppError::validation(
            "enabled_themes",
            "at least one theme must stay enabled",
        ));
    }
    if !keys.contains(&default_key) {
        return Err(AppError::validation(
            "default_theme",
            "the default theme must be enabled",
        ));
    }

    let keys: Vec<String> = keys.iter().map(|key| key.key().to_string()).collect();
    theme_db::set_state_and_setting(pool, &keys, default_key.key(), switching.as_str(), actor)
        .await
        .map_err(|err| AppError::internal(format!("cannot update themes: {err}")))?;

    // Only after the commit: mirror the committed values into the caches. No
    // second database read (or settings validation) can fail at this point.
    registry.apply_state(&keys, default_key.key());
    settings.set_cached("theme_switching", switching.as_str());
    Ok(AppliedSelection {
        enabled: keys,
        default_key: default_key.key().to_string(),
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
        let view = ThemeView::from_parts(
            Lang::En,
            ThemeKey::Paper,
            &[ThemeKey::Verdant, ThemeKey::Paper],
            true,
        );
        assert_eq!(view.key, ThemeKey::Paper);
        assert_eq!(view.css_url, "/assets/themes/paper");
        assert!(view.can_switch);
        assert_eq!(view.options.len(), 2);
        assert!(view
            .options
            .iter()
            .any(|option| option.current && option.key == "paper"));
    }

    #[test]
    fn a_disabled_theme_is_not_offered() {
        // The registry only sees what the database exposes; `enabled_keys`
        // falls back to Verdant when every row is disabled.
        let registry = ThemeRegistry::new();
        assert_eq!(registry.enabled_keys(), vec![ThemeKey::Verdant]);
        assert_eq!(registry.default_key(), ThemeKey::Verdant);
        assert_eq!(registry.resolve(Some("aurora")), ThemeKey::Verdant);
    }
}
