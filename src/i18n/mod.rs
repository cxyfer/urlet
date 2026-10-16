//! UI localisation.
//!
//! The browser facing interface is available in English, Traditional Chinese and
//! Simplified Chinese. Catalogues are JSON documents embedded into the binary
//! with [`include_str!`] and parsed once into a [`OnceLock`], so a lookup costs a
//! hash map probe and returns a `&'static str` that templates can print directly.
//!
//! The active language lives in a Tokio task local. That matters because code
//! without access to the request — most notably [`crate::error::AppError`]'s
//! `IntoResponse` implementation — still has to render in the caller's language.
//! The locale middleware wraps the whole request in [`scope`].

use std::collections::HashMap;
use std::future::Future;
use std::sync::OnceLock;

use axum::http::{header, HeaderMap};

use crate::services::cookies;

/// Cookie remembering an explicit language choice.
pub const COOKIE_NAME: &str = "uwurls_lang";

/// Lifetime of the language cookie: one year.
pub const COOKIE_MAX_AGE: i64 = 31_536_000;

const EN_JSON: &str = include_str!("en.json");
const ZH_HANT_JSON: &str = include_str!("zh-Hant.json");
const ZH_HANS_JSON: &str = include_str!("zh-Hans.json");

/// A language the interface can be rendered in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Lang {
    En,
    ZhHant,
    ZhHans,
}

/// Every supported language, in switcher order.
pub const ALL: [Lang; 3] = [Lang::En, Lang::ZhHant, Lang::ZhHans];

impl Lang {
    /// BCP-47 tag: used in cookies, `Content-Language` and `<html lang>`.
    pub fn code(self) -> &'static str {
        match self {
            Self::En => "en",
            Self::ZhHant => "zh-Hant",
            Self::ZhHans => "zh-Hans",
        }
    }

    /// Name shown by the language switcher.
    pub fn label(self) -> &'static str {
        match self {
            Self::En => "English",
            Self::ZhHant => "繁體中文",
            Self::ZhHans => "简体中文",
        }
    }

    /// Value for `<html lang="...">`.
    pub fn html_lang(self) -> &'static str {
        self.code()
    }

    /// The three switcher entries.
    pub fn all(self) -> &'static [Lang] {
        &ALL
    }

    /// Parse a language tag, case insensitively.
    ///
    /// `en` and any `en-*` tag map to English. For Chinese the script subtag wins
    /// over the region, and a bare `zh` is treated as Simplified.
    pub fn parse(value: &str) -> Option<Lang> {
        let value = value.trim();
        if value.is_empty() {
            return None;
        }
        let normalized = value.replace('_', "-").to_ascii_lowercase();
        let mut parts = normalized.split('-');
        let primary = parts.next().unwrap_or_default();
        let rest: Vec<&str> = parts.filter(|part| !part.is_empty()).collect();
        match primary {
            "en" => Some(Lang::En),
            "zh" => {
                if rest.contains(&"hant") {
                    return Some(Lang::ZhHant);
                }
                if rest.contains(&"hans") {
                    return Some(Lang::ZhHans);
                }
                if rest.iter().any(|part| matches!(*part, "tw" | "hk" | "mo")) {
                    return Some(Lang::ZhHant);
                }
                // Every other Chinese variant (including bare `zh`) is Simplified.
                Some(Lang::ZhHans)
            }
            _ => None,
        }
    }

    /// Pick the best supported language from an `Accept-Language` header.
    ///
    /// Quality weights are honoured; equal weights keep header order. `q=0`
    /// entries and the `*` wildcard are ignored.
    pub fn from_accept_language(header: &str) -> Option<Lang> {
        let mut candidates: Vec<(f32, usize, Lang)> = Vec::new();
        for (index, item) in header.split(',').enumerate() {
            let item = item.trim();
            if item.is_empty() {
                continue;
            }
            let mut segments = item.split(';');
            let tag = segments.next().unwrap_or_default().trim();
            let mut quality = 1.0f32;
            for parameter in segments {
                let parameter = parameter.trim();
                let raw = parameter
                    .strip_prefix("q=")
                    .or_else(|| parameter.strip_prefix("Q="));
                if let Some(raw) = raw {
                    quality = raw.trim().parse().unwrap_or(0.0);
                }
            }
            if quality <= 0.0 {
                continue;
            }
            if let Some(lang) = Lang::parse(tag) {
                candidates.push((quality, index, lang));
            }
        }
        // `sort_by` is stable, so the index tie-break only documents the intent.
        candidates.sort_by(|a, b| {
            b.0.partial_cmp(&a.0)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then(a.1.cmp(&b.1))
        });
        candidates.first().map(|(_, _, lang)| *lang)
    }

    /// Look up a translation.
    ///
    /// Missing keys fall back to English and, when English does not have them
    /// either, to the key itself. Keys come from the templates rather than from
    /// user input, so the leak in that last branch is bounded and only happens
    /// while developing a new page.
    pub fn t(self, key: &str) -> &'static str {
        if let Some(value) = catalog(self).get(key) {
            return value;
        }
        if let Some(value) = catalog(Lang::En).get(key) {
            return value;
        }
        Box::leak(key.to_string().into_boxed_str())
    }

    /// Every key of this language's catalogue, sorted. Used by the tests that
    /// keep the three catalogues in sync.
    pub fn keys(self) -> Vec<&'static str> {
        let mut keys: Vec<&'static str> = catalog(self).keys().map(String::as_str).collect();
        keys.sort_unstable();
        keys
    }
}

