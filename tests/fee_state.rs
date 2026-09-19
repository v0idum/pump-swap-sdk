//! Decoding tests for the live pump-amm `GlobalConfig` and pump fee program
//! `FeeConfig` accounts.
//!
//! The offline tests run against byte-for-byte fixtures captured from mainnet
//! at slot 448452407 (2026-09-19), so CI never depends on RPC availability.
//! The `#[ignore]`d tests re-run the same assertions against live chain state
//! and act as a drift alarm for the hardcoded tables in `constants.rs`:
//!
//! ```text
//! cargo test --test fee_state -- --ignored
//! RPC_URL=https://my-private-rpc cargo test --test fee_state -- --ignored
//! ```

use pump_swap_sdk::{
    BUYBACK_FEE_RECIPIENTS, FeeConfig, Fees, GlobalConfig, PROTOCOL_FEE_RECIPIENTS,
    RESERVED_FEE_RECIPIENTS,
};

const GLOBAL_CONFIG_FIXTURE: &[u8] = include_bytes!("fixtures/global_config.bin");
const FEE_CONFIG_FIXTURE: &[u8] = include_bytes!("fixtures/fee_config.bin");

fn fee_config() -> FeeConfig {
    FeeConfig::from_account_data(FEE_CONFIG_FIXTURE).expect("fixture decodes")
}

fn global_config() -> GlobalConfig {
    GlobalConfig::from_account_data(GLOBAL_CONFIG_FIXTURE).expect("fixture decodes")
}

#[test]
fn fee_config_decodes_flat_fees() {
    let fees = fee_config().flat_fees;
    assert_eq!(fees.lp_fee_bps, 25);
    assert_eq!(fees.protocol_fee_bps, 5);
    assert_eq!(fees.creator_fee_bps, 0);
    assert_eq!(fees.total_bps(), 30);
}

#[test]
fn fee_config_decodes_both_tier_vectors() {
    let config = fee_config();
    assert_eq!(config.fee_tiers.len(), 25);
    assert_eq!(config.stable_fee_tiers.len(), 25);
}

#[test]
fn fee_config_decodes_first_and_last_tier() {
    let config = fee_config();
    let first = config.fee_tiers.first().expect("non-empty");
    let last = config.fee_tiers.last().expect("non-empty");

    assert_eq!(first.market_cap_lamports_threshold, 0);
    assert_eq!(first.fees.total_bps(), 125);
    assert_eq!(last.market_cap_lamports_threshold, 98_240_000_000_000);
    assert_eq!(last.fees.total_bps(), 30);
}

/// A `u64` misread of `market_cap_lamports_threshold` produces plausible
/// garbage rather than an error; monotonically increasing thresholds across
/// the whole ladder is the cheapest signal that the u128 width is right.
#[test]
fn fee_tier_thresholds_are_strictly_increasing() {
    let config = fee_config();
    for tiers in [&config.fee_tiers, &config.stable_fee_tiers] {
        for pair in tiers.windows(2) {
            assert!(
                pair[0].market_cap_lamports_threshold < pair[1].market_cap_lamports_threshold,
                "thresholds out of order: {} then {}",
                pair[0].market_cap_lamports_threshold,
                pair[1].market_cap_lamports_threshold
            );
        }
    }
}

#[test]
fn fee_tier_lookup_picks_the_highest_threshold_at_or_below_market_cap() {
    let config = fee_config();
    let cheapest = config.fee_tiers.last().expect("non-empty");

    // Below every threshold above zero: the base tier applies.
    let base = config.fee_tier_for_market_cap(0).expect("base tier");
    assert_eq!(base.market_cap_lamports_threshold, 0);
    assert_eq!(base.fees.total_bps(), 125);

    // Above the top threshold: the cheapest tier applies and stays applied.
    let top = config.fee_tier_for_market_cap(u128::MAX).expect("top tier");
    assert_eq!(top.market_cap_lamports_threshold, 98_240_000_000_000);
    assert_eq!(top.fees, cheapest.fees);

    // Exactly on a threshold selects that tier, one lamport below does not.
    let second = &config.fee_tiers[1];
    let threshold = second.market_cap_lamports_threshold;
    assert_eq!(
        config
            .fee_tier_for_market_cap(threshold)
            .expect("tier")
            .market_cap_lamports_threshold,
        threshold
    );
    assert_eq!(
        config
            .fee_tier_for_market_cap(threshold - 1)
            .expect("tier")
            .market_cap_lamports_threshold,
        0
    );
}

