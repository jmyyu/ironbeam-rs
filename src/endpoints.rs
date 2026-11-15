//! The table of REST operations this SDK implements.
//!
//! Every REST method builds its request from one of these constants, and
//! `tests/protocol.rs` checks the table against the locally downloaded OpenAPI document
//! in both directions, so the SDK cannot silently drift from the
//! specification's methods and paths.

/// One REST operation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Endpoint {
    /// HTTP method, upper case.
    pub method: &'static str,
    /// Path template relative to the base URL, with `{name}` parameters.
    pub path: &'static str,
    /// `operationId` in the OpenAPI document.
    pub operation_id: &'static str,
    /// Whether the SDK retries this operation automatically on transient
    /// failures. Only `GET`s and `/auth` are; nothing that places, changes or
    /// cancels an order, or mutates an account, ever is.
    pub auto_retry: bool,
}

macro_rules! endpoints {
    ($( $name:ident = $method:literal $path:literal $op:literal $retry:literal; )+) => {
        $(
        #[doc = concat!("`", $method, " ", $path, "` — operation `", $op, "`.")]
        pub const $name: Endpoint = Endpoint {
            method: $method, path: $path, operation_id: $op, auto_retry: $retry,
        }; )+
        /// Every operation implemented by the SDK.
        pub const ALL: &[Endpoint] = &[ $( $name ),+ ];
    };
}

