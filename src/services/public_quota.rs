//! 依伺服器時區計算遊客的自然週期配額。

use jiff::{tz::TimeZone, Timestamp};
use sqlx::SqlitePool;
use time::{Date, Duration, Month, OffsetDateTime, UtcOffset};

use crate::db;
use crate::error::{AppError, Result};
use crate::services::secret;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PublicRatePeriod {
    Minute,
    Hour,
    Day,
    Week,
    Month,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Window {
    pub start: i64,
    pub end: i64,
}

impl PublicRatePeriod {
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "minute" => Some(Self::Minute),
            "hour" => Some(Self::Hour),
            "day" => Some(Self::Day),
            "week" => Some(Self::Week),
            "month" => Some(Self::Month),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Minute => "minute",
            Self::Hour => "hour",
            Self::Day => "day",
            Self::Week => "week",
            Self::Month => "month",
        }
    }

    /// 日期運算沿用 time；Jiff 只負責系統時區及本地邊界到 UTC 的轉換。
    pub fn window(self, now: Timestamp, zone: &TimeZone) -> Result<Window> {
        let local = now.to_zoned(zone.clone());
        let offset = UtcOffset::from_whole_seconds(local.offset().seconds())
            .map_err(|err| AppError::internal(err.to_string()))?;
        let calendar = OffsetDateTime::from_unix_timestamp(now.as_second())
            .map_err(|err| AppError::internal(err.to_string()))?
            .to_offset(offset);
        let date = calendar.date();
        let (start, end) = match self {
            Self::Minute | Self::Hour => {
                let elapsed = Duration::seconds(i64::from(calendar.second()))
                    + if self == Self::Hour {
                        Duration::minutes(i64::from(calendar.minute()))
                    } else {
                        Duration::ZERO
                    };
                let mut start = (calendar - elapsed).unix_timestamp();
                let length = if self == Self::Minute {
                    Duration::minutes(1)
                } else {
                    Duration::hours(1)
                };
                let mut end = start + length.whole_seconds();
                // 時區可能回撥或跳過半小時；週期不可跨越 offset 切換點。
                let inclusive_now = now
                    .checked_add(jiff::SignedDuration::from_nanos(1))
                    .map_err(|err| AppError::internal(err.to_string()))?;
                if let Some(previous) = zone.preceding(inclusive_now).next() {
                    start = start.max(previous.timestamp().as_second());
                }
                if let Some(next) = zone.following(now).next() {
                    end = end.min(next.timestamp().as_second());
                }
                (start, end)
            }
            Self::Day => (
                midnight(date, zone)?,
                midnight(date + Duration::days(1), zone)?,
            ),
            Self::Week => {
                let monday =
                    date - Duration::days(i64::from(date.weekday().number_days_from_monday()));
                (
                    midnight(monday, zone)?,
                    midnight(monday + Duration::weeks(1), zone)?,
                )
            }
            Self::Month => {
                let first = Date::from_calendar_date(date.year(), date.month(), 1)
                    .map_err(|err| AppError::internal(err.to_string()))?;
                let next_year = date.year() + i32::from(date.month() == Month::December);
                let next = Date::from_calendar_date(next_year, date.month().next(), 1)
                    .map_err(|err| AppError::internal(err.to_string()))?;
                (midnight(first, zone)?, midnight(next, zone)?)
            }
        };
        Ok(Window { start, end })
    }
}

/// 僅將 time 算出的日期交給時區庫解析；重複的午夜取較早者，缺少時向前移。
fn midnight(date: Date, zone: &TimeZone) -> Result<i64> {
    let civil = jiff::civil::Date::new(date.year() as i16, date.month() as i8, date.day() as i8)
        .map_err(|err| AppError::internal(err.to_string()))?;
    civil
        .at(0, 0, 0, 0)
        .to_zoned(zone.clone())
        .map(|zoned| zoned.timestamp().as_second())
        .map_err(|err| AppError::internal(err.to_string()))
}