/// The catalogue of a language.
fn catalog(lang: Lang) -> &'static HashMap<String, String> {
    static EN: OnceLock<HashMap<String, String>> = OnceLock::new();
    static ZH_HANT: OnceLock<HashMap<String, String>> = OnceLock::new();
    static ZH_HANS: OnceLock<HashMap<String, String>> = OnceLock::new();
    match lang {
        Lang::En => EN.get_or_init(|| parse_catalog(EN_JSON, "en")),
        Lang::ZhHant => ZH_HANT.get_or_init(|| parse_catalog(ZH_HANT_JSON, "zh-Hant")),
        Lang::ZhHans => ZH_HANS.get_or_init(|| parse_catalog(ZH_HANS_JSON, "zh-Hans")),
    }
}

fn parse_catalog(raw: &str, code: &str) -> HashMap<String, String> {
    serde_json::from_str(raw).unwrap_or_else(|err| {
        // An empty catalogue degrades to English instead of failing the request.
        tracing::error!(language = code, error = %err, "invalid translation catalogue");
        HashMap::new()
    })
}

tokio::task_local! {
    static CURRENT: Lang;
}

/// The language of the request being handled, or English outside a request.
pub fn current() -> Lang {
    CURRENT.try_with(|lang| *lang).unwrap_or(Lang::En)
}

/// Run `fut` with `lang` installed as the task local language.
pub async fn scope<F: Future>(lang: Lang, fut: F) -> F::Output {
    CURRENT.scope(lang, fut).await
}

/// Resolve the language of a request: cookie, then `Accept-Language`, then
/// English.
pub fn resolve(headers: &HeaderMap) -> Lang {
    if let Some(value) = cookies::get(headers, COOKIE_NAME) {
        if let Some(lang) = Lang::parse(&value) {
            return lang;
        }
    }
    if let Some(value) = headers
        .get(header::ACCEPT_LANGUAGE)
        .and_then(|value| value.to_str().ok())
    {
        if let Some(lang) = Lang::from_accept_language(value) {
            return lang;
        }
    }
    Lang::En
}

/// Translate `key` and substitute `{name}` placeholders.
pub fn format(lang: Lang, key: &str, args: &[(&str, String)]) -> String {
    let mut out = lang.t(key).to_string();
    for (name, value) in args {
        out = out.replace(&format!("{{{name}}}"), value);
    }
    out
}

