//! A local, consistent view of order state.

use std::collections::{HashMap, HashSet};

use crate::models::{Fill, Order, OrderRequest};

/// Where a piece of order information came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Source {
    /// The WebSocket stream's `o` field.
    Stream,
    /// A REST response.
    Rest,
}

/// An order together with tracking metadata.
#[derive(Debug, Clone, PartialEq)]
pub struct TrackedOrder {
    /// Latest known state.
    pub order: Order,
    /// Source of the latest accepted update.
    pub source: Source,
    /// `true` when the tracker believes the order is open but the last
    /// complete REST snapshot did not contain it. The true state is unknown
    /// until another update arrives.
    pub unconfirmed: bool,
    pub(crate) first_seen: u64,
    pub(crate) last_update: u64,
}

/// A change to the tracked state.
#[derive(Debug, Clone, PartialEq)]
#[allow(clippy::large_enum_variant)]
pub enum OrderChange {
    /// An order the tracker had not seen before.
    Created(Order),
    /// A known order changed.
    Updated {
        /// State before the update.
        previous: Order,
        /// State after the update.
        current: Order,
    },
}

impl OrderChange {
    /// The order after the change.
    pub fn order(&self) -> &Order {
        match self {
            Self::Created(o) => o,
            Self::Updated { current, .. } => current,
        }
    }
}

/// Result of reconciling the tracker against a REST snapshot.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ReconcileReport {
    /// Changes the snapshot produced (missed creations, fills, cancels, …).
    pub changes: Vec<OrderChange>,
    /// Ids of orders the tracker believed open that the snapshot did not
    /// contain. They are kept and flagged
    /// [`unconfirmed`](TrackedOrder::unconfirmed), never silently dropped.
    pub missing: Vec<String>,
    /// Ids of orders whose snapshot record was ignored because the tracker
    /// already held newer information.
    pub stale: Vec<String>,
}

/// Merges order information from the stream and from REST.
///
/// The API provides no sequence numbers on order updates, so freshness is
/// inferred from two invariants:
///
/// 1. **Terminal states are final.** Once an order is filled, cancelled,
///    rejected or expired, an update carrying a non-terminal status is stale.
/// 2. **Cumulative fill quantity never decreases.** An update with a smaller
///    filled quantity than already known is stale.
///
/// In addition, a REST snapshot never overrides information that arrived
/// *after* the snapshot request was started (see
/// [`apply_snapshot`](Self::apply_snapshot)).
///
/// Stream updates may be partial; absent fields keep their previous values.
#[derive(Debug, Default)]
pub struct OrderTracker {
    orders: HashMap<String, TrackedOrder>,
    seq: u64,
    claimed_orders: HashSet<String>,
    claimed_strategies: HashSet<i64>,
    fills_seen: HashSet<String>,
}

impl OrderTracker {
    /// Creates an empty tracker.
    pub fn new() -> Self {
        Self::default()
    }

    /// Monotonic counter, incremented on every accepted update. Take it
    /// *before* requesting a REST snapshot and pass it to
    /// [`apply_snapshot`](Self::apply_snapshot).
    pub fn watermark(&self) -> u64 {
        self.seq
    }

    /// Number of tracked orders.
    pub fn len(&self) -> usize {
        self.orders.len()
    }

    /// `true` when nothing is tracked.
    pub fn is_empty(&self) -> bool {
        self.orders.is_empty()
    }

    /// Looks up an order by id.
    pub fn get(&self, order_id: &str) -> Option<&TrackedOrder> {
        self.orders.get(order_id)
    }

    /// All tracked orders, in no particular order.
    pub fn orders(&self) -> impl Iterator<Item = &TrackedOrder> {
        self.orders.values()
    }

    /// Orders that are pending or working.
    pub fn open_orders(&self) -> impl Iterator<Item = &TrackedOrder> {
        self.orders.values().filter(|t| t.order.status.is_open())
    }

