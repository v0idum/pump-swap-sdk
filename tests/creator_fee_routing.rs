//! Creator-fee routing for fee-sharing coins.
//!
//! A coin whose creator fees are split does not collect them with
//! `collect_coin_creator_fee` — pump-amm rejects that with
//! `CreatorVaultMigratedToSharingConfig`. The fees leave the pump-amm vault
//! through `transfer_creator_fees_to_pump`, and pump.fun's
//! `distribute_creator_fees` pays the shareholders out of the pump creator
//! vault.
//!
//! The account lists below are transcribed from mainnet transactions, so a
//! layout change here fails against what the programs actually accepted rather
//! than against the IDL alone.
//!
//! ```text
//! cargo test --test creator_fee_routing -- --ignored
//! RPC_URL=https://my-private-rpc cargo test --test creator_fee_routing -- --ignored
//! ```

use pump_swap_sdk::{
    PUMP_SWAP_PROGRAM_ID, PUMPFUN_EVENT_AUTHORITY, PUMPFUN_PROGRAM, WRAPPED_SOL_MINT,
    bonding_curve_pda, distribute_creator_fees_instruction, distribute_creator_fees_v2_instruction,
    find_coin_creator_vault_authority, pump_creator_vault_pda, sharing_config_pda,
    transfer_creator_fees_to_pump_instruction, transfer_creator_fees_to_pump_v2_instruction,
};
use solana_sdk::instruction::Instruction;
use solana_sdk::pubkey;
use solana_sdk::pubkey::Pubkey;

/// A live fee-sharing coin, five shareholders at 2000 bps each.
const MINT: Pubkey = pubkey!("GhhaqhLh9ZR1WRLdTxBRkKaK21HEvb7N4MoxvuTapump");
const BONDING_CURVE: Pubkey = pubkey!("8ueBrJgPmGYYm2QGcYZ5zXru1MTEDCMcoVDphbhfs9Xr");
const SHARING_CONFIG: Pubkey = pubkey!("5tw6HCdjzQvxkgvocBkb6T4Yj7pBPC4UZSn1GyKAEPT");
const PUMP_CREATOR_VAULT: Pubkey = pubkey!("CijnjLEPfz3Locv6mmdwhDvfpkwjcwrkWGyXgUhxcwm2");
/// `get_associated_token_address(PUMP_CREATOR_VAULT, wSOL)`, as the live
/// `distribute_creator_fees_v2` passed it.
const PUMP_CREATOR_VAULT_WSOL_ATA: Pubkey = pubkey!("FJvQRQ6pphra97MJWujDF4ngoxdx6gLonW7UJha9ruKL");

const SHAREHOLDERS: [Pubkey; 5] = [
    pubkey!("GV6UUmNxz2RpKxmNAPadYKb7uQpszwqQAu3qLJxVdC52"),
    pubkey!("2fg5QD1eD7rzNNCsvnhmXFm5hqNgwTTG8p7kQ6f3rx6f"),
    pubkey!("CEoJPnnCfYisckyJnqZLeefc559KeheVsFLbpfDuTPWo"),
    pubkey!("HH7jLETzcSBnFQcAG5D4VUiFYPSATTyhu5R3QCgzW2M"),
    pubkey!("9fnyojTv8GYHWr4Vaj4tvVL82scPPYYkW1CAXAbfUsdj"),
];

const SYSTEM_PROGRAM: Pubkey = pubkey!("11111111111111111111111111111111");
const ASSOCIATED_TOKEN_PROGRAM: Pubkey = pubkey!("ATokenGPvbdGVxr1b2hvZbsiqW5xWH25efTNsLJA8knL");

/// `(pubkey, is_writable, is_signer)` for every account, in order.
fn metas(ix: &Instruction) -> Vec<(Pubkey, bool, bool)> {
    ix.accounts
        .iter()
        .map(|m| (m.pubkey, m.is_writable, m.is_signer))
        .collect()
}

/// Every address the routing needs is derived from the mint alone.
#[test]
fn the_whole_route_derives_from_the_mint() {
    assert_eq!(sharing_config_pda(&MINT), SHARING_CONFIG);
    assert_eq!(bonding_curve_pda(&MINT), BONDING_CURVE);
    assert_eq!(pump_creator_vault_pda(&SHARING_CONFIG), PUMP_CREATOR_VAULT);
}

