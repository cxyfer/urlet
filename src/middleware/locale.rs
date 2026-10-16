//! Locale middleware.
//!
//! Resolves the language of every request once and installs it as the task local
//! language for the rest of the stack, so handlers, templates and
//! `AppError::into_response` all render in the same language.

use axum::extract::Request;
use axum::http::{header, HeaderMap, HeaderValue};
use axum::middleware::Next;
use axum::response::Response;

use crate::i18n;

/// Added to the `Vary` header of HTML responses: they are the only ones whose
/// body depends on the negotiation inputs.
const VARY_TOKENS: &[&str] = &["Accept-Language", "Cookie"];

/// Install the request language and advertise it on the responses that use it.
///
/// `Content-Language` and `Vary` are set on HTML only. `/api/v1` answers in
/// English whatever the request asked for, so tagging it with the negotiated
/// language would be a lie; and `/assets/*` is served with a one hour public
/// cache, so a per-request marker there would let a CDN hand one visitor's
/// language to another.
pub async fn locale(request: Request, next: Next) -> Response {
    let lang = i18n::resolve(request.headers());
    let mut response = i18n::scope(lang, next.run(request)).await;
    if is_html(response.headers()) {
        if let Ok(value) = HeaderValue::from_str(lang.code()) {
            response
                .headers_mut()
                .insert(header::CONTENT_LANGUAGE, value);
        }
        merge_vary(response.headers_mut(), VARY_TOKENS);
    }
    response
}

fn is_html(headers: &HeaderMap) -> bool {
    headers
        .get(header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .map(|value| value.starts_with("text/html"))
        .unwrap_or(false)
}

/// Append `tokens` to whatever `Vary` an inner layer already produced.
///
/// Overwriting would silently drop the inner layer's tokens: this middleware is
/// the outermost one and therefore runs last on the way out.
fn merge_vary(headers: &mut HeaderMap, tokens: &[&str]) {
    let mut values: Vec<String> = headers
        .get_all(header::VARY)
        .iter()
        .filter_map(|value| value.to_str().ok())
        .flat_map(|value| value.split(','))
        .map(|token| token.trim().to_string())
        .filter(|token| !token.is_empty())
        .collect();
    for token in tokens {
        if !values
            .iter()
            .any(|existing| existing.eq_ignore_ascii_case(token))
        {
            values.push((*token).to_string());
        }
    }
    let joined = values.join(", ");
    headers.remove(header::VARY);
    if let Ok(value) = HeaderValue::from_str(&joined) {
        headers.insert(header::VARY, value);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::Request as HttpRequest;
    use axum::routing::get;
    use axum::Router;
    use tower::ServiceExt;

    /// Echoes the task local language in the body: the header is built by the
    /// middleware from its own copy, so only the body proves the scope worked.
    async fn language() -> String {
        i18n::current().code().to_string()
    }

    async fn html() -> axum::response::Html<String> {
        axum::response::Html(format!("<p>{}</p>", i18n::current().code()))
    }

    async fn asset() -> impl axum::response::IntoResponse {
        (
            [(header::CONTENT_TYPE, "text/css; charset=utf-8")],
            "body {}",
        )
    }

    async fn json() -> axum::response::Json<serde_json::Value> {
        axum::response::Json(serde_json::json!({ "lang": i18n::current().code() }))
    }

    async fn body_text(response: Response) -> String {
        let bytes = axum::body::to_bytes(response.into_body(), 64 * 1024)
            .await
            .expect("body");
        String::from_utf8_lossy(&bytes).to_string()
    }

    fn app() -> Router {
        Router::new()
            .route("/plain", get(language))
            .route("/", get(html))
            .route("/app.css", get(asset))
            .route("/api.json", get(json))
            .layer(axum::middleware::from_fn(locale))
    }

    fn request(
        uri: &str,
        accept_language: Option<&str>,
        cookie: Option<&str>,
    ) -> HttpRequest<Body> {
        let mut builder = HttpRequest::builder().uri(uri);
        if let Some(value) = accept_language {
            builder = builder.header("accept-language", value);
        }
        if let Some(value) = cookie {
            builder = builder.header("cookie", value);
        }
        builder.body(Body::empty()).expect("request")
    }

    #[tokio::test]
    async fn installs_the_language_for_handlers() {
        let response = app()
            .oneshot(request("/plain", Some("zh-TW,zh;q=0.9"), None))
            .await
            .expect("response");
        assert_eq!(body_text(response).await, "zh-Hant");

        let response = app()
            .oneshot(request(
                "/plain",
                Some("en-US"),
                Some("uwurls_lang=zh-Hans"),
            ))
            .await
            .expect("response");
        assert_eq!(body_text(response).await, "zh-Hans");
    }

    #[tokio::test]
    async fn defaults_to_english() {
        let response = app()
            .oneshot(request("/plain", None, None))
            .await
            .expect("response");
        assert_eq!(body_text(response).await, "en");
    }

    #[tokio::test]
    async fn html_responses_carry_the_language_and_vary() {
        let response = app()
            .oneshot(request("/", Some("zh-Hant"), None))
            .await
            .expect("response");
        assert_eq!(response.headers()[header::CONTENT_LANGUAGE], "zh-Hant");
        assert_eq!(response.headers()[header::VARY], "Accept-Language, Cookie");
        assert!(body_text(response).await.contains("zh-Hant"));
    }

    #[tokio::test]
    async fn non_html_responses_stay_unmarked() {
        // Static assets are publicly cacheable: a per-request language marker
        // would let a shared cache serve the wrong one.
        let response = app()
            .oneshot(request("/app.css", Some("zh-TW"), None))
            .await
            .expect("response");
        assert!(response.headers().get(header::CONTENT_LANGUAGE).is_none());
        assert!(response.headers().get(header::VARY).is_none());

        // The JSON API always answers in English, so the negotiated language is
        // not a property of the response.
        let response = app()
            .oneshot(request("/api.json", Some("zh-TW"), None))
            .await
            .expect("response");
        assert!(response.headers().get(header::CONTENT_LANGUAGE).is_none());
        assert!(response.headers().get(header::VARY).is_none());
    }

    #[tokio::test]
    async fn vary_is_merged_with_inner_layers_and_deduplicated() {
        // A realistic inner layer that already owns a `Vary` value.
        async fn inner(request: Request, next: Next) -> Response {
            let mut response = next.run(request).await;
            response
                .headers_mut()
                .insert(header::VARY, HeaderValue::from_static("Origin, Cookie"));
            response
        }

        async fn html_page() -> axum::response::Html<&'static str> {
            axum::response::Html("<p>page</p>")
        }

        let app = Router::new()
            .route("/", get(html_page))
            .layer(axum::middleware::from_fn(inner))
            .layer(axum::middleware::from_fn(locale));

        let response = app
            .oneshot(request("/", Some("en"), None))
            .await
            .expect("response");
        // Inner tokens keep their order, `Cookie` is not repeated.
        assert_eq!(
            response.headers()[header::VARY],
            "Origin, Cookie, Accept-Language"
        );
    }
}
