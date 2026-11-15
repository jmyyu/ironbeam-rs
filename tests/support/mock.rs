//! A stateful in-process imitation of the Ironbeam API (REST + WebSocket on
//! one port) with fault injection.
//!
//! It implements the subset of behaviour the tests exercise, following the
//! published specification's paths, methods and payload shapes. It is *not*
//! evidence of how the real service behaves.

use std::collections::{BTreeSet, HashMap, HashSet, VecDeque};
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::Router;
use axum::body::{Body, Bytes};
use axum::extract::ws::{CloseFrame, Message, WebSocket, WebSocketUpgrade};
use axum::extract::{Path, Query, Request, State};
use axum::http::{HeaderMap, StatusCode, header};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Json, Response};
use axum::routing::{delete, get, post, put};
use serde_json::{Value, json};
use tokio::sync::mpsc;

use ironbeam_sdk::{Client, ClientConfig, Credentials, Environment, RetryPolicy};

pub const USER: &str = "tester";
pub const API_KEY: &str = "good-key";
pub const ACCOUNT: &str = "ACC1";

/// An injected failure, consumed by the next matching request.
#[derive(Debug, Clone)]
pub enum Fault {
    /// Answer with this status and JSON body; the request is NOT processed.
    Status(u16, Value),
    /// Like `Status`, with a `Retry-After` header (seconds).
    RetryAfter(u16, u64),
    /// Process the request normally, then delay the response.
    SlowResponse(Duration),
    /// Process the request normally, then break the connection while sending
    /// the response body.
    AbortBody,
    /// Process the request normally but answer 200 with this raw body.
    Garbage(&'static str),
}

#[derive(Debug, Clone)]
pub struct Recorded {
    pub method: String,
    pub path: String,
    pub query: String,
    pub bearer: Option<String>,
    pub body: Option<Value>,
}

pub enum WsCmd {
    Send(String),
    Close(u16, String),
    Drop,
    Silence,
}

#[derive(Default)]
pub struct StreamState {
    pub tx: Option<mpsc::UnboundedSender<WsCmd>>,
    pub opened: bool,
    pub connected: bool,
    pub quotes: BTreeSet<String>,
    pub depths: BTreeSet<String>,
    pub trades: BTreeSet<String>,
    pub indicators: Vec<(String, String, Value)>,
}

#[derive(Default)]
struct Inner {
    valid_tokens: HashSet<String>,
    token_counter: u32,
    auth_count: u32,
    requests: Vec<Recorded>,
    faults: HashMap<&'static str, VecDeque<Fault>>,
    orders: Vec<Value>,
    order_counter: u64,
    streams: HashMap<String, StreamState>,
    stream_order: Vec<String>,
    indicator_counter: u64,
    ws_rejections: VecDeque<u16>,
    ping_interval: Option<Duration>,
    subscribe_replaces: bool,
}

#[derive(Clone)]
pub struct Mock {
    pub addr: SocketAddr,
    inner: Arc<Mutex<Inner>>,
}

type St = State<Mock>;

fn err(status: u16, message: &str) -> Response {
    (
        StatusCode::from_u16(status).unwrap(),
        Json(json!({"status": "ERROR", "message": message, "error": message})),
    )
        .into_response()
}

impl Mock {
    pub async fn start() -> Mock {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let inner = Inner {
            ping_interval: Some(Duration::from_millis(40)),
            ..Inner::default()
        };
        let mock = Mock {
            addr,
            inner: Arc::new(Mutex::new(inner)),
        };
        let api = Router::new()
            .route("/auth", post(auth))
            .route("/logout", post(logout))
            .route("/account/getAllAccounts", get(all_accounts))
            .route("/order/{account}/place", post(place_order))
            .route("/order/{account}/toorderid/{sid}", get(to_order_id))
            .route("/order/{account}/update/{id}", put(update_order))
            .route("/order/{account}/cancel/{id}", delete(cancel_order))
            .route("/order/{account}/cancelMultiple", delete(cancel_multiple))
            .route("/order/{account}/{status}", get(get_orders))
            .route("/stream/create", get(stream_create))
            .route("/stream/{id}", get(stream_socket))
            .route("/market/{kind}/{action}/{id}", get(market_subscription))
            .route(
                "/indicator/{id}/{kind}/subscribe",
                post(indicator_subscribe),
            )
            .route(
                "/indicator/{id}/unsubscribe/{indicator}",
                delete(indicator_unsubscribe),
            )
            .layer(middleware::from_fn_with_state(mock.clone(), record))
            .with_state(mock.clone());
        let app = Router::new().nest("/v2", api);
        tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        mock
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(|e| e.into_inner())
    }

