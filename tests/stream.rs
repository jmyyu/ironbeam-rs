//! WebSocket streaming: decoding, liveness, reconnection, subscription
//! recovery, and order-state reconciliation across connection failures.

mod support;

use std::time::Duration;

use serde_json::json;

use ironbeam_sdk::models::{BarSubscription, BarType, OrderRequest, OrderSide, OrderStatus};
use ironbeam_sdk::orders::{OrderChange, OrderManager};
use ironbeam_sdk::rest::BarKind;
use ironbeam_sdk::stream::{DisconnectReason, MarketDataKind, Stream, StreamConfig, StreamEvent};
use ironbeam_sdk::{Client, Error};
use support::mock::{ACCOUNT, Fault, Mock};
use support::{eventually, wait_for};

fn fast() -> StreamConfig {
    StreamConfig {
        idle_timeout: Duration::from_millis(400),
        connect_timeout: Duration::from_secs(2),
        reconnect_base_delay: Duration::from_millis(10),
        reconnect_max_delay: Duration::from_millis(40),
        reconnect_jitter: false,
        max_reconnect_attempts: None,
        stable_after: Duration::from_secs(60),
        event_buffer: 256,
    }
}

async fn connected(stream: &mut Stream, generation: u64) -> String {
    let event = wait_for(stream, "Connected", |e| {
        matches!(e, StreamEvent::Connected { .. })
    })
    .await;
    match event {
        StreamEvent::Connected {
            generation: g,
            stream_id,
        } => {
            assert_eq!(g, generation, "unexpected connection generation");
            stream_id
        }
        _ => unreachable!(),
    }
}

async fn next_data(stream: &mut Stream) -> ironbeam_sdk::models::StreamMessage {
    match wait_for(stream, "Data", |e| matches!(e, StreamEvent::Data(_))).await {
        StreamEvent::Data(message) => *message,
        _ => unreachable!(),
    }
}

#[tokio::test]
async fn connects_with_token_in_query_and_delivers_decoded_frames() {
    let mock = Mock::start().await;
    let client = mock.client();
    let mut stream = Stream::connect(&client, fast()).await.unwrap();
    let id = connected(&mut stream, 1).await;
    assert_eq!(mock.stream_ids(), vec![id.clone()]);
    assert!(stream.handle().is_connected());

    let socket = &mock.requests("GET", &format!("/stream/{id}"))[0];
    assert_eq!(
        socket.query, "token=tok-1",
        "the token travels in the query string"
    );

    mock.ws_send(&id, json!({"q": [{"s": "XCME:ES.Z26", "b": 5000.25, "bs": 12, "a": 5000.5, "as": 9, "secs": 17}]}));
    let message = next_data(&mut stream).await;
    let quote = &message.quotes[0];
    assert_eq!(quote.symbol.as_deref(), Some("XCME:ES.Z26"));
    assert_eq!((quote.bid, quote.ask), (Some(5000.25), Some(5000.5)));
    assert_eq!(
        quote.security_status,
        Some(ironbeam_sdk::models::SecurityStatus::Open)
    );

    mock.ws_send(
        &id,
        json!({
            "d": [{"s": "XCME:ES.Z26", "b": [{"l": 0, "s": "B", "p": 5000.25, "sz": 12}], "a": []}],
            "tr": [{"s": "XCME:ES.Z26", "p": 5000.5, "sz": 2, "as": 1, "td": "PLUS"}]
        }),
    );
    let message = next_data(&mut stream).await;
    assert_eq!(message.depths[0].bids[0].price, Some(5000.25));
    assert_eq!(message.trades[0].size, Some(2.0));
}

#[tokio::test]
async fn pings_are_consumed_and_keep_the_connection_alive() {
    let mock = Mock::start().await;
    let client = mock.client();
    let mut stream = Stream::connect(&client, fast()).await.unwrap();
    let id = connected(&mut stream, 1).await;
    // Several idle timeouts' worth of pings and nothing else.
    tokio::time::sleep(Duration::from_millis(1000)).await;
    mock.ws_send(&id, json!({"b": {"a": ACCOUNT, "cc": "USD", "cb": 100.0}}));
    // The first event after the pause is the data frame: no ping events, no
    // disconnect.
    match stream.next().await.unwrap() {
        StreamEvent::Data(message) => assert_eq!(message.balances[0].cash_balance, Some(100.0)),
        other => panic!("{other:?}"),
    }
    assert_eq!(mock.stream_ids().len(), 1);
}