#[test]
fn stable_fee_tier_lookup_uses_the_stable_ladder() {
    let config = fee_config();
    let stable_top = config
        .stable_fee_tier_for_market_cap(u128::MAX)
        .expect("top stable tier");
    assert_eq!(stable_top.market_cap_lamports_threshold, 20_000_000_000_000);
    assert_eq!(stable_top.fees.total_bps(), 30);
}

#[test]
fn global_config_decodes_scalar_fields() {
    let config = global_config();
    assert_eq!(config.lp_fee_basis_points, 20);
    assert_eq!(config.protocol_fee_basis_points, 5);
    assert_eq!(config.coin_creator_fee_basis_points, 5);
    assert_eq!(config.buyback_basis_points, 5000);
    assert_eq!(config.disable_flags, 0);
    assert!(config.mayhem_mode_enabled);
    assert!(config.is_cashback_enabled);
}

#[test]
fn global_config_recipients_match_hardcoded_constants() {
    let config = global_config();
    assert_eq!(config.protocol_fee_recipients, PROTOCOL_FEE_RECIPIENTS);
    assert_eq!(
        config.all_reserved_fee_recipients(),
        RESERVED_FEE_RECIPIENTS
    );
    assert_eq!(config.buyback_fee_recipients, BUYBACK_FEE_RECIPIENTS);
}

/// `all_reserved_fee_recipients` is the only place the on-chain 1 + 7 split is
/// flattened into the SDK's array of 8; keep the halves addressable too.
#[test]
fn global_config_keeps_the_reserved_recipient_split() {
    let config = global_config();
    assert_eq!(config.reserved_fee_recipient, RESERVED_FEE_RECIPIENTS[0]);
    assert_eq!(
        config.reserved_fee_recipients[..],
        RESERVED_FEE_RECIPIENTS[1..]
    );
}

#[test]
fn decoding_rejects_a_foreign_discriminator() {
    let mut data = GLOBAL_CONFIG_FIXTURE.to_vec();
    data[0] ^= 0xff;
    let err = GlobalConfig::from_account_data(&data).expect_err("bad discriminator");
    assert!(
        err.to_string().contains("discriminator"),
        "unexpected error: {err}"
    );
}

#[test]
fn decoding_rejects_truncated_accounts() {
    let truncated = &GLOBAL_CONFIG_FIXTURE[..100];
    assert!(GlobalConfig::from_account_data(truncated).is_err());

    let truncated = &FEE_CONFIG_FIXTURE[..100];
    assert!(FeeConfig::from_account_data(truncated).is_err());
}

/// Both accounts carry unused tail bytes beyond the IDL layout (42 for
/// `GlobalConfig`, 2024 for `FeeConfig`), so decoding must not require an
/// exact length.
#[test]
fn decoding_tolerates_trailing_account_slack() {
    let mut data = GLOBAL_CONFIG_FIXTURE.to_vec();
    data.extend_from_slice(&[0xab; 64]);
    assert_eq!(
        GlobalConfig::from_account_data(&data).expect("decodes"),
        global_config()
    );
}

mod live {
    use super::*;
    use pump_swap_sdk::PumpSwapClient;
    use solana_client::nonblocking::rpc_client::RpcClient;
    use solana_commitment_config::CommitmentConfig;
    use std::sync::Arc;

    fn client() -> PumpSwapClient<Arc<RpcClient>> {
        let url = std::env::var("RPC_URL")
            .unwrap_or_else(|_| "https://api.mainnet-beta.solana.com".to_string());
        PumpSwapClient::new(Arc::new(RpcClient::new_with_commitment(
            url,
            CommitmentConfig::confirmed(),
        )))
    }

    #[tokio::test]
    #[ignore = "hits mainnet RPC"]
    async fn live_fee_config_matches_the_expected_fee_schedule() {
        let config = client().fetch_fee_config().await.expect("fetch fee config");

        assert_eq!(config.flat_fees.lp_fee_bps, 25);
        assert_eq!(config.flat_fees.protocol_fee_bps, 5);
        assert_eq!(config.flat_fees.creator_fee_bps, 0);
        assert_eq!(config.fee_tiers.len(), 25);
        assert_eq!(config.stable_fee_tiers.len(), 25);

        let first = config.fee_tiers.first().expect("non-empty");
        assert_eq!(first.market_cap_lamports_threshold, 0);
        assert_eq!(first.fees.total_bps(), 125);

        let last = config.fee_tiers.last().expect("non-empty");
        assert_eq!(last.market_cap_lamports_threshold, 98_240_000_000_000);
        assert_eq!(last.fees.total_bps(), 30);
    }

