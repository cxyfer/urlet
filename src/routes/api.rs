//! `/api/v1` router.

use axum::routing::{delete, get, patch, post, put};
use axum::Router;

use crate::routes::{
    api_admin, api_auth, api_domains, api_invites, api_not_found, api_public, api_stats, api_tags,
    api_tokens, api_urls, public_urls,
};
use crate::state::AppState;

/// JSON API, version 1.
pub fn router() -> Router<AppState> {
    Router::new()
        // authentication and profile
        .route("/auth/register", post(api_auth::register))
        .route("/auth/login", post(api_auth::login))
        .route("/auth/logout", post(api_auth::logout))
        .route("/auth/me", get(api_auth::me).patch(api_auth::update_me))
        .route("/auth/password", post(api_auth::change_password))
        .route("/auth/sessions", get(api_auth::list_sessions))
        .route("/auth/sessions/{id}", delete(api_auth::revoke_session))
        // short links
        .route("/urls", get(api_urls::list).post(api_urls::create))
        .route(
            "/urls/{id}",
            get(api_urls::get)
                .patch(api_urls::update)
                .delete(api_urls::delete),
        )
        .route("/urls/{id}/stats", get(api_stats::stats))
        .route("/urls/{id}/clicks", get(api_stats::clicks_log))
        // active domains available to the signed-in user
        .route("/domains", get(api_domains::list))
        // the signed-in member's own invite codes
        .route("/invites", get(api_invites::list).post(api_invites::create))
        .route("/invites/{id}", delete(api_invites::revoke))
        // tags
        .route("/tags", get(api_tags::list))
        .route("/tags/{id}", delete(api_tags::delete))
        // public metadata
        .route("/public/urls/{slug}", get(api_public::show))
        // the public shortener (unauthenticated when the access level allows it)
        .route("/public/urls", post(public_urls::create_api))
        // api tokens
        .route("/tokens", get(api_tokens::list).post(api_tokens::create))
        .route("/tokens/{id}", delete(api_tokens::revoke))
        // administration
        .route(
            "/admin/domains",
            get(api_admin::list_domains).post(api_admin::create_domain),
        )
        .route(
            "/admin/domains/{id}",
            patch(api_admin::update_domain).delete(api_admin::delete_domain),
        )
        .route("/admin/users", get(api_admin::list_users))
        .route(
            "/admin/users/{id}",
            patch(api_admin::update_user).delete(api_admin::delete_user),
        )
        .route(
            "/admin/users/{id}/password",
            post(api_admin::reset_password),
        )
        .route("/admin/urls", get(api_admin::list_urls))
        .route(
            "/admin/urls/{id}",
            patch(api_admin::update_url).delete(api_admin::delete_url),
        )
        .route(
            "/admin/invites",
            get(api_admin::list_invites).post(api_admin::create_invite),
        )
        .route("/admin/invites/{id}", delete(api_admin::revoke_invite))
        .route(
            "/admin/groups",
            get(api_admin::list_groups).post(api_admin::create_group),
        )
        .route(
            "/admin/groups/{id}",
            put(api_admin::update_group).delete(api_admin::delete_group),
        )
        .route(
            "/admin/themes",
            get(api_admin::get_themes).put(api_admin::put_themes),
        )
        .route(
            "/admin/settings",
            get(api_admin::get_settings).put(api_admin::put_settings),
        )
        .route("/admin/audit", get(api_admin::audit))
        .route("/admin/stats", get(api_admin::stats))
        .fallback(api_not_found)
}
