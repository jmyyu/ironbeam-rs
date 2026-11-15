//! REST client behaviour against the mock server: authentication, token
//! renewal, retry classification and error mapping.

mod support;

use std::time::{Duration, Instant};

use serde_json::json;

use ironbeam_sdk::models::{BalanceType, OrderStatus};
use ironbeam_sdk::{Client, Credentials, Delivery, Environment, Error, RetryPolicy};
use support::mock::{ACCOUNT, API_KEY, Fault, Mock, USER, spawn_reset_server};

#[tokio::test]
async fn authenticates_lazily_and_sends_bearer_token() {
    let mock = Mock::start().await;
    let client = mock.client();
    assert_eq!(mock.auth_count(), 0, "building a client performs no I/O");

    let accounts = client.all_accounts().await.unwrap();
    assert_eq!(accounts.accounts, vec![ACCOUNT.to_owned()]);
    client.all_accounts().await.unwrap();
    assert_eq!(mock.auth_count(), 1, "the token is reused");

    let auth = &mock.requests("POST", "/auth")[0];
    assert_eq!(
        auth.body,
        Some(json!({"username": USER, "apikey": API_KEY}))
    );
    assert!(auth.bearer.is_none());
    for request in mock.requests("GET", "/account/getAllAccounts") {
        assert_eq!(request.bearer.as_deref(), Some("tok-1"));
    }
}

#[tokio::test]
async fn wrong_credentials_fail_without_retry() {
    let mock = Mock::start().await;
    let client = Client::builder()
        .config(mock.config())
        .credentials(Credentials::api_key(USER, "wrong"))
        .build()
        .unwrap();
    let err = client.all_accounts().await.unwrap_err();
    assert_eq!(err.status(), Some(401), "{err}");
    assert!(err.is_auth());
    assert!(matches!(&err, Error::Api { message: Some(m), .. } if m.contains("invalid username")));
    assert_eq!(mock.auth_count(), 1);
    assert_eq!(mock.count("GET", "/account"), 0);
}

#[tokio::test]
async fn missing_credentials_is_a_local_error() {
    let mock = Mock::start().await;
    let client = Client::builder().config(mock.config()).build().unwrap();
    assert!(matches!(
        client.all_accounts().await,
        Err(Error::MissingCredentials)
    ));
    assert_eq!(mock.count("GET", ""), 0);
    assert_eq!(mock.count("POST", ""), 0);
}

#[tokio::test]
async fn expired_token_is_renewed_once_and_the_request_replayed() {
    let mock = Mock::start().await;
    let client = mock.client();
    client.all_accounts().await.unwrap();
    mock.expire_tokens();

    client.all_accounts().await.unwrap();
    assert_eq!(mock.auth_count(), 2);
    let bearers: Vec<_> = mock
        .requests("GET", "/account/getAllAccounts")
        .into_iter()
        .map(|r| r.bearer.unwrap())
        .collect();
    assert_eq!(bearers, ["tok-1", "tok-1", "tok-2"]);
}

#[tokio::test]
async fn persistent_401_is_returned_after_one_renewal() {
    let mock = Mock::start().await;
    let client = mock.client();
    // The data endpoint keeps answering 401 even with a fresh token.
    for _ in 0..5 {
        mock.fault(
            "allAccounts",
            Fault::Status(401, json!({"error": "Unauthorized"})),
        );
    }
    let err = client.all_accounts().await.unwrap_err();
    assert_eq!(err.status(), Some(401));
    assert_eq!(mock.auth_count(), 2, "exactly one renewal");
    assert_eq!(
        mock.count("GET", "/account/getAllAccounts"),
        2,
        "401 is not retried in a loop"
    );
}

#[tokio::test]
async fn token_without_credentials_cannot_be_renewed() {
    let mock = Mock::start().await;
    let client = Client::builder()
        .config(mock.config())
        .token("stale")
        .build()
        .unwrap();
    let err = client.all_accounts().await.unwrap_err();
    assert_eq!(err.status(), Some(401));
    assert_eq!(mock.auth_count(), 0);
    assert_eq!(mock.count("GET", "/account/getAllAccounts"), 1);
}

#[tokio::test]
async fn concurrent_requests_share_one_token_renewal() {
    let mock = Mock::start().await;
    let client = mock.client();
    client.all_accounts().await.unwrap();
    mock.expire_tokens();

    let tasks: Vec<_> = (0..16)
        .map(|_| {
            let c = client.clone();
            tokio::spawn(async move { c.all_accounts().await })
        })
        .collect();
    for task in tasks {
        task.await.unwrap().unwrap();
    }
    assert_eq!(
        mock.auth_count(),
        2,
        "16 concurrent 401s must trigger a single re-authentication"
    );
}