    pub fn base_url(&self) -> String {
        format!("http://{}/v2", self.addr)
    }

    /// Configuration with short timeouts and fast, jitter-free retries.
    pub fn config(&self) -> ClientConfig {
        ClientConfig {
            environment: Environment::custom(&self.base_url()).unwrap(),
            request_timeout: Duration::from_millis(1500),
            order_timeout: Duration::from_millis(400),
            connect_timeout: Duration::from_millis(1000),
            retry: RetryPolicy {
                max_retries: 3,
                base_delay: Duration::from_millis(10),
                max_delay: Duration::from_millis(50),
                jitter: false,
            },
            ..ClientConfig::default()
        }
    }

    pub fn client(&self) -> Client {
        self.client_with(self.config())
    }

    pub fn client_with(&self, config: ClientConfig) -> Client {
        Client::builder()
            .config(config)
            .credentials(Credentials::api_key(USER, API_KEY))
            .build()
            .unwrap()
    }

    // ---- inspection -----------------------------------------------------

    pub fn requests(&self, method: &str, path_contains: &str) -> Vec<Recorded> {
        self.lock()
            .requests
            .iter()
            .filter(|r| r.method == method && r.path.contains(path_contains))
            .cloned()
            .collect()
    }

    pub fn count(&self, method: &str, path_contains: &str) -> usize {
        self.requests(method, path_contains).len()
    }

    pub fn auth_count(&self) -> u32 {
        self.lock().auth_count
    }

    pub fn orders(&self) -> Vec<Value> {
        self.lock().orders.clone()
    }

    pub fn stream_ids(&self) -> Vec<String> {
        self.lock().stream_order.clone()
    }

    pub fn connected_streams(&self) -> Vec<String> {
        let inner = self.lock();
        inner
            .stream_order
            .iter()
            .filter(|id| inner.streams[*id].connected)
            .cloned()
            .collect()
    }

    /// (quotes, depths, trades, indicator ids) subscribed on a stream.
    pub fn subscriptions(&self, id: &str) -> (Vec<String>, Vec<String>, Vec<String>, Vec<String>) {
        let inner = self.lock();
        let s = &inner.streams[id];
        (
            s.quotes.iter().cloned().collect(),
            s.depths.iter().cloned().collect(),
            s.trades.iter().cloned().collect(),
            s.indicators.iter().map(|(id, _, _)| id.clone()).collect(),
        )
    }

    // ---- control --------------------------------------------------------

    pub fn fault(&self, op: &'static str, fault: Fault) {
        self.lock().faults.entry(op).or_default().push_back(fault);
    }

    pub fn expire_tokens(&self) {
        self.lock().valid_tokens.clear();
    }

    pub fn set_ping_interval(&self, interval: Option<Duration>) {
        self.lock().ping_interval = interval;
    }

    pub fn set_subscribe_replaces(&self, replaces: bool) {
        self.lock().subscribe_replaces = replaces;
    }

    pub fn reject_ws(&self, status: u16) {
        self.lock().ws_rejections.push_back(status);
    }

    /// Adds an order directly (as if placed by another session). Returns its id.
    pub fn add_order(&self, overrides: Value) -> String {
        let mut inner = self.lock();
        inner.order_counter += 1;
        let n = inner.order_counter;
        let mut order = json!({
            "orderId": format!("{ACCOUNT}-{n}"), "strategyId": 1000 + n, "accountId": ACCOUNT,
            "exchSym": "XCME:ES.Z26", "status": "NEW", "side": "BUY", "quantity": 1.0,
            "orderType": "2", "duration": "0", "limitPrice": 5000.0, "fillQuantity": 0.0
        });
        for (k, v) in overrides.as_object().unwrap() {
            order[k] = v.clone();
        }
        let id = order["orderId"].as_str().unwrap().to_owned();
        inner.orders.push(order);
        id
    }

