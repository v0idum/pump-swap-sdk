//! The two pump-amm instructions that write a pool's `coin_creator`:
//! `set_coin_creator`, which backfills it from the coin's bonding curve, and
//! `migrate_pool_coin_creator`, which repoints it at the coin's fee-sharing
//! config.
//!
//! Neither takes arguments, so the whole surface is the account list. It is
//! transcribed from the live pump-amm IDL
//! (`anchor idl fetch pAMMBay6oceH9fJKBRHGP5D4bD4sWpmSwMn52FMfXEA`), and every
//! derived address is pinned to an account that exists on mainnet rather than
//! re-derived here, so a broken seed cannot satisfy both sides.
//!
//! ```text
//! cargo test --test coin_creator_ix -- --ignored
//! RPC_URL=https://my-private-rpc cargo test --test coin_creator_ix -- --ignored
//! ```

use pump_swap_sdk::{
    EVENT_AUTHORITY, PUMP_SWAP_PROGRAM_ID, bonding_curve_pda,
    migrate_pool_coin_creator_instruction, set_coin_creator_instruction, sharing_config_pda,
    token_metadata_pda,
};
use solana_sdk::instruction::Instruction;
use solana_sdk::pubkey;
use solana_sdk::pubkey::Pubkey;

/// The pool behind `fixtures/pools/pool_fee_sharing.bin`, captured at slot
/// 448_529_034: a canonical pump pool that has already been migrated, so its
/// stored `coin_creator` is `SHARING_CONFIG` — the state this instruction
/// produces.
const MIGRATED_POOL: Pubkey = pubkey!("FAbQ88YXBRCw8A2t7Lomg9s1Ds2dX7hUvqwWtdfG4AWL");
const MIGRATED_BASE_MINT: Pubkey = pubkey!("3RzyjCaSHjDDaa4jrZfhDyQ3cLA8jSXvqEnWHwYcg4Hg");
const SHARING_CONFIG: Pubkey = pubkey!("B8NYJNrff1PJsSWpEdrKPkFG3ntNFpJgFercJBz89ey");

/// A pump.fun coin from the spl-token era, which is the population
/// `set_coin_creator` backfills: it has both a live bonding curve and a live
/// Metaplex metadata account. Coins minted later carry their metadata as a
/// Token-2022 extension and have no Metaplex account at all.
const LEGACY_MINT: Pubkey = pubkey!("ED5nyyWEzpPPiWimP8vYm7sD7TD3LAt3Q3gRTWHzPJBY");
const LEGACY_METADATA: Pubkey = pubkey!("4Q7aoDr7zBsaBB7emx9vvugE71xsPUsc8MvKHtRfhgY1");
const LEGACY_BONDING_CURVE: Pubkey = pubkey!("Ak4Cu8ZTTbaLvAubaCSpzdrEKbpwMpkF4RArSoRLRvMF");

/// `(pubkey, is_writable, is_signer)` for every account, in order — the same
/// shape `creator_fee_routing.rs` uses.
fn metas(ix: &Instruction) -> Vec<(Pubkey, bool, bool)> {
    ix.accounts
        .iter()
        .map(|m| (m.pubkey, m.is_writable, m.is_signer))
        .collect()
}

/// Four accounts, no arguments, no signer. `sharing_config` is the base mint's
/// config PDA; the program derives the same address from `pool.base_mint` and
/// rejects a mismatch with `InvalidSharingConfigBaseMint`.
#[test]
fn migrate_pool_coin_creator_matches_the_idl() {
    let ix =
        migrate_pool_coin_creator_instruction(&MIGRATED_POOL, &MIGRATED_BASE_MINT).expect("builds");

    assert_eq!(ix.program_id, PUMP_SWAP_PROGRAM_ID);
    assert_eq!(ix.data, vec![208, 8, 159, 4, 74, 175, 16, 58]);
    assert_eq!(
        metas(&ix),
        vec![
            (MIGRATED_POOL, true, false),
            (SHARING_CONFIG, false, false),
            (EVENT_AUTHORITY, false, false),
            (PUMP_SWAP_PROGRAM_ID, false, false),
        ]
    );
}

/// The account the instruction writes is the one the migrated pool already
/// holds: `pool.coin_creator` after migration is exactly the `sharing_config`
/// passed in. `PoolInfo::fee_sharing_config` reads the same relation back.
#[test]
fn migrate_pool_coin_creator_writes_the_config_the_routing_reads_back() {
    let ix =
        migrate_pool_coin_creator_instruction(&MIGRATED_POOL, &MIGRATED_BASE_MINT).expect("builds");

    assert_eq!(
        ix.accounts[1].pubkey,
        sharing_config_pda(&MIGRATED_BASE_MINT)
    );

    let info = pump_swap_sdk::PoolInfo::from_account_data(
        MIGRATED_POOL,
        include_bytes!("fixtures/pools/pool_fee_sharing.bin"),
        spl_token::ID,
        spl_token::ID,
    )
    .expect("fixture decodes");
    assert_eq!(info.base_mint, MIGRATED_BASE_MINT);
    assert_eq!(info.coin_creator, ix.accounts[1].pubkey);
}

