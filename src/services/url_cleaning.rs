//! 建立短網址時依網域移除追蹤參數，保留其他 URL 元件的原始編碼。

use std::sync::OnceLock;

use crate::error::{AppError, Result};
use crate::services::target;

pub const MAX_RULE_BYTES: usize = 8192;
const MAX_RULES: usize = 100;
const MAX_PARAMETERS: usize = 32;
const MAX_PARAMETER_LEN: usize = 64;

/// 與管理介面顯示的規則共用同一份定義；不移除內容 ID 或播放時間。
pub const BUILTIN_RULES: &str = "facebook.com utm_* fbclid mibextid\n\
instagram.com utm_* igshid igsh\n\
threads.net utm_* igshid igsh\n\
threads.com utm_* igshid igsh\n\
x.com utm_* s t ref_src ref_url\n\
twitter.com utm_* s t ref_src ref_url\n\
youtube.com utm_* si feature\n\
youtu.be utm_* si feature\n\
tiktok.com utm_* _t _r share_app_name share_iid\n\
bilibili.com utm_* spm_id_from vd_source share_source share_medium share_plat share_session_id share_tag";

#[derive(Debug, PartialEq, Eq)]
pub struct Rule {
    domain: String,
    parameters: Vec<String>,
}

/// 每行一個網域與參數清單；只允許參數尾端的前綴萬用字元。
pub fn parse_rules(raw: &str) -> Result<Vec<Rule>> {
    let invalid = |line: usize, reason: &str| {
        AppError::validation("url_cleaning_rules", format!("line {line}: {reason}"))
    };
    if raw.len() > MAX_RULE_BYTES {
        return Err(AppError::validation(
            "url_cleaning_rules",
            "cleanup rules must be at most 8192 bytes",
        ));
    }
    let mut rules = Vec::new();
    for (index, line) in raw.lines().enumerate() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if rules.len() == MAX_RULES {
            return Err(invalid(index + 1, "at most 100 rules are allowed"));
        }
        let mut fields = line.split_whitespace();
        let domain = fields.next().unwrap_or_default().to_ascii_lowercase();
        let valid_label = |label: &str| {
            !label.is_empty()
                && label.len() <= 63
                && !label.starts_with('-')
                && !label.ends_with('-')
                && label
                    .bytes()
                    .all(|c| c.is_ascii_alphanumeric() || c == b'-')
        };
        if domain.len() > 253 || !domain.contains('.') || !domain.split('.').all(valid_label) {
            return Err(invalid(
                index + 1,
                "use a hostname without scheme, port, path or wildcard",
            ));
        }
        let parameters: Vec<String> = fields.map(str::to_ascii_lowercase).collect();
        if parameters.is_empty() || parameters.len() > MAX_PARAMETERS {
            return Err(invalid(
                index + 1,
                "provide 1-32 parameter names or prefixes",
            ));
        }
        for parameter in &parameters {
            let name = parameter.strip_suffix('*').unwrap_or(parameter);
            if parameter.len() > MAX_PARAMETER_LEN
                || name.is_empty()
                || !name
                    .bytes()
                    .all(|c| c.is_ascii_alphanumeric() || matches!(c, b'_' | b'-' | b'.'))
            {
                return Err(invalid(index + 1, "parameters must be 1-64 ASCII letters, digits, _, - or ., with an optional final *"));
            }
        }
        rules.push(Rule { domain, parameters });
    }
    Ok(rules)
}

fn domain_matches(host: &str, domain: &str) -> bool {
    host == domain
        || host
            .strip_suffix(domain)
            .is_some_and(|prefix| prefix.ends_with('.'))
}

fn parameter_matches(name: &str, pattern: &str) -> bool {
    match pattern.strip_suffix('*') {
        Some(prefix) => name.starts_with(prefix),
        None => name == pattern,
    }
}

