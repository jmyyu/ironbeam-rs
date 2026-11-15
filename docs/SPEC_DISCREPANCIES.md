# Specification discrepancies, ambiguities and SDK assumptions

Source: the OpenAPI 3.0.3 document embedded in <https://docs.ironbeamapi.com/>
(`info.version` 2.1), retrieved 2026-11-12 and stored unmodified in
the local specification described in [`spec/README.md`](../spec/README.md).

Two kinds of entries:

* **D-nn — discrepancies.** The document contradicts itself, or is malformed.
  Each entry states what the SDK does.
* **A-nn — ambiguities.** The document is silent on something a client needs
  to know. Each entry states the assumption the SDK makes.

None of the choices below has been checked against the real service. Entries
marked ⚠ can cause requests to be rejected or data to be misread if the guess
is wrong; they are the first things to verify on the demo environment.

## Discrepancies

| # | Where | What the document says | What the SDK does |
|---|---|---|---|
| D-01 | `info.description` | "accepts form-encoded request bodies", but every `requestBody` is `application/json` and all examples send JSON. | Sends JSON. |
| D-02 | `servers` vs. text | Servers are `…/v2`. Several examples and the stream section use `…/v1/…`; the change log says "1.0: Initial version" while `info.version` is 2.1. | Uses `/v2`. |
| D-03 ⚠ | `POST /auth` | Schema `AuthorizationRequest` has `username`, `password`, `apikey`. The prose shows `Username`/`Password`/`ApiKey`; the Python example sends `apiKey`. | Sends the schema names (`username`, `password`, `apikey`). Correct if the server matches names case-insensitively, as the examples imply. |
| D-04 | `GET /stream/create` | Defined as `GET`; the JavaScript and C# examples call it with `POST` (the Python one with `GET`). The C# example also omits the bearer token. | Uses `GET`. |
| D-05 | `OrderRequest.required` | Lists `account`, which is not a property (the property is `accountId`). | Sends `accountId` in the body as well as in the path. |
| D-06 ⚠ | `OrderRequest` examples | All five examples use properties that are not in the schema (`clientOrderId`, `type`, `stopLossPrice`, `takeProfitPrice`) and omit the required `orderType`. | Follows the schema (`orderType`, `stopLoss`, `takeProfit`). `OrderRequest::extra_field` can add undocumented properties if they turn out to be real. |
| D-07 ⚠ | `OrderType`, `DurationType` | Enum values are `"1"`..`"4"` and `"0"`/`"1"` (names only in `x-enumNames`), but the schema-level examples are `"LIMIT"` / `"1"`, property examples are `MARKET` / `DAY`, and the request examples send `"type": "2"` and `"duration": "DAY"`. | Sends the enum codes by default; `EnumEncoding::Names` switches to names. Accepts codes, names and bare numbers in responses. |
| D-08 ⚠ | `OrderSide` | Enum is `BUY`, `SELL`, `INVALID`; the description maps the invalid side to `""`; the `side` property example is `"0 // BUY"` and request examples send `"side": "0"`. | Sends `BUY`/`SELL`. Accepts `""` as invalid. Numeric sides in responses are preserved as `Unknown("0")` rather than guessed. |
| D-09 | `OrderUpdateRequest` | `quantity` is `integer/int32` while every other quantity is `number/double`. `orderId` is required in the body although it is also a path parameter. | Sends an integer quantity and repeats `orderId` in the body. |
| D-10 | `OrderRequest` description | Says `stopLossOffset`/`takeProfitOffset` are "currently not supported" and then defines how they are calculated; the property descriptions treat them as supported. | Passes them through; rejects only offset *and* absolute price together (which the update schema forbids). |
| D-11 | `TradesResponse` | The list property is named `traders`. | Reads `traders`, accepts `trades` as an alias. |
| D-12 | `AccountsPositionsResponse` | `positions.items` is a JSON array (tuple form), invalid in OpenAPI 3.0. | Accepts both a flat list and a list of lists. |
| D-13 | Casing | `QuotesResponse.Quotes`, `DepthResponse.Depths`, `StrategyIdResponse.Id/Minimum/Maximum` and all simulated-account schemas are PascalCase; everything else is camelCase. | Uses the documented casing; accepts the camelCase form too when reading. |
| D-14 | Misspelled properties | `userDefinded`, `intradayDefinded`, `initialMaginShort`. | Reads the misspelled names, accepts the correct spelling as an alias. |
| D-15 | Integer enums with string examples | `QuoteFull.ags` (integer 0/1/2) has example `BUY`; `secs` and `SecurityStatus.status` (integer codes) have example `OPEN`; `td` has example `PLUS`. | Accepts the integer code or the name. |
| D-16 | Tick direction | `TickDirectionType` (quotes) is `255,0,1,2` = INVALID, PLUS, SAME, MINUS; `TickDirection` (trades) is a string enum listed in a different order. | One enum that accepts both encodings. |
| D-17 | `SecurityDefinitionLeg.side` | Typed as `Side` (`BID`/`ASK`), example `BUY`. | Accepts all four. |
| D-18 | Stream `ri` | The prose documents `mnlvmd` (multi-day max); schema `RiskInfoOpt` lacks it. The sample shows liquidation event code `0`, which is not among the documented codes. | Reads `mnlvmd`. `RiskInfo::is_liquidated()` returns `None` when the last code is undocumented. |
| D-19 | Symbol pattern | `^([A-Z]{1,10}:)?[A-Z0-9 _\.]{1,120}$`, but the quote example is `XCME:6E1!`. `Spread` has `maxLength: 21`, shorter than its own example. | Does not enforce the patterns; rejects only characters that would corrupt a URL or the comma-separated list. |
| D-20 | `GET /info/symbols` | Text says the result is limited to 100 and that an empty `text` returns the first 100; the parameters say default limit 1000, maximum 1000, and `text` has `minLength: 3`. | Validates `text` 3..=20 and `limit` 1..=1000 when given; sends neither otherwise. |
| D-21 | Error responses | On the account endpoints 406 and 429 reference `ErrorResponse403`. `Error` declares only `error`, while its example also carries `status` and `message`. | Maps errors by HTTP status and reads `error`, falling back to `message`. |
| D-22 | `DELETE` with a body | `cancelMultiple` and `simulatedAccountExpire` are `DELETE` requests with a JSON body, which some proxies strip. | Sends the body as documented. |
| D-23 | Tags | Operations use tags (`Authorization`, `Info`, `Market`, `Order`, `Indicator`) that differ from the declared tag list. | No effect. |
| D-24 | `cancelMultiple` | The schema example is an array of objects; the schema is a single object. The path already carries the account. | Sends a single object including `accountId`. |

