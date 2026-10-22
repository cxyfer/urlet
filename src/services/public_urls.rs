//! The public shortener: a visitor may create a link from the landing page.
//!
//! Three things stand between the public form and the database, and all three
//! are enforced here rather than in the templates:
//!
//! 1. **Access level** (`public_create_access`) — `guest` also serves anonymous
//!    visitors, `user` only signed-in accounts (the historical behaviour) and
//!    `admin` restricts creation to administrators. A request that fails the
//!    level is refused even when it forges the form fields.
//! 2. **Field switches** (`public_allow_*`) — the administrator decides which
//!    optional inputs the form offers. Submitting a disabled field is refused,
//!    so hiding it in the template is never the only defence.
//! 3. **Rate limits** — a per-address budget for everyone, plus one shared
//!    bucket that the whole anonymous population draws from.
//!
//! A link created by an anonymous visitor is owned by the reserved Guest
//! account ([`crate::db::users::User::is_system`]), because `urls.user_id` is a
//! required foreign key and a visitor has no account. That account is hidden
//! from every user listing, cannot sign in and cannot be edited.

use crate::db::urls::Url;
use crate::db::users::{self, User};
use crate::error::{AppError, Result};
use crate::middleware::auth::{ClientInfo, CurrentUser};
use crate::services::captcha::CaptchaFlow;
use crate::services::ratelimit::{limits, RateLimiter};
use crate::services::settings::{PublicCreateAccess, PublicFields, Settings};
use crate::services::urls::{CreateInput, UrlContext};
use crate::state::AppState;

/// Key prefix of the per-address bucket.
const PER_IP_KEY: &str = "public-create:ip:";
/// Key of the bucket every anonymous visitor shares.
const GLOBAL_KEY: &str = "public-create:global";

/// One hour, the window the configurable limits are expressed in.
const HOUR_SECS: f64 = 3600.0;

/// Everything the public form and the public API may submit.
///
/// The set is deliberately the same as the owner-facing create path so the two
/// stay comparable; which of these a visitor may actually use is decided by
/// [`PublicFields`].
#[derive(Debug, Clone, Default)]
pub struct PublicInput {
    pub target_url: String,
    pub domain: Option<String>,
    pub slug: Option<String>,
    pub title: Option<String>,
    pub tags: Vec<String>,
    pub password: Option<String>,
    pub expires_at: Option<String>,
    pub max_clicks: Option<i64>,
    pub redirect_mode: Option<String>,
    pub splash_delay_seconds: Option<i64>,
    pub splash_title: Option<String>,
    pub splash_message: Option<String>,
}

/// Messages used when a field switch blocks a submission. Kept in one place so
/// the JSON API and the HTML form describe the same rule.
pub mod blocked {
    pub const SLUG: &str = "the public form does not allow custom slugs";
    pub const TITLE: &str = "the public form does not allow titles";
    pub const TAGS: &str = "the public form does not allow tags";
    pub const PASSWORD: &str = "the public form does not allow link passwords";
    pub const EXPIRY: &str = "the public form does not allow link expiry";
    pub const MAX_CLICKS: &str = "the public form does not allow click caps";
    pub const REDIRECT: &str = "the public form does not allow choosing a redirect mode";
    pub const DOMAIN: &str = "the public form does not allow choosing another domain";
}

/// Bot protection flow for a caller of the public endpoint.
///
/// An anonymous visitor is challenged through the `public_captcha` switch; a
/// signed-in browser session through `captcha_on_create`, exactly like the
/// dashboard form, so the public endpoint cannot be used to bypass that switch.
/// A `None` result means the flow is not protected (or no provider is
/// configured) and no provider call is made.
pub fn guard_flow(settings: &Settings, anonymous: bool) -> Option<CaptchaFlow> {
    if anonymous {
        settings.public_captcha().then_some(CaptchaFlow::Public)
    } else {
        settings.captcha_on_create().then_some(CaptchaFlow::Create)
    }
}

