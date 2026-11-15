# ironbeam-sdk

An unofficial, typed, async Rust SDK for the [Ironbeam](https://www.ironbeam.com/)
futures trading API: every documented REST operation, plus a resilient
WebSocket stream with automatic reconnection, subscription recovery and
order-state reconciliation.

It is written against Ironbeam's published OpenAPI document (version 2.1,
local test setup in [`spec/`](spec/)) and is built around one rule:

> **An order that may have been executed is never sent twice.**

> **Status.** Everything here is verified against the specification and
> against mock servers only. No request has been sent to the real API yet.
> [`REPORT.md`](REPORT.md) separates what is verified from what is assumed, and
> [`docs/SPEC_DISCREPANCIES.md`](docs/SPEC_DISCREPANCIES.md) lists where the
> specification contradicts itself and what the SDK does about it. Read both
> before trusting it with a live account.

## Installing

The crate is not on crates.io. Use it as a path or git dependency:

```toml
[dependencies]
ironbeam-sdk = { path = "../ironbeam-rs" }
tokio = { version = "1", features = ["macros", "rt-multi-thread"] }
```

Rust 1.85 or newer. TLS is rustls with the *ring* provider and bundled Mozilla
roots: no OpenSSL, no cmake, and no dependence on (or change to) rustls'
process-wide default provider.

## Quick start

```rust,no_run
use ironbeam_sdk::{Client, Credentials, Environment};
use ironbeam_sdk::models::BalanceType;

# async fn run() -> ironbeam_sdk::Result<()> {
let client = Client::builder()
    .environment(Environment::Demo) // the default; `Environment::Live` is opt-in
    .credentials(Credentials::api_key("your-username", "your-api-key"))
    .build()?;

// Authentication happens on first use and again whenever a token expires.
for account in client.all_accounts().await?.accounts {
    let balances = client.account_balance(&account, BalanceType::CurrentOpen).await?;
    println!("{account}: {:?}", balances.balances);
}
let quotes = client.quotes(&["XCME:ES.Z26"]).await?;
println!("{:?}", quotes.quotes);
# Ok(()) }
```

## Placing orders safely

A submission has exactly three outcomes, and the type system keeps them apart:

| Result | Meaning | What to do |
|---|---|---|
| `Ok(Submission)` | The API accepted the order. | Track it. |
| `Err(Error::OrderOutcomeUnknown(_))` | The request may have reached the server but no usable answer came back (timeout, connection lost, 5xx, unreadable body). **The order may be live.** | Investigate. Never resubmit blindly. |
| any other `Err` | Definitely not placed: rejected locally, never sent, or refused with a 4xx. | Fix and resubmit if you wish. |

```rust,no_run
use ironbeam_sdk::{Client, Error};
use ironbeam_sdk::models::{OrderRequest, OrderSide, OrderDuration};
use ironbeam_sdk::orders::{Investigation, OrderManager};

# async fn run(client: Client) -> ironbeam_sdk::Result<()> {
let orders = OrderManager::new(client, "5123345");
let request = OrderRequest::limit("XCME:ES.Z26", OrderSide::Buy, 1.0, 5000.25)
    .duration(OrderDuration::Day)
    .stop_loss(4990.0)      // optional bracket legs
    .take_profit(5020.0);

match orders.submit(request).await {
    Ok(accepted) => println!("accepted: {accepted:?}"),
    Err(Error::OrderOutcomeUnknown(unknown)) => match orders.investigate(&unknown).await? {
        Investigation::Found(order) => println!("it went through as {}", order.order_id),
        Investigation::NotFound => println!("not visible (yet); look again before concluding"),
        Investigation::Ambiguous(candidates) => println!("{} look-alikes; review", candidates.len()),
    },
    Err(e) => println!("not placed: {e}"),
}
# Ok(()) }
```

What the SDK does and does not do on your behalf:

* **Never retried:** placing, modifying and cancelling orders, and every other
  mutating call. Not on timeouts, not on 5xx, not on 429.
* **Retried with backoff:** reads (`GET`) and `/auth`, on transport errors,
  429 (honouring `Retry-After`) and 5xx.
* **Resent once after a 401:** any call, including orders. A 401 means the
  server refused to process the request, so replaying it with a fresh token
  cannot duplicate anything.
* **Investigation is read-only.** The API documents no client order id, so an
  unknown outcome is matched to orders that *appeared after the submission*
  and agree with it in symbol, side, quantity, type and prices, excluding
  orders already attributed to other submissions. One match is `Found`;
  several are reported as `Ambiguous` rather than guessed at.

## Streaming

```rust,no_run
use ironbeam_sdk::Client;
use ironbeam_sdk::stream::{Stream, StreamConfig, StreamEvent};

# async fn run(client: Client) -> ironbeam_sdk::Result<()> {
let mut stream = Stream::connect(&client, StreamConfig::default()).await?;
let handle = stream.handle();
handle.subscribe_quotes(&["XCME:ES.Z26", "XCME:NQ.Z26"]).await?;

while let Some(event) = stream.next().await {
    match event {
        StreamEvent::Data(message) => {
            for quote in &message.quotes {
                println!("{:?} {:?}/{:?}", quote.symbol, quote.bid, quote.ask);
            }
        }
        StreamEvent::Connected { generation, .. } => println!("connected (#{generation})"),
        StreamEvent::Disconnected { reason, retry_in } => println!("{reason:?}, retry in {retry_in:?}"),
        StreamEvent::Closed { error } => { println!("gave up: {error}"); break; }
        _ => {}
    }
}
# Ok(()) }
```

