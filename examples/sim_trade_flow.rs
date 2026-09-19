//! Simulate the full trader round-trip — token buy, then token sell — via RPC
//! `simulateTransaction` (sigVerify=false, replaceRecentBlockhash=true), one
//! transaction per leg. Works on either pool orientation and needs no token
//! balance: the sell leg is a layout check, not a fill.
//!
//! Both legs are priced with the fee-aware quotes
//! ([`PumpSwapClient::quote_token_buy`] / `quote_token_sell`), which subtract
//! the pool's live tiered fee, so the slippage below is a slippage budget and
//! nothing more. The sell is quoted off pre-buy reserves and sells only 90% of
//! the buy's floor.
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

use pump_swap_sdk::{PumpSwapClient, create_ata_token_or_not_with_program};

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
    // 0.1% slippage is a real slippage budget now that the quote subtracts the
    // pool's fee; before it was fee-blind, this had to be 20% to pass.
    let quote = client.quote_token_buy(sol_in, &pool_info, 0.001).await?;
    let min_tokens_out = quote.min_amount_out;
    let tokens_to_sell = min_tokens_out * 9 / 10;
    println!(
        "sol_in={sol_in} fee={} expected_out={} min_tokens_out={min_tokens_out} tokens_to_sell={tokens_to_sell}",
        quote.fee, quote.amount_out,
    );

    let buy_ixs =
        client.build_token_buy_ixs(sol_in, min_tokens_out, true, &pool_info, &user_pubkey, true)?;

    // Sell the tokens the buy just received, in the same transaction: the buy
    // creates the token ATA and funds it, so the sell leg exercises the real
    // account layout instead of tripping over a missing account.
    let sell_quote = client
        .quote_token_sell(tokens_to_sell, &pool_info, 0.001)
        .await?;
    println!(
        "tokens_to_sell={tokens_to_sell} fee={} expected_sol_out={} min_sol_out={}",
        sell_quote.fee, sell_quote.amount_out, sell_quote.min_amount_out,
    );
    let sell_ixs = client.build_token_sell_ixs(
        tokens_to_sell,
        sell_quote.min_amount_out,
        &pool_info,
        &user_pubkey,
        false,
    )?;

    // Both legs together overflow a legacy transaction, so they are simulated
    // separately. The sell therefore runs against a wallet that holds none of
    // the token: with the ATA created up front, a clean account layout shows
    // up as an insufficient-funds failure *inside* the token transfer. Any
    // earlier, account-related error would mean a layout bug.
    simulate("TOKEN BUY", &rpc, buy_ixs, &user_pubkey).await?;

    let (_, create_token_ata) = create_ata_token_or_not_with_program(
        &user_pubkey,
        &pool_info.token_mint(),
        &user_pubkey,
        &pool_info.token_program(),
        true,
    );
    let mut sell_leg = create_token_ata.into_iter().collect::<Vec<_>>();
    sell_leg.extend(sell_ixs);
    simulate(
        "TOKEN SELL (expect insufficient-funds, NOT account errors)",
        &rpc,
        sell_leg,
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
