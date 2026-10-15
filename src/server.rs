//! Router construction, bootstrap and the serving entry point.

use std::net::SocketAddr;

use axum::routing::get;
use axum::Router;
use tokio::net::TcpListener;

use crate::config::Config;
use crate::db;
use crate::error::Result;
use crate::middleware;
use crate::routes;
use crate::state::AppState;

/// Build the complete application router. Used by the server and by tests.
pub fn build_router(state: AppState) -> Router {
    Router::new()
        .route("/health", get(routes::health::health))
        .route("/healthz", get(routes::health::health))
        .route("/assets/app.css", get(routes::assets::css))
        .route("/assets/app.js", get(routes::assets::js))
        .route("/assets/favicon.svg", get(routes::assets::favicon))
        .route("/favicon.ico", get(routes::assets::favicon_ico))
        .fallback(routes::not_found)
        .layer(axum::middleware::from_fn_with_state(
            state.clone(),
            middleware::security::security_headers,
        ))
        .layer(axum::middleware::from_fn(middleware::request_id))
        .with_state(state)
}

/// Idempotent start-up work: register the configured default domain.
pub async fn bootstrap(state: &AppState) -> Result<()> {
    let host = state.config.base_host.clone();
    db::domains::ensure_default(&state.db, &host)
        .await
        .map_err(|err| crate::error::AppError::internal(format!("cannot seed default domain: {err}")))?;
    Ok(())
}

/// Run migrations, bootstrap and serve until SIGINT/SIGTERM.
pub async fn serve(config: Config) -> Result<()> {
    let pool = db::init(&config).await?;
    let bind = config.bind;
    let base_url = config.base_url.clone();
    let state = AppState::new(config, pool);
    bootstrap(&state).await?;

    let listener = TcpListener::bind(bind)
        .await
        .map_err(|err| crate::error::AppError::internal(format!("cannot bind {bind}: {err}")))?;
    let local = listener
        .local_addr()
        .map_err(|err| crate::error::AppError::internal(format!("cannot read local addr: {err}")))?;

    tracing::info!(version = crate::VERSION, %local, %base_url, "uwurls is listening");

    axum::serve(listener, build_router(state).into_make_service_with_connect_info::<SocketAddr>())
        .with_graceful_shutdown(shutdown_signal())
        .await
        .map_err(|err| crate::error::AppError::internal(format!("server error: {err}")))?;

    tracing::info!("shutdown complete");
    Ok(())
}

async fn shutdown_signal() {
    let ctrl_c = async {
        let _ = tokio::signal::ctrl_c().await;
    };

    #[cfg(unix)]
    let terminate = async {
        match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
            Ok(mut signal) => {
                signal.recv().await;
            }
            Err(_) => std::future::pending::<()>().await,
        }
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        _ = ctrl_c => {}
        _ = terminate => {}
    }
    tracing::info!("shutdown signal received");
}