    /// Applies one order record. Returns the resulting change, or `None` when
    /// the record was stale or changed nothing.
    pub fn apply(&mut self, incoming: Order, source: Source) -> Option<OrderChange> {
        match self.orders.get_mut(&incoming.order_id) {
            None => {
                self.seq += 1;
                let tracked = TrackedOrder {
                    order: incoming.clone(),
                    source,
                    unconfirmed: false,
                    first_seen: self.seq,
                    last_update: self.seq,
                };
                self.orders.insert(incoming.order_id.clone(), tracked);
                Some(OrderChange::Created(incoming))
            }
            Some(tracked) => {
                if is_stale(&tracked.order, &incoming) {
                    tracing::debug!(
                        order_id = %incoming.order_id,
                        known = %tracked.order.status,
                        incoming = %incoming.status,
                        "ignoring stale order update"
                    );
                    return None;
                }
                let merged = merge(&tracked.order, incoming);
                let was_unconfirmed = std::mem::replace(&mut tracked.unconfirmed, false);
                if merged == tracked.order {
                    if was_unconfirmed {
                        tracked.source = source;
                    }
                    return None;
                }
                self.seq += 1;
                let previous = std::mem::replace(&mut tracked.order, merged.clone());
                tracked.source = source;
                tracked.last_update = self.seq;
                Some(OrderChange::Updated {
                    previous,
                    current: merged,
                })
            }
        }
    }

    /// Reconciles against a REST snapshot of one account's orders.
    ///
    /// * `as_of` — the [`watermark`](Self::watermark) taken before the
    ///   snapshot was requested. Orders updated after it (by the stream, while
    ///   the request was in flight) are left alone: the stream is newer.
    /// * `complete` — whether the snapshot lists *all* of the account's orders
    ///   (status filter `ANY`). Only then are absent orders reported missing.
    pub fn apply_snapshot(
        &mut self,
        account_id: &str,
        snapshot: Vec<Order>,
        as_of: u64,
        complete: bool,
    ) -> ReconcileReport {
        let mut report = ReconcileReport::default();
        let mut present: HashSet<String> = HashSet::with_capacity(snapshot.len());
        for order in snapshot {
            present.insert(order.order_id.clone());
            if let Some(tracked) = self.orders.get(&order.order_id) {
                if tracked.last_update > as_of || is_stale(&tracked.order, &order) {
                    report.stale.push(order.order_id.clone());
                    continue;
                }
            }
            if let Some(change) = self.apply(order, Source::Rest) {
                report.changes.push(change);
            }
        }
        if complete {
            for tracked in self.orders.values_mut() {
                let same_account = tracked
                    .order
                    .account_id
                    .as_deref()
                    .is_none_or(|a| a == account_id);
                if same_account
                    && tracked.order.status.is_open()
                    && tracked.last_update <= as_of
                    && !present.contains(&tracked.order.order_id)
                {
                    tracked.unconfirmed = true;
                    report.missing.push(tracked.order.order_id.clone());
                }
            }
            report.missing.sort();
        }
        report
    }

    /// Records a fill and reports whether it is new. The server replays a
    /// snapshot of existing fills after every (re)connect; this filters the
    /// repeats.
    pub fn record_fill(&mut self, fill: &Fill) -> bool {
        self.fills_seen.insert(fill_key(fill))
    }

    /// Drops orders in a terminal state to bound memory in long-running
    /// processes, returning how many were removed.
    ///
    /// Caveat: the terminal-state guard goes with them. If the server later
    /// repeats one of these orders (for instance in a REST snapshot), it is
    /// reported as [`OrderChange::Created`] again. De-duplication keys of
    /// fills are kept.
    pub fn prune_terminal(&mut self) -> usize {
        let before = self.orders.len();
        self.orders.retain(|_, t| !t.order.status.is_terminal());
        before - self.orders.len()
    }

    /// Marks an order / strategy id as belonging to a submission whose result
    /// is known, so it is never matched to an unknown outcome.
    pub(crate) fn claim(&mut self, order_id: Option<&str>, strategy_id: Option<i64>) {
        if let Some(id) = order_id {
            self.claimed_orders.insert(id.to_owned());
        }
        if let Some(sid) = strategy_id {
            self.claimed_strategies.insert(sid);
        }
    }

    fn is_claimed(&self, order: &Order) -> bool {
        self.claimed_orders.contains(&order.order_id)
            || order
                .strategy_id
                .is_some_and(|s| self.claimed_strategies.contains(&s))
    }

