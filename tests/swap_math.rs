//! Swap-quote math checked against pump-amm's own swap events.
//!
//! Every fixture under `tests/fixtures/swap_events/` is a byte-for-byte
//! `Program data:` payload captured from mainnet, and every fixture under
//! `tests/fixtures/pools/` is a raw pool account from the same session
//! (2026-09-19). Replaying them offline pins the SDK's arithmetic to what the
//! program actually did, with no RPC in CI.
//!
//! The provenance of each fixture — pool, transaction, and the fee schedule
//! that applied — is recorded on its constant below.

use pump_swap_sdk::{
    Fees, PoolInfo, can_quote_fees, is_tiered_fee_pool, quote_buy_exact_base_out,
    quote_buy_exact_quote_in, quote_sell,
};
use solana_sdk::pubkey::Pubkey;

/// Fields of a pump-amm `SellEvent`, read by fixed offset past the 8-byte
/// discriminator. The layout has no variable-length field, so every offset is
/// a constant.
struct SellEvent {
    base_amount_in: u64,
    pool_base_token_reserves: u64,
    pool_quote_token_reserves: u64,
    quote_amount_out: u64,
    user_quote_amount_out: u64,
    lp_fee_basis_points: u64,
    lp_fee: u64,
    protocol_fee_basis_points: u64,
    protocol_fee: u64,
    coin_creator_fee_basis_points: u64,
    coin_creator_fee: u64,
    cashback_fee_basis_points: u64,
    cashback: u64,
}

/// Fields of a pump-amm `BuyEvent`. `ix_name` is a borsh string sitting before
/// the cashback fields, so those two are found by walking past it.
struct BuyEvent {
    base_amount_out: u64,
    pool_base_token_reserves: u64,
    pool_quote_token_reserves: u64,
    quote_amount_in: u64,
    user_quote_amount_in: u64,
    lp_fee_basis_points: u64,
    lp_fee: u64,
    protocol_fee_basis_points: u64,
    protocol_fee: u64,
    coin_creator_fee_basis_points: u64,
    coin_creator_fee: u64,
    cashback_fee_basis_points: u64,
    cashback: u64,
    ix_name: String,
}

fn u64_at(data: &[u8], offset: usize) -> u64 {
    u64::from_le_bytes(data[offset..offset + 8].try_into().unwrap())
}

impl SellEvent {
    fn decode(data: &[u8]) -> Self {
        Self {
            base_amount_in: u64_at(data, 16),
            pool_base_token_reserves: u64_at(data, 48),
            pool_quote_token_reserves: u64_at(data, 56),
            quote_amount_out: u64_at(data, 64),
            lp_fee_basis_points: u64_at(data, 72),
            lp_fee: u64_at(data, 80),
            protocol_fee_basis_points: u64_at(data, 88),
            protocol_fee: u64_at(data, 96),
            user_quote_amount_out: u64_at(data, 112),
            coin_creator_fee_basis_points: u64_at(data, 344),
            coin_creator_fee: u64_at(data, 352),
            cashback_fee_basis_points: u64_at(data, 360),
            cashback: u64_at(data, 368),
        }
    }

    /// The tier's fee schedule. A cashback pool has the tier's creator bps
    /// moved into the cashback slot, so the two are added back together here
    /// — the total, and the per-component rounding, are the tier's.
    fn fees(&self) -> Fees {
        Fees {
            lp_fee_bps: self.lp_fee_basis_points,
            protocol_fee_bps: self.protocol_fee_basis_points,
            creator_fee_bps: self.coin_creator_fee_basis_points + self.cashback_fee_basis_points,
        }
    }

    fn total_fee(&self) -> u64 {
        self.lp_fee + self.protocol_fee + self.coin_creator_fee + self.cashback
    }
}

