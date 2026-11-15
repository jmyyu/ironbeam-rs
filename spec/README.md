# Local specification for protocol tests

The SDK was written and verified against Ironbeam's OpenAPI 3.0.3 document
(`info.version` 2.1), obtained on 2026-11-12. The document is not distributed
with this repository or crate and is excluded from the implementation's
MIT OR Apache-2.0 license grant.

Before running `cargo test` or `cargo test --test protocol`:

1. Open <https://docs.ironbeamapi.com/>.
2. Use the **Download OpenAPI specification** control to download the JSON.
3. Save it as `spec/openapi.json` in this repository.

The local file is ignored by Git and excluded from Cargo packages. The SDK
does not need it at build time or runtime; `tests/protocol.rs` reads it to
validate requests and generate example responses. Once downloaded, the
protocol tests run without network access.

The specification remains Ironbeam's property and subject to its applicable
terms. A newer specification may expose API changes that require SDK or test
updates; the original verification report refers to the version above.
