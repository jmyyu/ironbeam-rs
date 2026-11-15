//! **Places a real order** (on the demo environment unless told otherwise),
//! demonstrates the three possible outcomes of a submission, and cancels the
//! order again.
//!
//! ```text
//! IRONBEAM_USERNAME=... IRONBEAM_API_KEY=... \
//!     cargo run --example place_order -- XCME:ES.Z26 buy 1 4000.00
//! ```
//!
//! Pick a limit price far from the market if you do not want a fill. Running
//! against `IRONBEAM_ENV=live` additionally requires
//! `IRONBEAM_CONFIRM_LIVE_ORDER=yes`.

mod common;

use std::time::Duration;

use ironbeam_sdk::models::{OrderRequest, OrderSide};
use ironbeam_sdk::orders::{Investigation, OrderManager};
use ironbeam_sdk::{Environment, Error};

#[tokio::main]
async fn main() -> Result<(), common::BoxError> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let [symbol, side, quantity, price] = args.as_slice() else {
        return Err("usage: place_order <SYMBOL> <buy|sell> <QUANTITY> <LIMIT_PRICE>".into());
    };
    let side = match side.to_ascii_lowercase().as_str() {
        "buy" => OrderSide::Buy,
        "sell" => OrderSide::Sell,
        other => return Err(format!("side must be buy or sell, got {other}").into()),
    };
    if common::environment()? == Environment::Live
        && std::env::var("IRONBEAM_CONFIRM_LIVE_ORDER").as_deref() != Ok("yes")
    {
        return Err(
            "refusing to place a LIVE order without IRONBEAM_CONFIRM_LIVE_ORDER=yes".into(),
        );
    }

    let client = common::client()?;
    let account = common::account(&client).await?;
    let orders = OrderManager::new(client, account.clone());
    let request = OrderRequest::limit(symbol.as_str(), side, quantity.parse()?, price.parse()?);
    println!("submitting to {account}: {request:?}");

    let order_id = match orders.submit(request).await {
        // 1. Accepted.
        Ok(accepted) => {
            println!(
                "accepted: strategy id {:?}, order id {:?}",
                accepted.strategy_id, accepted.order_id
            );
            match (accepted.order_id, accepted.strategy_id) {
                (Some(id), _) => Some(id),
                (None, Some(sid)) => {
                    orders
                        .resolve_order_id(sid, 10, Duration::from_secs(1))
                        .await?
                }
                (None, None) => None,
            }
        }
        // 2. Unknown: the order may be live. Never resubmit; find out.
        Err(Error::OrderOutcomeUnknown(unknown)) => {
            println!(
                "outcome unknown ({}); investigating instead of retrying",
                unknown.cause
            );
            match orders
                .investigate_until_found(&unknown, 5, Duration::from_secs(2))
                .await?
            {
                Investigation::Found(order) => {
                    println!(
                        "the order did go through: {} [{}]",
                        order.order_id, order.status
                    );
                    Some(order.order_id)
                }
                Investigation::NotFound => {
                    println!(
                        "no matching order is visible; treat as not placed only after manual review"
                    );
                    None
                }
                Investigation::Ambiguous(candidates) => {
                    println!("{} orders match; manual review required:", candidates.len());
                    for c in &candidates {
                        println!("  {} [{}]", c.order_id, c.status);
                    }
                    None
                }
            }
        }
        // 3. Definitely not placed.
        Err(e) => return Err(format!("order was not placed: {e}").into()),
    };

    if let Some(id) = order_id {
        tokio::time::sleep(Duration::from_secs(2)).await;
        match orders.cancel(&id).await {
            Ok(response) => {
                for o in response.orders {
                    println!("cancelled {} [{}]", o.order_id, o.status);
                }
            }
            Err(e) if e.is_ambiguous() => {
                println!("cancel outcome unknown ({e}); re-reading state");
                orders.reconcile().await?;
                println!(
                    "order is now: {:?}",
                    orders.order(&id).map(|t| t.order.status)
                );
            }
            Err(e) => println!("cancel refused: {e}"),
        }
    }
    Ok(())
}