impl BuyEvent {
    fn decode(data: &[u8]) -> Self {
        let name_len = u32::from_le_bytes(data[401..405].try_into().unwrap()) as usize;
        let ix_name = String::from_utf8(data[405..405 + name_len].to_vec()).unwrap();
        let tail = 405 + name_len;
        Self {
            base_amount_out: u64_at(data, 16),
            pool_base_token_reserves: u64_at(data, 48),
            pool_quote_token_reserves: u64_at(data, 56),
            quote_amount_in: u64_at(data, 64),
            lp_fee_basis_points: u64_at(data, 72),
            lp_fee: u64_at(data, 80),
            protocol_fee_basis_points: u64_at(data, 88),
            protocol_fee: u64_at(data, 96),
            user_quote_amount_in: u64_at(data, 112),
            coin_creator_fee_basis_points: u64_at(data, 344),
            coin_creator_fee: u64_at(data, 352),
            cashback_fee_basis_points: u64_at(data, tail),
            cashback: u64_at(data, tail + 8),
            ix_name,
        }
    }

    fn fees(&self) -> Fees {
        Fees {
            lp_fee_bps: self.lp_fee_basis_points,
            protocol_fee_bps: self.protocol_fee_basis_points,
            creator_fee_bps: self.coin_creator_fee_basis_points + self.cashback_fee_basis_points,
        }
    }

    fn total_fee(&self) -> u64 {
        self.lp_fee + self.protocol_fee + self.coin_creator_fee + self.cashback
    }
}

/// Simulated 0.001 SOL token buy on
/// `2TrDYCA4kEKTGXs622pENzQADGydndXM8zKdDn3x3DeV`, a SOL-base pool — so the
/// token buy is a pump-amm `sell`. Not a pump pool, so the fee program
/// returned `flat_fees`: lp 25 / protocol 5 / creator 0.
const SELL_FLAT_FEE: &[u8] = include_bytes!("fixtures/swap_events/sell_flat_fee_2TrDY.bin");

/// `sell` on `8r76XGVKMyMoBfJ7pE1syhf7xvaeXtZ3Tt4vKBrm4Fx`, tx
/// `4xtAZuxFgdxE8kZMcFUC6CEUE9F5fW8YYB8NLaTV9VfUgvQYJb6AfBm69gyaN4ygHqzgNEwKCq1SD5QEWDTx6Rth`.
/// Tier lp 20 / protocol 5 / creator 95, and the pool carries virtual quote
/// liquidity — the case that is off by orders of magnitude without it.
const SELL_VIRTUAL: &[u8] = include_bytes!("fixtures/swap_events/sell_tiered_virtual_reserves.bin");
const SELL_VIRTUAL_RESERVES: u64 = 17_584_505_289;

/// `sell` on `BhBDxSh74dUgVbDZ78866o9UiPfjcX6n7zBSGPC97ttz`, tx
/// `5jX6vW2jSviH4pqFVtVEFqQspWny42QuUedpgj9Gj4gLuhfd8J8y3e3ezcUQv7PfDqrhj71k3dmsMAe7Fapmc6C5`.
/// A cashback pool: the base tier's 30 creator bps are charged as cashback
/// instead, so `coin_creator_fee_basis_points` reads 0.
const SELL_CASHBACK: &[u8] = include_bytes!("fixtures/swap_events/sell_tiered_cashback.bin");

/// `sell` on `2Y8QhdP4Zox3mTKiJNpCQiFqUncHDE69LKZnTyvFjeqG`, tx
/// `35YcEsUSLkTCqXEVLxAFusydEekWkPKWfTsi4Gyn3SXkQxvjiw8MQogzqDfEXLudnFPxWRxe6b1GVSM7uEdEw1zR`.
/// Base tier, lp 2 / protocol 93 / creator 30 — the 125 bps rung the issue
/// calls out as the one a hardcoded 1% slippage cannot absorb.
const SELL_BASE_TIER: &[u8] = include_bytes!("fixtures/swap_events/sell_tiered_plain.bin");

/// `buy_exact_quote_in` on `3MtkgTLjpeisDEszY1WLAGLj5nS7yWWz9KE3mnjj7Krj`, tx
/// `NiuZNgB7p2PbW3oTW6oZ7ihGtFoddxQZUzvPqQMxWrrdMNxbhgjZsdUAsvgh1jmxcQXLW6ZNfUqAbewD5CS4KMG`.
/// Tier lp 20 / protocol 5 / creator 75, with virtual quote liquidity.
const BUY_QUOTE_IN_VIRTUAL: &[u8] =
    include_bytes!("fixtures/swap_events/buy_exact_quote_in_virtual_reserves.bin");
