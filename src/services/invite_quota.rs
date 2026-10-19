//! Invite issuance quotas.
//!
//! Every group may issue at most `max_invites` invite codes per calendar
//! `invite_period` (day, week or month, in UTC). A NULL group quota inherits the
//! site-wide `max_invites_per_period` setting and `0` means unlimited, matching
//! how link and token quotas work. Invites that are not pinned to a group use
//! the site-wide quota and the site-wide `invite_period` setting.
//!
//! The counter lives in the `invite_issuance` ledger, not in `invite_codes`, so
//! revoking a code does not free its slot. Periods are calendar based: the
//! counter resets on its own at the next boundary, with no scheduled job.

use sqlx::SqlitePool;

use crate::db::invite_issuance;
use crate::db::user_groups::UserGroup;
use crate::error::{AppError, Result};
use crate::services::settings::Settings;
use crate::services::time as clock;
use time::{Date, Duration, OffsetDateTime};

/// Calendar window of an invite quota.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum InvitePeriod {
    Day,
    Week,
    #[default]
    Month,
}

impl InvitePeriod {
    /// Parse a stored or submitted period. Unknown values are rejected so a typo
    /// cannot silently become "month".
    pub fn parse(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "day" => Some(Self::Day),
            "week" => Some(Self::Week),
            "month" => Some(Self::Month),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Day => "day",
            Self::Week => "week",
            Self::Month => "month",
        }
    }

    /// First instant of the period containing `now`, in UTC.
    pub fn start(self, now: OffsetDateTime) -> OffsetDateTime {
        let date = now.date();
        let start = match self {
            Self::Day => date,
            // ISO weeks start on Monday.
            Self::Week => {
                date - Duration::days(i64::from(date.weekday().number_days_from_monday()))
            }
            Self::Month => Date::from_calendar_date(date.year(), date.month(), 1).unwrap_or(date),
        };
        start.midnight().assume_utc()
    }

    /// Inclusive lower bound passed to the ledger query.
    pub fn window_start(self, now: OffsetDateTime) -> String {
        clock::to_iso(self.start(now))
    }
}

/// The effective quota of one group (or of the site-wide pool).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InviteQuota {
    /// `0` means unlimited.
    pub limit: i64,
    pub period: InvitePeriod,
}

impl InviteQuota {
    /// Resolve the effective quota. `group = None` means an invite that is not
    /// pinned to a group and therefore consumes the site-wide pool.
    pub fn resolve(settings: &Settings, group: Option<&UserGroup>) -> Self {
        match group {
            Some(group) => Self {
                limit: group
                    .max_invites
                    .unwrap_or_else(|| settings.max_invites_per_period()),
                period: InvitePeriod::parse(&group.invite_period).unwrap_or_default(),
            },
            None => Self {
                limit: settings.max_invites_per_period(),
                period: settings.invite_period(),
            },
        }
    }

    pub fn unlimited(self) -> bool {
        self.limit == 0
    }

    /// Invites issued in the current period for `group_id`.
    pub async fn used(
        self,
        pool: &SqlitePool,
        group_id: Option<i64>,
        now: OffsetDateTime,
    ) -> Result<i64> {
        invite_issuance::used(pool, group_id, &self.period.window_start(now))
            .await
            .map_err(AppError::from)
    }

    /// Slots left in the current period; `None` when unlimited.
    pub async fn remaining(
        self,
        pool: &SqlitePool,
        group_id: Option<i64>,
        now: OffsetDateTime,
    ) -> Result<Option<i64>> {
        if self.unlimited() {
            return Ok(None);
        }
        let used = self.used(pool, group_id, now).await?;
        Ok(Some((self.limit - used).max(0)))
    }

    /// Claim one slot. Returns the ledger row id, or `None` when the period is
    /// exhausted. `quota.limit == 0` always grants.
    pub async fn reserve(
        self,
        pool: &SqlitePool,
        group_id: Option<i64>,
        actor_id: Option<i64>,
        now: OffsetDateTime,
    ) -> Result<Option<i64>> {
        let window_start = self.period.window_start(now);
        invite_issuance::reserve(
            pool,
            group_id,
            actor_id,
            self.limit,
            &window_start,
            &clock::to_iso(now),
        )
        .await
        .map_err(AppError::from)
    }

