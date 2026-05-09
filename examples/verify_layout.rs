//! Verify the SDK's buy/sell instruction layouts against the live program by
//! running RPC `simulateTransaction` with sigVerify=false and
//! replaceRecentBlockhash=true. No signing keypair is required: pass a real
//! funded wallet pubkey via USER and simulation will execute the program
//! against current pool state without submitting anything.
//!
//! Run:
//!   RPC_URL=https://api.mainnet-beta.solana.com \
//!   POOL=<pool_pubkey> \
//!   USER=<user_pubkey_with_sol_balance> \
//!   cargo run --example verify_layout

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

use pump_swap_sdk::{calc_amount_out, PumpSwapClient};

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

    // Use the client's load_pool (honors `with_token_program(...)` if set).
    let client = PumpSwapClient::new(Arc::clone(&rpc));
    let pool_info = client.load_pool(&pool_pubkey).await?;
    println!(
        "pool ok: base_mint={} quote_mint={} cashback={} base_prog={} quote_prog={}",
        pool_info.base_mint,
        pool_info.quote_mint,
        pool_info.is_cashback_coin,
        pool_info.base_token_program,
        pool_info.quote_token_program,
    );
    let (base_reserve, quote_reserve) = client.fetch_pool_reserves(&pool_info).await?;
    println!("reserves: base={base_reserve} quote={quote_reserve}");

    // Use 0.01 SOL with realistic slippage so the program math succeeds.
    let amount_in = sol_to_lamports(0.01);
    let base_amount_out = calc_amount_out(amount_in, quote_reserve, base_reserve, 0.1);
    println!("buy_amount_in={amount_in} base_amount_out={base_amount_out}");
    let buy_ixs = client.build_buy_ixs(base_amount_out, amount_in, &pool_info, &user_pubkey, true)?;
    println!("\nBUY: {} accounts in pump-amm ix", buy_ixs[buy_ixs.len() - 2].accounts.len());

    let msg = Message::new(&buy_ixs, Some(&user_pubkey));
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

    println!("\n=== BUY SIMULATION ===");
    if let Some(err) = &result.value.err {
        println!("ERROR: {err:?}");
    } else {
        println!("SUCCESS");
    }
    if let Some(units) = result.value.units_consumed {
        println!("compute units: {units}");
    }
    if let Some(logs) = &result.value.logs {
        for line in logs {
            println!("  {line}");
        }
    }

    // Sell: 1k base units, 0 minimum quote out — pure layout check.
    let amount_in_base: u64 = 1_000;
    let sell_ixs = client.build_sell_ixs(amount_in_base, 0, &pool_info, &user_pubkey, false)?;
    println!("\nSELL: {} accounts in pump-amm ix", sell_ixs[sell_ixs.len() - 2].accounts.len());

    let msg = Message::new(&sell_ixs, Some(&user_pubkey));
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

    println!("\n=== SELL SIMULATION ===");
    if let Some(err) = &result.value.err {
        println!("ERROR: {err:?}");
    } else {
        println!("SUCCESS");
    }
    if let Some(logs) = &result.value.logs {
        for line in logs {
            println!("  {line}");
        }
    }

    Ok(())
}
