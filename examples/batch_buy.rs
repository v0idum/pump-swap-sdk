//! Send a single buy from each of N wallets, paced by an interval.
//!
//! Demonstrates: building a buy tx with compute-budget instructions,
//! orchestrating a paced batch across multiple keypairs, optional Jito-bundle
//! submission via [`pump_swap_sdk::JitoPool`].
//!
//! Run:
//!   RPC_URL=https://api.mainnet-beta.solana.com \
//!   POOL=<pool_pubkey> \
//!   KEYPAIRS=<base58_secret_1>,<base58_secret_2>,... \
//!   AMOUNT_LAMPORTS=500000 \
//!   INTERVAL_MS=700 \
//!   SLIPPAGE=0.1 \
//!   cargo run --example batch_buy
//!
//! Optional: `JITO_ENDPOINTS=https://mainnet.block-engine.jito.wtf,https://...`
//! to send via Jito bundles instead of plain RPC.

use std::str::FromStr;
use std::sync::Arc;

use anyhow::{Context, Result, anyhow};
use solana_client::nonblocking::rpc_client::RpcClient;
use solana_commitment_config::CommitmentConfig;
use solana_compute_budget_interface::ComputeBudgetInstruction;
use solana_sdk::pubkey::Pubkey;
use solana_sdk::signature::{Keypair, Signer};
use solana_sdk::transaction::Transaction;
use tokio::time::{Duration, sleep};

use pump_swap_sdk::{JitoPool, PumpSwapClient, calc_amount_out, send_jito_bundle};

#[tokio::main]
async fn main() -> Result<()> {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();

    let rpc_url = std::env::var("RPC_URL").context("RPC_URL not set")?;
    let pool = Pubkey::from_str(&std::env::var("POOL").context("POOL not set")?)?;
    let keypairs = parse_keypairs(&std::env::var("KEYPAIRS").context("KEYPAIRS not set")?)?;
    let amount_in: u64 = std::env::var("AMOUNT_LAMPORTS")
        .context("AMOUNT_LAMPORTS not set")?
        .parse()?;
    let interval_ms: u64 = std::env::var("INTERVAL_MS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(700);
    let slippage: f64 = std::env::var("SLIPPAGE")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(0.1);

    let rpc = Arc::new(RpcClient::new_with_commitment(
        rpc_url,
        CommitmentConfig::processed(),
    ));
    let client = PumpSwapClient::new(Arc::clone(&rpc));
    let pool_info = client.load_pool(&pool).await?;
    println!(
        "pool ok: base={} quote={} wallets={}",
        pool_info.base_mint,
        pool_info.quote_mint,
        keypairs.len()
    );

    let jito = parse_jito_pool();

    for (i, payer) in keypairs.iter().enumerate() {
        if i > 0 {
            sleep(Duration::from_millis(interval_ms)).await;
        }
        let (base_reserve, quote_reserve) = client.fetch_pool_reserves(&pool_info).await?;
        let amount_out = calc_amount_out(amount_in, quote_reserve, base_reserve, slippage);
        let mut ixs = vec![
            ComputeBudgetInstruction::set_compute_unit_limit(1_000_000),
            ComputeBudgetInstruction::set_compute_unit_price(100_000),
        ];
        ixs.extend(client.build_buy_ixs(
            amount_out,
            amount_in,
            true, // track_volume
            &pool_info,
            &payer.pubkey(),
            true,
        )?);
        let mut tx = Transaction::new_with_payer(&ixs, Some(&payer.pubkey()));
        tx.sign(&[payer], rpc.get_latest_blockhash().await?);

        match &jito {
            Some(pool) => match send_jito_bundle(vec![tx], pool.next_client().await).await {
                Ok(()) => println!("[{}] {} jito-submitted", i + 1, payer.pubkey()),
                Err(e) => eprintln!("[{}] {} jito error: {e}", i + 1, payer.pubkey()),
            },
            None => match rpc.send_and_confirm_transaction(&tx).await {
                Ok(sig) => println!("[{}] {} sig={sig}", i + 1, payer.pubkey()),
                Err(e) => eprintln!("[{}] {} rpc error: {e}", i + 1, payer.pubkey()),
            },
        }
    }
    Ok(())
}

fn parse_keypairs(csv: &str) -> Result<Vec<Keypair>> {
    csv.split(',')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(|s| {
            let bytes = solana_sdk::bs58::decode(s)
                .into_vec()
                .context("base58 decode")?;
            Keypair::try_from(&bytes[..]).map_err(|e| anyhow!("keypair: {e}"))
        })
        .collect()
}

fn parse_jito_pool() -> Option<Arc<JitoPool>> {
    let endpoints = std::env::var("JITO_ENDPOINTS").ok()?;
    let refs: Vec<&str> = endpoints
        .split(',')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .collect();
    if refs.is_empty() {
        return None;
    }
    Some(Arc::new(
        JitoPool::new(&refs, None, Duration::from_millis(0)).ok()?,
    ))
}
