# Verification report

Date: 2026-11-12 · Crate: `ironbeam-sdk` 0.1.0 · Specification: Ironbeam OpenAPI
3.0.3, `info.version` 2.1 (local setup: [`spec/README.md`](spec/README.md))

**Bottom line.** The SDK covers every operation in the specification and its
own behaviour (retry rules, order safety, reconnection, reconciliation) is
tested thoroughly — but only against the specification and against mock
servers. **No request has been sent to the real Ironbeam API.** Where the
specification contradicts itself the SDK had to guess, and those guesses are
unverified. Do not use it on a live account before working through
[the checklist](#first-contact-checklist) on the demo environment.

## What was built

| Area | Content |
|---|---|
| REST | All 57 REST operations, typed, as methods on `Client` |
| WebSocket | `Stream`: decoding of all 18 documented stream fields, liveness watchdog, reconnect with backoff, subscription and indicator recovery, back-pressure |
| Orders | `OrderManager` / `OrderTracker`: single-shot submission, explicit unknown outcomes with read-only investigation, stale-update rejection, REST reconciliation after connection gaps, fill de-duplication |
| Auth | Lazy login, single-flight token renewal on 401, logout; secrets redacted from `Debug` |
| Docs | `README.md`, rustdoc on the public API, `docs/SPEC_DISCREPANCIES.md` (24 discrepancies, 19 ambiguities) |
| Examples | 4 (three read-only, one that places and cancels an order) — compiled, not run |

## Verified

Verified means: asserted by an automated test that passes. `cargo test` runs
118 tests (33 unit, 79 integration, 6 doc); the suite was run six times in a
row without a failure.

**Against the specification** (`tests/protocol.rs`)

* The SDK's endpoint table and the specification's operations match exactly,
  in both directions (method, path, operation id).
* Every REST method was called against a server that validates the request
  against the specification — routing, path and query parameters, bearer
  authentication, JSON body types, enums, required and unknown properties —
  with zero violations, and that answers with an example built from the
  response schema. Every documented response property survived decoding.
* Every documented stream field and every enum value in the specification is
  modelled; REST and abbreviated stream shapes decode to the same types.
* Only `GET`s and `/auth` are marked for automatic retry.

**Against the mock API** (`tests/rest.rs`, `tests/orders.rs`, `tests/stream.rs`)

* *Authentication:* lazy login; one renewal and one replay on 401; sixteen
  concurrent 401s cause a single re-login; a persistent 401 is not looped on.
* *Retries:* reads retry on 5xx, 429 (honouring `Retry-After`), timeouts and
  connection resets, and stop at the configured limit; 4xx is never retried.
* *Orders are never sent twice:* exactly one request reaches the wire on 500,
  502, 503, 504, 429, timeout, lost response, unreadable response and
  connection reset — including with an aggressive retry policy configured.
* *Outcome classification:* those ambiguous cases yield
  `OrderOutcomeUnknown`; 4xx, 429 and "connection refused" yield definitive
  errors; a 401 is renewed and replayed, producing exactly one order.
* *Investigation:* finds the order when it went through; reports not-found
  when it did not; ignores identical orders that pre-existed or belong to
  other submissions; reports look-alikes as ambiguous instead of guessing.
* *Cancel and modify* are not retried either.
* *Stream:* dropped sockets, server close frames and silent connections are
  detected and recovered with a new stream id; quotes, depth, trades and
  indicators are restored (under both "replace" and "extend" subscription
  semantics); backoff is exponential and capped; expired tokens are renewed;
  permanent authentication failure and exhausted attempts end the stream
  cleanly; malformed frames do not kill the connection; a slow consumer
  loses nothing and cannot deadlock the task; shutdown and drop stop it.
* *Reconciliation:* fills and new orders missed during a connection gap are
  recovered over REST on reconnect; stale frames cannot revert a terminal
  order; replayed fill snapshots are filtered.

**Against the network, without touching Ironbeam** (`cargo test --lib -- --ignored`,
run once, passed): the TLS stack completes real handshakes for both the HTTP
client and the WebSocket connector.

## Not verified — assumptions

Everything below is unconfirmed until exercised against the real service.
The mock servers were written from the same specification as the SDK, so they
cannot reveal a misreading of it. References are to
`docs/SPEC_DISCREPANCIES.md`.

Most likely to matter, in rough order:

1. **Order request encoding** (D-06, D-07, D-08). The schema, the prose and
   the examples disagree about property names and about whether `orderType`,
   `duration` and `side` are codes or names. The SDK follows the schema. If
   orders are rejected with 400, try `EnumEncoding::Names` first.
2. **Login body** (D-03). `apikey` vs `apiKey` vs `ApiKey`.
3. **Order status semantics** (A-04). Which statuses are final is inferred
   from FIX naming, and stale-update detection depends on it.
4. **What the order list contains** (A-08) and **whether order updates reach
   the stream without a subscription** (A-01).
5. **Meaning of 5xx and of `status: ERROR` in a 200** for orders (A-09, A-10).
6. **Matching unknown outcomes without a client order id** (A-07). Sound in
   the tests; inherently heuristic when another session trades the same
   account at the same moment.
7. **Keep-alive and token lifetime** (A-11, A-12): no client pong is sent;
   the 20 s idle timeout is a guess based on the documented 5 s ping.
8. **Response shapes in general.** Decoding is lenient (optional fields,
   unknown enum values preserved, unknown properties ignored), so surprises
   should degrade to missing fields rather than failures — except that an
   order without `orderId` or `status` is rejected on purpose.

Also not exercised: the Enterprise-only simulated-account endpoints beyond
schema conformance, real rate limits, HTTP/2 behaviour, long-running
operation over days.

## Known limitations

* A trade bust that lowers an order's filled quantity is treated as stale
  until the next REST reconciliation (A-05).
* `OrderTracker` grows with the number of orders and fills seen; call
  `prune_terminal()` periodically in long-running processes.
* Stream subscriptions are restored after the socket opens and before it is
  read, so a slow REST call delays the first frames of a new connection.
* The examples compile but have not been run.
* No license has been chosen for the crate, and it is marked `publish = false`.

## First-contact checklist

On the **demo** environment, in this order, stopping at the first surprise:

1. `account_snapshot` — confirms login (D-03), envelope and field names.
2. `stream_quotes` for a few minutes — confirms the socket URL, ping cadence
   and idle timeout (A-12), quote decoding; pull the network cable once.
3. `order_monitor` while placing an order by hand in another client —
   confirms that order updates stream unprompted (A-01) and what the order
   list returns (A-08); note the status sequence seen (A-04).
4. `place_order` with a far-from-market limit — confirms the order encoding
   (D-06 to D-08). On a 400, switch `enum_encoding` and retry.
5. Update `docs/SPEC_DISCREPANCIES.md` with what was observed.
