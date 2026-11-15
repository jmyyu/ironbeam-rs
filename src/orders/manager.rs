//! Safe order submission and state reconciliation on top of a [`Client`].

use std::sync::Mutex;
use std::time::Duration;

use crate::client::Client;
use crate::error::{Error, Result};
use crate::models::{Fill, Order, OrderRequest, OrderStatus, OrderUpdate, OrdersResponse};
use crate::stream::StreamEvent;

use super::UnknownOutcome;
use super::tracker::{OrderChange, OrderTracker, ReconcileReport, Source, TrackedOrder};

/// A submission the API accepted.
#[derive(Debug, Clone, PartialEq)]
pub struct Submission {
    /// Strategy id assigned by the server.
    pub strategy_id: Option<i64>,
    /// Exchange order id, when the server already had it.
    pub order_id: Option<String>,
}

/// Result of investigating an [`UnknownOutcome`].
#[derive(Debug, Clone, PartialEq)]
#[allow(clippy::large_enum_variant)]
pub enum Investigation {
    /// No order matching the submission exists (yet). This is **not** proof
    /// that the order was not accepted: it may still be queued. Investigate
    /// again after a delay before concluding anything.
    NotFound,
    /// Exactly one order matching the submission appeared after it was sent.
    /// The match is by symbol, side, quantity, type and prices — the API has
    /// no documented client order id — so review it before relying on it.
    Found(Order),
    /// Several orders match; the SDK cannot tell which (if any) belongs to
    /// the submission. Needs a human or strategy-level decision.
    Ambiguous(Vec<Order>),
}

/// What an incoming stream event meant for the tracked orders.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct OrderEvents {
    /// Order changes carried by the event.
    pub changes: Vec<OrderChange>,
    /// Fills not seen before (replayed fill snapshots are filtered out).
    pub fills: Vec<Fill>,
    /// Set when the event triggered a REST reconciliation (stream
    /// (re)connected).
    pub reconciled: Option<ReconcileReport>,
}

/// Order entry and order-state reconciliation for one account.
///
/// ```no_run
/// # async fn demo(client: ironbeam_sdk::Client) -> ironbeam_sdk::Result<()> {
/// use ironbeam_sdk::{Error, models::{OrderRequest, OrderSide}, orders::{Investigation, OrderManager}};
///
/// let orders = OrderManager::new(client, "5123345");
/// let request = OrderRequest::limit("XCME:ES.Z26", OrderSide::Buy, 1.0, 5000.0);
/// match orders.submit(request).await {
///     Ok(accepted) => println!("accepted: {accepted:?}"),
///     Err(Error::OrderOutcomeUnknown(unknown)) => {
///         // Never resubmit here. Find out what happened first.
///         match orders.investigate(&unknown).await? {
///             Investigation::Found(order) => println!("it went through: {}", order.order_id),
///             Investigation::NotFound => println!("not visible (yet) — check again later"),
///             Investigation::Ambiguous(c) => println!("{} candidates — manual review", c.len()),
///         }
///     }
///     Err(rejected) => println!("definitely not placed: {rejected}"),
/// }
/// # Ok(()) }
/// ```
#[derive(Debug)]
pub struct OrderManager {
    client: Client,
    account_id: String,
    state: Mutex<State>,
}

#[derive(Debug, Default)]
struct State {
    tracker: OrderTracker,
    synced: bool,
}

impl OrderManager {
    /// Creates a manager for one account. Performs no I/O.
    pub fn new(client: Client, account_id: impl Into<String>) -> Self {
        Self {
            client,
            account_id: account_id.into(),
            state: Mutex::default(),
        }
    }

    /// The account this manager trades.
    pub fn account_id(&self) -> &str {
        &self.account_id
    }

    /// The underlying client.
    pub fn client(&self) -> &Client {
        &self.client
    }

    fn state(&self) -> std::sync::MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Runs `f` with read access to the tracker.
    pub fn with_tracker<R>(&self, f: impl FnOnce(&OrderTracker) -> R) -> R {
        f(&self.state().tracker)
    }

