//! The background task behind a [`Stream`](super::Stream).

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::Duration;

use futures_util::StreamExt;
use tokio::net::TcpStream;
use tokio::sync::{mpsc, watch};
use tokio::time::Instant;
use tokio_tungstenite::tungstenite::{self, Message};
use tokio_tungstenite::{Connector, MaybeTlsStream, WebSocketStream};

use super::{
    Command, DisconnectReason, IndicatorKey, IndicatorSubscription, MarketDataKind, Shared,
    StreamConfig, StreamEvent, Subscriptions,
};
use crate::client::Client;
use crate::config::backoff_delay;
use crate::endpoints;
use crate::error::{Error, Result, truncate_body};
use crate::models::{BarSubscription, StreamMessage};
use crate::rest::{BarKind, MAX_SYMBOLS};

type Socket = WebSocketStream<MaybeTlsStream<TcpStream>>;

/// After this many consecutive frames the command queue is polled even if
/// more frames are ready, so a firehose cannot starve control requests.
const COMMAND_POLL_INTERVAL: u32 = 64;

/// Creates a stream id and opens its WebSocket.
pub(super) async fn open(client: &Client, config: &StreamConfig) -> Result<(Socket, String)> {
    let stream_id = client.create_stream_id().await?;
    let token = client.token().ok_or(Error::MissingCredentials)?;
    let mut url = client.url_for(&endpoints::STREAM_SOCKET, &[("streamId", &stream_id)])?;
    let scheme = if url.scheme() == "https" { "wss" } else { "ws" };
    url.set_scheme(scheme)
        .map_err(|()| Error::Config("cannot derive WebSocket URL".into()))?;
    // The token travels in the query string, as the API requires. The URL is
    // therefore never logged or put into an error.
    url.query_pairs_mut().append_pair("token", &token);

    let connector = Connector::Rustls(client.tls_config());
    let connect =
        tokio_tungstenite::connect_async_tls_with_config(url.as_str(), None, true, Some(connector));
    match tokio::time::timeout(config.connect_timeout, connect).await {
        Err(_) => Err(Error::WebSocket(Box::new(tungstenite::Error::Io(
            std::io::Error::new(
                std::io::ErrorKind::TimedOut,
                "WebSocket handshake timed out",
            ),
        )))),
        Ok(Err(e)) => {
            if let tungstenite::Error::Http(response) = &e {
                if response.status().as_u16() == 401 {
                    // The token was refused: forget it so that the next
                    // attempt authenticates afresh (or fails for good when
                    // there are no credentials).
                    if client.token().is_some_and(|t| *t == *token) {
                        client.set_token(None);
                    }
                }
            }
            Err(Error::WebSocket(Box::new(e)))
        }
        Ok(Ok((socket, _response))) => Ok((socket, stream_id)),
    }
}

/// Errors after which reconnecting cannot help.
fn is_fatal(error: &Error) -> bool {
    matches!(error, Error::MissingCredentials | Error::Config(_)) || error.is_auth()
}

/// A server answer that will not change by trying again on a new connection.
fn is_definitive_refusal(error: &Error) -> bool {
    match error {
        Error::Api { status, .. } => {
            (400..500).contains(status) && !matches!(status, 401 | 403 | 408)
        }
        Error::InvalidRequest(_) => true,
        _ => false,
    }
}

struct Indicator {
    kind: BarKind,
    request: BarSubscription,
    current: IndicatorSubscription,
}

#[derive(Default)]
struct Desired {
    quotes: BTreeSet<String>,
    depths: BTreeSet<String>,
    trades: BTreeSet<String>,
    indicators: BTreeMap<u64, Indicator>,
    next_key: u64,
}

impl Desired {
    fn set(&mut self, kind: MarketDataKind) -> &mut BTreeSet<String> {
        match kind {
            MarketDataKind::Quotes => &mut self.quotes,
            MarketDataKind::Depths => &mut self.depths,
            MarketDataKind::Trades => &mut self.trades,
        }
    }