The background task

* declares the connection dead after `idle_timeout` of silence (the server
  pings every 5 s) and reconnects with exponential backoff and jitter;
* creates a **new stream id** for every connection, as the API requires;
* restores all quote/depth/trade subscriptions and re-creates bar indicators
  (their server-side ids change; `StreamEvent::IndicatorResubscribed` tells
  you the new one, the `IndicatorKey` stays stable);
* never reports `Connected` with subscriptions silently missing: a transient
  failure while restoring drops the connection and tries again, a definitive
  refusal is reported as `SubscriptionLost`;
* renews an expired token by itself, and stops with `Closed` only when
  authentication is impossible or the configured attempts are exhausted;
* applies back-pressure instead of dropping events when the consumer is slow.

Updates published while the connection was down are **not** replayed by the
server. On every `Connected` event, re-read the state you care about. For
orders, `OrderManager::on_event` does exactly that:

```rust,no_run
# use ironbeam_sdk::{Client, orders::OrderManager, stream::{Stream, StreamConfig}};
# async fn run(client: Client) -> ironbeam_sdk::Result<()> {
let orders = OrderManager::new(client.clone(), "5123345");
let mut stream = Stream::connect(&client, StreamConfig::default()).await?;
while let Some(event) = stream.next().await {
    // Merges streamed order updates, de-duplicates replayed fills, and
    // reconciles against REST after every (re)connect.
    let outcome = orders.on_event(&event).await?;
    for change in outcome.changes { println!("{change:?}"); }
    for fill in outcome.fills { println!("{fill:?}"); }
}
# Ok(()) }
```

The tracker rejects stale information using two invariants (the API has no
sequence numbers): a terminal status is final, and cumulative filled quantity
never decreases. Orders it believes open that are absent from the server's
list are flagged `unconfirmed` and reported, never silently dropped.

## Configuration

`ClientConfig` (timeouts, retry policy, user agent, order-enum dialect) and
`StreamConfig` (idle timeout, backoff, attempt limit, buffer size) have
conservative defaults. Two settings deserve attention:

* `ClientConfig::environment` defaults to **demo**.
* `ClientConfig::enum_encoding` selects how `orderType`/`duration` are sent:
  `Codes` (`"2"`, `"0"` — what the schema says, the default) or `Names`
  (`"LIMIT"`, `"DAY"` — what the prose and examples say). If the real API
  rejects the default, flip this one setting. Responses accept both.

## Errors

| `Error` variant | When | `is_ambiguous()` |
|---|---|---|
| `InvalidRequest`, `Config`, `MissingCredentials` | Rejected locally; nothing was sent. | no |
| `Api { status, .. }` | Non-2xx, or a 2xx whose envelope says `ERROR`/`FATAL`. | only for 5xx |
| `RateLimited` | HTTP 429. | no |
| `Transport { delivery, .. }` | Network failure; `delivery` says whether the request can have left. | when `MaybeSent` |
| `Decode` | A success response that could not be parsed. | yes |
| `OrderOutcomeUnknown` | Any ambiguous failure of `place_order`/`submit`. | yes |
| `WebSocket`, `StreamClosed`, `StreamNotConnected` | Stream handling. | no |

For mutating calls other than order placement (`update_order`,
`cancel_order`, …) an ambiguous error means "may or may not have been
applied": re-read state before acting.

## Examples

All read credentials from `IRONBEAM_USERNAME` and `IRONBEAM_API_KEY` (and
optionally `IRONBEAM_PASSWORD`, `IRONBEAM_ENV`, `IRONBEAM_ACCOUNT`).

| Example | Touches orders? | What it shows |
|---|---|---|
| `account_snapshot` | no | balances, positions, risk, open orders |
| `stream_quotes` | no | resilient market-data streaming |
| `order_monitor` | no | consistent order view from stream + REST |
| `place_order` | **yes** | the three submission outcomes, then cancel; refuses live without an explicit opt-in |

```text
cargo run --example stream_quotes -- XCME:ES.Z26
```

## Tests

First download the OpenAPI specification to `spec/openapi.json` using the
instructions in [`spec/README.md`](spec/README.md). The protocol tests need
this local file; it is not included in the repository or crate package.

```text
cargo test                         # everything; no network access needed
cargo test --lib -- --ignored      # TLS handshake check against a public, non-Ironbeam host
```

* `tests/protocol.rs` validates every request the SDK can make against the
  locally downloaded OpenAPI document and decodes schema-derived example responses.
* `tests/rest.rs`, `tests/orders.rs`, `tests/stream.rs` run against an
  in-process mock of the API with fault injection: expired tokens, 5xx, 429,
  slow and aborted responses, connection resets, dropped and silent
  WebSockets, refused handshakes.

## Layout

```text
src/client.rs        authentication, request execution, retry classification
src/endpoints.rs     table of operations (checked against the spec by tests)
src/rest/            typed REST operations
src/models/          request/response/stream payloads
src/stream/          WebSocket task: liveness, reconnect, subscription recovery
src/orders/          order tracker, reconciliation, safe submission
spec/openapi.json    local specification for protocol tests (not distributed)
docs/                specification discrepancies and SDK assumptions
tests/               mock servers and integration tests
```

## License

The original implementation, tests, examples, and project documentation are
licensed under either the [MIT License](LICENSE-MIT) or the
[Apache License, Version 2.0](LICENSE-APACHE), at your option.

Third-party dependencies retain their own licenses. Ironbeam's OpenAPI
specification is not covered by this license grant and is not distributed
with the project; see [`spec/README.md`](spec/README.md).
