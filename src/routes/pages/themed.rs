//! Compile-time theme dispatch for Askama pages.
//!
//! Askama resolves `{% extends %}` and `#[template(path = ...)]` at compile
//! time, so a page cannot choose its template from a runtime string. The
//! [`themed_page!`] macro solves that with one tiny wrapper struct per theme,
//! all dereferencing the same page context. Templates keep using `{{ field }}`
//! as before; field access auto-dereferences to the shared context.
//!
//! The request theme is read from the task local installed by
//! [`crate::middleware::theme`], so handlers only add `.themed()` to the page
//! value they already build.

/// A page context that can be wrapped in the selected theme.
///
/// Implemented by [`themed_page!`]; the admin render helper uses it so its
/// call sites do not have to append `.themed()` to every page value.
pub trait ThemedContext {
    type View: askama::Template;
    fn themed(self) -> Self::View;
}

/// Generate the per-theme template wrappers and the dispatch enum.
///
/// ```ignore
/// themed_page!(landing_templates, LandingView, LandingPage,
///     Verdant => "landing.html",
///     Aurora => "themes/aurora/landing.html",
/// );
/// ```
///
/// A theme may point at the base template when it does not override that page;
/// that is the fallback that lets a theme ship incrementally. The module name
/// is explicit because several pages live in the same Rust module.
#[macro_export]
macro_rules! themed_page {
    (
        $module:ident,
        $(#[$meta:meta])*
        $view:ident, $ctx:ty,
        $(
            $variant:ident => $path:literal
        ),+ $(,)?
    ) => {
        pub mod $module {
            use super::*;

            $(
                #[derive(askama::Template)]
                #[template(path = $path)]
                pub struct $variant {
                    pub ctx: $ctx,
                }

                impl std::ops::Deref for $variant {
                    type Target = $ctx;
                    fn deref(&self) -> &$ctx {
                        &self.ctx
                    }
                }

                #[allow(dead_code)]
                impl $variant {
                    /// Stylesheet of the request theme.
                    pub fn theme_css_url(&self) -> String {
                        $crate::services::theme::current_css_url()
                    }

                    /// Build version appended to embedded asset URLs.
                    ///
                    /// Browsers may serve a CSS file from cache for an hour
                    /// (`public, max-age=3600`), so an unversioned URL keeps
                    /// the old stylesheet after an upgrade. Tying the URL to
                    /// the release makes every deploy fetch fresh assets.
                    pub fn asset_version(&self) -> &'static str {
                        $crate::VERSION
                    }

                    /// Stable key, exposed as `data-theme-key` for CSS hooks.
                    pub fn theme_key(&self) -> String {
                        $crate::services::theme::current_key()
                    }

                    /// Whether the theme is dark-only (no light/dark control).
                    pub fn theme_is_dark_only(&self) -> bool {
                        $crate::services::theme::current_is_dark_only()
                    }

                    /// Whether the theme is light-only (no light/dark control).
                    pub fn theme_is_light_only(&self) -> bool {
                        $crate::services::theme::current_is_light_only()
                    }

                    /// Whether the light/dark control belongs on this page.
                    pub fn theme_allows_toggle(&self) -> bool {
                        $crate::services::theme::current_allows_toggle()
                    }

                    /// Server-side `data-theme-lock`: empty, `light` or `dark`.
                    pub fn theme_lock(&self) -> &'static str {
                        $crate::services::theme::current_theme_lock()
                    }

                    /// Whether the theme forces a palette.
                    pub fn theme_has_lock(&self) -> bool {
                        !$crate::services::theme::current_theme_lock().is_empty()
                    }
                }
            )+

            $(#[$meta])*
            pub enum $view {
                $($variant($variant)),+
            }

            impl $view {
                /// Build the selected theme's wrapper around `ctx`.
                #[allow(unreachable_patterns)]
                pub fn new(theme: $crate::services::theme::ThemeKey, ctx: $ctx) -> Self {
                    match theme {
                        $(
                            $crate::services::theme::ThemeKey::$variant => {
                                Self::$variant($variant { ctx })
                            }
                        )+
                        // A newly compiled theme that a page has not listed yet
                        // must not take the whole page down. Verdant is always
                        // present in every invocation and is the base template.
                        _ => Self::Verdant(Verdant { ctx }),
                    }
                }
            }

            impl std::fmt::Display for $view {
                fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                    match self {
                        $(Self::$variant(page) => page.fmt(f)),+
                    }
                }
            }

            impl askama::Template for $view {
                fn render_into(
                    &self,
                    writer: &mut (impl std::fmt::Write + ?Sized),
                ) -> askama::Result<()> {
                    match self {
                        $(Self::$variant(page) => page.render_into(writer)),+
                    }
                }

                const EXTENSION: Option<&'static str> = Some("html");
                const SIZE_HINT: usize = 0;
                const MIME_TYPE: &'static str = "text/html";
            }
        }

        impl $ctx {
            /// Wrap this page in the theme selected for the current request.
            pub fn themed(self) -> $module::$view {
                $module::$view::new($crate::services::theme::selected_key(), self)
            }
        }

        impl $crate::routes::pages::themed::ThemedContext for $ctx {
            type View = $module::$view;

            fn themed(self) -> Self::View {
                $module::$view::new($crate::services::theme::selected_key(), self)
            }
        }
    };
}
