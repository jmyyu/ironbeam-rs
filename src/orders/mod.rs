//! Order-state tracking, reconciliation and safe submission.
//!
//! * [`OrderTracker`] — a pure, synchronous state machine that merges order
//!   updates from the stream and snapshots from REST into one consistent view,
//!   rejecting stale information.
//! * [`OrderManager`] — ties a tracker to a [`Client`](crate::Client): submits
//!   orders without ever retrying them, turns ambiguous submissions into an
//!   explicit [`UnknownOutcome`] that can be investigated, and reconciles
//!   state after stream reconnects.

mod manager;
mod tracker;

use std::time::SystemTime;

pub use manager::{Investigation, OrderEvents, OrderManager, Submission};
pub use tracker::{OrderChange, OrderTracker, ReconcileReport, Source, TrackedOrder};

use crate::error::Error;
use crate::models::OrderRequest;

/// An order submission whose result is not known.
///
/// Produced when the request may have reached the server but no usable
/// response was received. The order **may be live**. Carried by
/// [`Error::OrderOutcomeUnknown`].
#[derive(Debug)]
pub struct UnknownOutcome {
    /// Account the order was submitted for.
    pub account_id: String,
    /// The order as submitted.
    pub request: OrderRequest,
    /// What went wrong (timeout, connection reset, 5xx, undecodable body).
    pub cause: Error,
    /// Local wall-clock time of the submission attempt.
    pub submitted_at: SystemTime,
    /// Tracker sequence number at submission time; orders first seen after it
    /// are candidates when investigating. `None` when the submission did not
    /// go through an [`OrderManager`].
    pub(crate) watermark: Option<u64>,
}

impl UnknownOutcome {
    pub(crate) fn new(account_id: &str, request: OrderRequest, cause: Error) -> Self {
        Self {
            account_id: account_id.to_owned(),
            request,
            cause,
            submitted_at: SystemTime::now(),
            watermark: None,
        }
    }
}
