//! Client configuration: environments, timeouts, retry policy and wire dialect.

use std::time::Duration;

use url::Url;

use crate::error::{Error, Result};

/// Which Ironbeam deployment to talk to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Environment {
    /// `https://demo.ironbeamapi.com/v2` — simulated trading.
    Demo,
    /// `https://live.ironbeamapi.com/v2` — real money.
    Live,
    /// Any other base URL (mock servers, proxies). Must include the version
    /// prefix, e.g. `http://127.0.0.1:8080/v2`.
    Custom(Url),
}

impl Environment {
    /// Parses a custom base URL.
    pub fn custom(url: &str) -> Result<Self> {
        let url = Url::parse(url).map_err(|e| Error::Config(format!("invalid base URL: {e}")))?;
        Ok(Self::Custom(url))
    }

    /// REST base URL, guaranteed to end without a trailing slash.
    pub fn base_url(&self) -> Url {
        match self {
            Self::Demo => Url::parse("https://demo.ironbeamapi.com/v2").expect("static URL"),
            Self::Live => Url::parse("https://live.ironbeamapi.com/v2").expect("static URL"),
            Self::Custom(u) => u.clone(),
        }
    }
}

/// How `orderType` and `duration` are written in order requests.
///
/// The specification contradicts itself here (see
/// `docs/SPEC_DISCREPANCIES.md`, D-07): the schema enumerates numeric string
/// codes (`"1"`..`"4"`, `"0"`/`"1"`) while examples and prose use names
/// (`"LIMIT"`, `"DAY"`). Responses are always parsed leniently, accepting
/// either form.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum EnumEncoding {
    /// Schema enum values: `orderType: "2"`, `duration: "0"`. Default, because
    /// the schema is the machine-readable source of truth.
    #[default]
    Codes,
    /// Symbolic names: `orderType: "LIMIT"`, `duration: "DAY"`.
    Names,
}

/// Automatic retry policy for **idempotent** requests only.
///
/// Order placement and order modification are never retried automatically,
/// whatever this policy says.
#[derive(Debug, Clone, PartialEq)]
pub struct RetryPolicy {
    /// Retries after the first attempt. `0` disables automatic retries.
    pub max_retries: u32,
    /// Delay before the first retry; doubles on each subsequent retry.
    pub base_delay: Duration,
    /// Upper bound for a single backoff delay (also caps `Retry-After`).
    pub max_delay: Duration,
    /// Randomise each delay uniformly in `[delay/2, delay]`.
    pub jitter: bool,
}

impl Default for RetryPolicy {
    fn default() -> Self {
        Self {
            max_retries: 3,
            base_delay: Duration::from_millis(250),
            max_delay: Duration::from_secs(5),
            jitter: true,
        }
    }
}

impl RetryPolicy {
    /// A policy that never retries.
    pub fn none() -> Self {
        Self {
            max_retries: 0,
            ..Self::default()
        }
    }

    /// Backoff delay before retry number `retry` (1-based).
    pub(crate) fn delay(&self, retry: u32) -> Duration {
        backoff_delay(self.base_delay, self.max_delay, retry, self.jitter)
    }
}

/// Exponential backoff shared by REST retries and stream reconnects.
pub(crate) fn backoff_delay(base: Duration, max: Duration, attempt: u32, jitter: bool) -> Duration {
    let exp = attempt.saturating_sub(1).min(20);
    let raw = base.saturating_mul(1u32 << exp).min(max);
    if jitter && !raw.is_zero() {
        let half = raw / 2;
        half + Duration::from_secs_f64(half.as_secs_f64() * fastrand::f64())
    } else {
        raw
    }
}

/// Settings for [`Client`](crate::Client).
#[derive(Debug, Clone)]
pub struct ClientConfig {
    /// Target deployment. Defaults to [`Environment::Demo`] so that nothing
    /// reaches a live account by accident.
    pub environment: Environment,
    /// Total timeout for ordinary requests.
    pub request_timeout: Duration,
    /// Total timeout for order placement. The server may hold the response for
    /// up to 10 s while waiting for an exchange order id (`waitForOrderId`),
    /// so this must comfortably exceed that.
    pub order_timeout: Duration,
    /// TCP + TLS connect timeout.
    pub connect_timeout: Duration,
    /// Retry policy for idempotent requests.
    pub retry: RetryPolicy,
    /// `User-Agent` header value.
    pub user_agent: String,
    /// Wire form of order enums in requests.
    pub enum_encoding: EnumEncoding,
}

impl Default for ClientConfig {
    fn default() -> Self {
        Self {
            environment: Environment::Demo,
            request_timeout: Duration::from_secs(15),
            order_timeout: Duration::from_secs(25),
            connect_timeout: Duration::from_secs(10),
            retry: RetryPolicy::default(),
            user_agent: concat!("ironbeam-sdk-rs/", env!("CARGO_PKG_VERSION")).to_owned(),
            enum_encoding: EnumEncoding::default(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backoff_grows_and_caps() {
        let base = Duration::from_millis(100);
        let max = Duration::from_secs(1);
        assert_eq!(
            backoff_delay(base, max, 1, false),
            Duration::from_millis(100)
        );
        assert_eq!(
            backoff_delay(base, max, 2, false),
            Duration::from_millis(200)
        );
        assert_eq!(
            backoff_delay(base, max, 4, false),
            Duration::from_millis(800)
        );
        assert_eq!(backoff_delay(base, max, 5, false), max);
        assert_eq!(backoff_delay(base, max, 500, false), max);
    }

    #[test]
    fn jitter_stays_within_bounds() {
        let base = Duration::from_millis(100);
        for _ in 0..200 {
            let d = backoff_delay(base, Duration::from_secs(1), 3, true);
            assert!(
                d >= Duration::from_millis(200) && d <= Duration::from_millis(400),
                "{d:?}"
            );
        }
    }

    #[test]
    fn environment_urls() {
        assert_eq!(
            Environment::Demo.base_url().as_str(),
            "https://demo.ironbeamapi.com/v2"
        );
        assert_eq!(
            Environment::Live.base_url().as_str(),
            "https://live.ironbeamapi.com/v2"
        );
        assert!(Environment::custom("not a url").is_err());
    }
}
