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
        let date = OffsetDateTime::from_unix_timestamp(now.as_second())
            .map_err(|err| AppError::internal(err.to_string()))?
            .to_offset(offset)
            .date();
        let (start, end) = match self {
            Self::Minute | Self::Hour => {
                let minute = if self == Self::Minute {
                    local.minute()
                } else {
                    0
                };
                let start = clock_boundary(&local, local.hour(), minute)?;
                let length = if self == Self::Minute {
                    Duration::minutes(1)
                } else {
                    Duration::hours(1)
                };
                (start, start + length.whole_seconds())
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

/// 保留當下 offset 偏好，正確區分回撥時重複的小時。
fn clock_boundary(local: &jiff::Zoned, hour: i8, minute: i8) -> Result<i64> {
    local
        .with()
        .hour(hour)
        .minute(minute)
        .second(0)
        .subsec_nanosecond(0)
        .build()
        .map(|zoned| zoned.timestamp().as_second())
        .map_err(|err| AppError::internal(err.to_string()))
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
}
