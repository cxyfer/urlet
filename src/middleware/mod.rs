//! Middleware: cross cutting HTTP concerns.

pub mod security;

use std::net::{IpAddr, SocketAddr};

use axum::extract::{ConnectInfo, Request};
use axum::http::{header, HeaderValue};
use axum::middleware::Next;
use axum::response::Response;

use crate::config::Config;
use crate::services::secret;

/// Best effort client address. Proxy headers are only honoured when
/// `UWURLS_TRUST_PROXY` is enabled, so a spoofed header cannot bypass the
/// rate limiter in a direct-exposure deployment.
pub fn client_ip(request: &Request, config: &Config) -> Option<IpAddr> {
    if config.trust_proxy {
        for name in ["x-forwarded-for", "x-real-ip"] {
            if let Some(value) = request.headers().get(name).and_then(|v| v.to_str().ok()) {
                if let Some(first) = value.split(',').next() {
                    if let Ok(ip) = first.trim().parse::<IpAddr>() {
                        return Some(ip);
                    }
                }
            }
        }
    }
    request.extensions().get::<ConnectInfo<SocketAddr>>().map(|info| info.0.ip())
}

/// True when the request was made over TLS (directly or via a trusted proxy).
pub fn is_secure_request(request: &Request, config: &Config) -> bool {
    if config.is_https() {
        return true;
    }
    if config.trust_proxy {
        if let Some(proto) = request.headers().get("x-forwarded-proto").and_then(|v| v.to_str().ok()) {
            return proto.split(',').next().unwrap_or_default().trim() == "https";
        }
    }
    false
}

/// Anonymised client identity, safe to store and to log.
pub fn hashed_client_ip(request: &Request, config: &Config) -> Option<String> {
    client_ip(request, config).map(|ip| secret::hmac_hex(&config.secret, &ip.to_string()))
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

/// `Set-Cookie` helper used by the session and unlock flows.
pub fn set_cookie_header(value: String) -> (header::HeaderName, HeaderValue) {
    (header::SET_COOKIE, HeaderValue::from_str(&value).unwrap_or_else(|_| HeaderValue::from_static("")))
}

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
    fn ignores_forwarded_header_unless_trusted() {
        let request = HttpRequest::builder()
            .header("x-forwarded-for", "203.0.113.9, 10.0.0.1")
            .body(Body::empty())
            .expect("request");
        assert_eq!(client_ip(&request, &test_config(false)), None);
        assert_eq!(
            client_ip(&request, &test_config(true)),
            Some("203.0.113.9".parse().expect("ip"))
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
        let request = HttpRequest::builder()
            .header("x-forwarded-proto", "https")
            .body(Body::empty())
            .expect("request");
        assert!(is_secure_request(&request, &config));

        let plain = HttpRequest::builder().body(Body::empty()).expect("request");
        assert!(!is_secure_request(&plain, &test_config(false)));
    }
}
