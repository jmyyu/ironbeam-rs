//! WebSocket stream payloads and indicator subscription types.

use serde::{Deserialize, Deserializer, Serialize};
use serde_json::{Map, Value};

use super::account::{AccountPositions, Balance, Fill, Position, RiskInfo};
use super::common::{ApiResponse, ResponseStatus, null_default, one_or_many};
use super::enums::BarType;
use super::market::{Bar, Depth, Quote, Trade};
use super::order::Order;

/// `GET /stream/create` response.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct StreamIdResponse {
    pub status: Option<ResponseStatus>,
    pub message: Option<String>,
    pub stream_id: Option<String>,
}

/// Keep-alive sent by the server every 5 seconds (`{"p":{"ping":"ping"}}`).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Ping {
    pub ping: Option<String>,
}

fn string_matrix<'de, D: Deserializer<'de>>(d: D) -> Result<Vec<Vec<String>>, D::Error> {
    let raw = Option::<Vec<Vec<Value>>>::deserialize(d)?.unwrap_or_default();
    Ok(raw
        .into_iter()
        .map(|row| {
            row.into_iter()
                .map(|v| match v {
                    Value::String(s) => s,
                    Value::Null => String::new(),
                    other => other.to_string(),
                })
                .collect()
        })
        .collect())
}

/// Values of one indicator subscription (`i` field).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct IndicatorValues {
    /// Indicator id the values belong to.
    #[serde(rename = "n")]
    pub name: Option<String>,
    /// Index of the first row.
    #[serde(rename = "fi")]
    pub first_index: Option<i64>,
    /// Rows of values, in the order of
    /// [`IndicatorSubscribeResponse::value_names`]. Always strings.
    #[serde(rename = "v", deserialize_with = "string_matrix")]
    pub values: Vec<Vec<String>>,
}

/// One frame of the WebSocket stream. Every field is optional; a frame
/// usually carries one or two of them.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct StreamMessage {
    /// Keep-alive.
    #[serde(rename = "p", skip_serializing_if = "Option::is_none")]
    pub ping: Option<Ping>,
    /// Quote updates.
    #[serde(
        rename = "q",
        deserialize_with = "one_or_many",
        skip_serializing_if = "Vec::is_empty"
    )]
    pub quotes: Vec<Quote>,
    /// Depth updates.
    #[serde(
        rename = "d",
        deserialize_with = "one_or_many",
        skip_serializing_if = "Vec::is_empty"
    )]
    pub depths: Vec<Depth>,
    /// Trades.
    #[serde(
        rename = "tr",
        deserialize_with = "one_or_many",
        skip_serializing_if = "Vec::is_empty"
    )]
    pub trades: Vec<Trade>,
    /// Trade bars.
    #[serde(
        rename = "tb",
        deserialize_with = "one_or_many",
        skip_serializing_if = "Vec::is_empty"
    )]
    pub trade_bars: Vec<Bar>,
    /// Tick bars.
    #[serde(
        rename = "tc",
        deserialize_with = "one_or_many",
        skip_serializing_if = "Vec::is_empty"
    )]
    pub tick_bars: Vec<Bar>,
    /// Time bars.
    #[serde(
        rename = "ti",
        deserialize_with = "one_or_many",
        skip_serializing_if = "Vec::is_empty"
    )]
    pub time_bars: Vec<Bar>,
    /// Volume bars.
    #[serde(
        rename = "vb",
        deserialize_with = "one_or_many",
        skip_serializing_if = "Vec::is_empty"
    )]
    pub volume_bars: Vec<Bar>,
    /// Generic indicator values.
    #[serde(
        rename = "i",
        deserialize_with = "one_or_many",
        skip_serializing_if = "Vec::is_empty"
    )]
    pub indicators: Vec<IndicatorValues>,
    /// Order updates (new orders and changes).
    #[serde(
        rename = "o",
        deserialize_with = "one_or_many",
        skip_serializing_if = "Vec::is_empty"
    )]
    pub orders: Vec<Order>,
    /// Fills. A snapshot of existing fills is sent after each (re)connect.
    #[serde(
        rename = "f",
        deserialize_with = "one_or_many",
        skip_serializing_if = "Vec::is_empty"
    )]
    pub fills: Vec<Fill>,
    /// Position changes.
    #[serde(
        rename = "ps",
        deserialize_with = "one_or_many",
        skip_serializing_if = "Vec::is_empty"
    )]
    pub positions: Vec<Position>,
    /// Initial snapshot of positions for all accounts.
    #[serde(
        rename = "psa",
        deserialize_with = "one_or_many",
        skip_serializing_if = "Vec::is_empty"
    )]
    pub positions_snapshot: Vec<AccountPositions>,
    /// Balance change (documented as a single object; a list is accepted).
    #[serde(
        rename = "b",
        deserialize_with = "one_or_many",
        skip_serializing_if = "Vec::is_empty"
    )]
    pub balances: Vec<Balance>,
    /// Initial snapshot of balances for all accounts.
    #[serde(
        rename = "ba",
        deserialize_with = "one_or_many",
        skip_serializing_if = "Vec::is_empty"
    )]
    pub balances_snapshot: Vec<Balance>,
    /// Risk change (documented as a single object; a list is accepted).
    #[serde(
        rename = "ri",
        deserialize_with = "one_or_many",
        skip_serializing_if = "Vec::is_empty"
    )]
    pub risk: Vec<RiskInfo>,
    /// Initial snapshot of risk info for all accounts.
    #[serde(
        rename = "ria",
        deserialize_with = "one_or_many",
        skip_serializing_if = "Vec::is_empty"
    )]
    pub risk_snapshot: Vec<RiskInfo>,
    /// "Reset": the account changed (settings, trading day open/close).
    #[serde(rename = "r", skip_serializing_if = "Option::is_none")]
    pub reset: Option<ApiResponse>,
    /// Top-level properties this SDK version does not know about.
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

