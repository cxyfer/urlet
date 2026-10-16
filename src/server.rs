//! Router construction, bootstrap and the serving entry point.

use std::net::SocketAddr;

use axum::routing::get;
use axum::Router;
use tokio::net::TcpListener;

use crate::config::Config;
use crate::db;
use crate::db::users::NewUser;
use crate::error::{AppError, Result};
use crate::middleware;
use crate::routes;
use crate::services::settings::Settings;
use crate::services::{auth, password};
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
        .route("/", get(routes::pages::home))
        .nest("/app", routes::pages::router())
        .route("/{slug}", get(routes::redirect::redirect))
        .route("/{slug}/unlock", axum::routing::post(routes::redirect::unlock_link))
        .nest("/api/v1", routes::api::router())
        .fallback(routes::not_found)
        .layer(axum::middleware::from_fn_with_state(
            state.clone(),
            middleware::security::security_headers,
        ))
        .layer(axum::middleware::from_fn(middleware::request_id))
        .layer(axum::middleware::from_fn_with_state(
            state.clone(),
            middleware::csrf::csrf_protect,
        ))
        .with_state(state)
}

/// Idempotent start-up work: default domain, settings and bootstrap admin.
pub async fn bootstrap(state: &AppState) -> Result<()> {
    let host = state.config.base_host.clone();
    db::domains::ensure_default(&state.db, &host)
        .await
        .map_err(|err| AppError::internal(format!("cannot seed default domain: {err}")))?;
    db::settings::ensure_defaults(&state.db)
        .await
        .map_err(|err| AppError::internal(format!("cannot seed settings: {err}")))?;
    bootstrap_admin(state).await?;
    warn_about_first_user(state).await?;
    Ok(())
}

/// Create or promote the administrator configured through the environment.
async fn bootstrap_admin(state: &AppState) -> Result<()> {
    let (Some(email), Some(plain_password)) =
        (state.config.admin_email.clone(), state.config.admin_password.clone())
    else {
        return Ok(());
    };

    auth::validate_email(&email)?;
    let email_normalized = auth::normalize_email(&email);

    match db::users::find_by_email(&state.db, &email_normalized)
        .await
        .map_err(AppError::from)?
    {
        Some(existing) => {
            if !existing.is_admin() || existing.status != "active" {
                db::users::set_role_status(&state.db, existing.id, "admin", "active")
                    .await
                    .map_err(AppError::from)?;
                tracing::info!(user_id = existing.id, "bootstrap account promoted to administrator");
            }
        }
        None => {
            let password_hash = password::hash(&plain_password)?;
            let user_id = db::users::create(
                &state.db,
                &NewUser {
                    email: email.clone(),
                    email_normalized,
                    password_hash,
                    display_name: Some("Administrator".to_string()),
                    role: "admin".to_string(),
                },
            )
            .await
            .map_err(AppError::from)?;
            tracing::info!(user_id, email = %email, "bootstrap administrator created");
        }
    }
    Ok(())
}

/// Point out the obvious foot-gun: a closed instance with no accounts.
async fn warn_about_first_user(state: &AppState) -> Result<()> {
    let users = db::users::count(&state.db).await.map_err(AppError::from)?;
    if users == 0 && !state.settings.registration_mode().allows_public_registration() {
        tracing::warn!(
            mode = state.settings.registration_mode().as_str(),
            "no accounts exist and registration is not open; set UWURLS_ADMIN_EMAIL and \
             UWURLS_ADMIN_PASSWORD or open registration"
        );
    }
    Ok(())
}

/// Periodic housekeeping: stale sessions, expired attempts and retention.
fn spawn_maintenance(state: AppState) {
    tokio::spawn(async move {
        let mut ticker = tokio::time::interval(std::time::Duration::from_secs(6 * 3600));
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            ticker.tick().await;
            if let Err(err) =
                crate::services::admin::run_maintenance(&state.db, &state.settings).await
            {
                tracing::warn!(error = %err, "maintenance pass failed");
            }
        }
    });
}

/// Run migrations, bootstrap and serve until SIGINT/SIGTERM.
pub async fn serve(config: Config) -> Result<()> {
    let pool = db::init(&config).await?;
    let settings = Settings::load(&pool, &config).await?;
    let bind = config.bind;
    let base_url = config.base_url.clone();
    let state = AppState::new(config, pool, settings);
    bootstrap(&state).await?;
    spawn_maintenance(state.clone());

    let listener = TcpListener::bind(bind)
        .await
        .map_err(|err| AppError::internal(format!("cannot bind {bind}: {err}")))?;
    let local = listener
        .local_addr()
        .map_err(|err| AppError::internal(format!("cannot read local addr: {err}")))?;

    tracing::info!(
        version = crate::VERSION,
        %local,
        %base_url,
        registration = state.settings.registration_mode().as_str(),
        "uwurls is listening"
    );

    axum::serve(listener, build_router(state).into_make_service_with_connect_info::<SocketAddr>())
        .with_graceful_shutdown(shutdown_signal())
        .await
        .map_err(|err| AppError::internal(format!("server error: {err}")))?;

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
