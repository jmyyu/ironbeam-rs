//! Enumerations used across REST and streaming payloads.

use super::common::wire_enum;

wire_enum! {
    /// Order side.
    pub enum OrderSide {
        Buy = "BUY",
        Sell = "SELL",
        Invalid = "INVALID" | "",
    }
}

wire_enum! {
    /// Order type. The schema enumerates `""`, `"1"`..`"4"`; names are
    /// accepted as aliases on input.
    pub enum OrderType {
        Market = "1" | "MARKET",
        Limit = "2" | "LIMIT",
        Stop = "3" | "STOP",
        StopLimit = "4" | "STOP_LIMIT",
        Invalid = "" | "INVALID",
    }
}

impl OrderType {
    /// Symbolic name (`MARKET`, `LIMIT`, `STOP`, `STOP_LIMIT`).
    pub fn name(&self) -> &str {
        match self {
            Self::Market => "MARKET",
            Self::Limit => "LIMIT",
            Self::Stop => "STOP",
            Self::StopLimit => "STOP_LIMIT",
            Self::Invalid => "INVALID",
            Self::Unknown(raw) => raw,
        }
    }
}

wire_enum! {
    /// Order duration (time in force). The schema enumerates `""`, `"0"`,
    /// `"1"`; names are accepted as aliases on input.
    pub enum OrderDuration {
        Day = "0" | "DAY",
        GoodTillCancel = "1" | "GOOD_TILL_CANCEL" | "GTC",
        Invalid = "" | "INVALID",
    }
}

impl OrderDuration {
    /// Symbolic name (`DAY`, `GOOD_TILL_CANCEL`).
    pub fn name(&self) -> &str {
        match self {
            Self::Day => "DAY",
            Self::GoodTillCancel => "GOOD_TILL_CANCEL",
            Self::Invalid => "INVALID",
            Self::Unknown(raw) => raw,
        }
    }
}

wire_enum! {
    /// Order status. `Any` is only meaningful as a query filter.
    pub enum OrderStatus {
        Any = "ANY",
        Invalid = "INVALID",
        Submitted = "SUBMITTED",
        New = "NEW",
        PartiallyFilled = "PARTIALLY_FILLED",
        Filled = "FILLED",
        DoneForDay = "DONE_FOR_DAY",
        Cancelled = "CANCELLED" | "CANCELED",
        Replaced = "REPLACED",
        PendingCancel = "PENDING_CANCEL",
        Stopped = "STOPPED",
        Rejected = "REJECTED",
        Suspended = "SUSPENDED",
        PendingNew = "PENDING_NEW",
        Calculated = "CALCULATED",
        Expired = "EXPIRED",
        AcceptedForBidding = "ACCEPTED_FOR_BIDDING",
        PendingReplace = "PENDING_REPLACE",
        CancelRejected = "CANCEL_REJECTED",
        OrderNotFound = "ORDER_NOT_FOUND",
        QueuedNew = "QUEUED_NEW",
        QueuedCancel = "QUEUED_CANCEL",
        Complete = "COMPLETE",
    }
}

/// Coarse life-cycle phase of an order, derived from [`OrderStatus`].
///
/// The specification lists the statuses without defining their semantics; this
/// classification follows the FIX `OrdStatus` meanings the names are borrowed
/// from and is an SDK interpretation (see `docs/SPEC_DISCREPANCIES.md`, A-04).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum OrderPhase {
    /// Sent but not yet acknowledged by the exchange.
    Pending,
    /// Live at the exchange (possibly partially filled or mid-change).
    Working,
    /// Finished; no further fills or changes are expected.
    Terminal,
    /// The status carries no usable life-cycle information.
    Indeterminate,
}

impl OrderStatus {
    /// Life-cycle phase implied by this status.
    pub fn phase(&self) -> OrderPhase {
        use OrderStatus::*;
        match self {
            Submitted | PendingNew | QueuedNew => OrderPhase::Pending,
            New | PartiallyFilled | Replaced | PendingCancel | PendingReplace | QueuedCancel
            | CancelRejected | Stopped | Suspended | Calculated | AcceptedForBidding => {
                OrderPhase::Working
            }
            Filled | Cancelled | Rejected | Expired | DoneForDay | Complete => OrderPhase::Terminal,
            Any | Invalid | OrderNotFound | Unknown(_) => OrderPhase::Indeterminate,
        }
    }