#[tokio::test]
async fn undecodable_frames_are_reported_without_dropping_the_connection() {
    let mock = Mock::start().await;
    let client = mock.client();
    let mut stream = Stream::connect(&client, fast()).await.unwrap();
    let id = connected(&mut stream, 1).await;
    mock.ws_send_raw(&id, "this is not json");
    mock.ws_send(&id, json!({"o": [{"oid": "1"}]})); // order without a status
    mock.ws_send(&id, json!({"q": [{"s": "XCME:ES.Z26"}]}));
    for _ in 0..2 {
        match stream.next().await.unwrap() {
            StreamEvent::Undecodable { raw, .. } => assert!(!raw.is_empty()),
            other => panic!("{other:?}"),
        }
    }
    assert_eq!(next_data(&mut stream).await.quotes.len(), 1);
    assert_eq!(mock.stream_ids().len(), 1, "no reconnect happened");
}

#[tokio::test]
async fn subscriptions_are_applied_and_bounded() {
    let mock = Mock::start().await;
    let client = mock.client();
    let mut stream = Stream::connect(&client, fast()).await.unwrap();
    let handle = stream.handle();
    let id = connected(&mut stream, 1).await;

    handle
        .subscribe_quotes(&["XCME:ES.Z26", "XCME:NQ.Z26"])
        .await
        .unwrap();
    handle.subscribe_depths(&["XCME:ES.Z26"]).await.unwrap();
    handle.subscribe_trades(&["XCME:ES.Z26"]).await.unwrap();
    handle.subscribe_quotes(&["XCME:YM.Z26"]).await.unwrap();
    handle
        .unsubscribe(MarketDataKind::Quotes, &["XCME:NQ.Z26"])
        .await
        .unwrap();
    // No-ops do not reach the server.
    let calls = mock.count("GET", "/market/quotes/");
    handle.subscribe_quotes(&["XCME:ES.Z26"]).await.unwrap();
    handle
        .unsubscribe(MarketDataKind::Quotes, &["XCME:XX.Z26"])
        .await
        .unwrap();
    assert_eq!(mock.count("GET", "/market/quotes/"), calls);

    let (quotes, depths, trades, _) = mock.subscriptions(&id);
    assert_eq!(quotes, ["XCME:ES.Z26", "XCME:YM.Z26"]);
    assert_eq!(depths, ["XCME:ES.Z26"]);
    assert_eq!(trades, ["XCME:ES.Z26"]);
    let subs = handle.subscriptions().await.unwrap();
    assert_eq!(subs.quotes.len(), 2);

    // The API allows 10 symbols per stream and kind.
    let many: Vec<String> = (0..9).map(|i| format!("XCME:S{i}")).collect();
    let err = handle.subscribe_quotes(&many).await.unwrap_err();
    assert!(matches!(err, Error::InvalidRequest(_)), "{err:?}");
    let eleven: Vec<String> = (0..11).map(|i| format!("XCME:S{i}")).collect();
    assert!(matches!(
        handle.subscribe_depths(&eleven).await,
        Err(Error::InvalidRequest(_))
    ));
    assert_eq!(
        handle.subscriptions().await.unwrap().quotes.len(),
        2,
        "failed request left no trace"
    );
}

#[tokio::test]
async fn refused_subscription_is_not_remembered() {
    let mock = Mock::start().await;
    let client = mock.client();
    let mut stream = Stream::connect(&client, fast()).await.unwrap();
    let handle = stream.handle();
    connected(&mut stream, 1).await;
    mock.fault(
        "getSubscribeQuotes",
        Fault::Status(400, json!({"error": "unknown symbol"})),
    );
    let err = handle.subscribe_quotes(&["XCME:BAD"]).await.unwrap_err();
    assert_eq!(err.status(), Some(400));
    assert!(handle.subscriptions().await.unwrap().quotes.is_empty());
}

