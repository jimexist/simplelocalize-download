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

impl RetryPolicy {
    /// Backoff builder for `backon`. `backon` counts *retries*, so the number of
    /// retries is `max_attempts - 1` (the first call is not a retry).
    pub fn builder(&self) -> ExponentialBuilder {
        ExponentialBuilder::default()
            .with_min_delay(self.base_delay)
            .with_max_delay(self.max_delay)
            .with_factor(self.factor as f32)
            .with_max_times(self.max_attempts.saturating_sub(1) as usize)
            .with_jitter()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::Error;
    use backon::Retryable;
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
