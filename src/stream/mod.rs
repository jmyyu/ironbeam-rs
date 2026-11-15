//! Resilient WebSocket streaming.
//!
//! [`Stream::connect`] creates a stream id, opens the WebSocket and spawns a
//! background task that
//!
//! * decodes frames into [`StreamMessage`]s and delivers them in order,
//! * detects dead connections (the server pings every 5 s; silence for
//!   [`StreamConfig::idle_timeout`] is treated as a failure),
//! * reconnects with exponential backoff — with a **new stream id**, as the
//!   API requires — and
//! * restores every market-data and indicator subscription on the new stream.
//!
//! Subscriptions are held as *desired state* by the task, so a reconnect is
//! invisible to the consumer except for the [`StreamEvent::Disconnected`] /
//! [`StreamEvent::Connected`] pair. Data published by the server while the
//! connection was down is **not** replayed; consumers that need continuity
//! must re-read state over REST when they see `Connected` (the
//! [`OrderManager`](crate::orders::OrderManager) does this for orders).

mod task;

use std::collections::BTreeSet;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use tokio::sync::{mpsc, oneshot, watch};

use crate::client::Client;
use crate::error::{Error, Result};
use crate::models::{BarSubscription, StreamMessage, validate_symbol};
use crate::rest::{BarKind, MAX_SYMBOLS};

/// Tuning knobs for a [`Stream`].
#[derive(Debug, Clone, PartialEq)]
pub struct StreamConfig {
    /// Maximum silence tolerated on the socket before the connection is
    /// declared dead. The server sends a ping every 5 seconds.
    pub idle_timeout: Duration,
    /// Timeout for the WebSocket handshake.
    pub connect_timeout: Duration,
    /// Delay before the first reconnect attempt; doubles per failed attempt.
    pub reconnect_base_delay: Duration,
    /// Upper bound for the reconnect delay.
    pub reconnect_max_delay: Duration,
    /// Randomise reconnect delays in `[delay/2, delay]`.
    pub reconnect_jitter: bool,
    /// Give up after this many consecutive failed attempts (`None`: never).
    pub max_reconnect_attempts: Option<u32>,
    /// A connection that lived at least this long resets the backoff.
    pub stable_after: Duration,
    /// Capacity of the event channel. When it is full the socket is no longer
    /// read (back-pressure); nothing is dropped.
    pub event_buffer: usize,
}

impl Default for StreamConfig {
    fn default() -> Self {
        Self {
            idle_timeout: Duration::from_secs(20),
            connect_timeout: Duration::from_secs(15),
            reconnect_base_delay: Duration::from_millis(500),
            reconnect_max_delay: Duration::from_secs(30),
            reconnect_jitter: true,
            max_reconnect_attempts: None,
            stable_after: Duration::from_secs(30),
            event_buffer: 4096,
        }
    }
}

/// Market-data subscription kinds that take a symbol list.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum MarketDataKind {
    /// Quotes, delivered in `q`.
    Quotes,
    /// Market depth, delivered in `d`.
    Depths,
    /// Trades, delivered in `tr`.
    Trades,
}

/// Why a connection ended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DisconnectReason {
    /// Nothing (not even a ping) arrived within the idle timeout.
    IdleTimeout,
    /// The server closed the connection.
    ServerClosed {
        /// WebSocket close code, when a close frame was received.
        code: Option<u16>,
        /// Close reason text.
        reason: String,
    },
    /// The socket failed.
    SocketError(String),
    /// A (re)connect attempt failed.
    ConnectFailed(String),
    /// The socket opened but subscriptions could not be restored.
    RestoreFailed(String),
}

/// Stable identifier of an indicator subscription across reconnects. The
/// server-side indicator id changes with every reconnect; this does not.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct IndicatorKey(pub(crate) u64);

/// An active indicator subscription.
#[derive(Debug, Clone, PartialEq)]
pub struct IndicatorSubscription {
    /// Stable key for [`StreamHandle::unsubscribe_indicator`].
    pub key: IndicatorKey,
    /// Which bar indicator.
    pub kind: BarKind,
    /// The subscription request.
    pub request: BarSubscription,
    /// Current server-side id; bars and indicator values carry it.
    pub indicator_id: String,
    /// Names of the values delivered for this indicator.
    pub value_names: Vec<String>,
    /// Types of the values delivered for this indicator.
    pub value_types: Vec<String>,
}