    /// Orders that could be the result of `request`, submitted when the
    /// tracker was at `watermark`: first seen afterwards, not attributed to
    /// another submission, not a bracket child, and matching the request's
    /// symbol, side, quantity, type and prices.
    pub fn candidates_for(
        &self,
        account_id: &str,
        request: &OrderRequest,
        watermark: Option<u64>,
    ) -> Vec<Order> {
        let mut found: Vec<&TrackedOrder> = self
            .orders
            .values()
            .filter(|t| watermark.is_none_or(|w| t.first_seen > w))
            .filter(|t| {
                t.order
                    .account_id
                    .as_deref()
                    .is_none_or(|a| a == account_id)
            })
            .filter(|t| t.order.parent_order_id.as_deref().is_none_or(str::is_empty))
            .filter(|t| !self.is_claimed(&t.order))
            .filter(|t| matches_request(&t.order, request))
            .collect();
        found.sort_by_key(|t| t.first_seen);
        found.into_iter().map(|t| t.order.clone()).collect()
    }
}

fn is_stale(known: &Order, incoming: &Order) -> bool {
    if known.status.is_terminal() && !incoming.status.is_terminal() {
        return true;
    }
    matches!(
        (known.fill_quantity, incoming.fill_quantity),
        (Some(k), Some(i)) if i < k
    )
}

/// Field-wise merge: values present in `incoming` win, absent ones are kept.
fn merge(known: &Order, incoming: Order) -> Order {
    Order {
        order_id: incoming.order_id,
        strategy_id: incoming.strategy_id.or(known.strategy_id),
        parent_order_id: incoming
            .parent_order_id
            .or_else(|| known.parent_order_id.clone()),
        account_id: incoming.account_id.or_else(|| known.account_id.clone()),
        exch_sym: incoming.exch_sym.or_else(|| known.exch_sym.clone()),
        status: incoming.status,
        side: incoming.side.or_else(|| known.side.clone()),
        quantity: incoming.quantity.or(known.quantity),
        limit_price: incoming.limit_price.or(known.limit_price),
        stop_price: incoming.stop_price.or(known.stop_price),
        order_type: incoming.order_type.or_else(|| known.order_type.clone()),
        duration: incoming.duration.or_else(|| known.duration.clone()),
        fill_quantity: incoming.fill_quantity.or(known.fill_quantity),
        fill_price: incoming.fill_price.or(known.fill_price),
        fill_date: incoming.fill_date.or_else(|| known.fill_date.clone()),
        child_orders: if incoming.child_orders.is_empty() {
            known.child_orders.clone()
        } else {
            incoming.child_orders
        },
        order_error: incoming.order_error.or_else(|| known.order_error.clone()),
    }
}

fn approx_eq(a: f64, b: f64) -> bool {
    (a - b).abs() <= 1e-9 * a.abs().max(b.abs()).max(1.0)
}

/// A field matches when the order does not report it or reports the same
/// value as the request.
fn matches_request(order: &Order, request: &OrderRequest) -> bool {
    let price_ok = |requested: Option<f64>, reported: Option<f64>| match (requested, reported) {
        (Some(r), Some(o)) => approx_eq(r, o),
        _ => true,
    };
    order
        .exch_sym
        .as_deref()
        .is_none_or(|s| s == request.exch_sym)
        && order.side.as_ref().is_none_or(|s| *s == request.side)
        && order
            .quantity
            .is_none_or(|q| approx_eq(q, request.quantity))
        && order
            .order_type
            .as_ref()
            .is_none_or(|t| *t == request.order_type)
        && price_ok(request.limit_price, order.limit_price)
        && price_ok(request.stop_price, order.stop_price)
}

