//! In-memory token bucket rate limiter.
//!
//! Deliberately process local: a self hosted single container does not need
//! Redis. Every purpose (login, registration, unlock, ...) owns a separate
//! [`RateLimiter`] in [`Limiters`], with its own limit and its own key cap, so
//! a burst of keys in one of them cannot crowd out the others.
//!
//! Buckets are pruned once they have refilled completely: a full bucket
//! behaves exactly like one that was never created. When the map reaches its
//! cap, the tenth of the buckets that refill soonest is dropped to make room,
//! and the new key is always admitted. Every bucket in one limiter shares the
//! same limit, so the bucket that owes the most tokens refills last and is
//! dropped last: a burst of new keys, each spending one token, is evicted
//! before a bucket that has been drained.

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use sha2::{Digest, Sha256};

/// Default cap on tracked keys per limiter, protects memory against key flooding.
pub const MAX_BUCKETS: usize = 50_000;
/// Drop refilled buckets every N checks.
const PRUNE_INTERVAL: u64 = 512;
/// A full map drops `1 / EVICT_DIVISOR` of its buckets at once, so the linear
/// selection runs once per that many insertions.
const EVICT_DIVISOR: usize = 10;
/// Retry hint for a limit that never refills.
const FALLBACK_RETRY_SECS: u64 = 60;

/// Burst size and sustained rate of one limit.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Limit {
    pub capacity: f64,
    pub refill_per_sec: f64,
}

/// Eviction order of a bucket, see [`Bucket::eviction_rank`].
type EvictionRank = (bool, Option<Instant>);

#[derive(Debug, Clone, Copy)]
struct Bucket {
    tokens: f64,
    updated: Instant,
}

impl Bucket {
    /// Tokens available at `now`, capped at the burst size.
    fn level(&self, now: Instant, limit: Limit) -> f64 {
        let elapsed = now.saturating_duration_since(self.updated).as_secs_f64();
        (self.tokens + elapsed * limit.refill_per_sec).min(limit.capacity)
    }

    /// The moment the bucket is full again, following the refill in
    /// [`Bucket::level`]; `None` when it never refills.
    fn full_at(&self, limit: Limit) -> Option<Instant> {
        let missing = limit.capacity - self.tokens;
        if missing <= 0.0 {
            return Some(self.updated);
        }
        if limit.refill_per_sec <= 0.0 {
            return None;
        }
        Duration::try_from_secs_f64(missing / limit.refill_per_sec)
            .ok()
            .and_then(|wait| self.updated.checked_add(wait))
    }

    /// Eviction order: the bucket that refills soonest sorts first, one that
    /// never refills sorts last.
    fn eviction_rank(&self, limit: Limit) -> EvictionRank {
        let full_at = self.full_at(limit);
        (full_at.is_none(), full_at)
    }
}

fn digest(key: &str) -> [u8; 32] {
    Sha256::digest(key.as_bytes()).into()
}

#[derive(Debug, Default)]
struct Inner {
    buckets: HashMap<[u8; 32], Bucket>,
    checks: u64,
}

impl Inner {
    /// Drop every bucket that has refilled completely.
    fn prune(&mut self, now: Instant, limit: Limit) {
        self.buckets
            .retain(|_, bucket| bucket.full_at(limit).is_none_or(|at| at > now));
    }

    /// Drop the `count` buckets that refill soonest; refilled buckets go first.
    fn evict(&mut self, count: usize, limit: Limit) {
        let count = count.min(self.buckets.len());
        if count == 0 {
            return;
        }
        let mut ranked: Vec<(EvictionRank, [u8; 32])> = self
            .buckets
            .iter()
            .map(|(key, bucket)| (bucket.eviction_rank(limit), *key))
            .collect();
        if count < ranked.len() {
            // Linear-time selection of the first `count`, no full sort needed.
            ranked.select_nth_unstable_by_key(count - 1, |(rank, _)| *rank);
        }
        for (_, key) in &ranked[..count] {
            self.buckets.remove(key);
        }
    }
}

