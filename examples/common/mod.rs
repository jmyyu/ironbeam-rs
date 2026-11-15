//! Shared setup for the examples: builds a client from environment variables.
//!
//! | Variable | Meaning |
//! |---|---|
//! | `IRONBEAM_USERNAME` | account / user name (required) |
//! | `IRONBEAM_API_KEY` | API key (required unless a password is given) |
//! | `IRONBEAM_PASSWORD` | password (Enterprise logins use both) |
//! | `IRONBEAM_ENV` | `demo` (default) or `live` |
//! | `IRONBEAM_ACCOUNT` | account id; defaults to the first account of the user |
#![allow(dead_code)]

use ironbeam_sdk::{Client, Credentials, Environment};

pub type BoxError = Box<dyn std::error::Error + Send + Sync>;

pub fn environment() -> Result<Environment, BoxError> {
    match std::env::var("IRONBEAM_ENV")
        .unwrap_or_else(|_| "demo".into())
        .as_str()
    {
        "demo" => Ok(Environment::Demo),
        "live" => Ok(Environment::Live),
        other => Err(format!("IRONBEAM_ENV must be `demo` or `live`, got `{other}`").into()),
    }
}

pub fn client() -> Result<Client, BoxError> {
    let username = std::env::var("IRONBEAM_USERNAME").map_err(|_| "set IRONBEAM_USERNAME")?;
    let api_key = std::env::var("IRONBEAM_API_KEY").ok();
    let password = std::env::var("IRONBEAM_PASSWORD").ok();
    let credentials = match (password, api_key) {
        (Some(p), Some(k)) => Credentials::enterprise(username, p, k),
        (None, Some(k)) => Credentials::api_key(username, k),
        (Some(p), None) => Credentials::password(username, p),
        (None, None) => return Err("set IRONBEAM_API_KEY and/or IRONBEAM_PASSWORD".into()),
    };
    Ok(Client::builder()
        .environment(environment()?)
        .credentials(credentials)
        .build()?)
}

pub async fn account(client: &Client) -> Result<String, BoxError> {
    if let Ok(account) = std::env::var("IRONBEAM_ACCOUNT") {
        return Ok(account);
    }
    let accounts = client.all_accounts().await?.accounts;
    accounts
        .into_iter()
        .next()
        .ok_or_else(|| "the user has no accounts".into())
}