#[tokio::test]
async fn reads_are_retried_on_5xx_then_succeed() {
    let mock = Mock::start().await;
    let client = mock.client();
    mock.fault(
        "allAccounts",
        Fault::Status(503, json!({"error": "unavailable"})),
    );
    mock.fault("allAccounts", Fault::Status(500, json!({"error": "boom"})));
    client.all_accounts().await.unwrap();
    assert_eq!(mock.count("GET", "/account/getAllAccounts"), 3);
}

#[tokio::test]
async fn reads_give_up_after_max_retries() {
    let mock = Mock::start().await;
    let client = mock.client();
    for _ in 0..10 {
        mock.fault(
            "allAccounts",
            Fault::Status(503, json!({"message": "down for maintenance"})),
        );
    }
    let err = client.all_accounts().await.unwrap_err();
    assert!(
        matches!(&err, Error::Api { status: 503, message: Some(m), .. } if m == "down for maintenance"),
        "{err:?}"
    );
    assert!(err.is_transient() && err.is_ambiguous());
    assert_eq!(
        mock.count("GET", "/account/getAllAccounts"),
        4,
        "1 attempt + 3 retries"
    );
}

#[tokio::test]
async fn retry_can_be_disabled() {
    let mock = Mock::start().await;
    let mut config = mock.config();
    config.retry = RetryPolicy::none();
    let client = mock.client_with(config);
    mock.fault("allAccounts", Fault::Status(503, json!({})));
    assert!(client.all_accounts().await.is_err());
    assert_eq!(mock.count("GET", "/account/getAllAccounts"), 1);
}

#[tokio::test]
async fn client_errors_are_not_retried() {
    let mock = Mock::start().await;
    let client = mock.client();
    for status in [400u16, 403, 404, 406] {
        mock.fault(
            "allAccounts",
            Fault::Status(status, json!({"error": "nope", "message": "ignored"})),
        );
        let before = mock.count("GET", "/account/getAllAccounts");
        let err = client.all_accounts().await.unwrap_err();
        assert_eq!(err.status(), Some(status));
        assert!(matches!(&err, Error::Api { message: Some(m), .. } if m == "nope"));
        assert!(!err.is_transient() && !err.is_ambiguous());
        assert_eq!(mock.count("GET", "/account/getAllAccounts") - before, 1);
    }
}

#[tokio::test]
async fn rate_limit_honours_retry_after_for_reads() {
    let mock = Mock::start().await;
    let mut config = mock.config();
    config.retry.max_delay = Duration::from_secs(2);
    let client = mock.client_with(config);
    mock.fault("allAccounts", Fault::RetryAfter(429, 1));
    let started = Instant::now();
    client.all_accounts().await.unwrap();
    assert!(
        started.elapsed() >= Duration::from_millis(950),
        "waited {:?}",
        started.elapsed()
    );
    assert_eq!(mock.count("GET", "/account/getAllAccounts"), 2);
}

#[tokio::test]
async fn rate_limit_error_is_typed_when_retries_are_exhausted() {
    let mock = Mock::start().await;
    let mut config = mock.config();
    config.retry = RetryPolicy::none();
    let client = mock.client_with(config);
    mock.fault("allAccounts", Fault::RetryAfter(429, 7));
    match client.all_accounts().await.unwrap_err() {
        Error::RateLimited { retry_after, .. } => {
            assert_eq!(retry_after, Some(Duration::from_secs(7)))
        }
        other => panic!("{other:?}"),
    }
}

#[tokio::test]
async fn error_status_in_a_200_envelope_is_an_error() {
    let mock = Mock::start().await;
    let client = mock.client();
    mock.fault(
        "allAccounts",
        Fault::Status(200, json!({"status": "ERROR", "message": "account locked"})),
    );
    let err = client.all_accounts().await.unwrap_err();
    assert!(
        matches!(&err, Error::Api { status: 200, message: Some(m), .. } if m == "account locked"),
        "{err:?}"
    );
    // Other statuses are not failures.
    mock.fault(
        "allAccounts",
        Fault::Status(200, json!({"status": "WARNING", "accounts": ["A"]})),
    );
    assert_eq!(client.all_accounts().await.unwrap().accounts, ["A"]);
}

#[tokio::test]
async fn undecodable_success_body_is_a_decode_error() {
    let mock = Mock::start().await;
    let client = mock.client();
    mock.fault("allAccounts", Fault::Garbage("<html>gateway</html>"));
    let err = client.all_accounts().await.unwrap_err();
    assert!(
        matches!(&err, Error::Decode { body, .. } if body.contains("gateway")),
        "{err:?}"
    );
}