/// Cheap, thread safe token bucket limiter for one purpose.
#[derive(Debug)]
pub struct RateLimiter {
    limit: Limit,
    max_buckets: usize,
    inner: Mutex<Inner>,
}

impl RateLimiter {
    pub fn new(limit: Limit) -> Self {
        Self::with_max_buckets(limit, MAX_BUCKETS)
    }

    /// A limiter tracking at most `max_buckets` keys.
    pub fn with_max_buckets(limit: Limit, max_buckets: usize) -> Self {
        Self {
            limit,
            max_buckets: max_buckets.max(1),
            inner: Mutex::new(Inner::default()),
        }
    }

    pub fn limit(&self) -> Limit {
        self.limit
    }

    /// Consume one token for `key`.
    ///
    /// Returns `Err(retry_after_seconds)` when the bucket is empty.
    pub fn check(&self, key: &str) -> Result<(), u64> {
        self.check_at(key, Instant::now())
    }

    fn check_at(&self, key: &str, now: Instant) -> Result<(), u64> {
        let limit = self.limit;
        let mut inner = match self.inner.lock() {
            Ok(guard) => guard,
            // A poisoned limiter must not take the whole service down: fail open.
            Err(poisoned) => poisoned.into_inner(),
        };

        inner.checks = inner.checks.wrapping_add(1);
        if inner.checks % PRUNE_INTERVAL == 0 {
            inner.prune(now, limit);
        }

        // Keys are hashed: they embed attacker controlled text (an email
        // address or a path segment) and would otherwise be retained verbatim
        // until the bucket refills.
        let key = digest(key);
        if !inner.buckets.contains_key(&key) && inner.buckets.len() >= self.max_buckets {
            inner.evict((self.max_buckets / EVICT_DIVISOR).max(1), limit);
        }

        let bucket = inner.buckets.entry(key).or_insert(Bucket {
            tokens: limit.capacity,
            updated: now,
        });
        let level = bucket.level(now, limit);
        // `now` is read before the lock is taken, so a concurrent check may
        // already have moved the bucket further: never move it back in time,
        // or the next refill would count the same interval twice.
        bucket.updated = bucket.updated.max(now);
        if level >= 1.0 {
            bucket.tokens = level - 1.0;
            Ok(())
        } else {
            bucket.tokens = level;
            let wait = if limit.refill_per_sec > 0.0 {
                ((1.0 - level) / limit.refill_per_sec).ceil() as u64
            } else {
                FALLBACK_RETRY_SECS
            };
            Err(wait.max(1))
        }
    }

    /// Number of tracked keys (used by tests).
    pub fn tracked_keys(&self) -> usize {
        self.inner
            .lock()
            .map(|inner| inner.buckets.len())
            .unwrap_or(0)
    }

    /// Is a bucket tracked for `key`? Does not touch the bucket.
    #[cfg(test)]
    fn is_tracked(&self, key: &str) -> bool {
        let key = digest(key);
        self.inner
            .lock()
            .map(|inner| inner.buckets.contains_key(&key))
            .unwrap_or(false)
    }

    /// Forget every bucket (used by tests).
    pub fn reset(&self) {
        if let Ok(mut inner) = self.inner.lock() {
            *inner = Inner::default();
        }
    }
}

/// One limiter per purpose.
///
/// Login and registration keys are derived from the client network (plus the
/// submitted email for the per-account login bucket), so anyone can create
/// new ones; unlock keys exist only for an active, password protected link.
/// The remaining limiters are keyed by signed-in account.
#[derive(Debug)]
pub struct Limiters {
    /// Login attempts per client network, across every account.
    pub login_ip: RateLimiter,
    /// Login attempts per (client network, email).
    pub login: RateLimiter,
    /// Registrations per client network.
    pub register: RateLimiter,
    /// Unlock attempts per (password protected link, client network).
    pub unlock: RateLimiter,
    /// Password changes per user.
    pub password: RateLimiter,
    /// New links per user, shared by every create path.
    pub create_url: RateLimiter,
    /// SNS preview image uploads per user.
    pub preview_upload: RateLimiter,
    /// Member-issued invite codes per user.
    pub create_invite: RateLimiter,
}

