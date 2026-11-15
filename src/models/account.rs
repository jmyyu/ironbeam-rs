//! Account payloads: balances, positions, risk and fills.
//!
//! The REST API uses long property names and the WebSocket stream uses
//! abbreviated ones for the same objects. Each type here accepts both (the
//! short names are serde aliases) and serializes with the long names.

use serde::{Deserialize, Serialize};

use super::common::{ResponseStatus, flat_or_nested, null_default, opt_lenient_i64};
use super::enums::*;

/// One margin computation.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct MarginDetail {
    #[serde(alias = "me")]
    pub margin_error: Option<String>,
    #[serde(alias = "es")]
    pub error_symbols: Option<String>,
    #[serde(alias = "irm")]
    pub initial_risk_margin: Option<f64>,
    #[serde(alias = "mrm")]
    pub maintenance_risk_margin: Option<f64>,
    #[serde(alias = "itm")]
    pub initial_total_margin: Option<f64>,
    #[serde(alias = "mtm")]
    pub maintenance_total_margin: Option<f64>,
    #[serde(alias = "ie")]
    pub is_estimated: Option<bool>,
    /// Milliseconds since the Unix epoch.
    #[serde(alias = "t")]
    pub as_of_time: Option<i64>,
}

/// Margin figures of an account. The specification does not define the three
/// variants; the names suggest open positions (`O`), plus working orders
/// (`OW`), plus implied (`OWI`).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct MarginInfo {
    #[serde(alias = "a")]
    pub account_id: Option<String>,
    #[serde(alias = "cc")]
    pub currency_code: Option<String>,
    #[serde(rename = "marginO", alias = "mo")]
    pub margin_o: Option<MarginDetail>,
    #[serde(rename = "marginOW", alias = "mow")]
    pub margin_ow: Option<MarginDetail>,
    #[serde(rename = "marginOWI", alias = "mowi")]
    pub margin_owi: Option<MarginDetail>,
}

/// Balance of one account in one currency.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Balance {
    #[serde(alias = "a")]
    pub account_id: Option<String>,
    #[serde(alias = "cc")]
    pub currency_code: Option<String>,
    #[serde(alias = "cb")]
    pub cash_balance: Option<f64>,
    #[serde(alias = "cba")]
    pub cash_balance_available: Option<f64>,
    #[serde(alias = "ote")]
    pub open_trade_equity: Option<f64>,
    #[serde(alias = "te")]
    pub total_equity: Option<f64>,
    #[serde(alias = "cbta")]
    pub cash_added_today: Option<f64>,
    #[serde(alias = "nl")]
    pub net_liquidity: Option<f64>,
    #[serde(alias = "nla")]
    pub net_liquidity_available: Option<f64>,
    #[serde(alias = "dc", deserialize_with = "opt_lenient_i64")]
    pub days_on_call: Option<i64>,
    #[serde(alias = "bt")]
    pub balance_type: Option<BalanceType>,
    #[serde(alias = "mi")]
    pub margin_info: Option<MarginInfo>,
}

/// `GET /account/{accountId}/balance` and `GET /account/getAllBalances`.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct AccountBalanceResponse {
    pub status: Option<ResponseStatus>,
    pub message: Option<String>,
    #[serde(deserialize_with = "null_default")]
    pub balances: Vec<Balance>,
}

/// An open position.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Position {
    #[serde(alias = "a")]
    pub account_id: Option<String>,
    #[serde(alias = "cc")]
    pub currency_code: Option<String>,
    #[serde(alias = "s")]
    pub exch_sym: Option<String>,
    #[serde(alias = "pId")]
    pub position_id: Option<String>,
    #[serde(alias = "q")]
    pub quantity: Option<f64>,
    /// Average price.
    #[serde(alias = "p")]
    pub price: Option<f64>,
    /// `YYYYMMDD`.
    #[serde(alias = "do")]
    pub date_opened: Option<String>,
    #[serde(alias = "sd")]
    pub side: Option<PositionSide>,
    #[serde(rename = "unrealizedPL", alias = "upl")]
    pub unrealized_pl: Option<f64>,
}

impl Position {
    /// Quantity with sign: positive for long, negative for short. `None` when
    /// quantity or side is missing or unrecognised.
    pub fn signed_quantity(&self) -> Option<f64> {
        let q = self.quantity?;
        match self.side.as_ref()? {
            PositionSide::Long => Some(q),
            PositionSide::Short => Some(-q),
            PositionSide::Unknown(_) => None,
        }
    }
}

/// Positions of one account. Also the response of
/// `GET /account/{accountId}/positions`.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct AccountPositions {
    pub status: Option<ResponseStatus>,
    pub message: Option<String>,
    #[serde(alias = "a")]
    pub account_id: Option<String>,
    #[serde(alias = "p", deserialize_with = "null_default")]
    pub positions: Vec<Position>,
}