/// The subscriptions a stream maintains.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Subscriptions {
    /// Symbols subscribed for quotes.
    pub quotes: BTreeSet<String>,
    /// Symbols subscribed for depth.
    pub depths: BTreeSet<String>,
    /// Symbols subscribed for trades.
    pub trades: BTreeSet<String>,
    /// Indicator subscriptions.
    pub indicators: Vec<IndicatorSubscription>,
}

/// What a [`Stream`] yields.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub enum StreamEvent {
    /// The socket is open and all subscriptions are in place. `generation`
    /// starts at 1 and increases with every reconnect. Updates published
    /// while disconnected were missed: re-read whatever state matters.
    Connected {
        /// Connection counter.
        generation: u64,
        /// The stream id in use.
        stream_id: String,
    },
    /// A decoded frame (keep-alive pings are consumed internally).
    Data(Box<StreamMessage>),
    /// A frame that could not be decoded. The connection stays up.
    Undecodable {
        /// Decoder error.
        error: String,
        /// The frame, truncated to 4 KiB.
        raw: String,
    },
    /// After a reconnect an indicator was re-created under a new id.
    IndicatorResubscribed {
        /// Id before the reconnect.
        previous_id: String,
        /// The subscription with its new id.
        subscription: IndicatorSubscription,
    },
    /// The server refused to restore a subscription after a reconnect; it has
    /// been dropped from the desired state.
    SubscriptionLost {
        /// What was lost, human readable (e.g. `quotes [XCME:ES.Z26]`).
        what: String,
        /// The server's error.
        error: String,
    },
    /// The connection ended (or a reconnect attempt failed); another attempt
    /// follows after `retry_in`.
    Disconnected {
        /// Why.
        reason: DisconnectReason,
        /// Delay before the next attempt.
        retry_in: Duration,
    },
    /// The stream stopped for good (authentication failure or attempts
    /// exhausted). No further events follow.
    Closed {
        /// Why.
        error: String,
    },
}

pub(crate) enum Command {
    Subscribe(MarketDataKind, Vec<String>, oneshot::Sender<Result<()>>),
    Unsubscribe(MarketDataKind, Vec<String>, oneshot::Sender<Result<()>>),
    AddBars(
        BarKind,
        BarSubscription,
        oneshot::Sender<Result<IndicatorSubscription>>,
    ),
    RemoveIndicator(IndicatorKey, oneshot::Sender<Result<()>>),
    Snapshot(oneshot::Sender<Subscriptions>),
}

#[derive(Debug)]
pub(crate) struct Shared {
    pub connected: AtomicBool,
    pub shutdown: watch::Sender<bool>,
}

/// A live stream: the event receiver plus a control handle.
#[derive(Debug)]
pub struct Stream {
    events: mpsc::Receiver<StreamEvent>,
    handle: StreamHandle,
}

impl Stream {
    /// Creates a stream id, opens the WebSocket and starts the background
    /// task. Fails if the *first* connection cannot be established; later
    /// failures are handled by reconnecting.
    ///
    /// Order, fill, position, balance and risk updates for the user's
    /// accounts arrive without any subscription; market data and indicators
    /// must be subscribed through the [`handle`](Self::handle).
    pub async fn connect(client: &Client, config: StreamConfig) -> Result<Self> {
        if config.event_buffer == 0 {
            return Err(Error::Config("event_buffer must be positive".into()));
        }
        let first = task::open(client, &config).await?;
        let (event_tx, event_rx) = mpsc::channel(config.event_buffer);
        let (cmd_tx, cmd_rx) = mpsc::channel(64);
        let (shutdown_tx, shutdown_rx) = watch::channel(false);
        let shared = Arc::new(Shared {
            connected: AtomicBool::new(false),
            shutdown: shutdown_tx,
        });
        let task = task::Task::new(client.clone(), config, event_tx, cmd_rx, shared.clone());
        tokio::spawn(task.run(first, shutdown_rx));
        Ok(Self {
            events: event_rx,
            handle: StreamHandle {
                commands: cmd_tx,
                shared,
            },
        })
    }

    /// Next event; `None` once the stream has stopped.
    pub async fn next(&mut self) -> Option<StreamEvent> {
        self.events.recv().await
    }

