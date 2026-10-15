//! uwurls — lightweight multi-user URL shortener.
//!
//! The crate is split into a library (everything testable) and a thin binary
//! (`src/main.rs`) that only handles process concerns: CLI parsing, logging
//! setup and signal handling.

pub mod assets;
pub mod config;
pub mod db;
pub mod error;
pub mod middleware;
pub mod routes;
pub mod server;
pub mod services;
pub mod state;

pub use config::Config;
pub use error::{ApiError, AppError};
pub use state::AppState;

/// Semantic version of the running binary.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
