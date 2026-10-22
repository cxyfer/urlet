//! 訪客目標白名單：精確主機或萬用子網域與選填的路徑前綴，不抓取目標內容。

use crate::services::domains;

pub const DEFAULT_RULES: &str = include_str!("guest_allowlist.txt");
pub const MAX_RULES: usize = 200;
pub const MAX_BYTES: usize = 16_384;
pub const BLOCKED: &str = "guests may only shorten destinations on the allowed list";

struct Rule {
    host: String,
    subdomains_only: bool,
    path: String,
}

impl Rule {
    fn parse(raw: &str) -> Option<Self> {
        // 只接受 hostname[/path] 或 *.hostname[/path]，星號限用於開頭的子網域標記。
        if raw.is_empty()
            || !raw.is_ascii()
            || raw.chars().any(|c| c.is_control() || c.is_whitespace())
            || raw.contains([':', '@', '?', '#', '\\', '%'])
        {
            return None;
        }
        let (host, path) = match raw.find('/') {
            Some(index) => (&raw[..index], &raw[index..]),
            None => (raw, ""),
        };
        let subdomains_only = host.starts_with("*.");
        let host = host.strip_prefix("*.").unwrap_or(host);
        let host = domains::validate_domain_hostname(host).ok()?;
        if !host.contains('.') || host.parse::<std::net::IpAddr>().is_ok() {
            return None;
        }
        if !path
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || b"/-._~".contains(&c))
            || path.contains("//")
            || path.split('/').any(|part| matches!(part, "." | ".."))
        {
            return None;
        }
        Some(Self {
            host,
            subdomains_only,
            path: if path == "/" { "" } else { path }.to_string(),
        })
    }

    fn matches(&self, url: &url::Url) -> bool {
        let Some(host) = url.host_str().map(|host| host.trim_end_matches('.')) else {
            return false;
        };
        let host_matches = if self.subdomains_only {
            host.strip_suffix(&self.host)
                .and_then(|prefix| prefix.strip_suffix('.'))
                .is_some_and(|subdomain| !subdomain.is_empty())
        } else {
            host == self.host
        };
        if !host_matches {
            return false;
        }
        if self.path.is_empty() {
            return true;
        }
        let path = url.path();
        let lowered = path.to_ascii_lowercase();
        // 拒絕伺服器再次解碼後可能改變路徑區段的分隔符與巢狀編碼。
        if path.contains("//")
            || ["%2f", "%5c", "%25"]
                .iter()
                .any(|part| lowered.contains(part))
        {
            return false;
        }
        path == self.path
            || path
                .strip_prefix(&self.path)
                .is_some_and(|rest| self.path.ends_with('/') || rest.starts_with('/'))
    }
}

/// 驗證並去除空行、重複項目；空字串代表刻意拒絕所有目標。
pub fn normalize(raw: &str) -> Result<String, &'static str> {
    if raw.len() > MAX_BYTES {
        return Err("the guest allowlist must be at most 16384 bytes");
    }
    let mut rules = Vec::new();
    for line in raw.lines().map(str::trim).filter(|line| !line.is_empty()) {
        let rule = Rule::parse(line).ok_or(
            "use one hostname per line with an optional leading *. and ASCII path (no scheme, port, other wildcard, query or fragment)",
        )?;
        let wildcard = if rule.subdomains_only { "*." } else { "" };
        let rule = format!("{wildcard}{}{}", rule.host, rule.path);
        if !rules.contains(&rule) {
            rules.push(rule);
        }
        if rules.len() > MAX_RULES {
            return Err("the guest allowlist must contain at most 200 rules");
        }
    }
    Ok(rules.join("\n"))
}

