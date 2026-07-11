//! Simulate the full trader round-trip — token buy then token sell — in one
//! transaction via RPC `simulateTransaction` (sigVerify=false,
//! replaceRecentBlockhash=true). Works on either pool orientation; the sell
//! spends the tokens the buy just received, so no token balance is needed.
//!
//! Run:
//!   RPC_URL=https://api.mainnet-beta.solana.com \
//!   POOL=<pool_pubkey> \
//!   USER=<funded_wallet_pubkey> \
//!   cargo run --example sim_trade_flow

use std::str::FromStr;
use std::sync::Arc;

use anyhow::{Context, Result};
use solana_client::nonblocking::rpc_client::RpcClient;
use solana_client::rpc_config::RpcSimulateTransactionConfig;
use solana_sdk::commitment_config::CommitmentConfig;
use solana_sdk::message::Message;
use solana_sdk::native_token::sol_to_lamports;
use solana_sdk::pubkey::Pubkey;
use solana_sdk::transaction::Transaction;

use pump_swap_sdk::{PumpSwapClient, calc_amount_out};

#[tokio::main]
async fn main() -> Result<()> {
    let rpc_url = std::env::var("RPC_URL").context("RPC_URL not set")?;
    let pool = std::env::var("POOL").context("POOL not set")?;
    let user = std::env::var("USER").context("USER (pubkey) not set")?;

    let rpc = Arc::new(RpcClient::new_with_commitment(
        rpc_url,
        CommitmentConfig::confirmed(),
    ));
    let pool_pubkey = Pubkey::from_str(&pool)?;
    let user_pubkey = Pubkey::from_str(&user)?;

    let client = PumpSwapClient::new(Arc::clone(&rpc));
    let pool_info = client.load_pool(&pool_pubkey).await?;
    let (base_reserve, quote_reserve) = client.fetch_pool_reserves(&pool_info).await?;
    let (sol_reserve, token_reserve) = pool_info.orient_reserves(base_reserve, quote_reserve);
    println!(
        "pool: sol_is_base={} token_mint={} sol_reserve={} token_reserve={}",
        pool_info.sol_is_base(),
        pool_info.token_mint(),
        sol_reserve,
        token_reserve,
    );

    let sol_in = sol_to_lamports(0.001);
    // Expected tokens out with 20% slippage floor, then sell 90% of that floor
    // (the actual fill is above the floor, so the sell amount is guaranteed held).
    let min_tokens_out = calc_amount_out(sol_in, sol_reserve, token_reserve, 0.2);
    let tokens_to_sell = min_tokens_out * 9 / 10;
    println!("sol_in={sol_in} min_tokens_out={min_tokens_out} tokens_to_sell={tokens_to_sell}");

    let buy_ixs =
        client.build_token_buy_ixs(sol_in, min_tokens_out, true, &pool_info, &user_pubkey, true)?;
    simulate("TOKEN BUY", &rpc, buy_ixs, &user_pubkey).await?;

    // Sell sim: the wallet holds no tokens, so a clean layout pass surfaces as
    // an insufficient-funds failure INSIDE the token transfer (all accounts
    // already validated) — anything account-related earlier means layout bug.
    let sell_ixs =
        client.build_token_sell_ixs(tokens_to_sell, 1, &pool_info, &user_pubkey, false)?;
    simulate(
        "TOKEN SELL (expect insufficient-funds, NOT account errors)",
        &rpc,
        sell_ixs,
        &user_pubkey,
    )
    .await?;
    Ok(())
}

async fn simulate(
    label: &str,
    rpc: &RpcClient,
    ixs: Vec<solana_sdk::instruction::Instruction>,
    user: &Pubkey,
) -> Result<()> {
    let msg = Message::new(&ixs, Some(user));
    let mut tx = Transaction::new_unsigned(msg);
    tx.message.recent_blockhash = solana_sdk::hash::Hash::default();

    let result = rpc
        .simulate_transaction_with_config(
            &tx,
            RpcSimulateTransactionConfig {
                sig_verify: false,
                replace_recent_blockhash: true,
                commitment: Some(CommitmentConfig::confirmed()),
                ..RpcSimulateTransactionConfig::default()
            },
        )
        .await?;

    println!("\n=== {label} ===");
    match &result.value.err {
        Some(err) => println!("ERROR: {err:?}"),
        None => println!("SUCCESS"),
    }
    if let Some(units) = result.value.units_consumed {
        println!("compute units: {units}");
    }
    if let Some(logs) = &result.value.logs {
        for line in logs {
            if line.contains("Instruction:") || line.contains("Error") || line.contains("failed") {
                println!("  {line}");
            }
        }
    }
    Ok(())
}
