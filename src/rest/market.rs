use crate::client::{Call, Client};
use crate::endpoints as ep;
use crate::error::{Error, Result};
use crate::models::*;

use super::join_symbols;

impl Client {
    /// Current quotes for up to 10 symbols.
    ///
    /// The first call for a symbol may return nothing: the server starts
    /// collecting quotes for a symbol when it is first requested.
    pub async fn quotes<S: AsRef<str>>(&self, symbols: &[S]) -> Result<QuotesResponse> {
        self.send(Call::new(&ep::QUOTES).query("symbols", join_symbols(symbols)?))
            .await
    }

    /// Current market depth for up to 10 symbols.
    pub async fn depth<S: AsRef<str>>(&self, symbols: &[S]) -> Result<DepthResponse> {
        self.send(Call::new(&ep::DEPTH).query("symbols", join_symbols(symbols)?))
            .await
    }

    /// Historical trades of a symbol between two instants (milliseconds since
    /// the Unix epoch).
    ///
    /// `max` must be within 1..=100. The meaning of `earlier` is not defined
    /// by the specification; it presumably selects the oldest rather than the
    /// newest `max` trades of the window.
    pub async fn trades(
        &self,
        symbol: &str,
        from_ms: i64,
        to_ms: i64,
        max: u32,
        earlier: bool,
    ) -> Result<TradesResponse> {
        validate_symbol(symbol)?;
        if !(1..=100).contains(&max) {
            return Err(Error::InvalidRequest(format!(
                "max must be 1..=100, got {max}"
            )));
        }
        if from_ms < 0 || to_ms < 0 || from_ms > to_ms {
            return Err(Error::InvalidRequest(format!(
                "invalid time window: from={from_ms} to={to_ms}"
            )));
        }
        let (from, to, max, earlier) = (
            from_ms.to_string(),
            to_ms.to_string(),
            max.to_string(),
            earlier.to_string(),
        );
        let params = [
            ("symbol", symbol),
            ("from", from.as_str()),
            ("to", to.as_str()),
            ("max", max.as_str()),
            ("earlier", earlier.as_str()),
        ];
        self.send(Call::new(&ep::TRADES).params(&params)).await
    }
}
