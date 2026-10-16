//! Health endpoint used by docker health checks and uptime probes.

use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde::Serialize;

use crate::state::AppState;
use crate::VERSION;

#[derive(Debug, Serialize)]
pub struct Health {
    pub status: &'static str,
    pub version: &'static str,
    pub database: &'static str,
    pub uptime_seconds: i64,
}

pub async fn health(State(state): State<AppState>) -> Response {
    let database_ok = sqlx::query_scalar::<_, i64>("SELECT 1")
        .fetch_one(&state.db)
        .await
        .is_ok();
    let body = Health {
        status: if database_ok { "ok" } else { "degraded" },
        version: VERSION,
        database: if database_ok { "ok" } else { "unavailable" },
        uptime_seconds: state.uptime_seconds(),
    };
    let status = if database_ok {
        StatusCode::OK
    } else {
        StatusCode::SERVICE_UNAVAILABLE
    };
    (status, Json(body)).into_response()
}
