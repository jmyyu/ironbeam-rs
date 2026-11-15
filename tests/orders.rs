//! Order entry safety: orders are never retried, ambiguous submissions become
//! explicit unknown outcomes, and unknown outcomes can be investigated.

mod support;

use std::sync::atomic::Ordering;
use std::time::Duration;

use serde_json::json;

use ironbeam_sdk::models::{OrderDuration, OrderRequest, OrderSide, OrderStatus, OrderUpdate};
use ironbeam_sdk::orders::{Investigation, OrderManager};
use ironbeam_sdk::{Client, ClientConfig, Delivery, EnumEncoding, Environment, Error, RetryPolicy};
use support::mock::{ACCOUNT, Fault, Mock, spawn_reset_server};

fn limit_order() -> OrderRequest {
    OrderRequest::limit("XCME:ES.Z26", OrderSide::Buy, 2.0, 5000.25)
}

fn places(mock: &Mock) -> usize {
    mock.count("POST", "/place")
}

#[tokio::test]
async fn accepted_order_returns_ids_and_sends_the_documented_body() {
    let mock = Mock::start().await;
    let client = mock.client();
    let ids = client.place_order(ACCOUNT, &limit_order()).await.unwrap();
    assert_eq!(ids.order_id.as_deref(), Some("ACC1-1"));
    assert_eq!(ids.strategy_id, Some(1001));

    let sent = mock.requests("POST", "/place");
    assert_eq!(sent.len(), 1);
    assert_eq!(
        sent[0].path, "/order/ACC1/place",
        "path below the /v2 prefix"
    );
    assert_eq!(
        sent[0].body,
        Some(json!({
            "accountId": "ACC1", "exchSym": "XCME:ES.Z26", "side": "BUY", "quantity": 2.0,
            "orderType": "2", "duration": "0", "limitPrice": 5000.25
        }))
    );
}

#[tokio::test]
async fn names_dialect_is_configurable() {
    let mock = Mock::start().await;
    let mut config = mock.config();
    config.enum_encoding = EnumEncoding::Names;
    let client = mock.client_with(config);
    let order = limit_order().duration(OrderDuration::GoodTillCancel);
    client.place_order(ACCOUNT, &order).await.unwrap();
    let body = mock.requests("POST", "/place")[0].body.clone().unwrap();
    assert_eq!(body["orderType"], "LIMIT");
    assert_eq!(body["duration"], "GOOD_TILL_CANCEL");
}

#[tokio::test]
async fn invalid_orders_never_leave_the_process() {
    let mock = Mock::start().await;
    let client = mock.client();
    let mut order = limit_order();
    order.quantity = -1.0;
    assert!(matches!(
        client.place_order(ACCOUNT, &order).await,
        Err(Error::InvalidRequest(_))
    ));
    assert_eq!(places(&mock), 0);
    assert_eq!(mock.auth_count(), 0);
}

#[tokio::test]
async fn rejection_is_definitive_and_not_retried() {
    let mock = Mock::start().await;
    let client = mock.client();
    for status in [400u16, 403, 406] {
        mock.fault(
            "placeOrder",
            Fault::Status(status, json!({"error": "rejected"})),
        );
        let before = places(&mock);
        let err = client
            .place_order(ACCOUNT, &limit_order())
            .await
            .unwrap_err();
        assert!(
            matches!(&err, Error::Api { status: s, .. } if *s == status),
            "{err:?}"
        );
        assert!(!err.is_ambiguous());
        assert_eq!(places(&mock) - before, 1);
    }
    assert!(mock.orders().is_empty());
}

#[tokio::test]
async fn rate_limited_order_is_definitive_and_not_retried() {
    let mock = Mock::start().await;
    let client = mock.client();
    mock.fault("placeOrder", Fault::RetryAfter(429, 1));
    let err = client
        .place_order(ACCOUNT, &limit_order())
        .await
        .unwrap_err();
    assert!(matches!(err, Error::RateLimited { .. }), "{err:?}");
    assert_eq!(
        places(&mock),
        1,
        "a throttled order must not be resubmitted automatically"
    );
    assert!(mock.orders().is_empty());
}

#[tokio::test]
async fn server_error_is_an_unknown_outcome_and_never_retried() {
    let mock = Mock::start().await;
    let client = mock.client();
    for status in [500u16, 502, 503, 504] {
        mock.fault(
            "placeOrder",
            Fault::Status(status, json!({"error": "boom"})),
        );
        let before = places(&mock);
        match client
            .place_order(ACCOUNT, &limit_order())
            .await
            .unwrap_err()
        {
            Error::OrderOutcomeUnknown(unknown) => {
                assert_eq!(unknown.account_id, ACCOUNT);
                assert_eq!(unknown.request, limit_order());
                assert_eq!(unknown.cause.status(), Some(status));
            }
            other => panic!("{status}: {other:?}"),
        }
        assert_eq!(
            places(&mock) - before,
            1,
            "HTTP {status} must not trigger a resubmission"
        );
    }
}