#[tokio::test]
async fn dropped_connection_reconnects_with_a_new_stream_id_and_restores_everything() {
    for replaces in [false, true] {
        let mock = Mock::start().await;
        // The specification does not say whether subscribing replaces or
        // extends a stream's symbol list; recovery must work either way.
        mock.set_subscribe_replaces(replaces);
        let client = mock.client();
        let mut stream = Stream::connect(&client, fast()).await.unwrap();
        let handle = stream.handle();
        let first = connected(&mut stream, 1).await;

        handle.subscribe_quotes(&["XCME:ES.Z26"]).await.unwrap();
        handle.subscribe_quotes(&["XCME:NQ.Z26"]).await.unwrap();
        handle.subscribe_depths(&["XCME:ES.Z26"]).await.unwrap();
        handle.subscribe_trades(&["XCME:CL.Z26"]).await.unwrap();
        let bars = BarSubscription::new("XCME:ES.Z26", 1, BarType::Minute, 50);
        let indicator = handle
            .subscribe_bars(BarKind::Time, bars.clone())
            .await
            .unwrap();
        assert_eq!(indicator.indicator_id, "timeBars_1");
        assert_eq!(indicator.value_names.len(), 8);

        mock.ws_drop(&first);
        let event = wait_for(&mut stream, "Disconnected", |e| {
            matches!(e, StreamEvent::Disconnected { .. })
        })
        .await;
        assert!(
            matches!(
                &event,
                StreamEvent::Disconnected {
                    reason: DisconnectReason::SocketError(_)
                        | DisconnectReason::ServerClosed { .. },
                    ..
                }
            ),
            "{event:?}"
        );
        assert!(!handle.is_connected());

        let event = wait_for(&mut stream, "IndicatorResubscribed", |e| {
            matches!(e, StreamEvent::IndicatorResubscribed { .. })
        })
        .await;
        match event {
            StreamEvent::IndicatorResubscribed {
                previous_id,
                subscription,
            } => {
                assert_eq!(previous_id, "timeBars_1");
                assert_eq!(subscription.indicator_id, "timeBars_2");
                assert_eq!(
                    subscription.key, indicator.key,
                    "the key is stable across reconnects"
                );
                assert_eq!(subscription.request, bars);
            }
            _ => unreachable!(),
        }
        let second = connected(&mut stream, 2).await;
        assert_ne!(first, second, "a closed stream id must never be reused");
        assert!(handle.is_connected());

        let (quotes, depths, trades, indicators) = mock.subscriptions(&second);
        assert_eq!(
            quotes,
            ["XCME:ES.Z26", "XCME:NQ.Z26"],
            "replaces={replaces}"
        );
        assert_eq!(depths, ["XCME:ES.Z26"]);
        assert_eq!(trades, ["XCME:CL.Z26"]);
        assert_eq!(indicators, ["timeBars_2"]);

        // Data flows again, and the indicator can be removed by its stable key.
        mock.ws_send(
            &second,
            json!({"ti": [{"t": 1, "o": 1.0, "c": 2.0, "i": "timeBars_2"}]}),
        );
        assert_eq!(
            next_data(&mut stream).await.time_bars[0]
                .indicator
                .as_deref(),
            Some("timeBars_2")
        );
        handle.unsubscribe_indicator(indicator.key).await.unwrap();
        assert!(mock.subscriptions(&second).3.is_empty());
        assert!(handle.subscriptions().await.unwrap().indicators.is_empty());
    }
}

#[tokio::test]
async fn silent_connection_is_detected_by_the_idle_watchdog() {
    let mock = Mock::start().await;
    let client = mock.client();
    let mut stream = Stream::connect(&client, fast()).await.unwrap();
    let first = connected(&mut stream, 1).await;
    // The TCP connection stays open but the server stops talking.
    mock.ws_silence(&first);
    let started = std::time::Instant::now();
    let event = wait_for(&mut stream, "Disconnected", |e| {
        matches!(e, StreamEvent::Disconnected { .. })
    })
    .await;
    assert!(
        matches!(
            event,
            StreamEvent::Disconnected {
                reason: DisconnectReason::IdleTimeout,
                ..
            }
        ),
        "{event:?}"
    );
    assert!(
        started.elapsed() >= Duration::from_millis(350),
        "fired too early: {:?}",
        started.elapsed()
    );
    let second = connected(&mut stream, 2).await;
    assert_ne!(first, second);
}

