//! Uploaded theme stylesheets.
//!
//! An upload is deliberately *only* a stylesheet: the page templates are
//! compiled into the binary, so a runtime upload cannot add or override one.
//! The file is stored on the persistent volume under `URLET_THEMES_DIR`, named
//! `<key>-<content hash>.css`, while the catalogue metadata lives in SQLite.
//! The content hash is part of the public URL, so replacing a stylesheet
//! invalidates the browser cache without a restart.
//!
//! Uploads happen under the same settings lock as the catalogue update: a
//! theme cannot be deleted between validating a selection and committing it.

use std::path::{Path, PathBuf};

use sqlx::SqlitePool;

use crate::db::custom_themes::{self as custom_db, CustomThemeRow, NewCustomTheme};
use crate::db::themes as theme_db;
use crate::error::{AppError, Result};
use crate::services::secret;
use crate::services::settings::Settings;
use crate::services::theme::{ThemeAppearance, ThemeRegistry, CUSTOM_KEY_PREFIX};

/// Largest accepted stylesheet.
pub const MAX_CSS_BYTES: usize = 256 * 1024;

/// Room for the multipart envelope around the largest stylesheet.
pub const MAX_UPLOAD_BYTES: usize = MAX_CSS_BYTES + 64 * 1024;

/// Longest display name.
pub const MAX_NAME_LEN: usize = 60;

/// Longest description.
pub const MAX_DESCRIPTION_LEN: usize = 200;

/// How many uploads the instance keeps. Small on purpose: each one is served
/// to every visitor and lives in the admin catalogue.
pub const MAX_THEMES: i64 = 32;

/// Longest displayed original file name.
const MAX_FILENAME_LEN: usize = 120;

/// Validate an uploaded display name.
pub fn validate_name(raw: &str) -> Result<String> {
    let name = raw.trim();
    if name.is_empty() || name.chars().count() > MAX_NAME_LEN {
        return Err(AppError::validation(
            "name",
            format!("the name must contain 1 to {MAX_NAME_LEN} characters"),
        ));
    }
    if name.chars().any(char::is_control) {
        return Err(AppError::validation(
            "name",
            "the name contains control characters",
        ));
    }
    Ok(name.to_string())
}

/// Validate an uploaded description. Empty is allowed.
pub fn validate_description(raw: &str) -> Result<String> {
    let description = raw.trim();
    if description.chars().count() > MAX_DESCRIPTION_LEN {
        return Err(AppError::validation(
            "description",
            format!("the description must stay under {MAX_DESCRIPTION_LEN} characters"),
        ));
    }
    if description.chars().any(char::is_control) {
        return Err(AppError::validation(
            "description",
            "the description contains control characters",
        ));
    }
    Ok(description.to_string())
}

/// Validate a stylesheet and return it as text.
///
/// The checker rejects constructs that would either escape the stylesheet's
/// role (IE `expression`, legacy `behavior`) or make the page fetch a third
/// party resource. The CSP is the authoritative boundary; this is
/// defence-in-depth plus an early, readable error for the administrator.
pub fn validate_css(bytes: &[u8]) -> Result<String> {
    if bytes.is_empty() {
        return Err(AppError::validation("css", "choose a non-empty CSS file"));
    }
    if bytes.len() > MAX_CSS_BYTES {
        return Err(AppError::validation(
            "css",
            format!(
                "the stylesheet must not exceed {} KiB",
                MAX_CSS_BYTES / 1024
            ),
        ));
    }
    let css = std::str::from_utf8(bytes)
        .map_err(|_| AppError::validation("css", "the stylesheet must be valid UTF-8"))?;
    if css
        .chars()
        .any(|c| c.is_control() && !matches!(c, '\n' | '\r' | '\t'))
    {
        return Err(AppError::validation(
            "css",
            "the stylesheet contains control characters",
        ));
    }
    let trimmed = css.trim();
    if trimmed.is_empty() {
        return Err(AppError::validation("css", "choose a non-empty CSS file"));
    }
    if !trimmed.contains('{') || !trimmed.contains('}') {
        return Err(AppError::validation(
            "css",
            "the file does not look like a stylesheet",
        ));
    }
    let lower = trimmed.to_ascii_lowercase();
    for forbidden in [
        "expression(",
        "javascript:",
        "vbscript:",
        "-moz-binding",
        "behavior:",
        "@import",
    ] {
        if lower.contains(forbidden) {
            return Err(AppError::validation(
                "css",
                format!("the stylesheet may not use {forbidden}"),
            ));
        }
    }
    if has_external_url(&lower) {
        return Err(AppError::validation(
            "css",
            "the stylesheet may not load external resources",
        ));
    }
    Ok(trimmed.to_string())
}