endpoints! {
    AUTHORIZE = "POST" "/auth" "authorize" true;
    LOGOUT = "POST" "/logout" "logout" false;

    TRADER_INFO = "GET" "/info/trader" "getTraderInfo" true;
    USER_INFO = "GET" "/info/user" "getUserGeneralInfo" true;
    SECURITY_DEFINITIONS = "GET" "/info/security/definitions" "getSecurityDefinitions" true;
    SECURITY_MARGIN = "GET" "/info/security/margin" "getSecurityMarginAndValue" true;
    SECURITY_STATUS = "GET" "/info/security/status" "getSecurityStatus" true;
    SYMBOLS = "GET" "/info/symbols" "getSymbols" true;
    EXCHANGE_SOURCES = "GET" "/info/exchangeSources" "getExchangeSources" true;
    COMPLEXES = "GET" "/info/complexes/{exchange}" "getComplexes" true;
    SYMBOL_FUTURES = "GET" "/info/symbol/search/futures/{exchange}/{marketGroup}" "getSymbolFutures" true;
    SYMBOL_GROUPS = "GET" "/info/symbol/search/groups/{complex}" "getSymbolGroupsByComplex" true;
    SYMBOL_OPTION_GROUPS = "GET" "/info/symbol/search/options/{symbol}" "getSymbolOptionGroups" true;
    SYMBOL_OPTIONS = "GET" "/info/symbol/search/options/ext/{symbol}/{group}/{optionType}/{near}" "getSearchSymbolOption" true;
    SYMBOL_OPTION_SPREADS = "GET" "/info/symbol/search/options/spreads/{symbol}" "getSearchForOptionSpreads" true;
    STRATEGY_ID = "GET" "/info/strategyId" "getStrategyId" true;

    ACCOUNT_BALANCE = "GET" "/account/{accountId}/balance" "accountBalance" true;
    ACCOUNT_POSITIONS = "GET" "/account/{accountId}/positions" "positions" true;
    ACCOUNT_RISK = "GET" "/account/{accountId}/risk" "risk" true;
    ACCOUNT_FILLS = "GET" "/account/{accountId}/fills" "fills" true;
    ALL_ACCOUNTS = "GET" "/account/getAllAccounts" "allAccounts" true;
    ALL_BALANCES = "GET" "/account/getAllBalances" "allAccountsBalance" true;
    ALL_FILLS = "GET" "/account/getAllFills" "allfills" true;
    ALL_POSITIONS = "GET" "/account/getAllPositions" "allpositions" true;
    ALL_RISK = "GET" "/account/getAllRiskInfo" "allaccountsriskinfo" true;

    QUOTES = "GET" "/market/quotes" "getQuotes" true;
    DEPTH = "GET" "/market/depth" "getDepth" true;
    TRADES = "GET" "/market/trades/{symbol}/{from}/{to}/{max}/{earlier}" "getTrades" true;

    PLACE_ORDER = "POST" "/order/{accountId}/place" "placeOrder" false;
    ORDER_ID_FOR_STRATEGY = "GET" "/order/{accountId}/toorderid/{strategyId}" "getToOrderId" true;
    STRATEGY_ID_FOR_ORDER = "GET" "/order/{accountId}/tostrategyId/{orderId}" "getToStrategyId" true;
    UPDATE_ORDER = "PUT" "/order/{accountId}/update/{orderId}" "updateOrder" false;
    ORDERS = "GET" "/order/{accountId}/{orderStatus}" "getOrders" true;
    CANCEL_ORDER = "DELETE" "/order/{accountId}/cancel/{orderId}" "cancelOrder" false;
    CANCEL_MULTIPLE = "DELETE" "/order/{accountId}/cancelMultiple" "cancelMultipleOrders" false;
    ORDER_FILLS = "GET" "/order/{accountId}/fills" "getOrdersFills" true;

    SIM_TRADER_CREATE = "POST" "/simulatedTraderCreate" "simulatedTraderCreate" false;
    SIM_ACCOUNT_ADD = "POST" "/simulatedAccountAdd" "simulatedAccountAdd" false;
    SIM_ACCOUNT_RESET = "PUT" "/simulatedAccountReset" "simulatedAccountReset" false;
    SIM_ACCOUNT_EXPIRE = "DELETE" "/simulatedAccountExpire" "simulatedAccountExpire" false;
    SIM_ADD_CASH = "POST" "/simulatedAccount/addCash" "simulatedAccountAddCash" false;
    SIM_CASH_REPORT = "GET" "/simulatedAccount/getCashReport/{accountId}" "simulatedAccountGetCashReport" true;
    SIM_SET_LIQUIDATE_ONLY = "POST" "/simulatedAccount/setLiquidateOnly" "simulatedAccountSetLiquidateOnly" false;
    SIM_LIQUIDATE = "POST" "/simulatedAccount/liquidate" "simulatedAccountLiquidate" false;
    SIM_SET_RISK = "POST" "/simulatedAccount/setRisk" "simulatedAccountSetRisk" false;

    STREAM_CREATE = "GET" "/stream/create" "getStreamId" true;
    SUBSCRIBE_QUOTES = "GET" "/market/quotes/subscribe/{streamId}" "getSubscribeQuotes" true;
    SUBSCRIBE_DEPTHS = "GET" "/market/depths/subscribe/{streamId}" "getSubscribeDepths" true;
    SUBSCRIBE_TRADES = "GET" "/market/trades/subscribe/{streamId}" "getSubscribeTrades" true;
    UNSUBSCRIBE_QUOTES = "GET" "/market/quotes/unsubscribe/{streamId}" "getUnsubscribeQuotes" true;
    UNSUBSCRIBE_DEPTHS = "GET" "/market/depths/unsubscribe/{streamId}" "UnsubscribeDepths" true;
    UNSUBSCRIBE_TRADES = "GET" "/market/trades/unsubscribe/{streamId}" "getUnsubscribeTrades" true;
    INDICATOR_UNSUBSCRIBE = "DELETE" "/indicator/{streamId}/unsubscribe/{indicatorId}" "indicatorUnsubscribe" false;
    SUBSCRIBE_TICK_BARS = "POST" "/indicator/{streamId}/tickBars/subscribe" "indicatorSubscribeTickBars" false;
    SUBSCRIBE_TRADE_BARS = "POST" "/indicator/{streamId}/tradeBars/subscribe" "indicatorSubscribeTradeBars" false;
    SUBSCRIBE_TIME_BARS = "POST" "/indicator/{streamId}/timeBars/subscribe" "indicatorSubscribeTimeBars" false;
    SUBSCRIBE_VOLUME_BARS = "POST" "/indicator/{streamId}/volumeBars/subscribe" "indicatorSubscribeVolumeBars" false;
}

/// The WebSocket endpoint (`GET /stream/{streamId}` upgraded to WebSocket).
/// Not part of [`ALL`] because it is not a REST call.
pub const STREAM_SOCKET: Endpoint = Endpoint {
    method: "GET",
    path: "/stream/{streamId}",
    operation_id: "getStream",
    auto_retry: false,
};
