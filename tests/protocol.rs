//! Protocol validation against the locally downloaded OpenAPI document.
//!
//! * the SDK's endpoint table and the specification agree in both directions;
//! * every REST method, called against a server that validates requests
//!   against the specification, produces a conforming request (method, path,
//!   parameters, authentication, body) and decodes the schema-derived example
//!   response without losing a single documented property;
//! * the stream message model covers every documented stream field.

mod support;

use std::collections::BTreeSet;

use serde::Serialize;
use serde_json::{Value, json};

use ironbeam_sdk::models::*;
use ironbeam_sdk::rest::BarKind;
use ironbeam_sdk::{Client, ClientConfig, Credentials, EnumEncoding, Environment, endpoints};
use support::spec::{Spec, SpecServer, missing_keys};

#[test]
fn endpoint_table_matches_the_specification() {
    let spec = Spec::load();
    let documented: BTreeSet<(String, String, String)> = spec
        .operations()
        .into_iter()
        .map(|o| (o.method, o.path, o.id))
        .collect();
    let mut implemented: BTreeSet<(String, String, String)> = endpoints::ALL
        .iter()
        .map(|e| {
            (
                e.method.to_owned(),
                e.path.to_owned(),
                e.operation_id.to_owned(),
            )
        })
        .collect();
    assert_eq!(
        implemented.len(),
        endpoints::ALL.len(),
        "duplicate entries in the endpoint table"
    );
    let ws = endpoints::STREAM_SOCKET;
    implemented.insert((
        ws.method.to_owned(),
        ws.path.to_owned(),
        ws.operation_id.to_owned(),
    ));

    let unknown: Vec<_> = implemented.difference(&documented).collect();
    let uncovered: Vec<_> = documented.difference(&implemented).collect();
    assert!(
        unknown.is_empty(),
        "SDK endpoints not in the specification: {unknown:#?}"
    );
    assert!(
        uncovered.is_empty(),
        "specified operations the SDK lacks: {uncovered:#?}"
    );
}

#[test]
fn only_reads_and_auth_are_auto_retried() {
    for e in endpoints::ALL {
        if e.auto_retry {
            assert!(
                e.method == "GET" || e.operation_id == "authorize",
                "{} {} must not be auto-retried",
                e.method,
                e.path
            );
        }
    }
    for e in [
        endpoints::PLACE_ORDER,
        endpoints::UPDATE_ORDER,
        endpoints::CANCEL_ORDER,
        endpoints::CANCEL_MULTIPLE,
        endpoints::SIM_ADD_CASH,
        endpoints::SIM_LIQUIDATE,
    ] {
        assert!(
            !e.auto_retry,
            "{} must never be auto-retried",
            e.operation_id
        );
    }
}

#[test]
fn servers_match_the_built_in_environments() {
    let spec = Spec::load();
    let urls: Vec<&str> = spec.doc["servers"]
        .as_array()
        .unwrap()
        .iter()
        .map(|s| s["url"].as_str().unwrap())
        .collect();
    assert!(urls.contains(&Environment::Live.base_url().as_str()));
    assert!(urls.contains(&Environment::Demo.base_url().as_str()));
}

/// Asserts that the last request conformed to the specification and that the
/// decoded response kept every property of the example the server sent.
fn check<T: Serialize>(server: &SpecServer, operation_id: &str, decoded: &T) {
    let call = server.last_call();
    assert_eq!(
        call.operation_id, operation_id,
        "request was routed to the wrong operation"
    );
    assert!(
        call.problems.is_empty(),
        "{operation_id}: request violates the spec: {:#?}",
        call.problems
    );
    let reserialized = serde_json::to_value(decoded).unwrap();
    let mut missing = Vec::new();
    missing_keys(&call.response, &reserialized, "$", &mut missing);
    assert!(
        missing.is_empty(),
        "{operation_id}: documented response properties lost in decoding: {missing:#?}\nsent: {}\ngot: {}",
        call.response,
        reserialized
    );
}

