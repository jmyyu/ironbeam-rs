//! Read-only: prints the user's accounts with balances, positions, risk and
//! open orders.
//!
//! ```text
//! IRONBEAM_USERNAME=... IRONBEAM_API_KEY=... cargo run --example account_snapshot
//! ```

mod common;

use ironbeam_sdk::models::{BalanceType, OrderStatus};

#[tokio::main]
async fn main() -> Result<(), common::BoxError> {
    let client = common::client()?;
    let trader = client.trader_info(None).await?;
    println!("trader {:?} (live: {:?})", trader.trader_id, trader.is_live);

    for account in client.all_accounts().await?.accounts {
        println!("\naccount {account}");
        for b in client
            .account_balance(&account, BalanceType::CurrentOpen)
            .await?
            .balances
        {
            println!(
                "  balance  {}: cash {:?}, open trade equity {:?}, net liquidity {:?}",
                b.currency_code.as_deref().unwrap_or("?"),
                b.cash_balance,
                b.open_trade_equity,
                b.net_liquidity
            );
        }
        for p in client.account_positions(&account).await?.positions {
            println!(
                "  position {} {:?} @ {:?} (unrealized {:?})",
                p.exch_sym.as_deref().unwrap_or("?"),
                p.signed_quantity(),
                p.price,
                p.unrealized_pl
            );
        }
        for r in client.account_risk(&account).await?.risks {
            println!(
                "  risk     liquidation at {:?}, current value {:?}, liquidated: {:?}",
                r.liquidation_value,
                r.current_net_liquidation_value,
                r.is_liquidated()
            );
        }
        for o in client.orders(&account, OrderStatus::Any).await?.orders {
            if o.status.is_open() {
                println!(
                    "  order    {} {:?} {:?} {} @ {:?} [{}]",
                    o.order_id,
                    o.side,
                    o.quantity,
                    o.exch_sym.as_deref().unwrap_or("?"),
                    o.limit_price.or(o.stop_price),
                    o.status
                );
            }
        }
    }
    client.logout().await?;
    Ok(())
}
