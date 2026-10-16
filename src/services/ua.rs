//! Very small user agent parser.
//!
//! The goal is a useful device/browser/OS breakdown for a self hosted
//! dashboard, not full UA fidelity — that is why this is a few hundred lines
//! shorter than a dedicated crate.

/// Parsed user agent.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UserAgentInfo {
    pub device_type: &'static str,
    pub browser: &'static str,
    pub os: &'static str,
    pub is_bot: bool,
}

impl Default for UserAgentInfo {
    fn default() -> Self {
        Self {
            device_type: "Unknown",
            browser: "Other",
            os: "Unknown",
            is_bot: false,
        }
    }
}

/// Substrings that identify automated traffic.
const BOT_MARKERS: &[&str] = &[
    "bot",
    "crawler",
    "spider",
    "crawl",
    "slurp",
    "curl/",
    "wget",
    "python-requests",
    "python-urllib",
    "go-http-client",
    "java/",
    "okhttp",
    "axios",
    "node-fetch",
    "headlesschrome",
    "phantomjs",
    "monitoring",
    "uptime",
    "pingdom",
    "statuscake",
    "facebookexternalhit",
    "whatsapp",
    "telegrambot",
    "slackbot",
    "discordbot",
    "twitterbot",
    "linkedinbot",
    "embedly",
    "preview",
    "scanner",
    "nmap",
    "masscan",
];

/// Parse a user agent string. Missing agents are treated as `Unknown`.
pub fn parse(user_agent: Option<&str>) -> UserAgentInfo {
    let Some(raw) = user_agent.map(str::trim).filter(|value| !value.is_empty()) else {
        return UserAgentInfo {
            device_type: "Unknown",
            browser: "Other",
            os: "Unknown",
            is_bot: false,
        };
    };
    let lower = raw.to_ascii_lowercase();
    let is_bot = BOT_MARKERS.iter().any(|marker| lower.contains(marker));

    let os = detect_os(&lower);
    let browser = detect_browser(&lower);
    let device_type = detect_device(&lower, is_bot);

    UserAgentInfo {
        device_type,
        browser,
        os,
        is_bot,
    }
}

fn detect_os(lower: &str) -> &'static str {
    if lower.contains("windows") {
        "Windows"
    } else if lower.contains("android") {
        "Android"
    } else if lower.contains("iphone") || lower.contains("ipad") || lower.contains("ipod") {
        "iOS"
    } else if lower.contains("mac os x") || lower.contains("macintosh") {
        "macOS"
    } else if lower.contains("cros") {
        "ChromeOS"
    } else if lower.contains("linux") || lower.contains("x11") {
        "Linux"
    } else if lower.contains("freebsd") || lower.contains("openbsd") {
        "BSD"
    } else {
        "Unknown"
    }
}

fn detect_browser(lower: &str) -> &'static str {
    // Order matters: Edge and Opera also advertise Chrome, Chrome also Safari.
    if lower.contains("edg/") || lower.contains("edge/") || lower.contains("edgios") {
        "Edge"
    } else if lower.contains("opr/") || lower.contains("opera") || lower.contains("opt/") {
        "Opera"
    } else if lower.contains("samsungbrowser") {
        "Samsung Internet"
    } else if lower.contains("firefox") || lower.contains("fxios") {
        "Firefox"
    } else if lower.contains("chrome") || lower.contains("crios") {
        "Chrome"
    } else if lower.contains("safari") {
        "Safari"
    } else if lower.contains("msie") || lower.contains("trident") {
        "Internet Explorer"
    } else {
        "Other"
    }
}

fn detect_device(lower: &str, is_bot: bool) -> &'static str {
    if is_bot {
        return "Bot";
    }
    if lower.contains("ipad")
        || lower.contains("tablet")
        || lower.contains("kindle")
        || lower.contains("silk")
        || (lower.contains("android") && !lower.contains("mobile"))
    {
        "Tablet"
    } else if lower.contains("mobi")
        || lower.contains("iphone")
        || lower.contains("ipod")
        || lower.contains("android")
        || lower.contains("windows phone")
    {
        "Mobile"
    } else {
        "Desktop"
    }
}

/// Referrer host, ignoring referrals from the shortener itself.
pub fn referrer_host(referrer: Option<&str>, own_hosts: &[&str]) -> Option<String> {
    let referrer = referrer?.trim();
    if referrer.is_empty() || referrer.eq_ignore_ascii_case("null") {
        return None;
    }
    let rest = referrer
        .split_once("://")
        .map(|(_, rest)| rest)
        .unwrap_or(referrer);
    let authority = rest.split(['/', '?', '#']).next()?;
    let host = authority
        .rsplit('@')
        .next()?
        .split(':')
        .next()?
        .to_ascii_lowercase();
    if host.is_empty() {
        return None;
    }
    let host = host.strip_prefix("www.").unwrap_or(&host).to_string();
    if own_hosts.iter().any(|own| {
        let own = own.split(':').next().unwrap_or(own).to_ascii_lowercase();
        own == host || own.strip_prefix("www.") == Some(host.as_str())
    }) {
        return None;
    }
    Some(host)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_desktop_chrome() {
        let info = parse(Some(
            "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) \
             Chrome/120.0.0.0 Safari/537.36",
        ));
        assert_eq!(info.device_type, "Desktop");
        assert_eq!(info.browser, "Chrome");
        assert_eq!(info.os, "Windows");
        assert!(!info.is_bot);
    }

    #[test]
    fn parses_ios_safari() {
        let info = parse(Some(
            "Mozilla/5.0 (iPhone; CPU iPhone OS 17_0 like Mac OS X) AppleWebKit/605.1.15 \
             (KHTML, like Gecko) Version/17.0 Mobile/15E148 Safari/604.1",
        ));
        assert_eq!(info.device_type, "Mobile");
        assert_eq!(info.browser, "Safari");
        assert_eq!(info.os, "iOS");
    }

    #[test]
    fn edge_is_not_reported_as_chrome() {
        let info = parse(Some(
            "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) \
             Chrome/120.0.0.0 Safari/537.36 Edg/120.0.0.0",
        ));
        assert_eq!(info.browser, "Edge");
    }

    #[test]
    fn detects_bots_and_scripts() {
        for agent in [
            "Googlebot/2.1 (+http://www.google.com/bot.html)",
            "curl/8.4.0",
            "python-requests/2.31.0",
            "Mozilla/5.0 (compatible; Bingbot/2.0)",
        ] {
            let info = parse(Some(agent));
            assert!(info.is_bot, "{agent} should be a bot");
            assert_eq!(info.device_type, "Bot");
        }
    }

    #[test]
    fn handles_missing_agents() {
        let info = parse(None);
        assert_eq!(info, UserAgentInfo::default());
        assert_eq!(parse(Some("   ")), UserAgentInfo::default());
    }

    #[test]
    fn extracts_referrer_hosts_and_drops_self_referrals() {
        assert_eq!(
            referrer_host(Some("https://news.example.com/article"), &["s.example.com"]),
            Some("news.example.com".to_string())
        );
        assert_eq!(
            referrer_host(Some("https://www.s.example.com/x"), &["s.example.com"]),
            None
        );
        assert_eq!(
            referrer_host(Some("https://s.example.com:8443/x"), &["s.example.com"]),
            None
        );
        assert_eq!(referrer_host(None, &[]), None);
        assert_eq!(referrer_host(Some(""), &[]), None);
    }
}