async fn spec_client(server: &SpecServer, encoding: EnumEncoding) -> Client {
    let config = ClientConfig {
        environment: Environment::custom(&server.base_url()).unwrap(),
        enum_encoding: encoding,
        ..ClientConfig::default()
    };
    Client::builder()
        .config(config)
        .credentials(Credentials::enterprise("user", "pass", "key"))
        .build()
        .unwrap()
}

#[tokio::test]
#[allow(deprecated)]
async fn every_rest_operation_conforms_to_the_specification() {
    let server = SpecServer::start().await;
    let c = spec_client(&server, EnumEncoding::Codes).await;
    let syms = ["XCME:ES.Z26", "XCME:NQ.Z26"];
    let acct = "5123345";
    let stream = "123e4567-e89b-12d3-a456-426614174000";

    // Authentication (implicit authorize happens on the first call).
    c.authenticate().await.unwrap();
    let call = server.last_call();
    assert_eq!(call.operation_id, "authorize");
    assert!(call.problems.is_empty(), "{:#?}", call.problems);

    // Information.
    check(
        &server,
        "getTraderInfo",
        &c.trader_info(Some("trader_1")).await.unwrap(),
    );
    check(
        &server,
        "getUserGeneralInfo",
        &c.user_info(None).await.unwrap(),
    );
    check(
        &server,
        "getSecurityDefinitions",
        &c.security_definitions(&syms).await.unwrap(),
    );
    check(
        &server,
        "getSecurityMarginAndValue",
        &c.security_margin(&syms).await.unwrap(),
    );
    check(
        &server,
        "getSecurityStatus",
        &c.security_status(&syms).await.unwrap(),
    );
    check(
        &server,
        "getSymbols",
        &c.symbols(Some("ESZ"), Some(50), Some(true)).await.unwrap(),
    );
    check(
        &server,
        "getExchangeSources",
        &c.exchange_sources().await.unwrap(),
    );
    check(&server, "getComplexes", &c.complexes("XCME").await.unwrap());
    check(
        &server,
        "getSymbolFutures",
        &c.symbol_futures("XCME", "ES").await.unwrap(),
    );
    check(
        &server,
        "getSymbolGroupsByComplex",
        &c.symbol_groups("Currency").await.unwrap(),
    );
    check(
        &server,
        "getSymbolOptionGroups",
        &c.symbol_option_groups("XCME:ES.Z26").await.unwrap(),
    );
    check(
        &server,
        "getSearchSymbolOption",
        &c.symbol_options("XCME:ES.Z26", "ES", OptionRight::Call, true)
            .await
            .unwrap(),
    );
    check(
        &server,
        "getSearchForOptionSpreads",
        &c.symbol_option_spreads("XCME:ES.Z26").await.unwrap(),
    );
    check(&server, "getStrategyId", &c.strategy_id().await.unwrap());

    // Account.
    check(
        &server,
        "accountBalance",
        &c.account_balance(acct, BalanceType::CurrentOpen)
            .await
            .unwrap(),
    );
    check(
        &server,
        "positions",
        &c.account_positions(acct).await.unwrap(),
    );
    check(&server, "risk", &c.account_risk(acct).await.unwrap());
    check(&server, "fills", &c.account_fills(acct).await.unwrap());
    check(&server, "allAccounts", &c.all_accounts().await.unwrap());
    check(
        &server,
        "allAccountsBalance",
        &c.all_balances(BalanceType::StartOfDay).await.unwrap(),
    );
    check(&server, "allfills", &c.all_fills().await.unwrap());
    check(&server, "allpositions", &c.all_positions().await.unwrap());
    check(&server, "allaccountsriskinfo", &c.all_risk().await.unwrap());

    // Market data.
    check(&server, "getQuotes", &c.quotes(&syms).await.unwrap());
    check(&server, "getDepth", &c.depth(&syms).await.unwrap());
    check(
        &server,
        "getTrades",
        &c.trades(
            "XCME:ES.Z26",
            1_612_137_600_000,
            1_612_137_660_000,
            100,
            true,
        )
        .await
        .unwrap(),
    );

    // Orders.
    let bracket = OrderRequest::limit("XCME:ES.Z26", OrderSide::Buy, 2.0, 5000.25)
        .duration(OrderDuration::GoodTillCancel)
        .stop_loss(4990.0)
        .take_profit(5020.0)
        .wait_for_order_id(true);
    check(
        &server,
        "placeOrder",
        &c.place_order(acct, &bracket).await.unwrap(),
    );
    for order in [
        OrderRequest::market("XCME:ES.Z26", OrderSide::Sell, 1.0),
        OrderRequest::stop("XCME:ES.Z26", OrderSide::Sell, 1.0, 4980.0),
        OrderRequest::stop_limit("XCME:ES.Z26", OrderSide::Buy, 3.0, 5010.0, 5011.0)
            .stop_loss_offset(25.0)
            .take_profit_offset(0.5),
    ] {
        check(
            &server,
            "placeOrder",
            &c.place_order(acct, &order).await.unwrap(),
        );
    }
    check(
        &server,
        "getToOrderId",
        &c.order_id_for_strategy(acct, 1_234_567_890).await.unwrap(),
    );
    check(
        &server,
        "getToStrategyId",
        &c.strategy_id_for_order(acct, "5123345-99").await.unwrap(),
    );
    let update = OrderUpdate::new(3)
        .limit_price(5001.0)
        .stop_loss(4991.0)
        .take_profit(5021.0);
    check(
        &server,
        "updateOrder",
        &c.update_order(acct, "5123345-99", &update).await.unwrap(),
    );
    for status in [
        OrderStatus::Any,
        OrderStatus::New,
        OrderStatus::Filled,
        OrderStatus::Cancelled,
    ] {
        check(&server, "getOrders", &c.orders(acct, status).await.unwrap());
    }
    check(
        &server,
        "cancelOrder",
        &c.cancel_order(acct, "5123345-99").await.unwrap(),
    );
    check(
        &server,
        "cancelMultipleOrders",
        &c.cancel_orders(acct, &["1", "2"]).await.unwrap(),
    );
    check(
        &server,
        "getOrdersFills",
        &c.order_fills(acct).await.unwrap(),
    );

    // Simulated accounts.
    let trader = SimulatedTraderCreate {
        first_name: "John".into(),
        last_name: "Doe".into(),
        address1: "123 Main St".into(),
        address2: Some("Apt 4B".into()),
        city: "New York".into(),
        state: "NY".into(),
        country: "USA".into(),
        zip_code: "10001".into(),
        phone: "+1-555-555-5555".into(),
        email: "email@email.com".into(),
        password: "password123".into(),
        template_id: "EVAL50".into(),
    };
    check(
        &server,
        "simulatedTraderCreate",
        &c.simulated_trader_create(&trader).await.unwrap(),
    );
    let add = SimulatedAccountAdd {
        trader_id: "T1".into(),
        password: "password123".into(),
        template_id: "EVAL50".into(),
    };
    check(
        &server,
        "simulatedAccountAdd",
        &c.simulated_account_add(&add).await.unwrap(),
    );
    check(
        &server,
        "simulatedAccountReset",
        &c.simulated_account_reset("A1", "EVAL50").await.unwrap(),
    );
    check(
        &server,
        "simulatedAccountExpire",
        &c.simulated_account_expire("A1").await.unwrap(),
    );
    check(
        &server,
        "simulatedAccountAddCash",
        &c.simulated_account_add_cash("A1", 1000.0).await.unwrap(),
    );
    check(
        &server,
        "simulatedAccountGetCashReport",
        &c.simulated_account_cash_report("A1", 20250901, 20250910)
            .await
            .unwrap(),
    );
    check(
        &server,
        "simulatedAccountSetLiquidateOnly",
        &c.simulated_account_set_liquidate_only("A1", false, Some("EVAL50"))
            .await
            .unwrap(),
    );
    let liquidate = SimulatedAccountLiquidate {
        accounts: vec!["A1".into()],
        groups: vec!["703-230".into()],
        except_accounts: vec!["A2".into()],
        force_manual_liquidation: Some(false),
        use_manual_liquidation_for_illiquid_markets: Some(true),
        send_account_email: Some(false),
        send_office_email: Some(false),
    };
    check(
        &server,
        "simulatedAccountLiquidate",
        &c.simulated_account_liquidate(&liquidate).await.unwrap(),
    );
    let mut risk = SimulatedAccountSetRisk::new("A1");
    risk.liquidation_account_value = Setting::Set(90_000.0);
    risk.liquidation_loss_from_high_of_day = Setting::Clear;
    risk.liquidation_pct_loss_from_start_of_day = Setting::Set(20.0);
    risk.reduce_positions_only = Some(true);
    risk.margin_schedule_name = Setting::Clear;
    risk.template_id = Some("EVAL50".into());
    check(
        &server,
        "simulatedAccountSetRisk",
        &c.simulated_account_set_risk(&risk).await.unwrap(),
    );

    // Streaming control.
    let id = c.create_stream_id().await.unwrap();
    assert_eq!(server.last_call().operation_id, "getStreamId");
    assert!(server.last_call().problems.is_empty());
    assert_eq!(id, stream);
    check(
        &server,
        "getSubscribeQuotes",
        &c.subscribe_quotes(stream, &syms).await.unwrap(),
    );
    check(
        &server,
        "getSubscribeDepths",
        &c.subscribe_depths(stream, &syms).await.unwrap(),
    );
    check(
        &server,
        "getSubscribeTrades",
        &c.subscribe_trades(stream, &syms).await.unwrap(),
    );
    check(
        &server,
        "getUnsubscribeQuotes",
        &c.unsubscribe_quotes(stream, &syms).await.unwrap(),
    );
    check(
        &server,
        "UnsubscribeDepths",
        &c.unsubscribe_depths(stream, &syms).await.unwrap(),
    );
    check(
        &server,
        "getUnsubscribeTrades",
        &c.unsubscribe_trades(stream, &syms).await.unwrap(),
    );
    let bars = BarSubscription::new("XCME:ES.Z26", 5, BarType::Minute, 100);
    for (kind, op) in [
        (BarKind::Tick, "indicatorSubscribeTickBars"),
        (BarKind::Trade, "indicatorSubscribeTradeBars"),
        (BarKind::Time, "indicatorSubscribeTimeBars"),
        (BarKind::Volume, "indicatorSubscribeVolumeBars"),
    ] {
        check(
            &server,
            op,
            &c.subscribe_bars(stream, kind, &bars).await.unwrap(),
        );
    }
    check(
        &server,
        "indicatorUnsubscribe",
        &c.unsubscribe_indicator(stream, "TradeBars_638639418687259084")
            .await
            .unwrap(),
    );

    c.logout().await.unwrap();
    assert_eq!(server.last_call().operation_id, "logout");
    assert!(server.last_call().problems.is_empty());

    // Every documented REST operation was exercised, and none was malformed.
    let calls = server.calls();
    let bad: Vec<_> = calls.iter().filter(|c| !c.problems.is_empty()).collect();
    assert!(bad.is_empty(), "{bad:#?}");
    let exercised: BTreeSet<String> = calls.into_iter().map(|c| c.operation_id).collect();
    let documented: BTreeSet<String> = server
        .spec()
        .operations()
        .into_iter()
        .map(|o| o.id)
        .filter(|id| id != "getStream")
        .collect();
    assert_eq!(exercised, documented);
}