    /// Latest known state of an order.
    pub fn order(&self, order_id: &str) -> Option<TrackedOrder> {
        self.state().tracker.get(order_id).cloned()
    }

    /// Orders currently believed to be pending or working.
    pub fn open_orders(&self) -> Vec<TrackedOrder> {
        self.state().tracker.open_orders().cloned().collect()
    }

    /// Forgets orders in a terminal state and returns how many were dropped.
    /// See [`OrderTracker::prune_terminal`] for the caveat.
    pub fn prune_terminal(&self) -> usize {
        self.state().tracker.prune_terminal()
    }

    /// Fetches all of the account's orders over REST and merges them into the
    /// tracker. Call after anything that may have caused missed updates; this
    /// is done automatically by [`on_event`](Self::on_event) when the stream
    /// (re)connects.
    pub async fn reconcile(&self) -> Result<ReconcileReport> {
        let as_of = self.state().tracker.watermark();
        let snapshot = self
            .client
            .orders(&self.account_id, OrderStatus::Any)
            .await?;
        let mut state = self.state();
        let report = state
            .tracker
            .apply_snapshot(&self.account_id, snapshot.orders, as_of, true);
        state.synced = true;
        if !report.missing.is_empty() {
            tracing::warn!(
                account = %self.account_id,
                missing = ?report.missing,
                "orders believed open are absent from the server's order list"
            );
        }
        Ok(report)
    }

    async fn ensure_synced(&self) -> Result<()> {
        if self.state().synced {
            return Ok(());
        }
        self.reconcile().await.map(|_| ())
    }

    /// Submits an order exactly once.
    ///
    /// Before the first submission the manager takes a baseline of the
    /// account's existing orders, so that an unknown outcome can later be
    /// told apart from orders that were already there.
    ///
    /// See [`Client::place_order`] for the three possible outcomes.
    pub async fn submit(&self, request: OrderRequest) -> Result<Submission> {
        request.validate()?;
        self.ensure_synced().await?;
        let watermark = self.state().tracker.watermark();
        match self.client.place_order(&self.account_id, &request).await {
            Ok(ids) => {
                self.state()
                    .tracker
                    .claim(ids.order_id.as_deref(), ids.strategy_id);
                Ok(Submission {
                    strategy_id: ids.strategy_id,
                    order_id: ids.order_id,
                })
            }
            Err(Error::OrderOutcomeUnknown(mut unknown)) => {
                unknown.watermark = Some(watermark);
                tracing::warn!(
                    account = %self.account_id,
                    symbol = %request.exch_sym,
                    cause = %unknown.cause,
                    "order outcome unknown; not retrying"
                );
                Err(Error::OrderOutcomeUnknown(unknown))
            }
            Err(e) => Err(e),
        }
    }

    /// Finds out what became of a submission with an unknown outcome by
    /// re-reading the account's orders. Read-only and safe to repeat.
    ///
    /// When exactly one match is found it is attributed to the submission and
    /// will not be matched again.
    pub async fn investigate(&self, unknown: &UnknownOutcome) -> Result<Investigation> {
        if unknown.account_id != self.account_id {
            return Err(Error::InvalidRequest(format!(
                "outcome belongs to account {}, this manager handles {}",
                unknown.account_id, self.account_id
            )));
        }
        self.reconcile().await?;
        let mut state = self.state();
        let mut candidates =
            state
                .tracker
                .candidates_for(&self.account_id, &unknown.request, unknown.watermark);
        Ok(match candidates.len() {
            0 => Investigation::NotFound,
            1 => {
                let order = candidates.remove(0);
                state
                    .tracker
                    .claim(Some(&order.order_id), order.strategy_id);
                Investigation::Found(order)
            }
            _ => Investigation::Ambiguous(candidates),
        })
    }

    /// Repeats [`investigate`](Self::investigate) until something other than
    /// `NotFound` comes back or `attempts` are used up, sleeping `interval`
    /// between attempts.
    pub async fn investigate_until_found(
        &self,
        unknown: &UnknownOutcome,
        attempts: u32,
        interval: Duration,
    ) -> Result<Investigation> {
        let mut last = Investigation::NotFound;
        for attempt in 0..attempts.max(1) {
            if attempt > 0 {
                tokio::time::sleep(interval).await;
            }
            last = self.investigate(unknown).await?;
            if last != Investigation::NotFound {
                break;
            }
        }
        Ok(last)
    }

