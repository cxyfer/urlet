//! Short link business logic: validation, quota, slug allocation, tags, audit.

use sqlx::SqlitePool;

use crate::db::domains::{self as domain_db, Domain};
use crate::db::tags;
use crate::db::urls::{self, NewUrl, Url, UrlState, UrlUpdate};
use crate::db::users::User;
use crate::error::{AppError, Result};
use crate::services::audit::{action, target, Audit};
use crate::services::domain_policy;
use crate::services::limits::{self, blocked};
use crate::services::redirect_mode::{RedirectMode, RedirectModePolicy};
use crate::services::settings::Settings;
use crate::services::{
    domains as domain_service, password, redirect_mode, slug, target as target_url, time,
};

/// Maximum tags attached to one link.
pub const MAX_TAGS: usize = 10;
/// Maximum tag length.
pub const MAX_TAG_LEN: usize = 32;
/// Maximum title length.
pub const MAX_TITLE_LEN: usize = 200;
/// Maximum notes length.
pub const MAX_NOTES_LEN: usize = 1000;
/// How often slug allocation retries on a collision (spec FR-URL-2).
const SLUG_RETRIES: usize = 5;

/// Everything the link service needs from the request context.
pub struct UrlContext<'a> {
    pub pool: &'a SqlitePool,
    pub settings: &'a Settings,
    pub actor: &'a User,
    pub ip_hash: Option<&'a str>,
}

/// Input for creating a short link.
#[derive(Debug, Clone, Default)]
pub struct CreateInput {
    pub target_url: String,
    /// Hostname of the link domain; `None` means the current default domain.
    pub domain: Option<String>,
    pub slug: Option<String>,
    pub title: Option<String>,
    pub notes: Option<String>,
    pub tags: Vec<String>,
    pub password: Option<String>,
    pub expires_at: Option<String>,
    pub max_clicks: Option<i64>,
    pub redirect_type: Option<i64>,
    /// `direct` or `splash`; omitted means the group policy's default.
    pub redirect_mode: Option<String>,
    /// Splash countdown, 1-60 seconds; omitted means the five-second default.
    pub splash_delay_seconds: Option<i64>,
    pub splash_title: Option<String>,
    pub splash_message: Option<String>,
}

/// Input for a partial update. `None` leaves a field untouched; the inner
/// `None` of a double option clears it.
#[derive(Debug, Clone, Default)]
pub struct UpdateInput {
    pub target_url: Option<String>,
    /// Move the link to another active domain.
    pub domain: Option<String>,
    pub slug: Option<String>,
    pub title: Option<Option<String>>,
    pub notes: Option<Option<String>>,
    pub tags: Option<Vec<String>>,
    pub password: Option<Option<String>>,
    pub expires_at: Option<Option<String>>,
    pub max_clicks: Option<Option<i64>>,
    pub redirect_type: Option<i64>,
    pub redirect_mode: Option<String>,
    pub splash_delay_seconds: Option<i64>,
    /// The inner `None` clears the custom heading and falls back to the default.
    pub splash_title: Option<Option<String>>,
    pub splash_message: Option<Option<String>>,
    pub status: Option<String>,
}

/// Validate a redirect type.
pub fn validate_redirect_type(value: i64) -> Result<i64> {
    match value {
        301 | 302 | 307 | 308 => Ok(value),
        _ => Err(AppError::validation(
            "redirect_type",
            "redirect type must be 301, 302, 307 or 308",
        )),
    }
}

/// Validate an expiry timestamp and normalise it to the storage format.
pub fn validate_expiry(raw: &str) -> Result<String> {
    let raw = raw.trim();
    if raw.is_empty() {
        return Err(AppError::validation(
            "expires_at",
            "expiry must be a date or empty",
        ));
    }
    let parsed = time::parse_flexible(raw).ok_or_else(|| {
        AppError::validation("expires_at", "expiry must look like 2030-01-31T12:00:00Z")
    })?;
    if parsed <= time::now() {
        return Err(AppError::validation(
            "expires_at",
            "expiry must be in the future",
        ));
    }
    Ok(time::to_iso(parsed))
}