#[tokio::test]
async fn server_close_frame_triggers_reconnect() {
    let mock = Mock::start().await;
    let client = mock.client();
    let mut stream = Stream::connect(&client, fast()).await.unwrap();
    let first = connected(&mut stream, 1).await;
    mock.ws_close(&first, 1001, "going away");
    let event = wait_for(&mut stream, "Disconnected", |e| {
        matches!(e, StreamEvent::Disconnected { .. })
    })
    .await;
    match event {
        StreamEvent::Disconnected {
            reason: DisconnectReason::ServerClosed { code, reason },
            ..
        } => {
            assert_eq!(code, Some(1001));
            assert_eq!(reason, "going away");
        }
        other => panic!("{other:?}"),
    }
    connected(&mut stream, 2).await;
}

#[tokio::test]
async fn reconnect_backs_off_through_repeated_failures() {
    let mock = Mock::start().await;
    let mut config = mock.config();
    config.retry = ironbeam_sdk::RetryPolicy::none();
    let client = mock.client_with(config);
    let mut stream = Stream::connect(&client, fast()).await.unwrap();
    let first = connected(&mut stream, 1).await;

    // Stream creation fails three times, then the handshake is refused once.
    for _ in 0..3 {
        mock.fault(
            "getStreamId",
            Fault::Status(503, json!({"error": "unavailable"})),
        );
    }
    mock.reject_ws(502);
    mock.ws_drop(&first);

    let mut delays = Vec::new();
    let mut failures = 0;
    loop {
        match wait_for(&mut stream, "reconnect", |e| {
            matches!(
                e,
                StreamEvent::Disconnected { .. } | StreamEvent::Connected { .. }
            )
        })
        .await
        {
            StreamEvent::Disconnected { reason, retry_in } => {
                if matches!(reason, DisconnectReason::ConnectFailed(_)) {
                    failures += 1;
                }
                delays.push(retry_in);
            }
            StreamEvent::Connected { generation, .. } => {
                assert_eq!(generation, 2);
                break;
            }
            _ => unreachable!(),
        }
    }
    assert_eq!(
        failures, 4,
        "three failed stream creations and one refused handshake"
    );
    let ms: Vec<u128> = delays.iter().map(Duration::as_millis).collect();
    assert_eq!(
        ms,
        [10, 20, 40, 40, 40],
        "exponential backoff capped at the maximum"
    );
}

#[tokio::test]
async fn gives_up_after_the_configured_number_of_attempts() {
    let mock = Mock::start().await;
    let mut config = mock.config();
    config.retry = ironbeam_sdk::RetryPolicy::none();
    let client = mock.client_with(config);
    let mut stream_config = fast();
    stream_config.max_reconnect_attempts = Some(2);
    let mut stream = Stream::connect(&client, stream_config).await.unwrap();
    let handle = stream.handle();
    let first = connected(&mut stream, 1).await;
    for _ in 0..10 {
        mock.fault(
            "getStreamId",
            Fault::Status(503, json!({"error": "unavailable"})),
        );
    }
    mock.ws_drop(&first);
    let event = wait_for(&mut stream, "Closed", |e| {
        matches!(e, StreamEvent::Closed { .. })
    })
    .await;
    assert!(
        matches!(&event, StreamEvent::Closed { error } if error.contains("giving up")),
        "{event:?}"
    );
    assert_eq!(stream.next().await, None, "nothing follows Closed");
    assert!(matches!(
        handle.subscribe_quotes(&["XCME:ES.Z26"]).await,
        Err(Error::StreamClosed)
    ));
    assert_eq!(
        mock.count("GET", "/stream/create"),
        3,
        "initial + 2 attempts"
    );
}

