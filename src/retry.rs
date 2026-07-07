//! Generic exponential-backoff retry helper shared by the API client and the
//! download engine.
//!
//! The operation returns an [`Attempt`], distinguishing success, a retryable
//! failure (optionally carrying a server-suggested `Retry-After`), and a fatal
//! failure. Backoff uses full jitter; a server `Retry-After` overrides it.

use std::future::Future;
use std::time::{Duration, SystemTime};

use crate::error::Error;

/// Backoff configuration.
#[derive(Debug, Clone)]
pub struct RetryPolicy {
    /// Maximum number of attempts (including the first).
    pub max_attempts: u32,
    /// Delay for the first retry, doubled (by `factor`) each subsequent retry.
    pub base_delay: Duration,
    /// Upper bound on any single backoff delay.
    pub max_delay: Duration,
    /// Exponential growth factor.
    pub factor: u32,
}

impl Default for RetryPolicy {
    fn default() -> Self {
        Self {
            max_attempts: 5,
            base_delay: Duration::from_millis(250),
            max_delay: Duration::from_secs(10),
            factor: 2,
        }
    }
}

/// Outcome of a single attempt.
pub enum Attempt<T> {
    /// Success.
    Done(T),
    /// Retryable failure; `retry_after` overrides computed backoff when set.
    Retry {
        error: Error,
        retry_after: Option<Duration>,
    },
    /// Permanent failure — do not retry.
    Fatal(Error),
}

/// Cheap thread-local xorshift PRNG for jitter (no external dependency).
fn rand_u64() -> u64 {
    use std::cell::Cell;
    thread_local! {
        static STATE: Cell<u64> = Cell::new(seed());
    }
    fn seed() -> u64 {
        let nanos = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .map(|d| d.as_nanos() as u64)
            .unwrap_or(0x9E37_79B9_7F4A_7C15);
        nanos | 1
    }
    STATE.with(|s| {
        let mut x = s.get();
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        s.set(x);
        x
    })
}

/// Full-jitter backoff delay for the given retry index (0-based).
fn backoff_delay(policy: &RetryPolicy, retry_index: u32) -> Duration {
    let factor_pow = u64::from(policy.factor).saturating_pow(retry_index);
    let base_ms = policy.base_delay.as_millis() as u64;
    let uncapped = base_ms.saturating_mul(factor_pow);
    let cap = (policy.max_delay.as_millis() as u64).max(1);
    let ceiling = uncapped.min(cap).max(1);
    // Full jitter: uniform in [0, ceiling].
    Duration::from_millis(rand_u64() % (ceiling + 1))
}

/// Run `op`, retrying retryable failures per `policy`.
pub async fn retry<T, F, Fut>(policy: &RetryPolicy, mut op: F) -> Result<T, Error>
where
    F: FnMut() -> Fut,
    Fut: Future<Output = Attempt<T>>,
{
    let mut attempt: u32 = 0;
    loop {
        match op().await {
            Attempt::Done(value) => return Ok(value),
            Attempt::Fatal(error) => return Err(error),
            Attempt::Retry { error, retry_after } => {
                attempt += 1;
                if attempt >= policy.max_attempts {
                    log::debug!("giving up after {attempt} attempt(s): {error}");
                    return Err(error);
                }
                let delay = retry_after.unwrap_or_else(|| backoff_delay(policy, attempt - 1));
                log::debug!(
                    "retryable error (attempt {attempt}/{}), backing off {delay:?}: {error}",
                    policy.max_attempts
                );
                tokio::time::sleep(delay).await;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;

    #[tokio::test]
    async fn succeeds_first_try() {
        let policy = RetryPolicy::default();
        let out = retry(&policy, || async { Attempt::Done(42) })
            .await
            .unwrap();
        assert_eq!(out, 42);
    }

    #[tokio::test]
    async fn retries_then_succeeds() {
        let policy = RetryPolicy {
            max_attempts: 5,
            base_delay: Duration::from_millis(1),
            max_delay: Duration::from_millis(2),
            factor: 2,
        };
        let calls = Cell::new(0u32);
        let out = retry(&policy, || async {
            let n = calls.get() + 1;
            calls.set(n);
            if n < 3 {
                Attempt::Retry {
                    error: Error::Api {
                        status: 503,
                        msg: "busy".into(),
                    },
                    retry_after: None,
                }
            } else {
                Attempt::Done(n)
            }
        })
        .await
        .unwrap();
        assert_eq!(out, 3);
        assert_eq!(calls.get(), 3);
    }

    #[tokio::test]
    async fn exhausts_attempts() {
        let policy = RetryPolicy {
            max_attempts: 3,
            base_delay: Duration::from_millis(1),
            max_delay: Duration::from_millis(2),
            factor: 2,
        };
        let calls = Cell::new(0u32);
        let err = retry(&policy, || async {
            calls.set(calls.get() + 1);
            Attempt::<()>::Retry {
                error: Error::Api {
                    status: 500,
                    msg: "boom".into(),
                },
                retry_after: None,
            }
        })
        .await
        .unwrap_err();
        assert_eq!(calls.get(), 3);
        assert_eq!(err.status(), Some(500));
    }

    #[tokio::test]
    async fn fatal_does_not_retry() {
        let policy = RetryPolicy::default();
        let calls = Cell::new(0u32);
        let err = retry(&policy, || async {
            calls.set(calls.get() + 1);
            Attempt::<()>::Fatal(Error::Auth {
                status: 401,
                msg: "nope".into(),
            })
        })
        .await
        .unwrap_err();
        assert_eq!(calls.get(), 1);
        assert_eq!(err.status(), Some(401));
    }
}