/// True when a `url(...)` token points at another origin.
fn has_external_url(lower: &str) -> bool {
    let mut rest = lower;
    while let Some(position) = rest.find("url(") {
        let after = &rest[position + 4..];
        let target = after
            .trim_start()
            .trim_start_matches(['\'', '"'])
            .trim_start();
        for prefix in [
            "http://",
            "https://",
            "//",
            "ftp://",
            "data:text/html",
            "data:image/svg",
        ] {
            if target.starts_with(prefix) {
                return true;
            }
        }
        rest = after;
    }
    false
}

/// Clean the browser supplied file name for display only. The stored path is
/// always derived from the generated key, never from this value.
pub fn display_filename(raw: Option<&str>) -> String {
    let Some(raw) = raw else {
        return "theme.css".to_string();
    };
    let base = raw.rsplit(['/', '\\']).next().unwrap_or("theme.css").trim();
    let cleaned: String = base.chars().filter(|c| !c.is_control()).collect();
    if cleaned.is_empty() {
        return "theme.css".to_string();
    }
    let mut out = String::new();
    for character in cleaned.chars() {
        if out.chars().count() >= MAX_FILENAME_LEN {
            break;
        }
        out.push(character);
    }
    out
}

/// Validate a generated catalogue key before it is used in a path.
pub fn validate_key(key: &str) -> bool {
    let Some(rest) = key.strip_prefix(CUSTOM_KEY_PREFIX) else {
        return false;
    };
    !rest.is_empty()
        && rest.len() <= 64
        && rest
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
}

/// Content hash used as the version in the public URL.
pub fn version_of(css: &str) -> String {
    secret::sha256_hex(css)[..12].to_string()
}

/// Absolute path of the stylesheet for a key/version pair.
pub fn css_path(dir: &Path, key: &str, version: &str) -> PathBuf {
    dir.join(format!("{key}-{version}.css"))
}

/// Read the stylesheet of a catalogue row.
pub async fn read_css(dir: &Path, row: &CustomThemeRow) -> std::io::Result<Vec<u8>> {
    let version_ok = !row.css_version.is_empty()
        && row.css_version.len() <= 64
        && row.css_version.bytes().all(|byte| byte.is_ascii_hexdigit());
    if !validate_key(&row.key) || !version_ok {
        return Err(std::io::Error::new(
            std::io::ErrorKind::PermissionDenied,
            "invalid custom theme key or version",
        ));
    }
    tokio::fs::read(css_path(dir, &row.key, &row.css_version)).await
}

