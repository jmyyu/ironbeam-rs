//! Order payloads.

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

use super::common::{
    ResponseStatus, null_default, opt_lenient_i64, opt_string_or_number, string_or_number,
};
use super::enums::*;
use crate::config::EnumEncoding;
use crate::error::{Error, Result};

/// Error details attached to an order (`orderError` / `err`).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct OrderError {
    pub error_code: Option<i64>,
    pub error_text: Option<String>,
}

/// An order as reported by the REST API (long names) or the stream's `o`
/// field (abbreviated names, accepted as aliases).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Order {
    /// Server order id. Required: an order without an id cannot be tracked.
    #[serde(alias = "oid", deserialize_with = "string_or_number")]
    pub order_id: String,
    /// Strategy id assigned at submission.
    #[serde(default, alias = "sid", deserialize_with = "opt_lenient_i64")]
    pub strategy_id: Option<i64>,
    /// Parent order id for bracket children.
    #[serde(default, alias = "poid", deserialize_with = "opt_string_or_number")]
    pub parent_order_id: Option<String>,
    #[serde(default, alias = "a")]
    pub account_id: Option<String>,
    #[serde(default, alias = "s")]
    pub exch_sym: Option<String>,
    /// Required: state reconciliation is meaningless without it.
    #[serde(alias = "st")]
    pub status: OrderStatus,
    #[serde(default, alias = "sd")]
    pub side: Option<OrderSide>,
    #[serde(default, alias = "q")]
    pub quantity: Option<f64>,
    #[serde(default, alias = "lp")]
    pub limit_price: Option<f64>,
    #[serde(default, alias = "sp")]
    pub stop_price: Option<f64>,
    #[serde(default, alias = "ot")]
    pub order_type: Option<OrderType>,
    #[serde(default, alias = "dr")]
    pub duration: Option<OrderDuration>,
    /// Cumulative filled quantity.
    #[serde(default, alias = "fq")]
    pub fill_quantity: Option<f64>,
    /// Average fill price.
    #[serde(default, alias = "fp")]
    pub fill_price: Option<f64>,
    /// ISO-8601 timestamp of the last fill, kept as sent.
    #[serde(default, alias = "fd")]
    pub fill_date: Option<String>,
    /// Ids of bracket children (stop loss, take profit).
    #[serde(default, alias = "cor", deserialize_with = "null_default")]
    pub child_orders: Vec<String>,
    #[serde(default, alias = "err")]
    pub order_error: Option<OrderError>,
}

impl Order {
    /// Quantity not yet filled, when both figures are known.
    pub fn remaining_quantity(&self) -> Option<f64> {
        Some((self.quantity? - self.fill_quantity.unwrap_or(0.0)).max(0.0))
    }
}

/// Order lists: `GET /order/{accountId}/{orderStatus}` and the responses of
/// update, cancel and cancel-multiple.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct OrdersResponse {
    pub status: Option<ResponseStatus>,
    pub message: Option<String>,
    #[serde(deserialize_with = "null_default")]
    pub orders: Vec<Order>,
}

/// Response of order placement and of the strategy-id/order-id lookups.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct OrderIds {
    pub status: Option<ResponseStatus>,
    pub message: Option<String>,
    /// Exchange order id. Absent when `waitForOrderId` was `false` or the
    /// exchange did not answer within the server's 10 s wait.
    #[serde(deserialize_with = "opt_string_or_number")]
    pub order_id: Option<String>,
    /// Strategy id the server assigned to the submission.
    #[serde(deserialize_with = "opt_lenient_i64")]
    pub strategy_id: Option<i64>,
}

