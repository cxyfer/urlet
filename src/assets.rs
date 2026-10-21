//! Static assets embedded into the binary.
//!
//! Keeping CSS/JS inside the executable means the container image needs no
//! writable filesystem and no separate frontend build step.

pub const APP_CSS: &str = include_str!("assets/app.css");
pub const APP_JS: &str = include_str!("assets/app.js");
pub const THEME_JS: &str = include_str!("assets/theme.js");
pub const THEME_UI_CSS: &str = include_str!("assets/theme-ui.css");
pub const AURORA_CSS: &str = include_str!("assets/themes/aurora.css");
pub const PAPER_CSS: &str = include_str!("assets/themes/paper.css");
pub const SIGNAL_CSS: &str = include_str!("assets/themes/signal.css");
pub const GILDED_CSS: &str = include_str!("assets/themes/gilded.css");
pub const SWISS_CSS: &str = include_str!("assets/themes/swiss.css");
pub const FAVICON: &str = include_str!("assets/favicon.svg");
