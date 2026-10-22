//! 訪客目標白名單：精確主機名稱與選填的路徑前綴，不抓取目標內容。

use crate::services::domains;

pub const DEFAULT_RULES: &str = include_str!("guest_allowlist.txt");
pub const MAX_RULES: usize = 200;
pub const MAX_BYTES: usize = 16_384;
pub const BLOCKED: &str = "guests may only shorten destinations on the allowed list";

struct Rule {
    host: String,
    path: String,
}

impl Rule {
    fn parse(raw: &str) -> Option<Self> {
        // 設定只接受 hostname[/path]，避免 scheme、帳密、連接埠及萬用字元的歧義。
        if raw.is_empty()
            || !raw.is_ascii()
            || raw.chars().any(|c| c.is_control() || c.is_whitespace())
            || raw.contains([':', '@', '?', '#', '\\', '*', '%'])
        {
            return None;
        }
        let (host, path) = match raw.find('/') {
            Some(index) => (&raw[..index], &raw[index..]),
            None => (raw, ""),
        };
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
            path: if path == "/" { "" } else { path }.to_string(),
        })
    }

    fn matches(&self, url: &url::Url) -> bool {
        if url.host_str().map(|host| host.trim_end_matches('.')) != Some(self.host.as_str()) {
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
            "use one exact hostname per line, optionally followed by an ASCII path (no scheme, port, wildcard, query or fragment)",
        )?;
        let rule = format!("{}{}", rule.host, rule.path);
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
            "*.example.com",
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
    fn builtins_are_valid_and_keep_unrelated_redirect_paths_closed() {
        assert_eq!(normalize(DEFAULT_RULES).unwrap(), DEFAULT_RULES.trim_end());
        for target in [
            "https://www.youtube.com/watch?v=123",
            "https://zh.wikipedia.org/wiki/Rust",
            "https://github.com/cxyfer/urlet",
            "https://www.ptt.cc/bbs/Rust/index.html",
        ] {
            assert!(allows(DEFAULT_RULES, target), "{target}");
        }
        for target in [
            "https://www.youtube.com/redirect?q=https://evil.test",
            "https://en.wikipedia.org/w/index.php",
            "https://youtu.be/123",
        ] {
            assert!(!allows(DEFAULT_RULES, target), "{target}");
        }
    }
}