    fn snapshot(&self) -> Subscriptions {
        Subscriptions {
            quotes: self.quotes.clone(),
            depths: self.depths.clone(),
            trades: self.trades.clone(),
            indicators: self
                .indicators
                .values()
                .map(|i| i.current.clone())
                .collect(),
        }
    }
}

enum SessionEnd {
    /// Nobody is listening any more.
    ConsumerGone,
    Disconnected(DisconnectReason),
}

pub(super) struct Task {
    client: Client,
    config: StreamConfig,
    events: mpsc::Sender<StreamEvent>,
    commands: mpsc::Receiver<Command>,
    shared: Arc<Shared>,
    desired: Desired,
    generation: u64,
    /// Stream id of the live connection, `None` while disconnected.
    stream_id: Option<String>,
}

impl Task {
    pub(super) fn new(
        client: Client,
        config: StreamConfig,
        events: mpsc::Sender<StreamEvent>,
        commands: mpsc::Receiver<Command>,
        shared: Arc<Shared>,
    ) -> Self {
        Self {
            client,
            config,
            events,
            commands,
            shared,
            desired: Desired::default(),
            generation: 0,
            stream_id: None,
        }
    }

    /// Runs until shut down, abandoned by the consumer, or failed for good.
    pub(super) async fn run(
        mut self,
        first: (Socket, String),
        mut shutdown: watch::Receiver<bool>,
    ) {
        let events = self.events.clone();
        let shared = self.shared.clone();
        tokio::select! {
            _ = self.supervise(first) => {}
            _ = shutdown.wait_for(|stop| *stop) => tracing::debug!("stream shut down"),
            _ = events.closed() => tracing::debug!("stream consumer dropped"),
        }
        shared.connected.store(false, Ordering::Release);
        // Dropping `self` closes the socket and both channels.
    }

    async fn supervise(&mut self, first: (Socket, String)) {
        let mut connection = Some(first);
        let mut failures = 0u32;
        let mut reason = DisconnectReason::ConnectFailed(String::new());
        loop {
            if let Some((socket, stream_id)) = connection.take() {
                self.generation += 1;
                let started = Instant::now();
                let end = self.session(socket, stream_id).await;
                self.stream_id = None;
                self.shared.connected.store(false, Ordering::Release);
                match end {
                    SessionEnd::ConsumerGone => return,
                    SessionEnd::Disconnected(r) => reason = r,
                }
                if started.elapsed() >= self.config.stable_after {
                    failures = 0;
                }
            }

            failures += 1;
            if self
                .config
                .max_reconnect_attempts
                .is_some_and(|max| failures > max)
            {
                let error = format!(
                    "giving up after {} failed attempts; last: {reason:?}",
                    failures - 1
                );
                self.emit(StreamEvent::Closed { error }).await;
                return;
            }
            let retry_in = backoff_delay(
                self.config.reconnect_base_delay,
                self.config.reconnect_max_delay,
                failures,
                self.config.reconnect_jitter,
            );
            tracing::warn!(
                ?reason,
                ?retry_in,
                attempt = failures,
                "stream disconnected"
            );
            if !self
                .emit(StreamEvent::Disconnected {
                    reason: reason.clone(),
                    retry_in,
                })
                .await
            {
                return;
            }
            if !self.wait_offline(retry_in).await {
                return;
            }
            match open(&self.client, &self.config).await {
                Ok(c) => connection = Some(c),
                Err(e) if is_fatal(&e) => {
                    self.emit(StreamEvent::Closed {
                        error: e.to_string(),
                    })
                    .await;
                    return;
                }
                Err(e) => reason = DisconnectReason::ConnectFailed(e.to_string()),
            }
        }
    }

    /// Sleeps for the backoff delay while still answering commands. Returns
    /// `false` when every handle is gone.
    async fn wait_offline(&mut self, delay: Duration) -> bool {
        let sleep = tokio::time::sleep(delay);
        tokio::pin!(sleep);
        loop {
            tokio::select! {
                _ = &mut sleep => return true,
                command = self.commands.recv() => match command {
                    Some(c) => self.handle_command(c).await,
                    None => return false,
                },
            }
        }
    }