/// Refuse a submission that fills in a field the administrator disabled.
fn enforce_fields(fields: PublicFields, input: &PublicInput) -> Result<()> {
    let present = |value: &Option<String>| {
        value
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .is_some()
    };

    if present(&input.slug) && !fields.slug {
        return Err(AppError::forbidden(blocked::SLUG));
    }
    if present(&input.title) && !fields.title {
        return Err(AppError::forbidden(blocked::TITLE));
    }
    if input.tags.iter().any(|tag| !tag.trim().is_empty()) && !fields.tags {
        return Err(AppError::forbidden(blocked::TAGS));
    }
    if present(&input.password) && !fields.password {
        return Err(AppError::forbidden(blocked::PASSWORD));
    }
    if present(&input.expires_at) && !fields.expiry {
        return Err(AppError::forbidden(blocked::EXPIRY));
    }
    if input.max_clicks.is_some() && !fields.max_clicks {
        return Err(AppError::forbidden(blocked::MAX_CLICKS));
    }
    if (present(&input.redirect_mode)
        || input.splash_delay_seconds.is_some()
        || present(&input.splash_title)
        || present(&input.splash_message))
        && !fields.redirect
    {
        return Err(AppError::forbidden(blocked::REDIRECT));
    }
    if present(&input.domain) && !fields.domain {
        return Err(AppError::forbidden(blocked::DOMAIN));
    }
    Ok(())
}

/// Consume one token from a bucket, mapping an empty bucket onto a 429.
fn enforce(limiter: &RateLimiter, key: &str, capacity: f64, refill_per_second: f64) -> Result<()> {
    limiter
        .check(key, capacity, refill_per_second)
        .map_err(AppError::rate_limited)
}

/// Consume one token from a budget expressed as creations per hour.
///
/// `per_hour <= 0` disables the limit entirely, which is what an administrator
/// who wants no ceiling stores.
fn enforce_hourly(limiter: &RateLimiter, key: &str, per_hour: i64) -> Result<()> {
    if per_hour <= 0 {
        return Ok(());
    }
    let capacity = per_hour as f64;
    enforce(limiter, key, capacity, capacity / HOUR_SECS)
}

/// Proof that [`authorize`] accepted a request.
///
/// The inner field is private, so only this module can build one: [`create`]
/// cannot run before the access level, the field switches and the budgets were
/// checked, and the bot protection call the route performs in between is
/// reached only by a request that already paid for a per-address token.
#[derive(Debug)]
pub struct Authorized(());

/// Check everything that must hold before the provider is contacted or the
/// database is touched: the access level, the field switches and the two
/// *per-caller* budgets.
///
/// This runs before bot protection on purpose, matching every other create
/// path: an unprotected submission must not be able to spend provider calls.
/// The shared visitor budget is deliberately **not** charged here — a request
/// that fails bot protection would otherwise let one attacker drain the budget
/// every other visitor draws from. [`admit`] charges it afterwards.
pub fn authorize(
    state: &AppState,
    client: &ClientInfo,
    current: Option<&CurrentUser>,
    input: &PublicInput,
) -> Result<Authorized> {
    let settings = &state.settings;

    if !settings
        .public_create_access()
        .allows(current.map(|current| &current.user))
    {
        return Err(AppError::forbidden(
            crate::services::limits::blocked::CREATE_DISABLED,
        ));
    }
    enforce_fields(settings.public_fields(), input)?;

    // The per-address budget protects the instance from one host, and bounds
    // how many provider calls a single address can trigger.
    enforce_hourly(
        &state.limiter,
        &format!("{PER_IP_KEY}{}", client.limiter_key()),
        settings.public_rate_limit_per_ip(),
    )?;

    // A signed-in caller keeps the per-account bucket every other create path
    // charges, so the public endpoint cannot be used to outrun it.
    if let Some(current) = current {
        enforce(
            &state.limiter,
            &format!("create-url:{}", current.user.id),
            limits::CREATE_URL_CAPACITY,
            limits::CREATE_URL_REFILL_PER_SEC,
        )?;
    }

    Ok(Authorized(()))
}

/// Charge the budget every anonymous visitor shares.
///
/// Call this **after** the flow's bot protection check succeeded (and with the
/// proof [`authorize`] returned), so that a rejected submission cannot consume
/// the budget legitimate visitors need. A signed-in caller has already paid the
/// per-account budget and does not touch the shared one.
pub fn admit(
    state: &AppState,
    current: Option<&CurrentUser>,
    authorized: Authorized,
) -> Result<Authorized> {
    if current.is_none() {
        enforce_hourly(
            &state.limiter,
            GLOBAL_KEY,
            state.settings.public_rate_limit_global(),
        )?;
    }
    Ok(authorized)
}