/// pump.fun's `creator-vault` and pump-amm's `creator_vault` differ by one
/// character of seed and land on different addresses for the same creator.
/// Swapping them silently drains nothing and fails a seeds constraint.
#[test]
fn the_two_creator_vault_seeds_are_not_interchangeable() {
    assert_ne!(
        pump_creator_vault_pda(&SHARING_CONFIG),
        find_coin_creator_vault_authority(&SHARING_CONFIG)
    );
}

/// Transcribed from mainnet tx
/// `4yE8bVSJkiG3vCQEnuVPSpdbKGXwUHMLYZUnk4FYp5xxScj9DEpdm4gCM7aPXgFm7kpQqCgvUrNJQUupnh548Dce`
/// (slot 430_382_254), which paid out five shareholders in one lamport
/// transfer each. No account signs; only the vault and the shareholders are
/// writable.
#[test]
fn distribute_creator_fees_matches_a_live_payout() {
    let ix = distribute_creator_fees_instruction(&MINT, &SHAREHOLDERS).expect("builds");

    assert_eq!(ix.program_id, PUMPFUN_PROGRAM);
    assert_eq!(ix.data, vec![165, 114, 103, 0, 121, 206, 247, 81]);
    assert_eq!(
        metas(&ix),
        vec![
            (MINT, false, false),
            (BONDING_CURVE, false, false),
            (SHARING_CONFIG, false, false),
            (PUMP_CREATOR_VAULT, true, false),
            (SYSTEM_PROGRAM, false, false),
            (PUMPFUN_EVENT_AUTHORITY, false, false),
            (PUMPFUN_PROGRAM, false, false),
            (SHAREHOLDERS[0], true, false),
            (SHAREHOLDERS[1], true, false),
            (SHAREHOLDERS[2], true, false),
            (SHAREHOLDERS[3], true, false),
            (SHAREHOLDERS[4], true, false),
        ]
    );
}

/// The program matches remaining accounts against the config's shareholders
/// position by position (`ShareholdersAndRemainingAccountsMismatch`), so the
/// builder must not sort, dedupe or otherwise reorder what it is given.
#[test]
fn distribute_creator_fees_preserves_shareholder_order() {
    let mut reversed = SHAREHOLDERS;
    reversed.reverse();
    let ix = distribute_creator_fees_instruction(&MINT, &reversed).expect("builds");

    let tail: Vec<_> = ix.accounts[7..].iter().map(|m| m.pubkey).collect();
    assert_eq!(tail, reversed.to_vec());
}

/// A config with no shareholders still builds: seven accounts, no remainder.
#[test]
fn distribute_creator_fees_accepts_an_empty_shareholder_list() {
    let ix = distribute_creator_fees_instruction(&MINT, &[]).expect("builds");
    assert_eq!(ix.accounts.len(), 7);
}

/// Transcribed from the `distribute_creator_fees_v2` CPI inside mainnet tx
/// `4oMSAyMEqZYvYN1nKHyfCyi7arDGmj4aepU1aiGSFcCHoDkneevFVYfaNekQdA7Zr6cnuPYtY62zUbPR5wKVmCvT`
/// (slot 430_067_303), which ran with `initialize_ata = true`.
#[test]
fn distribute_creator_fees_v2_matches_a_live_payout() {
    let payer = pubkey!("9rDMVCH7mQ9N2PkyHw8KT8wraMhF8tyMz9R631yyL1df");
    let ix = distribute_creator_fees_v2_instruction(
        &payer,
        &MINT,
        &WRAPPED_SOL_MINT,
        &spl_token::ID,
        true,
        &[payer],
    )
    .expect("builds");

    assert_eq!(ix.program_id, PUMPFUN_PROGRAM);
    assert_eq!(ix.data, vec![255, 203, 19, 79, 244, 68, 8, 159, 1]);
    assert_eq!(
        metas(&ix),
        vec![
            (payer, true, true),
            (MINT, false, false),
            (BONDING_CURVE, false, false),
            (SHARING_CONFIG, false, false),
            (PUMP_CREATOR_VAULT, true, false),
            (SYSTEM_PROGRAM, false, false),
            (PUMPFUN_EVENT_AUTHORITY, false, false),
            (PUMPFUN_PROGRAM, false, false),
            (PUMP_CREATOR_VAULT_WSOL_ATA, true, false),
            (WRAPPED_SOL_MINT, false, false),
            (spl_token::ID, false, false),
            (ASSOCIATED_TOKEN_PROGRAM, false, false),
            (payer, true, false),
        ]
    );
}

