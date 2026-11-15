use crate::client::{Call, Client};
use crate::endpoints::{self as ep, Endpoint};
use crate::error::{Error, Result};
use crate::models::*;

use super::join_symbols;

/// The four bar indicators that can be subscribed on a stream.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum BarKind {
    /// `tradeBars` — delivered in the stream's `tb` field.
    Trade,
    /// `tickBars` — delivered in the stream's `tc` field.
    Tick,
    /// `timeBars` — delivered in the stream's `ti` field.
    Time,
    /// `volumeBars` — delivered in the stream's `vb` field.
    Volume,
}

impl BarKind {
    fn endpoint(self) -> &'static Endpoint {
        match self {
            Self::Trade => &ep::SUBSCRIBE_TRADE_BARS,
            Self::Tick => &ep::SUBSCRIBE_TICK_BARS,
            Self::Time => &ep::SUBSCRIBE_TIME_BARS,
            Self::Volume => &ep::SUBSCRIBE_VOLUME_BARS,
        }
    }
}

/// Low-level stream control. Most users should use
/// [`Stream`](crate::stream::Stream), which calls these and keeps the
/// subscriptions alive across reconnects.
impl Client {
    /// Creates a stream id. A stream id is single-use: after its WebSocket
    /// closes a new one must be created.
    pub async fn create_stream_id(&self) -> Result<String> {
        let resp: StreamIdResponse = self.send(Call::new(&ep::STREAM_CREATE)).await?;
        resp.stream_id
            .filter(|s| !s.is_empty())
            .ok_or_else(|| Error::Api {
                status: 200,
                message: Some("stream creation response contained no streamId".into()),
                body: String::new(),
            })
    }

    async fn symbols_call<S: AsRef<str>>(
        &self,
        endpoint: &'static Endpoint,
        stream_id: &str,
        symbols: &[S],
    ) -> Result<ApiResponse> {
        let call = Call::new(endpoint)
            .params(&[("streamId", stream_id)])
            .query("symbols", join_symbols(symbols)?);
        self.send(call).await
    }

    /// Subscribes a stream to quotes (max 10 symbols per stream).
    pub async fn subscribe_quotes<S: AsRef<str>>(
        &self,
        stream_id: &str,
        symbols: &[S],
    ) -> Result<ApiResponse> {
        self.symbols_call(&ep::SUBSCRIBE_QUOTES, stream_id, symbols)
            .await
    }

    /// Subscribes a stream to market depth (max 10 symbols per stream).
    pub async fn subscribe_depths<S: AsRef<str>>(
        &self,
        stream_id: &str,
        symbols: &[S],
    ) -> Result<ApiResponse> {
        self.symbols_call(&ep::SUBSCRIBE_DEPTHS, stream_id, symbols)
            .await
    }

    /// Subscribes a stream to trades (max 10 symbols per stream).
    pub async fn subscribe_trades<S: AsRef<str>>(
        &self,
        stream_id: &str,
        symbols: &[S],
    ) -> Result<ApiResponse> {
        self.symbols_call(&ep::SUBSCRIBE_TRADES, stream_id, symbols)
            .await
    }

    /// Unsubscribes a stream from quotes.
    pub async fn unsubscribe_quotes<S: AsRef<str>>(
        &self,
        stream_id: &str,
        symbols: &[S],
    ) -> Result<ApiResponse> {
        self.symbols_call(&ep::UNSUBSCRIBE_QUOTES, stream_id, symbols)
            .await
    }

    /// Unsubscribes a stream from market depth.
    pub async fn unsubscribe_depths<S: AsRef<str>>(
        &self,
        stream_id: &str,
        symbols: &[S],
    ) -> Result<ApiResponse> {
        self.symbols_call(&ep::UNSUBSCRIBE_DEPTHS, stream_id, symbols)
            .await
    }

    /// Unsubscribes a stream from trades.
    pub async fn unsubscribe_trades<S: AsRef<str>>(
        &self,
        stream_id: &str,
        symbols: &[S],
    ) -> Result<ApiResponse> {
        self.symbols_call(&ep::UNSUBSCRIBE_TRADES, stream_id, symbols)
            .await
    }

    /// Subscribes a stream to a bar indicator. Not retried: each successful
    /// call creates a new indicator.
    pub async fn subscribe_bars(
        &self,
        stream_id: &str,
        kind: BarKind,
        request: &BarSubscription,
    ) -> Result<IndicatorSubscribeResponse> {
        validate_symbol(&request.symbol)?;
        if request.period == 0 {
            return Err(Error::InvalidRequest("bar period must be positive".into()));
        }
        let body = serde_json::to_value(request)
            .map_err(|e| Error::InvalidRequest(format!("unserializable bar request: {e}")))?;
        let call = Call::new(kind.endpoint())
            .params(&[("streamId", stream_id)])
            .body(body);
        self.send(call).await
    }

    /// Removes an indicator subscription from a stream.
    pub async fn unsubscribe_indicator(
        &self,
        stream_id: &str,
        indicator_id: &str,
    ) -> Result<ApiResponse> {
        let params = [("streamId", stream_id), ("indicatorId", indicator_id)];
        self.send(Call::new(&ep::INDICATOR_UNSUBSCRIBE).params(&params))
            .await
    }
}