    #[tokio::test]
    #[ignore = "hits mainnet RPC"]
    async fn live_global_config_matches_hardcoded_constants() {
        let config = client()
            .fetch_global_config()
            .await
            .expect("fetch global config");

        assert_eq!(config.protocol_fee_recipients, PROTOCOL_FEE_RECIPIENTS);
        assert_eq!(
            config.all_reserved_fee_recipients(),
            RESERVED_FEE_RECIPIENTS
        );
        assert_eq!(config.buyback_fee_recipients, BUYBACK_FEE_RECIPIENTS);
    }

    #[tokio::test]
    #[ignore = "hits mainnet RPC"]
    async fn live_fee_state_fetches_both_accounts_in_one_call() {
        let (global_config, fee_config) =
            client().fetch_fee_state().await.expect("fetch fee state");

        assert_eq!(
            global_config.protocol_fee_recipients,
            PROTOCOL_FEE_RECIPIENTS
        );
        assert_eq!(fee_config.fee_tiers.len(), 25);
    }
}

/// Fee schedules the fee program actually returned, decoded from the
/// `GetFeesWithQuoteMint` CPI and the swap event of live mainnet transactions
/// on 2026-09-19. Each row is `(is_pump_pool, market_cap_lamports, fees)`.
///
/// A cashback pool is charged the tier's creator basis points as cashback
/// instead, so the two are summed back into `creator_fee_bps` here.
const OBSERVED_SCHEDULES: &[(bool, u128, Fees)] = &[
    // pool HfpiSYjkviLNTHF8FjM8yCPauMwq5px2XMerSZPUKvdT — not a pump pool.
    (false, 0, fees(25, 5, 0)),
    // pool AF16XnzeaQdUnJMhXE3CQ6N13QHJLgH2mfquSdZVQo4e — cheapest rung.
    (true, 155_988_215_454_556, fees(20, 5, 5)),
    (true, 95_165_109_922_025, fees(20, 5, 8)),
    (true, 85_235_430_439_200, fees(20, 5, 13)),
    (true, 74_967_324_444_046, fees(20, 5, 18)),
    (true, 65_090_408_441_691, fees(20, 5, 23)),
    (true, 61_264_917_393_483, fees(20, 5, 25)),
    (true, 53_969_221_648_203, fees(20, 5, 30)),
    (true, 46_357_214_426_210, fees(20, 5, 35)),
    (true, 37_584_591_767_307, fees(20, 5, 45)),
    (true, 15_869_836_765_263, fees(20, 5, 65)),
    (true, 10_690_600_722_838, fees(20, 5, 70)),
    (true, 5_470_616_941_054, fees(20, 5, 75)),
    (true, 3_684_524_298_250, fees(20, 5, 80)),
    (true, 2_692_043_212_031, fees(20, 5, 85)),
    (true, 1_917_839_092_121, fees(20, 5, 90)),
    (true, 1_046_767_344_422, fees(20, 5, 95)),
    // pool 2Y8QhdP4Zox3mTKiJNpCQiFqUncHDE69LKZnTyvFjeqG — base rung, 125 bps.
    (true, 162_452_108_672, fees(2, 93, 30)),
];

const fn fees(lp_fee_bps: u64, protocol_fee_bps: u64, creator_fee_bps: u64) -> Fees {
    Fees {
        lp_fee_bps,
        protocol_fee_bps,
        creator_fee_bps,
    }
}

#[test]
fn fees_for_pool_reproduces_every_observed_mainnet_schedule() {
    let config = fee_config();
    let quote_mint = solana_sdk::pubkey::Pubkey::new_unique();

    for (is_tiered, market_cap, expected) in OBSERVED_SCHEDULES {
        assert_eq!(
            config.fees_for_pool(*is_tiered, *market_cap, &quote_mint),
            *expected,
            "is_tiered={is_tiered} market_cap={market_cap}"
        );
    }
}

/// The dangerous direction: a pool the program does price off the ladder must
/// never be quoted at `flat_fees`, which is 95 bps cheaper than the base rung.
#[test]
fn a_tiered_pool_below_every_threshold_gets_the_base_rung_not_the_flat_fee() {
    let config = fee_config();
    let quote_mint = solana_sdk::pubkey::Pubkey::new_unique();

    let base = config.fees_for_pool(true, 0, &quote_mint);
    assert_eq!(base.total_bps(), 125);
    assert_ne!(base, config.flat_fees);
    assert_eq!(config.flat_fees.total_bps(), 30);
}