    /// `true` when no further activity is expected on the order.
    pub fn is_terminal(&self) -> bool {
        self.phase() == OrderPhase::Terminal
    }

    /// `true` when the order is pending or working, i.e. may still trade.
    pub fn is_open(&self) -> bool {
        matches!(self.phase(), OrderPhase::Pending | OrderPhase::Working)
    }
}

wire_enum! {
    /// Balance flavour requested from the balance endpoints.
    pub enum BalanceType {
        CurrentOpen = "CURRENT_OPEN",
        StartOfDay = "START_OF_DAY",
    }
}

wire_enum! {
    /// Position side.
    pub enum PositionSide {
        Long = "LONG",
        Short = "SHORT",
    }
}

wire_enum! {
    /// Book side as used in security-definition legs.
    pub enum Side {
        Bid = "BID",
        Ask = "ASK",
        Buy = "BUY",
        Sell = "SELL",
    }
}

wire_enum! {
    /// Book side as used in depth levels (`B`/`A`).
    pub enum BookSide {
        Bid = "B",
        Ask = "A",
    }
}

wire_enum! {
    /// Aggressor side of a trade. Sent as an integer code (`0`/`1`/`2`).
    pub enum AggressorSide {
        Invalid = "INVALID" | "0",
        Buy = "BUY" | "1",
        Sell = "SELL" | "2",
    }
}

wire_enum! {
    /// Tick direction. Quotes send integer codes (`255`, `0`, `1`, `2`),
    /// trades send names.
    pub enum TickDirection {
        Invalid = "INVALID" | "255",
        Plus = "PLUS" | "0",
        Same = "SAME" | "1",
        Minus = "MINUS" | "2",
    }
}

wire_enum! {
    /// Trading status of a security. Sent as an integer code.
    pub enum SecurityStatus {
        TradingHalt = "TRADING_HALT" | "2",
        Close = "CLOSE" | "18",
        PriceIndication = "PRICE_INDICATION" | "15",
        Open = "OPEN" | "17",
        Closed = "CLOSED" | "4",
        UnknownStatus = "UNKNOWN" | "20",
        PreOpen = "PRE_OPEN" | "21",
        OpeningRotation = "OPENING_ROTATION" | "22",
        PreCross = "PRE_CROSS" | "24",
        Cross = "CROSS" | "25",
        NoCancel = "NO_CANCEL" | "26",
        Expired = "EXPIRED" | "30",
        PreClose = "PRE_CLOSE" | "31",
        NoChange = "NO_CHANGE" | "103",
        PostClose = "POST_CLOSE" | "126",
    }
}

wire_enum! {
    /// Option right.
    pub enum OptionType {
        Invalid = "INVALID",
        Put = "PUT",
        Call = "CALL",
    }
}

wire_enum! {
    /// Option exercise style.
    pub enum OptionExpirationType {
        Invalid = "INVALID",
        American = "AMERICAN",
        European = "EUROPEAN",
    }
}

wire_enum! {
    /// Security type.
    pub enum SecurityType {
        Invalid = "INVALID",
        Future = "FUT",
        Option = "OPT",
        Mixed = "MIXED",
    }
}

wire_enum! {
    /// Regulatory code of a sub-account.
    pub enum RegCode {
        Invalid = "INVALID",
        Combined = "COMBINED",
        Regulated = "REGULATED",
        NonSecured = "NON_SECURED",
        Secured = "SECURED",
    }
}

wire_enum! {
    /// Bar aggregation unit for indicator subscriptions.
    pub enum BarType {
        Daily = "DAILY",
        Hour = "HOUR",
        Minute = "MINUTE",
        Tick = "TICK",
    }
}

wire_enum! {
    /// `spt` field of a streamed trade.
    pub enum SystemPricedTrade {
        Invalid = "INVALID",
        System = "SYSTEM",
        Crack = "CRACK",
    }
}

wire_enum! {
    /// `ist` field of a streamed trade.
    pub enum InvestigationStatus {
        Invalid = "INVALID",
        Investigating = "INVESTIGATING",
        Completed = "COMPLETED",
    }
}