#[tokio::test]
async fn slow_reads_time_out_and_are_retried() {
    let mock = Mock::start().await;
    let mut config = mock.config();
    config.request_timeout = Duration::from_millis(100);
    config.retry.max_retries = 1;
    let client = mock.client_with(config);
    client.authenticate().await.unwrap();
    mock.fault(
        "allAccounts",
        Fault::SlowResponse(Duration::from_millis(500)),
    );
    mock.fault(
        "allAccounts",
        Fault::SlowResponse(Duration::from_millis(500)),
    );
    let err = client.all_accounts().await.unwrap_err();
    match err {
        Error::Transport {
            delivery,
            timed_out,
            ..
        } => {
            assert_eq!(delivery, Delivery::MaybeSent);
            assert!(timed_out);
        }
        other => panic!("{other:?}"),
    }
    assert_eq!(mock.count("GET", "/account/getAllAccounts"), 2);
}

#[tokio::test]
async fn connection_refused_is_reported_as_not_sent() {
    // Bind and drop a listener to obtain a port nothing listens on.
    let port = {
        let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        l.local_addr().unwrap().port()
    };
    let client = Client::builder()
        .environment(Environment::custom(&format!("http://127.0.0.1:{port}/v2")).unwrap())
        .token("t")
        .build()
        .unwrap();
    let mut config = client.config().clone();
    config.retry = RetryPolicy::none();
    let client = Client::builder().config(config).token("t").build().unwrap();
    match client.all_accounts().await.unwrap_err() {
        Error::Transport { delivery, .. } => assert_eq!(delivery, Delivery::NotSent),
        other => panic!("{other:?}"),
    }
}

#[tokio::test]
async fn connection_reset_after_send_is_reported_as_maybe_sent() {
    let (url, hits) = spawn_reset_server().await;
    let mut config = ironbeam_sdk::ClientConfig {
        environment: Environment::custom(&url).unwrap(),
        ..Default::default()
    };
    config.retry = RetryPolicy {
        max_retries: 2,
        base_delay: Duration::from_millis(5),
        ..RetryPolicy::none()
    };
    let client = Client::builder().config(config).token("t").build().unwrap();
    match client.all_accounts().await.unwrap_err() {
        Error::Transport {
            delivery,
            timed_out,
            ..
        } => {
            assert_eq!(delivery, Delivery::MaybeSent);
            assert!(!timed_out);
        }
        other => panic!("{other:?}"),
    }
    assert_eq!(
        hits.load(std::sync::atomic::Ordering::SeqCst),
        3,
        "a read is retried after a reset"
    );
}

#[tokio::test]
async fn logout_invalidates_and_forgets_the_token() {
    let mock = Mock::start().await;
    let client = mock.client();
    client.all_accounts().await.unwrap();
    assert!(client.token().is_some());
    client.logout().await.unwrap();
    assert!(client.token().is_none());
    assert_eq!(mock.count("POST", "/logout"), 1);
    // The next call logs in again.
    client.all_accounts().await.unwrap();
    assert_eq!(mock.auth_count(), 2);
    // Logging out twice does nothing the second time.
    client.logout().await.unwrap();
    client.logout().await.unwrap();
    assert_eq!(mock.count("POST", "/logout"), 2);
}

#[tokio::test]
async fn invalid_arguments_are_rejected_before_any_io() {
    let mock = Mock::start().await;
    let client = mock.client();
    let eleven: Vec<String> = (0..11).map(|i| format!("XCME:S{i}")).collect();
    let none: [&str; 0] = [];
    assert!(matches!(
        client.quotes(&eleven).await,
        Err(Error::InvalidRequest(_))
    ));
    assert!(matches!(
        client.quotes(&none).await,
        Err(Error::InvalidRequest(_))
    ));
    assert!(matches!(
        client.account_balance("", BalanceType::CurrentOpen).await,
        Err(Error::InvalidRequest(_))
    ));
    assert!(matches!(
        client.orders("..", OrderStatus::Any).await,
        Err(Error::InvalidRequest(_))
    ));
    assert!(matches!(
        client.trades("XCME:ES.Z26", 10, 5, 10, true).await,
        Err(Error::InvalidRequest(_))
    ));
    assert!(matches!(
        client.trades("XCME:ES.Z26", 1, 5, 101, true).await,
        Err(Error::InvalidRequest(_))
    ));
    assert!(matches!(
        client.symbols(Some("ab"), None, None).await,
        Err(Error::InvalidRequest(_))
    ));
    assert!(matches!(
        client.symbols(None, Some(0), None).await,
        Err(Error::InvalidRequest(_))
    ));
    assert_eq!(mock.count("GET", ""), 0);
    assert_eq!(mock.auth_count(), 0);
}

#[tokio::test]
async fn debug_output_never_contains_secrets() {
    let mock = Mock::start().await;
    let client = mock.client();
    client.all_accounts().await.unwrap();
    let dbg = format!("{client:?}");
    assert!(!dbg.contains(API_KEY) && !dbg.contains("tok-1"), "{dbg}");
}