#[tokio::test]
async fn expired_token_is_renewed_during_reconnect() {
    let mock = Mock::start().await;
    let client = mock.client();
    let mut stream = Stream::connect(&client, fast()).await.unwrap();
    let handle = stream.handle();
    let first = connected(&mut stream, 1).await;
    handle.subscribe_quotes(&["XCME:ES.Z26"]).await.unwrap();
    assert_eq!(mock.auth_count(), 1);

    mock.expire_tokens();
    mock.ws_drop(&first);
    let second = connected(&mut stream, 2).await;
    assert_eq!(
        mock.auth_count(),
        2,
        "the stream re-authenticated by itself"
    );
    let socket = &mock.requests("GET", &format!("/stream/{second}"))[0];
    assert_eq!(socket.query, "token=tok-2");
    assert_eq!(mock.subscriptions(&second).0, ["XCME:ES.Z26"]);
}

#[tokio::test]
async fn token_refused_at_handshake_is_renewed_on_the_next_attempt() {
    let mock = Mock::start().await;
    let client = mock.client();
    let mut stream = Stream::connect(&client, fast()).await.unwrap();
    let first = connected(&mut stream, 1).await;
    // REST still accepts the token but the WebSocket handshake answers 401.
    mock.reject_ws(401);
    mock.ws_drop(&first);
    connected(&mut stream, 2).await;
    assert_eq!(mock.auth_count(), 2);
}

#[tokio::test]
async fn permanent_authentication_failure_closes_the_stream() {
    let mock = Mock::start().await;
    // A client with a valid token but no credentials to renew it.
    let seed = mock.client();
    seed.authenticate().await.unwrap();
    let token = seed.token().unwrap().to_string();
    let client = Client::builder()
        .config(mock.config())
        .token(token)
        .build()
        .unwrap();

    let mut stream = Stream::connect(&client, fast()).await.unwrap();
    let first = connected(&mut stream, 1).await;
    mock.expire_tokens();
    mock.ws_drop(&first);
    let event = wait_for(&mut stream, "Closed", |e| {
        matches!(e, StreamEvent::Closed { .. })
    })
    .await;
    assert!(
        matches!(&event, StreamEvent::Closed { error } if error.contains("401")),
        "{event:?}"
    );
    assert_eq!(stream.next().await, None);
}

#[tokio::test]
async fn initial_connection_failure_is_returned_to_the_caller() {
    let mock = Mock::start().await;
    let mut config = mock.config();
    config.retry = ironbeam_sdk::RetryPolicy::none();
    let client = mock.client_with(config);
    mock.fault(
        "getStreamId",
        Fault::Status(503, json!({"error": "unavailable"})),
    );
    let err = Stream::connect(&client, fast()).await.unwrap_err();
    assert_eq!(err.status(), Some(503));

    mock.reject_ws(403);
    let err = Stream::connect(&client, fast()).await.unwrap_err();
    assert!(matches!(err, Error::WebSocket(_)), "{err:?}");
    let text = err.to_string();
    assert!(
        !text.contains("tok-"),
        "errors must not leak the token: {text}"
    );
}

#[tokio::test]
async fn subscription_refused_during_recovery_is_reported_and_dropped() {
    let mock = Mock::start().await;
    let client = mock.client();
    let mut stream = Stream::connect(&client, fast()).await.unwrap();
    let handle = stream.handle();
    let first = connected(&mut stream, 1).await;
    handle.subscribe_quotes(&["XCME:ES.Z26"]).await.unwrap();
    handle.subscribe_trades(&["XCME:ES.Z26"]).await.unwrap();

    // After the reconnect the server refuses the quote subscription
    // (e.g. the contract expired).
    mock.fault(
        "getSubscribeQuotes",
        Fault::Status(400, json!({"error": "symbol expired"})),
    );
    mock.ws_drop(&first);
    let event = wait_for(&mut stream, "SubscriptionLost", |e| {
        matches!(e, StreamEvent::SubscriptionLost { .. })
    })
    .await;
    match event {
        StreamEvent::SubscriptionLost { what, error } => {
            assert!(
                what.contains("Quotes") && what.contains("XCME:ES.Z26"),
                "{what}"
            );
            assert!(error.contains("symbol expired"), "{error}");
        }
        _ => unreachable!(),
    }
    let second = connected(&mut stream, 2).await;
    let subs = handle.subscriptions().await.unwrap();
    assert!(subs.quotes.is_empty());
    assert_eq!(
        mock.subscriptions(&second).2,
        ["XCME:ES.Z26"],
        "other subscriptions survive"
    );
}

