use crate::constants::{BUYBACK_FEE_RECIPIENTS, FEE_PROGRAM, PROTOCOL_FEE_RECIPIENTS, PUMP_SWAP_PROGRAM_ID};
use rand::seq::IndexedRandom;
use crate::state::{Pool, PoolInfo};
use anyhow::{anyhow, Result};
use base64::engine::general_purpose;
use base64::Engine as _;
use bytemuck::from_bytes;
use jito_sdk_rust::JitoJsonRpcSDK;
use log::info;
use rand::RngCore;
use serde_json::json;
use solana_client::nonblocking::rpc_client::RpcClient;
use solana_sdk::bs58;
use solana_sdk::instruction::Instruction;
use solana_sdk::pubkey::Pubkey;
use solana_sdk::signature::Keypair;
use solana_sdk::transaction::Transaction;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use tokio::sync::Mutex;
use tokio::time::{Duration, Instant};

/// Round-robin pool of Jito JSON-RPC clients with per-region cooldown.
///
/// Each call to [`JitoPool::next_client`] reserves the next slot for one bundle
/// send, optionally waiting if that region is still cooling down.
pub struct JitoPool {
    clients: Vec<Arc<JitoJsonRpcSDK>>,
    last: Mutex<Vec<Instant>>,
    interval: Duration,
    rr: AtomicUsize,
}

impl JitoPool {
    pub fn new(endpoints: &[&str], uuid: Option<String>, interval: Duration) -> Self {
        let clients = endpoints
            .iter()
            .map(|b| Arc::new(JitoJsonRpcSDK::new(&format!("{}/api/v1", b), uuid.clone())))
            .collect::<Vec<_>>();
        let last = Mutex::new(vec![Instant::now(); clients.len()]);
        Self {
            clients,
            last,
            interval,
            rr: AtomicUsize::new(0),
        }
    }

    /// Returns the next available client (round-robin, 1 call = 1 reservation).
    /// Waits if that region is still within its cooldown window.
    pub async fn next_client(&self) -> Arc<JitoJsonRpcSDK> {
        let i = self.rr.fetch_add(1, Ordering::Relaxed) % self.clients.len();

        loop {
            let wait_opt = {
                let mut last = self.last.lock().await;
                let now = Instant::now();
                if now >= last[i] {
                    last[i] = now + self.interval;
                    None
                } else {
                    Some(last[i] - now)
                }
            };

            match wait_opt {
                Some(wait) => tokio::time::sleep(wait).await,
                None => break,
            }
        }

        Arc::clone(&self.clients[i])
    }
}

/// Fetch a pool account from chain and decode it into a [`PoolInfo`],
/// auto-detecting each side's token program from the mint account owners.
///
/// Issues two RPC calls: one for the pool, one batched `getMultipleAccounts`
/// for the base and quote mints. If you already know the token program for
/// every pool you'll touch, prefer
/// [`load_pool_with_token_program`] (or the
/// [`PumpSwapClient::load_pool`](crate::client::PumpSwapClient::load_pool)
/// method when the client was constructed with a pinned token program) to
/// skip the second RPC call.
pub async fn load_pool(pool_pubkey: &Pubkey, rpc: &RpcClient) -> Result<PoolInfo> {
    load_pool_with_token_program(pool_pubkey, rpc, None).await
}

/// Same as [`load_pool`], but `token_program` (when `Some`) is used for both
/// base and quote sides instead of detecting them via `getMultipleAccounts`.
/// Saves one RPC call when the caller knows their pools are uniform.
pub async fn load_pool_with_token_program(
    pool_pubkey: &Pubkey,
    rpc: &RpcClient,
    token_program: Option<Pubkey>,
) -> Result<PoolInfo> {
    let data = rpc.get_account_data(pool_pubkey).await?;
    let pool_size = size_of::<Pool>();
    if data.len() < pool_size + 8 {
        anyhow::bail!(
            "Data size mismatch: expected at least {}, got {}",
            pool_size + 8,
            data.len()
        );
    }
    let pool_data = *from_bytes::<Pool>(&data[8..pool_size + 8]);

    let (base_token_program, quote_token_program) = match token_program {
        Some(p) => (p, p),
        None => {
            let mint_accounts = rpc
                .get_multiple_accounts(&[pool_data.base_mint, pool_data.quote_mint])
                .await?;
            let base = mint_accounts[0]
                .as_ref()
                .map(|a| a.owner)
                .unwrap_or(spl_token::ID);
            let quote = mint_accounts[1]
                .as_ref()
                .map(|a| a.owner)
                .unwrap_or(spl_token::ID);
            (base, quote)
        }
    };

    Ok(PoolInfo {
        pool: *pool_pubkey,
        base_mint: pool_data.base_mint,
        quote_mint: pool_data.quote_mint,
        lp_mint: pool_data.lp_mint,
        pool_base_token_account: pool_data.pool_base_token_account,
        pool_quote_token_account: pool_data.pool_quote_token_account,
        creator: pool_data.creator,
        coin_creator: pool_data.coin_creator,
        is_cashback_coin: pool_data.is_cashback_coin != 0,
        base_token_program,
        quote_token_program,
    })
}