const BUY_QUOTE_IN_VIRTUAL_RESERVES: u64 = 17_584_505_288;

/// `buy_exact_quote_in` on `2Y8QhdP4Zox3mTKiJNpCQiFqUncHDE69LKZnTyvFjeqG`, tx
/// `3731o63YRxffQx3PzS9pSm6fgpCPfwWg1ViimjE9aaZM2D98nSn8e6dWM3gsPVjtKTPi5WShXWistQc9CNZG3yhY`.
const BUY_QUOTE_IN_PLAIN: &[u8] =
    include_bytes!("fixtures/swap_events/buy_exact_quote_in_plain.bin");

/// `buy` (exact base out) on `HfpiSYjkviLNTHF8FjM8yCPauMwq5px2XMerSZPUKvdT`, tx
/// `37LowDLAUgD7T4qSXLMZnCK7M5JdqWjqorQrEzS2YB7ohExQQXgsC9eLxv3cXShrmT7CWoLhQLaa2reQFNBgp2ab`.
const BUY_EXACT_BASE_OUT: &[u8] = include_bytes!("fixtures/swap_events/buy_exact_base_out.bin");

fn assert_sell_matches(fixture: &[u8], virtual_quote_reserves: u64) {
    let event = SellEvent::decode(fixture);
    let quote = quote_sell(
        event.base_amount_in,
        event.pool_base_token_reserves,
        event.pool_quote_token_reserves + virtual_quote_reserves,
        event.fees(),
        0.0,
    );

    assert_eq!(
        quote.gross_amount_out, event.quote_amount_out,
        "curve output before fees"
    );
    assert_eq!(quote.fee, event.total_fee(), "total fee");
    assert_eq!(
        quote.amount_out, event.user_quote_amount_out,
        "amount credited to the user"
    );
    // With zero slippage the floor is the fill itself: any lower and the
    // caller is giving away protection, any higher and the program reverts.
    assert_eq!(quote.min_amount_out, event.user_quote_amount_out);
}

#[test]
fn sell_quote_matches_the_program_on_a_flat_fee_pool() {
    assert_sell_matches(SELL_FLAT_FEE, 0);
}

#[test]
fn sell_quote_matches_the_program_on_the_base_fee_tier() {
    assert_sell_matches(SELL_BASE_TIER, 0);
}

#[test]
fn sell_quote_matches_the_program_when_the_pool_has_virtual_quote_liquidity() {
    assert_sell_matches(SELL_VIRTUAL, SELL_VIRTUAL_RESERVES);
}

#[test]
fn sell_quote_counts_cashback_as_part_of_the_fee() {
    let event = SellEvent::decode(SELL_CASHBACK);
    assert_ne!(event.cashback, 0, "fixture must exercise the cashback path");
    assert_eq!(event.coin_creator_fee_basis_points, 0);
    assert_sell_matches(SELL_CASHBACK, 0);
}

/// Ignoring the pool's virtual quote liquidity overstates how much base a buy
/// receives — the direction that makes the program reject the resulting
/// `min_out` with `ExceededSlippage`, which is the whole failure this issue is
/// about.
#[test]
fn dropping_virtual_quote_liquidity_overquotes_a_buy() {
    let event = BuyEvent::decode(BUY_QUOTE_IN_VIRTUAL);
    let without = quote_buy_exact_quote_in(
        event.quote_amount_in,
        event.pool_base_token_reserves,
        event.pool_quote_token_reserves,
        event.fees(),
        0.0,
    );
    assert!(
        without.amount_out > event.base_amount_out,
        "expected an over-quote, got {} against the program's {}",
        without.amount_out,
        event.base_amount_out
    );
}

/// Same field, opposite direction: on a sell it understates the proceeds.
/// Harmless for slippage, but it still means the quote is not the program's.
#[test]
fn dropping_virtual_quote_liquidity_underquotes_a_sell() {
    let event = SellEvent::decode(SELL_VIRTUAL);
    let without = quote_sell(
        event.base_amount_in,
        event.pool_base_token_reserves,
        event.pool_quote_token_reserves,
        event.fees(),
        0.0,
    );
    assert!(
        without.amount_out < event.user_quote_amount_out,
        "expected an under-quote, got {} against the program's {}",
        without.amount_out,
        event.user_quote_amount_out
    );
}