impl Default for Limiters {
    fn default() -> Self {
        Self::new()
    }
}

impl Limiters {
    pub fn new() -> Self {
        Self::with_max_buckets(MAX_BUCKETS)
    }

    /// Every limiter capped at `max_buckets` keys.
    pub fn with_max_buckets(max_buckets: usize) -> Self {
        let limiter = |limit| RateLimiter::with_max_buckets(limit, max_buckets);
        Self {
            login_ip: limiter(limits::LOGIN_IP),
            login: limiter(limits::LOGIN),
            register: limiter(limits::REGISTER),
            unlock: limiter(limits::UNLOCK),
            password: limiter(limits::PASSWORD),
            create_url: limiter(limits::CREATE_URL),
            preview_upload: limiter(limits::PREVIEW_UPLOAD),
            create_invite: limiter(limits::CREATE_INVITE),
        }
    }

    fn all(&self) -> [&RateLimiter; 8] {
        // Exhaustive on purpose: a new limiter that is not listed fails to compile.
        let Self {
            login_ip,
            login,
            register,
            unlock,
            password,
            create_url,
            preview_upload,
            create_invite,
        } = self;
        [
            login_ip,
            login,
            register,
            unlock,
            password,
            create_url,
            preview_upload,
            create_invite,
        ]
    }

    /// Forget every bucket in every limiter (used by tests).
    pub fn reset(&self) {
        for limiter in self.all() {
            limiter.reset();
        }
    }
}

/// Named limits used across the application.
pub mod limits {
    use super::Limit;

    /// Login: 10 attempts per 5 minutes per (email, IP).
    pub const LOGIN: Limit = Limit {
        capacity: 10.0,
        refill_per_sec: 10.0 / 300.0,
    };

    /// Login: 40 attempts per 5 minutes per IP, across every account. Caps the
    /// amount of Argon2 work a single host can request.
    pub const LOGIN_IP: Limit = Limit {
        capacity: 40.0,
        refill_per_sec: 40.0 / 300.0,
    };

    /// Registration: 5 attempts per hour per IP.
    pub const REGISTER: Limit = Limit {
        capacity: 5.0,
        refill_per_sec: 5.0 / 3600.0,
    };

    /// Unlock: 10 attempts per 5 minutes per (link, IP).
    pub const UNLOCK: Limit = Limit {
        capacity: 10.0,
        refill_per_sec: 10.0 / 300.0,
    };

    /// Password change: 10 per hour per user.
    pub const PASSWORD: Limit = Limit {
        capacity: 10.0,
        refill_per_sec: 10.0 / 3600.0,
    };

    /// Shortening: 120 new links per hour per user.
    pub const CREATE_URL: Limit = Limit {
        capacity: 120.0,
        refill_per_sec: 120.0 / 3600.0,
    };

    /// SNS preview uploads: the same allowance as shortening, counted apart.
    pub const PREVIEW_UPLOAD: Limit = CREATE_URL;

    /// Member-issued invite codes: 20 per hour per user. The group quota is the
    /// primary control; this caps a runaway client on groups set to unlimited.
    pub const CREATE_INVITE: Limit = Limit {
        capacity: 20.0,
        refill_per_sec: 20.0 / 3600.0,
    };
}

#[cfg(test)]
mod tests {
    use super::*;

