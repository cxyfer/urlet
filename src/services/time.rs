//! Time helpers.
//!
//! Every timestamp stored in the database is an ISO-8601 UTC string with a
//! *fixed* width (`YYYY-MM-DDTHH:MM:SS.sssZ`). Fixed width matters: it makes
//! lexicographic comparison in SQL identical to chronological comparison.

pub use time::OffsetDateTime;

use time::format_description::FormatItem;
use time::macros::format_description;
use time::{Duration, PrimitiveDateTime, UtcOffset};

/// The canonical on-disk timestamp format. The trailing `Z` is a literal, so
/// parsing goes through [`PrimitiveDateTime`] and assumes UTC.
pub const ISO: &[FormatItem<'static>] =
    format_description!("[year]-[month]-[day]T[hour]:[minute]:[second].[subsecond digits:3]Z");

/// The canonical on-disk date format (daily rollups).
pub const DAY: &[FormatItem<'static>] = format_description!("[year]-[month]-[day]");

/// Current UTC time.
pub fn now() -> OffsetDateTime {
    OffsetDateTime::now_utc()
}

/// Format an instant for storage.
pub fn to_iso(t: OffsetDateTime) -> String {
    t.to_offset(UtcOffset::UTC)
        .format(ISO)
        .unwrap_or_else(|_| "1970-01-01T00:00:00.000Z".to_string())
}

/// Current instant, formatted for storage.
pub fn now_iso() -> String {
    to_iso(now())
}

/// Parse a stored timestamp. Returns `None` for malformed input.
pub fn parse_iso(s: &str) -> Option<OffsetDateTime> {
    PrimitiveDateTime::parse(s, ISO)
        .ok()
        .map(PrimitiveDateTime::assume_utc)
}

/// Parse user supplied timestamps leniently.
///
/// Accepted: the canonical storage format, `...:SSZ`, full RFC 3339 with an
/// offset, and a bare `YYYY-MM-DD` date (interpreted as midnight UTC).
pub fn parse_flexible(input: &str) -> Option<OffsetDateTime> {
    let input = input.trim();
    if input.is_empty() {
        return None;
    }
    // Accept "2030-01-02 03:04" as well as the canonical "2030-01-02T03:04:05Z".
    let owned;
    let input = if input.contains(' ') && !input.contains('T') {
        owned = input.split_whitespace().collect::<Vec<_>>().join("T");
        owned.as_str()
    } else {
        input
    };
    if let Some(parsed) = parse_iso(input) {
        return Some(parsed);
    }

    const SECONDS: &[FormatItem<'static>] =
        format_description!("[year]-[month]-[day]T[hour]:[minute]:[second]Z");
    if let Ok(parsed) = PrimitiveDateTime::parse(input, SECONDS) {
        return Some(parsed.assume_utc());
    }

    const MINUTES: &[FormatItem<'static>] =
        format_description!("[year]-[month]-[day]T[hour]:[minute]");
    if let Ok(parsed) = PrimitiveDateTime::parse(input, MINUTES) {
        return Some(parsed.assume_utc());
    }

    if let Ok(parsed) = OffsetDateTime::parse(input, &time::format_description::well_known::Rfc3339)
    {
        return Some(parsed.to_offset(UtcOffset::UTC));
    }

    if let Ok(date) = time::Date::parse(input, DAY) {
        return Some(date.midnight().assume_utc());
    }

    None
}

/// Format a date-only value for daily rollups.
pub fn to_day(t: OffsetDateTime) -> String {
    t.format(DAY).unwrap_or_else(|_| "1970-01-01".to_string())
}

/// `true` when `timestamp` is strictly in the past.
pub fn is_past(s: &str) -> bool {
    match parse_iso(s) {
        Some(t) => t < now(),
        // A malformed expiry must not silently keep a link alive.
        None => true,
    }
}

/// `now + hours`, formatted for storage.
pub fn iso_in_hours(hours: i64) -> String {
    to_iso(now() + Duration::hours(hours))
}

/// `now + minutes`, formatted for storage.
pub fn iso_in_minutes(minutes: i64) -> String {
    to_iso(now() + Duration::minutes(minutes))
}

/// `base + days`.
pub fn plus_days(base: OffsetDateTime, days: i64) -> OffsetDateTime {
    base + Duration::days(days)
}

/// `now - days`, formatted for storage.
pub fn iso_days_ago(days: i64) -> String {
    to_iso(now() - Duration::days(days))
}

/// `now - seconds`, formatted for storage.
pub fn iso_seconds_ago(seconds: i64) -> String {
    to_iso(now() - Duration::seconds(seconds))
}

/// `now - minutes`, formatted for storage.
pub fn iso_minutes_ago(minutes: i64) -> String {
    to_iso(now() - Duration::minutes(minutes))
}

/// First instant of the UTC day `days` ago (inclusive lower bound for charts).
pub fn day_start_days_ago(days: i64) -> String {
    let start = (now() - Duration::days(days)).replace_time(time::Time::MIDNIGHT);
    to_iso(start)
}

/// Day string for `days` ago.
pub fn day_days_ago(days: i64) -> String {
    to_day(now() - Duration::days(days))
}

/// A human friendly relative description such as `in 3 days` / `2 hours ago`.
///
/// Always English: this is the form embedded in `/api/v1` payloads. The pages
/// use [`humanize_lang`] instead.
pub fn humanize(timestamp: &str) -> String {
    let Some(t) = parse_iso(timestamp) else {
        return "unknown".to_string();
    };
    let delta = t - now();
    let secs = delta.whole_seconds();
    let abs = secs.abs();

    // Thresholds are chosen so that rounding never produces "60 minutes".
    let (unit, unit_secs) = if abs < 60 {
        ("second", 1)
    } else if abs < 3_570 {
        ("minute", 60)
    } else if abs < 82_800 {
        ("hour", 3_600)
    } else if abs < 3_888_000 {
        ("day", 86_400)
    } else {
        ("month", 2_592_000)
    };

    let mut value = (abs + unit_secs / 2) / unit_secs;
    if unit_secs > 1 && value == 0 {
        value = 1;
    }
    let plural = if value == 1 { "" } else { "s" };

    if secs > 0 {
        format!("in {value} {unit}{plural}")
    } else {
        format!("{value} {unit}{plural} ago")
    }
}

/// [`humanize`] rendered in the language of the page being built.
///
/// English delegates to [`humanize`] so the API wording and the web wording can
/// never drift apart.
pub fn humanize_lang(lang: crate::i18n::Lang, timestamp: &str) -> String {
    if lang == crate::i18n::Lang::En {
        return humanize(timestamp);
    }
    let Some(t) = parse_iso(timestamp) else {
        return lang.t("time.unknown").to_string();
    };
    let delta = t - now();
    let secs = delta.whole_seconds();
    let abs = secs.abs();

    let (unit, unit_secs) = if abs < 60 {
        ("second", 1)
    } else if abs < 3_570 {
        ("minute", 60)
    } else if abs < 82_800 {
        ("hour", 3_600)
    } else if abs < 3_888_000 {
        ("day", 86_400)
    } else {
        ("month", 2_592_000)
    };

    let mut value = (abs + unit_secs / 2) / unit_secs;
    if unit_secs > 1 && value == 0 {
        value = 1;
    }
    let unit_key = match (unit, value == 1) {
        ("second", true) => "time.unit.second",
        ("second", false) => "time.unit.seconds",
        ("minute", true) => "time.unit.minute",
        ("minute", false) => "time.unit.minutes",
        ("hour", true) => "time.unit.hour",
        ("hour", false) => "time.unit.hours",
        ("day", true) => "time.unit.day",
        ("day", false) => "time.unit.days",
        (_, true) => "time.unit.month",
        (_, false) => "time.unit.months",
    };
    let template = if secs > 0 {
        lang.t("time.in")
    } else {
        lang.t("time.ago")
    };
    template
        .replace("{value}", &value.to_string())
        .replace("{unit}", lang.t(unit_key))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn iso_round_trip() {
        let now = now();
        let s = to_iso(now);
        assert_eq!(s.len(), 24, "fixed width timestamp: {s}");
        assert!(s.ends_with('Z'));
        let parsed = parse_iso(&s).expect("parses back");
        assert_eq!(parsed.unix_timestamp(), now.unix_timestamp());
    }

    #[test]
    fn fixed_width_sorts_chronologically() {
        let a = to_iso(now());
        let b = to_iso(now() + Duration::milliseconds(1));
        assert!(a < b, "{a} should sort before {b}");
        let c = to_iso(now() + Duration::seconds(1));
        assert!(b < c);
    }

    #[test]
    fn malformed_expiry_is_treated_as_expired() {
        assert!(is_past("not-a-date"));
        assert!(is_past(&to_iso(now() - Duration::seconds(1))));
        assert!(!is_past(&to_iso(now() + Duration::hours(1))));
    }

    #[test]
    fn humanize_handles_both_directions() {
        assert!(humanize(&iso_in_hours(2)).starts_with("in 2 hours"));
        assert!(humanize(&iso_in_minutes(-5)).ends_with("ago"));
        assert_eq!(humanize("garbage"), "unknown");
    }

    #[test]
    fn humanize_lang_matches_english_and_translates() {
        use crate::i18n::Lang;
        // English output is byte identical to the API wording.
        assert_eq!(
            humanize_lang(Lang::En, &iso_in_hours(2)),
            humanize(&iso_in_hours(2))
        );
        assert_eq!(humanize_lang(Lang::En, "garbage"), "unknown");

        assert_eq!(humanize_lang(Lang::ZhHant, &iso_in_hours(2)), "2 小時後");
        assert_eq!(humanize_lang(Lang::ZhHant, &iso_in_minutes(-5)), "5 分鐘前");
        assert_eq!(humanize_lang(Lang::ZhHant, &iso_days_ago(1)), "1 天前");
        assert_eq!(humanize_lang(Lang::ZhHant, "garbage"), "未知");
        assert_eq!(humanize_lang(Lang::ZhHans, &iso_in_hours(2)), "2 小时后");
    }

    #[test]
    fn parses_user_supplied_timestamps() {
        assert!(parse_flexible("2030-01-02T03:04:05Z").is_some());
        assert!(parse_flexible("2030-01-02T03:04:05.123Z").is_some());
        assert!(parse_flexible("2030-01-02T03:04:05+08:00").is_some());
        assert!(parse_flexible("2030-01-02").is_some());
        assert!(parse_flexible(" 2030-01-02T03:04:05Z ").is_some());
        assert!(parse_flexible("2030-01-02 03:04").is_some());
        assert!(parse_flexible("2030-01-02T03:04").is_some());
        assert!(parse_flexible("tomorrow").is_none());
        assert!(parse_flexible("").is_none());
    }

    #[test]
    fn offsets_are_normalised_to_utc() {
        let parsed = parse_flexible("2030-01-02T11:04:05+08:00").expect("parses");
        assert_eq!(to_iso(parsed), "2030-01-02T03:04:05.000Z");
    }
}
