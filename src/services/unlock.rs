//! Signed cookies that unlock a password protected short link.
//!
//! The token binds the slug and an expiry to an HMAC of the server secret, so
//! a cookie issued for one link cannot be replayed against another.

use crate::services::secret;
use crate::services::time;

/// Issue an unlock token for `slug` (always lowercased: slugs are
/// case-insensitive, so the cookie name must be too).
pub fn issue(secret: &[u8], slug: &str, ttl_minutes: i64) -> String {
    let slug = slug.to_ascii_lowercase();
    let expires_at = time::now().unix_timestamp() + ttl_minutes.max(1) * 60;
    let signature = signature(secret, &slug, expires_at);
    format!("{expires_at}.{signature}")
}

/// Verify an unlock token for `slug`.
pub fn verify(secret: &[u8], slug: &str, token: &str) -> bool {
    let slug = slug.to_ascii_lowercase();
    let Some((expires_at, signature)) = token.split_once('.') else {
        return false;
    };
    let Ok(expires_at) = expires_at.parse::<i64>() else {
        return false;
    };
    if expires_at <= time::now().unix_timestamp() {
        return false;
    }
    let expected = self::signature(secret, &slug, expires_at);
    secret::constant_time_eq(&expected, signature)
}

/// Cookie name for a slug (lowercased for case-insensitive matching).
pub fn cookie_name(slug: &str) -> String {
    crate::services::cookies::unlock_cookie_name(&slug.to_ascii_lowercase())
}

fn signature(secret: &[u8], slug: &str, expires_at: i64) -> String {
    secret::hmac_hex(secret, &format!("unlock:{slug}:{expires_at}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    const SECRET: &[u8] = b"a-test-secret-that-is-long-enough";

    #[test]
    fn issues_and_verifies() {
        let token = issue(SECRET, "Promo", 30);
        assert!(verify(SECRET, "promo", &token), "slug matching is case-insensitive");
        assert!(verify(SECRET, "PROMO", &token));
    }

    #[test]
    fn rejects_other_slugs_and_secrets() {
        let token = issue(SECRET, "promo", 30);
        assert!(!verify(SECRET, "other", &token));
        assert!(!verify(b"a-different-secret-long-enough", "promo", &token));
    }

    #[test]
    fn rejects_expired_and_malformed_tokens() {
        let expired = format!("{}.{}", time::now().unix_timestamp() - 1, "deadbeef");
        assert!(!verify(SECRET, "promo", &expired));
        assert!(!verify(SECRET, "promo", ""));
        assert!(!verify(SECRET, "promo", "no-dot"));
        assert!(!verify(SECRET, "promo", "not-a-number.abc"));
        assert!(!verify(SECRET, "promo", &format!("{}.", time::now().unix_timestamp() + 60)));
    }

    #[test]
    fn cookie_names_are_slug_scoped() {
        assert_eq!(cookie_name("Promo"), "uwurls_unlock_promo");
        assert_ne!(cookie_name("promo"), cookie_name("other"));
    }
}