#[tokio::test]
async fn timeout_is_an_unknown_outcome_and_the_order_may_exist() {
    let mock = Mock::start().await;
    let client = mock.client();
    // The server accepts the order but answers after the client gave up.
    mock.fault(
        "placeOrder",
        Fault::SlowResponse(Duration::from_millis(900)),
    );
    let err = client
        .place_order(ACCOUNT, &limit_order())
        .await
        .unwrap_err();
    match &err {
        Error::OrderOutcomeUnknown(unknown) => {
            assert!(
                matches!(
                    unknown.cause,
                    Error::Transport {
                        delivery: Delivery::MaybeSent,
                        timed_out: true,
                        ..
                    }
                ),
                "{:?}",
                unknown.cause
            );
        }
        other => panic!("{other:?}"),
    }
    assert!(err.is_ambiguous());
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert_eq!(places(&mock), 1);
    assert_eq!(
        mock.orders().len(),
        1,
        "the order went through although the client saw a timeout"
    );
}

#[tokio::test]
async fn lost_response_is_an_unknown_outcome() {
    let mock = Mock::start().await;
    let client = mock.client();
    mock.fault("placeOrder", Fault::AbortBody);
    let err = client
        .place_order(ACCOUNT, &limit_order())
        .await
        .unwrap_err();
    assert!(matches!(err, Error::OrderOutcomeUnknown(_)), "{err:?}");
    assert_eq!(places(&mock), 1);
    assert_eq!(mock.orders().len(), 1);
}

#[tokio::test]
async fn unreadable_success_response_is_an_unknown_outcome() {
    let mock = Mock::start().await;
    let client = mock.client();
    mock.fault("placeOrder", Fault::Garbage("OK"));
    match client
        .place_order(ACCOUNT, &limit_order())
        .await
        .unwrap_err()
    {
        Error::OrderOutcomeUnknown(unknown) => {
            assert!(matches!(unknown.cause, Error::Decode { .. }))
        }
        other => panic!("{other:?}"),
    }
    assert_eq!(places(&mock), 1);
}

#[tokio::test]
async fn connection_reset_mid_request_is_unknown_and_sent_exactly_once() {
    let (url, hits) = spawn_reset_server().await;
    let config = ClientConfig {
        environment: Environment::custom(&url).unwrap(),
        // Even an aggressive retry policy must not apply to orders.
        retry: RetryPolicy {
            max_retries: 5,
            base_delay: Duration::from_millis(1),
            ..RetryPolicy::none()
        },
        ..ClientConfig::default()
    };
    let client = Client::builder().config(config).token("t").build().unwrap();
    let err = client
        .place_order(ACCOUNT, &limit_order())
        .await
        .unwrap_err();
    match err {
        Error::OrderOutcomeUnknown(unknown) => assert!(matches!(
            unknown.cause,
            Error::Transport {
                delivery: Delivery::MaybeSent,
                ..
            }
        )),
        other => panic!("{other:?}"),
    }
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert_eq!(
        hits.load(Ordering::SeqCst),
        1,
        "the order request must hit the wire exactly once"
    );
}

#[tokio::test]
async fn connection_refused_is_definitive_because_nothing_was_sent() {
    let port = {
        let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        l.local_addr().unwrap().port()
    };
    let config = ClientConfig {
        environment: Environment::custom(&format!("http://127.0.0.1:{port}/v2")).unwrap(),
        ..ClientConfig::default()
    };
    let client = Client::builder().config(config).token("t").build().unwrap();
    let err = client
        .place_order(ACCOUNT, &limit_order())
        .await
        .unwrap_err();
    assert!(
        matches!(
            err,
            Error::Transport {
                delivery: Delivery::NotSent,
                ..
            }
        ),
        "an unsent order is a definite failure, not an unknown outcome: {err:?}"
    );
    assert!(!err.is_ambiguous());
}

#[tokio::test]
async fn expired_token_on_order_is_renewed_because_401_means_not_processed() {
    let mock = Mock::start().await;
    let client = mock.client();
    client.authenticate().await.unwrap();
    mock.expire_tokens();
    let ids = client.place_order(ACCOUNT, &limit_order()).await.unwrap();
    assert_eq!(ids.order_id.as_deref(), Some("ACC1-1"));
    assert_eq!(places(&mock), 2, "one refused with 401, one accepted");
    assert_eq!(mock.orders().len(), 1, "exactly one order exists");
}