    pub fn patch_order(&self, id: &str, patch: Value) {
        let mut inner = self.lock();
        let order = inner
            .orders
            .iter_mut()
            .find(|o| o["orderId"] == id)
            .expect("order exists");
        for (k, v) in patch.as_object().unwrap() {
            order[k] = v.clone();
        }
    }

    pub fn remove_order(&self, id: &str) {
        self.lock().orders.retain(|o| o["orderId"] != id);
    }

    fn ws(&self, id: &str, cmd: WsCmd) {
        let inner = self.lock();
        let tx = inner.streams[id].tx.as_ref().expect("stream has a socket");
        let _ = tx.send(cmd);
    }

    pub fn ws_send(&self, id: &str, message: Value) {
        self.ws(id, WsCmd::Send(message.to_string()));
    }

    pub fn ws_send_raw(&self, id: &str, text: &str) {
        self.ws(id, WsCmd::Send(text.to_owned()));
    }

    /// Kills the TCP connection without a close handshake.
    pub fn ws_drop(&self, id: &str) {
        self.ws(id, WsCmd::Drop);
    }

    pub fn ws_close(&self, id: &str, code: u16, reason: &str) {
        self.ws(id, WsCmd::Close(code, reason.to_owned()));
    }

    /// Stops pings on one connection (simulates a half-dead link).
    pub fn ws_silence(&self, id: &str) {
        self.ws(id, WsCmd::Silence);
    }

    // ---- internals ------------------------------------------------------

    fn next_fault(&self, op: &'static str) -> Option<Fault> {
        self.lock().faults.get_mut(op).and_then(VecDeque::pop_front)
    }

    fn authorized(&self, headers: &HeaderMap) -> bool {
        let token = headers
            .get(header::AUTHORIZATION)
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.strip_prefix("Bearer "));
        token.is_some_and(|t| self.lock().valid_tokens.contains(t))
    }

    /// Applies injected faults around `handler`, after the bearer check.
    async fn run(
        &self,
        op: &'static str,
        headers: &HeaderMap,
        handler: impl FnOnce(&mut Inner) -> Response,
    ) -> Response {
        if !self.authorized(headers) {
            return err(401, "Unauthorized");
        }
        self.run_unauthenticated(op, handler).await
    }

    async fn run_unauthenticated(
        &self,
        op: &'static str,
        handler: impl FnOnce(&mut Inner) -> Response,
    ) -> Response {
        match self.next_fault(op) {
            None => handler(&mut self.lock()),
            Some(Fault::Status(code, body)) => {
                (StatusCode::from_u16(code).unwrap(), Json(body)).into_response()
            }
            Some(Fault::RetryAfter(code, secs)) => (
                StatusCode::from_u16(code).unwrap(),
                [(header::RETRY_AFTER, secs.to_string())],
                Json(json!({"error": "slow down"})),
            )
                .into_response(),
            Some(Fault::SlowResponse(delay)) => {
                let response = handler(&mut self.lock());
                tokio::time::sleep(delay).await;
                response
            }
            Some(Fault::AbortBody) => {
                let _ = handler(&mut self.lock());
                let broken = futures_util::stream::once(async {
                    Err::<Bytes, std::io::Error>(std::io::Error::other("injected abort"))
                });
                Response::builder()
                    .status(200)
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from_stream(broken))
                    .unwrap()
            }
            Some(Fault::Garbage(text)) => {
                let _ = handler(&mut self.lock());
                (StatusCode::OK, text).into_response()
            }
        }
    }
}

async fn record(State(mock): St, request: Request, next: Next) -> Response {
    let (parts, body) = request.into_parts();
    let bytes = axum::body::to_bytes(body, usize::MAX)
        .await
        .unwrap_or_default();
    let recorded = Recorded {
        method: parts.method.to_string(),
        path: parts.uri.path().to_owned(),
        query: parts.uri.query().unwrap_or("").to_owned(),
        bearer: parts
            .headers
            .get(header::AUTHORIZATION)
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.strip_prefix("Bearer "))
            .map(str::to_owned),
        body: serde_json::from_slice(&bytes).ok(),
    };
    mock.lock().requests.push(recorded);
    next.run(Request::from_parts(parts, Body::from(bytes)))
        .await
}

