//! Keeps the container health check from flagging a migrating container.
//!
//! The server does not listen until migrations are done, and a second process
//! waits up to `LOCK_WAIT` for the first one's migration lock. Probe failures
//! inside the health check's start period are not counted, so a start period
//! shorter than that wait marks a container that is only waiting as
//! `unhealthy` — which also fails `depends_on: condition: service_healthy` in
//! the TLS overlay.
//!
//! The Dockerfile `HEALTHCHECK` and the `healthcheck:` block in
//! `docker-compose.yml` each carry their own start period (compose overrides
//! the image's), so both are checked, and they must agree.

use std::time::Duration;

use urlet::db::migrate::LOCK_WAIT;

/// Both files compiled in, so a rename fails the build rather than the test.
const DOCKERFILE: &str = include_str!("../Dockerfile");
const COMPOSE: &str = include_str!("../docker-compose.yml");

/// What the start period must add on top of `LOCK_WAIT`: the final attempt can
/// run on to `busy_timeout` (5 s) past the deadline, and the rest of startup
/// (opening the pool, seeding, binding) needs a few more seconds.
const HEADROOM: Duration = Duration::from_secs(30);

/// Parse the Go-style durations Docker accepts, limited to `h`, `m` and `s`
/// components such as `150s` or `2m30s`. Anything else fails the test instead
/// of being guessed at.
fn parse_duration(text: &str) -> Duration {
    let mut total = Duration::ZERO;
    let mut digits = String::new();
    for ch in text.chars() {
        if ch.is_ascii_digit() {
            digits.push(ch);
            continue;
        }
        let amount: u64 = digits
            .parse()
            .unwrap_or_else(|_| panic!("no number before `{ch}` in duration `{text}`"));
        digits.clear();
        total += Duration::from_secs(match ch {
            'h' => amount * 3600,
            'm' => amount * 60,
            's' => amount,
            other => panic!("unsupported unit `{other}` in duration `{text}`"),
        });
    }
    assert!(digits.is_empty(), "duration `{text}` has no unit");
    assert!(!total.is_zero(), "duration `{text}` is zero");
    total
}

/// The start period of the Dockerfile's `HEALTHCHECK` instruction.
fn dockerfile_start_period() -> Duration {
    // Fold `\` continuations so the flags and `CMD` read as one instruction.
    let folded = DOCKERFILE.replace("\\\n", " ");
    let mut instructions = folded
        .lines()
        .filter(|line| line.trim_start().starts_with("HEALTHCHECK "));
    let instruction = instructions
        .next()
        .expect("the Dockerfile has no HEALTHCHECK instruction");
    assert!(
        instructions.next().is_none(),
        "more than one HEALTHCHECK instruction: only the last one takes effect"
    );
    let value = instruction
        .split_whitespace()
        .find_map(|token| token.strip_prefix("--start-period="))
        .expect("the Dockerfile HEALTHCHECK has no --start-period");
    parse_duration(value)
}

/// The `start_period` inside the `healthcheck:` block of the compose file.
fn compose_start_period() -> Duration {
    let mut lines = COMPOSE.lines();
    let header = lines
        .by_ref()
        .find(|line| line.trim() == "healthcheck:")
        .expect("docker-compose.yml has no healthcheck block");
    let indent = header.len() - header.trim_start().len();

    // The block ends at the first non-blank line indented no deeper than its key.
    let value = lines
        .take_while(|line| line.trim().is_empty() || line.len() - line.trim_start().len() > indent)
        .find_map(|line| line.trim().strip_prefix("start_period:"))
        .expect("the compose healthcheck has no start_period");
    parse_duration(value.split('#').next().unwrap().trim().trim_matches('"'))
}

#[test]
fn durations_parse_like_docker_reads_them() {
    assert_eq!(parse_duration("150s"), Duration::from_secs(150));
    assert_eq!(parse_duration("2m30s"), Duration::from_secs(150));
    assert_eq!(parse_duration("1h"), Duration::from_secs(3600));
}

#[test]
fn the_dockerfile_start_period_outlasts_the_migration_lock_wait() {
    let start_period = dockerfile_start_period();
    assert!(
        start_period >= LOCK_WAIT + HEADROOM,
        "Dockerfile --start-period={start_period:?} is shorter than the migration lock wait \
         {LOCK_WAIT:?} plus {HEADROOM:?} of headroom; a container waiting for the lock would \
         be reported unhealthy"
    );
}

#[test]
fn the_compose_start_period_outlasts_the_migration_lock_wait() {
    let start_period = compose_start_period();
    assert!(
        start_period >= LOCK_WAIT + HEADROOM,
        "docker-compose.yml start_period={start_period:?} is shorter than the migration lock \
         wait {LOCK_WAIT:?} plus {HEADROOM:?} of headroom; a container waiting for the lock \
         would be reported unhealthy"
    );
}

#[test]
fn compose_and_the_dockerfile_agree_on_the_start_period() {
    assert_eq!(
        compose_start_period(),
        dockerfile_start_period(),
        "docker-compose.yml overrides the image's health check; keep the two start periods equal"
    );
}