#[tokio::test]
async fn transient_failure_during_recovery_retries_on_a_fresh_connection() {
    let mock = Mock::start().await;
    let mut config = mock.config();
    config.retry = ironbeam_sdk::RetryPolicy::none();
    let client = mock.client_with(config);
    let mut stream = Stream::connect(&client, fast()).await.unwrap();
    let handle = stream.handle();
    let first = connected(&mut stream, 1).await;
    handle.subscribe_quotes(&["XCME:ES.Z26"]).await.unwrap();

    mock.fault(
        "getSubscribeQuotes",
        Fault::Status(503, json!({"error": "unavailable"})),
    );
    mock.ws_drop(&first);
    let event = wait_for(&mut stream, "RestoreFailed", |e| {
        matches!(
            e,
            StreamEvent::Disconnected {
                reason: DisconnectReason::RestoreFailed(_),
                ..
            }
        )
    })
    .await;
    assert!(matches!(event, StreamEvent::Disconnected { .. }));
    // Never "Connected" with data silently missing: the next event of that
    // kind is a connection whose subscriptions are complete.
    let id = connected(&mut stream, 3).await;
    assert_eq!(mock.subscriptions(&id).0, ["XCME:ES.Z26"]);
    assert_eq!(handle.subscriptions().await.unwrap().quotes.len(), 1);
}

#[tokio::test]
async fn subscriptions_made_while_offline_are_applied_on_reconnect() {
    let mock = Mock::start().await;
    let mut config = mock.config();
    config.retry = ironbeam_sdk::RetryPolicy::none();
    let client = mock.client_with(config);
    let mut stream_config = fast();
    stream_config.reconnect_base_delay = Duration::from_millis(300);
    stream_config.reconnect_max_delay = Duration::from_millis(300);
    let mut stream = Stream::connect(&client, stream_config).await.unwrap();
    let handle = stream.handle();
    let first = connected(&mut stream, 1).await;
    handle
        .subscribe_quotes(&["XCME:ES.Z26", "XCME:NQ.Z26"])
        .await
        .unwrap();

    mock.ws_drop(&first);
    wait_for(&mut stream, "Disconnected", |e| {
        matches!(e, StreamEvent::Disconnected { .. })
    })
    .await;
    // During the backoff window:
    handle.subscribe_trades(&["XCME:CL.Z26"]).await.unwrap();
    handle
        .unsubscribe(MarketDataKind::Quotes, &["XCME:NQ.Z26"])
        .await
        .unwrap();
    let bars = BarSubscription::new("XCME:ES.Z26", 1, BarType::Minute, 10);
    assert!(matches!(
        handle.subscribe_bars(BarKind::Trade, bars).await,
        Err(Error::StreamNotConnected)
    ));

    let second = connected(&mut stream, 2).await;
    let (quotes, _, trades, _) = mock.subscriptions(&second);
    assert_eq!(quotes, ["XCME:ES.Z26"]);
    assert_eq!(trades, ["XCME:CL.Z26"]);
}

#[tokio::test]
async fn shutdown_stops_the_task_and_closes_the_socket() {
    let mock = Mock::start().await;
    let client = mock.client();
    let mut stream = Stream::connect(&client, fast()).await.unwrap();
    let handle = stream.handle();
    let id = connected(&mut stream, 1).await;
    handle.shutdown();
    handle.shutdown();
    assert_eq!(stream.next().await, None);
    assert!(!handle.is_connected());
    assert!(matches!(
        handle.subscriptions().await,
        Err(Error::StreamClosed)
    ));
    eventually("server sees the socket close", || {
        mock.connected_streams().is_empty()
    })
    .await;
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert_eq!(mock.stream_ids(), vec![id], "no reconnect after shutdown");
}

#[tokio::test]
async fn dropping_the_stream_stops_the_task() {
    let mock = Mock::start().await;
    let client = mock.client();
    let mut stream = Stream::connect(&client, fast()).await.unwrap();
    connected(&mut stream, 1).await;
    drop(stream);
    eventually("server sees the socket close", || {
        mock.connected_streams().is_empty()
    })
    .await;
}