/// `GET /account/getAllPositions`.
///
/// The schema declares `positions` with tuple-style `items`, which is not
/// valid OpenAPI 3.0; both a flat list and a list of lists are accepted.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct AllPositionsResponse {
    pub status: Option<ResponseStatus>,
    pub message: Option<String>,
    #[serde(deserialize_with = "flat_or_nested")]
    pub positions: Vec<AccountPositions>,
}

/// Risk state of one sub-account (account / reg code / currency).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct RiskInfo {
    #[serde(alias = "a")]
    pub account_id: Option<String>,
    #[serde(alias = "rc")]
    pub reg_code: Option<RegCode>,
    #[serde(alias = "cc")]
    pub currency_code: Option<String>,
    /// Account value at or below which the account is auto-liquidated. `None`
    /// when no liquidation level is configured.
    #[serde(alias = "lv")]
    pub liquidation_value: Option<f64>,
    #[serde(alias = "snlv")]
    pub start_net_liquidation_value: Option<f64>,
    #[serde(alias = "cnlv")]
    pub current_net_liquidation_value: Option<f64>,
    #[serde(alias = "mnlv")]
    pub max_net_liquidation_value: Option<f64>,
    #[serde(alias = "mnlvmd")]
    pub max_net_liquidation_value_multi_day: Option<f64>,
    /// History of liquidation event codes; see [`LiquidationEvent`].
    #[serde(alias = "le", deserialize_with = "null_default")]
    pub liquidation_events: Vec<i64>,
}

impl RiskInfo {
    /// Whether the sub-account is currently in (or has attempted) liquidation.
    ///
    /// Per the documentation the account is free to trade when the event list
    /// is empty or ends in `RESET_LIQUIDATION`; otherwise a liquidation has at
    /// least been attempted. Returns `None` when the last code is not one of
    /// the documented ones (the documentation's own sample ends in `0`).
    pub fn is_liquidated(&self) -> Option<bool> {
        match self
            .liquidation_events
            .last()
            .copied()
            .map(LiquidationEvent::from_code)
        {
            None | Some(LiquidationEvent::ResetLiquidation) => Some(false),
            Some(LiquidationEvent::Other(_)) => None,
            Some(_) => Some(true),
        }
    }
}

/// `GET /account/{accountId}/risk` and `GET /account/getAllRiskInfo`.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct AccountRiskResponse {
    pub status: Option<ResponseStatus>,
    pub message: Option<String>,
    #[serde(deserialize_with = "null_default")]
    pub risks: Vec<RiskInfo>,
}

/// A fill (execution) of an order.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Fill {
    #[serde(alias = "oid")]
    pub order_id: Option<String>,
    #[serde(alias = "sid", deserialize_with = "opt_lenient_i64")]
    pub strategy_id: Option<i64>,
    #[serde(alias = "a")]
    pub account_id: Option<String>,
    #[serde(alias = "s")]
    pub exch_sym: Option<String>,
    #[serde(alias = "st")]
    pub status: Option<OrderStatus>,
    #[serde(alias = "sd")]
    pub side: Option<OrderSide>,
    /// Order quantity.
    #[serde(alias = "q")]
    pub quantity: Option<f64>,
    /// Order price.
    #[serde(alias = "p")]
    pub price: Option<f64>,
    /// Quantity of this fill.
    #[serde(alias = "fq")]
    pub fill_quantity: Option<f64>,
    /// Cumulative filled quantity of the order.
    #[serde(alias = "ftq")]
    pub fill_total_quantity: Option<f64>,
    /// Price of this fill.
    #[serde(alias = "fp")]
    pub fill_price: Option<f64>,
    #[serde(alias = "afp")]
    pub avg_fill_price: Option<f64>,
    /// ISO-8601 timestamp, kept as sent.
    #[serde(alias = "fd")]
    pub fill_date: Option<String>,
    /// Milliseconds since the Unix epoch.
    #[serde(alias = "t")]
    pub time_order_event: Option<i64>,
    /// Unique id of the order update that carried this fill.
    #[serde(alias = "ouid")]
    pub order_update_id: Option<String>,
}

/// Fill lists: `GET /account/{accountId}/fills`, `GET /account/getAllFills`
/// and `GET /order/{accountId}/fills`.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct FillsResponse {
    pub status: Option<ResponseStatus>,
    pub message: Option<String>,
    #[serde(deserialize_with = "null_default")]
    pub fills: Vec<Fill>,
}

/// `GET /account/getAllAccounts`.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct AllAccountsResponse {
    pub status: Option<ResponseStatus>,
    pub message: Option<String>,
    #[serde(deserialize_with = "null_default")]
    pub accounts: Vec<String>,
}