    /// User facing error when the quota is used up.
    pub fn exceeded(self) -> AppError {
        AppError::forbidden(format!(
            "invite quota reached: at most {} invite codes per {}",
            self.limit,
            self.period.as_str()
        ))
    }
}

/// Used/limit snapshot for the admin UI and API.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InviteUsage {
    pub quota: InviteQuota,
    pub used: i64,
}

impl InviteUsage {
    pub async fn load(
        quota: InviteQuota,
        pool: &SqlitePool,
        group_id: Option<i64>,
        now: OffsetDateTime,
    ) -> Result<Self> {
        Ok(Self {
            quota,
            used: quota.used(pool, group_id, now).await?,
        })
    }

    pub fn unlimited(self) -> bool {
        self.quota.unlimited()
    }

    /// Remaining slots; `None` when unlimited.
    pub fn remaining(self) -> Option<i64> {
        if self.unlimited() {
            None
        } else {
            Some((self.quota.limit - self.used).max(0))
        }
    }

    pub fn exhausted(self) -> bool {
        self.remaining() == Some(0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use time::macros::datetime;

    #[test]
    fn parses_periods_case_insensitively() {
        assert_eq!(InvitePeriod::parse(" DAY "), Some(InvitePeriod::Day));
        assert_eq!(InvitePeriod::parse("Week"), Some(InvitePeriod::Week));
        assert_eq!(InvitePeriod::parse("month"), Some(InvitePeriod::Month));
        assert_eq!(InvitePeriod::parse("yearly"), None);
        assert_eq!(InvitePeriod::default(), InvitePeriod::Month);
    }

    #[test]
    fn day_window_starts_at_midnight_utc() {
        let now = datetime!(2026-03-18 15:42:07 UTC);
        assert_eq!(
            InvitePeriod::Day.window_start(now),
            "2026-03-18T00:00:00.000Z"
        );
    }

    #[test]
    fn week_window_starts_on_monday() {
        // 2026-03-18 is a Wednesday; the ISO week began on the 16th.
        let now = datetime!(2026-03-18 15:42:07 UTC);
        assert_eq!(
            InvitePeriod::Week.window_start(now),
            "2026-03-16T00:00:00.000Z"
        );
        // A Monday is its own week start, including the time of day reset.
        let monday = datetime!(2026-03-16 00:00:00 UTC);
        assert_eq!(
            InvitePeriod::Week.window_start(monday),
            "2026-03-16T00:00:00.000Z"
        );
        // A Sunday belongs to the week that started six days earlier.
        let sunday = datetime!(2026-03-22 23:59:59 UTC);
        assert_eq!(
            InvitePeriod::Week.window_start(sunday),
            "2026-03-16T00:00:00.000Z"
        );
    }

    #[test]
    fn month_window_starts_on_the_first() {
        let now = datetime!(2026-03-31 23:59:59 UTC);
        assert_eq!(
            InvitePeriod::Month.window_start(now),
            "2026-03-01T00:00:00.000Z"
        );
        let january = datetime!(2026-01-01 00:00:00 UTC);
        assert_eq!(
            InvitePeriod::Month.window_start(january),
            "2026-01-01T00:00:00.000Z"
        );
    }

    #[test]
    fn quota_semantics_match_the_other_quotas() {
        let quota = InviteQuota {
            limit: 0,
            period: InvitePeriod::Month,
        };
        assert!(quota.unlimited());

        let limited = InviteQuota {
            limit: 3,
            period: InvitePeriod::Week,
        };
        assert!(!limited.unlimited());
        let usage = InviteUsage {
            quota: limited,
            used: 2,
        };
        assert_eq!(usage.remaining(), Some(1));
        assert!(!usage.exhausted());
        let full = InviteUsage {
            quota: limited,
            used: 3,
        };
        assert_eq!(full.remaining(), Some(0));
        assert!(full.exhausted());
        // Over-use (for example after lowering the quota) never goes negative.
        let over = InviteUsage {
            quota: limited,
            used: 9,
        };
        assert_eq!(over.remaining(), Some(0));
    }

    #[test]
    fn exceeded_error_carries_the_limit_and_period() {
        let error = InviteQuota {
            limit: 5,
            period: InvitePeriod::Month,
        }
        .exceeded();
        assert!(error
            .to_string()
            .contains("at most 5 invite codes per month"));
    }
}
