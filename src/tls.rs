//! TLS configuration shared by the HTTP client and the WebSocket connector.
//!
//! The configuration is built with an explicit crypto provider (ring) and the
//! Mozilla root set (`webpki-roots`), so the crate neither depends on nor
//! modifies rustls' process-wide default provider.

use std::sync::Arc;

pub(crate) fn client_config() -> Arc<rustls::ClientConfig> {
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let mut roots = rustls::RootCertStore::empty();
    roots.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
    let config = rustls::ClientConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()
        .expect("ring supports the default TLS protocol versions")
        .with_root_certificates(roots)
        .with_no_client_auth();
    Arc::new(config)
}

#[cfg(test)]
mod tests {
    #[test]
    fn config_builds_without_a_process_default_provider() {
        let cfg = super::client_config();
        assert!(cfg.alpn_protocols.is_empty());
    }

    /// Network test (not run by default, and it does not touch Ironbeam):
    /// proves that the TLS stack — explicit ring provider, bundled roots —
    /// completes real handshakes for both the HTTP client and the WebSocket
    /// connector. Run with `cargo test --lib -- --ignored`.
    #[tokio::test]
    #[ignore = "needs internet access"]
    async fn tls_stack_completes_real_handshakes() {
        use tokio_tungstenite::tungstenite::Error as WsError;

        let http = reqwest::Client::builder()
            .tls_backend_preconfigured((*super::client_config()).clone())
            .build()
            .unwrap();
        let response = http.get("https://www.rust-lang.org/").send().await.unwrap();
        assert!(response.status().is_success(), "{}", response.status());

        // The host does not speak WebSocket: a clean HTTP refusal proves the
        // TLS layer underneath worked.
        let connector = tokio_tungstenite::Connector::Rustls(super::client_config());
        let result = tokio_tungstenite::connect_async_tls_with_config(
            "wss://www.rust-lang.org/",
            None,
            true,
            Some(connector),
        )
        .await;
        assert!(
            matches!(result, Err(WsError::Http(_))),
            "{:?}",
            result.map(|_| ())
        );
    }
}
