//! Typed request and response payloads.
//!
//! Decoding is deliberately lenient — nearly every field is optional, unknown
//! enum values are preserved rather than rejected, and unknown properties are
//! ignored — because the published specification is not reliable enough to
//! justify failing a whole response over one unexpected field. The two
//! exceptions are [`Order::order_id`] and [`Order::status`], without which an
//! order cannot be tracked.
#![allow(missing_docs)]

pub(crate) mod common;

mod account;
mod enums;
mod info;
mod market;
mod order;
mod simulated;
mod stream;

pub use account::*;
pub use common::{ApiResponse, ResponseStatus};
pub use enums::*;
pub use info::*;
pub use market::*;
pub(crate) use order::validate_symbol;
pub use order::{Order, OrderError, OrderIds, OrderRequest, OrderUpdate, OrdersResponse};
pub use simulated::*;
pub use stream::*;
