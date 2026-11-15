//! Payloads of the `/info/*` endpoints.

use serde::{Deserialize, Serialize};

use super::common::{ResponseStatus, null_default, opt_lenient_i64};
use super::enums::*;

/// `GET /info/trader`.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct TraderInfo {
    pub status: Option<ResponseStatus>,
    pub message: Option<String>,
    /// Accounts the trader can access.
    #[serde(deserialize_with = "null_default")]
    pub accounts: Vec<String>,
    /// Whether this is a live (as opposed to demo) trader.
    pub is_live: Option<bool>,
    pub trader_id: Option<String>,
}

/// `GET /info/user`.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct UserInfo {
    pub status: Option<ResponseStatus>,
    pub message: Option<String>,
    pub account_category: Option<i64>,
    pub account_title: Option<String>,
    pub email_address1: Option<String>,
    pub email_address2: Option<String>,
    pub group: Option<String>,
    pub is_clearing_account: Option<bool>,
    pub phone1: Option<String>,
    pub phone2: Option<String>,
    pub sub_group: Option<String>,
    #[serde(deserialize_with = "null_default")]
    pub accounts: Vec<String>,
}

/// One leg of a spread/strategy security.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct SecurityDefinitionLeg {
    pub symbol: Option<String>,
    pub ratio: Option<i64>,
    pub side: Option<Side>,
    pub security_id: Option<String>,
    pub exchange: Option<String>,
    pub leg_exchange_symbol: Option<String>,
}

/// Static definition of a tradable security.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct SecurityDefinition {
    pub exch_sym: Option<String>,
    pub exchange_source: Option<String>,
    /// Milliseconds since the Unix epoch.
    pub activation_time: Option<i64>,
    /// Milliseconds since the Unix epoch.
    pub expiration_time: Option<i64>,
    pub market_complex: Option<String>,
    pub market_group: Option<String>,
    pub market_symbol: Option<String>,
    pub cfi_code: Option<String>,
    pub allow_open_orders: Option<bool>,
    pub maturity_month: Option<i64>,
    pub maturity_year: Option<i64>,
    pub product_description: Option<String>,
    /// Wire name is the specification's misspelling `userDefinded`.
    #[serde(rename = "userDefinded", alias = "userDefined")]
    pub user_defined: Option<bool>,
    /// Wire name is the specification's misspelling `intradayDefinded`.
    #[serde(rename = "intradayDefinded", alias = "intradayDefined")]
    pub intraday_defined: Option<bool>,
    pub option_type: Option<OptionType>,
    pub option_expiration_type: Option<OptionExpirationType>,
    pub strike_price: Option<f64>,
    pub underlying_symbol: Option<String>,
    pub variable_tick_table_code: Option<i64>,
    pub exchange_strategy_type: Option<String>,
    pub security_type: Option<SecurityType>,
    pub security_id: Option<String>,
    #[serde(deserialize_with = "null_default")]
    pub legs: Vec<SecurityDefinitionLeg>,
    pub depth_levels: Option<i64>,
    pub main_fraction: Option<f64>,
    pub sub_fraction: Option<f64>,
    pub scale: Option<i64>,
    /// Minimum price increment (tick size).
    pub min_price_increment: Option<f64>,
    /// Currency value of one tick.
    pub min_price_increment_value: Option<f64>,
    pub reg_code: Option<RegCode>,
    pub currency_code: Option<String>,
    pub display_factor: Option<f64>,
    pub allow_trading: Option<bool>,
    pub scaling_factor_screen: Option<f64>,
    pub exchange_symbol: Option<String>,
    /// Milliseconds since the Unix epoch.
    pub creation_date: Option<i64>,
}

/// `GET /info/security/definitions`.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct SecurityDefinitionsResponse {
    pub status: Option<ResponseStatus>,
    pub message: Option<String>,
    #[serde(deserialize_with = "null_default")]
    pub security_definitions: Vec<SecurityDefinition>,
}

/// A time window with its own margin requirement.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct MarginScheduleDetail {
    pub start_time: Option<i64>,
    pub end_time: Option<i64>,
    pub margin: Option<f64>,
}

/// Margin and current value of one security.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct SecurityMarginAndValue {
    pub exch_sym: Option<String>,
    pub current_price: Option<f64>,
    pub current_time: Option<i64>,
    pub current_value: Option<f64>,
    pub initial_margin_long: Option<f64>,
    /// Wire name is the specification's misspelling `initialMaginShort`.
    #[serde(rename = "initialMaginShort", alias = "initialMarginShort")]
    pub initial_margin_short: Option<f64>,
    pub maint_margin_long: Option<f64>,
    pub maint_margin_short: Option<f64>,
    pub span_settle_price: Option<f64>,
    pub span_settle_value: Option<f64>,
    #[serde(deserialize_with = "null_default")]
    pub margin_schedule_details: Vec<MarginScheduleDetail>,
}

