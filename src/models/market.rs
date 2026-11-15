//! Market data payloads: quotes, depth, trades and bars.

use serde::{Deserialize, Serialize};

use super::common::{ResponseStatus, null_default, opt_lenient_i64};
use super::enums::*;

/// Full quote for one symbol. Property names are the abbreviated wire names;
/// the same object is used by `GET /market/quotes` and the stream's `q` field.
///
/// Stream updates may be partial: any field can be absent.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Quote {
    /// Exchange symbol.
    #[serde(rename = "s")]
    pub symbol: Option<String>,
    /// Last trade price.
    #[serde(rename = "l")]
    pub last: Option<f64>,
    /// Last trade size.
    #[serde(rename = "sz")]
    pub last_size: Option<f64>,
    /// Change of last price from previous settlement.
    #[serde(rename = "ch")]
    pub change: Option<f64>,
    #[serde(rename = "op")]
    pub open: Option<f64>,
    #[serde(rename = "hi")]
    pub high: Option<f64>,
    #[serde(rename = "lo")]
    pub low: Option<f64>,
    /// Aggressor side of the last trade.
    #[serde(rename = "ags")]
    pub aggressor_side: Option<AggressorSide>,
    #[serde(rename = "td")]
    pub tick_direction: Option<TickDirection>,
    /// Settlement price.
    #[serde(rename = "stt")]
    pub settlement: Option<f64>,
    /// Trade date (`YYYYMMDD`) of the most recent settlement.
    #[serde(rename = "stts")]
    pub settlement_date: Option<String>,
    /// Time the settlement was sent, ms since epoch.
    #[serde(rename = "sttst")]
    pub settlement_time: Option<i64>,
    #[serde(rename = "pstt")]
    pub prev_settlement: Option<f64>,
    #[serde(rename = "pstts")]
    pub prev_settlement_date: Option<String>,
    #[serde(rename = "sttch")]
    pub settlement_change: Option<f64>,
    #[serde(rename = "hb")]
    pub high_bid: Option<f64>,
    #[serde(rename = "la")]
    pub low_ask: Option<f64>,
    #[serde(rename = "b")]
    pub bid: Option<f64>,
    /// Bid time, ms since epoch.
    #[serde(rename = "bt")]
    pub bid_time: Option<i64>,
    #[serde(rename = "bs")]
    pub bid_size: Option<f64>,
    /// Number of implied orders at the bid.
    #[serde(rename = "ibc")]
    pub implied_bid_count: Option<f64>,
    #[serde(rename = "ibs")]
    pub implied_bid_size: Option<f64>,
    #[serde(rename = "a")]
    pub ask: Option<f64>,
    /// Ask time, ms since epoch.
    #[serde(rename = "at")]
    pub ask_time: Option<i64>,
    #[serde(rename = "as")]
    pub ask_size: Option<f64>,
    #[serde(rename = "ias")]
    pub implied_ask_size: Option<f64>,
    /// Number of implied orders at the ask.
    #[serde(rename = "iac")]
    pub implied_ask_count: Option<f64>,
    /// Last trade time, ms since epoch.
    #[serde(rename = "tt")]
    pub trade_time: Option<i64>,
    /// Trade date (`YYYYMMDD`) of the last trade.
    #[serde(rename = "tdt")]
    pub trade_date: Option<String>,
    #[serde(rename = "secs")]
    pub security_status: Option<SecurityStatus>,
    /// Session date (`YYYYMMDD`).
    #[serde(rename = "sdt")]
    pub session_date: Option<String>,
    #[serde(rename = "oi")]
    pub open_interest: Option<f64>,
    #[serde(rename = "tv")]
    pub total_volume: Option<f64>,
    #[serde(rename = "bv")]
    pub block_volume: Option<f64>,
    #[serde(rename = "swv")]
    pub swaps_volume: Option<f64>,
    #[serde(rename = "pv")]
    pub physical_volume: Option<f64>,
}

/// `GET /market/quotes`. The wire property is capitalised (`Quotes`).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct QuotesResponse {
    pub status: Option<ResponseStatus>,
    pub message: Option<String>,
    #[serde(rename = "Quotes", alias = "quotes", deserialize_with = "null_default")]
    pub quotes: Vec<Quote>,
}

