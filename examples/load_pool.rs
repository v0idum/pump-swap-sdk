//! Load a pool's PoolInfo and current reserves.
//!
//! Run:
//!   RPC_URL=https://api.mainnet-beta.solana.com \
//!   POOL=<pool_pubkey> \
//!   cargo run --example load_pool

use std::str::FromStr;
use std::sync::Arc;

use anyhow::{Context, Result};
use solana_client::nonblocking::rpc_client::RpcClient;
use solana_sdk::commitment_config::CommitmentConfig;
use solana_sdk::pubkey::Pubkey;

use pump_swap_sdk::{load_pool, PumpSwapClient};

#[tokio::main]
async fn main() -> Result<()> {
    let rpc_url = std::env::var("RPC_URL").context("RPC_URL not set")?;
    let pool = std::env::var("POOL").context("POOL not set")?;

    let rpc = Arc::new(RpcClient::new_with_commitment(
        rpc_url,
        CommitmentConfig::confirmed(),
    ));
    let pool_info = load_pool(&Pubkey::from_str(&pool)?, &rpc).await?;

    println!("pool: {}", pool_info.pool);
    println!("  base_mint:               {}", pool_info.base_mint);
    println!("  quote_mint:              {}", pool_info.quote_mint);
    println!("  lp_mint:                 {}", pool_info.lp_mint);
    println!("  pool_base_token_account: {}", pool_info.pool_base_token_account);
    println!("  pool_quote_token_account:{}", pool_info.pool_quote_token_account);
    println!("  creator:                 {}", pool_info.creator);
    println!("  coin_creator:            {}", pool_info.coin_creator);

    let client = PumpSwapClient::new(Arc::clone(&rpc));
    let (base, quote) = client.fetch_pool_reserves(&pool_info).await?;
    println!("\nreserves: base={base}, quote={quote}");
    Ok(())
}