#[tokio::test]
async fn cancel_and_update_are_never_retried() {
    let mock = Mock::start().await;
    let client = mock.client();
    let id = mock.add_order(json!({}));

    mock.fault(
        "cancelOrder",
        Fault::Status(503, json!({"error": "unavailable"})),
    );
    let err = client.cancel_order(ACCOUNT, &id).await.unwrap_err();
    assert!(err.is_ambiguous());
    assert_eq!(mock.count("DELETE", "/cancel/"), 1);

    mock.fault(
        "updateOrder",
        Fault::Status(503, json!({"error": "unavailable"})),
    );
    let err = client
        .update_order(ACCOUNT, &id, &OrderUpdate::new(3))
        .await
        .unwrap_err();
    assert!(err.is_ambiguous());
    assert_eq!(mock.count("PUT", "/update/"), 1);

    mock.fault(
        "cancelMultipleOrders",
        Fault::Status(503, json!({"error": "unavailable"})),
    );
    assert!(client.cancel_orders(ACCOUNT, &[id.as_str()]).await.is_err());
    assert_eq!(mock.count("DELETE", "/cancelMultiple"), 1);
}

#[tokio::test]
async fn update_and_cancel_round_trip() {
    let mock = Mock::start().await;
    let client = mock.client();
    let id = mock.add_order(json!({"quantity": 2.0}));

    let update = OrderUpdate::new(3).limit_price(5001.5);
    let updated = client.update_order(ACCOUNT, &id, &update).await.unwrap();
    assert_eq!(updated.orders[0].quantity, Some(3.0));
    assert_eq!(updated.orders[0].limit_price, Some(5001.5));
    let sent = mock.requests("PUT", "/update/")[0].body.clone().unwrap();
    assert_eq!(
        sent,
        json!({"orderId": id, "quantity": 3, "limitPrice": 5001.5})
    );

    let cancelled = client.cancel_order(ACCOUNT, &id).await.unwrap();
    assert_eq!(cancelled.orders[0].status, OrderStatus::Cancelled);
    let err = client.cancel_order(ACCOUNT, &id).await.unwrap_err();
    assert_eq!(err.status(), Some(400));

    let a = mock.add_order(json!({}));
    let b = mock.add_order(json!({}));
    let both = client
        .cancel_orders(ACCOUNT, &[a.as_str(), b.as_str()])
        .await
        .unwrap();
    assert_eq!(both.orders.len(), 2);
    let sent = mock.requests("DELETE", "/cancelMultiple")[0]
        .body
        .clone()
        .unwrap();
    assert_eq!(sent, json!({"accountId": ACCOUNT, "orderIds": [a, b]}));
}

// ---- OrderManager ---------------------------------------------------------

#[tokio::test]
async fn manager_takes_a_baseline_before_the_first_order() {
    let mock = Mock::start().await;
    mock.add_order(json!({}));
    let manager = OrderManager::new(mock.client(), ACCOUNT);
    let accepted = manager.submit(limit_order()).await.unwrap();
    assert_eq!(accepted.order_id.as_deref(), Some("ACC1-2"));
    manager.submit(limit_order()).await.unwrap();
    assert_eq!(
        mock.count("GET", "/order/ACC1/ANY"),
        1,
        "baseline is taken once"
    );
    assert_eq!(
        manager.open_orders().len(),
        1,
        "the pre-existing order is tracked"
    );
}

#[tokio::test]
async fn manager_does_not_submit_when_the_baseline_cannot_be_taken() {
    let mock = Mock::start().await;
    let manager = OrderManager::new(mock.client(), ACCOUNT);
    for _ in 0..4 {
        mock.fault("getOrders", Fault::Status(503, json!({"error": "down"})));
    }
    let err = manager.submit(limit_order()).await.unwrap_err();
    assert!(!matches!(err, Error::OrderOutcomeUnknown(_)), "{err:?}");
    assert_eq!(places(&mock), 0, "no order may be sent without a baseline");
}