    /// A cloneable handle for managing subscriptions and shutting down.
    pub fn handle(&self) -> StreamHandle {
        self.handle.clone()
    }
}

/// Controls a [`Stream`] from any task.
///
/// Calling a handle method from the task that also reads the events is fine:
/// the stream task keeps serving commands even while the event channel is
/// full.
#[derive(Debug, Clone)]
pub struct StreamHandle {
    commands: mpsc::Sender<Command>,
    shared: Arc<Shared>,
}

fn checked_symbols<S: AsRef<str>>(symbols: &[S]) -> Result<Vec<String>> {
    if symbols.is_empty() {
        return Err(Error::InvalidRequest("no symbols given".into()));
    }
    if symbols.len() > MAX_SYMBOLS {
        return Err(Error::InvalidRequest(format!(
            "at most {MAX_SYMBOLS} symbols per stream and kind, got {}",
            symbols.len()
        )));
    }
    symbols
        .iter()
        .map(|s| validate_symbol(s.as_ref()).map(|()| s.as_ref().to_owned()))
        .collect()
}

impl StreamHandle {
    async fn request<T>(&self, build: impl FnOnce(oneshot::Sender<T>) -> Command) -> Result<T> {
        let (tx, rx) = oneshot::channel();
        self.commands
            .send(build(tx))
            .await
            .map_err(|_| Error::StreamClosed)?;
        rx.await.map_err(|_| Error::StreamClosed)
    }

    /// Adds symbols to a market-data subscription (at most 10 symbols per
    /// kind). While disconnected the request is recorded and applied on
    /// reconnect.
    pub async fn subscribe<S: AsRef<str>>(
        &self,
        kind: MarketDataKind,
        symbols: &[S],
    ) -> Result<()> {
        let symbols = checked_symbols(symbols)?;
        self.request(|tx| Command::Subscribe(kind, symbols, tx))
            .await?
    }

    /// Removes symbols from a market-data subscription.
    pub async fn unsubscribe<S: AsRef<str>>(
        &self,
        kind: MarketDataKind,
        symbols: &[S],
    ) -> Result<()> {
        let symbols = checked_symbols(symbols)?;
        self.request(|tx| Command::Unsubscribe(kind, symbols, tx))
            .await?
    }

    /// Shorthand for `subscribe(MarketDataKind::Quotes, ..)`.
    pub async fn subscribe_quotes<S: AsRef<str>>(&self, symbols: &[S]) -> Result<()> {
        self.subscribe(MarketDataKind::Quotes, symbols).await
    }

    /// Shorthand for `subscribe(MarketDataKind::Depths, ..)`.
    pub async fn subscribe_depths<S: AsRef<str>>(&self, symbols: &[S]) -> Result<()> {
        self.subscribe(MarketDataKind::Depths, symbols).await
    }

    /// Shorthand for `subscribe(MarketDataKind::Trades, ..)`.
    pub async fn subscribe_trades<S: AsRef<str>>(&self, symbols: &[S]) -> Result<()> {
        self.subscribe(MarketDataKind::Trades, symbols).await
    }

    /// Subscribes to a bar indicator. Requires a live connection
    /// ([`Error::StreamNotConnected`] otherwise). The subscription is
    /// re-created automatically after reconnects, under a new indicator id
    /// announced by [`StreamEvent::IndicatorResubscribed`].
    pub async fn subscribe_bars(
        &self,
        kind: BarKind,
        request: BarSubscription,
    ) -> Result<IndicatorSubscription> {
        validate_symbol(&request.symbol)?;
        self.request(|tx| Command::AddBars(kind, request, tx))
            .await?
    }

    /// Removes an indicator subscription.
    pub async fn unsubscribe_indicator(&self, key: IndicatorKey) -> Result<()> {
        self.request(|tx| Command::RemoveIndicator(key, tx)).await?
    }

    /// The subscriptions the stream currently maintains.
    pub async fn subscriptions(&self) -> Result<Subscriptions> {
        self.request(Command::Snapshot).await
    }

    /// Whether the socket is currently open with subscriptions in place.
    pub fn is_connected(&self) -> bool {
        self.shared.connected.load(Ordering::Acquire)
    }

    /// Stops the background task and closes the socket. Idempotent. The
    /// stream's `next()` returns `None` afterwards.
    pub fn shutdown(&self) {
        self.shared.shutdown.send_replace(true);
    }
}
