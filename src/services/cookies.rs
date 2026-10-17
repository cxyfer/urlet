//! Cookie parsing and building.
//!
//! Cookies are hand rolled so the dependency tree stays small; the attribute
//! set is fixed by policy (`HttpOnly`, `Path=/`, `SameSite=Lax`, optional
//! `Secure`).

use axum::http::HeaderMap;
use sha2::{Digest, Sha256};

/// Name of the session cookie.
pub const SESSION_COOKIE: &str = "urlet_session";

/// Prefix of per-slug unlock cookies.
pub const UNLOCK_PREFIX: &str = "urlet_unlock_";

/// Read a cookie value from the request headers.
pub fn get(headers: &HeaderMap, name: &str) -> Option<String> {
    for header in headers.get_all(axum::http::header::COOKIE) {
        let Ok(raw) = header.to_str() else { continue };
        for pair in raw.split(';') {
            let pair = pair.trim();
            if let Some((key, value)) = pair.split_once('=') {
                if key.trim() == name {
                    return Some(value.trim().to_string());
                }
            }
        }
    }
    None
}

/// Read every cookie pair from the request headers.
pub fn get_all(headers: &HeaderMap) -> Vec<(String, String)> {
    let mut pairs = Vec::new();
    for header in headers.get_all(axum::http::header::COOKIE) {
        let Ok(raw) = header.to_str() else { continue };
        for pair in raw.split(';') {
            let pair = pair.trim();
            if let Some((key, value)) = pair.split_once('=') {
                let key = key.trim();
                if !key.is_empty() {
                    pairs.push((key.to_string(), value.trim().to_string()));
                }
            }
        }
    }
    pairs
}

/// Name of the unlock cookie for a `(domain, slug)` pair.
///
/// A short hash of the hostname keeps the name inside the cookie-name length
/// limits while still distinguishing the same slug on different domains.
pub fn unlock_cookie_name(domain: &str, slug: &str) -> String {
    let digest = Sha256::digest(domain.to_ascii_lowercase().as_bytes());
    let tag: String = digest[..6]
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    format!("{UNLOCK_PREFIX}{tag}_{}", slug.to_ascii_lowercase())
}

/// Build a `Set-Cookie` value.
pub fn build(name: &str, value: &str, max_age_secs: i64, secure: bool) -> String {
    let mut cookie =
        format!("{name}={value}; Path=/; Max-Age={max_age_secs}; HttpOnly; SameSite=Lax");
    if secure {
        cookie.push_str("; Secure");
    }
    cookie
}

/// Build a `Set-Cookie` value that removes the cookie.
pub fn clear(name: &str, secure: bool) -> String {
    build(name, "", 0, secure)
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::HeaderValue;

    fn headers(cookie: &str) -> HeaderMap {
        let mut headers = HeaderMap::new();
        headers.insert(
            axum::http::header::COOKIE,
            HeaderValue::from_str(cookie).expect("value"),
        );
        headers
    }

    #[test]
    fn reads_a_single_cookie() {
        let headers = headers("urlet_session=abc123");
        assert_eq!(get(&headers, SESSION_COOKIE), Some("abc123".to_string()));
        assert_eq!(get(&headers, "missing"), None);
    }

    #[test]
    fn reads_cookies_among_others() {
        let headers = headers("theme=dark; urlet_session=xyz; urlet_unlock_promo=1");
        assert_eq!(get(&headers, SESSION_COOKIE), Some("xyz".to_string()));
        assert_eq!(get(&headers, "urlet_unlock_promo"), Some("1".to_string()));
        assert_eq!(get(&headers, "theme"), Some("dark".to_string()));
    }

    #[test]
    fn tolerates_malformed_pairs() {
        let headers = headers("garbage; =empty; urlet_session=ok");
        assert_eq!(get(&headers, SESSION_COOKIE), Some("ok".to_string()));
    }

    #[test]
    fn builds_hardened_cookies() {
        let cookie = build(SESSION_COOKIE, "token", 3600, true);
        assert!(cookie.contains("HttpOnly"));
        assert!(cookie.contains("SameSite=Lax"));
        assert!(cookie.contains("Path=/"));
        assert!(cookie.contains("Secure"));
        assert!(cookie.contains("Max-Age=3600"));

        let cleared = clear(SESSION_COOKIE, false);
        assert!(cleared.contains("Max-Age=0"));
        assert!(!cleared.contains("Secure"));
    }

    #[test]
    fn lists_every_cookie_pair() {
        let headers = headers("theme=dark; urlet_session=xyz; broken");
        let pairs = get_all(&headers);
        assert_eq!(pairs.len(), 2);
        assert_eq!(pairs[0], ("theme".to_string(), "dark".to_string()));
        assert_eq!(pairs[1], ("urlet_session".to_string(), "xyz".to_string()));
        assert!(get_all(&HeaderMap::new()).is_empty());
    }

    #[test]
    fn unlock_cookie_names_are_domain_and_slug_scoped() {
        let first = unlock_cookie_name("a.example.com", "Promo");
        let second = unlock_cookie_name("b.example.com", "promo");
        assert!(first.starts_with("urlet_unlock_"));
        assert!(first.ends_with("_promo"));
        assert_ne!(first, second, "same slug on another domain differs");
        assert_eq!(
            first,
            unlock_cookie_name("A.EXAMPLE.COM", "PROMO"),
            "hostname and slug are case-insensitive"
        );
    }
}