    /// Delivers an event, applying back-pressure when the consumer is slow
    /// but never ceasing to serve commands (which would deadlock a consumer
    /// that awaits a handle call between two `next()` calls).
    async fn emit(&mut self, event: StreamEvent) -> bool {
        let event = match self.events.try_send(event) {
            Ok(()) => return true,
            Err(mpsc::error::TrySendError::Closed(_)) => return false,
            Err(mpsc::error::TrySendError::Full(event)) => event,
        };
        let events = self.events.clone();
        let mut commands_open = true;
        loop {
            tokio::select! {
                permit = events.reserve() => {
                    return match permit {
                        Ok(permit) => {
                            permit.send(event);
                            true
                        }
                        Err(_) => false,
                    };
                }
                command = self.commands.recv(), if commands_open => match command {
                    Some(c) => self.handle_command(c).await,
                    None => commands_open = false,
                },
            }
        }
    }

    async fn session(&mut self, mut socket: Socket, stream_id: String) -> SessionEnd {
        match self.restore(&stream_id).await {
            Ok(true) => {}
            Ok(false) => return SessionEnd::ConsumerGone,
            Err(e) => {
                return SessionEnd::Disconnected(DisconnectReason::RestoreFailed(e.to_string()));
            }
        }
        self.stream_id = Some(stream_id.clone());
        self.shared.connected.store(true, Ordering::Release);
        tracing::info!(generation = self.generation, "stream connected");
        if !self
            .emit(StreamEvent::Connected {
                generation: self.generation,
                stream_id,
            })
            .await
        {
            return SessionEnd::ConsumerGone;
        }

        let idle_timeout = self.config.idle_timeout;
        let idle = tokio::time::sleep(idle_timeout);
        tokio::pin!(idle);
        let mut burst = 0u32;
        loop {
            tokio::select! {
                biased;
                frame = socket.next() => {
                    idle.as_mut().reset(Instant::now() + idle_timeout);
                    let event = match frame {
                        None => {
                            return SessionEnd::Disconnected(DisconnectReason::ServerClosed {
                                code: None,
                                reason: "connection closed".into(),
                            });
                        }
                        Some(Err(e)) => {
                            return SessionEnd::Disconnected(match e {
                                tungstenite::Error::ConnectionClosed
                                | tungstenite::Error::AlreadyClosed => DisconnectReason::ServerClosed {
                                    code: None,
                                    reason: "connection closed".into(),
                                },
                                other => DisconnectReason::SocketError(other.to_string()),
                            });
                        }
                        Some(Ok(Message::Close(frame))) => {
                            return SessionEnd::Disconnected(DisconnectReason::ServerClosed {
                                code: frame.as_ref().map(|f| u16::from(f.code)),
                                reason: frame.map(|f| f.reason.to_string()).unwrap_or_default(),
                            });
                        }
                        Some(Ok(Message::Text(text))) => decode_frame(text.as_str()),
                        Some(Ok(Message::Binary(bytes))) => match std::str::from_utf8(&bytes) {
                            Ok(text) => decode_frame(text),
                            Err(e) => Some(StreamEvent::Undecodable {
                                error: format!("binary frame is not UTF-8: {e}"),
                                raw: String::new(),
                            }),
                        },
                        // WebSocket-level pings are answered by tungstenite.
                        Some(Ok(_)) => None,
                    };
                    if let Some(event) = event {
                        if !self.emit(event).await {
                            return SessionEnd::ConsumerGone;
                        }
                    }
                    burst += 1;
                    if burst >= COMMAND_POLL_INTERVAL {
                        burst = 0;
                        if let Ok(command) = self.commands.try_recv() {
                            self.handle_command(command).await;
                        }
                    }
                }
                command = self.commands.recv() => {
                    burst = 0;
                    match command {
                        Some(c) => self.handle_command(c).await,
                        None => return SessionEnd::ConsumerGone,
                    }
                }
                _ = &mut idle => return SessionEnd::Disconnected(DisconnectReason::IdleTimeout),
            }
        }
    }

