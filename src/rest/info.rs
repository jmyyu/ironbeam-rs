use crate::client::{Call, Client};
use crate::endpoints as ep;
use crate::error::{Error, Result};
use crate::models::*;

use super::join_symbols;

impl Client {
    /// Trader information. `trader_id` is optional (Enterprise users can
    /// query other traders).
    pub async fn trader_info(&self, trader_id: Option<&str>) -> Result<TraderInfo> {
        let mut call = Call::new(&ep::TRADER_INFO);
        if let Some(id) = trader_id {
            call = call.query("traderId", id);
        }
        self.send(call).await
    }

    /// General user information.
    pub async fn user_info(&self, trader_id: Option<&str>) -> Result<UserInfo> {
        let mut call = Call::new(&ep::USER_INFO);
        if let Some(id) = trader_id {
            call = call.query("traderId", id);
        }
        self.send(call).await
    }

    /// Security definitions for up to 10 symbols. Symbols the user is not
    /// entitled to are silently omitted by the server.
    pub async fn security_definitions<S: AsRef<str>>(
        &self,
        symbols: &[S],
    ) -> Result<SecurityDefinitionsResponse> {
        let call = Call::new(&ep::SECURITY_DEFINITIONS).query("symbols", join_symbols(symbols)?);
        self.send(call).await
    }

    /// Margin requirements and current value for up to 10 symbols.
    pub async fn security_margin<S: AsRef<str>>(
        &self,
        symbols: &[S],
    ) -> Result<SecurityMarginAndValueResponse> {
        let call = Call::new(&ep::SECURITY_MARGIN).query("symbols", join_symbols(symbols)?);
        self.send(call).await
    }

    /// Trading status for up to 10 symbols.
    pub async fn security_status<S: AsRef<str>>(
        &self,
        symbols: &[S],
    ) -> Result<SecurityStatusResponse> {
        let call = Call::new(&ep::SECURITY_STATUS).query("symbols", join_symbols(symbols)?);
        self.send(call).await
    }

    /// Symbols starting with `text` (3–20 characters when given).
    ///
    /// `limit` must be within 1..=1000; `prefer_active` defaults to `true` on
    /// the server.
    pub async fn symbols(
        &self,
        text: Option<&str>,
        limit: Option<u32>,
        prefer_active: Option<bool>,
    ) -> Result<SymbolsResponse> {
        let mut call = Call::new(&ep::SYMBOLS);
        if let Some(text) = text {
            let n = text.chars().count();
            if !(3..=20).contains(&n) {
                return Err(Error::InvalidRequest(format!(
                    "symbol search text must be 3..=20 characters, got {n}"
                )));
            }
            call = call.query("text", text);
        }
        if let Some(limit) = limit {
            if !(1..=1000).contains(&limit) {
                return Err(Error::InvalidRequest(format!(
                    "limit must be 1..=1000, got {limit}"
                )));
            }
            call = call.query("limit", limit.to_string());
        }
        if let Some(active) = prefer_active {
            call = call.query("preferActive", active.to_string());
        }
        self.send(call).await
    }

    /// Exchanges the user is entitled to.
    pub async fn exchange_sources(&self) -> Result<ExchangeSourcesResponse> {
        self.send(Call::new(&ep::EXCHANGE_SOURCES)).await
    }

    /// Market complexes of an exchange (e.g. `XCME`).
    pub async fn complexes(&self, exchange: &str) -> Result<ComplexesResponse> {
        self.send(Call::new(&ep::COMPLEXES).params(&[("exchange", exchange)]))
            .await
    }

    /// Futures contracts of a market group (e.g. `XCME`, `ES`).
    pub async fn symbol_futures(
        &self,
        exchange: &str,
        market_group: &str,
    ) -> Result<SymbolFuturesResponse> {
        let params = [("exchange", exchange), ("marketGroup", market_group)];
        self.send(Call::new(&ep::SYMBOL_FUTURES).params(&params))
            .await
    }

    /// Symbol groups of a market complex (e.g. `Currency`).
    pub async fn symbol_groups(&self, complex: &str) -> Result<ComplexGroupsResponse> {
        self.send(Call::new(&ep::SYMBOL_GROUPS).params(&[("complex", complex)]))
            .await
    }

    /// Option groups available on an underlying symbol.
    pub async fn symbol_option_groups(&self, symbol: &str) -> Result<SymbolOptionsResponse> {
        validate_symbol(symbol)?;
        self.send(Call::new(&ep::SYMBOL_OPTION_GROUPS).params(&[("symbol", symbol)]))
            .await
    }

    /// Option symbols of a group. `near_the_money` restricts the result to
    /// strikes near the money.
    pub async fn symbol_options(
        &self,
        symbol: &str,
        group: &str,
        option_type: OptionRight,
        near_the_money: bool,
    ) -> Result<SymbolSearchOptionsResponse> {
        validate_symbol(symbol)?;
        let near = near_the_money.to_string();
        let params = [
            ("symbol", symbol),
            ("group", group),
            ("optionType", option_type.as_str()),
            ("near", near.as_str()),
        ];
        self.send(Call::new(&ep::SYMBOL_OPTIONS).params(&params))
            .await
    }

    /// Option spreads available on a symbol.
    pub async fn symbol_option_spreads(&self, symbol: &str) -> Result<SymbolOptionSpreadsResponse> {
        validate_symbol(symbol)?;
        self.send(Call::new(&ep::SYMBOL_OPTION_SPREADS).params(&[("symbol", symbol)]))
            .await
    }

    /// A strategy id issued by the server, with the valid range.
    ///
    /// The specification documents no way to *attach* a strategy id to an
    /// order: the server assigns one on placement and returns it.
    pub async fn strategy_id(&self) -> Result<StrategyIdResponse> {
        self.send(Call::new(&ep::STRATEGY_ID)).await
    }
}
