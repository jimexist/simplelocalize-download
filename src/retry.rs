//! Retry policy for the API client and download engine.
//!
//! The actual retry loop is provided by the [`backon`] crate. This module holds
//! the tunable [`RetryPolicy`] and maps it to a [`backon::ExponentialBuilder`]
//! (capped exponential backoff with jitter). Which errors are retryable — and
//! any server-suggested `Retry-After` — is decided by [`crate::error::Error`].

use std::time::Duration;

use backon::ExponentialBuilder;

/// Backoff configuration.
#[derive(Debug, Clone)]
pub struct RetryPolicy {
    /// Maximum number of attempts (including the first).
    pub max_attempts: u32,
    /// Delay for the first retry, grown by `factor` each subsequent retry.
    pub base_delay: Duration,
    /// Upper bound on any single backoff delay.
    pub max_delay: Duration,
    /// Exponential growth factor.
    pub factor: u32,
}

/// Default attempt count. Sized so a short upstream outage rides out unattended:
/// with [`DEFAULT_BASE_DELAY`] and [`DEFAULT_MAX_DELAY`] the un-jittered backoff
/// sums to 91s (1 + 2 + 4 + 8 + 16 + 30 + 30) before giving up — 91-182s once
/// `backon`'s additive jitter is included (see [`RetryPolicy::builder`]).
///
/// Note this budget is per *operation*: the list call pays it once, but each file
/// download pays it too, and files run [`crate::download::DEFAULT_CONCURRENCY`] at
/// a time — so a persistent failure over a large batch costs roughly
/// `(files / concurrency) * budget`.
pub const DEFAULT_MAX_ATTEMPTS: u32 = 8;
/// Default delay before the first retry.
pub const DEFAULT_BASE_DELAY: Duration = Duration::from_secs(1);
/// Default ceiling on any single backoff delay.
pub const DEFAULT_MAX_DELAY: Duration = Duration::from_secs(30);

impl Default for RetryPolicy {
    fn default() -> Self {
        Self {
            max_attempts: DEFAULT_MAX_ATTEMPTS,
            base_delay: DEFAULT_BASE_DELAY,
            max_delay: DEFAULT_MAX_DELAY,
            factor: 2,
        }
    }
}

impl RetryPolicy {
    /// Policy from caller-supplied overrides, each falling back to the default.
    ///
    /// Values are clamped into a sane range rather than rejected: `max_attempts`
    /// to at least 1 (no retries), `factor` to at least 1 (constant delay), and
    /// `max_delay` to at least `base_delay` so a small ceiling cannot silently
    /// shorten the first retry.
    pub fn from_overrides(
        max_attempts: Option<u32>,
        base_delay: Option<Duration>,
        max_delay: Option<Duration>,
        factor: Option<u32>,
    ) -> Self {
        let base_delay = base_delay.unwrap_or(DEFAULT_BASE_DELAY);
        Self {
            max_attempts: max_attempts.unwrap_or(DEFAULT_MAX_ATTEMPTS).max(1),
            base_delay,
            max_delay: max_delay.unwrap_or(DEFAULT_MAX_DELAY).max(base_delay),
            factor: factor.unwrap_or(2).max(1),
        }
    }

    /// Backoff builder for `backon`. `backon` counts *retries*, so the number of
    /// retries is `max_attempts - 1` (the first call is not a retry).
    ///
    /// Note that `backon` applies jitter *after* clamping to `max_delay`, adding
    /// up to 100% of the delay ([`backon::ExponentialBackoff`]), so an individual
    /// sleep can reach `2 * max_delay` and the total up to twice the un-jittered
    /// sum. Callers documenting a budget should quote the range, not the sum.
    pub fn builder(&self) -> ExponentialBuilder {
        ExponentialBuilder::default()
            .with_min_delay(self.base_delay)
            .with_max_delay(self.max_delay)
            .with_factor(self.factor as f32)
            .with_max_times(self.max_attempts.saturating_sub(1) as usize)
            .with_jitter()
    }
}

