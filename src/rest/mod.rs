//! Typed REST operations, implemented as methods on [`Client`](crate::Client).
//!
//! | Group | Source file | Operations |
//! |---|---|---|
//! | Information | `info.rs` | trader/user info, security definitions/margin/status, symbol searches, strategy id |
//! | Account | `account.rs` | balances, positions, risk, fills (per account and for all accounts) |
//! | Market data | `market.rs` | quotes, depth, historical trades |
//! | Orders | `orders.rs` | place, update, cancel, cancel-multiple, list, fills, id lookups |
//! | Streaming control | `streaming.rs` | stream id creation, market-data and indicator (un)subscription |
//! | Simulated accounts | `simulated.rs` | Enterprise demo-account management |
//!
//! Reads are retried automatically on transient failures according to
//! [`RetryPolicy`](crate::RetryPolicy); nothing else is.

mod account;
mod info;
mod market;
mod orders;
mod simulated;
mod streaming;

pub use streaming::BarKind;

use crate::error::{Error, Result};
use crate::models::validate_symbol;

/// Maximum number of symbols the API accepts per request / per stream
/// subscription kind.
pub const MAX_SYMBOLS: usize = 10;

/// Validates and joins symbols into the comma-separated `symbols` parameter.
pub(crate) fn join_symbols<S: AsRef<str>>(symbols: &[S]) -> Result<String> {
    if symbols.is_empty() || symbols.len() > MAX_SYMBOLS {
        return Err(Error::InvalidRequest(format!(
            "between 1 and {MAX_SYMBOLS} symbols are required, got {}",
            symbols.len()
        )));
    }
    let mut joined = String::new();
    for (i, s) in symbols.iter().enumerate() {
        validate_symbol(s.as_ref())?;
        if i > 0 {
            joined.push(',');
        }
        joined.push_str(s.as_ref());
    }
    Ok(joined)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn symbol_lists_are_bounded() {
        assert!(join_symbols::<&str>(&[]).is_err());
        assert_eq!(
            join_symbols(&["XCME:ES.Z26", "XCME:NQ.Z26"]).unwrap(),
            "XCME:ES.Z26,XCME:NQ.Z26"
        );
        let eleven: Vec<String> = (0..11).map(|i| format!("XCME:S{i}")).collect();
        assert!(join_symbols(&eleven).is_err());
        assert!(join_symbols(&["A,B"]).is_err());
    }
}
