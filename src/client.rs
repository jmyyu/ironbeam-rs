//! The REST client core: authentication, request execution, retry
//! classification and error mapping.

use std::fmt;
use std::sync::{Arc, RwLock};
use std::time::Duration;

use reqwest::{Method, StatusCode};
use serde::Deserialize;
use serde::de::DeserializeOwned;
use serde_json::{Value, json};
use url::Url;

use crate::config::{ClientConfig, Environment};
use crate::endpoints::{self, Endpoint};
use crate::error::{Delivery, Error, Result, truncate_body};
use crate::models::ResponseStatus;

/// Login credentials for `POST /auth`.
///
/// `Debug` never prints the secrets.
#[derive(Clone)]
pub struct Credentials {
    username: String,
    password: Option<String>,
    api_key: Option<String>,
}

impl Credentials {
    /// Username plus API key — the standard API login.
    pub fn api_key(username: impl Into<String>, api_key: impl Into<String>) -> Self {
        Self {
            username: username.into(),
            password: None,
            api_key: Some(api_key.into()),
        }
    }

    /// Username plus password, without an API key.
    pub fn password(username: impl Into<String>, password: impl Into<String>) -> Self {
        Self {
            username: username.into(),
            password: Some(password.into()),
            api_key: None,
        }
    }

    /// Username, password and Enterprise API key.
    pub fn enterprise(
        username: impl Into<String>,
        password: impl Into<String>,
        api_key: impl Into<String>,
    ) -> Self {
        Self {
            username: username.into(),
            password: Some(password.into()),
            api_key: Some(api_key.into()),
        }
    }

    /// The username.
    pub fn username(&self) -> &str {
        &self.username
    }

    fn to_wire(&self) -> Value {
        // Property names follow the `AuthorizationRequest` schema exactly
        // (`apikey`, all lower case). See docs/SPEC_DISCREPANCIES.md, D-03.
        let mut body = json!({ "username": self.username });
        if let Some(p) = &self.password {
            body["password"] = json!(p);
        }
        if let Some(k) = &self.api_key {
            body["apikey"] = json!(k);
        }
        body
    }
}

impl fmt::Debug for Credentials {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Credentials")
            .field("username", &self.username)
            .field("password", &self.password.as_ref().map(|_| "<redacted>"))
            .field("api_key", &self.api_key.as_ref().map(|_| "<redacted>"))
            .finish()
    }
}

/// Builder for [`Client`].
#[derive(Debug, Default)]
pub struct ClientBuilder {
    config: ClientConfig,
    credentials: Option<Credentials>,
    token: Option<String>,
}

impl ClientBuilder {
    /// Selects the deployment (default: demo).
    pub fn environment(mut self, environment: Environment) -> Self {
        self.config.environment = environment;
        self
    }

    /// Replaces the whole configuration.
    pub fn config(mut self, config: ClientConfig) -> Self {
        self.config = config;
        self
    }

    /// Credentials used to obtain a token on first use and to transparently
    /// re-authenticate when the server answers 401.
    pub fn credentials(mut self, credentials: Credentials) -> Self {
        self.credentials = Some(credentials);
        self
    }

    /// Starts with an existing bearer token. Without credentials the client
    /// cannot renew it.
    pub fn token(mut self, token: impl Into<String>) -> Self {
        self.token = Some(token.into());
        self
    }

    /// Builds the client. Performs no I/O.
    pub fn build(self) -> Result<Client> {
        let base = self.config.environment.base_url();
        match base.scheme() {
            "https" | "http" => {}
            other => return Err(Error::Config(format!("unsupported URL scheme {other:?}"))),
        }
        if base.cannot_be_a_base() || base.host_str().is_none() {
            return Err(Error::Config(format!("base URL {base} has no host")));
        }
        let tls = crate::tls::client_config();
        let http = reqwest::Client::builder()
            .tls_backend_preconfigured((*tls).clone())
            .user_agent(self.config.user_agent.clone())
            .connect_timeout(self.config.connect_timeout)
            // A redirect would silently replay a request (possibly an order)
            // against another URL.
            .redirect(reqwest::redirect::Policy::none())
            // All retry decisions are made in this crate, per operation.
            .retry(reqwest::retry::never())
            .build()
            .map_err(|e| Error::Config(format!("failed to build HTTP client: {e}")))?;
        Ok(Client {
            inner: Arc::new(Inner {
                http,
                base,
                config: self.config,
                credentials: self.credentials,
                token: RwLock::new(self.token.map(Arc::from)),
                auth_lock: tokio::sync::Mutex::new(()),
                tls,
            }),
        })
    }
}