#[tokio::test]
async fn unknown_outcome_is_found_when_the_order_went_through() {
    let mock = Mock::start().await;
    let manager = OrderManager::new(mock.client(), ACCOUNT);
    mock.fault(
        "placeOrder",
        Fault::SlowResponse(Duration::from_millis(900)),
    );
    let Error::OrderOutcomeUnknown(unknown) = manager.submit(limit_order()).await.unwrap_err()
    else {
        panic!("expected an unknown outcome");
    };
    match manager.investigate(&unknown).await.unwrap() {
        Investigation::Found(order) => {
            assert_eq!(order.order_id, "ACC1-1");
            assert_eq!(order.status, OrderStatus::New);
        }
        other => panic!("{other:?}"),
    }
    assert_eq!(places(&mock), 1, "investigation never resubmits");
    // The order is now attributed; it cannot be matched to another outcome.
    assert_eq!(
        manager.investigate(&unknown).await.unwrap(),
        Investigation::NotFound
    );
}

#[tokio::test]
async fn unknown_outcome_is_not_found_when_the_order_never_arrived() {
    let mock = Mock::start().await;
    let manager = OrderManager::new(mock.client(), ACCOUNT);
    mock.fault(
        "placeOrder",
        Fault::Status(502, json!({"error": "bad gateway"})),
    );
    let Error::OrderOutcomeUnknown(unknown) = manager.submit(limit_order()).await.unwrap_err()
    else {
        panic!("expected an unknown outcome");
    };
    assert_eq!(
        manager.investigate(&unknown).await.unwrap(),
        Investigation::NotFound
    );
    let again = manager
        .investigate_until_found(&unknown, 3, Duration::from_millis(5))
        .await
        .unwrap();
    assert_eq!(again, Investigation::NotFound);
    assert_eq!(places(&mock), 1);
    assert!(mock.orders().is_empty());
}

#[tokio::test]
async fn investigation_ignores_older_and_attributed_look_alikes() {
    let mock = Mock::start().await;
    // An identical order that existed before the manager started.
    mock.add_order(json!({"quantity": 2.0, "limitPrice": 5000.25}));
    let manager = OrderManager::new(mock.client(), ACCOUNT);
    // An identical order this manager placed successfully.
    manager.submit(limit_order()).await.unwrap();

    mock.fault("placeOrder", Fault::Status(500, json!({"error": "boom"})));
    let Error::OrderOutcomeUnknown(unknown) = manager.submit(limit_order()).await.unwrap_err()
    else {
        panic!("expected an unknown outcome");
    };
    assert_eq!(
        manager.investigate(&unknown).await.unwrap(),
        Investigation::NotFound,
        "neither the pre-existing nor the successfully placed twin may be mistaken for it"
    );
}

#[tokio::test]
async fn investigation_distinguishes_different_orders() {
    let mock = Mock::start().await;
    let manager = OrderManager::new(mock.client(), ACCOUNT);
    mock.fault("placeOrder", Fault::Status(500, json!({"error": "boom"})));
    let Error::OrderOutcomeUnknown(unknown) = manager.submit(limit_order()).await.unwrap_err()
    else {
        panic!("expected an unknown outcome");
    };
    // Someone else places a different order on the account afterwards.
    mock.add_order(json!({"side": "SELL", "quantity": 2.0, "limitPrice": 5000.25}));
    mock.add_order(json!({"quantity": 2.0, "limitPrice": 4999.0}));
    assert_eq!(
        manager.investigate(&unknown).await.unwrap(),
        Investigation::NotFound
    );
}

#[tokio::test]
async fn two_identical_unknown_outcomes_stay_ambiguous() {
    let mock = Mock::start().await;
    let manager = OrderManager::new(mock.client(), ACCOUNT);
    let mut unknowns = Vec::new();
    for _ in 0..2 {
        mock.fault("placeOrder", Fault::AbortBody);
        let Error::OrderOutcomeUnknown(u) = manager.submit(limit_order()).await.unwrap_err() else {
            panic!("expected an unknown outcome");
        };
        unknowns.push(u);
    }
    assert_eq!(mock.orders().len(), 2);
    // Nothing told the tracker about either order between the submissions, so
    // each outcome has two indistinguishable candidates. The SDK must say so
    // rather than guess.
    for unknown in &unknowns {
        match manager.investigate(unknown).await.unwrap() {
            Investigation::Ambiguous(candidates) => assert_eq!(candidates.len(), 2),
            other => panic!("{other:?}"),
        }
    }
    assert_eq!(places(&mock), 2);
}

