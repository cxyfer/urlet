//! Locale middleware.
//!
//! Resolves the language of every request once and installs it as the task local
//! language for the rest of the stack, so handlers, templates and
//! `AppError::into_response` all render in the same language.

use axum::extract::Request;
use axum::http::{header, HeaderValue};
use axum::middleware::Next;
use axum::response::Response;

use crate::i18n;

/// Install the request language and advertise it with `Content-Language`.
pub async fn locale(request: Request, next: Next) -> Response {
    let lang = i18n::resolve(request.headers());
    let mut response = i18n::scope(lang, next.run(request)).await;
    if let Ok(value) = HeaderValue::from_str(lang.code()) {
        response
            .headers_mut()
            .insert(header::CONTENT_LANGUAGE, value);
    }
    response
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::Request as HttpRequest;
    use axum::routing::get;
    use axum::Router;
    use tower::ServiceExt;

    async fn language() -> String {
        i18n::current().code().to_string()
    }

    #[tokio::test]
    async fn installs_the_language_for_handlers() {
        let app = Router::new()
            .route("/", get(language))
            .layer(axum::middleware::from_fn(locale));

        let response = app
            .clone()
            .oneshot(
                HttpRequest::builder()
                    .uri("/")
                    .header("accept-language", "zh-TW,zh;q=0.9")
                    .body(Body::empty())
                    .expect("request"),
            )
            .await
            .expect("response");
        assert_eq!(response.headers()["content-language"], "zh-Hant");

        let response = app
            .oneshot(
                HttpRequest::builder()
                    .uri("/")
                    .header("cookie", "uwurls_lang=zh-Hans")
                    .header("accept-language", "en-US")
                    .body(Body::empty())
                    .expect("request"),
            )
            .await
            .expect("response");
        assert_eq!(response.headers()["content-language"], "zh-Hans");
    }

    #[tokio::test]
    async fn defaults_to_english() {
        let app = Router::new()
            .route("/", get(language))
            .layer(axum::middleware::from_fn(locale));
        let response = app
            .oneshot(
                HttpRequest::builder()
                    .uri("/")
                    .body(Body::empty())
                    .expect("request"),
            )
            .await
            .expect("response");
        assert_eq!(response.headers()["content-language"], "en");
    }
}