/// A new order.
///
/// Build one with [`market`](Self::market), [`limit`](Self::limit),
/// [`stop`](Self::stop) or [`stop_limit`](Self::stop_limit) and refine it with
/// the chained setters.
///
/// ```
/// use ironbeam_sdk::models::{OrderRequest, OrderSide, OrderDuration};
///
/// let order = OrderRequest::limit("XCME:ES.Z26", OrderSide::Buy, 1.0, 5000.25)
///     .duration(OrderDuration::GoodTillCancel)
///     .stop_loss(4990.0)
///     .take_profit(5020.0);
/// assert!(order.validate().is_ok());
/// ```
#[derive(Debug, Clone, PartialEq)]
pub struct OrderRequest {
    /// Exchange-qualified symbol, e.g. `XCME:ES.Z26`.
    pub exch_sym: String,
    pub side: OrderSide,
    pub quantity: f64,
    pub order_type: OrderType,
    /// Defaults to [`OrderDuration::Day`].
    pub duration: OrderDuration,
    pub limit_price: Option<f64>,
    pub stop_price: Option<f64>,
    /// Bracket stop-loss price.
    pub stop_loss: Option<f64>,
    /// Bracket take-profit price.
    pub take_profit: Option<f64>,
    pub stop_loss_offset: Option<f64>,
    pub take_profit_offset: Option<f64>,
    /// Documented as "currently not supported".
    pub trailing_stop: Option<f64>,
    /// `None` leaves the server default (`true`: wait up to 10 s for the
    /// exchange order id).
    pub wait_for_order_id: Option<bool>,
    /// Extra top-level properties merged into the request body verbatim.
    ///
    /// An escape hatch for properties that appear in the documentation's
    /// examples but not in its schema (e.g. `clientOrderId`). Nothing here is
    /// validated and nothing is known about how the server treats it.
    pub extra: Map<String, Value>,
}

impl OrderRequest {
    fn new(exch_sym: impl Into<String>, side: OrderSide, quantity: f64, ty: OrderType) -> Self {
        Self {
            exch_sym: exch_sym.into(),
            side,
            quantity,
            order_type: ty,
            duration: OrderDuration::Day,
            limit_price: None,
            stop_price: None,
            stop_loss: None,
            take_profit: None,
            stop_loss_offset: None,
            take_profit_offset: None,
            trailing_stop: None,
            wait_for_order_id: None,
            extra: Map::new(),
        }
    }

    /// Market order.
    pub fn market(exch_sym: impl Into<String>, side: OrderSide, quantity: f64) -> Self {
        Self::new(exch_sym, side, quantity, OrderType::Market)
    }

    /// Limit order.
    pub fn limit(exch_sym: impl Into<String>, side: OrderSide, quantity: f64, price: f64) -> Self {
        let mut o = Self::new(exch_sym, side, quantity, OrderType::Limit);
        o.limit_price = Some(price);
        o
    }

    /// Stop (stop-market) order.
    pub fn stop(exch_sym: impl Into<String>, side: OrderSide, quantity: f64, stop: f64) -> Self {
        let mut o = Self::new(exch_sym, side, quantity, OrderType::Stop);
        o.stop_price = Some(stop);
        o
    }

    /// Stop-limit order.
    pub fn stop_limit(
        exch_sym: impl Into<String>,
        side: OrderSide,
        quantity: f64,
        stop: f64,
        limit: f64,
    ) -> Self {
        let mut o = Self::new(exch_sym, side, quantity, OrderType::StopLimit);
        o.stop_price = Some(stop);
        o.limit_price = Some(limit);
        o
    }

    /// Sets the time in force.
    pub fn duration(mut self, duration: OrderDuration) -> Self {
        self.duration = duration;
        self
    }

    /// Attaches a bracket stop-loss at an absolute price.
    pub fn stop_loss(mut self, price: f64) -> Self {
        self.stop_loss = Some(price);
        self
    }

    /// Attaches a bracket take-profit at an absolute price.
    pub fn take_profit(mut self, price: f64) -> Self {
        self.take_profit = Some(price);
        self
    }

    /// Attaches a bracket stop-loss as an offset.
    pub fn stop_loss_offset(mut self, offset: f64) -> Self {
        self.stop_loss_offset = Some(offset);
        self
    }

    /// Attaches a bracket take-profit as an offset.
    pub fn take_profit_offset(mut self, offset: f64) -> Self {
        self.take_profit_offset = Some(offset);
        self
    }

    /// Sets `waitForOrderId`.
    pub fn wait_for_order_id(mut self, wait: bool) -> Self {
        self.wait_for_order_id = Some(wait);
        self
    }

    /// Adds an undocumented top-level property; see [`extra`](Self::extra).
    pub fn extra_field(mut self, name: impl Into<String>, value: impl Into<Value>) -> Self {
        self.extra.insert(name.into(), value.into());
        self
    }

