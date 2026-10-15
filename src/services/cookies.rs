//! Cookie parsing and building.
//!
//! Cookies are hand rolled so the dependency tree stays small; the attribute
//! set is fixed by policy (`HttpOnly`, `Path=/`, `SameSite=Lax`, optional
//! `Secure`).

use axum::http::HeaderMap;

/// Name of the session cookie.
pub const SESSION_COOKIE: &str = "uwurls_session";

/// Prefix of per-slug unlock cookies.
pub const UNLOCK_PREFIX: &str = "uwurls_unlock_";

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

/// Name of the unlock cookie for a slug.
pub fn unlock_cookie_name(slug: &str) -> String {
    format!("{UNLOCK_PREFIX}{slug}")
}

/// Build a `Set-Cookie` value.
pub fn build(name: &str, value: &str, max_age_secs: i64, secure: bool) -> String {
    let mut cookie = format!(
        "{name}={value}; Path=/; Max-Age={max_age_secs}; HttpOnly; SameSite=Lax"
    );
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
        headers.insert(axum::http::header::COOKIE, HeaderValue::from_str(cookie).expect("value"));
        headers
    }

    #[test]
    fn reads_a_single_cookie() {
        let headers = headers("uwurls_session=abc123");
        assert_eq!(get(&headers, SESSION_COOKIE), Some("abc123".to_string()));
        assert_eq!(get(&headers, "missing"), None);
    }

    #[test]
    fn reads_cookies_among_others() {
        let headers = headers("theme=dark; uwurls_session=xyz; uwurls_unlock_promo=1");
        assert_eq!(get(&headers, SESSION_COOKIE), Some("xyz".to_string()));
        assert_eq!(get(&headers, "uwurls_unlock_promo"), Some("1".to_string()));
        assert_eq!(get(&headers, "theme"), Some("dark".to_string()));
    }

    #[test]
    fn tolerates_malformed_pairs() {
        let headers = headers("garbage; =empty; uwurls_session=ok");
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
    fn unlock_cookie_names_are_slug_scoped() {
        assert_eq!(unlock_cookie_name("aB3xK9z"), "uwurls_unlock_aB3xK9z");
    }
}