/// Validate the click limit.
pub fn validate_max_clicks(value: i64) -> Result<i64> {
    if value < 1 {
        return Err(AppError::validation(
            "max_clicks",
            "the click limit must be at least 1",
        ));
    }
    if value > 100_000_000 {
        return Err(AppError::validation(
            "max_clicks",
            "the click limit is too large",
        ));
    }
    Ok(value)
}

/// Validate an optional title.
pub fn validate_title(raw: &str) -> Result<Option<String>> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Ok(None);
    }
    if trimmed.chars().count() > MAX_TITLE_LEN {
        return Err(AppError::validation(
            "title",
            format!("the title must be at most {MAX_TITLE_LEN} characters"),
        ));
    }
    if trimmed.chars().any(|c| c.is_control()) {
        return Err(AppError::validation(
            "title",
            "the title must not contain control characters",
        ));
    }
    Ok(Some(trimmed.to_string()))
}

/// Validate optional notes.
pub fn validate_notes(raw: &str) -> Result<Option<String>> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Ok(None);
    }
    if trimmed.chars().count() > MAX_NOTES_LEN {
        return Err(AppError::validation(
            "notes",
            format!("notes must be at most {MAX_NOTES_LEN} characters"),
        ));
    }
    Ok(Some(trimmed.to_string()))
}

/// Normalise a tag list: trimmed, de-duplicated case-insensitively, bounded.
pub fn normalize_tags(tags: &[String]) -> Result<Vec<String>> {
    let mut seen: Vec<String> = Vec::new();
    let mut result: Vec<String> = Vec::new();
    for tag in tags {
        let trimmed = tag.trim();
        if trimmed.is_empty() {
            continue;
        }
        if trimmed.chars().count() > MAX_TAG_LEN {
            return Err(AppError::validation(
                "tags",
                format!("each tag must be at most {MAX_TAG_LEN} characters"),
            ));
        }
        if trimmed.chars().any(|c| c.is_control() || c == ',') {
            return Err(AppError::validation(
                "tags",
                "tags must not contain commas or control characters",
            ));
        }
        let lowered = trimmed.to_lowercase();
        if seen.contains(&lowered) {
            continue;
        }
        seen.push(lowered);
        result.push(trimmed.to_string());
    }
    if result.len() > MAX_TAGS {
        return Err(AppError::validation(
            "tags",
            format!("a link can carry at most {MAX_TAGS} tags"),
        ));
    }
    Ok(result)
}

/// Validate an optional link password without blocking the runtime.
pub async fn prepare_password_async(raw: Option<&str>) -> Result<Option<String>> {
    match raw.map(str::trim) {
        None => Ok(None),
        Some("") => Ok(None),
        Some(plain) => {
            validate_link_password(plain)?;
            Ok(Some(password::hash_async(plain.to_string()).await?))
        }
    }
}

/// Link password policy (spec FR-PWD-1).
pub fn validate_link_password(plain: &str) -> Result<()> {
    if plain.chars().count() < 4 {
        return Err(AppError::validation(
            "password",
            "the link password must be at least 4 characters",
        ));
    }
    if plain.chars().count() > 128 {
        return Err(AppError::validation(
            "password",
            "the link password must be at most 128 characters",
        ));
    }
    Ok(())
}

/// Validate an optional link password. `Ok(None)` means "no password".
pub fn prepare_password(raw: Option<&str>) -> Result<Option<String>> {
    match raw.map(str::trim) {
        None => Ok(None),
        Some("") => Ok(None),
        Some(plain) => {
            validate_link_password(plain)?;
            Ok(Some(password::hash(plain)?))
        }
    }
}