#[tokio::test]
async fn name_encoded_order_enums_are_rejected_by_the_schema() {
    // Documents discrepancy D-07: the *names* dialect, which the prose and
    // examples use, does not validate against the schema's enums.
    let server = SpecServer::start().await;
    let c = spec_client(&server, EnumEncoding::Names).await;
    let order = OrderRequest::limit("XCME:ES.Z26", OrderSide::Buy, 1.0, 5000.0);
    c.place_order("5123345", &order).await.unwrap();
    let problems = server.last_call().problems;
    assert_eq!(problems.len(), 2, "{problems:#?}");
    assert!(
        problems
            .iter()
            .any(|p| p.contains("orderType") && p.contains("LIMIT"))
    );
    assert!(
        problems
            .iter()
            .any(|p| p.contains("duration") && p.contains("DAY"))
    );
}

#[tokio::test]
async fn undocumented_extra_fields_are_flagged_by_the_schema() {
    let server = SpecServer::start().await;
    let c = spec_client(&server, EnumEncoding::Codes).await;
    let order = OrderRequest::market("XCME:ES.Z26", OrderSide::Buy, 1.0)
        .extra_field("clientOrderId", "abc-1");
    c.place_order("5123345", &order).await.unwrap();
    let problems = server.last_call().problems;
    assert_eq!(problems.len(), 1, "{problems:#?}");
    assert!(problems[0].contains("clientOrderId"));
}