#[tokio::test]
async fn slow_consumers_apply_back_pressure_without_loss_or_deadlock() {
    let mock = Mock::start().await;
    let client = mock.client();
    let mut config = fast();
    config.event_buffer = 2;
    config.idle_timeout = Duration::from_secs(5);
    let mut stream = Stream::connect(&client, config).await.unwrap();
    let handle = stream.handle();
    let id = connected(&mut stream, 1).await;

    const N: i64 = 500;
    for n in 0..N {
        mock.ws_send(&id, json!({"tr": [{"s": "XCME:ES.Z26", "sq": n}]}));
    }
    // Let the tiny buffer fill up, then issue a command from the consuming
    // task without draining events first. This must not deadlock.
    tokio::time::sleep(Duration::from_millis(100)).await;
    tokio::time::timeout(
        Duration::from_secs(3),
        handle.subscribe_quotes(&["XCME:ES.Z26"]),
    )
    .await
    .expect("command served while the event buffer is full")
    .unwrap();

    for expected in 0..N {
        let message = next_data(&mut stream).await;
        assert_eq!(
            message.trades[0].sequence_number,
            Some(expected),
            "frames lost or reordered"
        );
    }
    assert_eq!(
        mock.stream_ids().len(),
        1,
        "back-pressure must not cause a reconnect"
    );
}

#[tokio::test]
async fn orders_are_reconciled_after_a_connection_gap() {
    let mock = Mock::start().await;
    let client = mock.client();
    let manager = OrderManager::new(client.clone(), ACCOUNT);
    let mut stream = Stream::connect(&client, fast()).await.unwrap();

    // Generation 1: the initial reconciliation picks up existing orders.
    let working = mock.add_order(json!({"quantity": 3.0}));
    let event = wait_for(&mut stream, "Connected", |e| {
        matches!(e, StreamEvent::Connected { .. })
    })
    .await;
    let first = match &event {
        StreamEvent::Connected { stream_id, .. } => stream_id.clone(),
        _ => unreachable!(),
    };
    let outcome = manager.on_event(&event).await.unwrap();
    assert!(outcome.reconciled.is_some());
    assert!(matches!(&outcome.changes[..], [OrderChange::Created(o)] if o.order_id == working));

    // Live updates arrive in the abbreviated stream shape.
    mock.ws_send(
        &first,
        json!({"o": [{"oid": working, "a": ACCOUNT, "st": "PARTIALLY_FILLED", "fq": 1.0}]}),
    );
    let event = wait_for(&mut stream, "Data", |e| matches!(e, StreamEvent::Data(_))).await;
    let outcome = manager.on_event(&event).await.unwrap();
    match &outcome.changes[..] {
        [OrderChange::Updated { previous, current }] => {
            assert_eq!(previous.status, OrderStatus::New);
            assert_eq!(current.status, OrderStatus::PartiallyFilled);
            assert_eq!(
                current.quantity,
                Some(3.0),
                "fields absent from the update are kept"
            );
        }
        other => panic!("{other:?}"),
    }
    // Updates for other accounts are ignored.
    mock.ws_send(
        &first,
        json!({"o": [{"oid": "X-1", "a": "OTHER", "st": "NEW"}]}),
    );
    let event = wait_for(&mut stream, "Data", |e| matches!(e, StreamEvent::Data(_))).await;
    assert!(manager.on_event(&event).await.unwrap().changes.is_empty());

    // The connection dies. While it is down the order fills completely and a
    // new order appears; neither update is ever streamed.
    mock.ws_drop(&first);
    mock.patch_order(
        &working,
        json!({"status": "FILLED", "fillQuantity": 3.0, "fillPrice": 5000.0}),
    );
    let appeared = mock.add_order(json!({"side": "SELL"}));

    let event = wait_for(&mut stream, "Connected", |e| {
        matches!(e, StreamEvent::Connected { .. })
    })
    .await;
    assert!(matches!(
        event,
        StreamEvent::Connected { generation: 2, .. }
    ));
    let outcome = manager.on_event(&event).await.unwrap();
    let report = outcome
        .reconciled
        .expect("reconnect triggers reconciliation");
    assert_eq!(report.changes.len(), 2, "{report:?}");
    assert!(report.missing.is_empty());
    assert_eq!(
        manager.order(&working).unwrap().order.status,
        OrderStatus::Filled
    );
    assert!(manager.order(&appeared).is_some());

    // A late, stale frame from before the gap cannot resurrect the order.
    let second = mock.stream_ids()[1].clone();
    mock.ws_send(
        &second,
        json!({"o": [{"oid": working, "a": ACCOUNT, "st": "PARTIALLY_FILLED", "fq": 2.0}]}),
    );
    let event = wait_for(&mut stream, "Data", |e| matches!(e, StreamEvent::Data(_))).await;
    assert!(manager.on_event(&event).await.unwrap().changes.is_empty());
    assert_eq!(
        manager.order(&working).unwrap().order.status,
        OrderStatus::Filled
    );
}