impl UrlContext<'_> {
    /// Create a short link for the current actor.
    pub async fn create(&self, input: CreateInput) -> Result<Url> {
        // The access level is site wide and gates *every* creation path, not
        // only the public form: an administrator who restricts creation to
        // themselves means the dashboard and the API as well.
        if !self
            .settings
            .public_create_access()
            .allows(Some(self.actor))
        {
            return Err(AppError::forbidden(blocked::CREATE_DISABLED));
        }

        let target = target_url::validate(&input.target_url)?;
        let limits = limits::effective(self.pool, self.settings, self.actor).await?;

        // A group toggle blocks the *use* of a feature; it never touches links
        // that already exist.
        let custom_slug = input
            .slug
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty());
        if custom_slug.is_some() && !limits.allow_custom_slug {
            return Err(AppError::forbidden(blocked::CUSTOM_SLUG));
        }
        if input.tags.iter().any(|tag| !tag.trim().is_empty()) && !limits.allow_tags {
            return Err(AppError::forbidden(blocked::TAGS));
        }
        if input
            .password
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .is_some()
            && !limits.allow_password
        {
            return Err(AppError::forbidden(blocked::PASSWORD));
        }
        if input
            .expires_at
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .is_some()
            && !limits.allow_expiry
        {
            return Err(AppError::forbidden(blocked::EXPIRY));
        }
        if input.max_clicks.is_some() && !limits.allow_max_clicks {
            return Err(AppError::forbidden(blocked::MAX_CLICKS));
        }

        // A group may lock the redirect mode. An omitted mode takes the
        // policy's default, so a locked form does not have to submit one.
        let redirect_mode = match input
            .redirect_mode
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
        {
            Some(raw) => RedirectMode::parse(raw).ok_or_else(|| {
                AppError::validation(
                    "redirect_mode",
                    "redirect mode must be 'direct' or 'splash'",
                )
            })?,
            None => limits.redirect_mode_policy.default_mode(),
        };
        ensure_mode_allowed(limits.redirect_mode_policy, redirect_mode)?;
        let splash_delay_seconds = match input.splash_delay_seconds {
            Some(value) => redirect_mode::validate_delay(value)?,
            None => redirect_mode::DEFAULT_DELAY,
        };
        let splash_title = match input.splash_title.as_deref() {
            Some(raw) => redirect_mode::validate_title(raw)?,
            None => None,
        };
        let splash_message = match input.splash_message.as_deref() {
            Some(raw) => redirect_mode::validate_message(raw)?,
            None => None,
        };

        let policy = domain_policy::effective(self.pool, self.actor).await?;
        let domain = match input
            .domain
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
        {
            Some(raw) => self.resolve_domain(Some(raw)).await?,
            None => domain_db::find_by_id(self.pool, policy.default_domain_id)
                .await
                .map_err(AppError::from)?
                .filter(|domain| domain.is_active())
                .ok_or_else(|| AppError::internal("no active default domain"))?,
        };
        if !policy.allows(domain.id) {
            return Err(AppError::forbidden(blocked::DOMAIN_SELECTION));
        }
        let domain_id = domain.id;

        // The quota is enforced inside the INSERT, not by a separate count:
        // concurrent requests must not both pass the same check.
        let quota = limits.max_urls;

        let tags = normalize_tags(&input.tags)?;
        let (slug, is_custom) = match custom_slug {
            Some(custom) => {
                let custom = slug::validate(custom)?;
                if urls::slug_taken(self.pool, domain_id, &custom)
                    .await
                    .map_err(AppError::from)?
                {
                    return Err(AppError::conflict(format!(
                        "the slug '{custom}' is already taken on {}",
                        domain.hostname
                    )));
                }
                (custom, true)
            }
            None => (self.allocate_slug(domain_id).await?, false),
        };

        let expires_at = match input.expires_at.as_deref() {
            Some(raw) if !raw.trim().is_empty() => Some(validate_expiry(raw)?),
            _ => None,
        };
        let max_clicks = match input.max_clicks {
            Some(value) => Some(validate_max_clicks(value)?),
            None => None,
        };
        let redirect_type = match input.redirect_type {
            Some(value) => validate_redirect_type(value)?,
            None => self.settings.default_redirect_type(),
        };
        let title = match input.title.as_deref() {
            Some(raw) => validate_title(raw)?,
            None => None,
        };
        let notes = match input.notes.as_deref() {
            Some(raw) => validate_notes(raw)?,
            None => None,
        };
        let password_hash = prepare_password_async(input.password.as_deref()).await?;

        let new_url = NewUrl {
            user_id: self.actor.id,
            domain_id,
            slug: slug.clone(),
            target_url: target.clone(),
            title,
            notes,
            password_hash,
            expires_at,
            max_clicks,
            redirect_type,
            redirect_mode: redirect_mode.as_str().to_string(),
            splash_delay_seconds,
            splash_title,
            splash_message,
            is_custom_slug: is_custom,
        };
        let inserted = if quota > 0 {
            urls::create_within_quota(self.pool, &new_url, quota)
                .await
                .map_err(|err| map_url_insert_error(err, &slug))?
        } else {
            Some(
                urls::create(self.pool, &new_url)
                    .await
                    .map_err(|err| map_url_insert_error(err, &slug))?,
            )
        };
        let Some(url_id) = inserted else {
            return Err(AppError::forbidden(format!(
                "you have reached your limit of {quota} links"
            )));
        };

        self.sync_tags(url_id, &tags).await?;

        Audit::new(action::URL_CREATED)
            .actor(self.actor.id, &self.actor.role)
            .target(target::URL, url_id)
            .metadata(serde_json::json!({
                "slug": slug,
                "domain": domain.hostname,
                "custom_slug": is_custom
            }))
            .ip_hash(self.ip_hash.map(str::to_string))
            .record(self.pool)
            .await;

        self.reload(url_id).await
    }

    /// Apply a partial update to a link owned by the current actor.
    pub async fn update(&self, url: &Url, input: UpdateInput) -> Result<Url> {
        let limits = limits::effective(self.pool, self.settings, self.actor).await?;
        let mut changes = UrlUpdate::default();

        if let Some(raw) = input.target_url.as_deref() {
            changes.target_url = Some(target_url::validate(raw)?);
        }

        let current_domain = domain_db::find_by_id(self.pool, url.domain_id)
            .await
            .map_err(AppError::from)?
            .ok_or_else(|| AppError::internal("link domain is missing"))?;

        let target_domain = match input
            .domain
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
        {
            Some(raw) => {
                if domain_service::raw_host_key(raw)
                    .eq_ignore_ascii_case(&domain_service::raw_host_key(&current_domain.hostname))
                {
                    // A link on a domain that has since been disabled (or an
                    // opaque canonical host) can still be edited; keeping the
                    // current domain is not a move.
                    current_domain.clone()
                } else {
                    self.resolve_domain(Some(raw)).await?
                }
            }
            None => current_domain,
        };

        let moving = target_domain.id != url.domain_id;
        if moving {
            let policy = domain_policy::effective(self.pool, self.actor).await?;
            if !policy.allows(target_domain.id) {
                return Err(AppError::forbidden(blocked::DOMAIN_SELECTION));
            }
        }
        if let Some(raw) = input.slug.as_deref() {
            let custom = slug::validate(raw)?;
            if custom != url.slug || moving {
                // Only a *new* slug is the custom-slug feature; moving a link
                // with its current slug is governed by domain selection.
                if custom != url.slug && !limits.allow_custom_slug {
                    return Err(AppError::forbidden(blocked::CUSTOM_SLUG));
                }
                if urls::slug_taken_excluding(self.pool, target_domain.id, &custom, url.id)
                    .await
                    .map_err(AppError::from)?
                {
                    return Err(AppError::conflict(format!(
                        "the slug '{custom}' is already taken on {}",
                        target_domain.hostname
                    )));
                }
                if custom != url.slug {
                    changes.slug = Some(custom);
                    changes.is_custom_slug = Some(true);
                }
            }
        } else if moving
            && urls::slug_taken_excluding(self.pool, target_domain.id, &url.slug, url.id)
                .await
                .map_err(AppError::from)?
        {
            return Err(AppError::conflict(format!(
                "the slug '{}' is already taken on {}",
                url.slug, target_domain.hostname
            )));
        }
        if moving {
            changes.domain_id = Some(target_domain.id);
        }
        if let Some(raw) = &input.title {
            changes.title = Some(match raw.as_deref() {
                Some(value) => validate_title(value)?,
                None => None,
            });
        }
        if let Some(raw) = &input.notes {
            changes.notes = Some(match raw.as_deref() {
                Some(value) => validate_notes(value)?,
                None => None,
            });
        }
        if let Some(raw) = &input.password {
            if raw
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .is_some()
                && !limits.allow_password
            {
                return Err(AppError::forbidden(blocked::PASSWORD));
            }
            changes.password_hash = Some(prepare_password_async(raw.as_deref()).await?);
        }
        if let Some(raw) = &input.expires_at {
            let next = match raw
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty())
            {
                Some(value) => {
                    // An expired link must remain editable from its own page:
                    // reusing the stored instant is not a new expiry.
                    if Some(value) == url.expires_at.as_deref() {
                        Some(value.to_string())
                    } else {
                        Some(validate_expiry(value)?)
                    }
                }
                None => None,
            };
            // Keeping the current value is not "newly enabling" the feature.
            if next.is_some() && !limits.allow_expiry && next != url.expires_at {
                return Err(AppError::forbidden(blocked::EXPIRY));
            }
            changes.expires_at = Some(next);
        }
        if let Some(raw) = &input.max_clicks {
            let next = match raw {
                Some(value) => Some(validate_max_clicks(*value)?),
                None => None,
            };
            if next.is_some() && !limits.allow_max_clicks && next != url.max_clicks {
                return Err(AppError::forbidden(blocked::MAX_CLICKS));
            }
            changes.max_clicks = Some(next);
        }
        if let Some(value) = input.redirect_type {
            changes.redirect_type = Some(validate_redirect_type(value)?);
        }
        if let Some(raw) = input
            .redirect_mode
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
        {
            let mode = RedirectMode::parse(raw).ok_or_else(|| {
                AppError::validation(
                    "redirect_mode",
                    "redirect mode must be 'direct' or 'splash'",
                )
            })?;
            // Keeping the current mode is always allowed; only switching to a
            // mode the group blocks is refused, matching the feature toggles.
            if mode.as_str() != url.redirect_mode {
                ensure_mode_allowed(limits.redirect_mode_policy, mode)?;
            }
            changes.redirect_mode = Some(mode.as_str().to_string());
        }
        if let Some(value) = input.splash_delay_seconds {
            changes.splash_delay_seconds = Some(redirect_mode::validate_delay(value)?);
        }
        if let Some(raw) = &input.splash_title {
            // `Some(None)` clears the column; a bare `None` would mean "keep".
            changes.splash_title = Some(match raw.as_deref() {
                Some(value) => redirect_mode::validate_title(value)?,
                None => None,
            });
        }
        if let Some(raw) = &input.splash_message {
            changes.splash_message = Some(match raw.as_deref() {
                Some(value) => redirect_mode::validate_message(value)?,
                None => None,
            });
        }
        if let Some(status) = input.status.as_deref() {
            let status = status.trim().to_ascii_lowercase();
            if status != "active" && status != "disabled" {
                return Err(AppError::validation(
                    "status",
                    "status must be 'active' or 'disabled'",
                ));
            }
            changes.status = Some(status);
        }

        let new_tags = match &input.tags {
            Some(tags) => Some(normalize_tags(tags)?),
            None => None,
        };
        if let Some(tags) = &new_tags {
            if !tags.is_empty() && !limits.allow_tags {
                let current = stored_tag_names(self.pool, url.id).await?;
                if !same_tags(tags, &current) {
                    return Err(AppError::forbidden(blocked::TAGS));
                }
            }
        }

        if changes.is_empty() && new_tags.is_none() {
            return Err(AppError::bad_request("no changes were submitted"));
        }

        if !changes.is_empty() {
            let affected = urls::update(self.pool, url.id, Some(self.actor.id), &changes)
                .await
                .map_err(|err| {
                    if is_unique_violation(&err) {
                        AppError::conflict(format!(
                            "that slug is already taken on {}",
                            target_domain.hostname
                        ))
                    } else {
                        AppError::from(err)
                    }
                })?;
            if affected == 0 {
                return Err(AppError::not_found());
            }
        }

        if let Some(tags) = &new_tags {
            self.sync_tags(url.id, tags).await?;
        }

        Audit::new(action::URL_UPDATED)
            .actor(self.actor.id, &self.actor.role)
            .target(target::URL, url.id)
            .metadata(serde_json::json!({
                "slug": url.slug,
                "domain": target_domain.hostname,
                "fields": describe_changes(&changes, new_tags.is_some()),
            }))
            .ip_hash(self.ip_hash.map(str::to_string))
            .record(self.pool)
            .await;

        self.reload(url.id).await
    }

    /// Soft delete a link.
    pub async fn delete(&self, url: &Url) -> Result<()> {
        let deleted = urls::soft_delete(self.pool, url.id, Some(self.actor.id))
            .await
            .map_err(AppError::from)?;
        if !deleted {
            return Err(AppError::not_found());
        }
        Audit::new(action::URL_DELETED)
            .actor(self.actor.id, &self.actor.role)
            .target(target::URL, url.id)
            .metadata(serde_json::json!({ "slug": url.slug }))
            .ip_hash(self.ip_hash.map(str::to_string))
            .record(self.pool)
            .await;
        Ok(())
    }

    /// Load a link that must belong to the actor.
    pub async fn find_owned(&self, id: i64) -> Result<Url> {
        urls::find_owned(self.pool, id, self.actor.id)
            .await
            .map_err(AppError::from)?
            .ok_or_else(AppError::not_found)
    }

    /// Allocate a random slug that is not taken yet on `domain_id`.
    async fn allocate_slug(&self, domain_id: i64) -> Result<String> {
        let length = self.settings.default_slug_length();
        for _ in 0..SLUG_RETRIES {
            let candidate = slug::generate(length);
            if !urls::slug_taken(self.pool, domain_id, &candidate)
                .await
                .map_err(AppError::from)?
            {
                return Ok(candidate);
            }
        }
        // Extremely unlikely; fall back to a longer slug before giving up.
        for _ in 0..SLUG_RETRIES {
            let candidate = slug::generate(length + 4);
            if !urls::slug_taken(self.pool, domain_id, &candidate)
                .await
                .map_err(AppError::from)?
            {
                return Ok(candidate);
            }
        }
        Err(AppError::internal("could not allocate a unique slug"))
    }

    /// Resolve the requested domain or the current active default.
    async fn resolve_domain(&self, requested: Option<&str>) -> Result<Domain> {
        let Some(raw) = requested.map(str::trim).filter(|value| !value.is_empty()) else {
            return domain_db::active_default(self.pool)
                .await
                .map_err(AppError::from)?
                .ok_or_else(|| AppError::internal("no active default domain"));
        };

        domain_service::find_active_by_input(self.pool, raw)
            .await
            .map_err(AppError::from)?
            .ok_or_else(|| AppError::validation("domain", "unknown or inactive domain"))
    }

    async fn sync_tags(&self, url_id: i64, tags: &[String]) -> Result<()> {
        let mut ids = Vec::with_capacity(tags.len());
        for tag in tags {
            let id = tags::ensure(self.pool, self.actor.id, tag)
                .await
                .map_err(AppError::from)?;
            ids.push(id);
        }
        tags::set_for_url(self.pool, url_id, &ids)
            .await
            .map_err(AppError::from)?;
        Ok(())
    }

    async fn reload(&self, url_id: i64) -> Result<Url> {
        urls::find_by_id(self.pool, url_id)
            .await
            .map_err(AppError::from)?
            .ok_or_else(AppError::not_found)
    }
}

