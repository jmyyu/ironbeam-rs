//! Read-only: streams quotes and trades for the given symbols until Ctrl-C,
//! surviving disconnects.
//!
//! ```text
//! IRONBEAM_USERNAME=... IRONBEAM_API_KEY=... \
//!     cargo run --example stream_quotes -- XCME:ES.Z26 XCME:NQ.Z26
//! ```

mod common;

use ironbeam_sdk::stream::{Stream, StreamConfig, StreamEvent};

#[tokio::main]
async fn main() -> Result<(), common::BoxError> {
    let symbols: Vec<String> = std::env::args().skip(1).collect();
    if symbols.is_empty() {
        return Err("usage: stream_quotes <SYMBOL>...   (e.g. XCME:ES.Z26)".into());
    }
    let client = common::client()?;
    let mut stream = Stream::connect(&client, StreamConfig::default()).await?;
    let handle = stream.handle();
    // Subscriptions are remembered and restored after every reconnect.
    handle.subscribe_quotes(&symbols).await?;
    handle.subscribe_trades(&symbols).await?;

    loop {
        let event = tokio::select! {
            event = stream.next() => event,
            _ = tokio::signal::ctrl_c() => break,
        };
        match event {
            None => break,
            Some(StreamEvent::Connected { generation, .. }) => {
                println!("-- connected (#{generation})")
            }
            Some(StreamEvent::Disconnected { reason, retry_in }) => {
                println!("-- disconnected: {reason:?}; retrying in {retry_in:?}")
            }
            Some(StreamEvent::Closed { error }) => {
                println!("-- stream closed for good: {error}");
                break;
            }
            Some(StreamEvent::Data(message)) => {
                for q in &message.quotes {
                    println!(
                        "quote {:<14} bid {:?} x {:?}   ask {:?} x {:?}   last {:?}",
                        q.symbol.as_deref().unwrap_or("?"),
                        q.bid,
                        q.bid_size,
                        q.ask,
                        q.ask_size,
                        q.last
                    );
                }
                for t in &message.trades {
                    println!(
                        "trade {:<14} {:?} @ {:?} ({:?})",
                        t.symbol.as_deref().unwrap_or("?"),
                        t.size,
                        t.price,
                        t.aggressor_side
                    );
                }
            }
            Some(other) => println!("-- {other:?}"),
        }
    }
    handle.shutdown();
    Ok(())
}