/// One price level of the book.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct DepthLevel {
    /// Zero-based level number.
    #[serde(rename = "l")]
    pub level: Option<i64>,
    /// Milliseconds since the Unix epoch.
    #[serde(rename = "t")]
    pub time: Option<i64>,
    #[serde(rename = "s")]
    pub side: Option<BookSide>,
    #[serde(rename = "p")]
    pub price: Option<f64>,
    /// Number of orders at the level.
    #[serde(rename = "o")]
    pub orders: Option<i64>,
    /// Total size at the level.
    #[serde(rename = "sz")]
    pub size: Option<f64>,
    /// Number of implied orders at the level.
    #[serde(rename = "ioc")]
    pub implied_orders: Option<i64>,
    /// Total implied size at the level.
    #[serde(rename = "is")]
    pub implied_size: Option<f64>,
}

/// Market depth of one symbol.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Depth {
    #[serde(rename = "s")]
    pub symbol: Option<String>,
    #[serde(rename = "b", deserialize_with = "null_default")]
    pub bids: Vec<DepthLevel>,
    #[serde(rename = "a", deserialize_with = "null_default")]
    pub asks: Vec<DepthLevel>,
}

/// `GET /market/depth`. The wire property is capitalised (`Depths`).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct DepthResponse {
    pub status: Option<ResponseStatus>,
    pub message: Option<String>,
    #[serde(rename = "Depths", alias = "depths", deserialize_with = "null_default")]
    pub depths: Vec<Depth>,
}

/// A market trade (time & sales). Accepts both the REST long names and the
/// stream's abbreviated names.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Trade {
    #[serde(alias = "s")]
    pub symbol: Option<String>,
    #[serde(alias = "p")]
    pub price: Option<f64>,
    #[serde(alias = "ch")]
    pub change: Option<f64>,
    #[serde(alias = "sz")]
    pub size: Option<f64>,
    #[serde(alias = "sq", deserialize_with = "opt_lenient_i64")]
    pub sequence_number: Option<i64>,
    /// Milliseconds since the Unix epoch.
    #[serde(alias = "st")]
    pub send_time: Option<i64>,
    #[serde(alias = "td")]
    pub tick_direction: Option<TickDirection>,
    #[serde(alias = "as")]
    pub aggressor_side: Option<AggressorSide>,
    /// `YYYYMMDD`.
    #[serde(alias = "tdt")]
    pub trade_date: Option<String>,
    #[serde(alias = "tid", deserialize_with = "opt_lenient_i64")]
    pub trade_id: Option<i64>,
    /// REST only.
    pub total_volume: Option<f64>,
    /// Stream only: settlement trade.
    #[serde(alias = "is")]
    pub is_settlement: Option<bool>,
    /// Stream only: cancelled trade.
    #[serde(alias = "clx")]
    pub is_cancelled: Option<bool>,
    /// Stream only.
    #[serde(alias = "spt")]
    pub system_priced_trade: Option<SystemPricedTrade>,
    /// Stream only.
    #[serde(alias = "ist")]
    pub investigation_status: Option<InvestigationStatus>,
    /// Stream only.
    #[serde(alias = "bt")]
    pub block_trade: Option<BlockTrade>,
}

/// `GET /market/trades/...`.
///
/// The schema names the list `traders` (sic). `trades` is accepted as an alias
/// in case the server uses the evident intended name.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct TradesResponse {
    pub status: Option<ResponseStatus>,
    pub message: Option<String>,
    #[serde(
        rename = "traders",
        alias = "trades",
        deserialize_with = "null_default"
    )]
    pub trades: Vec<Trade>,
}

/// An OHLCV bar delivered on the stream (`tb`, `tc`, `ti`, `vb`). The four bar
/// schemas are identical.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Bar {
    /// Bar time, ms since epoch.
    #[serde(rename = "t")]
    pub time: Option<i64>,
    #[serde(rename = "o")]
    pub open: Option<f64>,
    #[serde(rename = "h")]
    pub high: Option<f64>,
    #[serde(rename = "l")]
    pub low: Option<f64>,
    #[serde(rename = "c")]
    pub close: Option<f64>,
    #[serde(rename = "v")]
    pub volume: Option<f64>,
    #[serde(rename = "tc")]
    pub trade_count: Option<i64>,
    #[serde(rename = "d")]
    pub delta: Option<f64>,
    /// Indicator id this bar belongs to.
    #[serde(rename = "i")]
    pub indicator: Option<String>,
}