    const ONE_PER_SEC: Limit = Limit {
        capacity: 1.0,
        refill_per_sec: 1.0,
    };
    /// Two tokens, one per second: a drained bucket refills in 2 s, one that
    /// spent a single token in 1 s.
    const TWO_PER_SEC_BURST: Limit = Limit {
        capacity: 2.0,
        refill_per_sec: 1.0,
    };
    const NO_REFILL: Limit = Limit {
        capacity: 1.0,
        refill_per_sec: 0.0,
    };

    fn at(base: Instant, millis: u64) -> Instant {
        base + Duration::from_millis(millis)
    }

    #[test]
    fn allows_up_to_capacity_then_blocks() {
        let limiter = RateLimiter::new(Limit {
            capacity: 3.0,
            refill_per_sec: 0.0,
        });
        for _ in 0..3 {
            assert!(limiter.check("k").is_ok());
        }
        let retry = limiter.check("k").expect_err("blocked");
        assert!(retry >= 1);
    }

    #[test]
    fn keys_are_independent() {
        let limiter = RateLimiter::new(NO_REFILL);
        assert!(limiter.check("a").is_ok());
        assert!(limiter.check("b").is_ok());
        assert!(limiter.check("a").is_err());
    }

    #[test]
    fn refills_over_time() {
        let limiter = RateLimiter::new(ONE_PER_SEC);
        let base = Instant::now();
        assert!(limiter.check_at("k", base).is_ok());
        assert_eq!(limiter.check_at("k", at(base, 100)), Err(1));
        assert!(limiter.check_at("k", at(base, 1_100)).is_ok());
    }

    #[test]
    fn keys_are_hashed_and_the_map_stays_bounded() {
        let limiter = RateLimiter::with_max_buckets(NO_REFILL, 64);
        let huge = "x".repeat(10_000);
        assert!(limiter.check(&huge).is_ok());
        assert!(
            limiter.check(&huge).is_err(),
            "the digest keeps the bucket identity"
        );
        assert_eq!(
            limiter.tracked_keys(),
            1,
            "a huge key is stored as a digest"
        );

        for index in 0..1_000 {
            assert!(
                limiter.check(&format!("flood-{index}")).is_ok(),
                "a new key is always admitted"
            );
            assert!(limiter.tracked_keys() <= 64, "the map stays bounded");
        }
        limiter.reset();
        assert_eq!(limiter.tracked_keys(), 0);
    }

    #[test]
    fn exhausted_bucket_survives_a_full_map() {
        let limiter = RateLimiter::with_max_buckets(TWO_PER_SEC_BURST, 8);
        let base = Instant::now();
        assert!(limiter.check_at("exhausted", base).is_ok());
        assert!(limiter.check_at("exhausted", base).is_ok());
        assert!(limiter.check_at("exhausted", base).is_err());
        for index in 0..32 {
            assert!(limiter
                .check_at(&format!("other-{index}"), at(base, 10))
                .is_ok());
        }
        assert_eq!(limiter.tracked_keys(), 8);
        assert!(
            limiter.check_at("exhausted", at(base, 20)).is_err(),
            "an exhausted bucket must not be reset by new keys"
        );
    }

    #[test]
    fn default_cap_keeps_an_exhausted_bucket() {
        let limiter = RateLimiter::new(TWO_PER_SEC_BURST);
        let base = Instant::now();
        assert!(limiter.check_at("exhausted", base).is_ok());
        assert!(limiter.check_at("exhausted", base).is_ok());
        // Overfill by a tenth so the full-map eviction runs at least once.
        for index in 0..(MAX_BUCKETS + MAX_BUCKETS / 10) {
            let _ = limiter.check_at(&format!("other-{index}"), at(base, 10));
        }
        assert!(limiter.tracked_keys() <= MAX_BUCKETS);
        assert!(limiter.is_tracked("exhausted"));
        assert!(limiter.check_at("exhausted", at(base, 20)).is_err());
    }