/// Human readable summary of a change set, stored in the audit metadata.
fn describe_changes(changes: &UrlUpdate, tags_changed: bool) -> Vec<&'static str> {
    let mut fields = Vec::new();
    if changes.domain_id.is_some() {
        fields.push("domain");
    }
    if changes.slug.is_some() {
        fields.push("slug");
    }
    if changes.target_url.is_some() {
        fields.push("target_url");
    }
    if changes.title.is_some() {
        fields.push("title");
    }
    if changes.notes.is_some() {
        fields.push("notes");
    }
    if changes.password_hash.is_some() {
        fields.push("password");
    }
    if changes.expires_at.is_some() {
        fields.push("expires_at");
    }
    if changes.max_clicks.is_some() {
        fields.push("max_clicks");
    }
    if changes.redirect_type.is_some() {
        fields.push("redirect_type");
    }
    if changes.redirect_mode.is_some() {
        fields.push("redirect_mode");
    }
    if changes.splash_delay_seconds.is_some()
        || changes.splash_title.is_some()
        || changes.splash_message.is_some()
    {
        fields.push("splash");
    }
    if changes.status.is_some() {
        fields.push("status");
    }
    if tags_changed {
        fields.push("tags");
    }
    fields
}

/// Refuse a redirect mode the actor's group blocks.
fn ensure_mode_allowed(policy: RedirectModePolicy, mode: RedirectMode) -> Result<()> {
    if policy.allows(mode) {
        return Ok(());
    }
    Err(AppError::forbidden(match mode {
        RedirectMode::Direct => blocked::REDIRECT_DIRECT,
        RedirectMode::Splash => blocked::REDIRECT_SPLASH,
    }))
}