/// Register a validated stylesheet and reload the catalogue cache.
#[allow(clippy::too_many_arguments)]
pub async fn create(
    pool: &SqlitePool,
    registry: &ThemeRegistry,
    settings: &Settings,
    themes_dir: &Path,
    name: &str,
    description: &str,
    appearance: ThemeAppearance,
    css: &str,
    filename: &str,
    actor: Option<i64>,
) -> Result<CustomThemeRow> {
    let name = validate_name(name)?;
    let description = validate_description(description)?;
    let _guard = settings.lock_updates().await;

    let count = custom_db::count(pool)
        .await
        .map_err(|err| AppError::internal(format!("cannot count custom themes: {err}")))?;
    if count >= MAX_THEMES {
        return Err(AppError::validation(
            "themes",
            format!("at most {MAX_THEMES} custom themes can be stored"),
        ));
    }

    // A random key keeps the URL, cookie and file name independent from the
    // administrator supplied name, and makes path traversal impossible.
    let key = loop {
        let candidate = format!("{CUSTOM_KEY_PREFIX}{}", secret::random_token(9));
        let exists = custom_db::find_by_key(pool, &candidate)
            .await
            .map_err(|err| AppError::internal(format!("cannot check theme key: {err}")))?;
        if exists.is_none() && validate_key(&candidate) {
            break candidate;
        }
    };

    let version = version_of(css);
    let path = css_path(themes_dir, &key, &version);
    write_stylesheet(&path, css).await?;

    let new = NewCustomTheme {
        key: key.clone(),
        name: name.clone(),
        description: description.clone(),
        appearance,
        css_version: version,
        css_size: css.len() as i64,
        original_filename: display_filename(Some(filename)),
        created_by: actor,
    };
    let id = match custom_db::create(pool, &new).await {
        Ok(id) => id,
        Err(err) => {
            let _ = tokio::fs::remove_file(&path).await;
            return Err(AppError::internal(format!(
                "cannot save custom theme: {err}"
            )));
        }
    };

    registry.reload(pool).await?;
    custom_db::find(pool, id)
        .await
        .map_err(|err| AppError::internal(format!("cannot read custom theme: {err}")))?
        .ok_or_else(|| AppError::internal("custom theme disappeared after insert"))
}

/// Update the editable metadata and, when supplied, replace the stylesheet.
#[allow(clippy::too_many_arguments)]
pub async fn update(
    pool: &SqlitePool,
    registry: &ThemeRegistry,
    settings: &Settings,
    themes_dir: &Path,
    id: i64,
    name: &str,
    description: &str,
    appearance: ThemeAppearance,
    css: Option<&str>,
    filename: Option<&str>,
) -> Result<CustomThemeRow> {
    let name = validate_name(name)?;
    let description = validate_description(description)?;
    let _guard = settings.lock_updates().await;

    let current = custom_db::find(pool, id)
        .await
        .map_err(|err| AppError::internal(format!("cannot read custom theme: {err}")))?
        .ok_or(AppError::NotFound)?;

    let mut old_path = None;
    let mut new_path = None;
    let mut new_version = current.css_version.clone();
    let mut new_size = current.css_size;
    let mut new_filename = current.original_filename.clone();
    if let Some(css) = css {
        let version = version_of(css);
        if version != current.css_version {
            let path = css_path(themes_dir, &current.key, &version);
            write_stylesheet(&path, css).await?;
            new_path = Some(path);
            old_path = Some(css_path(themes_dir, &current.key, &current.css_version));
            new_version = version;
            new_size = css.len() as i64;
            new_filename = display_filename(filename);
        } else if filename.is_some() {
            new_filename = display_filename(filename);
        }
    }

    if let Err(err) = custom_db::update_metadata(pool, id, &name, &description, appearance).await {
        // The database still points at the old version; remove the file that
        // was just written rather than the one still referenced by the row.
        if let Some(path) = new_path.as_ref() {
            let _ = tokio::fs::remove_file(path).await;
        }
        return Err(AppError::internal(format!(
            "cannot update custom theme: {err}"
        )));
    }
    if let Err(err) = custom_db::update_css(pool, id, &new_version, new_size, &new_filename).await {
        return Err(AppError::internal(format!(
            "cannot update custom theme stylesheet: {err}"
        )));
    }

    if let Some(path) = old_path {
        // The row now points at the new file. A failed cleanup only leaves an
        // unreferenced file behind; it is never served.
        let _ = tokio::fs::remove_file(path).await;
    }
    registry.reload(pool).await?;
    custom_db::find(pool, id)
        .await
        .map_err(|err| AppError::internal(format!("cannot read custom theme: {err}")))?
        .ok_or_else(|| AppError::internal("custom theme disappeared after update"))
}