    #[test]
    fn partially_consumed_bucket_outlives_fresher_ones() {
        let limit = Limit {
            capacity: 3.0,
            refill_per_sec: 1.0,
        };
        let limiter = RateLimiter::with_max_buckets(limit, 2);
        let base = Instant::now();
        // `partial` spends 2 tokens and refills in 2 s; `fresh` spends 1 and refills in 1 s.
        assert!(limiter.check_at("partial", base).is_ok());
        assert!(limiter.check_at("partial", base).is_ok());
        assert!(limiter.check_at("fresh", base).is_ok());

        // The full map still admits the newcomer and evicts `fresh`, which refills sooner.
        assert!(limiter.check_at("newcomer", at(base, 500)).is_ok());
        assert!(!limiter.is_tracked("fresh"));
        assert!(limiter.is_tracked("partial"));
        // `partial` now holds 1 + 0.5 = 1.5 tokens: one more attempt passes, the next is refused.
        assert!(limiter.check_at("partial", at(base, 500)).is_ok());
        assert!(limiter.check_at("partial", at(base, 500)).is_err());
    }

    #[test]
    fn a_full_map_admits_newcomers_and_evicts_the_earliest_refill() {
        let limiter = RateLimiter::with_max_buckets(ONE_PER_SEC, 3);
        let base = Instant::now();
        assert!(limiter.check_at("a", base).is_ok());
        assert!(limiter.check_at("b", at(base, 200)).is_ok());
        assert!(limiter.check_at("c", at(base, 400)).is_ok());

        // Refill moments: a = 1.0 s, b = 1.2 s, c = 1.4 s, so `a` is evicted.
        assert!(limiter.check_at("d", at(base, 500)).is_ok());
        assert_eq!(limiter.tracked_keys(), 3);
        assert!(!limiter.is_tracked("a"));
        assert!(limiter.is_tracked("b") && limiter.is_tracked("c") && limiter.is_tracked("d"));
        assert!(limiter.check_at("b", at(base, 600)).is_err());

        // A refilled bucket goes first, even though it was used more recently.
        assert!(limiter.check_at("e", at(base, 1_300)).is_ok());
        assert!(!limiter.is_tracked("b"));
        assert!(limiter.is_tracked("c"));
    }

    #[test]
    fn exhausted_bucket_goes_only_after_every_other_refills_later() {
        let limiter = RateLimiter::with_max_buckets(TWO_PER_SEC_BURST, 3);
        let base = Instant::now();
        // `drained` spends both tokens and refills at 2.0 s.
        assert!(limiter.check_at("drained", base).is_ok());
        assert!(limiter.check_at("drained", base).is_ok());
        // Each new key spends one token and refills 1 s after it was written.
        let steps = [
            ("k1", 500),   // 1.5s
            ("k2", 600),   // 1.6s
            ("k3", 700),   // 1.7s, evicts k1
            ("k4", 1_200), // 2.2s, evicts k2
            ("k5", 1_300), // 2.3s, evicts k3; every other bucket now refills after `drained`
        ];
        for (key, millis) in steps {
            assert!(limiter.check_at(key, at(base, millis)).is_ok());
            assert!(
                limiter.is_tracked("drained"),
                "kept while another bucket refills sooner ({key})"
            );
        }
        assert!(!limiter.is_tracked("k1") && !limiter.is_tracked("k2"));

        // Only now that `drained` refills first is it evicted; it has regained 1.4 tokens.
        assert!(limiter.check_at("k6", at(base, 1_400)).is_ok());
        assert!(!limiter.is_tracked("drained"));
        assert!(limiter.is_tracked("k4") && limiter.is_tracked("k5"));
    }

