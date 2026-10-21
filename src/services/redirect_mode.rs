//! Redirect modes and per-group redirect-mode policy.
//!
//! A link either redirects immediately (`direct`, the historical behaviour) or
//! renders a splash/interstitial page first (`splash`). The splash page carries
//! optional promotional copy and a countdown, and then sends the visitor on.
//!
//! A user group may narrow what its members can pick: `direct` only, `splash`
//! only, or `choice` (either mode). The policy gates creation and switching the
//! mode on update; it never rewrites existing links, matching the other group
//! feature toggles (FR-GROUP-3/FR-GROUP-7).

use crate::error::{AppError, Result};

/// How a short link hands the visitor to its target.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum RedirectMode {
    /// Send a redirect response immediately.
    #[default]
    Direct,
    /// Render a splash page first, then go to the target.
    Splash,
}

impl RedirectMode {
    /// Every accepted value, in form order.
    pub const ALL: [Self; 2] = [Self::Direct, Self::Splash];

    pub fn parse(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "direct" => Some(Self::Direct),
            "splash" => Some(Self::Splash),
            _ => None,
        }
    }

    /// Parse a stored column value. An unknown value falls back to `direct`, so
    /// a manually edited database row can never break the redirect path.
    pub fn from_db(value: &str) -> Self {
        Self::parse(value).unwrap_or(Self::Direct)
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Direct => "direct",
            Self::Splash => "splash",
        }
    }

    pub fn is_splash(self) -> bool {
        self == Self::Splash
    }

    /// Catalogue key of the short label used in the UI.
    pub fn label_key(self) -> &'static str {
        match self {
            Self::Direct => "url.mode.direct",
            Self::Splash => "url.mode.splash",
        }
    }
}

/// Which redirect modes a group may use for new links.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum RedirectModePolicy {
    /// Only immediate redirects.
    Direct,
    /// Only the splash page.
    Splash,
    /// The member chooses per link.
    #[default]
    Choice,
}

impl RedirectModePolicy {
    pub const ALL: [Self; 3] = [Self::Choice, Self::Direct, Self::Splash];

    pub fn parse(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "direct" => Some(Self::Direct),
            "splash" => Some(Self::Splash),
            "choice" => Some(Self::Choice),
            _ => None,
        }
    }

    /// Parse a stored column value; an unknown value means `choice`, which
    /// preserves the permissive historical behaviour.
    pub fn from_db(value: &str) -> Self {
        Self::parse(value).unwrap_or(Self::Choice)
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Direct => "direct",
            Self::Splash => "splash",
            Self::Choice => "choice",
        }
    }

    /// Whether `mode` may be selected under this policy.
    pub fn allows(self, mode: RedirectMode) -> bool {
        match self {
            Self::Choice => true,
            Self::Direct => mode == RedirectMode::Direct,
            Self::Splash => mode == RedirectMode::Splash,
        }
    }

    /// Mode a link gets when the create request omits `redirect_mode`.
    ///
    /// `choice` keeps the historical default (`direct`); a locked policy picks
    /// its only allowed mode instead of forcing the form to send a value.
    pub fn default_mode(self) -> RedirectMode {
        match self {
            Self::Splash => RedirectMode::Splash,
            Self::Direct | Self::Choice => RedirectMode::Direct,
        }
    }

    /// Catalogue key of the selector label.
    pub fn label_key(self) -> &'static str {
        match self {
            Self::Direct => "admin.groups.redirect_mode.direct",
            Self::Splash => "admin.groups.redirect_mode.splash",
            Self::Choice => "admin.groups.redirect_mode.choice",
        }
    }
}

/// Shortest accepted splash countdown.
pub const MIN_DELAY: i64 = 1;
/// Longest accepted splash countdown.
pub const MAX_DELAY: i64 = 60;
/// Default countdown when the client does not send one.
pub const DEFAULT_DELAY: i64 = 5;
/// Maximum length of the custom splash heading.
pub const MAX_TITLE_LEN: usize = 120;
/// Maximum length of the custom splash message (newlines allowed).
pub const MAX_MESSAGE_LEN: usize = 500;

/// Validate the splash countdown.
pub fn validate_delay(value: i64) -> Result<i64> {
    if !(MIN_DELAY..=MAX_DELAY).contains(&value) {
        return Err(AppError::validation(
            "splash_delay_seconds",
            format!("the splash delay must be between {MIN_DELAY} and {MAX_DELAY} seconds"),
        ));
    }
    Ok(value)
}

/// Validate the optional splash heading. Blank means "use the default copy".
pub fn validate_title(raw: &str) -> Result<Option<String>> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Ok(None);
    }
    if trimmed.chars().count() > MAX_TITLE_LEN {
        return Err(AppError::validation(
            "splash_title",
            format!("the splash title must be at most {MAX_TITLE_LEN} characters"),
        ));
    }
    if trimmed.chars().any(|c| c.is_control()) {
        return Err(AppError::validation(
            "splash_title",
            "the splash title must not contain control characters",
        ));
    }
    Ok(Some(trimmed.to_string()))
}

