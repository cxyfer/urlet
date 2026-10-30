//! Server clock readings for the landing page.
//!
//! The landing page used to print instrument-shaped constants (`01:06 UTC`,
//! `99.9 SLO`) that nobody could verify. This module produces the subset the
//! process can actually state: the wall clock in the configured zone, that
//! zone's current UTC offset, the calendar year and how long the process has
//! been running.
//!
//! Everything is computed once per request, so the values are fresh on every
//! render — the landing response is `no-store`. `assets/app.js` advances the
//! clock from the `epoch_ms` anchor when the zone is an IANA name; without
//! JavaScript the rendered time is still correct at the moment it was rendered.

use jiff::tz::TimeZone;

/// A truthful reading of the server clock, ready for the template.
#[derive(Debug, Clone)]
pub struct Reading {
    /// `HH:MM:SS` in the configured zone.
    pub time: String,
    /// IANA zone name, or the UTC offset when the zone has no name.
    pub zone: String,
    /// Current UTC offset, e.g. `+08:00`.
    pub offset: String,
    /// Calendar year in the configured zone.
    pub year: String,
    /// Unix epoch milliseconds: the anchor the browser ticks forward from.
    pub epoch_ms: i64,
    /// Compact process uptime, e.g. `3d 04h`.
    pub uptime: String,
}

/// Read the clock for `zone`, anchored at "now".
pub fn reading(zone: &TimeZone, uptime_seconds: i64) -> Reading {
    let now = jiff::Timestamp::now().to_zoned(zone.clone());
    let offset = format_offset(now.offset().seconds());
    Reading {
        time: format!("{:02}:{:02}:{:02}", now.hour(), now.minute(), now.second()),
        zone: zone
            .iana_name()
            .map(str::to_string)
            .unwrap_or_else(|| offset.clone()),
        offset,
        year: now.year().to_string(),
        epoch_ms: now.timestamp().as_millisecond(),
        uptime: format_uptime(uptime_seconds),
    }
}

/// `+08:00`, `-03:30`, `+00:00`.
///
/// `Offset`'s own `Display` drops the minutes when they are zero (`+08`), which
/// reads like a time of day next to a clock. Spell the minutes out instead.
fn format_offset(seconds: i32) -> String {
    let sign = if seconds < 0 { '-' } else { '+' };
    let abs = seconds.abs();
    format!("{sign}{:02}:{:02}", abs / 3_600, (abs % 3_600) / 60)
}

/// Compact uptime: `3d 04h`, `12h 40m` or `7m 12s`.
///
/// Two units at most, so the readout stays narrow on a phone. Unit letters are
/// deliberately language neutral: the label next to them is translated.
pub fn format_uptime(seconds: i64) -> String {
    let seconds = seconds.max(0);
    let days = seconds / 86_400;
    let hours = (seconds % 86_400) / 3_600;
    let minutes = (seconds % 3_600) / 60;
    let secs = seconds % 60;
    if days > 0 {
        format!("{days}d {hours:02}h")
    } else if hours > 0 {
        format!("{hours}h {minutes:02}m")
    } else {
        format!("{minutes}m {secs:02}s")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn uptime_picks_the_two_most_significant_units() {
        assert_eq!(format_uptime(0), "0m 00s");
        assert_eq!(format_uptime(59), "0m 59s");
        assert_eq!(format_uptime(7 * 60 + 12), "7m 12s");
        assert_eq!(format_uptime(3_600 + 40 * 60), "1h 40m");
        assert_eq!(format_uptime(3 * 86_400 + 4 * 3_600), "3d 04h");
        assert_eq!(format_uptime(-5), "0m 00s");
    }

    #[test]
    fn a_named_zone_reports_its_name_and_offset() {
        let zone = TimeZone::get("Asia/Taipei").expect("zone");
        let reading = reading(&zone, 90);
        assert_eq!(reading.zone, "Asia/Taipei");
        assert_eq!(reading.offset, "+08:00");
        assert_eq!(reading.time.len(), 8, "HH:MM:SS");
        assert_eq!(reading.year.len(), 4);
        assert_eq!(reading.uptime, "1m 30s");
        assert!(reading.epoch_ms > 0);
    }

    #[test]
    fn utc_reports_the_utc_offset() {
        let reading = reading(&TimeZone::UTC, 0);
        assert_eq!(reading.offset, "+00:00");
        assert_eq!(reading.zone, "UTC");
    }
}