## Ambiguities and assumptions

| # | Question the document leaves open | Assumption in the SDK | If wrong |
|---|---|---|---|
| A-01 | Which stream data needs a subscription? There are endpoints for quotes, depth, trades and bars only. | Orders, fills, positions, balances and risk for the user's accounts arrive on any open stream. | No order updates would arrive; REST reconciliation still works, on demand. |
| A-02 ⚠ | Does subscribing *replace* or *extend* a stream's symbol list ("to change the list, call again with the new list")? | Unknown, so the SDK always sends the complete desired list when adding and calls unsubscribe when removing. Correct under both readings. | — |
| A-03 | Is "10 instruments" a limit per call or per stream? | Per stream and kind; enforced locally. | Over-restrictive; raise `MAX_SYMBOLS`. |
| A-04 ⚠ | Semantics of the 23 order statuses. | FIX `OrdStatus` meanings: pending (`SUBMITTED`, `PENDING_NEW`, `QUEUED_NEW`), working (`NEW`, `PARTIALLY_FILLED`, `REPLACED`, `PENDING_*`, `CANCEL_REJECTED`, …), terminal (`FILLED`, `CANCELLED`, `REJECTED`, `EXPIRED`, `DONE_FOR_DAY`, `COMPLETE`). Terminal is treated as final. | A status wrongly classed terminal would make later updates look stale; wrongly classed working, an order would look open. |
| A-05 ⚠ | Ordering of order updates: there are no sequence numbers or timestamps on `o` messages, and no documented order snapshot on connect. | Stale updates are detected by "terminal is final" and "filled quantity never decreases". After every (re)connect the full order list is re-read over REST. | A legitimately decreasing fill quantity (trade bust) would be ignored until the next REST reconciliation. |
| A-06 | Are stream `o` updates complete objects? | Possibly partial: absent fields keep their previous values. | — |
| A-07 ⚠ | There is no documented client order id or idempotency key (`clientOrderId` appears only in examples; `/info/strategyId` issues ids but no request accepts one). | An unknown outcome is matched by "appeared after the submission" plus symbol, side, quantity, type and prices. Several matches are reported as ambiguous. | Another session placing an identical order in the same window can cause a false match; hence `Found` returns the order for review. |
| A-08 | What does `GET /order/{accountId}/ANY` return — all orders ever, the trading day's, only open ones? | It contains every currently open order. | Open orders absent from the list are flagged `unconfirmed` (never deleted), so a wrong assumption is visible rather than silent. |
| A-09 | Can a 200 response carry a failure? `Response.status` includes `ERROR` and `FATAL`. | A 2xx whose top-level `status` is `ERROR`/`FATAL` is an error. For order placement that counts as a definitive rejection. | If such a response could accompany an accepted order, it would be misreported as rejected. |
| A-10 ⚠ | What do 5xx responses mean for an order? | Unknown outcome: the order may have been accepted. Never retried. | Conservative either way. |
| A-11 | Token lifetime, and whether a new `/auth` invalidates earlier tokens or open streams. | Lifetime unknown; a 401 triggers one re-authentication and one replay. | If re-authentication kills open streams, they reconnect by themselves. |
| A-12 | Keep-alive: the server sends `{"p":{"ping":"ping"}}` every 5 s. Must the client answer? | No. 20 s of total silence means the connection is dead. | If a reply is required the server will drop the stream periodically; the SDK reconnects, and the pattern would be visible in the logs. |
| A-13 | Where do bar subscriptions deliver data: `tb`/`tc`/`ti`/`vb`, or the generic `i` field? | Both are modelled. | — |
| A-14 | Rate limits: 429 exists, limits and `Retry-After` are not documented. | Reads back off exponentially and honour `Retry-After` when present. Orders are not retried after a 429. | — |
| A-15 | Meaning of `earlier` in the trades path, of `marginO`/`OW`/`OWI`, and of `fillPrice` on an order ("Average fill price"). | Passed through / exposed as documented, with the uncertainty noted in the rustdoc. | — |
| A-16 | Are quantities ever fractional? They are `double` on placement. | Sent as numbers; the update call takes an integer because its schema says so. | — |
| A-17 | Time units and zones. | `Timestamp` fields are epoch milliseconds (documented); `fillDate` is kept as the string sent. | — |
| A-18 | Does the server replay missed stream data after a reconnect? The document says only that a new stream id is needed and that fills/positions/balances/risk snapshots are sent "after subscription". | Nothing is replayed. Fill snapshots are de-duplicated by `orderUpdateId`. | — |
| A-19 | Does cancelling an already-cancelled order succeed or fail? | Unknown, which is why cancels are not auto-retried: a second attempt could turn a successful cancel into a reported error. | — |