    /// Checks the request for errors that can be detected locally.
    pub fn validate(&self) -> Result<()> {
        let bad = |msg: String| Err(Error::InvalidRequest(msg));
        validate_symbol(&self.exch_sym)?;
        if !(self.quantity.is_finite() && self.quantity > 0.0) {
            return bad(format!(
                "quantity must be a positive finite number, got {}",
                self.quantity
            ));
        }
        match self.side {
            OrderSide::Buy | OrderSide::Sell => {}
            ref other => return bad(format!("side must be BUY or SELL, got {other:?}")),
        }
        if matches!(
            self.duration,
            OrderDuration::Invalid | OrderDuration::Unknown(_)
        ) {
            return bad(format!("unsupported duration {:?}", self.duration));
        }
        let prices = [
            ("limitPrice", self.limit_price),
            ("stopPrice", self.stop_price),
            ("stopLoss", self.stop_loss),
            ("takeProfit", self.take_profit),
            ("stopLossOffset", self.stop_loss_offset),
            ("takeProfitOffset", self.take_profit_offset),
            ("trailingStop", self.trailing_stop),
        ];
        for (name, value) in prices {
            if let Some(v) = value {
                if !v.is_finite() {
                    return bad(format!("{name} must be finite, got {v}"));
                }
            }
        }
        match self.order_type {
            OrderType::Market => {}
            OrderType::Limit => {
                if self.limit_price.is_none() {
                    return bad("a LIMIT order requires limitPrice".into());
                }
            }
            OrderType::Stop => {
                if self.stop_price.is_none() {
                    return bad("a STOP order requires stopPrice".into());
                }
            }
            OrderType::StopLimit => {
                if self.stop_price.is_none() || self.limit_price.is_none() {
                    return bad("a STOP_LIMIT order requires stopPrice and limitPrice".into());
                }
            }
            ref other => return bad(format!("unsupported order type {other:?}")),
        }
        if self.stop_loss.is_some() && self.stop_loss_offset.is_some() {
            return bad("stopLoss and stopLossOffset are mutually exclusive".into());
        }
        if self.take_profit.is_some() && self.take_profit_offset.is_some() {
            return bad("takeProfit and takeProfitOffset are mutually exclusive".into());
        }
        if let (Some(sl), Some(tp)) = (self.stop_loss, self.take_profit) {
            if sl == tp {
                return bad("stopLoss and takeProfit must differ".into());
            }
        }
        Ok(())
    }

    /// Builds the JSON request body.
    pub fn to_wire(&self, account_id: &str, encoding: EnumEncoding) -> Value {
        let (order_type, duration) = match encoding {
            EnumEncoding::Codes => (self.order_type.as_wire(), self.duration.as_wire()),
            EnumEncoding::Names => (self.order_type.name(), self.duration.name()),
        };
        let mut body = Map::new();
        // Undocumented extras first so that documented fields always win.
        for (k, v) in &self.extra {
            body.insert(k.clone(), v.clone());
        }
        body.insert("accountId".into(), json!(account_id));
        body.insert("exchSym".into(), json!(self.exch_sym));
        body.insert("side".into(), json!(self.side.as_wire()));
        body.insert("quantity".into(), json!(self.quantity));
        body.insert("orderType".into(), json!(order_type));
        body.insert("duration".into(), json!(duration));
        let optional = [
            ("limitPrice", self.limit_price),
            ("stopPrice", self.stop_price),
            ("stopLoss", self.stop_loss),
            ("takeProfit", self.take_profit),
            ("stopLossOffset", self.stop_loss_offset),
            ("takeProfitOffset", self.take_profit_offset),
            ("trailingStop", self.trailing_stop),
        ];
        for (name, value) in optional {
            if let Some(v) = value {
                body.insert(name.into(), json!(v));
            }
        }
        if let Some(wait) = self.wait_for_order_id {
            body.insert("waitForOrderId".into(), json!(wait));
        }
        Value::Object(body)
    }
}

