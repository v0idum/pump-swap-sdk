//! Rust SDK for the PumpSwap (pump-amm) AMM on Solana.
//!
//! Build buy / sell / create-pool / withdraw / creator-fee-distribution
//! instructions, derive pool and fee PDAs, simulate or submit swaps via an
//! [`RpcClient`](solana_client::nonblocking::rpc_client::RpcClient), and
//! bundle through Jito.
//!
//! The instruction layouts target the current pump-amm IDL: Buy uses 23
//! accounts (with `coin_creator_vault_*`, volume accumulators, fee config,
//! fee program), Sell uses 21. All non-input accounts are derived internally
//! from `PoolInfo` + `user`.
//!
//! # Quickstart
//! ```no_run
//! use std::sync::Arc;
//! use solana_client::nonblocking::rpc_client::RpcClient;
//! use solana_sdk::commitment_config::CommitmentConfig;
//! use solana_sdk::pubkey::Pubkey;
//! use std::str::FromStr;
//! use pump_swap_sdk::{load_pool, PumpSwapClient};
//!
//! # async fn run(rpc_url: &str, pool: &str) -> anyhow::Result<()> {
//! let rpc = Arc::new(RpcClient::new_with_commitment(
//!     rpc_url.to_string(),
//!     CommitmentConfig::confirmed(),
//! ));
//! let pool_info = load_pool(&Pubkey::from_str(pool)?, &rpc).await?;
//! let client = PumpSwapClient::new(rpc);
//! let (base, quote) = client.fetch_pool_reserves(&pool_info).await?;
//! println!("reserves: base={}, quote={}", base, quote);
//! # Ok(()) }
//! ```

pub mod client;
pub mod constants;
pub mod instruction;
pub mod math;
pub mod state;
pub mod util;

pub use client::{PumpSwapClient, get_token_balance};
pub use constants::{
    BUYBACK_FEE_RECIPIENTS, EVENT_AUTHORITY, FEE_PROGRAM, GLOBAL_CONFIG, GLOBAL_VOLUME_ACCUMULATOR,
    POOL_ACCOUNT_NEW_SIZE, PROTOCOL_FEE_RECIPIENTS, PUMP_CREATOR_VAULT, PUMP_SWAP_PROGRAM_ID,
    PUMPFUN_EVENT_AUTHORITY, PUMPFUN_PROGRAM, RESERVED_FEE_RECIPIENTS, WRAPPED_SOL_MINT,
};
pub use instruction::{
    BuyExactQuoteInInstruction, BuyInstruction, ClaimCashbackInstruction, CreatePoolInstruction,
    DepositInstruction, SellInstruction, WithdrawInstruction, create_pool_instruction,
    create_pool_instruction_with_options, distribute_creator_fees_instruction,
    make_buy_exact_quote_in_instruction, make_buy_instruction, make_claim_cashback_instruction,
    make_claim_token_incentives_instruction, make_close_user_volume_accumulator_instruction,
    make_collect_coin_creator_fee_instruction, make_deposit_instruction,
    make_extend_account_instruction, make_init_user_volume_accumulator_instruction,
    make_sell_instruction, make_sync_user_volume_accumulator_instruction,
    transfer_creator_fees_to_pump_instruction, withdraw_instruction,
};
pub use math::{
    BuyExactOutQuote, SwapQuote, buy_amount_out, calc_amount_out, can_quote_fees,
    constant_product_out, is_tiered_fee_pool, market_cap_lamports, quote_buy_exact_base_out,
    quote_buy_exact_quote_in, quote_sell, sell_amount_out, token_buy_quote, token_sell_quote,
};
pub use state::{FeeConfig, FeeTier, Fees, GlobalConfig, Pool, PoolInfo, TokenSide};
pub use util::{
    JitoPool, calc_lp_mint_pda, calc_pool_pda, calc_pool_pda_with_index,
    calc_user_pool_token_account, clone_keypairs, create_ata_token_or_not,
    create_ata_token_or_not_with_program, fee_config_pda, find_coin_creator_vault_ata,
    find_coin_creator_vault_authority, find_user_vol_accumulator, gen_pubkey_with_seed, load_pool,
    load_pool_with_token_program, pick_buyback_fee_recipient, pick_protocol_fee_recipient,
    pick_protocol_fee_recipient_for_pool, pick_reserved_fee_recipient, pool_v2_pda,
    send_bundle_with_retry, send_jito_bundle, user_volume_accumulator_quote_ata,
};