/// Validate the optional splash message.
///
/// Newlines are allowed so a promotion can use short paragraphs; every other
/// control character is refused. Empty input clears the field.
pub fn validate_message(raw: &str) -> Result<Option<String>> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Ok(None);
    }
    if trimmed.chars().count() > MAX_MESSAGE_LEN {
        return Err(AppError::validation(
            "splash_message",
            format!("the splash message must be at most {MAX_MESSAGE_LEN} characters"),
        ));
    }
    if trimmed
        .chars()
        .any(|c| c.is_control() && !matches!(c, '\n' | '\r'))
    {
        return Err(AppError::validation(
            "splash_message",
            "the splash message must not contain control characters",
        ));
    }
    // Normalise CRLF so the stored value and the rendered text agree.
    Ok(Some(trimmed.replace("\r\n", "\n").replace('\r', "\n")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_modes_case_insensitively_and_falls_back_on_read() {
        assert_eq!(RedirectMode::parse(" DIRECT "), Some(RedirectMode::Direct));
        assert_eq!(RedirectMode::parse("Splash"), Some(RedirectMode::Splash));
        assert_eq!(RedirectMode::parse("splash-page"), None);
        assert_eq!(RedirectMode::from_db("nonsense"), RedirectMode::Direct);
        assert_eq!(RedirectMode::Splash.as_str(), "splash");
    }

    #[test]
    fn policies_only_allow_their_mode() {
        assert!(RedirectModePolicy::Direct.allows(RedirectMode::Direct));
        assert!(!RedirectModePolicy::Direct.allows(RedirectMode::Splash));
        assert!(RedirectModePolicy::Splash.allows(RedirectMode::Splash));
        assert!(!RedirectModePolicy::Splash.allows(RedirectMode::Direct));
        for mode in RedirectMode::ALL {
            assert!(RedirectModePolicy::Choice.allows(mode));
        }
    }

    #[test]
    fn default_mode_follows_the_policy() {
        assert_eq!(
            RedirectModePolicy::Direct.default_mode(),
            RedirectMode::Direct
        );
        assert_eq!(
            RedirectModePolicy::Splash.default_mode(),
            RedirectMode::Splash
        );
        assert_eq!(
            RedirectModePolicy::Choice.default_mode(),
            RedirectMode::Direct
        );
    }

    #[test]
    fn parses_policies_and_falls_back_to_choice() {
        assert_eq!(
            RedirectModePolicy::parse(" direct "),
            Some(RedirectModePolicy::Direct)
        );
        assert_eq!(
            RedirectModePolicy::parse("SPLASH"),
            Some(RedirectModePolicy::Splash)
        );
        assert_eq!(
            RedirectModePolicy::parse("choice"),
            Some(RedirectModePolicy::Choice)
        );
        assert_eq!(RedirectModePolicy::parse("nonsense"), None);
        assert_eq!(
            RedirectModePolicy::from_db("nonsense"),
            RedirectModePolicy::Choice
        );
    }

    #[test]
    fn validates_the_delay_range() {
        assert_eq!(validate_delay(1).expect("valid"), 1);
        assert_eq!(validate_delay(60).expect("valid"), 60);
        assert!(validate_delay(0).is_err());
        assert!(validate_delay(61).is_err());
    }

    #[test]
    fn validates_and_clears_splash_copy() {
        assert_eq!(validate_title("  ").expect("blank"), None);
        assert_eq!(
            validate_title("  Summer sale  ").expect("valid"),
            Some("Summer sale".to_string())
        );
        assert!(validate_title(&"x".repeat(MAX_TITLE_LEN + 1)).is_err());
        assert!(validate_title("two\nlines").is_err());

        assert_eq!(validate_message("  ").expect("blank"), None);
        assert_eq!(
            validate_message("First line\r\nSecond line").expect("valid"),
            Some("First line\nSecond line".to_string())
        );
        assert!(validate_message(&"x".repeat(MAX_MESSAGE_LEN + 1)).is_err());
        assert!(validate_message("bad\u{7}text").is_err());
    }

    #[test]
    fn every_mode_and_policy_label_exists_in_every_catalogue() {
        for lang in crate::i18n::ALL {
            for mode in RedirectMode::ALL {
                let key = mode.label_key();
                assert_ne!(lang.t(key), key, "{} is missing {key}", lang.code());
            }
            for policy in RedirectModePolicy::ALL {
                let key = policy.label_key();
                assert_ne!(lang.t(key), key, "{} is missing {key}", lang.code());
            }
        }
    }
}