    async fn call_symbols(
        &self,
        kind: MarketDataKind,
        subscribe: bool,
        stream_id: &str,
        symbols: &[String],
    ) -> Result<()> {
        let c = &self.client;
        match (kind, subscribe) {
            (MarketDataKind::Quotes, true) => c.subscribe_quotes(stream_id, symbols).await,
            (MarketDataKind::Depths, true) => c.subscribe_depths(stream_id, symbols).await,
            (MarketDataKind::Trades, true) => c.subscribe_trades(stream_id, symbols).await,
            (MarketDataKind::Quotes, false) => c.unsubscribe_quotes(stream_id, symbols).await,
            (MarketDataKind::Depths, false) => c.unsubscribe_depths(stream_id, symbols).await,
            (MarketDataKind::Trades, false) => c.unsubscribe_trades(stream_id, symbols).await,
        }
        .map(|_| ())
    }

    async fn create_indicator(
        &self,
        stream_id: &str,
        key: u64,
        kind: BarKind,
        request: &BarSubscription,
    ) -> Result<IndicatorSubscription> {
        let response = self.client.subscribe_bars(stream_id, kind, request).await?;
        let indicator_id = response
            .indicator_id
            .filter(|s| !s.is_empty())
            .ok_or_else(|| Error::Api {
                status: 200,
                message: Some("indicator subscription response contained no indicatorId".into()),
                body: String::new(),
            })?;
        Ok(IndicatorSubscription {
            key: IndicatorKey(key),
            kind,
            request: request.clone(),
            indicator_id,
            value_names: response.value_names,
            value_types: response.value_types,
        })
    }

    /// Re-creates all desired subscriptions on a fresh stream.
    ///
    /// * `Ok(true)` — everything that can be restored is in place.
    /// * `Ok(false)` — the consumer went away.
    /// * `Err(_)` — a transient failure; the caller drops this connection and
    ///   tries again, rather than run with silently missing data.
    async fn restore(&mut self, stream_id: &str) -> Result<bool> {
        for kind in [
            MarketDataKind::Quotes,
            MarketDataKind::Depths,
            MarketDataKind::Trades,
        ] {
            let symbols: Vec<String> = self.desired.set(kind).iter().cloned().collect();
            if symbols.is_empty() {
                continue;
            }
            match self.call_symbols(kind, true, stream_id, &symbols).await {
                Ok(()) => {}
                Err(e) if is_definitive_refusal(&e) => {
                    self.desired.set(kind).clear();
                    let event = StreamEvent::SubscriptionLost {
                        what: format!("{kind:?} {symbols:?}"),
                        error: e.to_string(),
                    };
                    if !self.emit(event).await {
                        return Ok(false);
                    }
                }
                Err(e) => return Err(e),
            }
        }
        let keys: Vec<u64> = self.desired.indicators.keys().copied().collect();
        for key in keys {
            let Some(entry) = self.desired.indicators.get(&key) else {
                continue;
            };
            let (kind, request) = (entry.kind, entry.request.clone());
            let previous_id = entry.current.indicator_id.clone();
            let event = match self.create_indicator(stream_id, key, kind, &request).await {
                Ok(subscription) => {
                    if let Some(entry) = self.desired.indicators.get_mut(&key) {
                        entry.current = subscription.clone();
                    }
                    StreamEvent::IndicatorResubscribed {
                        previous_id,
                        subscription,
                    }
                }
                Err(e) if is_definitive_refusal(&e) => {
                    self.desired.indicators.remove(&key);
                    StreamEvent::SubscriptionLost {
                        what: format!("{kind:?} bars {request:?}"),
                        error: e.to_string(),
                    }
                }
                Err(e) => return Err(e),
            };
            if !self.emit(event).await {
                return Ok(false);
            }
        }
        Ok(true)
    }