/// Delete an upload. The current default cannot be removed: the administrator
/// has to pick another default first, which keeps exactly one enabled default.
pub async fn delete(
    pool: &SqlitePool,
    registry: &ThemeRegistry,
    settings: &Settings,
    themes_dir: &Path,
    id: i64,
) -> Result<()> {
    let _guard = settings.lock_updates().await;
    let current = custom_db::find(pool, id)
        .await
        .map_err(|err| AppError::internal(format!("cannot read custom theme: {err}")))?
        .ok_or(AppError::NotFound)?;
    let row = theme_db::find(pool, &current.key)
        .await
        .map_err(|err| AppError::internal(format!("cannot read theme state: {err}")))?;
    if row.map(|row| row.is_default).unwrap_or(false) {
        return Err(AppError::conflict(
            "choose another default theme before deleting this custom theme",
        ));
    }

    custom_db::delete(pool, id, &current.key)
        .await
        .map_err(|err| AppError::internal(format!("cannot delete custom theme: {err}")))?;
    let path = css_path(themes_dir, &current.key, &current.css_version);
    let _ = tokio::fs::remove_file(path).await;
    registry.reload(pool).await?;
    Ok(())
}

/// Write the stylesheet atomically: a crash never leaves a half-written file
/// at the URL the database points at.
async fn write_stylesheet(path: &Path, css: &str) -> Result<()> {
    if let Some(parent) = path.parent() {
        tokio::fs::create_dir_all(parent).await.map_err(|err| {
            AppError::internal(format!(
                "cannot create theme directory {}: {err}",
                parent.display()
            ))
        })?;
    }
    let temporary = path.with_extension("css.tmp");
    tokio::fs::write(&temporary, css.as_bytes())
        .await
        .map_err(|err| AppError::internal(format!("cannot write theme stylesheet: {err}")))?;
    tokio::fs::rename(&temporary, path)
        .await
        .map_err(|err| AppError::internal(format!("cannot install theme stylesheet: {err}")))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_and_descriptions_are_bounded() {
        assert_eq!(validate_name("  Night  ").expect("name"), "Night");
        assert!(validate_name("").is_err());
        assert!(validate_name(&"x".repeat(MAX_NAME_LEN + 1)).is_err());
        assert!(validate_name("bad\nname").is_err());
        assert_eq!(validate_description("").expect("empty"), "");
        assert!(validate_description(&"x".repeat(MAX_DESCRIPTION_LEN + 1)).is_err());
    }

    #[test]
    fn stylesheets_are_checked_for_size_utf8_and_dangerous_tokens() {
        assert!(validate_css(b":root { --ink: #111; }").is_ok());
        assert!(validate_css(b"").is_err());
        assert!(validate_css(b"not css").is_err());
        assert!(validate_css(b":root { background: url(https://x.example/a.png); }").is_err());
        assert!(validate_css(b"@import url(/assets/app.css);").is_err());
        assert!(validate_css(b"body { width: expression(alert(1)); }").is_err());
        assert!(validate_css(&[0xff, 0xfe, 0xfd]).is_err());
        let huge = format!("body {{ color: red; }}/*{}*/", "x".repeat(MAX_CSS_BYTES));
        assert!(validate_css(huge.as_bytes()).is_err());
    }

    #[test]
    fn keys_and_versions_are_path_safe() {
        assert!(validate_key("custom-Abc_123-xyz"));
        assert!(!validate_key("custom-"));
        assert!(!validate_key("custom-../escape"));
        assert!(!validate_key("verdant"));
        assert_eq!(version_of("body{}").len(), 12);
        assert!(version_of("body{}")
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit()));
    }

    #[test]
    fn display_filename_drops_paths_and_controls() {
        assert_eq!(display_filename(Some("../../evil.css")), "evil.css");
        assert_eq!(display_filename(Some("a\\b\\night.css")), "night.css");
        assert_eq!(display_filename(Some("bad\u{0}name.css")), "badname.css");
        assert_eq!(display_filename(None), "theme.css");
    }
}