/// Changes to an existing order (`PUT /order/{accountId}/update/{orderId}`).
///
/// `quantity` is required by the API even when unchanged. For bracket orders
/// the quantity cannot be changed and `stop_loss`/`take_profit` are required.
#[derive(Debug, Clone, PartialEq)]
pub struct OrderUpdate {
    /// New (or unchanged) total quantity. An integer per the schema.
    pub quantity: u32,
    pub limit_price: Option<f64>,
    pub stop_price: Option<f64>,
    pub stop_loss: Option<f64>,
    pub take_profit: Option<f64>,
    pub stop_loss_offset: Option<f64>,
    pub take_profit_offset: Option<f64>,
    pub trailing_stop: Option<f64>,
}

impl OrderUpdate {
    /// An update carrying only the (mandatory) quantity.
    pub fn new(quantity: u32) -> Self {
        Self {
            quantity,
            limit_price: None,
            stop_price: None,
            stop_loss: None,
            take_profit: None,
            stop_loss_offset: None,
            take_profit_offset: None,
            trailing_stop: None,
        }
    }

    /// Sets a new limit price.
    pub fn limit_price(mut self, price: f64) -> Self {
        self.limit_price = Some(price);
        self
    }

    /// Sets a new stop price.
    pub fn stop_price(mut self, price: f64) -> Self {
        self.stop_price = Some(price);
        self
    }

    /// Sets a new bracket stop-loss price.
    pub fn stop_loss(mut self, price: f64) -> Self {
        self.stop_loss = Some(price);
        self
    }

    /// Sets a new bracket take-profit price.
    pub fn take_profit(mut self, price: f64) -> Self {
        self.take_profit = Some(price);
        self
    }

    /// Checks the update for errors that can be detected locally.
    pub fn validate(&self) -> Result<()> {
        let bad = |msg: String| Err(Error::InvalidRequest(msg));
        if self.quantity == 0 {
            return bad("quantity must be positive".into());
        }
        let prices = [
            ("limitPrice", self.limit_price),
            ("stopPrice", self.stop_price),
            ("stopLoss", self.stop_loss),
            ("takeProfit", self.take_profit),
            ("stopLossOffset", self.stop_loss_offset),
            ("takeProfitOffset", self.take_profit_offset),
            ("trailingStop", self.trailing_stop),
        ];
        for (name, value) in prices {
            if let Some(v) = value {
                if !v.is_finite() {
                    return bad(format!("{name} must be finite, got {v}"));
                }
            }
        }
        if self.stop_loss.is_some() && self.stop_loss_offset.is_some() {
            return bad("stopLoss and stopLossOffset are mutually exclusive".into());
        }
        if self.take_profit.is_some() && self.take_profit_offset.is_some() {
            return bad("takeProfit and takeProfitOffset are mutually exclusive".into());
        }
        if let (Some(sl), Some(tp)) = (self.stop_loss, self.take_profit) {
            if sl == tp {
                return bad("stopLoss and takeProfit must differ".into());
            }
        }
        Ok(())
    }

    /// Builds the JSON request body.
    pub fn to_wire(&self, order_id: &str) -> Value {
        let mut body = Map::new();
        body.insert("orderId".into(), json!(order_id));
        body.insert("quantity".into(), json!(self.quantity));
        let optional = [
            ("limitPrice", self.limit_price),
            ("stopPrice", self.stop_price),
            ("stopLoss", self.stop_loss),
            ("takeProfit", self.take_profit),
            ("stopLossOffset", self.stop_loss_offset),
            ("takeProfitOffset", self.take_profit_offset),
            ("trailingStop", self.trailing_stop),
        ];
        for (name, value) in optional {
            if let Some(v) = value {
                body.insert(name.into(), json!(v));
            }
        }
        Value::Object(body)
    }
}

