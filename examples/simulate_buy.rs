//! Build a buy transaction and simulate it against the live RPC.
//!
//! Run:
//!   RPC_URL=https://api.mainnet-beta.solana.com \
//!   POOL=<pool_pubkey> \
//!   KEYPAIR=<base58_secret> \
//!   AMOUNT_SOL=0.001 \
//!   cargo run --example simulate_buy

use std::str::FromStr;
use std::sync::Arc;

use anyhow::{Context, Result};
use solana_client::nonblocking::rpc_client::RpcClient;
use solana_commitment_config::CommitmentConfig;
use solana_native_token::{LAMPORTS_PER_SOL, sol_str_to_lamports};
use solana_sdk::pubkey::Pubkey;
use solana_sdk::signature::Keypair;

use pump_swap_sdk::{PumpSwapClient, load_pool};

#[tokio::main]
async fn main() -> Result<()> {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();

    let rpc_url = std::env::var("RPC_URL").context("RPC_URL not set")?;
    let pool = std::env::var("POOL").context("POOL not set")?;
    let keypair_b58 = std::env::var("KEYPAIR").context("KEYPAIR (base58 secret) not set")?;
    let amount_lamports = match std::env::var("AMOUNT_SOL") {
        Ok(s) => sol_str_to_lamports(&s).context("AMOUNT_SOL is not a valid SOL amount")?,
        Err(_) => LAMPORTS_PER_SOL / 1_000, // 0.001 SOL
    };

    let rpc = Arc::new(RpcClient::new_with_commitment(
        rpc_url,
        CommitmentConfig::confirmed(),
    ));
    let pool_info = load_pool(&Pubkey::from_str(&pool)?, &rpc).await?;
    println!(
        "pool ok: base_mint={} quote_mint={} coin_creator={}",
        pool_info.base_mint, pool_info.quote_mint, pool_info.coin_creator
    );

    let client = PumpSwapClient::new(Arc::clone(&rpc));
    let payer = Keypair::from_base58_string(&keypair_b58);

    client
        .simulate_buy(&pool_info, amount_lamports, &payer)
        .await?;
    Ok(())
}