fn assert_buy_quote_in_matches(fixture: &[u8], virtual_quote_reserves: u64) {
    let event = BuyEvent::decode(fixture);
    assert_eq!(event.ix_name, "buy_exact_quote_in");
    let quote = quote_buy_exact_quote_in(
        event.quote_amount_in,
        event.pool_base_token_reserves,
        event.pool_quote_token_reserves + virtual_quote_reserves,
        event.fees(),
        0.0,
    );

    assert_eq!(quote.amount_out, event.base_amount_out, "base received");
    assert_eq!(quote.fee, event.total_fee(), "total fee");
    assert_eq!(
        event.quote_amount_in - quote.fee,
        event.user_quote_amount_in,
        "quote left after fees"
    );
    assert_eq!(quote.min_amount_out, event.base_amount_out);
}

#[test]
fn buy_exact_quote_in_matches_the_program() {
    assert_buy_quote_in_matches(BUY_QUOTE_IN_PLAIN, 0);
}

#[test]
fn buy_exact_quote_in_matches_the_program_with_virtual_quote_liquidity() {
    assert_buy_quote_in_matches(BUY_QUOTE_IN_VIRTUAL, BUY_QUOTE_IN_VIRTUAL_RESERVES);
}

#[test]
fn buy_exact_base_out_matches_the_program() {
    let event = BuyEvent::decode(BUY_EXACT_BASE_OUT);
    assert_eq!(event.ix_name, "buy");
    let quote = quote_buy_exact_base_out(
        event.base_amount_out,
        event.pool_base_token_reserves,
        event.pool_quote_token_reserves,
        event.fees(),
        0.0,
    )
    .expect("pool can price this output");

    assert_eq!(quote.quote_amount_in, event.quote_amount_in, "curve input");
    assert_eq!(quote.fee, event.total_fee(), "total fee");
    assert_eq!(
        quote.total_quote_in, event.user_quote_amount_in,
        "total the user spends"
    );
    assert_eq!(quote.max_quote_in, event.user_quote_amount_in);
}

/// Raw pool account for `3MtkgTLjpeisDEszY1WLAGLj5nS7yWWz9KE3mnjj7Krj`,
/// 301 bytes, carrying virtual quote liquidity.
const POOL_VIRTUAL: &[u8] = include_bytes!("fixtures/pools/pool_virtual_quote_reserves.bin");

/// Raw pool account for `9hDd1xry45YaUHjYfZSP5Vef19DMCwJjesAnFy32KJvW`, 271
/// bytes — an account allocated before the current layout.
const POOL_LEGACY: &[u8] = include_bytes!("fixtures/pools/pool_legacy_short.bin");

/// Raw pool account for `2TrDYCA4kEKTGXs622pENzQADGydndXM8zKdDn3x3DeV`, the
/// SOL-base pool the `SELL_FLAT_FEE` event came from.
const POOL_SOL_BASE: &[u8] = include_bytes!("fixtures/pools/pool_sol_base_flat_fee.bin");

fn decode_pool(data: &[u8]) -> PoolInfo {
    PoolInfo::from_account_data(Pubkey::new_unique(), data, spl_token::ID, spl_token::ID)
        .expect("fixture decodes")
}

#[test]
fn pool_decoding_reads_virtual_quote_reserves() {
    assert_eq!(
        decode_pool(POOL_VIRTUAL).virtual_quote_reserves,
        17_584_505_288
    );
}

/// The field sits past the end of accounts allocated under the older layout;
/// those must decode as zero rather than failing or reading neighbouring data.
#[test]
fn pool_decoding_treats_a_short_account_as_having_no_virtual_reserves() {
    let pool = decode_pool(POOL_LEGACY);
    assert_eq!(pool.virtual_quote_reserves, 0);
    assert_eq!(pool.pool_account_data_len, POOL_LEGACY.len());
}

