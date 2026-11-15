#![doc = include_str!("../README.md")]
#![forbid(unsafe_code)]
#![warn(missing_docs, rust_2018_idioms, unreachable_pub)]

mod client;
mod config;
mod error;
mod tls;

pub mod endpoints;
pub mod models;
pub mod orders;
pub mod rest;
pub mod stream;

pub use client::{Client, ClientBuilder, Credentials};
pub use config::{ClientConfig, EnumEncoding, Environment, RetryPolicy};
pub use error::{Delivery, Error, Result};