fn non_null_fields(value: &Value) -> usize {
    value
        .as_object()
        .map(|o| o.values().filter(|v| !v.is_null()).count())
        .unwrap_or(0)
}

#[test]
fn stream_message_covers_every_documented_field() {
    let spec = Spec::load();
    let schema = spec.schema("StreamResponse");
    let example = spec.example(schema);
    let message: StreamMessage = serde_json::from_value(example.clone()).unwrap();
    assert!(
        message.extra.is_empty(),
        "undeclared stream fields: {:?}",
        message.extra.keys()
    );
    assert!(message.ping.is_some() && message.reset.is_some());

    // Each documented top-level field decodes to a non-empty collection whose
    // first element kept every property of the example.
    let decoded = json!({
        "q": message.quotes, "d": message.depths, "tr": message.trades, "tb": message.trade_bars,
        "tc": message.tick_bars, "ti": message.time_bars, "vb": message.volume_bars,
        "i": message.indicators, "o": message.orders, "f": message.fills, "ps": message.positions,
        "psa": message.positions_snapshot, "b": message.balances, "ba": message.balances_snapshot,
        "ri": message.risk, "ria": message.risk_snapshot,
    });
    let documented: BTreeSet<&String> = example.as_object().unwrap().keys().collect();
    for key in &documented {
        if ["p", "r"].contains(&key.as_str()) {
            continue;
        }
        let items = decoded[key.as_str()]
            .as_array()
            .unwrap_or_else(|| panic!("field {key} not modelled"));
        assert_eq!(items.len(), 1, "stream field {key} did not decode");
        let sent = match &example[key.as_str()] {
            Value::Array(a) => a[0].clone(),
            single => single.clone(),
        };
        assert_eq!(
            non_null_fields(&items[0]),
            sent.as_object().unwrap().len(),
            "stream field {key}: properties lost.\nsent: {sent}\ngot: {}",
            items[0]
        );
    }
}