impl StreamMessage {
    /// `true` when the frame carries nothing but a keep-alive.
    pub fn is_ping_only(&self) -> bool {
        self.ping.is_some() && self.is_empty_ignoring_ping()
    }

    /// `true` when the frame carries no payload other than (possibly) a ping.
    pub fn is_empty_ignoring_ping(&self) -> bool {
        self.quotes.is_empty()
            && self.depths.is_empty()
            && self.trades.is_empty()
            && self.trade_bars.is_empty()
            && self.tick_bars.is_empty()
            && self.time_bars.is_empty()
            && self.volume_bars.is_empty()
            && self.indicators.is_empty()
            && self.orders.is_empty()
            && self.fills.is_empty()
            && self.positions.is_empty()
            && self.positions_snapshot.is_empty()
            && self.balances.is_empty()
            && self.balances_snapshot.is_empty()
            && self.risk.is_empty()
            && self.risk_snapshot.is_empty()
            && self.reset.is_none()
            && self.extra.is_empty()
    }
}

/// Body of the four bar-indicator subscription endpoints.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BarSubscription {
    pub symbol: String,
    /// Number of `bar_type` units per bar.
    pub period: u32,
    pub bar_type: BarType,
    /// Number of historical bars to load initially.
    pub load_size: u32,
}

impl BarSubscription {
    /// Creates a bar subscription request.
    pub fn new(symbol: impl Into<String>, period: u32, bar_type: BarType, load_size: u32) -> Self {
        Self {
            symbol: symbol.into(),
            period,
            bar_type,
            load_size,
        }
    }
}

/// Response of the bar-indicator subscription endpoints.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct IndicatorSubscribeResponse {
    pub status: Option<ResponseStatus>,
    pub message: Option<String>,
    /// Id used on the stream and for unsubscribing. Changes on every
    /// subscription, hence after every reconnect.
    pub indicator_id: Option<String>,
    #[serde(deserialize_with = "null_default")]
    pub value_names: Vec<String>,
    #[serde(deserialize_with = "null_default")]
    pub value_types: Vec<String>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn ping_frame() {
        let m: StreamMessage = serde_json::from_str(r#"{"p":{"ping":"ping"}}"#).unwrap();
        assert!(m.is_ping_only());
    }

    #[test]
    fn documented_risk_sample_and_single_objects() {
        let m: StreamMessage = serde_json::from_value(json!({
            "ri": {"a": "5123345", "rc": "INVALID", "cc": "USD", "lv": 2000, "snlv": 2000,
                   "cnlv": 2000, "mnlv": 2000, "mnlvmd": 2500, "le": [0]},
            "b": {"a": "5123345", "cc": "USD", "cb": 1000.5},
            "r": {"status": "INFO", "message": "Account changed"}
        }))
        .unwrap();
        assert_eq!(m.risk.len(), 1);
        assert_eq!(m.risk[0].max_net_liquidation_value_multi_day, Some(2500.0));
        assert_eq!(m.risk[0].is_liquidated(), None);
        assert_eq!(m.balances[0].cash_balance, Some(1000.5));
        assert_eq!(m.reset.unwrap().message.as_deref(), Some("Account changed"));
    }

    #[test]
    fn unknown_fields_are_kept() {
        let m: StreamMessage =
            serde_json::from_value(json!({"zz": [1, 2], "q": [{"s": "XCME:ES.Z26", "b": 1.5}]}))
                .unwrap();
        assert_eq!(m.extra["zz"], json!([1, 2]));
        assert_eq!(m.quotes[0].bid, Some(1.5));
        assert!(!m.is_ping_only());
    }

    #[test]
    fn indicator_values_tolerate_non_strings() {
        let m: StreamMessage = serde_json::from_value(
            json!({"i": [{"n": "TradeBars_1", "fi": 0, "v": [["1.5", 2, null]]}]}),
        )
        .unwrap();
        assert_eq!(
            m.indicators[0].values,
            vec![vec!["1.5".to_owned(), "2".into(), String::new()]]
        );
    }
}