/// `initialize_ata` is the instruction's only argument, a borsh `bool`
/// appended to the discriminator.
#[test]
fn distribute_creator_fees_v2_encodes_initialize_ata() {
    for (flag, byte) in [(true, 1u8), (false, 0u8)] {
        let ix = distribute_creator_fees_v2_instruction(
            &Pubkey::new_unique(),
            &MINT,
            &WRAPPED_SOL_MINT,
            &spl_token::ID,
            flag,
            &[],
        )
        .expect("builds");
        assert_eq!(ix.data.len(), 9);
        assert_eq!(ix.data[8], byte);
    }
}

/// The sweep deposits into the same pump creator vault the payout draws from.
/// Before this was derived, both builders passed one hardcoded address, which
/// is correct for exactly one coin.
#[test]
fn transfer_creator_fees_to_pump_targets_the_coins_own_vault() {
    let ix = transfer_creator_fees_to_pump_instruction(&SHARING_CONFIG).expect("builds");

    assert_eq!(ix.program_id, PUMP_SWAP_PROGRAM_ID);
    assert_eq!(ix.data, vec![139, 52, 134, 85, 228, 229, 108, 241]);
    assert_eq!(ix.accounts.len(), 10);
    assert_eq!(ix.accounts[4].pubkey, SHARING_CONFIG);
    assert_eq!(ix.accounts[7].pubkey, PUMP_CREATOR_VAULT);
    assert!(ix.accounts[7].is_writable);

    let payout = distribute_creator_fees_instruction(&MINT, &SHAREHOLDERS).expect("builds");
    assert_eq!(ix.accounts[7].pubkey, payout.accounts[3].pubkey);
}

/// v2 adds a `payer` signer up front and the vault's quote ATA, which it
/// creates when missing. Layout from the live pump-amm IDL; the vault ATA is
/// cross-checked against the address the live `distribute_creator_fees_v2`
/// used for the same vault and quote mint.
#[test]
fn transfer_creator_fees_to_pump_v2_adds_a_payer_and_the_vault_ata() {
    let payer = Pubkey::new_unique();
    let ix = transfer_creator_fees_to_pump_v2_instruction(
        &payer,
        &SHARING_CONFIG,
        &WRAPPED_SOL_MINT,
        &spl_token::ID,
    )
    .expect("builds");

    assert_eq!(ix.program_id, PUMP_SWAP_PROGRAM_ID);
    assert_eq!(ix.data, vec![1, 33, 78, 185, 33, 67, 44, 92]);
    assert_eq!(
        metas(&ix),
        vec![
            (payer, true, true),
            (WRAPPED_SOL_MINT, false, false),
            (spl_token::ID, false, false),
            (SYSTEM_PROGRAM, false, false),
            (ASSOCIATED_TOKEN_PROGRAM, false, false),
            (SHARING_CONFIG, false, false),
            (
                find_coin_creator_vault_authority(&SHARING_CONFIG),
                true,
                false
            ),
            (
                spl_associated_token_account::get_associated_token_address_with_program_id(
                    &find_coin_creator_vault_authority(&SHARING_CONFIG),
                    &WRAPPED_SOL_MINT,
                    &spl_token::ID,
                ),
                true,
                false,
            ),
            (PUMP_CREATOR_VAULT, true, false),
            (PUMP_CREATOR_VAULT_WSOL_ATA, true, false),
            (pump_swap_sdk::EVENT_AUTHORITY, false, false),
            (PUMP_SWAP_PROGRAM_ID, false, false),
        ]
    );
}

mod pool_detection {
    use super::*;
    use pump_swap_sdk::{PoolInfo, TokenSide};

    fn pool(base_mint: Pubkey, coin_creator: Pubkey) -> PoolInfo {
        PoolInfo {
            pool: Pubkey::new_unique(),
            pool_account_data_len: 300,
            base_mint,
            quote_mint: WRAPPED_SOL_MINT,
            lp_mint: Pubkey::new_unique(),
            pool_base_token_account: Pubkey::new_unique(),
            pool_quote_token_account: Pubkey::new_unique(),
            creator: Pubkey::new_unique(),
            coin_creator,
            is_mayhem_mode: false,
            is_cashback_coin: false,
            virtual_quote_reserves: 0,
            base_token_program: spl_token::ID,
            quote_token_program: spl_token::ID,
        }
    }