    #[test]
    fn a_full_map_drops_a_tenth_of_its_buckets_at_once() {
        let limiter = RateLimiter::with_max_buckets(ONE_PER_SEC, 20);
        let base = Instant::now();
        for index in 0..20 {
            assert!(limiter
                .check_at(&format!("key-{index}"), at(base, index))
                .is_ok());
        }
        assert_eq!(limiter.tracked_keys(), 20);
        assert!(limiter.check_at("one-more", at(base, 30)).is_ok());
        // One eviction drops 20 / 10 = 2 buckets, so the next insertion needs none.
        assert_eq!(limiter.tracked_keys(), 19);
        assert!(!limiter.is_tracked("key-0") && !limiter.is_tracked("key-1"));
        assert!(limiter.check_at("two-more", at(base, 30)).is_ok());
        assert_eq!(limiter.tracked_keys(), 20);
        assert!(limiter.is_tracked("key-2"));
    }

    #[test]
    fn periodic_prune_drops_only_refilled_buckets() {
        let limiter = RateLimiter::with_max_buckets(ONE_PER_SEC, MAX_BUCKETS);
        let base = Instant::now();
        assert!(limiter.check_at("old", base).is_ok());
        for index in 0..(PRUNE_INTERVAL - 2) {
            assert!(limiter
                .check_at(&format!("recent-{index}"), at(base, 1_500))
                .is_ok());
        }
        // Check number PRUNE_INTERVAL prunes: `old` has refilled and goes, the rest still owe.
        assert!(limiter.check_at("recent-0", at(base, 1_500)).is_err());
        assert_eq!(limiter.tracked_keys(), (PRUNE_INTERVAL - 2) as usize);
        assert!(!limiter.is_tracked("old"));
    }

    #[test]
    fn idle_but_unrefilled_bucket_survives_prune() {
        let limiter = RateLimiter::new(limits::REGISTER);
        let base = Instant::now();
        for _ in 0..5 {
            assert!(limiter.check_at("drained", base).is_ok());
        }
        // Idle for longer than the old 900 s window, yet still far from refilled.
        let later = base + Duration::from_secs(1_000);
        for index in 0..PRUNE_INTERVAL {
            let _ = limiter.check_at(&format!("other-{index}"), later);
        }
        assert!(limiter.is_tracked("drained"));
        // 1000 seconds refill about 1.39 tokens: one more attempt passes, the next is refused.
        assert!(limiter.check_at("drained", later).is_ok());
        assert!(limiter.check_at("drained", later).is_err());
    }

    #[test]
    fn a_flood_in_one_limiter_leaves_the_others_alone() {
        let limiters = Limiters::with_max_buckets(4);
        let capacity = limits::LOGIN_IP.capacity as usize;
        for _ in 0..capacity {
            assert!(limiters.login_ip.check("client").is_ok());
        }
        assert!(limiters.login_ip.check("client").is_err());

        for index in 0..100 {
            assert!(limiters.unlock.check(&format!("link-{index}")).is_ok());
        }
        assert_eq!(limiters.unlock.tracked_keys(), 4);
        assert_eq!(limiters.login_ip.tracked_keys(), 1);
        assert!(
            limiters.login_ip.check("client").is_err(),
            "keys flooding the unlock limiter cannot evict a sign-in bucket"
        );

        limiters.reset();
        for limiter in limiters.all() {
            assert_eq!(limiter.tracked_keys(), 0);
        }
    }

    #[test]
    fn limiters_carry_the_documented_limits() {
        let limiters = Limiters::new();
        assert_eq!(limiters.login.limit(), limits::LOGIN);
        assert_eq!(limiters.login_ip.limit(), limits::LOGIN_IP);
        assert_eq!(limiters.register.limit(), limits::REGISTER);
        assert_eq!(limiters.unlock.limit(), limits::UNLOCK);
        assert_eq!(limiters.password.limit(), limits::PASSWORD);
        assert_eq!(limiters.create_url.limit(), limits::CREATE_URL);
        assert_eq!(limiters.preview_upload.limit(), limits::PREVIEW_UPLOAD);
        assert_eq!(limiters.create_invite.limit(), limits::CREATE_INVITE);
    }
}