fn fill_key(fill: &Fill) -> String {
    match fill.order_update_id.as_deref().filter(|s| !s.is_empty()) {
        Some(ouid) => format!("u:{ouid}"),
        // No update id: fall back to a composite of the identifying fields.
        None => format!(
            "c:{}|{}|{}|{}|{}",
            fill.order_id.as_deref().unwrap_or(""),
            fill.time_order_event
                .map(|t| t.to_string())
                .unwrap_or_default(),
            fill.fill_total_quantity
                .map(|q| q.to_string())
                .unwrap_or_default(),
            fill.fill_quantity
                .map(|q| q.to_string())
                .unwrap_or_default(),
            fill.fill_price.map(|p| p.to_string()).unwrap_or_default(),
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{OrderSide, OrderStatus, OrderType};
    use serde_json::json;

    fn order(id: &str, status: &str, filled: Option<f64>) -> Order {
        let mut v = json!({
            "orderId": id, "accountId": "A1", "exchSym": "XCME:ES.Z26", "status": status,
            "side": "BUY", "quantity": 5, "orderType": "2", "duration": "0", "limitPrice": 5000.0
        });
        if let Some(f) = filled {
            v["fillQuantity"] = json!(f);
        }
        serde_json::from_value(v).unwrap()
    }

    #[test]
    fn create_update_and_noop() {
        let mut t = OrderTracker::new();
        assert!(matches!(
            t.apply(order("1", "NEW", None), Source::Stream),
            Some(OrderChange::Created(_))
        ));
        assert_eq!(t.apply(order("1", "NEW", None), Source::Stream), None);
        let change = t
            .apply(order("1", "PARTIALLY_FILLED", Some(2.0)), Source::Stream)
            .unwrap();
        match change {
            OrderChange::Updated { previous, current } => {
                assert_eq!(previous.status, OrderStatus::New);
                assert_eq!(current.fill_quantity, Some(2.0));
            }
            other => panic!("{other:?}"),
        }
        assert_eq!(t.open_orders().count(), 1);
    }

    #[test]
    fn terminal_state_is_sticky() {
        let mut t = OrderTracker::new();
        t.apply(order("1", "FILLED", Some(5.0)), Source::Stream);
        assert_eq!(
            t.apply(order("1", "PARTIALLY_FILLED", Some(5.0)), Source::Stream),
            None
        );
        assert_eq!(t.apply(order("1", "NEW", None), Source::Rest), None);
        assert_eq!(t.get("1").unwrap().order.status, OrderStatus::Filled);
        assert_eq!(t.open_orders().count(), 0);
    }

    #[test]
    fn fill_quantity_never_decreases() {
        let mut t = OrderTracker::new();
        t.apply(order("1", "PARTIALLY_FILLED", Some(3.0)), Source::Stream);
        assert_eq!(
            t.apply(order("1", "PARTIALLY_FILLED", Some(1.0)), Source::Stream),
            None
        );
        assert_eq!(t.get("1").unwrap().order.fill_quantity, Some(3.0));
        assert!(
            t.apply(order("1", "PARTIALLY_FILLED", Some(4.0)), Source::Stream)
                .is_some()
        );
    }

    #[test]
    fn partial_updates_keep_known_fields() {
        let mut t = OrderTracker::new();
        t.apply(order("1", "NEW", None), Source::Rest);
        let partial: Order =
            serde_json::from_value(json!({"oid": "1", "st": "CANCELLED"})).unwrap();
        let change = t.apply(partial, Source::Stream).unwrap();
        let cur = change.order();
        assert_eq!(cur.status, OrderStatus::Cancelled);
        assert_eq!(cur.exch_sym.as_deref(), Some("XCME:ES.Z26"));
        assert_eq!(cur.limit_price, Some(5000.0));
    }

    #[test]
    fn snapshot_reports_changes_missing_and_stale() {
        let mut t = OrderTracker::new();
        t.apply(order("1", "NEW", None), Source::Stream); // will be filled in snapshot
        t.apply(order("2", "NEW", None), Source::Stream); // absent from snapshot
        t.apply(order("3", "FILLED", Some(5.0)), Source::Stream); // snapshot is behind
        let as_of = t.watermark();
        let report = t.apply_snapshot(
            "A1",
            vec![
                order("1", "FILLED", Some(5.0)),
                order("3", "PARTIALLY_FILLED", Some(2.0)),
                order("4", "NEW", None),
            ],
            as_of,
            true,
        );
        assert_eq!(report.changes.len(), 2, "{report:?}");
        assert_eq!(report.missing, vec!["2".to_owned()]);
        assert_eq!(report.stale, vec!["3".to_owned()]);
        assert!(t.get("2").unwrap().unconfirmed);
        assert_eq!(t.get("3").unwrap().order.status, OrderStatus::Filled);
        assert_eq!(t.get("4").unwrap().source, Source::Rest);

        // A later update for the missing order clears the flag.
        t.apply(order("2", "CANCELLED", None), Source::Stream);
        assert!(!t.get("2").unwrap().unconfirmed);
    }

    #[test]
    fn snapshot_does_not_override_or_flag_newer_stream_data() {
        let mut t = OrderTracker::new();
        t.apply(order("1", "NEW", None), Source::Stream);
        let as_of = t.watermark();
        // While the REST request is in flight the stream delivers news.
        let mut repriced = order("1", "REPLACED", None);
        repriced.limit_price = Some(5001.0);
        t.apply(repriced, Source::Stream);
        t.apply(order("9", "NEW", None), Source::Stream);
        // The snapshot predates both.
        let report = t.apply_snapshot("A1", vec![order("1", "NEW", None)], as_of, true);
        assert!(report.changes.is_empty());
        assert_eq!(report.stale, vec!["1".to_owned()]);
        assert!(
            report.missing.is_empty(),
            "order 9 must not be flagged: {report:?}"
        );
        assert_eq!(t.get("1").unwrap().order.limit_price, Some(5001.0));
    }

    #[test]
    fn incomplete_snapshot_never_flags_missing() {
        let mut t = OrderTracker::new();
        t.apply(order("1", "NEW", None), Source::Stream);
        let as_of = t.watermark();
        let report = t.apply_snapshot("A1", vec![], as_of, false);
        assert!(report.missing.is_empty());
        let report = t.apply_snapshot("OTHER", vec![], as_of, true);
        assert!(report.missing.is_empty(), "other account's snapshot");
    }

    #[test]
    fn pruning_drops_only_terminal_orders() {
        let mut t = OrderTracker::new();
        t.apply(order("1", "FILLED", Some(5.0)), Source::Stream);
        t.apply(order("2", "NEW", None), Source::Stream);
        t.apply(order("3", "CANCELLED", None), Source::Stream);
        assert_eq!(t.prune_terminal(), 2);
        assert_eq!(t.len(), 1);
        assert!(t.get("2").is_some());
    }

    #[test]
    fn fills_are_deduplicated() {
        let mut t = OrderTracker::new();
        let f: Fill = serde_json::from_value(json!({"oid": "1", "fq": 1, "ouid": "10-1"})).unwrap();
        assert!(t.record_fill(&f));
        assert!(!t.record_fill(&f));
        let g: Fill =
            serde_json::from_value(json!({"oid": "1", "fq": 1, "ftq": 2, "t": 5})).unwrap();
        assert!(t.record_fill(&g));
        assert!(!t.record_fill(&g));
    }

    #[test]
    fn candidates_respect_watermark_claims_and_fingerprint() {
        let req = OrderRequest::limit("XCME:ES.Z26", OrderSide::Buy, 5.0, 5000.0);
        let mut t = OrderTracker::new();
        t.apply(order("old", "NEW", None), Source::Rest);
        let w = t.watermark();
        t.apply(order("new", "NEW", None), Source::Rest);
        let mut other_price = order("px", "NEW", None);
        other_price.limit_price = Some(4999.0);
        t.apply(other_price, Source::Rest);
        let mut child = order("child", "NEW", None);
        child.parent_order_id = Some("new".into());
        t.apply(child, Source::Rest);
        let mut sell = order("sell", "NEW", None);
        sell.side = Some(OrderSide::Sell);
        t.apply(sell, Source::Rest);
        let mut market = order("mkt", "NEW", None);
        market.order_type = Some(OrderType::Market);
        t.apply(market, Source::Rest);

        let ids = |v: Vec<Order>| v.into_iter().map(|o| o.order_id).collect::<Vec<_>>();
        assert_eq!(ids(t.candidates_for("A1", &req, Some(w))), vec!["new"]);
        assert_eq!(ids(t.candidates_for("A1", &req, None)), vec!["old", "new"]);
        assert!(t.candidates_for("A2", &req, Some(w)).is_empty());
        t.claim(Some("new"), None);
        assert!(t.candidates_for("A1", &req, Some(w)).is_empty());
    }
}