/// 輸入須先通過 target::validate；僅解碼名稱以比對，不重新序列化 query。
pub fn clean(target_url: &str, custom_rules: &[Rule]) -> String {
    static BUILTINS: OnceLock<Vec<Rule>> = OnceLock::new();
    let builtins =
        BUILTINS.get_or_init(|| parse_rules(BUILTIN_RULES).expect("valid built-in cleanup rules"));
    let Some(host) = target::host_of(target_url) else {
        return target_url.to_string();
    };
    let host = host.trim_end_matches('.');
    let parameters: Vec<&str> = builtins
        .iter()
        .chain(custom_rules)
        .filter(|rule| domain_matches(host, &rule.domain))
        .flat_map(|rule| rule.parameters.iter().map(String::as_str))
        .collect();
    if parameters.is_empty() {
        return target_url.to_string();
    }
    let query_end = target_url.find('#').unwrap_or(target_url.len());
    let Some(query_start) = target_url[..query_end].find('?') else {
        return target_url.to_string();
    };
    let query = &target_url[query_start + 1..query_end];
    let mut removed = false;
    let kept: Vec<&str> = query
        .split('&')
        .filter(|part| {
            let key = part.split('=').next().unwrap_or_default();
            let decoded = serde_urlencoded::from_str::<Vec<(String, String)>>(&format!("{key}="))
                .ok()
                .and_then(|pairs| pairs.into_iter().next())
                .map(|(key, _)| key.to_ascii_lowercase());
            let remove = decoded.is_some_and(|key| {
                parameters
                    .iter()
                    .any(|pattern| parameter_matches(&key, pattern))
            });
            removed |= remove;
            !remove
        })
        .collect();
    if !removed {
        return target_url.to_string();
    }
    let query = kept.join("&");
    let mut cleaned = target_url[..query_start].to_string();
    if !query.is_empty() {
        cleaned.push('?');
        cleaned.push_str(&query);
    }
    cleaned.push_str(&target_url[query_end..]);
    cleaned
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cleans_supported_platforms_without_removing_content_parameters() {
        for (input, expected) in [
            ("https://www.facebook.com/watch/?v=123&fbclid=x&mibextid=y&utm_source=z", "https://www.facebook.com/watch/?v=123"),
            ("https://instagram.com/p/ABC?igsh=x&igshid=y&utm_medium=social", "https://instagram.com/p/ABC"),
            ("https://www.threads.net/@user/post/id?igsh=x", "https://www.threads.net/@user/post/id"),
            ("https://www.threads.com/@user/post/id?igsh=x", "https://www.threads.com/@user/post/id"),
            ("https://x.com/user/status/123?s=20&t=tracking&lang=en", "https://x.com/user/status/123?lang=en"),
            ("https://twitter.com/user/status/123?ref_src=x&ref_url=y", "https://twitter.com/user/status/123"),
            ("https://www.youtube.com/watch?v=ABC&si=x&feature=shared&t=90&list=PL1&index=2#chapter", "https://www.youtube.com/watch?v=ABC&t=90&list=PL1&index=2#chapter"),
            ("https://youtu.be/ABC?si=x&t=90", "https://youtu.be/ABC?t=90"),
            ("https://www.tiktok.com/@user/video/123?_t=x&_r=1&share_iid=y&lang=en", "https://www.tiktok.com/@user/video/123?lang=en"),
            ("https://www.bilibili.com/video/BV123?spm_id_from=x&vd_source=y&share_source=z&p=2&t=90", "https://www.bilibili.com/video/BV123?p=2&t=90"),
        ] {
            let cleaned = clean(input, &[]);
            assert_eq!(cleaned, expected, "{input}");
            assert_eq!(clean(&cleaned, &[]), cleaned, "idempotent");
        }
    }

    #[test]
    fn preserves_unmatched_urls_and_authority_boundaries() {
        for input in [
            "https://example.com/?utm_source=x&si=y",
            "https://evilyoutube.com/?si=x",
            "https://youtube.com.evil.com/?si=x",
            "https://youtube.com@evil.com/?si=x",
            "https://youtube.com/?v=x#?si=fragment&utm_source=fragment",
            "https://youtube.com/?v=ABC%2fdef&token=a+b&token=a%20b&&flag#x",
            "https://youtube.com/?",
        ] {
            assert_eq!(clean(input, &[]), input);
        }
        assert_eq!(
            clean("https://user:pass@m.youtube.com:8443/watch?si=x", &[]),
            "https://user:pass@m.youtube.com:8443/watch"
        );
        assert_eq!(
            clean("https://youtube.com./watch?si=x", &[]),
            "https://youtube.com./watch"
        );
    }

    #[test]
    fn preserves_raw_values_order_repeats_and_fragment() {
        let input = "https://youtube.com/watch?%75tm_Source=x&v=A%2fb&si=one&x=a+b&x=a%20b&si=&flag&utm_source&empty=#frag?si=keep";
        assert_eq!(
            clean(input, &[]),
            "https://youtube.com/watch?v=A%2fb&x=a+b&x=a%20b&flag&empty=#frag?si=keep"
        );
        assert_eq!(
            clean("https://youtube.com/?si=x#keep", &[]),
            "https://youtube.com/#keep"
        );
        assert_eq!(
            clean("https://youtube.com/?x=%FF&%FF=x&si=x", &[]),
            "https://youtube.com/?x=%FF&%FF=x"
        );
    }

    #[test]
    fn custom_rules_are_additive_and_include_subdomains() {
        let rules =
            parse_rules("# Campaign tracking\nExample.COM utm_* REF\nexample.org source").unwrap();
        assert_eq!(
            clean(
                "https://a.example.com/?%72ef=x&utm_test=y&id=42&reference=keep",
                &rules
            ),
            "https://a.example.com/?id=42&reference=keep"
        );
        assert_eq!(
            clean("https://youtu.be/ABC?si=x&t=12", &rules),
            "https://youtu.be/ABC?t=12"
        );
        assert_eq!(
            clean("https://evilexample.com/?ref=x", &rules),
            "https://evilexample.com/?ref=x"
        );
        assert_eq!(
            clean("https://example.org/?source=x&id=42", &rules),
            "https://example.org/?id=42"
        );
    }

    #[test]
    fn rejects_ambiguous_or_unbounded_rules() {
        for raw in [
            "example.com",
            "* utm_*",
            "*.example.com ref",
            "https://example.com ref",
            "example.com/path ref",
            "example.com:443 ref",
            "user@example.com ref",
            "example..com ref",
            "-example.com ref",
            "example-.com ref",
            "example.com. ref",
            "example.com *",
            "example.com ut*m",
            "example.com ref=",
            "example.com réf",
        ] {
            assert!(parse_rules(raw).is_err(), "{raw}");
        }
        assert!(parse_rules(&"example.com ref\n".repeat(101)).is_err());
        assert!(parse_rules(&format!("example.com {}", "ref ".repeat(33))).is_err());
        assert!(parse_rules(&format!("example.com {}", "a".repeat(65))).is_err());
        assert!(parse_rules(&format!("{}.com ref", "a".repeat(64))).is_err());
        assert!(parse_rules(&" ".repeat(MAX_RULE_BYTES + 1)).is_err());
        assert!(parse_rules("").unwrap().is_empty());
        assert_eq!(parse_rules(BUILTIN_RULES).unwrap().len(), 10);
    }
}
