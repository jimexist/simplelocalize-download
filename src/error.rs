//! Structured error type for the SimpleLocalize core.
//!
//! Variants stay structured (not stringly-typed) so the Python bindings can map
//! them to a matching exception hierarchy.

use std::time::Duration;

use thiserror::Error;

/// Errors returned by the API client and download engine.
#[derive(Debug, Error)]
pub enum Error {
    /// The API rejected the credentials (HTTP 401 / 403).
    #[error("authentication failed (HTTP {status}): {msg}")]
    Auth { status: u16, msg: String },

    /// The API returned a non-success status with an error payload.
    #[error("SimpleLocalize API error (HTTP {status}): {msg}")]
    Api {
        status: u16,
        msg: String,
        /// Server-suggested delay from a `Retry-After` header, when present.
        retry_after: Option<Duration>,
    },

    /// A transport-level failure (connection, timeout, TLS, …).
    #[error("network error: {0}")]
    Network(#[from] reqwest::Error),

    /// The response could not be parsed into the expected shape.
    #[error("invalid response from server: {0}")]
    InvalidResponse(String),

    /// A local filesystem error while writing a downloaded file.
    #[error("I/O error at {path}: {message}")]
    Io { path: String, message: String },

    /// A resolved output path would escape the download base directory, or is
    /// otherwise unusable.
    #[error("unsafe output path: {0}")]
    UnsafePath(String),
}

impl Error {
    /// The HTTP status associated with this error, if any.
    pub fn status(&self) -> Option<u16> {
        match self {
            Error::Auth { status, .. } | Error::Api { status, .. } => Some(*status),
            _ => None,
        }
    }

    /// Whether retrying the operation might succeed: transient transport
    /// failures (connect/timeout, or a body interrupted mid-stream) and
    /// rate-limit/server statuses (429/5xx). Auth failures, other 4xx, decode
    /// failures, and local I/O/path errors are permanent.
    pub fn is_retryable(&self) -> bool {
        match self {
            Error::Api { status, .. } => *status == 429 || (500..600).contains(status),
            Error::Network(err) => err.is_timeout() || err.is_connect() || err.is_body(),
            Error::Auth { .. }
            | Error::InvalidResponse(_)
            | Error::Io { .. }
            | Error::UnsafePath(_) => false,
        }
    }

    /// Server-suggested backoff from a `Retry-After` header, when present.
    pub fn retry_after(&self) -> Option<Duration> {
        match self {
            Error::Api { retry_after, .. } => *retry_after,
            _ => None,
        }
    }

    /// Build an [`Error::Io`] from a path and a [`std::io::Error`].
    pub fn io(path: &std::path::Path, err: std::io::Error) -> Self {
        Error::Io {
            path: path.display().to_string(),
            message: err.to_string(),
        }
    }
}