/// Five accounts, no arguments, no signer.
///
/// The IDL puts no PDA constraint on `pool`, so it is passed through as given;
/// the program instead rejects a non-graduation pool with
/// `OnlyCanonicalPumpPoolsCanHaveCoinCreator`, so `pool` below is an obvious
/// placeholder. Both derived accounts come from the base mint and are pinned
/// to live mainnet addresses.
#[test]
fn set_coin_creator_matches_the_idl() {
    let pool = Pubkey::new_from_array([7u8; 32]);
    let ix = set_coin_creator_instruction(&pool, &LEGACY_MINT).expect("builds");

    assert_eq!(ix.program_id, PUMP_SWAP_PROGRAM_ID);
    assert_eq!(ix.data, vec![210, 149, 128, 45, 188, 58, 78, 175]);
    assert_eq!(
        metas(&ix),
        vec![
            (pool, true, false),
            (LEGACY_METADATA, false, false),
            (LEGACY_BONDING_CURVE, false, false),
            (EVENT_AUTHORITY, false, false),
            (PUMP_SWAP_PROGRAM_ID, false, false),
        ]
    );
}

/// `["metadata", metaplex_program, mint]` under the Metaplex Token Metadata
/// program — the seed list repeats the program id, which is easy to drop.
/// Pinned against two live accounts: USDC's and a pump coin's.
#[test]
fn token_metadata_pda_matches_live_metaplex_accounts() {
    assert_eq!(
        token_metadata_pda(&pubkey!("EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v")),
        pubkey!("5x38Kp4hvdomTCnCrAny4UtMUt5rQBdB6px2K1Ui45Wq")
    );
    assert_eq!(token_metadata_pda(&LEGACY_MINT), LEGACY_METADATA);
    assert_eq!(bonding_curve_pda(&LEGACY_MINT), LEGACY_BONDING_CURVE);
}

mod live {
    use super::*;
    use pump_swap_sdk::{PUMPFUN_PROGRAM, SharingConfig, TOKEN_METADATA_PROGRAM};
    use solana_client::nonblocking::rpc_client::RpcClient;
    use solana_commitment_config::CommitmentConfig;

    fn rpc() -> RpcClient {
        let url = std::env::var("RPC_URL")
            .unwrap_or_else(|_| "https://api.mainnet-beta.solana.com".to_string());
        RpcClient::new_with_commitment(url, CommitmentConfig::confirmed())
    }

    /// Both derived accounts resolve to live accounts owned by the programs
    /// the IDL names, and the config is the one for this pool's base mint.
    #[tokio::test]
    #[ignore = "hits mainnet RPC"]
    async fn live_migrate_pool_coin_creator_accounts_exist() {
        let ix = migrate_pool_coin_creator_instruction(&MIGRATED_POOL, &MIGRATED_BASE_MINT)
            .expect("builds");
        let accounts = rpc()
            .get_multiple_accounts(&[ix.accounts[0].pubkey, ix.accounts[1].pubkey])
            .await
            .expect("fetch");

        let pool = accounts[0].as_ref().expect("pool exists");
        assert_eq!(pool.owner, PUMP_SWAP_PROGRAM_ID);

        let config = accounts[1].as_ref().expect("sharing config exists");
        assert_eq!(config.owner, pump_swap_sdk::FEE_PROGRAM);
        let decoded = SharingConfig::from_account_data(&config.data).expect("decodes");
        assert_eq!(decoded.mint, MIGRATED_BASE_MINT);
    }

    /// `set_coin_creator` reads both accounts, so both have to exist for the
    /// coin it is run against.
    #[tokio::test]
    #[ignore = "hits mainnet RPC"]
    async fn live_set_coin_creator_accounts_exist() {
        let ix = set_coin_creator_instruction(&Pubkey::new_from_array([7u8; 32]), &LEGACY_MINT)
            .expect("builds");
        let accounts = rpc()
            .get_multiple_accounts(&[ix.accounts[1].pubkey, ix.accounts[2].pubkey])
            .await
            .expect("fetch");

        assert_eq!(
            accounts[0].as_ref().expect("metadata exists").owner,
            TOKEN_METADATA_PROGRAM
        );
        assert_eq!(
            accounts[1].as_ref().expect("bonding curve exists").owner,
            PUMPFUN_PROGRAM
        );
    }
}