struct Inner {
    http: reqwest::Client,
    base: Url,
    config: ClientConfig,
    credentials: Option<Credentials>,
    token: RwLock<Option<Arc<str>>>,
    auth_lock: tokio::sync::Mutex<()>,
    tls: Arc<rustls::ClientConfig>,
}

/// Asynchronous client for the Ironbeam REST API.
///
/// Cheap to clone; clones share the connection pool and the bearer token.
/// The REST operations are defined in the [`rest`](crate::rest) module.
#[derive(Clone)]
pub struct Client {
    inner: Arc<Inner>,
}

impl fmt::Debug for Client {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Client")
            .field("base_url", &self.inner.base.as_str())
            .field("credentials", &self.inner.credentials)
            .field("has_token", &self.token().is_some())
            .finish()
    }
}

/// How a request is executed.
pub(crate) struct Call {
    endpoint: &'static Endpoint,
    /// Values for the `{name}` placeholders of the endpoint path.
    params: Vec<(&'static str, String)>,
    query: Vec<(&'static str, String)>,
    body: Option<Value>,
    timeout: Option<Duration>,
}

impl Call {
    pub(crate) fn new(endpoint: &'static Endpoint) -> Self {
        Self {
            endpoint,
            params: Vec::new(),
            query: Vec::new(),
            body: None,
            timeout: None,
        }
    }

    pub(crate) fn params(mut self, params: &[(&'static str, &str)]) -> Self {
        self.params = params
            .iter()
            .map(|(name, value)| (*name, (*value).to_owned()))
            .collect();
        self
    }

    pub(crate) fn query(mut self, name: &'static str, value: impl Into<String>) -> Self {
        self.query.push((name, value.into()));
        self
    }

    pub(crate) fn body(mut self, body: Value) -> Self {
        self.body = Some(body);
        self
    }

    pub(crate) fn timeout(mut self, timeout: Duration) -> Self {
        self.timeout = Some(timeout);
        self
    }
}

/// Envelope fields probed on every success body.
#[derive(Deserialize)]
struct StatusProbe {
    #[serde(default)]
    status: Option<ResponseStatus>,
    #[serde(default)]
    message: Option<String>,
    #[serde(default)]
    error: Option<String>,
}

#[derive(Deserialize)]
struct AuthorizationResponse {
    #[serde(default)]
    token: Option<String>,
}

impl Client {
    /// Starts building a client.
    pub fn builder() -> ClientBuilder {
        ClientBuilder::default()
    }

    /// The configuration this client was built with.
    pub fn config(&self) -> &ClientConfig {
        &self.inner.config
    }

    /// REST base URL.
    pub fn base_url(&self) -> &Url {
        &self.inner.base
    }

    /// The current bearer token, if any.
    pub fn token(&self) -> Option<Arc<str>> {
        self.inner
            .token
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }

    /// Replaces the bearer token (for tokens obtained elsewhere).
    pub fn set_token(&self, token: Option<String>) {
        *self.inner.token.write().unwrap_or_else(|e| e.into_inner()) = token.map(Arc::from);
    }

    pub(crate) fn tls_config(&self) -> Arc<rustls::ClientConfig> {
        self.inner.tls.clone()
    }

    /// Authenticates with the configured credentials (`POST /auth`) and stores
    /// the token. Called implicitly by the first authenticated request, and
    /// again whenever the server answers 401.
    pub async fn authenticate(&self) -> Result<()> {
        let _guard = self.inner.auth_lock.lock().await;
        self.authenticate_locked().await
    }

    async fn authenticate_locked(&self) -> Result<()> {
        let creds = self
            .inner
            .credentials
            .as_ref()
            .ok_or(Error::MissingCredentials)?;
        let call = Call::new(&endpoints::AUTHORIZE).body(creds.to_wire());
        // Boxed: `execute` can itself trigger authentication.
        let body = Box::pin(self.execute(call, false)).await?;
        let auth: AuthorizationResponse = decode(&body)?;
        match auth.token.filter(|t| !t.is_empty()) {
            Some(token) => {
                self.set_token(Some(token));
                tracing::debug!("authenticated");
                Ok(())
            }
            None => Err(Error::Api {
                status: 200,
                message: Some("authorization response contained no token".into()),
                body: truncate_body(body),
            }),
        }
    }

    /// Obtains a fresh token after `stale` was rejected. If another task has
    /// already replaced the token, that one is reused.
    pub(crate) async fn reauthenticate(&self, stale: Option<&Arc<str>>) -> Result<()> {
        let _guard = self.inner.auth_lock.lock().await;
        let current = self.token();
        let replaced = match (&current, stale) {
            (Some(cur), Some(old)) => !Arc::ptr_eq(cur, old) && **cur != **old,
            (Some(_), None) => true,
            (None, _) => false,
        };
        if replaced {
            return Ok(());
        }
        self.authenticate_locked().await
    }

    async fn bearer(&self) -> Result<Arc<str>> {
        if let Some(t) = self.token() {
            return Ok(t);
        }
        self.reauthenticate(None).await?;
        self.token().ok_or(Error::MissingCredentials)
    }

    /// Invalidates the token on the server (`POST /logout`) and forgets it.
    pub async fn logout(&self) -> Result<()> {
        if self.token().is_none() {
            return Ok(());
        }
        let result = self.send::<Value>(Call::new(&endpoints::LOGOUT)).await;
        self.set_token(None);
        result.map(|_| ())
    }

    pub(crate) fn url_for(&self, endpoint: &Endpoint, params: &[(&str, &str)]) -> Result<Url> {
        build_url(&self.inner.base, endpoint.path, params)
    }

    /// Executes an authenticated call and decodes the JSON body.
    pub(crate) async fn send<T: DeserializeOwned>(&self, call: Call) -> Result<T> {
        let body = self.execute(call, true).await?;
        decode(&body)
    }

    /// Executes a call and returns the raw success body.
    ///
    /// Retry rules:
    /// * 401 → re-authenticate once and resend. A 401 is a definitive
    ///   "not processed", so this is safe for every operation.
    /// * transport errors, 429 and 5xx → retried with backoff **only** when
    ///   the endpoint is marked `auto_retry` (reads and `/auth`).
    /// * everything else is returned to the caller.
    async fn execute(&self, call: Call, authenticated: bool) -> Result<String> {
        let endpoint = call.endpoint;
        let params: Vec<(&str, &str)> = call.params.iter().map(|(n, v)| (*n, v.as_str())).collect();
        let url = self.url_for(endpoint, &params)?;
        let method = Method::from_bytes(endpoint.method.as_bytes())
            .map_err(|_| Error::Config(format!("bad method {}", endpoint.method)))?;
        let timeout = call.timeout.unwrap_or(self.inner.config.request_timeout);
        let policy = &self.inner.config.retry;
        let max_retries = if endpoint.auto_retry {
            policy.max_retries
        } else {
            0
        };

        let mut retries = 0u32;
        let mut reauthenticated = false;
        loop {
            let token = if authenticated {
                Some(self.bearer().await?)
            } else {
                None
            };
            let mut req = self
                .inner
                .http
                .request(method.clone(), url.clone())
                .timeout(timeout)
                .header(reqwest::header::ACCEPT, "application/json");
            if !call.query.is_empty() {
                req = req.query(&call.query);
            }
            if let Some(t) = &token {
                req = req.bearer_auth(t);
            }
            if let Some(body) = &call.body {
                req = req.json(body);
            }

            let error = match self.attempt(req).await {
                Ok(body) => return Ok(body),
                Err(e) => e,
            };

            if error.status() == Some(401)
                && authenticated
                && !reauthenticated
                && self.inner.credentials.is_some()
            {
                tracing::debug!(
                    op = endpoint.operation_id,
                    "401 received; re-authenticating"
                );
                reauthenticated = true;
                self.reauthenticate(token.as_ref()).await?;
                continue;
            }

            if retries < max_retries && error.is_transient() {
                retries += 1;
                let mut delay = policy.delay(retries);
                if let Error::RateLimited {
                    retry_after: Some(ra),
                    ..
                } = &error
                {
                    delay = (*ra).min(policy.max_delay).max(delay);
                }
                tracing::debug!(
                    op = endpoint.operation_id,
                    retry = retries,
                    ?delay,
                    error = %error,
                    "transient failure; retrying idempotent request"
                );
                tokio::time::sleep(delay).await;
                continue;
            }
            return Err(error);
        }
    }

    async fn attempt(&self, req: reqwest::RequestBuilder) -> Result<String> {
        let response = req.send().await.map_err(transport_error)?;
        let status = response.status();
        let retry_after = response
            .headers()
            .get(reqwest::header::RETRY_AFTER)
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.trim().parse::<u64>().ok())
            .map(Duration::from_secs);
        // The request was delivered; a failure while reading the body leaves
        // the outcome unknown.
        let body = response.text().await.map_err(|source| Error::Transport {
            delivery: Delivery::MaybeSent,
            timed_out: source.is_timeout(),
            source,
        })?;

        if status.is_success() {
            // Some endpoints report failure in the envelope of a 200 response.
            if let Ok(probe) = serde_json::from_str::<StatusProbe>(&body) {
                if probe
                    .status
                    .as_ref()
                    .is_some_and(ResponseStatus::is_failure)
                {
                    return Err(Error::Api {
                        status: status.as_u16(),
                        message: probe.error.or(probe.message),
                        body: truncate_body(body),
                    });
                }
            }
            return Ok(body);
        }
        if status == StatusCode::TOO_MANY_REQUESTS {
            return Err(Error::RateLimited {
                retry_after,
                body: truncate_body(body),
            });
        }
        let message = serde_json::from_str::<StatusProbe>(&body)
            .ok()
            .and_then(|p| p.error.filter(|s| !s.is_empty()).or(p.message));
        Err(Error::Api {
            status: status.as_u16(),
            message,
            body: truncate_body(body),
        })
    }
}

fn transport_error(source: reqwest::Error) -> Error {
    // `is_connect` covers DNS, TCP and TLS establishment: the request was
    // never written. Anything else may have reached the server.
    let delivery = if source.is_connect() {
        Delivery::NotSent
    } else {
        Delivery::MaybeSent
    };
    Error::Transport {
        delivery,
        timed_out: source.is_timeout(),
        source,
    }
}

fn decode<T: DeserializeOwned>(body: &str) -> Result<T> {
    let text = if body.trim().is_empty() { "{}" } else { body };
    serde_json::from_str(text).map_err(|source| Error::Decode {
        source,
        body: truncate_body(body.to_owned()),
    })
}

/// Substitutes `{name}` placeholders and appends the path to `base`, letting
/// the `url` crate percent-encode each segment.
pub(crate) fn build_url(base: &Url, template: &str, params: &[(&str, &str)]) -> Result<Url> {
    let mut url = base.clone();
    {
        let mut segments = url
            .path_segments_mut()
            .map_err(|_| Error::Config("base URL cannot be a base".into()))?;
        segments.pop_if_empty();
        for part in template.split('/').filter(|p| !p.is_empty()) {
            match part.strip_prefix('{').and_then(|p| p.strip_suffix('}')) {
                Some(name) => {
                    let value = params
                        .iter()
                        .find(|(n, _)| *n == name)
                        .map(|(_, v)| *v)
                        .ok_or_else(|| {
                            Error::InvalidRequest(format!("missing path parameter {name}"))
                        })?;
                    if value.is_empty() || value == "." || value == ".." {
                        return Err(Error::InvalidRequest(format!(
                            "path parameter {name} must not be empty or a dot segment"
                        )));
                    }
                    segments.push(value);
                }
                None => {
                    segments.push(part);
                }
            }
        }
    }
    Ok(url)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn url_building_encodes_segments() {
        let base = Url::parse("https://demo.ironbeamapi.com/v2").unwrap();
        let url = build_url(
            &base,
            "/order/{accountId}/cancel/{orderId}",
            &[("accountId", "5123"), ("orderId", "a/b c?")],
        )
        .unwrap();
        assert_eq!(
            url.as_str(),
            "https://demo.ironbeamapi.com/v2/order/5123/cancel/a%2Fb%20c%3F"
        );
        let url = build_url(
            &base,
            "/info/symbol/search/options/{symbol}",
            &[("symbol", "XCME:ES.Z26")],
        )
        .unwrap();
        assert_eq!(url.path(), "/v2/info/symbol/search/options/XCME:ES.Z26");
        let trailing = Url::parse("http://localhost:1/v2/").unwrap();
        assert_eq!(
            build_url(&trailing, "/auth", &[]).unwrap().path(),
            "/v2/auth"
        );
    }

    #[test]
    fn url_building_rejects_empty_and_missing_params() {
        let base = Url::parse("https://demo.ironbeamapi.com/v2").unwrap();
        assert!(build_url(&base, "/account/{accountId}/risk", &[("accountId", "")]).is_err());
        assert!(build_url(&base, "/account/{accountId}/risk", &[("accountId", "..")]).is_err());
        assert!(build_url(&base, "/account/{accountId}/risk", &[]).is_err());
    }

    #[test]
    fn credentials_debug_is_redacted_and_wire_matches_schema() {
        let c = Credentials::enterprise("user1", "hunter2", "key-123");
        let dbg = format!("{c:?}");
        assert!(
            !dbg.contains("hunter2") && !dbg.contains("key-123"),
            "{dbg}"
        );
        assert_eq!(
            c.to_wire(),
            json!({"username": "user1", "password": "hunter2", "apikey": "key-123"})
        );
        assert_eq!(
            Credentials::api_key("user1", "k").to_wire(),
            json!({"username": "user1", "apikey": "k"})
        );
    }
}