async fn auth(State(mock): St, Json(body): Json<Value>) -> Response {
    mock.run_unauthenticated("authorize", |inner| {
        inner.auth_count += 1;
        let user_ok = body["username"] == USER;
        let secret_ok = body["apikey"] == API_KEY || body["password"] == "good-pass";
        if !(user_ok && secret_ok) {
            return err(401, "invalid username or password or apiKey");
        }
        inner.token_counter += 1;
        let token = format!("tok-{}", inner.token_counter);
        inner.valid_tokens.insert(token.clone());
        Json(json!({"token": token})).into_response()
    })
    .await
}

async fn logout(State(mock): St, headers: HeaderMap) -> Response {
    let token = headers
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .map(str::to_owned);
    mock.run("logout", &headers, |inner| {
        if let Some(t) = token {
            inner.valid_tokens.remove(&t);
        }
        Json(json!({"status": "OK", "message": "bye"})).into_response()
    })
    .await
}

async fn all_accounts(State(mock): St, headers: HeaderMap) -> Response {
    mock.run("allAccounts", &headers, |_| {
        Json(json!({"accounts": [ACCOUNT]})).into_response()
    })
    .await
}

async fn get_orders(
    State(mock): St,
    headers: HeaderMap,
    Path((account, status)): Path<(String, String)>,
) -> Response {
    mock.run("getOrders", &headers, |inner| {
        let orders: Vec<Value> = inner
            .orders
            .iter()
            .filter(|o| o["accountId"] == account.as_str())
            .filter(|o| status == "ANY" || o["status"] == status.as_str())
            .cloned()
            .collect();
        Json(json!({"orders": orders})).into_response()
    })
    .await
}

async fn place_order(
    State(mock): St,
    headers: HeaderMap,
    Path(account): Path<String>,
    Json(body): Json<Value>,
) -> Response {
    mock.run("placeOrder", &headers, |inner| {
        if body["quantity"].as_f64().is_none_or(|q| q <= 0.0) || body["exchSym"].is_null() {
            return err(400, "malformed order");
        }
        inner.order_counter += 1;
        let n = inner.order_counter;
        let order_id = format!("{account}-{n}");
        let strategy_id = 1000 + n;
        inner.orders.push(json!({
            "orderId": order_id, "strategyId": strategy_id, "accountId": account,
            "exchSym": body["exchSym"], "status": "NEW", "side": body["side"],
            "quantity": body["quantity"], "orderType": body["orderType"],
            "duration": body["duration"], "limitPrice": body["limitPrice"],
            "stopPrice": body["stopPrice"], "fillQuantity": 0.0
        }));
        if body["waitForOrderId"] == false {
            Json(json!({"strategyId": strategy_id})).into_response()
        } else {
            Json(json!({"orderId": order_id, "strategyId": strategy_id})).into_response()
        }
    })
    .await
}

async fn to_order_id(
    State(mock): St,
    headers: HeaderMap,
    Path((_account, sid)): Path<(String, u64)>,
) -> Response {
    mock.run("getToOrderId", &headers, |inner| {
        match inner.orders.iter().find(|o| o["strategyId"] == sid) {
            Some(o) => Json(json!({"orderId": o["orderId"], "strategyId": sid})).into_response(),
            None => err(404, "unknown strategy id"),
        }
    })
    .await
}

async fn update_order(
    State(mock): St,
    headers: HeaderMap,
    Path((_account, id)): Path<(String, String)>,
    Json(body): Json<Value>,
) -> Response {
    mock.run("updateOrder", &headers, |inner| {
        let Some(order) = inner
            .orders
            .iter_mut()
            .find(|o| o["orderId"] == id.as_str())
        else {
            return err(400, "order not found");
        };
        order["quantity"] = json!(body["quantity"].as_f64().unwrap_or(0.0));
        for key in ["limitPrice", "stopPrice"] {
            if !body[key].is_null() {
                order[key] = body[key].clone();
            }
        }
        order["status"] = json!("REPLACED");
        Json(json!({"orders": [order.clone()]})).into_response()
    })
    .await
}

fn cancel(inner: &mut Inner, id: &str) -> Option<Value> {
    let order = inner.orders.iter_mut().find(|o| o["orderId"] == id)?;
    if order["status"] == "NEW"
        || order["status"] == "PARTIALLY_FILLED"
        || order["status"] == "REPLACED"
    {
        order["status"] = json!("CANCELLED");
        Some(order.clone())
    } else {
        None
    }
}