/// Resolve the delay before the next retry, honoring a server `Retry-After`.
///
/// Two invariants that are easy to get wrong with [`backon::Retryable::adjust`]:
///
/// 1. **`None` means stop.** `backon` passes `None` once the attempt budget is
///    exhausted, and treats a `Some` return as "retry anyway" — so returning the
///    server's hint unconditionally makes any server that sends `Retry-After`
///    retry *forever*, ignoring `max_attempts` entirely. Only ever replace a
///    delay `backon` is still offering.
/// 2. **The hint is clamped.** An unclamped `Retry-After` lets the server choose
///    how long an unattended run sleeps (`Retry-After: 86400` stalls CI for a
///    day, and a near-`u64::MAX` value can overflow the sleep deadline).
pub(crate) fn adjust_delay(
    err: &crate::error::Error,
    dur: Option<Duration>,
    max_delay: Duration,
) -> Option<Duration> {
    dur.map(|computed| err.retry_after().unwrap_or(computed).min(max_delay))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::Error;
    use backon::{BackoffBuilder, Retryable};
    use std::cell::Cell;

    fn retryable(status: u16) -> Error {
        Error::Api {
            status,
            msg: "boom".into(),
            retry_after: None,
        }
    }

    fn fast_policy(max_attempts: u32) -> RetryPolicy {
        RetryPolicy {
            max_attempts,
            base_delay: Duration::from_millis(1),
            max_delay: Duration::from_millis(2),
            factor: 2,
        }
    }

    /// Drives the real `backon` builder rather than re-deriving the formula, so a
    /// bug in [`RetryPolicy::builder`] itself (a swapped min/max, an off-by-one in
    /// `max_times`) fails this test instead of sailing past a restated sum.
    #[test]
    fn default_budget_rides_out_a_short_outage() {
        let policy = RetryPolicy::default();
        let delays: Vec<Duration> = policy.builder().build().collect();

        // `backon` counts retries, not attempts.
        assert_eq!(delays.len(), policy.max_attempts as usize - 1);

        // Jitter is additive and applied *after* the `max_delay` clamp, so each
        // delay lands in [expected, 2 * expected) and no delay exceeds
        // 2 * max_delay. Asserting the range is what keeps the documented budget
        // honest — the un-jittered sum alone would understate it by ~2x.
        let mut expected = policy.base_delay;
        for delay in &delays {
            let floor = expected.min(policy.max_delay);
            assert!(
                *delay >= floor && *delay < floor * 2,
                "delay {delay:?} outside [{floor:?}, {:?})",
                floor * 2
            );
            assert!(*delay < policy.max_delay * 2);
            expected = expected.saturating_mul(policy.factor);
        }

        // Un-jittered sum is 1 + 2 + 4 + 8 + 16 + 30 + 30 = 91s; with jitter the
        // real total is in [91s, 182s).
        let total: Duration = delays.iter().sum();
        assert!(
            total >= Duration::from_secs(91) && total < Duration::from_secs(182),
            "total backoff {total:?} outside the documented [91s, 182s) envelope"
        );
    }

    #[test]
    fn retry_after_cannot_outlive_the_attempt_budget() {
        // `backon` passes `None` when the budget is exhausted; a server hint must
        // not resurrect it. Regression test: returning the hint unconditionally
        // made any `Retry-After`-sending server retry forever.
        let err = Error::Api {
            status: 429,
            msg: "slow down".into(),
            retry_after: Some(Duration::from_secs(1)),
        };
        assert_eq!(adjust_delay(&err, None, Duration::from_secs(30)), None);
    }

    #[test]
    fn retry_after_is_clamped_to_max_delay() {
        let err = Error::Api {
            status: 503,
            msg: "back later".into(),
            retry_after: Some(Duration::from_secs(86_400)),
        };
        let max_delay = Duration::from_secs(30);
        assert_eq!(
            adjust_delay(&err, Some(Duration::from_secs(2)), max_delay),
            Some(max_delay),
        );
    }

    #[test]
    fn computed_backoff_is_used_without_a_server_hint() {
        let err = Error::Api {
            status: 500,
            msg: "boom".into(),
            retry_after: None,
        };
        assert_eq!(
            adjust_delay(&err, Some(Duration::from_secs(4)), Duration::from_secs(30)),
            Some(Duration::from_secs(4)),
        );
    }

    #[test]
    fn overrides_fall_back_to_defaults() {
        let policy = RetryPolicy::from_overrides(Some(3), None, None, None);
        assert_eq!(policy.max_attempts, 3);
        assert_eq!(policy.base_delay, DEFAULT_BASE_DELAY);
        assert_eq!(policy.max_delay, DEFAULT_MAX_DELAY);
    }

    #[test]
    fn overrides_are_clamped_to_usable_values() {
        // 0 attempts would mean "never call at all"; 0 factor would flatten the
        // backoff to nothing.
        let policy = RetryPolicy::from_overrides(Some(0), None, None, Some(0));
        assert_eq!(policy.max_attempts, 1);
        assert_eq!(policy.factor, 1);

        // A ceiling below the base delay must not shorten the first retry.
        let policy = RetryPolicy::from_overrides(
            None,
            Some(Duration::from_secs(5)),
            Some(Duration::from_secs(1)),
            None,
        );
        assert_eq!(policy.max_delay, Duration::from_secs(5));
    }

    #[tokio::test]
    async fn succeeds_first_try() {
        let calls = Cell::new(0u32);
        let out = (|| {
            calls.set(calls.get() + 1);
            async { Ok::<u32, Error>(42) }
        })
        .retry(fast_policy(5).builder())
        .when(Error::is_retryable)
        .await
        .unwrap();
        assert_eq!(out, 42);
        assert_eq!(calls.get(), 1);
    }

    #[tokio::test]
    async fn retries_then_succeeds() {
        let calls = Cell::new(0u32);
        let out = (|| {
            let n = calls.get() + 1;
            calls.set(n);
            async move { if n < 3 { Err(retryable(503)) } else { Ok(n) } }
        })
        .retry(fast_policy(5).builder())
        .when(Error::is_retryable)
        .await
        .unwrap();
        assert_eq!(out, 3);
        assert_eq!(calls.get(), 3);
    }

    #[tokio::test]
    async fn exhausts_attempts() {
        let calls = Cell::new(0u32);
        let err = (|| {
            calls.set(calls.get() + 1);
            async { Err::<(), _>(retryable(500)) }
        })
        .retry(fast_policy(3).builder())
        .when(Error::is_retryable)
        .await
        .unwrap_err();
        // max_attempts = 3 → the first call plus 2 retries.
        assert_eq!(calls.get(), 3);
        assert_eq!(err.status(), Some(500));
    }

    #[tokio::test]
    async fn non_retryable_is_not_retried() {
        let calls = Cell::new(0u32);
        let err = (|| {
            calls.set(calls.get() + 1);
            async {
                Err::<(), _>(Error::Auth {
                    status: 401,
                    msg: "nope".into(),
                })
            }
        })
        .retry(fast_policy(5).builder())
        .when(Error::is_retryable)
        .await
        .unwrap_err();
        assert_eq!(calls.get(), 1);
        assert_eq!(err.status(), Some(401));
    }
}
