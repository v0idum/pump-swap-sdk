//! Decoding tests for the fee program's `SharingConfig` account — a coin's
//! creator fees split across several addresses by basis points.
//!
//! The account is owned by the fee program, not pump-amm; pump-amm declares it
//! in its own IDL because `migrate_pool_coin_creator` reads it. 664_802 of
//! them were live on mainnet at slot 448_520_505 (2026-09-20).
//!
//! The offline tests run against byte-for-byte fixtures captured from mainnet
//! at slot 448_520_804 (2026-09-20), so CI never depends on RPC availability.
//! The `#[ignore]`d tests re-run the same assertions against live chain state:
//!
//! ```text
//! cargo test --test sharing_config -- --ignored
//! RPC_URL=https://my-private-rpc cargo test --test sharing_config -- --ignored
//! ```

use pump_swap_sdk::{ConfigStatus, SharingConfig, sharing_config_pda};
use solana_sdk::pubkey;
use solana_sdk::pubkey::Pubkey;

/// `12P8uYeeBLpQACDR4JzHiireQp9UhSGuAsgybCAy8kkb`: four shareholders at
/// 2500 bps each, Active, admin not revoked.
const SPLIT_FIXTURE: &[u8] = include_bytes!("fixtures/sharing_configs/sharing_config_split.bin");

/// `3gm2fowrH1dJLgwJsT23UBJa4en11hXNNSqfvdCyFfPx`: Paused, admin revoked,
/// empty shareholder list — and live bytes past the encoded length, left over
/// from a longer list.
const PAUSED_FIXTURE: &[u8] = include_bytes!("fixtures/sharing_configs/sharing_config_paused.bin");

const SPLIT_ADDRESS: Pubkey = pubkey!("12P8uYeeBLpQACDR4JzHiireQp9UhSGuAsgybCAy8kkb");
const SPLIT_MINT: Pubkey = pubkey!("DmMpRXCfh2Kpy9dvb2Hor5MvHSfhdAfa3hqX1u7ypump");
const SPLIT_ADMIN: Pubkey = pubkey!("HyF3gui8takahPszgQYM1caHwEC8gT4gpnLnPFitQjrz");

const PAUSED_ADDRESS: Pubkey = pubkey!("3gm2fowrH1dJLgwJsT23UBJa4en11hXNNSqfvdCyFfPx");
const PAUSED_MINT: Pubkey = pubkey!("8mgM8w9BDeZvL1pm86xPDvPw9XHL3vwXkFBqU693pump");

fn split() -> SharingConfig {
    SharingConfig::from_account_data(SPLIT_FIXTURE).expect("fixture decodes")
}

fn paused() -> SharingConfig {
    SharingConfig::from_account_data(PAUSED_FIXTURE).expect("fixture decodes")
}

#[test]
fn decodes_an_active_four_way_split() {
    let config = split();

    assert_eq!(config.bump, 251);
    assert_eq!(config.version, 1);
    assert_eq!(config.status, ConfigStatus::Active);
    assert!(config.is_active());
    assert_eq!(config.mint, SPLIT_MINT);
    assert_eq!(config.admin, SPLIT_ADMIN);
    assert!(!config.admin_revoked);

    let shares: Vec<_> = config
        .shareholders
        .iter()
        .map(|s| (s.address, s.share_bps))
        .collect();
    assert_eq!(
        shares,
        vec![
            (
                pubkey!("8rjnM5GPXswFPBtgnoZUyK5RykuaLfPnTQTmAN5WDLbV"),
                2500
            ),
            (
                pubkey!("5FjarBWs1Yb9FgQERdMusuHZgwvJvFiVfzxViiqkVgzz"),
                2500
            ),
            (
                pubkey!("3Z62uZsndJRdQ5NLbK1CLraX7eX9myrYVdgAynMgc4X3"),
                2500
            ),
            (SPLIT_ADMIN, 2500),
        ]
    );
    assert_eq!(config.total_share_bps(), 10_000);
}

#[test]
fn decodes_a_paused_config_with_a_revoked_admin() {
    let config = paused();

    assert_eq!(config.bump, 254);
    assert_eq!(config.version, 2);
    assert_eq!(config.status, ConfigStatus::Paused);
    assert!(!config.is_active());
    assert_eq!(config.mint, PAUSED_MINT);
    assert!(config.admin_revoked);
    // This config's admin happens to be zeroed. Revocation does not imply it:
    // live configs carry `admin_revoked` with a non-zero `admin`, so
    // `SharingConfig::admin` is documented as independent of the flag.
    assert_eq!(config.admin, Pubkey::default());
    assert!(config.shareholders.is_empty());
    assert_eq!(config.total_share_bps(), 0);
}

/// The account is allocated at a fixed 1024 bytes and a shrunk shareholder
/// list leaves its old bytes in place, so the decoder must stop at the end of
/// the vector. The paused fixture carries such bytes; both fixtures pad.
#[test]
fn ignores_bytes_past_the_encoded_layout() {
    for fixture in [SPLIT_FIXTURE, PAUSED_FIXTURE] {
        assert_eq!(fixture.len(), 1024);
        let mut extended = fixture.to_vec();
        extended.extend_from_slice(&[0xab; 64]);
        assert_eq!(
            SharingConfig::from_account_data(&extended).expect("decodes"),
            SharingConfig::from_account_data(fixture).expect("decodes"),
        );
    }
}

/// Both fixtures live at `["sharing-config", mint]` under the fee program.
/// The hyphen matters: `fee_config`'s seed uses an underscore.
#[test]
fn pda_derivation_matches_the_live_addresses() {
    assert_eq!(sharing_config_pda(&SPLIT_MINT), SPLIT_ADDRESS);
    assert_eq!(sharing_config_pda(&PAUSED_MINT), PAUSED_ADDRESS);
}

#[test]
fn rejects_a_foreign_discriminator() {
    let mut data = SPLIT_FIXTURE.to_vec();
    data[0] ^= 0xff;
    let err = SharingConfig::from_account_data(&data).expect_err("wrong discriminator");
    assert!(
        err.to_string().contains("SharingConfig"),
        "unexpected error: {err}"
    );
}

/// A truncated account must fail rather than decode a short shareholder list
/// into plausible-looking garbage.
#[test]
fn rejects_a_truncated_account() {
    for len in [8, 40, 100] {
        assert!(
            SharingConfig::from_account_data(&SPLIT_FIXTURE[..len]).is_err(),
            "{len} bytes decoded"
        );
    }
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
    async fn live_sharing_config_matches_the_fixture() {
        let config = client()
            .fetch_sharing_config(&SPLIT_MINT)
            .await
            .expect("fetch sharing config")
            .expect("account exists");

        assert_eq!(config, split());
    }

    /// Most coins have no fee split, so a missing account is `Ok(None)`.
    #[tokio::test]
    #[ignore = "hits mainnet RPC"]
    async fn live_sharing_config_is_none_for_a_coin_without_one() {
        let config = client()
            .fetch_sharing_config(&Pubkey::new_unique())
            .await
            .expect("fetch sharing config");

        assert!(config.is_none());
    }
}
