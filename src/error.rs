//! Structured error type for the SimpleLocalize core.
//!
//! Variants stay structured (not stringly-typed) so the Python bindings can map
//! them to a matching exception hierarchy.

use thiserror::Error;

/// Errors returned by the API client and download engine.
#[derive(Debug, Error)]
pub enum Error {
    /// The API rejected the credentials (HTTP 401 / 403).
    #[error("authentication failed (HTTP {status}): {msg}")]
    Auth { status: u16, msg: String },

    /// The API returned a non-success status with an error payload.
    #[error("SimpleLocalize API error (HTTP {status}): {msg}")]
    Api { status: u16, msg: String },

    /// A transport-level failure (connection, timeout, TLS, …).
    #[error("network error: {0}")]
    Network(#[from] reqwest::Error),

    /// The response could not be parsed into the expected shape.
    #[error("invalid response from server: {0}")]
    InvalidResponse(String),
}

impl Error {
    /// The HTTP status associated with this error, if any.
    pub fn status(&self) -> Option<u16> {
        match self {
            Error::Auth { status, .. } | Error::Api { status, .. } => Some(*status),
            _ => None,
        }
    }
}