wire_enum! {
    /// `bt` field of a streamed trade.
    pub enum BlockTrade {
        Invalid = "INVALID",
        Normal = "NORMAL",
        Efp = "EFP",
        Efs = "EFS",
        OffExchange = "OFF_EXCHANGE",
        Ng = "NG",
        Ccx = "CCX",
        Efr = "EFR",
    }
}

/// Option right as used in the option-search **path** (lower case, unlike
/// [`OptionType`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum OptionRight {
    /// `call`
    Call,
    /// `put`
    Put,
}

impl OptionRight {
    /// Path segment value.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Call => "call",
            Self::Put => "put",
        }
    }
}

/// Liquidation event codes found in [`RiskInfo::liquidation_events`](super::RiskInfo).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum LiquidationEvent {
    /// `1`
    AutoLiqStarted,
    /// `2`
    AutoLiqSuccessful,
    /// `3`
    AutoLiqOrdersFailed,
    /// `4`
    AutoLiqFail,
    /// `12`
    ResetLiquidation,
    /// `13`
    ManualLiqStarted,
    /// `14`
    ManualLiqSuccessful,
    /// Any other code (the documentation's own sample shows `0`).
    Other(i64),
}

impl LiquidationEvent {
    /// Maps a numeric event code.
    pub fn from_code(code: i64) -> Self {
        match code {
            1 => Self::AutoLiqStarted,
            2 => Self::AutoLiqSuccessful,
            3 => Self::AutoLiqOrdersFailed,
            4 => Self::AutoLiqFail,
            12 => Self::ResetLiquidation,
            13 => Self::ManualLiqStarted,
            14 => Self::ManualLiqSuccessful,
            other => Self::Other(other),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn order_type_accepts_codes_names_and_numbers() {
        for raw in ["\"2\"", "\"LIMIT\"", "\"limit\"", "2"] {
            assert_eq!(
                serde_json::from_str::<OrderType>(raw).unwrap(),
                OrderType::Limit,
                "{raw}"
            );
        }
        assert_eq!(serde_json::to_string(&OrderType::Limit).unwrap(), "\"2\"");
        assert_eq!(OrderType::StopLimit.name(), "STOP_LIMIT");
        assert_eq!(
            serde_json::from_str::<OrderType>("\"\"").unwrap(),
            OrderType::Invalid
        );
    }

    #[test]
    fn unknown_values_are_preserved_not_rejected() {
        let s: OrderStatus = serde_json::from_str("\"SOMETHING_NEW\"").unwrap();
        assert_eq!(s, OrderStatus::Unknown("SOMETHING_NEW".into()));
        assert_eq!(s.phase(), OrderPhase::Indeterminate);
        assert_eq!(serde_json::to_string(&s).unwrap(), "\"SOMETHING_NEW\"");
    }

    #[test]
    fn integer_coded_enums() {
        assert_eq!(
            serde_json::from_str::<SecurityStatus>("17").unwrap(),
            SecurityStatus::Open
        );
        assert_eq!(
            serde_json::from_str::<SecurityStatus>("\"OPEN\"").unwrap(),
            SecurityStatus::Open
        );
        assert_eq!(
            serde_json::from_str::<AggressorSide>("2").unwrap(),
            AggressorSide::Sell
        );
        assert_eq!(
            serde_json::from_str::<TickDirection>("255").unwrap(),
            TickDirection::Invalid
        );
        assert_eq!(
            serde_json::from_str::<TickDirection>("\"MINUS\"").unwrap(),
            TickDirection::Minus
        );
        assert_eq!(
            serde_json::from_str::<SecurityStatus>("999").unwrap(),
            SecurityStatus::Unknown("999".into())
        );
    }

    #[test]
    fn status_phases() {
        assert!(OrderStatus::Filled.is_terminal());
        assert!(OrderStatus::Cancelled.is_terminal());
        assert!(OrderStatus::Rejected.is_terminal());
        assert!(OrderStatus::New.is_open());
        assert!(OrderStatus::PartiallyFilled.is_open());
        assert!(OrderStatus::PendingNew.is_open());
        assert!(OrderStatus::CancelRejected.is_open());
        assert!(!OrderStatus::OrderNotFound.is_open());
        assert!(!OrderStatus::OrderNotFound.is_terminal());
    }
}
