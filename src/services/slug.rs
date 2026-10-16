//! Slug generation and validation.

use rand::Rng;

use crate::error::{AppError, Result};

/// base62 alphabet used for generated slugs.
const ALPHABET: &[u8] = b"0123456789abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ";

/// Longest accepted custom slug.
pub const MAX_LEN: usize = 64;
/// Shortest accepted slug (custom or generated).
pub const MIN_LEN: usize = 1;

/// Paths that must never be shadowed by a short link.
pub const RESERVED: &[&str] = &[
    "api",
    "app",
    "assets",
    "health",
    "healthz",
    "favicon.ico",
    "favicon.svg",
    "robots.txt",
    "sitemap.xml",
    "static",
    "admin",
    "login",
    "logout",
    "register",
    "dashboard",
    "settings",
    "account",
    "urls",
    "well-known",
];

/// Generate a random slug of `length` base62 characters.
pub fn generate(length: usize) -> String {
    let length = length.clamp(MIN_LEN, MAX_LEN);
    let mut rng = rand::thread_rng();
    (0..length)
        .map(|_| ALPHABET[rng.gen_range(0..ALPHABET.len())] as char)
        .collect()
}

/// True when a slug is syntactically valid and not reserved.
pub fn is_valid(slug: &str) -> bool {
    validate(slug).is_ok()
}

/// Validate and normalise a user supplied slug.
pub fn validate(slug: &str) -> Result<String> {
    let slug = slug.trim();
    if slug.chars().count() < MIN_LEN {
        return Err(AppError::validation("slug", "slug must not be empty"));
    }
    if slug.chars().count() > MAX_LEN {
        return Err(AppError::validation(
            "slug",
            format!("slug must be at most {MAX_LEN} characters"),
        ));
    }
    if !slug
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
    {
        return Err(AppError::validation(
            "slug",
            "slug may only contain letters, digits, '-' and '_'",
        ));
    }
    if is_reserved(slug) {
        return Err(AppError::validation(
            "slug",
            format!("'{slug}' is a reserved path"),
        ));
    }
    Ok(slug.to_string())
}

/// Case-insensitive check against the reserved list.
pub fn is_reserved(slug: &str) -> bool {
    let lowered = slug.trim().to_ascii_lowercase();
    RESERVED.iter().any(|reserved| *reserved == lowered)
}

/// Suggest a slug derived from a target URL host, used by the web form.
pub fn suggest_from_host(host: &str) -> String {
    let cleaned: String = host
        .trim_start_matches("www.")
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_')
        .take(24)
        .collect();
    if cleaned.is_empty() {
        generate(7)
    } else {
        cleaned
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generated_slugs_have_the_right_shape() {
        for length in [4, 7, 16] {
            let slug = generate(length);
            assert_eq!(slug.len(), length);
            assert!(slug.chars().all(|c| c.is_ascii_alphanumeric()));
            assert!(is_valid(&slug), "{slug} should be valid");
        }
    }

    #[test]
    fn generated_slugs_are_not_obviously_repeating() {
        let a = generate(16);
        let b = generate(16);
        assert_ne!(a, b);
    }

    #[test]
    fn validates_custom_slugs() {
        assert_eq!(validate("  promo-2026  ").expect("valid"), "promo-2026");
        assert!(validate("under_score").is_ok());
        assert!(validate("").is_err());
        assert!(validate("with space").is_err());
        assert!(validate("with/slash").is_err());
        assert!(validate("with.dot").is_err());
        assert!(validate(&"a".repeat(MAX_LEN + 1)).is_err());
    }

    #[test]
    fn rejects_reserved_paths_case_insensitively() {
        for reserved in ["api", "APP", "Assets", "health", "favicon.ico"] {
            assert!(is_reserved(reserved), "{reserved} must be reserved");
            assert!(validate(reserved).is_err());
        }
        assert!(!is_reserved("apix"));
    }

    #[test]
    fn suggests_slugs_from_hosts() {
        assert_eq!(suggest_from_host("www.example.com"), "examplecom");
        // Unusable hosts still produce a valid random slug.
        let fallback = suggest_from_host("!!!");
        assert_eq!(fallback.len(), 7);
        assert!(is_valid(&fallback));
    }
}