/// Convenience accessor for the state of a link.
pub fn state_of(url: &Url) -> UrlState {
    url.state()
}

pub(crate) fn is_unique_violation(err: &sqlx::Error) -> bool {
    matches!(err, sqlx::Error::Database(db) if db.code().as_deref() == Some("2067"))
}

/// Turn a unique-index violation on insert into the user-facing conflict.
fn map_url_insert_error(err: sqlx::Error, slug: &str) -> AppError {
    if is_unique_violation(&err) {
        AppError::conflict(format!("the slug '{slug}' is already taken"))
    } else {
        AppError::from(err)
    }
}

/// Tag names currently attached to a link, lowercased for comparison.
async fn stored_tag_names(pool: &SqlitePool, url_id: i64) -> Result<Vec<String>> {
    Ok(tags::for_url(pool, url_id)
        .await
        .map_err(AppError::from)?
        .into_iter()
        .map(|tag| tag.name.to_lowercase())
        .collect())
}

/// Compare tag lists regardless of order, case and duplicates.
fn same_tags(left: &[String], right: &[String]) -> bool {
    fn normalize(values: &[String]) -> Vec<String> {
        let mut values: Vec<String> = values
            .iter()
            .map(|value| value.trim().to_lowercase())
            .filter(|value| !value.is_empty())
            .collect();
        values.sort_unstable();
        values.dedup();
        values
    }
    normalize(left) == normalize(right)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validates_redirect_types() {
        for code in [301, 302, 307, 308] {
            assert_eq!(validate_redirect_type(code).expect("valid"), code);
        }
        assert!(validate_redirect_type(200).is_err());
        assert!(validate_redirect_type(404).is_err());
    }

    #[test]
    fn validates_expiry() {
        let future = time::iso_in_hours(24);
        assert_eq!(validate_expiry(&future).expect("valid"), future);
        assert!(validate_expiry("2030-01-01").is_ok());
        assert!(validate_expiry("yesterday").is_err());
        let past = time::iso_days_ago(1);
        assert!(validate_expiry(&past).is_err());
        assert!(validate_expiry("").is_err());
    }

    #[test]
    fn validates_click_limits() {
        assert_eq!(validate_max_clicks(1).expect("valid"), 1);
        assert!(validate_max_clicks(0).is_err());
        assert!(validate_max_clicks(-5).is_err());
    }

    #[test]
    fn normalises_tags() {
        let tags = vec![
            "  Promo ".to_string(),
            "promo".to_string(),
            String::new(),
            "News".to_string(),
        ];
        assert_eq!(normalize_tags(&tags).expect("valid"), vec!["Promo", "News"]);
        assert!(normalize_tags(&["a,b".to_string()]).is_err());
        assert!(normalize_tags(&["x".repeat(MAX_TAG_LEN + 1)]).is_err());
        let too_many: Vec<String> = (0..MAX_TAGS + 1).map(|index| format!("t{index}")).collect();
        assert!(normalize_tags(&too_many).is_err());
    }

    #[test]
    fn validates_titles_and_notes() {
        assert_eq!(
            validate_title("  Hello  ").expect("valid"),
            Some("Hello".to_string())
        );
        assert_eq!(validate_title("   ").expect("valid"), None);
        assert!(validate_title(&"x".repeat(MAX_TITLE_LEN + 1)).is_err());
        assert_eq!(
            validate_notes(" note ").expect("valid"),
            Some("note".to_string())
        );
        assert!(validate_notes(&"x".repeat(MAX_NOTES_LEN + 1)).is_err());
    }

    #[test]
    fn prepares_link_passwords() {
        assert_eq!(prepare_password(None).expect("none"), None);
        assert_eq!(prepare_password(Some("")).expect("empty"), None);
        assert_eq!(prepare_password(Some("   ")).expect("blank"), None);
        assert!(prepare_password(Some("abc")).is_err());
        let hash = prepare_password(Some("linkpass"))
            .expect("hashed")
            .expect("some");
        assert!(hash.starts_with("$argon2id$"));
        assert!(password::verify("linkpass", &hash));
    }
}
