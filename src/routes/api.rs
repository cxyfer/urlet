//! `/api/v1` router.

use axum::routing::{delete, get, post};
use axum::Router;

use crate::routes::{api_auth, api_not_found};
use crate::state::AppState;

/// JSON API, version 1.
pub fn router() -> Router<AppState> {
    Router::new()
        .route("/auth/register", post(api_auth::register))
        .route("/auth/login", post(api_auth::login))
        .route("/auth/logout", post(api_auth::logout))
        .route("/auth/me", get(api_auth::me).patch(api_auth::update_me))
        .route("/auth/password", post(api_auth::change_password))
        .route("/auth/sessions", get(api_auth::list_sessions))
        .route("/auth/sessions/{id}", delete(api_auth::revoke_session))
        .fallback(api_not_found)
}