    /// Looks up the exchange order id of an accepted submission that did not
    /// return one, polling `attempts` times.
    pub async fn resolve_order_id(
        &self,
        strategy_id: i64,
        attempts: u32,
        interval: Duration,
    ) -> Result<Option<String>> {
        for attempt in 0..attempts.max(1) {
            if attempt > 0 {
                tokio::time::sleep(interval).await;
            }
            match self
                .client
                .order_id_for_strategy(&self.account_id, strategy_id)
                .await
            {
                Ok(ids) => {
                    if let Some(id) = ids.order_id.filter(|s| !s.is_empty()) {
                        self.state().tracker.claim(Some(&id), Some(strategy_id));
                        return Ok(Some(id));
                    }
                }
                // Not known yet.
                Err(Error::Api { status: 404, .. }) => {}
                Err(e) => return Err(e),
            }
        }
        Ok(None)
    }

    fn absorb(&self, response: &OrdersResponse) {
        let mut state = self.state();
        for order in &response.orders {
            state.tracker.apply(order.clone(), Source::Rest);
        }
    }

    /// Cancels an order (never retried) and records the returned state.
    ///
    /// After an [ambiguous](Error::is_ambiguous) failure call
    /// [`reconcile`](Self::reconcile) to learn whether the cancel took effect.
    pub async fn cancel(&self, order_id: &str) -> Result<OrdersResponse> {
        let response = self.client.cancel_order(&self.account_id, order_id).await?;
        self.absorb(&response);
        Ok(response)
    }

    /// Cancels every order the tracker believes open (after a fresh
    /// reconciliation). Returns `None` when there was nothing to cancel.
    pub async fn cancel_all_open(&self) -> Result<Option<OrdersResponse>> {
        self.reconcile().await?;
        let ids: Vec<String> = self
            .state()
            .tracker
            .open_orders()
            .filter(|t| {
                t.order
                    .account_id
                    .as_deref()
                    .is_none_or(|a| a == self.account_id)
            })
            .map(|t| t.order.order_id.clone())
            .collect();
        if ids.is_empty() {
            return Ok(None);
        }
        let response = self.client.cancel_orders(&self.account_id, &ids).await?;
        self.absorb(&response);
        Ok(Some(response))
    }

    /// Modifies an order (never retried) and records the returned state.
    pub async fn update(&self, order_id: &str, update: &OrderUpdate) -> Result<OrdersResponse> {
        let response = self
            .client
            .update_order(&self.account_id, order_id, update)
            .await?;
        self.absorb(&response);
        Ok(response)
    }

    /// Feeds a stream event into the tracker.
    ///
    /// * order updates (`o`) for this account are merged;
    /// * fills (`f`) are de-duplicated;
    /// * on [`StreamEvent::Connected`] — the stream was (re)established and
    ///   updates may have been missed — a REST reconciliation is performed.
    pub async fn on_event(&self, event: &StreamEvent) -> Result<OrderEvents> {
        let mut out = OrderEvents::default();
        match event {
            StreamEvent::Data(message) => {
                let mut state = self.state();
                for order in &message.orders {
                    if order
                        .account_id
                        .as_deref()
                        .is_some_and(|a| a != self.account_id)
                    {
                        continue;
                    }
                    if let Some(change) = state.tracker.apply(order.clone(), Source::Stream) {
                        out.changes.push(change);
                    }
                }
                for fill in &message.fills {
                    if fill
                        .account_id
                        .as_deref()
                        .is_some_and(|a| a != self.account_id)
                    {
                        continue;
                    }
                    if state.tracker.record_fill(fill) {
                        out.fills.push(fill.clone());
                    }
                }
            }
            StreamEvent::Connected { .. } => {
                let report = self.reconcile().await?;
                out.changes.extend(report.changes.iter().cloned());
                out.reconciled = Some(report);
            }
            _ => {}
        }
        Ok(out)
    }
}