    async fn handle_command(&mut self, command: Command) {
        let stream_id = self.stream_id.clone();
        match command {
            Command::Snapshot(reply) => {
                let _ = reply.send(self.desired.snapshot());
            }
            Command::Subscribe(kind, symbols, reply) => {
                let _ = reply.send(self.subscribe(kind, symbols, stream_id.as_deref()).await);
            }
            Command::Unsubscribe(kind, symbols, reply) => {
                let _ = reply.send(self.unsubscribe(kind, symbols, stream_id.as_deref()).await);
            }
            Command::AddBars(kind, request, reply) => {
                let result = match stream_id.as_deref() {
                    None => Err(Error::StreamNotConnected),
                    Some(id) => {
                        let key = self.desired.next_key;
                        self.desired.next_key += 1;
                        let created = self.create_indicator(id, key, kind, &request).await;
                        if let Ok(subscription) = &created {
                            let entry = Indicator {
                                kind,
                                request,
                                current: subscription.clone(),
                            };
                            self.desired.indicators.insert(key, entry);
                        }
                        created
                    }
                };
                let _ = reply.send(result);
            }
            Command::RemoveIndicator(key, reply) => {
                let result = match self.desired.indicators.get(&key.0) {
                    None => Err(Error::InvalidRequest(
                        "unknown indicator subscription".into(),
                    )),
                    Some(entry) => {
                        let outcome = match stream_id.as_deref() {
                            Some(id) => self
                                .client
                                .unsubscribe_indicator(id, &entry.current.indicator_id)
                                .await
                                .map(|_| ()),
                            // Offline: the server-side indicator died with
                            // the old stream.
                            None => Ok(()),
                        };
                        match outcome {
                            Err(e) if !is_definitive_refusal(&e) => Err(e),
                            // Removed, or the server no longer knows it.
                            _ => {
                                self.desired.indicators.remove(&key.0);
                                Ok(())
                            }
                        }
                    }
                };
                let _ = reply.send(result);
            }
        }
    }

    async fn subscribe(
        &mut self,
        kind: MarketDataKind,
        symbols: Vec<String>,
        stream_id: Option<&str>,
    ) -> Result<()> {
        let mut wanted = self.desired.set(kind).clone();
        let before = wanted.len();
        wanted.extend(symbols);
        if wanted.len() > MAX_SYMBOLS {
            return Err(Error::InvalidRequest(format!(
                "at most {MAX_SYMBOLS} symbols per stream and kind; this would make {}",
                wanted.len()
            )));
        }
        if wanted.len() == before {
            return Ok(());
        }
        if let Some(id) = stream_id {
            // The API does not say whether subscribing replaces or extends
            // the stream's symbol list; sending the complete desired list is
            // correct under both readings.
            let all: Vec<String> = wanted.iter().cloned().collect();
            self.call_symbols(kind, true, id, &all).await?;
        }
        *self.desired.set(kind) = wanted;
        Ok(())
    }

    async fn unsubscribe(
        &mut self,
        kind: MarketDataKind,
        symbols: Vec<String>,
        stream_id: Option<&str>,
    ) -> Result<()> {
        let current = self.desired.set(kind);
        let removed: Vec<String> = symbols
            .into_iter()
            .filter(|s| current.contains(s))
            .collect();
        if removed.is_empty() {
            return Ok(());
        }
        if let Some(id) = stream_id {
            self.call_symbols(kind, false, id, &removed).await?;
        }
        let current = self.desired.set(kind);
        for symbol in &removed {
            current.remove(symbol);
        }
        Ok(())
    }
}

fn decode_frame(text: &str) -> Option<StreamEvent> {
    match serde_json::from_str::<StreamMessage>(text) {
        Ok(message) if message.is_empty_ignoring_ping() => None,
        Ok(message) => Some(StreamEvent::Data(Box::new(message))),
        Err(e) => Some(StreamEvent::Undecodable {
            error: e.to_string(),
            raw: truncate_body(text.to_owned()),
        }),
    }
}