/// Returns the classic SPL Token associated token account for `(owner,
/// mint)`, optionally bundled with an idempotent instruction to create it.
///
/// This is a convenience wrapper around the more general
/// [`create_ata_token_or_not_with_program`] that defaults to
/// `spl_token::ID`. For Token-2022 mints, use the `_with_program` variant
/// (or thread `pool_info.base_token_program` from
/// [`PumpSwapClient::load_pool`](crate::client::PumpSwapClient::load_pool),
/// which auto-detects).
pub fn create_ata_token_or_not(
    payer: &Pubkey,
    mint: &Pubkey,
    owner: &Pubkey,
    create: bool,
) -> (Pubkey, Option<Instruction>) {
    create_ata_token_or_not_with_program(payer, mint, owner, &spl_token::ID, create)
}

/// Returns the associated token account for `(owner, mint)` under the given
/// `token_program` (`spl_token::ID` or `spl_token_2022::ID`), optionally
/// bundled with an idempotent instruction to create it.
pub fn create_ata_token_or_not_with_program(
    payer: &Pubkey,
    mint: &Pubkey,
    owner: &Pubkey,
    token_program: &Pubkey,
    create: bool,
) -> (Pubkey, Option<Instruction>) {
    let token_acc = spl_associated_token_account::get_associated_token_address_with_program_id(
        owner,
        mint,
        token_program,
    );
    if !create {
        return (token_acc, None);
    }
    (
        token_acc,
        Some(
            spl_associated_token_account::instruction::create_associated_token_account_idempotent(
                payer,
                owner,
                mint,
                token_program,
            ),
        ),
    )
}

/// Generates a fresh public key derived from `from_public_key` using a random
/// 32-char seed and returns `(pubkey, seed)`. Used to allocate ephemeral WSOL
/// accounts owned by the caller.
pub fn gen_pubkey_with_seed(from_public_key: &Pubkey) -> Result<(Pubkey, String)> {
    let mut rng = rand::rng();
    let mut seed_bytes = [0u8; 32];
    rng.fill_bytes(&mut seed_bytes);

    let seed = bs58::encode(seed_bytes)
        .into_string()
        .chars()
        .take(32)
        .collect::<String>();

    let public_key = Pubkey::create_with_seed(from_public_key, &seed, &spl_token::id())?;
    Ok((public_key, seed))
}

/// PDA: `["pool", index_le_bytes, creator, base_mint, quote_mint]` under pump-amm.
pub fn calc_pool_pda(creator: &Pubkey, base_mint: &Pubkey, quote_mint: &Pubkey) -> (Pubkey, u8) {
    Pubkey::find_program_address(
        &[
            b"pool",
            &[0u8, 0u8],
            creator.as_ref(),
            base_mint.as_ref(),
            quote_mint.as_ref(),
        ],
        &PUMP_SWAP_PROGRAM_ID,
    )
}

/// PDA: `["pool_lp_mint", pool]` under pump-amm.
pub fn calc_lp_mint_pda(pool: &Pubkey) -> (Pubkey, u8) {
    Pubkey::find_program_address(&[b"pool_lp_mint", pool.as_ref()], &PUMP_SWAP_PROGRAM_ID)
}

/// User's LP token account (Token-2022 ATA over `(creator, lp_mint)`).
pub fn calc_user_pool_token_account(creator: &Pubkey, lp_mint: &Pubkey) -> (Pubkey, u8) {
    Pubkey::find_program_address(
        &[creator.as_ref(), spl_token_2022::ID.as_ref(), lp_mint.as_ref()],
        &spl_associated_token_account::ID,
    )
}

/// Deep-clones a slice of `Keypair`s — `Keypair` itself is not `Clone`.
pub fn clone_keypairs(slice: &[Keypair]) -> Result<Vec<Keypair>> {
    slice
        .iter()
        .map(|kp| {
            Keypair::from_bytes(&kp.to_bytes())
                .map_err(|e| anyhow!("Failed to clone keypair: {}", e))
        })
        .collect()
}

