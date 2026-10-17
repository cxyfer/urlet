//! Signed cookies that unlock a password protected short link.
//!
//! The token binds the hostname, the slug and an expiry to an HMAC of the server
//! secret, so a cookie issued for `a.example.com/promo` cannot be replayed
//! against `b.example.com/promo`.

use crate::services::secret;
use crate::services::time;

/// Issue an unlock token for `(domain, slug)`.
pub fn issue(secret: &[u8], domain: &str, slug: &str, ttl_minutes: i64) -> String {
    let domain = normalise_domain(domain);
    let slug = slug.to_ascii_lowercase();
    let expires_at = time::now().unix_timestamp() + ttl_minutes.max(1) * 60;
    let signature = signature(secret, &domain, &slug, expires_at);
    format!("{expires_at}.{signature}")
}

/// Verify an unlock token for `(domain, slug)`.
pub fn verify(secret: &[u8], domain: &str, slug: &str, token: &str) -> bool {
    let domain = normalise_domain(domain);
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
    let expected = self::signature(secret, &domain, &slug, expires_at);
    secret::constant_time_eq(&expected, signature)
}

/// Cookie name for a `(domain, slug)` pair.
pub fn cookie_name(domain: &str, slug: &str) -> String {
    crate::services::cookies::unlock_cookie_name(&normalise_domain(domain), slug)
}

fn normalise_domain(domain: &str) -> String {
    domain.trim().trim_end_matches('.').to_ascii_lowercase()
}

fn signature(secret: &[u8], domain: &str, slug: &str, expires_at: i64) -> String {
    secret::hmac_hex(secret, &format!("unlock:{domain}:{slug}:{expires_at}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    const SECRET: &[u8] = b"a-test-secret-that-is-long-enough";
    const DOMAIN: &str = "a.example.com";

    #[test]
    fn issues_and_verifies() {
        let token = issue(SECRET, DOMAIN, "Promo", 30);
        assert!(
            verify(SECRET, DOMAIN, "promo", &token),
            "slug matching is case-insensitive"
        );
        assert!(verify(SECRET, "A.EXAMPLE.COM.", "PROMO", &token));
    }

    #[test]
    fn rejects_other_domains_slugs_and_secrets() {
        let token = issue(SECRET, DOMAIN, "promo", 30);
        assert!(!verify(SECRET, "b.example.com", "promo", &token));
        assert!(!verify(SECRET, DOMAIN, "other", &token));
        assert!(!verify(
            b"a-different-secret-long-enough",
            DOMAIN,
            "promo",
            &token
        ));
    }

    #[test]
    fn rejects_expired_and_malformed_tokens() {
        let expired = format!("{}.{}", time::now().unix_timestamp() - 1, "deadbeef");
        assert!(!verify(SECRET, DOMAIN, "promo", &expired));
        assert!(!verify(SECRET, DOMAIN, "promo", ""));
        assert!(!verify(SECRET, DOMAIN, "promo", "no-dot"));
        assert!(!verify(SECRET, DOMAIN, "promo", "not-a-number.abc"));
        assert!(!verify(
            SECRET,
            DOMAIN,
            "promo",
            &format!("{}.", time::now().unix_timestamp() + 60)
        ));
    }

    #[test]
    fn cookie_names_are_domain_and_slug_scoped() {
        assert!(cookie_name(DOMAIN, "promo").starts_with("urlet_unlock_"));
        assert_ne!(cookie_name(DOMAIN, "promo"), cookie_name(DOMAIN, "other"));
        assert_ne!(
            cookie_name(DOMAIN, "promo"),
            cookie_name("b.example.com", "promo")
        );
    }
}