/// Minimal sanity check for symbols placed in URLs and bodies.
///
/// The schema's pattern (`^([A-Z]{1,10}:)?[A-Z0-9 _\.]{1,120}$`) is violated by
/// the documentation's own examples (`XCME:6E1!`), so only what would corrupt
/// a request is rejected here.
pub(crate) fn validate_symbol(symbol: &str) -> Result<()> {
    if symbol.is_empty() || symbol.len() > 130 {
        return Err(Error::InvalidRequest(format!(
            "symbol must be 1..=130 characters, got {} ({symbol:?})",
            symbol.len()
        )));
    }
    if symbol
        .chars()
        .any(|c| c == ',' || c == '/' || c == '?' || c == '#' || c.is_control())
    {
        return Err(Error::InvalidRequest(format!(
            "symbol contains illegal characters: {symbol:?}"
        )));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wire_body_uses_schema_codes_by_default() {
        let body = OrderRequest::limit("XCME:ES.Z26", OrderSide::Buy, 2.0, 5000.25)
            .to_wire("51234", EnumEncoding::Codes);
        assert_eq!(
            body,
            json!({
                "accountId": "51234", "exchSym": "XCME:ES.Z26", "side": "BUY", "quantity": 2.0,
                "orderType": "2", "duration": "0", "limitPrice": 5000.25
            })
        );
    }

    #[test]
    fn wire_body_names_dialect() {
        let body = OrderRequest::stop_limit("XCME:ES.Z26", OrderSide::Sell, 1.0, 4990.0, 4989.0)
            .duration(OrderDuration::GoodTillCancel)
            .wait_for_order_id(false)
            .to_wire("51234", EnumEncoding::Names);
        assert_eq!(body["orderType"], "STOP_LIMIT");
        assert_eq!(body["duration"], "GOOD_TILL_CANCEL");
        assert_eq!(body["waitForOrderId"], false);
        assert_eq!(body["stopPrice"], 4990.0);
    }

    #[test]
    fn extra_fields_cannot_override_documented_ones() {
        let body = OrderRequest::market("XCME:ES.Z26", OrderSide::Buy, 1.0)
            .extra_field("clientOrderId", "abc")
            .extra_field("quantity", 100)
            .to_wire("51234", EnumEncoding::Codes);
        assert_eq!(body["clientOrderId"], "abc");
        assert_eq!(body["quantity"], 1.0);
    }

    #[test]
    fn validation() {
        let ok = OrderRequest::market("XCME:ES.Z26", OrderSide::Buy, 1.0);
        assert!(ok.validate().is_ok());
        let mut o = ok.clone();
        o.quantity = 0.0;
        assert!(o.validate().is_err());
        o.quantity = f64::NAN;
        assert!(o.validate().is_err());
        let mut o = OrderRequest::limit("XCME:ES.Z26", OrderSide::Buy, 1.0, 1.0);
        o.limit_price = None;
        assert!(o.validate().is_err());
        let mut o = OrderRequest::stop_limit("XCME:ES.Z26", OrderSide::Buy, 1.0, 2.0, 3.0);
        o.limit_price = None;
        assert!(o.validate().is_err());
        assert!(
            ok.clone()
                .stop_loss(1.0)
                .stop_loss_offset(1.0)
                .validate()
                .is_err()
        );
        assert!(
            ok.clone()
                .stop_loss(5.0)
                .take_profit(5.0)
                .validate()
                .is_err()
        );
        let mut o = ok.clone();
        o.side = OrderSide::Invalid;
        assert!(o.validate().is_err());
        let mut o = ok.clone();
        o.exch_sym = "A,B".into();
        assert!(o.validate().is_err());
        let mut o = ok;
        o.limit_price = Some(f64::INFINITY);
        assert!(o.validate().is_err());
        assert!(OrderUpdate::new(0).validate().is_err());
        assert!(OrderUpdate::new(1).limit_price(10.0).validate().is_ok());
    }

    #[test]
    fn order_decodes_from_rest_and_stream_shapes() {
        let rest: Order = serde_json::from_value(json!({
            "orderId": "5123-1", "strategyId": 77, "accountId": "5123", "exchSym": "XCME:ES.Z26",
            "status": "PARTIALLY_FILLED", "side": "BUY", "quantity": 3, "limitPrice": 5000.0,
            "orderType": "LIMIT", "duration": "DAY", "fillQuantity": 1, "childOrders": null
        }))
        .unwrap();
        let stream: Order = serde_json::from_value(json!({
            "oid": "5123-1", "sid": 77, "a": "5123", "s": "XCME:ES.Z26", "st": "PARTIALLY_FILLED",
            "sd": "BUY", "q": 3, "lp": 5000.0, "ot": "2", "dr": "0", "fq": 1
        }))
        .unwrap();
        assert_eq!(rest, stream);
        assert_eq!(rest.remaining_quantity(), Some(2.0));
        assert!(serde_json::from_value::<Order>(json!({"status": "NEW"})).is_err());
        assert!(serde_json::from_value::<Order>(json!({"orderId": "1"})).is_err());
    }
}
