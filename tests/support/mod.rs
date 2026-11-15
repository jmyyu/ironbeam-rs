//! Shared test infrastructure.
#![allow(dead_code, unused_imports)]

pub mod mock;
pub mod spec;

use std::time::Duration;

use ironbeam_sdk::stream::{Stream, StreamEvent};

/// Reads events until `pred` matches, failing the test after 5 seconds.
pub async fn wait_for<F>(stream: &mut Stream, what: &str, mut pred: F) -> StreamEvent
where
    F: FnMut(&StreamEvent) -> bool,
{
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    loop {
        match tokio::time::timeout_at(deadline, stream.next()).await {
            Ok(Some(event)) => {
                if pred(&event) {
                    return event;
                }
            }
            Ok(None) => panic!("stream ended while waiting for {what}"),
            Err(_) => panic!("timed out waiting for {what}"),
        }
    }
}

/// Polls `cond` until it holds, failing the test after 5 seconds.
pub async fn eventually(what: &str, mut cond: impl FnMut() -> bool) {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    while !cond() {
        if tokio::time::Instant::now() > deadline {
            panic!("condition not reached: {what}");
        }
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
}