async fn cancel_order(
    State(mock): St,
    headers: HeaderMap,
    Path((_account, id)): Path<(String, String)>,
) -> Response {
    mock.run("cancelOrder", &headers, |inner| match cancel(inner, &id) {
        Some(order) => Json(json!({"orders": [order]})).into_response(),
        None => err(400, "order not found or not cancellable"),
    })
    .await
}

async fn cancel_multiple(
    State(mock): St,
    headers: HeaderMap,
    Path(_account): Path<String>,
    Json(body): Json<Value>,
) -> Response {
    mock.run("cancelMultipleOrders", &headers, |inner| {
        let ids: Vec<String> = body["orderIds"]
            .as_array()
            .map(|a| {
                a.iter()
                    .filter_map(|v| v.as_str().map(str::to_owned))
                    .collect()
            })
            .unwrap_or_default();
        let cancelled: Vec<Value> = ids.iter().filter_map(|id| cancel(inner, id)).collect();
        Json(json!({"orders": cancelled})).into_response()
    })
    .await
}

async fn stream_create(State(mock): St, headers: HeaderMap) -> Response {
    mock.run("getStreamId", &headers, |inner| {
        let id = uuid::Uuid::new_v4().to_string();
        inner.streams.insert(id.clone(), StreamState::default());
        inner.stream_order.push(id.clone());
        Json(json!({"streamId": id})).into_response()
    })
    .await
}

async fn stream_socket(
    State(mock): St,
    Path(id): Path<String>,
    Query(query): Query<HashMap<String, String>>,
    upgrade: WebSocketUpgrade,
) -> Response {
    let (tx, rx) = mpsc::unbounded_channel();
    {
        let mut inner = mock.lock();
        if let Some(status) = inner.ws_rejections.pop_front() {
            return err(status, "injected handshake rejection");
        }
        if !query
            .get("token")
            .is_some_and(|t| inner.valid_tokens.contains(t))
        {
            return err(401, "Unauthorized");
        }
        let Some(stream) = inner.streams.get_mut(&id) else {
            return err(400, "unknown stream id");
        };
        if stream.opened {
            return err(400, "stream id already used");
        }
        stream.opened = true;
        stream.connected = true;
        stream.tx = Some(tx);
    }
    upgrade.on_upgrade(move |socket| serve_socket(mock, id, socket, rx))
}

