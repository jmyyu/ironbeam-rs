//! Read-only: keeps a consistent local view of one account's orders by
//! combining the stream with REST reconciliation, and prints every change
//! and every new fill.
//!
//! ```text
//! IRONBEAM_USERNAME=... IRONBEAM_API_KEY=... cargo run --example order_monitor
//! ```

mod common;

use ironbeam_sdk::orders::{OrderChange, OrderManager};
use ironbeam_sdk::stream::{Stream, StreamConfig, StreamEvent};

#[tokio::main]
async fn main() -> Result<(), common::BoxError> {
    let client = common::client()?;
    let account = common::account(&client).await?;
    let orders = OrderManager::new(client.clone(), account.clone());
    let mut stream = Stream::connect(&client, StreamConfig::default()).await?;
    println!("monitoring orders of account {account}; Ctrl-C to stop");

    loop {
        let event = tokio::select! {
            event = stream.next() => event,
            _ = tokio::signal::ctrl_c() => break,
        };
        let Some(event) = event else { break };
        if let StreamEvent::Closed { error } = &event {
            println!("stream closed for good: {error}");
            break;
        }
        // Applies order updates, de-duplicates fills, and re-reads all orders
        // over REST whenever the stream (re)connects.
        match orders.on_event(&event).await {
            Ok(outcome) => {
                if let Some(report) = &outcome.reconciled {
                    println!(
                        "reconciled: {} change(s), {} unconfirmed, {} open",
                        report.changes.len(),
                        report.missing.len(),
                        orders.open_orders().len()
                    );
                }
                for change in &outcome.changes {
                    match change {
                        OrderChange::Created(o) => {
                            println!("new     {} [{}]", o.order_id, o.status)
                        }
                        OrderChange::Updated { previous, current } => println!(
                            "changed {} [{} -> {}] filled {:?}",
                            current.order_id,
                            previous.status,
                            current.status,
                            current.fill_quantity
                        ),
                    }
                }
                for fill in &outcome.fills {
                    println!(
                        "fill    {} {:?} @ {:?}",
                        fill.order_id.as_deref().unwrap_or("?"),
                        fill.fill_quantity,
                        fill.fill_price
                    );
                }
            }
            // The local view may be behind until the next successful
            // reconciliation; say so rather than hide it.
            Err(e) => eprintln!("could not process {event:?}: {e}"),
        }
    }
    stream.handle().shutdown();
    Ok(())
}
