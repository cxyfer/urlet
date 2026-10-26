//! urlet — lightweight multi-user URL shortener.
//!
//! The crate is split into a library (everything testable) and a thin binary
//! (`src/main.rs`) that only handles process concerns: CLI parsing, logging
//! setup and signal handling.

pub mod assets;
pub mod config;
pub mod db;
pub mod error;
pub mod i18n;
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

/// Upstream repository of this project, linked from the footer of every design.
///
/// The value comes from the `repository` field of `Cargo.toml`, so the link in
/// the UI and the published package metadata cannot drift apart. A fork points
/// both at its own URL by editing that one field; the test below fails loudly
/// if the field is missing, because cargo would then expand it to an empty
/// string and every footer would render a dead link.
pub const REPOSITORY: &str = env!("CARGO_PKG_REPOSITORY");

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_package_names_a_repository_the_footer_can_link() {
        assert!(
            REPOSITORY.starts_with("https://"),
            "Cargo.toml needs an https repository: {REPOSITORY:?}"
        );
        assert!(
            !REPOSITORY.ends_with('/'),
            "drop the trailing slash so paths can be appended: {REPOSITORY:?}"
        );
    }
}