async fn serve_socket(
    mock: Mock,
    id: String,
    mut socket: WebSocket,
    mut rx: mpsc::UnboundedReceiver<WsCmd>,
) {
    let interval = mock.lock().ping_interval;
    let mut pings = interval.is_some();
    let mut ticker = tokio::time::interval(interval.unwrap_or(Duration::from_secs(3600)));
    loop {
        tokio::select! {
            cmd = rx.recv() => match cmd {
                Some(WsCmd::Send(text)) => {
                    if socket.send(Message::Text(text.into())).await.is_err() {
                        break;
                    }
                }
                Some(WsCmd::Close(code, reason)) => {
                    let frame = CloseFrame { code, reason: reason.into() };
                    let _ = socket.send(Message::Close(Some(frame))).await;
                    break;
                }
                Some(WsCmd::Silence) => pings = false,
                Some(WsCmd::Drop) | None => break,
            },
            _ = ticker.tick(), if pings => {
                if socket.send(Message::Text(r#"{"p":{"ping":"ping"}}"#.into())).await.is_err() {
                    break;
                }
            }
            incoming = socket.recv() => {
                if !matches!(incoming, Some(Ok(_))) {
                    break;
                }
            }
        }
    }
    let mut inner = mock.lock();
    if let Some(stream) = inner.streams.get_mut(&id) {
        stream.connected = false;
        stream.tx = None;
    }
}

async fn market_subscription(
    State(mock): St,
    headers: HeaderMap,
    Path((kind, action, id)): Path<(String, String, String)>,
    Query(query): Query<HashMap<String, String>>,
) -> Response {
    let op = match (kind.as_str(), action.as_str()) {
        ("quotes", "subscribe") => "getSubscribeQuotes",
        ("depths", "subscribe") => "getSubscribeDepths",
        ("trades", "subscribe") => "getSubscribeTrades",
        ("quotes", "unsubscribe") => "getUnsubscribeQuotes",
        ("depths", "unsubscribe") => "UnsubscribeDepths",
        ("trades", "unsubscribe") => "getUnsubscribeTrades",
        _ => return err(404, "no such route"),
    };
    mock.run(op, &headers, |inner| {
        let replaces = inner.subscribe_replaces;
        let Some(stream) = inner.streams.get_mut(&id).filter(|s| s.connected) else {
            return err(400, "stream is not open");
        };
        let symbols: Vec<String> = query
            .get("symbols")
            .map(|s| {
                s.split(',')
                    .filter(|x| !x.is_empty())
                    .map(str::to_owned)
                    .collect()
            })
            .unwrap_or_default();
        if symbols.is_empty() || symbols.len() > 10 {
            return err(400, "between 1 and 10 symbols required");
        }
        let set = match kind.as_str() {
            "quotes" => &mut stream.quotes,
            "depths" => &mut stream.depths,
            _ => &mut stream.trades,
        };
        if action == "subscribe" {
            if replaces {
                set.clear();
            }
            set.extend(symbols);
        } else {
            for s in &symbols {
                set.remove(s);
            }
        }
        Json(json!({"status": "OK", "message": "done"})).into_response()
    })
    .await
}

async fn indicator_subscribe(
    State(mock): St,
    headers: HeaderMap,
    Path((id, kind)): Path<(String, String)>,
    Json(body): Json<Value>,
) -> Response {
    mock.run("indicatorSubscribe", &headers, |inner| {
        inner.indicator_counter += 1;
        let n = inner.indicator_counter;
        let Some(stream) = inner.streams.get_mut(&id).filter(|s| s.connected) else {
            return err(400, "stream is not open");
        };
        let indicator_id = format!("{kind}_{n}");
        stream.indicators.push((indicator_id.clone(), kind, body));
        Json(json!({
            "indicatorId": indicator_id,
            "valueNames": ["date", "open", "close", "high", "low", "volume", "tradeCount", "delta"],
            "valueTypes": ["date", "number", "number", "number", "number", "number", "number", "number"]
        }))
        .into_response()
    })
    .await
}

async fn indicator_unsubscribe(
    State(mock): St,
    headers: HeaderMap,
    Path((id, indicator)): Path<(String, String)>,
) -> Response {
    mock.run("indicatorUnsubscribe", &headers, |inner| {
        let Some(stream) = inner.streams.get_mut(&id).filter(|s| s.connected) else {
            return err(400, "stream is not open");
        };
        let before = stream.indicators.len();
        stream.indicators.retain(|(i, _, _)| *i != indicator);
        if stream.indicators.len() == before {
            return err(400, "unknown indicator");
        }
        Json(json!({"status": "OK", "message": "done"})).into_response()
    })
    .await
}

/// A TCP server that reads each request and then closes the connection
/// without answering. Returns its base URL and a counter of accepted
/// connections.
pub async fn spawn_reset_server() -> (String, Arc<std::sync::atomic::AtomicUsize>) {
    use std::sync::atomic::{AtomicUsize, Ordering};
    use tokio::io::AsyncReadExt;
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let hits = Arc::new(AtomicUsize::new(0));
    let counter = hits.clone();
    tokio::spawn(async move {
        loop {
            let Ok((mut socket, _)) = listener.accept().await else {
                break;
            };
            counter.fetch_add(1, Ordering::SeqCst);
            tokio::spawn(async move {
                let mut buf = vec![0u8; 16 * 1024];
                let mut seen = Vec::new();
                // Read until the end of the headers plus a little body time.
                loop {
                    match tokio::time::timeout(Duration::from_millis(100), socket.read(&mut buf))
                        .await
                    {
                        Ok(Ok(n)) if n > 0 => {
                            seen.extend_from_slice(&buf[..n]);
                            if seen.windows(4).any(|w| w == b"\r\n\r\n") && seen.ends_with(b"}") {
                                break;
                            }
                        }
                        _ => break,
                    }
                }
                drop(socket);
            });
        }
    });
    (format!("http://{addr}/v2"), hits)
}