/// `target` 必須先通過一般目標網址驗證。未命中、解析失敗或不明確的輸入皆拒絕。
pub fn allows(rules: &str, target: &str) -> bool {
    let Ok(url) = url::Url::parse(target) else {
        return false;
    };
    if !matches!(url.scheme(), "http" | "https")
        || !url.username().is_empty()
        || url.password().is_some()
        || url.port().is_some()
    {
        return false;
    }
    rules
        .lines()
        .filter_map(Rule::parse)
        .any(|rule| rule.matches(&url))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalizes_only_hosts_and_preserves_case_sensitive_paths() {
        assert_eq!(
            normalize(" Example.COM./Case/ \r\nexample.com/Case/\n\nother.example/ ").unwrap(),
            "example.com/Case/\nother.example"
        );
        assert_eq!(normalize("\n  \n").unwrap(), "");
    }

    #[test]
    fn refuses_ambiguous_rules_and_bounds_the_list() {
        for rule in [
            "https://example.com",
            "*",
            "*example.com",
            "example.*",
            "foo.*.example.com",
            "*.*.example.com",
            "**.example.com",
            "*.com",
            "*.localhost",
            "*.127.0.0.1",
            "*.example.com/a*",
            "example.com:443",
            "user@example.com",
            "example.com?next=evil",
            "example.com#x",
            "example.com\\x",
            "example.com/a b",
            "example.com/%2f",
            "example.com/a/../b",
            "example.com/a/./b",
            "example.com//x",
            "example..com",
            "127.0.0.1",
            "localhost",
            "例子.com",
            "-bad.example",
        ] {
            assert!(normalize(rule).is_err(), "{rule}");
        }
        assert!(normalize(&"a".repeat(MAX_BYTES + 1)).is_err());
        let many = (0..=MAX_RULES)
            .map(|i| format!("host{i}.example"))
            .collect::<Vec<_>>()
            .join("\n");
        assert!(normalize(&many).is_err());
    }

    #[test]
    fn matches_exact_hosts_without_suffix_or_authority_bypasses() {
        for target in [
            "https://example.com/a",
            "HTTP://EXAMPLE.COM:80/x",
            "https://example.com.:443/x",
        ] {
            assert!(allows("example.com", target), "{target}");
        }
        for target in [
            "https://sub.example.com",
            "https://example.com.evil.test",
            "https://notexample.com",
            "https://example.com@evil.test",
            "https://evil.test@example.com",
            "https://example.com:8443",
            "ftp://example.com",
            "https://example.com%2eevil.test",
            "not a URL",
        ] {
            assert!(!allows("example.com", target), "{target}");
        }
        assert!(!allows("", "https://example.com"));
        assert!(!allows("invalid rule", "https://example.com"));
    }

    #[test]
    fn respects_path_segments_case_and_browser_normalization() {
        for target in [
            "https://example.com/Case",
            "https://example.com/Case/article?q=1#part",
        ] {
            assert!(allows("example.com/Case", target), "{target}");
        }
        for target in [
            "https://example.com/case",
            "https://example.com/Cases",
            "https://example.com/redirect?next=/Case",
            "https://example.com/Case/../redirect",
            "https://example.com/Case/%2e%2e/redirect",
            "https://example.com/Case/%252e%252e/redirect",
            "https://example.com/Case/a%2fb",
            "https://example.com/Case/a%5Cb",
            "https://example.com/Case//other",
        ] {
            assert!(!allows("example.com/Case", target), "{target}");
        }
        assert!(!allows("example.com/Case/", "https://example.com/Case"));
        assert!(allows(
            "example.com/Case/",
            "https://example.com/Case/Article"
        ));
    }

    #[test]
    fn wildcard_rules_match_all_subdomain_levels_but_never_the_root_or_spoofed_hosts() {
        assert_eq!(
            normalize(" *.Example.COM./Case/\n*.example.com/Case/\nexample.com/Case/ ").unwrap(),
            "*.example.com/Case/\nexample.com/Case/"
        );
        for target in [
            "https://zh.wikipedia.org/wiki/Rust",
            "https://en.m.wikipedia.org/wiki/Rust",
            "HTTPS://JA.WIKIPEDIA.ORG.:443/wiki/Rust",
        ] {
            assert!(allows("*.wikipedia.org", target), "{target}");
        }
        for target in [
            "https://wikipedia.org/wiki/Rust",
            "https://wikipedia.org./wiki/Rust",
            "https://evilwikipedia.org/wiki/Rust",
            "https://wikipedia.org.evil.test/wiki/Rust",
            "https://zh.wikipedia.org.evil.test/wiki/Rust",
            "https://zh.wikipedia.org@evil.test/wiki/Rust",
            "https://evil.test@zh.wikipedia.org/wiki/Rust",
            "https://zh.wikipedia.org:8443/wiki/Rust",
        ] {
            assert!(!allows("*.wikipedia.org", target), "{target}");
        }
        assert!(allows(
            "*.wikipedia.org\nwikipedia.org",
            "https://wikipedia.org"
        ));
    }

    #[test]
    fn wildcard_rules_preserve_path_restrictions() {
        let rules = "*.example.com/Case/";
        assert!(allows(rules, "https://a.b.example.com/Case/article?q=1#x"));
        for target in [
            "https://example.com/Case/article",
            "https://a.example.com/Case",
            "https://a.example.com/case/article",
            "https://a.example.com/Cases/article",
            "https://a.example.com/Case/../redirect",
            "https://a.example.com/Case/%2e%2e/redirect",
            "https://a.example.com/Case/a%2fb",
            "https://a.example.com/Case/a%5cb",
            "https://a.example.com/Case/%252e%252e/redirect",
            "https://a.example.com/Case//other",
        ] {
            assert!(!allows(rules, target), "{target}");
        }
    }

    #[test]
    fn builtins_allow_known_service_hosts_without_path_restrictions() {
        assert_eq!(normalize(DEFAULT_RULES).unwrap(), DEFAULT_RULES.trim_end());
        for target in [
            "https://www.youtube.com/watch?v=123",
            "https://youtube.com/",
            "https://m.youtube.com/shorts/123",
            "https://a.b.youtube.com/any-path",
            "https://www.youtube.com/redirect?q=https://evil.test",
            "https://zh.wikipedia.org/wiki/Rust",
            "https://de.m.wikipedia.org/wiki/Rust",
            "https://en.wikipedia.org/w/index.php",
            "https://wikipedia.org/",
            "https://github.com/cxyfer/urlet",
            "https://gitlab.com/gitlab-org/gitlab",
            "https://stackoverflow.com/users/1",
            "https://meta.stackoverflow.com/",
            "https://unix.stackexchange.com/",
            "https://developer.mozilla.org/en-US/docs/Web",
            "https://docs.python.org/3/",
            "https://doc.rust-lang.org/book/",
            "https://www.ptt.cc/bbs/Rust/index.html",
            "https://www.ptt.cc/about.html",
            "https://www.dcard.tw/",
            "https://forum.gamer.com.tw/",
            "https://www.cna.com.tw/",
            "https://m.bilibili.com/",
            "https://old.reddit.com/r/rust/",
            "https://clips.twitch.tv/example",
            "https://open.spotify.com/track/example",
            "https://www.instagram.com/example/",
            "https://m.facebook.com/example",
            "https://x.com/example",
            "https://www.bbc.com/",
            "https://www.bbc.co.uk/",
            "https://www.reuters.com/world/",
            "https://support.apple.com/zh-tw",
            "https://support.microsoft.com/zh-tw",
        ] {
            assert!(allows(DEFAULT_RULES, target), "{target}");
        }
        for target in [
            "https://youtube.com.evil.test/watch?v=123",
            "https://evilyoutube.com/watch?v=123",
            "https://youtu.be/123",
            "https://user.github.io/",
            "https://user.gitlab.io/",
            "https://sites.google.com/view/example",
            "https://notreddit.com/",
            "https://open.spotify.com.evil.test/",
            "https://support.apple.com.evil.test/",
            "https://unlisted.example/",
        ] {
            assert!(!allows(DEFAULT_RULES, target), "{target}");
        }
        assert_eq!(
            DEFAULT_RULES
                .lines()
                .filter(|rule| rule.contains("youtube"))
                .collect::<Vec<_>>(),
            ["*.youtube.com", "youtube.com"]
        );
    }
}
