//! In-memory token bucket rate limiter.
//!
//! Deliberately process local: a self hosted single container does not need
//! Redis, and the limits only guard login/registration/unlock endpoints.

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::Instant;

use crate::services::secret;

/// Buckets idle for longer than this are pruned.
const IDLE_TTL_SECS: u64 = 900;
/// Hard cap on tracked keys, protects memory against key flooding.
const MAX_BUCKETS: usize = 50_000;
/// Prune every N checks.
const PRUNE_INTERVAL: u64 = 512;

#[derive(Debug, Clone, Copy)]
struct Bucket {
    tokens: f64,
    updated: Instant,
}

#[derive(Debug, Default)]
struct Inner {
    buckets: HashMap<String, Bucket>,
    checks: u64,
}

/// Drop the `count` least recently used buckets so the map stays bounded even
/// under a key-flooding attempt.
fn evict_oldest(buckets: &mut HashMap<String, Bucket>, count: usize) {
    if buckets.is_empty() || count == 0 {
        return;
    }
    let mut ages: Vec<(String, Instant)> = buckets
        .iter()
        .map(|(key, bucket)| (key.clone(), bucket.updated))
        .collect();
    ages.sort_by_key(|(_, updated)| *updated);
    for (key, _) in ages.into_iter().take(count) {
        buckets.remove(&key);
    }
}

/// Cheap, thread safe token bucket limiter.
#[derive(Debug)]
pub struct RateLimiter {
    inner: Mutex<Inner>,
}

impl Default for RateLimiter {
    fn default() -> Self {
        Self::new()
    }
}

impl RateLimiter {
    pub fn new() -> Self {
        Self {
            inner: Mutex::new(Inner::default()),
        }
    }

    /// Consume one token for `key`.
    ///
    /// `capacity` is the burst size, `refill_per_second` the sustained rate.
    /// Returns `Err(retry_after_seconds)` when the bucket is empty.
    pub fn check(&self, key: &str, capacity: f64, refill_per_second: f64) -> Result<(), u64> {
        let now = Instant::now();
        let mut inner = match self.inner.lock() {
            Ok(guard) => guard,
            // A poisoned limiter must not take the whole service down: fail open.
            Err(poisoned) => poisoned.into_inner(),
        };

        inner.checks = inner.checks.wrapping_add(1);
        if inner.checks % PRUNE_INTERVAL == 0 || inner.buckets.len() > MAX_BUCKETS {
            inner
                .buckets
                .retain(|_, bucket| now.duration_since(bucket.updated).as_secs() < IDLE_TTL_SECS);
        }

        // Keys are hashed: they embed attacker controlled text (an email
        // address or a path segment) and would otherwise be retained verbatim
        // for the whole idle window.
        let key = secret::sha256_hex(key);
        if !inner.buckets.contains_key(&key) && inner.buckets.len() >= MAX_BUCKETS {
            evict_oldest(&mut inner.buckets, MAX_BUCKETS / 10);
        }
        let bucket = inner.buckets.entry(key).or_insert(Bucket {
            tokens: capacity,
            updated: now,
        });

        let elapsed = now.duration_since(bucket.updated).as_secs_f64();
        bucket.tokens = (bucket.tokens + elapsed * refill_per_second).min(capacity);
        bucket.updated = now;

        if bucket.tokens >= 1.0 {
            bucket.tokens -= 1.0;
            Ok(())
        } else {
            let missing = 1.0 - bucket.tokens;
            let wait = if refill_per_second > 0.0 {
                (missing / refill_per_second).ceil() as u64
            } else {
                60
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

    /// Forget every bucket (used by tests).
    pub fn reset(&self) {
        if let Ok(mut inner) = self.inner.lock() {
            inner.buckets.clear();
        }
    }
}

/// Named limits used across the application.
pub mod limits {
    /// Login: 10 attempts per 5 minutes per (email, IP).
    pub const LOGIN_CAPACITY: f64 = 10.0;
    pub const LOGIN_REFILL_PER_SEC: f64 = 10.0 / 300.0;

    /// Login: 40 attempts per 5 minutes per IP, across every account. Caps the
    /// amount of Argon2 work a single host can request.
    pub const LOGIN_IP_CAPACITY: f64 = 40.0;
    pub const LOGIN_IP_REFILL_PER_SEC: f64 = 40.0 / 300.0;

    /// Registration: 5 attempts per hour per IP.
    pub const REGISTER_CAPACITY: f64 = 5.0;
    pub const REGISTER_REFILL_PER_SEC: f64 = 5.0 / 3600.0;

    /// Unlock: 10 attempts per 5 minutes per (slug, IP).
    pub const UNLOCK_CAPACITY: f64 = 10.0;
    pub const UNLOCK_REFILL_PER_SEC: f64 = 10.0 / 300.0;

    /// Password change: 10 per hour per user.
    pub const PASSWORD_CAPACITY: f64 = 10.0;
    pub const PASSWORD_REFILL_PER_SEC: f64 = 10.0 / 3600.0;

    /// Shortening: 120 new links per hour per user.
    pub const CREATE_URL_CAPACITY: f64 = 120.0;
    pub const CREATE_URL_REFILL_PER_SEC: f64 = 120.0 / 3600.0;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn allows_up_to_capacity_then_blocks() {
        let limiter = RateLimiter::new();
        for _ in 0..3 {
            assert!(limiter.check("k", 3.0, 0.0).is_ok());
        }
        let retry = limiter.check("k", 3.0, 0.0).expect_err("blocked");
        assert!(retry >= 1);
    }

    #[test]
    fn keys_are_independent() {
        let limiter = RateLimiter::new();
        assert!(limiter.check("a", 1.0, 0.0).is_ok());
        assert!(limiter.check("b", 1.0, 0.0).is_ok());
        assert!(limiter.check("a", 1.0, 0.0).is_err());
    }

    #[test]
    fn refills_over_time() {
        let limiter = RateLimiter::new();
        assert!(limiter.check("k", 1.0, 1000.0).is_ok());
        assert!(limiter.check("k", 1.0, 1000.0).is_err());
        std::thread::sleep(std::time::Duration::from_millis(5));
        assert!(limiter.check("k", 1.0, 1000.0).is_ok());
    }

    #[test]
    fn keys_are_hashed_and_the_map_stays_bounded() {
        let limiter = RateLimiter::new();
        let huge = "x".repeat(10_000);
        assert!(limiter.check(&huge, 1.0, 0.0).is_ok());
        assert!(
            limiter.check(&huge, 1.0, 0.0).is_err(),
            "the digest keeps the bucket identity"
        );
        assert_eq!(
            limiter.tracked_keys(),
            1,
            "a huge key is stored as a digest"
        );

        for index in 0..(MAX_BUCKETS + 100) {
            let _ = limiter.check(&format!("flood-{index}"), 1.0, 1.0);
        }
        assert!(
            limiter.tracked_keys() <= MAX_BUCKETS,
            "the map stays bounded"
        );
        limiter.reset();
        assert_eq!(limiter.tracked_keys(), 0);
    }

    #[test]
    fn tracks_bounded_number_of_keys() {
        let limiter = RateLimiter::new();
        for index in 0..10 {
            let _ = limiter.check(&format!("key-{index}"), 1.0, 1.0);
        }
        assert_eq!(limiter.tracked_keys(), 10);
    }
}
