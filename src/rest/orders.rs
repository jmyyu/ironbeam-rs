use serde_json::json;

use crate::client::{Call, Client};
use crate::endpoints as ep;
use crate::error::{Error, Result};
use crate::models::*;
use crate::orders::UnknownOutcome;

impl Client {
    /// Submits a new order. **Never retried.**
    ///
    /// Outcomes:
    /// * `Ok(ids)` — accepted by the API. `ids.strategy_id` identifies the
    ///   submission; `ids.order_id` may be absent (see
    ///   [`OrderRequest::wait_for_order_id`]) and can be looked up later with
    ///   [`order_id_for_strategy`](Self::order_id_for_strategy).
    /// * `Err(Error::OrderOutcomeUnknown(_))` — the request may have reached
    ///   the server but no usable answer came back (timeout, connection lost,
    ///   5xx, undecodable body). **The order may be working.** Resolve it with
    ///   [`OrderManager::investigate`](crate::orders::OrderManager::investigate)
    ///   or by inspecting [`orders`](Self::orders); do not resubmit blindly.
    /// * any other `Err` — definitively not accepted (rejected locally, never
    ///   sent, or refused with a 4xx).
    ///
    /// Prefer [`OrderManager::submit`](crate::orders::OrderManager::submit),
    /// which additionally records what is needed to resolve unknown outcomes.
    pub async fn place_order(&self, account_id: &str, order: &OrderRequest) -> Result<OrderIds> {
        order.validate()?;
        let body = order.to_wire(account_id, self.config().enum_encoding);
        let call = Call::new(&ep::PLACE_ORDER)
            .params(&[("accountId", account_id)])
            .body(body)
            .timeout(self.config().order_timeout);
        match self.send(call).await {
            Ok(ids) => Ok(ids),
            Err(e) if e.is_ambiguous() => Err(Error::OrderOutcomeUnknown(Box::new(
                UnknownOutcome::new(account_id, order.clone(), e),
            ))),
            Err(e) => Err(e),
        }
    }

    /// Modifies a working order. **Never retried.**
    ///
    /// If the call fails with an [ambiguous](Error::is_ambiguous) error the
    /// modification may or may not have been applied; re-read the order
    /// before acting.
    pub async fn update_order(
        &self,
        account_id: &str,
        order_id: &str,
        update: &OrderUpdate,
    ) -> Result<OrdersResponse> {
        update.validate()?;
        let call = Call::new(&ep::UPDATE_ORDER)
            .params(&[("accountId", account_id), ("orderId", order_id)])
            .body(update.to_wire(order_id))
            .timeout(self.config().order_timeout);
        self.send(call).await
    }

    /// Cancels an order. **Never retried**: after an ambiguous failure the
    /// order may already be cancelled, and a second cancel would then report a
    /// misleading error. Cancelling again is harmless if you choose to.
    pub async fn cancel_order(&self, account_id: &str, order_id: &str) -> Result<OrdersResponse> {
        let call = Call::new(&ep::CANCEL_ORDER)
            .params(&[("accountId", account_id), ("orderId", order_id)])
            .timeout(self.config().order_timeout);
        self.send(call).await
    }

    /// Cancels several orders of one account. **Never retried.**
    pub async fn cancel_orders<S: AsRef<str>>(
        &self,
        account_id: &str,
        order_ids: &[S],
    ) -> Result<OrdersResponse> {
        if order_ids.is_empty() {
            return Err(Error::InvalidRequest("no order ids given".into()));
        }
        let ids: Vec<&str> = order_ids.iter().map(AsRef::as_ref).collect();
        if ids.iter().any(|id| id.is_empty()) {
            return Err(Error::InvalidRequest("empty order id".into()));
        }
        let call = Call::new(&ep::CANCEL_MULTIPLE)
            .params(&[("accountId", account_id)])
            .body(json!({ "accountId": account_id, "orderIds": ids }))
            .timeout(self.config().order_timeout);
        self.send(call).await
    }

    /// Orders of an account filtered by status. Use [`OrderStatus::Any`] for
    /// all of them.
    pub async fn orders(&self, account_id: &str, status: OrderStatus) -> Result<OrdersResponse> {
        let params = [("accountId", account_id), ("orderStatus", status.as_wire())];
        self.send(Call::new(&ep::ORDERS).params(&params)).await
    }

    /// Fills of the account's orders.
    pub async fn order_fills(&self, account_id: &str) -> Result<FillsResponse> {
        self.send(Call::new(&ep::ORDER_FILLS).params(&[("accountId", account_id)]))
            .await
    }

    /// Looks up the order id that belongs to a strategy id.
    pub async fn order_id_for_strategy(
        &self,
        account_id: &str,
        strategy_id: i64,
    ) -> Result<OrderIds> {
        let sid = strategy_id.to_string();
        let params = [("accountId", account_id), ("strategyId", sid.as_str())];
        self.send(Call::new(&ep::ORDER_ID_FOR_STRATEGY).params(&params))
            .await
    }

    /// Looks up the strategy id that belongs to an order id.
    pub async fn strategy_id_for_order(
        &self,
        account_id: &str,
        order_id: &str,
    ) -> Result<OrderIds> {
        let params = [("accountId", account_id), ("orderId", order_id)];
        self.send(Call::new(&ep::STRATEGY_ID_FOR_ORDER).params(&params))
            .await
    }
}