/// Best effort translation of a message produced by the service layer.
///
/// Service messages are English by design: the same text is returned by
/// `/api/v1`, which must stay machine readable and stable. The web pages run the
/// text through this function so the common ones still appear in the visitor's
/// language; anything unrecognised is returned unchanged.
pub fn message(lang: Lang, english: &str) -> String {
    if lang == Lang::En {
        return english.to_string();
    }
    if let Some(key) = message_key(english) {
        return lang.t(key).to_string();
    }
    if let Some(translated) = message_with_value(lang, english) {
        return translated;
    }
    english.to_string()
}

/// Exact matches between a service message and a catalogue key.
fn message_key(english: &str) -> Option<&'static str> {
    Some(match english {
        // auth
        "a valid email address is required" => "error.email.invalid",
        "email must not contain spaces" => "error.email.spaces",
        "email must contain exactly one @" | "email must contain @" => "error.email.at",
        "the part before @ is invalid" => "error.email.local",
        "the domain is invalid" => "error.email.domain",
        "display name must be at most 60 characters" => "error.display_name.length",
        "display name must be a single line" => "error.display_name.single_line",
        "an account with this email already exists" => "error.email.taken",
        "an invite code is required to register" => "error.invite.required",
        "this invite code is not valid" => "error.invite.invalid",
        "this invite code is no longer valid" => "error.invite.used",
        "registration is disabled on this instance" => "error.registration.closed",
        "invalid email or password" => "error.login.invalid",
        "this account is suspended" => "error.account.suspended",
        "current password is incorrect" => "error.password.current",
        "the new password must differ from the current one" => "error.password.same",
        "administrator privileges are required" => "error.admin.required",
        "request rejected: cross-site submission" => "error.csrf",
        // password policy
        "password must not be blank" => "error.password.blank",
        // target URL
        "a destination URL is required" => "error.target.required",
        "the URL must not contain control characters" => "error.target.control",
        "the URL must be absolute (http or https)" => "error.target.absolute",
        "only http and https URLs are supported" => "error.target.scheme",
        "the URL is missing a host" => "error.target.host",
        "the URL must not contain spaces" => "error.target.spaces",
        "the URL host must be a domain name, an IP address or localhost" => {
            "error.target.host_shape"
        }
        // slug
        "slug must not be empty" => "error.slug.empty",
        "slug may only contain letters, digits, '-' and '_'" => "error.slug.chars",
        "that slug is already taken" => "error.slug.taken_short",
        // links
        "redirect type must be 301, 302, 307 or 308" => "error.redirect_type",
        "expiry must be a date or empty" => "error.expiry.empty",
        "expiry must look like 2030-01-31T12:00:00Z" => "error.expiry.format",
        "expiry must be in the future" => "error.expiry.future",
        "the click limit must be at least 1" => "error.max_clicks.min",
        "the click limit is too large" => "error.max_clicks.max",
        "the title must not contain control characters" => "error.title.control",
        "tags must not contain commas or control characters" => "error.tags.chars",
        "the link password must be at least 4 characters" => "error.link_password.min",
        "the link password must be at most 128 characters" => "error.link_password.max",
        "status must be 'active' or 'disabled'" => "error.status.active_disabled",
        "no changes were submitted" => "error.no_changes",
        "no settings were submitted" => "error.no_changes",
        // administration
        "role must be 'user' or 'admin'" => "error.role",
        "status must be 'active' or 'suspended'" => "error.status.active_suspended",
        "you cannot change your own role or status" => "error.admin.self_role",
        "at least one active administrator is required" => "error.admin.last_admin",
        "you cannot delete your own account" => "error.admin.self_delete",
        "uses must be between 1 and 10000" => "error.invite.uses",
        // settings
        "site name must be 1-60 characters" => "error.setting.site_name",
        "slug length must be a number" => "error.setting.slug_number",
        "slug length must be between 4 and 16" => "error.setting.slug_range",
        "quota must be a number" => "error.setting.quota_number",
        "quota must be between 0 and 1000000 (0 = unlimited)" => "error.setting.quota_range",
        "retention must be a number of days" => "error.setting.retention_number",
        "retention must be between 0 and 3650 days" => "error.setting.retention_range",
        "unknown setting" => "error.setting.unknown",
        // API tokens
        "a token name is required" => "error.token.name",
        "the token name must not contain control characters" => "error.token.control",
        // generic errors
        "The requested resource does not exist." => "error.not_found.message",
        "Something went wrong on our side." => "error.internal.message",
        _ => return None,
    })
}

