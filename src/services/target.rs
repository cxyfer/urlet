//! Target URL validation.
//!
//! Only `http` and `https` targets are accepted. This is what keeps the
//! redirector from becoming an open redirector for `javascript:`, `data:` or
//! `file:` payloads.

use crate::error::{AppError, Result};

/// Longest accepted target URL.
pub const MAX_LEN: usize = 2048;

/// Validate and canonicalise a target URL.
pub fn validate(raw: &str) -> Result<String> {
    let raw = raw.trim();
    if raw.is_empty() {
        return Err(AppError::validation(
            "target_url",
            "a destination URL is required",
        ));
    }
    if raw.chars().count() > MAX_LEN {
        return Err(AppError::validation(
            "target_url",
            format!("the URL must be at most {MAX_LEN} characters"),
        ));
    }
    if raw.chars().any(|c| c.is_control()) {
        return Err(AppError::validation(
            "target_url",
            "the URL must not contain control characters",
        ));
    }

    let candidate = add_missing_scheme(raw);
    let (scheme, rest) = candidate.split_once("://").ok_or_else(|| {
        AppError::validation("target_url", "the URL must be absolute (http or https)")
    })?;

    if !scheme.eq_ignore_ascii_case("http") && !scheme.eq_ignore_ascii_case("https") {
        return Err(AppError::validation(
            "target_url",
            "only http and https URLs are supported",
        ));
    }

    let authority = rest.split(['/', '?', '#']).next().unwrap_or(rest);
    let host = authority.rsplit('@').next().unwrap_or(authority);
    if host.is_empty() {
        return Err(AppError::validation(
            "target_url",
            "the URL is missing a host",
        ));
    }
    if host.chars().any(char::is_whitespace) {
        return Err(AppError::validation(
            "target_url",
            "the URL must not contain spaces",
        ));
    }
    let host_without_port = host.split(':').next().unwrap_or(host);
    if host_without_port.is_empty() {
        return Err(AppError::validation(
            "target_url",
            "the URL is missing a host",
        ));
    }
    let looks_like_host = host_without_port == "localhost"
        || host_without_port.parse::<std::net::IpAddr>().is_ok()
        || host_without_port.contains('.');
    if !looks_like_host {
        return Err(AppError::validation(
            "target_url",
            "the URL host must be a domain name, an IP address or localhost",
        ));
    }

    Ok(format!(
        "{}://{}",
        scheme.to_ascii_lowercase(),
        lowercase_host(rest)
    ))
}

/// Prefix `https://` when the user typed a bare host.
///
/// `example.com/x` and `localhost:3000` get a scheme; `javascript:alert(1)` and
/// `data:text/html,...` do not, so the scheme check downstream rejects them.
fn add_missing_scheme(raw: &str) -> String {
    if raw.contains("://") {
        return raw.to_string();
    }
    let head = raw.split(['/', '?', '#']).next().unwrap_or(raw);
    if let Some((scheme, rest)) = head.split_once(':') {
        let looks_like_port = !rest.is_empty() && rest.chars().all(|c| c.is_ascii_digit());
        let looks_like_scheme =
            !scheme.is_empty() && scheme.chars().all(|c| c.is_ascii_alphabetic());
        if looks_like_scheme && !looks_like_port {
            return raw.to_string();
        }
    }
    format!("https://{raw}")
}

/// Lowercase only the authority component; paths are case sensitive.
fn lowercase_host(rest: &str) -> String {
    match rest.find(['/', '?', '#']) {
        Some(index) => format!("{}{}", rest[..index].to_ascii_lowercase(), &rest[index..]),
        None => rest.to_ascii_lowercase(),
    }
}

/// Extract the host of a validated target (used for stats grouping).
pub fn host_of(target_url: &str) -> Option<String> {
    let rest = target_url.split_once("://")?.1;
    let authority = rest.split(['/', '?', '#']).next()?;
    let host = authority.rsplit('@').next()?;
    let host = host.split(':').next()?;
    if host.is_empty() {
        None
    } else {
        Some(host.to_ascii_lowercase())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_http_and_https() {
        assert_eq!(
            validate("https://example.com/a?b=1").expect("valid"),
            "https://example.com/a?b=1"
        );
        assert_eq!(
            validate("http://example.com").expect("valid"),
            "http://example.com"
        );
    }

    #[test]
    fn adds_https_to_bare_hosts() {
        assert_eq!(
            validate("example.com/path").expect("valid"),
            "https://example.com/path"
        );
        assert_eq!(
            validate("localhost:3000").expect("valid"),
            "https://localhost:3000"
        );
        assert_eq!(
            validate("127.0.0.1/x").expect("valid"),
            "https://127.0.0.1/x"
        );
        assert_eq!(
            validate("example.com:8443/a").expect("valid"),
            "https://example.com:8443/a"
        );
    }

    #[test]
    fn lowercases_only_the_host() {
        assert_eq!(
            validate("HTTPS://Example.COM/CaseSensitive").expect("valid"),
            "https://example.com/CaseSensitive"
        );
    }

    #[test]
    fn rejects_dangerous_schemes() {
        for target in [
            "javascript:alert(1)",
            "data:text/html;base64,PHNjcmlwdD4=",
            "file:///etc/passwd",
            "ftp://example.com/x",
            "mailto:someone@example.com",
        ] {
            assert!(validate(target).is_err(), "{target} must be rejected");
        }
    }

    #[test]
    fn rejects_malformed_urls() {
        assert!(validate("").is_err());
        assert!(validate("   ").is_err());
        assert!(validate("https://").is_err());
        assert!(validate("https://exa mple.com").is_err());
        assert!(validate(&format!("https://example.com/{}", "a".repeat(MAX_LEN))).is_err());
    }

    #[test]
    fn extracts_hosts() {
        assert_eq!(
            host_of("https://Example.com:8443/a").as_deref(),
            Some("example.com")
        );
        assert_eq!(host_of("not-a-url"), None);
    }
}
