//! Static assets embedded into the binary.
//!
//! Keeping CSS/JS inside the executable means the container image needs no
//! writable filesystem and no separate frontend build step.

pub const APP_CSS: &str = include_str!("assets/app.css");
pub const APP_JS: &str = include_str!("assets/app.js");
pub const THEME_JS: &str = include_str!("assets/theme.js");
pub const FAVICON: &str = include_str!("assets/favicon.svg");