#[tokio::test]
async fn unknown_outcomes_are_told_apart_when_state_was_observed_in_between() {
    let mock = Mock::start().await;
    let manager = OrderManager::new(mock.client(), ACCOUNT);
    mock.fault("placeOrder", Fault::AbortBody);
    let Error::OrderOutcomeUnknown(first) = manager.submit(limit_order()).await.unwrap_err() else {
        panic!("expected an unknown outcome");
    };
    // The tracker learns about the first order (as it would from the stream)
    // before the second identical submission is made.
    manager.reconcile().await.unwrap();
    mock.fault("placeOrder", Fault::AbortBody);
    let Error::OrderOutcomeUnknown(second) = manager.submit(limit_order()).await.unwrap_err()
    else {
        panic!("expected an unknown outcome");
    };

    // Only one order is newer than the second submission.
    match manager.investigate(&second).await.unwrap() {
        Investigation::Found(order) => assert_eq!(order.order_id, "ACC1-2"),
        other => panic!("{other:?}"),
    }
    // With that one attributed, the first is unambiguous too.
    match manager.investigate(&first).await.unwrap() {
        Investigation::Found(order) => assert_eq!(order.order_id, "ACC1-1"),
        other => panic!("{other:?}"),
    }
}

#[tokio::test]
async fn investigating_another_accounts_outcome_is_refused() {
    let mock = Mock::start().await;
    let manager = OrderManager::new(mock.client(), ACCOUNT);
    let other = OrderManager::new(mock.client(), "OTHER");
    mock.fault("placeOrder", Fault::Status(500, json!({})));
    let Error::OrderOutcomeUnknown(unknown) = manager.submit(limit_order()).await.unwrap_err()
    else {
        panic!("expected an unknown outcome");
    };
    assert!(matches!(
        other.investigate(&unknown).await,
        Err(Error::InvalidRequest(_))
    ));
}

#[tokio::test]
async fn reconcile_reports_missed_changes_and_vanished_orders() {
    let mock = Mock::start().await;
    let manager = OrderManager::new(mock.client(), ACCOUNT);
    let filled = mock.add_order(json!({}));
    let vanished = mock.add_order(json!({}));
    let report = manager.reconcile().await.unwrap();
    assert_eq!(report.changes.len(), 2);
    assert_eq!(manager.open_orders().len(), 2);

    // While nobody was listening: one order filled, one disappeared, one appeared.
    mock.patch_order(
        &filled,
        json!({"status": "FILLED", "fillQuantity": 1.0, "fillPrice": 5000.0}),
    );
    mock.remove_order(&vanished);
    let appeared = mock.add_order(json!({"side": "SELL"}));

    let report = manager.reconcile().await.unwrap();
    assert_eq!(report.changes.len(), 2, "{report:?}");
    assert_eq!(report.missing, vec![vanished.clone()]);
    assert_eq!(
        manager.order(&filled).unwrap().order.status,
        OrderStatus::Filled
    );
    assert!(manager.order(&vanished).unwrap().unconfirmed);
    assert!(manager.order(&appeared).is_some());
}

#[tokio::test]
async fn cancel_all_open_cancels_only_what_is_open() {
    let mock = Mock::start().await;
    let manager = OrderManager::new(mock.client(), ACCOUNT);
    assert!(manager.cancel_all_open().await.unwrap().is_none());

    let a = mock.add_order(json!({}));
    let b = mock.add_order(json!({"status": "PARTIALLY_FILLED", "fillQuantity": 0.5}));
    mock.add_order(json!({"status": "FILLED", "fillQuantity": 1.0}));
    let response = manager.cancel_all_open().await.unwrap().unwrap();
    assert_eq!(response.orders.len(), 2);
    let mut sent: Vec<String> = mock.requests("DELETE", "/cancelMultiple")[0]
        .body
        .clone()
        .unwrap()["orderIds"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap().to_owned())
        .collect();
    sent.sort();
    assert_eq!(sent, vec![a, b]);
    assert!(manager.open_orders().is_empty());
}

#[tokio::test]
async fn order_id_is_resolved_later_when_not_waited_for() {
    let mock = Mock::start().await;
    let manager = OrderManager::new(mock.client(), ACCOUNT);
    let accepted = manager
        .submit(limit_order().wait_for_order_id(false))
        .await
        .unwrap();
    assert_eq!(accepted.order_id, None);
    let strategy_id = accepted.strategy_id.unwrap();

    // Unknown at first (404), then known.
    mock.fault(
        "getToOrderId",
        Fault::Status(404, json!({"error": "not yet"})),
    );
    let id = manager
        .resolve_order_id(strategy_id, 3, Duration::from_millis(5))
        .await
        .unwrap();
    assert_eq!(id.as_deref(), Some("ACC1-1"));
    assert_eq!(mock.count("GET", "/toorderid/"), 2);

    assert_eq!(
        manager
            .resolve_order_id(999_999, 2, Duration::from_millis(1))
            .await
            .unwrap(),
        None
    );
}