#[test]
fn effective_reserves_add_virtual_liquidity_to_the_quote_side_only() {
    let pool = decode_pool(POOL_VIRTUAL);
    assert_eq!(
        pool.effective_reserves(1_000, 2_000),
        (1_000, 2_000 + 17_584_505_288)
    );

    let sol_base = decode_pool(POOL_SOL_BASE);
    assert_eq!(sol_base.virtual_quote_reserves, 0);
    assert_eq!(sol_base.effective_reserves(1_000, 2_000), (1_000, 2_000));
    assert!(sol_base.sol_is_base());
}

/// Raw pool account for `FtY9RJrMc4EvQBxG2kGr1Sj7WkVXcPAFVrHbso8oztbh`: a
/// pump pool (it carries a `coin_creator`) quoted in something other than
/// SOL. The fee program prices it off neither ladder — live swaps show
/// lp 20 / protocol 5 / creator 300, against a ladder that tops out at 125 —
/// so it is the pool that proves the SDK must not guess here.
const POOL_NON_SOL_QUOTE_PUMP: &[u8] = include_bytes!("fixtures/pools/pool_non_sol_quote_pump.bin");

/// The flag pump-amm passes as `is_pump_pool` tracks the coin creator, not the
/// quote mint: across 614 mainnet swaps over 136 pools, `coin_creator != default`
/// agreed with the flag every time while `quote_mint == WSOL` missed five.
#[test]
fn tiered_fee_pools_are_the_ones_with_a_coin_creator() {
    let non_sol_quote = decode_pool(POOL_NON_SOL_QUOTE_PUMP);
    assert_ne!(non_sol_quote.coin_creator, Pubkey::default());
    assert_ne!(non_sol_quote.quote_mint, pump_swap_sdk::WRAPPED_SOL_MINT);
    assert!(
        is_tiered_fee_pool(&non_sol_quote),
        "a pool with a coin creator is priced off the ladder even when it is not SOL-quoted"
    );

    let with_creator = decode_pool(POOL_VIRTUAL);
    assert_ne!(with_creator.coin_creator, Pubkey::default());
    assert!(is_tiered_fee_pool(&with_creator));

    let without_creator = decode_pool(POOL_SOL_BASE);
    assert_eq!(without_creator.coin_creator, Pubkey::default());
    assert!(!is_tiered_fee_pool(&without_creator));
}

/// The SDK must refuse to price a pool it cannot price, rather than fall back
/// to a ladder that does not apply to it.
#[test]
fn a_pump_pool_quoted_in_another_mint_is_not_quotable() {
    assert!(!can_quote_fees(&decode_pool(POOL_NON_SOL_QUOTE_PUMP)));
    assert!(can_quote_fees(&decode_pool(POOL_VIRTUAL)));
    assert!(can_quote_fees(&decode_pool(POOL_SOL_BASE)));
    assert!(can_quote_fees(&decode_pool(POOL_LEGACY)));
}

/// Live checks against mainnet. Ignored by default so CI stays offline:
///
/// ```text
/// RPC_URL=https://my-private-rpc cargo test --test swap_math -- --ignored
/// ```
///
/// These are the issue's acceptance criteria: a 0.001 SOL buy quoted at 0.1%
/// slippage must simulate cleanly on pools in different fee tiers, where the
/// old fee-blind quote needed 0.3% or more.
mod live {
    use pump_swap_sdk::PumpSwapClient;
    use solana_client::nonblocking::rpc_client::RpcClient;
    use solana_client::rpc_config::RpcSimulateTransactionConfig;
    use solana_sdk::commitment_config::CommitmentConfig;
    use solana_sdk::message::Message;
    use solana_sdk::native_token::sol_to_lamports;
    use solana_sdk::pubkey::Pubkey;
    use solana_sdk::transaction::Transaction;
    use std::str::FromStr;
    use std::sync::Arc;

    /// A funded mainnet account, used only as the simulated fee payer —
    /// simulation needs no signature.
    const SIM_PAYER: &str = "EuNMMpT6Yeh2aydm4PoQ9dWtq8Lk53S6tNnVFq41gFDt";

    /// SOL-base pool, priced off `flat_fees` (30 bps). The issue's headline
    /// repro: a fee-blind quote fails here until slippage reaches 0.3%.
    const SOL_BASE_POOL: &str = "2TrDYCA4kEKTGXs622pENzQADGydndXM8zKdDn3x3DeV";

