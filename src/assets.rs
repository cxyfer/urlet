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
pub const COSMOS_CSS: &str = include_str!("assets/themes/cosmos.css");
pub const FAVICON: &str = include_str!("assets/favicon.svg");

// The API reference. Scalar is vendored rather than pulled from a CDN: the
// strict `script-src 'self'` policy would refuse a remote bundle, and a
// self-hosted instance may have no outbound network at all. Refresh it with
// `scripts/update-scalar.sh`, which pins the version and the digest.
pub const SCALAR_JS: &str = include_str!("assets/scalar.js");
pub const SCALAR_INIT_JS: &str = include_str!("assets/scalar-init.js");

// The OpenAPI contracts are generated from `openapi/` by
// `scripts/build-openapi.py` and committed, because `include_str!` needs them at
// compile time. Never edit them by hand.
pub const OPENAPI_EN: &str = include_str!("../openapi/dist/urlet.en.json");
pub const OPENAPI_ZH_TW: &str = include_str!("../openapi/dist/urlet.zh-TW.json");