#[tokio::test]
async fn replayed_fill_snapshots_are_deduplicated() {
    let mock = Mock::start().await;
    let client = mock.client();
    let manager = OrderManager::new(client.clone(), ACCOUNT);
    let mut stream = Stream::connect(&client, fast()).await.unwrap();
    let first = connected(&mut stream, 1).await;

    let fill_1 = json!({"oid": "ACC1-1", "a": ACCOUNT, "fq": 1.0, "ftq": 1.0, "fp": 5000.0, "ouid": "100-1"});
    let fill_2 = json!({"oid": "ACC1-1", "a": ACCOUNT, "fq": 1.0, "ftq": 2.0, "fp": 5000.25, "ouid": "100-2"});
    mock.ws_send(&first, json!({"f": [fill_1.clone()]}));
    let event = wait_for(&mut stream, "Data", |e| matches!(e, StreamEvent::Data(_))).await;
    assert_eq!(manager.on_event(&event).await.unwrap().fills.len(), 1);

    // After a reconnect the server replays its fill snapshot plus one new fill.
    mock.ws_drop(&first);
    let second = connected(&mut stream, 2).await;
    mock.ws_send(&second, json!({"f": [fill_1, fill_2]}));
    let event = wait_for(&mut stream, "Data", |e| matches!(e, StreamEvent::Data(_))).await;
    let fills = manager.on_event(&event).await.unwrap().fills;
    assert_eq!(fills.len(), 1, "only the new fill is reported");
    assert_eq!(fills[0].order_update_id.as_deref(), Some("100-2"));
}

#[tokio::test]
async fn unknown_order_outcome_is_resolved_from_stream_fed_state() {
    let mock = Mock::start().await;
    let client = mock.client();
    let manager = OrderManager::new(client.clone(), ACCOUNT);
    let mut stream = Stream::connect(&client, fast()).await.unwrap();
    let event = wait_for(&mut stream, "Connected", |e| {
        matches!(e, StreamEvent::Connected { .. })
    })
    .await;
    manager.on_event(&event).await.unwrap();

    let request = OrderRequest::limit("XCME:ES.Z26", OrderSide::Buy, 2.0, 5000.25);
    mock.fault("placeOrder", Fault::AbortBody);
    let Error::OrderOutcomeUnknown(unknown) = manager.submit(request).await.unwrap_err() else {
        panic!("expected an unknown outcome");
    };
    // The order did go through and the stream announces it.
    let id = mock.stream_ids()[0].clone();
    mock.ws_send(
        &id,
        json!({"o": [{"oid": "ACC1-1", "sid": 1001, "a": ACCOUNT, "s": "XCME:ES.Z26", "st": "NEW",
                      "sd": "BUY", "q": 2.0, "lp": 5000.25, "ot": "2", "dr": "0"}]}),
    );
    let event = wait_for(&mut stream, "Data", |e| matches!(e, StreamEvent::Data(_))).await;
    manager.on_event(&event).await.unwrap();

    match manager.investigate(&unknown).await.unwrap() {
        ironbeam_sdk::orders::Investigation::Found(order) => assert_eq!(order.order_id, "ACC1-1"),
        other => panic!("{other:?}"),
    }
    assert_eq!(mock.count("POST", "/place"), 1);
}
