//! Middleware: cross cutting HTTP concerns.

pub mod auth;
pub mod csrf;
pub mod locale;
pub mod security;

use std::net::{IpAddr, SocketAddr};

use axum::extract::{ConnectInfo, Request};
use axum::http::{Extensions, HeaderMap, HeaderValue};
use axum::middleware::Next;
use axum::response::Response;

use crate::config::Config;
use crate::services::secret;

/// Best effort client address. Proxy headers are only honoured when
/// `URLET_TRUST_PROXY` is enabled, so a spoofed header cannot bypass the
/// rate limiter in a direct-exposure deployment.
pub fn client_ip_parts(
    headers: &HeaderMap,
    extensions: &Extensions,
    config: &Config,
) -> Option<IpAddr> {
    if config.trust_proxy {
        // `X-Real-IP` is written by the proxy itself (the shipped Caddyfile sets
        // `header_up X-Real-IP {remote_host}`), so a client cannot forge it.
        if let Some(ip) = headers
            .get("x-real-ip")
            .and_then(|value| value.to_str().ok())
            .and_then(|value| value.trim().parse::<IpAddr>().ok())
        {
            return Some(ip);
        }
        // Otherwise take the *rightmost* `X-Forwarded-For` entry: that is the
        // hop the nearest proxy observed. The leftmost entry is attacker
        // supplied for any proxy that appends instead of replacing.
        if let Some(value) = headers
            .get("x-forwarded-for")
            .and_then(|value| value.to_str().ok())
        {
            if let Some(ip) = value
                .split(',')
                .rev()
                .find_map(|hop| hop.trim().parse::<IpAddr>().ok())
            {
                return Some(ip);
            }
        }
    }
    extensions
        .get::<ConnectInfo<SocketAddr>>()
        .map(|info| info.0.ip())
}

/// [`client_ip_parts`] for a whole request.
pub fn client_ip(request: &Request, config: &Config) -> Option<IpAddr> {
    client_ip_parts(request.headers(), request.extensions(), config)
}

/// True when the request was made over TLS (directly or via a trusted proxy).
pub fn is_secure_request(headers: &HeaderMap, config: &Config) -> bool {
    if config.is_https() {
        return true;
    }
    if config.trust_proxy {
        if let Some(proto) = headers
            .get("x-forwarded-proto")
            .and_then(|v| v.to_str().ok())
        {
            return proto.split(',').next().unwrap_or_default().trim() == "https";
        }
    }
    false
}

/// Anonymised client identity, safe to store and to log.
pub fn hashed_client_ip_parts(
    headers: &HeaderMap,
    extensions: &Extensions,
    config: &Config,
) -> Option<String> {
    client_ip_parts(headers, extensions, config)
        .map(|ip| secret::hmac_hex(&config.secret, &ip.to_string()))
}

/// [`hashed_client_ip_parts`] for a whole request.
pub fn hashed_client_ip(request: &Request, config: &Config) -> Option<String> {
    hashed_client_ip_parts(request.headers(), request.extensions(), config)
}

/// Assign a request id, emit one structured access log line per request.
pub async fn request_id(mut request: Request, next: Next) -> Response {
    let id = secret::random_token(6);
    request.extensions_mut().insert(RequestId(id.clone()));
    let method = request.method().clone();
    let path = request.uri().path().to_string();
    let started = std::time::Instant::now();

    let mut response = next.run(request).await;

    tracing::info!(
        request_id = %id,
        method = %method,
        path = %path,
        status = response.status().as_u16(),
        duration_ms = started.elapsed().as_millis() as u64,
        "request"
    );

    if let Ok(value) = HeaderValue::from_str(&id) {
        response.headers_mut().insert("x-request-id", value);
    }
    response
}

/// Request id, injected by [`request_id`].
#[derive(Debug, Clone)]
pub struct RequestId(pub String);

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::Request as HttpRequest;

    fn test_config(trust_proxy: bool) -> Config {
        let mut config = Config::for_tests(std::path::Path::new("."));
        config.trust_proxy = trust_proxy;
        config
    }

    #[test]
    fn ignores_forwarded_headers_unless_trusted() {
        let request = HttpRequest::builder()
            .header("x-forwarded-for", "203.0.113.9, 10.0.0.1")
            .body(Body::empty())
            .expect("request");
        assert_eq!(client_ip(&request, &test_config(false)), None);
        // Rightmost hop: the leftmost entry can be injected by the client.
        assert_eq!(
            client_ip(&request, &test_config(true)),
            Some("10.0.0.1".parse().expect("ip"))
        );
    }

    #[test]
    fn prefers_x_real_ip_over_forwarded_for() {
        let request = HttpRequest::builder()
            .header("x-forwarded-for", "203.0.113.9, 10.0.0.1")
            .header("x-real-ip", "198.51.100.7")
            .body(Body::empty())
            .expect("request");
        assert_eq!(
            client_ip(&request, &test_config(true)),
            Some("198.51.100.7".parse().expect("ip"))
        );
    }

    #[test]
    fn skips_unparsable_forwarded_hops() {
        let request = HttpRequest::builder()
            .header("x-forwarded-for", "not-an-ip, 10.0.0.1, also-bad")
            .body(Body::empty())
            .expect("request");
        assert_eq!(
            client_ip(&request, &test_config(true)),
            Some("10.0.0.1".parse().expect("ip"))
        );
    }

    #[test]
    fn hashes_client_ip() {
        let config = test_config(true);
        let request = HttpRequest::builder()
            .header("x-real-ip", "203.0.113.9")
            .body(Body::empty())
            .expect("request");
        let hashed = hashed_client_ip(&request, &config).expect("hash");
        assert_eq!(hashed.len(), 64);
        assert!(!hashed.contains("203"));
    }

    #[test]
    fn detects_secure_request_from_proxy() {
        let config = test_config(true);
        let mut headers = HeaderMap::new();
        headers.insert("x-forwarded-proto", HeaderValue::from_static("https"));
        assert!(is_secure_request(&headers, &config));
        assert!(!is_secure_request(&HeaderMap::new(), &test_config(false)));
    }
}
