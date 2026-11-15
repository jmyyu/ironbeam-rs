//! Error types.

use std::time::Duration;

use crate::orders::UnknownOutcome;

/// Convenience alias used throughout the crate.
pub type Result<T, E = Error> = std::result::Result<T, E>;

/// Whether a failed request can have reached the server.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Delivery {
    /// The connection was never established; the server cannot have seen the
    /// request.
    NotSent,
    /// The request was (or may have been) written to the wire. The server may
    /// have acted on it.
    MaybeSent,
}

/// Everything that can go wrong in this crate.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum Error {
    /// Invalid client configuration.
    #[error("configuration error: {0}")]
    Config(String),

    /// The request was rejected locally, before any I/O.
    #[error("invalid request: {0}")]
    InvalidRequest(String),

    /// An authenticated call was made without a token and without credentials
    /// to obtain one.
    #[error("no bearer token and no credentials configured")]
    MissingCredentials,

    /// The server answered with a non-success status (or a 2xx body whose
    /// `status` field is `ERROR`/`FATAL`).
    #[error("API error (HTTP {status}): {}", message.as_deref().unwrap_or("<no message>"))]
    Api {
        /// HTTP status code.
        status: u16,
        /// `error` or `message` field of the error body, when present.
        message: Option<String>,
        /// Raw response body, truncated to 4 KiB.
        body: String,
    },

    /// HTTP 429.
    #[error("rate limited (HTTP 429){}", retry_after.map(|d| format!(", retry after {d:?}")).unwrap_or_default())]
    RateLimited {
        /// Parsed `Retry-After` header, if the server sent one.
        retry_after: Option<Duration>,
        /// Raw response body, truncated to 4 KiB.
        body: String,
    },

    /// Network-level failure (DNS, connect, TLS, reset, timeout).
    #[error("transport error ({delivery:?}{}): {source}", if *timed_out { ", timed out" } else { "" })]
    Transport {
        /// Whether the server may have received the request.
        delivery: Delivery,
        /// Whether the failure was a timeout.
        timed_out: bool,
        /// Underlying error.
        #[source]
        source: reqwest::Error,
    },

    /// A success response whose body could not be decoded.
    #[error("failed to decode response body: {source}")]
    Decode {
        /// Underlying error.
        #[source]
        source: serde_json::Error,
        /// Raw response body, truncated to 4 KiB.
        body: String,
    },

    /// An order submission whose outcome is not known. **The order may be
    /// live.** Do not resubmit; resolve it with
    /// [`OrderManager::investigate`](crate::orders::OrderManager::investigate).
    #[error("order outcome unknown — the order MAY have been accepted; do not resubmit blindly (cause: {})", .0.cause)]
    OrderOutcomeUnknown(Box<UnknownOutcome>),

    /// WebSocket protocol or I/O failure.
    #[error("websocket error: {0}")]
    WebSocket(#[from] Box<tokio_tungstenite::tungstenite::Error>),

    /// The stream task has stopped (shut down, or failed permanently).
    #[error("stream is closed")]
    StreamClosed,

    /// The operation needs a live stream connection and there is none.
    #[error("stream is not connected")]
    StreamNotConnected,
}

impl Error {
    /// HTTP status code, when the error carries one.
    pub fn status(&self) -> Option<u16> {
        match self {
            Self::Api { status, .. } => Some(*status),
            Self::RateLimited { .. } => Some(429),
            _ => None,
        }
    }

    /// `true` when the server may have acted on the request even though the
    /// call failed: transport failures after the request left, 5xx responses
    /// and undecodable success bodies.
    ///
    /// For a mutating call this means **the outcome is unknown**.
    pub fn is_ambiguous(&self) -> bool {
        match self {
            Self::Transport { delivery, .. } => *delivery == Delivery::MaybeSent,
            Self::Api { status, .. } => *status >= 500,
            Self::Decode { .. } | Self::OrderOutcomeUnknown(_) => true,
            _ => false,
        }
    }

    /// `true` for failures that are worth retrying *for idempotent requests*.
    pub fn is_transient(&self) -> bool {
        match self {
            Self::Transport { .. } | Self::RateLimited { .. } => true,
            Self::Api { status, .. } => *status >= 500,
            _ => false,
        }
    }

    /// `true` for 401/403 responses.
    pub fn is_auth(&self) -> bool {
        matches!(self.status(), Some(401 | 403))
    }
}

pub(crate) fn truncate_body(mut body: String) -> String {
    const MAX: usize = 4096;
    if body.len() > MAX {
        let mut cut = MAX;
        while !body.is_char_boundary(cut) {
            cut -= 1;
        }
        body.truncate(cut);
        body.push_str("…[truncated]");
    }
    body
}