/// Submit a Jito bundle (base64-encoded) via the given Jito SDK client.
pub async fn send_jito_bundle(txs: Vec<Transaction>, jito_sdk: Arc<JitoJsonRpcSDK>) -> Result<()> {
    let serialized_txs = txs
        .iter()
        .map(|tx| Ok(general_purpose::STANDARD.encode(bincode::serialize(tx)?)))
        .collect::<Result<Vec<_>>>()?;
    let bundle = json!(serialized_txs);
    let params = json!([bundle, { "encoding": "base64" }]);
    info!("Sending bundle with {} transaction(s)...", serialized_txs.len());
    let response = jito_sdk.send_bundle(Some(params), None).await?;
    let bundle_uuid = response["result"]
        .as_str()
        .ok_or_else(|| anyhow!("Failed to get bundle UUID from response"))?;
    info!("Bundle sent with UUID: {}", bundle_uuid);
    Ok(())
}

/// Send a Jito bundle through a [`JitoPool`] with retries.
pub async fn send_bundle_with_retry(
    bundle: Vec<Transaction>,
    jito_pool: Arc<JitoPool>,
    max_retries: u64,
) -> Result<()> {
    let mut attempt = 1;
    loop {
        match send_jito_bundle(bundle.clone(), jito_pool.next_client().await).await {
            Ok(_) => {
                info!("Bundle sent successfully on attempt {}", attempt);
                return Ok(());
            }
            Err(e) => {
                attempt += 1;
                if attempt >= max_retries {
                    return Err(anyhow!("Failed to send after {} retries: {:?}", max_retries, e));
                }
                info!("Send failed, retry {}/{}", attempt, max_retries);
                tokio::time::sleep(Duration::from_secs(1)).await;
            }
        }
    }
}

/// PDA: `["user_volume_accumulator", user]` under pump-amm.
pub fn find_user_vol_accumulator(user: &Pubkey) -> Pubkey {
    Pubkey::find_program_address(
        &[b"user_volume_accumulator", user.as_ref()],
        &PUMP_SWAP_PROGRAM_ID,
    )
    .0
}

/// PDA: `["creator_vault", coin_creator]` under pump-amm.
pub fn find_coin_creator_vault_authority(coin_creator: &Pubkey) -> Pubkey {
    Pubkey::find_program_address(
        &[b"creator_vault", coin_creator.as_ref()],
        &PUMP_SWAP_PROGRAM_ID,
    )
    .0
}

/// Standard SPL associated token account for `(authority, quote_mint)` under
/// `quote_token_program`. For pump-amm the quote is typically WSOL on
/// `spl_token::ID`.
pub fn find_coin_creator_vault_ata(
    coin_creator_vault_authority: &Pubkey,
    quote_token_program: &Pubkey,
    quote_mint: &Pubkey,
) -> Pubkey {
    spl_associated_token_account::get_associated_token_address_with_program_id(
        coin_creator_vault_authority,
        quote_mint,
        quote_token_program,
    )
}

/// PDA: `["fee_config", pump_amm_program_id]` under the pump fee program.
pub fn fee_config_pda() -> Pubkey {
    Pubkey::find_program_address(
        &[b"fee_config", PUMP_SWAP_PROGRAM_ID.as_ref()],
        &FEE_PROGRAM,
    )
    .0
}

/// PDA: `["pool-v2", base_mint]` under pump-amm. Required as a remaining
/// account on Buy/Sell when `pool.coin_creator != Pubkey::default()`.
pub fn pool_v2_pda(base_mint: &Pubkey) -> Pubkey {
    Pubkey::find_program_address(&[b"pool-v2", base_mint.as_ref()], &PUMP_SWAP_PROGRAM_ID).0
}

/// ATA owned by `user_volume_accumulator(user)` for the given quote mint and
/// token program. Used as a remaining account on Buy/Sell when the pool is
/// flagged as a cashback coin.
pub fn user_volume_accumulator_quote_ata(
    user: &Pubkey,
    quote_mint: &Pubkey,
    quote_token_program: &Pubkey,
) -> Pubkey {
    spl_associated_token_account::get_associated_token_address_with_program_id(
        &find_user_vol_accumulator(user),
        quote_mint,
        quote_token_program,
    )
}

/// Pick a random `protocol_fee_recipient` from the GlobalConfig list. The
/// program accepts any of the 8.
pub fn pick_protocol_fee_recipient() -> Pubkey {
    *PROTOCOL_FEE_RECIPIENTS
        .choose(&mut rand::rng())
        .expect("PROTOCOL_FEE_RECIPIENTS is non-empty")
}

/// Pick a random `buyback_fee_recipient` from the GlobalConfig list. Required
/// as a remaining account on every Buy and Sell.
pub fn pick_buyback_fee_recipient() -> Pubkey {
    *BUYBACK_FEE_RECIPIENTS
        .choose(&mut rand::rng())
        .expect("BUYBACK_FEE_RECIPIENTS is non-empty")
}