/// Create a short link from the public form or the public API.
///
/// The caller must have run [`authorize`], verified bot protection with the
/// submitted token when [`guard_flow`] returns a flow, and then passed the proof
/// through [`admit`].
pub async fn create(
    state: &AppState,
    client: &ClientInfo,
    current: Option<&CurrentUser>,
    input: PublicInput,
    _authorized: Authorized,
) -> Result<Url> {
    let actor = match current {
        Some(current) => current.user.clone(),
        None => guest_actor(state).await?,
    };

    let context = UrlContext {
        pool: &state.db,
        settings: &state.settings,
        actor: &actor,
        ip_hash: client.ip_hash.as_deref(),
    };
    context
        .create(CreateInput {
            target_url: input.target_url,
            domain: input.domain,
            slug: input.slug,
            title: input.title,
            notes: None,
            tags: input.tags,
            password: input.password,
            expires_at: input.expires_at,
            max_clicks: input.max_clicks,
            redirect_type: None,
            redirect_mode: input.redirect_mode,
            splash_delay_seconds: input.splash_delay_seconds,
            splash_title: input.splash_title,
            splash_message: input.splash_message,
        })
        .await
}

/// The reserved account that owns links created by anonymous visitors.
async fn guest_actor(state: &AppState) -> Result<User> {
    users::find_system(&state.db)
        .await
        .map_err(AppError::from)?
        .ok_or_else(|| AppError::internal("the reserved Guest account is missing"))
}

/// Access level in force, for the templates and for the tests.
pub fn access(settings: &Settings) -> PublicCreateAccess {
    settings.public_create_access()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn input() -> PublicInput {
        PublicInput {
            target_url: "https://example.com".to_string(),
            ..PublicInput::default()
        }
    }

    fn all() -> PublicFields {
        PublicFields {
            slug: true,
            title: true,
            tags: true,
            password: true,
            expiry: true,
            max_clicks: true,
            redirect: true,
            domain: true,
        }
    }

    fn none() -> PublicFields {
        PublicFields {
            slug: false,
            title: false,
            tags: false,
            password: false,
            expiry: false,
            max_clicks: false,
            redirect: false,
            domain: false,
        }
    }

    #[test]
    fn a_bare_submission_is_always_allowed() {
        // Only the target URL is mandatory, so the switches never block the
        // minimal form.
        enforce_fields(none(), &input()).expect("target only");
        enforce_fields(all(), &input()).expect("target only");
    }

    #[test]
    fn disabled_fields_are_refused_even_when_submitted() {
        let cases: Vec<(&str, PublicInput)> = vec![
            (
                "slug",
                PublicInput {
                    slug: Some("promo".into()),
                    ..input()
                },
            ),
            (
                "title",
                PublicInput {
                    title: Some("Title".into()),
                    ..input()
                },
            ),
            (
                "tags",
                PublicInput {
                    tags: vec!["promo".into()],
                    ..input()
                },
            ),
            (
                "password",
                PublicInput {
                    password: Some("secret".into()),
                    ..input()
                },
            ),
            (
                "expiry",
                PublicInput {
                    expires_at: Some("2030-01-01".into()),
                    ..input()
                },
            ),
            (
                "max_clicks",
                PublicInput {
                    max_clicks: Some(5),
                    ..input()
                },
            ),
            (
                "redirect",
                PublicInput {
                    redirect_mode: Some("splash".into()),
                    ..input()
                },
            ),
            (
                "splash copy",
                PublicInput {
                    splash_title: Some("Wait".into()),
                    ..input()
                },
            ),
            (
                "domain",
                PublicInput {
                    domain: Some("s.example.com".into()),
                    ..input()
                },
            ),
        ];

        for (label, payload) in cases {
            assert!(
                enforce_fields(none(), &payload).is_err(),
                "{label} must be refused when the switch is off"
            );
            assert!(
                enforce_fields(all(), &payload).is_ok(),
                "{label} must be accepted when the switch is on"
            );
        }
    }

    #[test]
    fn blank_optional_values_are_not_submissions() {
        let blank = PublicInput {
            slug: Some("   ".into()),
            title: Some(String::new()),
            tags: vec![String::new(), "  ".into()],
            password: Some(" ".into()),
            expires_at: Some("".into()),
            domain: Some("\t".into()),
            ..input()
        };
        enforce_fields(none(), &blank).expect("blank fields are not submissions");
    }

    #[test]
    fn access_levels_decide_who_may_create() {
        assert!(PublicCreateAccess::Guest.allows(None));
        assert!(!PublicCreateAccess::User.allows(None));
        assert!(!PublicCreateAccess::Admin.allows(None));
    }
}