#[test]
fn stream_shorthand_and_rest_shapes_decode_to_the_same_model() {
    let spec = Spec::load();
    for (long, short) in [
        ("Order", "OrderOpt"),
        ("OrderFill", "OrderFillOpt"),
        ("Position", "PositionOpt"),
        ("Balance", "BalanceOpt"),
    ] {
        let long_keys = spec.example(spec.schema(long)).as_object().unwrap().len();
        let short_keys = spec.example(spec.schema(short)).as_object().unwrap().len();
        assert_eq!(
            long_keys, short_keys,
            "{long} and {short} have different property counts"
        );
    }
    let order: Order = serde_json::from_value(spec.example(spec.schema("OrderOpt"))).unwrap();
    let rest: Order = serde_json::from_value(spec.example(spec.schema("Order"))).unwrap();
    assert_eq!(order.status, rest.status);
    assert_eq!(order.order_type, rest.order_type);
    assert_eq!(order.duration, rest.duration);
}

#[test]
fn every_enum_value_in_the_specification_is_recognised() {
    fn assert_known<T: std::fmt::Debug>(schema: &str, raw: &Value, parsed: T) {
        let dbg = format!("{parsed:?}");
        assert!(
            !dbg.starts_with("Unknown("),
            "{schema}: value {raw} is not recognised"
        );
    }
    let spec = Spec::load();
    let values = |name: &str| spec.schema(name)["enum"].as_array().unwrap().clone();
    macro_rules! covers {
        ($schema:literal => $ty:ty) => {
            for v in values($schema) {
                assert_known(
                    $schema,
                    &v,
                    serde_json::from_value::<$ty>(v.clone()).unwrap(),
                );
            }
        };
    }
    covers!("OrderStatusType" => OrderStatus);
    covers!("OrderSide" => OrderSide);
    covers!("OrderType" => OrderType);
    covers!("DurationType" => OrderDuration);
    covers!("BalanceType" => BalanceType);
    covers!("PositionSide" => PositionSide);
    covers!("Side" => Side);
    covers!("SideShort" => BookSide);
    covers!("AggressorSideType" => AggressorSide);
    covers!("TickDirectionType" => TickDirection);
    covers!("TickDirection" => TickDirection);
    covers!("SecurityStatusType" => SecurityStatus);
    covers!("OptionType" => OptionType);
    covers!("OptionExpirationType" => OptionExpirationType);
    covers!("SecurityType" => SecurityType);
    covers!("RegCodeType" => RegCode);
    covers!("BarType" => BarType);
    covers!("SystemPricedTrade" => SystemPricedTrade);
    covers!("InvestigationStatus" => InvestigationStatus);
    covers!("BlockTrade" => BlockTrade);
    covers!("ResponseStatus" => ResponseStatus);

    // Integer-coded enums must map codes to the documented names.
    let names = spec.schema("SecurityStatusType")["x-enumNames"]
        .as_array()
        .unwrap()
        .clone();
    for (code, name) in values("SecurityStatusType").into_iter().zip(names) {
        let from_code: SecurityStatus = serde_json::from_value(code).unwrap();
        let from_name: SecurityStatus = serde_json::from_value(name).unwrap();
        assert_eq!(from_code, from_name);
    }
    let names = spec.schema("TickDirectionType")["x-enumNames"]
        .as_array()
        .unwrap()
        .clone();
    for (code, name) in values("TickDirectionType").into_iter().zip(names) {
        let from_code: TickDirection = serde_json::from_value(code).unwrap();
        let from_name: TickDirection = serde_json::from_value(name).unwrap();
        assert_eq!(from_code, from_name);
    }
    for (schema, names_key) in [
        ("OrderType", "x-enumNames"),
        ("DurationType", "x-enumNames"),
    ] {
        let names = spec.schema(schema)[names_key].as_array().unwrap().clone();
        for (code, name) in values(schema).into_iter().zip(names) {
            if schema == "OrderType" {
                let a: OrderType = serde_json::from_value(code).unwrap();
                let b: OrderType = serde_json::from_value(name).unwrap();
                assert_eq!(a, b);
            } else {
                let a: OrderDuration = serde_json::from_value(code).unwrap();
                let b: OrderDuration = serde_json::from_value(name).unwrap();
                assert_eq!(a, b);
            }
        }
    }
}