/// `GET /info/security/margin`.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct SecurityMarginAndValueResponse {
    pub status: Option<ResponseStatus>,
    pub message: Option<String>,
    #[serde(deserialize_with = "null_default")]
    pub security_margin_and_values: Vec<SecurityMarginAndValue>,
}

/// Trading status of one security.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct SecurityStatusInfo {
    pub exch_sym: Option<String>,
    pub status: Option<SecurityStatus>,
    pub status_value: Option<i64>,
    pub date_time: Option<i64>,
    pub trade_date: Option<i64>,
}

/// `GET /info/security/status`.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct SecurityStatusResponse {
    pub status: Option<ResponseStatus>,
    pub message: Option<String>,
    #[serde(deserialize_with = "null_default")]
    pub security_statuses: Vec<SecurityStatusInfo>,
}

/// One entry of the symbol search.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct SymbolInfo {
    pub symbol: Option<String>,
    pub currency: Option<String>,
    pub description: Option<String>,
    pub has_quotes: Option<bool>,
    pub pip_value: Option<f64>,
    pub pip_size: Option<f64>,
    pub min_tick: Option<f64>,
    pub qty_step: Option<f64>,
    pub symbol_type: Option<String>,
}

/// `GET /info/symbols`.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct SymbolsResponse {
    pub status: Option<ResponseStatus>,
    pub message: Option<String>,
    #[serde(deserialize_with = "null_default")]
    pub symbols: Vec<SymbolInfo>,
}

/// `GET /info/exchangeSources`.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ExchangeSourcesResponse {
    pub status: Option<ResponseStatus>,
    pub message: Option<String>,
    #[serde(deserialize_with = "null_default")]
    pub exchanges: Vec<String>,
}

/// A market group inside a complex.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ComplexGroupInfo {
    pub group: Option<String>,
    pub name: Option<String>,
}

/// A market complex and its groups.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ComplexGroups {
    #[serde(deserialize_with = "null_default")]
    pub groups: Vec<ComplexGroupInfo>,
    pub name: Option<String>,
}

/// `GET /info/complexes/{exchange}`.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ComplexesResponse {
    pub status: Option<ResponseStatus>,
    pub message: Option<String>,
    #[serde(deserialize_with = "null_default")]
    pub market_complexes: Vec<ComplexGroups>,
}

/// A futures contract returned by the futures search.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct FutureInfo {
    pub symbol: Option<String>,
    /// Three-letter month, e.g. `MAR`.
    pub maturity_month: Option<String>,
    pub maturity_year: Option<i64>,
    pub description: Option<String>,
}

/// `GET /info/symbol/search/futures/{exchange}/{marketGroup}`.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct SymbolFuturesResponse {
    pub status: Option<ResponseStatus>,
    pub message: Option<String>,
    #[serde(deserialize_with = "null_default")]
    pub symbols: Vec<FutureInfo>,
}

/// `GET /info/symbol/search/groups/{complex}`.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ComplexGroupsResponse {
    pub status: Option<ResponseStatus>,
    pub message: Option<String>,
    #[serde(deserialize_with = "null_default")]
    pub symbol_groups: Vec<ComplexGroupInfo>,
}

/// An option group (expiry series).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct OptionGroupInfo {
    pub group: Option<String>,
    /// Milliseconds since the Unix epoch.
    #[serde(deserialize_with = "opt_lenient_i64")]
    pub expiration: Option<i64>,
    pub description: Option<String>,
}

/// `GET /info/symbol/search/options/{symbol}`.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct SymbolOptionsResponse {
    pub status: Option<ResponseStatus>,
    pub message: Option<String>,
    #[serde(deserialize_with = "null_default")]
    pub groups: Vec<String>,
    #[serde(deserialize_with = "null_default")]
    pub option_groups: Vec<OptionGroupInfo>,
}

/// `GET /info/symbol/search/options/ext/...`.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct SymbolSearchOptionsResponse {
    pub status: Option<ResponseStatus>,
    pub message: Option<String>,
    #[serde(deserialize_with = "null_default")]
    pub symbol_options: Vec<String>,
}

/// `GET /info/symbol/search/options/spreads/{symbol}`.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct SymbolOptionSpreadsResponse {
    pub status: Option<ResponseStatus>,
    pub message: Option<String>,
    #[serde(deserialize_with = "null_default")]
    pub symbol_spreads: Vec<String>,
}

/// `GET /info/strategyId`: a server-issued strategy id and its valid range.
///
/// The wire names are PascalCase (`Id`, `Minimum`, `Maximum`), unlike the rest
/// of the API.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct StrategyIdResponse {
    pub status: Option<ResponseStatus>,
    pub message: Option<String>,
    #[serde(rename = "Id", alias = "id", deserialize_with = "opt_lenient_i64")]
    pub id: Option<i64>,
    #[serde(
        rename = "Minimum",
        alias = "minimum",
        deserialize_with = "opt_lenient_i64"
    )]
    pub minimum: Option<i64>,
    #[serde(
        rename = "Maximum",
        alias = "maximum",
        deserialize_with = "opt_lenient_i64"
    )]
    pub maximum: Option<i64>,
}