/// Prefix/suffix rules for the service messages that embed a value.
const VALUE_PATTERNS: &[(&str, &str, &str)] = &[
    (
        "the URL must be at most ",
        " characters",
        "error.target.length",
    ),
    ("slug must be at most ", " characters", "error.slug.length"),
    (
        "password must be at least ",
        " characters",
        "error.password.min",
    ),
    (
        "password must be at most ",
        " characters",
        "error.password.max",
    ),
    (
        "the title must be at most ",
        " characters",
        "error.title.length",
    ),
    (
        "notes must be at most ",
        " characters",
        "error.notes.length",
    ),
    (
        "each tag must be at most ",
        " characters",
        "error.tags.length",
    ),
    ("a link can carry at most ", " tags", "error.tags.count"),
    (
        "the token name must be at most ",
        " characters",
        "error.token.length",
    ),
    (
        "the note must be at most ",
        " characters",
        "error.invite.note_length",
    ),
    ("the slug '", "' is already taken", "error.slug.taken"),
    ("'", "' is a reserved path", "error.slug.reserved"),
    (
        "you have reached your limit of ",
        " links",
        "error.quota_reached",
    ),
    ("unknown setting: ", "", "error.setting.unknown_key"),
    (
        "Too many requests. Try again in ",
        " seconds.",
        "error.rate_limited.message",
    ),
];