    /// Token-base pool on the ladder's base rung (125 bps) — four times the
    /// fee of the pool above, and more than the SDK's old 1% default.
    const BASE_TIER_POOL: &str = "2Y8QhdP4Zox3mTKiJNpCQiFqUncHDE69LKZnTyvFjeqG";

    fn client() -> PumpSwapClient<Arc<RpcClient>> {
        let url = std::env::var("RPC_URL")
            .unwrap_or_else(|_| "https://api.mainnet-beta.solana.com".to_string());
        PumpSwapClient::new(Arc::new(RpcClient::new_with_commitment(
            url,
            CommitmentConfig::confirmed(),
        )))
    }

    async fn simulate_token_buy_at(pool: &str, slippage: f64) -> (u64, Option<String>) {
        let client = client();
        let payer = Pubkey::from_str(SIM_PAYER).expect("valid pubkey");
        let pool_info = client
            .load_pool(&Pubkey::from_str(pool).expect("valid pubkey"))
            .await
            .expect("load pool");

        let sol_in = sol_to_lamports(0.001);
        let quote = client
            .quote_token_buy(sol_in, &pool_info, slippage)
            .await
            .expect("quote");

        let ixs = client
            .build_token_buy_ixs(sol_in, quote.min_amount_out, true, &pool_info, &payer, true)
            .expect("build instructions");
        let mut tx = Transaction::new_unsigned(Message::new(&ixs, Some(&payer)));
        tx.message.recent_blockhash = solana_sdk::hash::Hash::default();

        let result = client
            .rpc
            .simulate_transaction_with_config(
                &tx,
                RpcSimulateTransactionConfig {
                    sig_verify: false,
                    replace_recent_blockhash: true,
                    commitment: Some(CommitmentConfig::confirmed()),
                    ..RpcSimulateTransactionConfig::default()
                },
            )
            .await
            .expect("simulate");

        (
            quote.min_amount_out,
            result.value.err.map(|err| format!("{err:?}")),
        )
    }

    #[tokio::test]
    #[ignore = "hits mainnet RPC"]
    async fn flat_fee_pool_buy_simulates_at_one_tenth_percent_slippage() {
        let (min_out, err) = simulate_token_buy_at(SOL_BASE_POOL, 0.001).await;
        assert!(err.is_none(), "min_out={min_out} failed: {err:?}");
    }

    #[tokio::test]
    #[ignore = "hits mainnet RPC"]
    async fn base_tier_pool_buy_simulates_at_one_tenth_percent_slippage() {
        let (min_out, err) = simulate_token_buy_at(BASE_TIER_POOL, 0.001).await;
        assert!(err.is_none(), "min_out={min_out} failed: {err:?}");
    }

    /// The quote must be exact, not merely safe: at zero slippage the program
    /// has to accept the floor the SDK produced.
    #[tokio::test]
    #[ignore = "hits mainnet RPC"]
    async fn a_zero_slippage_quote_is_accepted_by_the_program() {
        for pool in [SOL_BASE_POOL, BASE_TIER_POOL] {
            let (min_out, err) = simulate_token_buy_at(pool, 0.0).await;
            assert!(err.is_none(), "{pool}: min_out={min_out} failed: {err:?}");
        }
    }

    /// The live fee schedule must match what the fee program returns for the
    /// pool: 30 bps flat off the ladder, 125 bps on the base rung.
    #[tokio::test]
    #[ignore = "hits mainnet RPC"]
    async fn live_pool_fees_match_the_expected_schedules() {
        let client = client();
        for (pool, expected_bps) in [(SOL_BASE_POOL, 30u64), (BASE_TIER_POOL, 125)] {
            let pool_info = client
                .load_pool(&Pubkey::from_str(pool).expect("valid pubkey"))
                .await
                .expect("load pool");
            let (base_reserve, quote_reserve) = client
                .fetch_pool_reserves(&pool_info)
                .await
                .expect("reserves");
            let fees = client
                .fetch_pool_fees(&pool_info, base_reserve, quote_reserve)
                .await
                .expect("fees");
            assert_eq!(fees.total_bps(), expected_bps, "pool {pool}");
        }
    }
}
