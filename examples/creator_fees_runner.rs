//! One cycle of the creator-fee claim flow: claim accrued fees from the
//! coin-creator vault, then sweep the admin's surplus SOL to a target wallet.
//!
//! Demonstrates: [`PumpSwapClient::withdraw_creator_fees`] and the
//! [`find_coin_creator_vault_authority`] helper.
//!
//! Run:
//!   RPC_URL=https://api.mainnet-beta.solana.com \
//!   ADMIN_KEYPAIR=<base58_secret> \
//!   COIN_CREATOR=<pubkey> \
//!   TOKEN_MINT=<pubkey> \
//!   BONDING_CURVE=<pubkey> \
//!   SHARING_CONFIG=<pubkey> \
//!   TARGET=<destination_pubkey> \
//!   MIN_CLAIMABLE_SOL=0.5 \
//!   RESERVE_SOL=0.01 \
//!   cargo run --example creator_fees_runner

use std::str::FromStr;

use anyhow::{Context, Result};
use solana_client::nonblocking::rpc_client::RpcClient;
use solana_commitment_config::CommitmentConfig;
use solana_native_token::{LAMPORTS_PER_SOL, sol_str_to_lamports};
use solana_sdk::pubkey::Pubkey;
use solana_sdk::signature::{Keypair, Signer};
use solana_sdk::transaction::Transaction;
use solana_system_interface::instruction as system_instruction;

use pump_swap_sdk::{
    PumpSwapClient, WRAPPED_SOL_MINT, find_coin_creator_vault_authority, get_token_balance,
};

#[tokio::main]
async fn main() -> Result<()> {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();

    let rpc_url = std::env::var("RPC_URL").context("RPC_URL not set")?;
    let admin = Keypair::from_base58_string(
        &std::env::var("ADMIN_KEYPAIR").context("ADMIN_KEYPAIR (base58) not set")?,
    );
    let coin_creator = pk("COIN_CREATOR")?;
    let token_mint = pk("TOKEN_MINT")?;
    let bonding_curve = pk("BONDING_CURVE")?;
    let sharing_config = pk("SHARING_CONFIG")?;
    let target = pk("TARGET")?;
    let min_claimable_lamports = match std::env::var("MIN_CLAIMABLE_SOL") {
        Ok(s) => sol_str_to_lamports(&s).context("MIN_CLAIMABLE_SOL is not a valid SOL amount")?,
        Err(_) => LAMPORTS_PER_SOL / 2, // 0.5 SOL
    };
    let reserve_lamports = match std::env::var("RESERVE_SOL") {
        Ok(s) => sol_str_to_lamports(&s).context("RESERVE_SOL is not a valid SOL amount")?,
        Err(_) => LAMPORTS_PER_SOL / 100, // 0.01 SOL
    };

    let rpc = RpcClient::new_with_commitment(rpc_url, CommitmentConfig::confirmed());
    let client = PumpSwapClient::new(&rpc);

    let vault_authority = find_coin_creator_vault_authority(&coin_creator);
    let claimable_lamports =
        get_token_balance(&rpc, &vault_authority, &WRAPPED_SOL_MINT, &spl_token::ID)
            .await?
            .unwrap_or(0);
    println!("vault claimable: {claimable_lamports} lamports");

    if claimable_lamports >= min_claimable_lamports {
        client
            .withdraw_creator_fees(
                &admin,
                &coin_creator,
                &token_mint,
                &bonding_curve,
                &sharing_config,
            )
            .await?;
        println!("claimed creator fees");
    } else {
        println!("below threshold ({min_claimable_lamports} lamports); skipping claim");
    }

    let admin_balance = rpc.get_balance(&admin.pubkey()).await?;
    let fee_estimate: u64 = 5_000;
    let transfer = admin_balance.saturating_sub(reserve_lamports.saturating_add(fee_estimate));
    if transfer == 0 {
        println!("nothing to sweep (balance {admin_balance}, reserve {reserve_lamports})");
        return Ok(());
    }

    let ix = system_instruction::transfer(&admin.pubkey(), &target, transfer);
    let mut tx = Transaction::new_with_payer(&[ix], Some(&admin.pubkey()));
    tx.sign(&[&admin], rpc.get_latest_blockhash().await?);
    let sig = rpc.send_and_confirm_transaction(&tx).await?;
    println!("swept {transfer} lamports → {target} sig={sig}");
    Ok(())
}

fn pk(env: &str) -> Result<Pubkey> {
    let raw = std::env::var(env).with_context(|| format!("{env} not set"))?;
    Pubkey::from_str(&raw).with_context(|| format!("{env} parse"))
}