fn message_with_value(lang: Lang, english: &str) -> Option<String> {
    for (prefix, suffix, key) in VALUE_PATTERNS {
        let Some(rest) = english.strip_prefix(prefix) else {
            continue;
        };
        let Some(value) = rest.strip_suffix(suffix) else {
            continue;
        };
        let template = lang.t(key);
        if template.contains("{value}") {
            return Some(template.replace("{value}", value));
        }
        // The translation does not place the value itself; append it.
        return Some(format!("{template} {value}"));
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn headers_with(pairs: &[(&str, &str)]) -> HeaderMap {
        let mut headers = HeaderMap::new();
        for (name, value) in pairs {
            headers.insert(
                axum::http::HeaderName::from_bytes(name.as_bytes()).expect("name"),
                axum::http::HeaderValue::from_str(value).expect("value"),
            );
        }
        headers
    }

    #[test]
    fn parses_language_tags() {
        assert_eq!(Lang::parse("en"), Some(Lang::En));
        assert_eq!(Lang::parse("EN-us"), Some(Lang::En));
        assert_eq!(Lang::parse("zh"), Some(Lang::ZhHans));
        assert_eq!(Lang::parse("zh-TW"), Some(Lang::ZhHant));
        assert_eq!(Lang::parse("zh_HK"), Some(Lang::ZhHant));
        assert_eq!(Lang::parse("zh-MO"), Some(Lang::ZhHant));
        assert_eq!(Lang::parse("zh-Hant"), Some(Lang::ZhHant));
        assert_eq!(Lang::parse("zh-Hant-TW"), Some(Lang::ZhHant));
        assert_eq!(Lang::parse("zh-CN"), Some(Lang::ZhHans));
        assert_eq!(Lang::parse("zh-SG"), Some(Lang::ZhHans));
        assert_eq!(Lang::parse("zh-Hans"), Some(Lang::ZhHans));
        assert_eq!(Lang::parse(" zh-hans-cn "), Some(Lang::ZhHans));
        assert_eq!(Lang::parse("de"), None);
        assert_eq!(Lang::parse("*"), None);
        assert_eq!(Lang::parse(""), None);
    }

    #[test]
    fn orders_accept_language_by_quality() {
        assert_eq!(
            Lang::from_accept_language("zh-TW;q=0.9,en;q=0.8"),
            Some(Lang::ZhHant)
        );
        assert_eq!(
            Lang::from_accept_language("en;q=0.8,zh-TW;q=0.9"),
            Some(Lang::ZhHant)
        );
        assert_eq!(Lang::from_accept_language("de-DE,en;q=0.7"), Some(Lang::En));
        assert_eq!(
            Lang::from_accept_language("zh-CN,zh;q=0.9,en;q=0.8"),
            Some(Lang::ZhHans)
        );
        assert_eq!(Lang::from_accept_language("de,fr;q=0.5"), None);
        assert_eq!(Lang::from_accept_language("en;q=0"), None);
        assert_eq!(Lang::from_accept_language(""), None);
    }

    #[test]
    fn translates_with_english_fallback() {
        assert_eq!(Lang::ZhHant.t("nav.links"), "連結");
        assert_ne!(Lang::ZhHant.t("nav.links"), Lang::En.t("nav.links"));
        // Unknown keys come back verbatim.
        assert_eq!(Lang::En.t("does.not.exist"), "does.not.exist");
        assert_eq!(Lang::ZhHans.t("does.not.exist"), "does.not.exist");
    }

    #[test]
    fn every_catalogue_has_the_same_keys() {
        let english = Lang::En.keys();
        assert!(!english.is_empty());
        assert_eq!(Lang::ZhHant.keys(), english);
        assert_eq!(Lang::ZhHans.keys(), english);
    }

    #[test]
    fn no_catalogue_value_is_empty() {
        for lang in ALL {
            for key in lang.keys() {
                assert!(
                    !lang.t(key).trim().is_empty(),
                    "{} has an empty value for {key}",
                    lang.code()
                );
            }
        }
    }

    #[test]
    fn resolves_cookie_before_header() {
        let headers = headers_with(&[
            ("cookie", "uwurls_lang=zh-Hant"),
            ("accept-language", "en-US,en;q=0.9"),
        ]);
        assert_eq!(resolve(&headers), Lang::ZhHant);

        let headers = headers_with(&[("accept-language", "zh-TW")]);
        assert_eq!(resolve(&headers), Lang::ZhHant);

        assert_eq!(resolve(&HeaderMap::new()), Lang::En);

        // An unparsable cookie falls through to the header.
        let headers = headers_with(&[
            ("cookie", "uwurls_lang=nonsense"),
            ("accept-language", "zh-Hans"),
        ]);
        assert_eq!(resolve(&headers), Lang::ZhHans);
    }

    #[tokio::test]
    async fn scope_installs_the_task_local_language() {
        assert_eq!(current(), Lang::En);
        let seen = scope(Lang::ZhHant, async { current() }).await;
        assert_eq!(seen, Lang::ZhHant);
        assert_eq!(current(), Lang::En);
    }

    #[test]
    fn formats_placeholders() {
        assert_eq!(
            format(
                Lang::En,
                "common.page_of",
                &[("page", "2".into()), ("total", "5".into())]
            ),
            "Page 2 of 5"
        );
    }

    #[test]
    fn translates_known_service_messages() {
        assert_eq!(
            message(Lang::En, "this invite code is not valid"),
            "this invite code is not valid"
        );
        assert_eq!(
            message(Lang::ZhHant, "this invite code is not valid"),
            "該邀請碼無效。"
        );
        assert_eq!(
            message(Lang::ZhHant, "the slug 'promo' is already taken"),
            "短網址「promo」已被使用。"
        );
        assert_eq!(
            message(Lang::ZhHant, "you have reached your limit of 10 links"),
            "已達到 10 個連結的上限。"
        );
        // Unknown messages are returned unchanged.
        assert_eq!(
            message(Lang::ZhHant, "some brand new failure"),
            "some brand new failure"
        );
    }
}