    /// `FAbQ88YXBRCw8A2t7Lomg9s1Ds2dX7hUvqwWtdfG4AWL`, captured at slot
    /// 448_529_034 (2026-09-20): a live pool that has been migrated, so its
    /// `coin_creator` is the coin's sharing config rather than a wallet.
    const FEE_SHARING_POOL: &[u8] = include_bytes!("fixtures/pools/pool_fee_sharing.bin");

    /// The real thing: decode a migrated mainnet pool and recover its config
    /// from `base_mint` alone.
    #[test]
    fn a_live_migrated_pool_reports_its_sharing_config() {
        let info = PoolInfo::from_account_data(
            pubkey!("FAbQ88YXBRCw8A2t7Lomg9s1Ds2dX7hUvqwWtdfG4AWL"),
            FEE_SHARING_POOL,
            spl_token::ID,
            spl_token::ID,
        )
        .expect("fixture decodes");

        assert_eq!(
            info.base_mint,
            pubkey!("3RzyjCaSHjDDaa4jrZfhDyQ3cLA8jSXvqEnWHwYcg4Hg")
        );
        assert_eq!(info.quote_mint, WRAPPED_SOL_MINT);
        assert_eq!(info.token_side(), TokenSide::Base);
        assert_eq!(
            info.coin_creator,
            pubkey!("B8NYJNrff1PJsSWpEdrKPkFG3ntNFpJgFercJBz89ey")
        );
        assert_eq!(info.fee_sharing_config(), Some(info.coin_creator));
        assert_eq!(
            info.fee_sharing_config(),
            Some(sharing_config_pda(&info.base_mint))
        );
    }

    #[test]
    fn a_pool_whose_coin_creator_is_the_configs_pda_is_fee_sharing() {
        let info = pool(MINT, SHARING_CONFIG);
        assert_eq!(info.token_side(), TokenSide::Base);
        assert_eq!(info.fee_sharing_config(), Some(SHARING_CONFIG));
    }

    #[test]
    fn an_ordinary_coin_creator_is_not_fee_sharing() {
        assert_eq!(pool(MINT, Pubkey::new_unique()).fee_sharing_config(), None);
        assert_eq!(pool(MINT, Pubkey::default()).fee_sharing_config(), None);
    }

    /// The config is keyed on `pool.base_mint`, which for a SOL-base pool is
    /// wSOL — so the derivation cannot match a real coin's config there.
    #[test]
    fn a_sol_base_pool_never_matches_a_coins_config() {
        let mut info = pool(WRAPPED_SOL_MINT, SHARING_CONFIG);
        info.quote_mint = MINT;
        assert_eq!(info.token_side(), TokenSide::Quote);
        assert_eq!(info.fee_sharing_config(), None);
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

    /// The composed payout must reproduce, from the mint alone, the exact
    /// `distribute_creator_fees` account list the recorded mainnet payout used
    /// — including the shareholder order, which is read live.
    #[tokio::test]
    #[ignore = "hits mainnet RPC"]
    async fn live_withdraw_ixs_reproduce_the_recorded_payout() {
        let ixs = client()
            .build_creator_fee_withdraw_ixs(&MINT)
            .await
            .expect("builds");

        assert_eq!(ixs.len(), 2);
        assert_eq!(ixs[0].program_id, PUMP_SWAP_PROGRAM_ID);
        assert_eq!(
            metas(&ixs[1]),
            metas(&distribute_creator_fees_instruction(&MINT, &SHAREHOLDERS).expect("builds"))
        );
    }

    /// A coin with no split is not this route, and saying so beats building a
    /// payout the program would reject.
    #[tokio::test]
    #[ignore = "hits mainnet RPC"]
    async fn live_withdraw_ixs_reject_a_coin_without_a_split() {
        let err = client()
            .build_creator_fee_withdraw_ixs(&Pubkey::new_unique())
            .await
            .expect_err("no sharing config");
        assert!(
            err.to_string().contains("no SharingConfig"),
            "unexpected error: {err}"
        );
    }
}