/// `0` 表示不限；用量存進 SQLite，重啟仍會繼續計數到下一個自然邊界。
pub async fn enforce(
    pool: &SqlitePool,
    zone: &TimeZone,
    scope: &str,
    limit: i64,
    period: PublicRatePeriod,
    now: Timestamp,
) -> Result<()> {
    if limit <= 0 {
        return Ok(());
    }
    let window = period.window(now, zone)?;
    let accepted = db::public_quota::reserve(
        pool,
        &secret::sha256_hex(scope),
        period.as_str(),
        window.start,
        window.end,
        limit,
        now.as_second(),
    )
    .await
    .map_err(AppError::from)?;
    if accepted {
        Ok(())
    } else {
        Err(AppError::rate_limited(
            (window.end - now.as_second()).max(1) as u64,
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn check(period: PublicRatePeriod, zone: &str, now: &str, start: &str, end: &str) {
        let zone = TimeZone::get(zone).expect("timezone");
        let window = period
            .window(now.parse().expect("now"), &zone)
            .expect("window");
        assert_eq!(
            window.start,
            start.parse::<Timestamp>().expect("start").as_second()
        );
        assert_eq!(
            window.end,
            end.parse::<Timestamp>().expect("end").as_second()
        );
    }

    #[test]
    fn periods_follow_the_server_calendar() {
        let now = "2026-10-25T23:59:42+08:00";
        for (period, start, end) in [
            (
                PublicRatePeriod::Minute,
                "2026-10-25T23:59:00+08:00",
                "2026-10-26T00:00:00+08:00",
            ),
            (
                PublicRatePeriod::Hour,
                "2026-10-25T23:00:00+08:00",
                "2026-10-26T00:00:00+08:00",
            ),
            (
                PublicRatePeriod::Day,
                "2026-10-25T00:00:00+08:00",
                "2026-10-26T00:00:00+08:00",
            ),
            (
                PublicRatePeriod::Week,
                "2026-10-19T00:00:00+08:00",
                "2026-10-26T00:00:00+08:00",
            ),
            (
                PublicRatePeriod::Month,
                "2026-10-01T00:00:00+08:00",
                "2026-11-01T00:00:00+08:00",
            ),
        ] {
            check(period, "Asia/Taipei", now, start, end);
            check(period, "Asia/Taipei", start, start, end);
        }
    }

    #[test]
    fn weeks_cross_years_and_months_have_their_actual_length() {
        check(
            PublicRatePeriod::Week,
            "Asia/Taipei",
            "2027-01-01T12:00:00+08:00",
            "2026-12-28T00:00:00+08:00",
            "2027-01-04T00:00:00+08:00",
        );
        for (now, start, end) in [
            (
                "2028-02-29T12:00:00+08:00",
                "2028-02-01T00:00:00+08:00",
                "2028-03-01T00:00:00+08:00",
            ),
            (
                "2027-02-28T12:00:00+08:00",
                "2027-02-01T00:00:00+08:00",
                "2027-03-01T00:00:00+08:00",
            ),
            (
                "2026-04-30T12:00:00+08:00",
                "2026-04-01T00:00:00+08:00",
                "2026-05-01T00:00:00+08:00",
            ),
            (
                "2026-12-31T12:00:00+08:00",
                "2026-12-01T00:00:00+08:00",
                "2027-01-01T00:00:00+08:00",
            ),
        ] {
            check(PublicRatePeriod::Month, "Asia/Taipei", now, start, end);
        }
    }

    #[test]
    fn daylight_saving_changes_calendar_day_and_week_lengths() {
        check(
            PublicRatePeriod::Day,
            "America/New_York",
            "2026-03-08T12:00:00-04:00",
            "2026-03-08T00:00:00-05:00",
            "2026-03-09T00:00:00-04:00",
        );
        check(
            PublicRatePeriod::Week,
            "America/New_York",
            "2026-03-08T12:00:00-04:00",
            "2026-03-02T00:00:00-05:00",
            "2026-03-09T00:00:00-04:00",
        );
        check(
            PublicRatePeriod::Month,
            "America/New_York",
            "2026-03-08T12:00:00-04:00",
            "2026-03-01T00:00:00-05:00",
            "2026-04-01T00:00:00-04:00",
        );
        check(
            PublicRatePeriod::Day,
            "America/New_York",
            "2026-11-01T12:00:00-05:00",
            "2026-11-01T00:00:00-04:00",
            "2026-11-02T00:00:00-05:00",
        );
        // 智利在午夜跳至 01:00；仍必須以當天第一個有效時間為邊界。
        check(
            PublicRatePeriod::Day,
            "America/Santiago",
            "2026-09-06T12:00:00-03:00",
            "2026-09-06T01:00:00-03:00",
            "2026-09-07T00:00:00-03:00",
        );
    }

    #[test]
    fn a_repeated_midnight_does_not_split_the_calendar_day() {
        for now in ["2026-11-01T00:30:00-04:00", "2026-11-01T00:30:00-05:00"] {
            check(
                PublicRatePeriod::Day,
                "America/Havana",
                now,
                "2026-11-01T00:00:00-04:00",
                "2026-11-02T00:00:00-05:00",
            );
        }
    }

    #[test]
    fn repeated_hours_have_distinct_windows() {
        check(
            PublicRatePeriod::Hour,
            "America/New_York",
            "2026-11-01T01:30:00-04:00",
            "2026-11-01T01:00:00-04:00",
            "2026-11-01T01:00:00-05:00",
        );
        check(
            PublicRatePeriod::Hour,
            "America/New_York",
            "2026-11-01T01:30:00-05:00",
            "2026-11-01T01:00:00-05:00",
            "2026-11-01T02:00:00-05:00",
        );
    }

    #[test]
    fn half_hour_dst_transitions_keep_clock_hour_boundaries() {
        for (now, start, end) in [
            (
                "2026-10-04T02:45:00+11:00",
                "2026-10-04T02:30:00+11:00",
                "2026-10-04T03:00:00+11:00",
            ),
            (
                "2026-04-05T01:45:00+11:00",
                "2026-04-05T01:00:00+11:00",
                "2026-04-05T01:30:00+10:30",
            ),
            (
                "2026-04-05T01:45:00+10:30",
                "2026-04-05T01:30:00+10:30",
                "2026-04-05T02:00:00+10:30",
            ),
        ] {
            check(
                PublicRatePeriod::Hour,
                "Australia/Lord_Howe",
                now,
                start,
                end,
            );
            check(
                PublicRatePeriod::Hour,
                "Australia/Lord_Howe",
                start,
                start,
                end,
            );
        }
    }
}
