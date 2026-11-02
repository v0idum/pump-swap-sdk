//! Fee sweeps: the permissionless instructions that move fees kept by v2
//! trades out of a pool or bonding curve.
//!
//! The account lists are transcribed from mainnet transactions where one
//! exists, and the `#[ignore]`d tests simulate what the SDK builds against the
//! live programs:
//!
//! ```text
//! RPC_URL=https://my-private-rpc cargo test --test fee_sweeps -- --ignored
//! ```

use pump_swap_sdk::{PUMP_SWAP_PROGRAM_ID, PoolInfo, sweep_pool_creator_fee_instruction};
use solana_sdk::instruction::Instruction;
use solana_sdk::pubkey;
use solana_sdk::pubkey::Pubkey;

fn metas(ix: &Instruction) -> Vec<(Pubkey, bool, bool)> {
    ix.accounts
        .iter()
        .map(|m| (m.pubkey, m.is_writable, m.is_signer))
        .collect()
}

/// `sweep_creator_fee` on pump-amm, as sent by
/// `4ZRFhEwSaA3bc9Q3wBUt399uE5o7r4F2uGX22WKX2ApP89RDk5mer51bWZFJBa3YpprPbFL4UbDkyP1AR6L8QXCj`
/// (slot 464_552_903), against a pool quoted in a Token-2022 mint.
mod recorded_pool_sweep {
    use super::*;

    pub const PAYER: Pubkey = pubkey!("FHpcNSe6tb2n15bAdq4BkeYWGyZKFD7yLYrH92ng7wCT");
    pub const POOL: Pubkey = pubkey!("65w3wn9GNC1mEPsXYBUZFHuPEZsckkpW58GHTv1x6f6B");
    pub const POOL_DATA: &[u8] = include_bytes!("fixtures/pools/pool_swept_token_2022_quote.bin");
    pub const TOKEN_2022: Pubkey = pubkey!("TokenzQdBNbLqP5VEhdkAS6EPFLC1PHnBqCXEpPxuEb");

    pub fn pool() -> PoolInfo {
        PoolInfo::from_account_data(POOL, POOL_DATA, TOKEN_2022, TOKEN_2022).expect("decodes")
    }
}

#[test]
fn pool_creator_fee_sweep_matches_the_recorded_mainnet_transaction() {
    use recorded_pool_sweep::*;

    let ix = sweep_pool_creator_fee_instruction(&PAYER, &pool()).expect("builds");

    assert_eq!(ix.program_id, PUMP_SWAP_PROGRAM_ID);
    assert_eq!(ix.data, [32, 246, 191, 52, 8, 201, 73, 186]);
    assert_eq!(
        metas(&ix),
        [
            (PAYER, true, true),
            (
                pubkey!("ADyA8hdefvWN2dbGGWFotbzWxrAvLW83WG6QCVXvJKqw"),
                false,
                false
            ),
            (POOL, true, false),
            (
                pubkey!("BN4Z5NtntHhu6p3atEqkeAVi8Bn9Ri2QTv8FD7iupump"),
                false,
                false
            ),
            (TOKEN_2022, false, false),
            (
                pubkey!("JDe8yMPR3VuLdPYtiE3D4S9pAdJBSPMz77Tcctb3EZro"),
                true,
                false
            ),
            (
                pubkey!("CbvjtLskdpRbGLaKsQmtF9UMwEdyWhaeLkB7PiBZWPqv"),
                false,
                false
            ),
            (
                pubkey!("7zBqtpLBQqzaZNypdTufQs2ztqAzTK1Xb3uus1aCARcL"),
                true,
                false
            ),
            (pubkey!("11111111111111111111111111111111"), false, false),
            (
                pubkey!("ATokenGPvbdGVxr1b2hvZbsiqW5xWH25efTNsLJA8knL"),
                false,
                false
            ),
            (
                pubkey!("GS4CU59F31iL7aR2Q8zVS8DRrcRnXX1yjQ66TqNVQnaR"),
                false,
                false
            ),
            (PUMP_SWAP_PROGRAM_ID, false, false),
        ]
    );
}

mod canonical_pool {
    use super::*;
    use pump_swap_sdk::WRAPPED_SOL_MINT;
    use pump_swap_sdk::util::{canonical_pool_pda, pump_pool_authority_pda};

    /// The live pool in `fixtures/pools/pool_fee_sharing.bin`, whose `creator`
    /// field holds the authority.
    const BASE_MINT: Pubkey = pubkey!("3RzyjCaSHjDDaa4jrZfhDyQ3cLA8jSXvqEnWHwYcg4Hg");

    #[test]
    fn a_coins_pool_is_derived_from_its_mint() {
        assert_eq!(
            pump_pool_authority_pda(&BASE_MINT),
            pubkey!("4xRZEkKPs2RDmKx5FAQdrQUD66JdkcwLs3JNvvRA3qEc")
        );
        assert_eq!(
            canonical_pool_pda(&BASE_MINT, &WRAPPED_SOL_MINT),
            pubkey!("FAbQ88YXBRCw8A2t7Lomg9s1Ds2dX7hUvqwWtdfG4AWL")
        );
    }
}

/// Transcribed from the `sweep_creator_fee` entry in pump.fun's IDL. Nothing
/// recorded on mainnet was found to compare against, so the live test below is
/// what shows the program accepts it.
#[test]
fn bonding_curve_creator_fee_sweep_matches_the_idl() {
    use pump_swap_sdk::sweep_bonding_curve_creator_fee_instruction;
    use pump_swap_sdk::util::{
        bonding_curve_pda, find_coin_creator_vault_ata, pump_creator_vault_pda,
    };
    use pump_swap_sdk::{PUMPFUN_PROGRAM, WRAPPED_SOL_MINT};

    let payer = Pubkey::new_unique();
    let mint = Pubkey::new_unique();
    let creator = Pubkey::new_unique();
    let curve = bonding_curve_pda(&mint);
    let vault = pump_creator_vault_pda(&creator);

    let ix = sweep_bonding_curve_creator_fee_instruction(
        &payer,
        &mint,
        &creator,
        &WRAPPED_SOL_MINT,
        &spl_token::ID,
    )
    .expect("builds");

    assert_eq!(ix.program_id, PUMPFUN_PROGRAM);
    assert_eq!(ix.data, [32, 246, 191, 52, 8, 201, 73, 186]);
    assert_eq!(
        metas(&ix),
        [
            (payer, true, true),
            (
                pubkey!("4wTV1YmiEkRvAtNtsSGPtUrqRYQMe5SKy2uB4Jjaxnjf"),
                false,
                false
            ),
            (mint, false, false),
            (WRAPPED_SOL_MINT, false, false),
            (spl_token::ID, false, false),
            (
                pubkey!("ATokenGPvbdGVxr1b2hvZbsiqW5xWH25efTNsLJA8knL"),
                false,
                false
            ),
            (pubkey!("11111111111111111111111111111111"), false, false),
            (curve, true, false),
            (
                find_coin_creator_vault_ata(&curve, &spl_token::ID, &WRAPPED_SOL_MINT),
                true,
                false
            ),
            (vault, true, false),
            (
                find_coin_creator_vault_ata(&vault, &spl_token::ID, &WRAPPED_SOL_MINT),
                true,
                false
            ),
            (
                pubkey!("Ce6TQqeHC9p8KetsN6JsjHK7UTZk7nasjjnr7XxXp9F1"),
                false,
                false
            ),
            (PUMPFUN_PROGRAM, false, false),
        ]
    );
}

mod collect_route {
    use super::*;
    use pump_swap_sdk::PumpSwapClient;
    use solana_client::nonblocking::rpc_client::RpcClient;
    use std::sync::Arc;

    /// Nothing here touches the network; the route is built from `PoolInfo`.
    fn client() -> PumpSwapClient<Arc<RpcClient>> {
        PumpSwapClient::new(Arc::new(RpcClient::new("http://127.0.0.1:1".to_string())))
    }

    const SWEEP: [u8; 8] = [32, 246, 191, 52, 8, 201, 73, 186];
    const COLLECT: [u8; 8] = [160, 57, 89, 42, 181, 139, 43, 66];

    /// A fee only reaches the coin-creator vault through a sweep, so a collect
    /// without one leaves whatever v2 trades kept in the pool.
    #[test]
    fn collecting_a_pools_creator_fee_sweeps_it_first() {
        let pool = recorded_pool_sweep::pool();
        let payer = Pubkey::new_unique();

        let ixs = client()
            .build_collect_pool_creator_fee_ixs(&pool, &payer, false)
            .expect("builds");

        assert_eq!(ixs.len(), 2);
        assert_eq!(ixs[0].data, SWEEP);
        assert_eq!(ixs[1].data, COLLECT);
        assert_eq!(
            ixs[0],
            sweep_pool_creator_fee_instruction(&payer, &pool).expect("builds")
        );
    }

    /// pump-amm rejects a collect on a fee-sharing pool with
    /// `CreatorVaultMigratedToSharingConfig`.
    #[test]
    fn collecting_refuses_a_fee_sharing_pool() {
        const FEE_SHARING_POOL: &[u8] = include_bytes!("fixtures/pools/pool_fee_sharing.bin");
        let pool = PoolInfo::from_account_data(
            Pubkey::new_unique(),
            FEE_SHARING_POOL,
            spl_token::ID,
            spl_token::ID,
        )
        .expect("decodes");

        let err = client()
            .build_collect_pool_creator_fee_ixs(&pool, &Pubkey::new_unique(), false)
            .expect_err("fee-sharing pool");
        assert!(
            err.to_string().contains("build_creator_fee_withdraw_ixs"),
            "unexpected error: {err}"
        );
    }
}

mod bonding_curve_fee {
    use pump_swap_sdk::state::bonding_curve_creator_fee;

    /// A live 166-byte curve, for `14fhHk5K82rFeha8Edc41sNu7TW5UYGu5N4UYmdApump`.
    const CURVE: &[u8] = include_bytes!("fixtures/bonding_curve.bin");

    #[test]
    fn reads_the_waiting_fee_at_its_idl_offset() {
        let mut data = CURVE.to_vec();
        assert_eq!(bonding_curve_creator_fee(&data).expect("decodes"), 0);

        data[125..133].copy_from_slice(&4_730_722u64.to_le_bytes());
        assert_eq!(
            bonding_curve_creator_fee(&data).expect("decodes"),
            4_730_722
        );
    }

    #[test]
    fn an_account_predating_the_field_has_nothing_waiting() {
        let mut data = CURVE.to_vec();
        data.truncate(129);
        assert_eq!(bonding_curve_creator_fee(&data).expect("decodes"), 0);
    }

    #[test]
    fn rejects_another_account_type() {
        let mut data = CURVE.to_vec();
        data[0] ^= 1;
        assert!(bonding_curve_creator_fee(&data).is_err());
    }
}

/// pump.fun's payout has to see its sweep in the same transaction, so these
/// have to fit one. The sharing config allows at most 10 shareholders.
mod payout_size {
    use super::*;
    use pump_swap_sdk::util::sharing_config_pda;
    use pump_swap_sdk::{
        WRAPPED_SOL_MINT, distribute_creator_fees_instruction,
        distribute_creator_fees_v2_instruction, sweep_bonding_curve_creator_fee_instruction,
        transfer_creator_fees_to_pump_instruction, transfer_creator_fees_to_pump_v2_instruction,
    };
    use solana_compute_budget_interface::ComputeBudgetInstruction;
    use solana_sdk::message::Message;
    use solana_sdk::transaction::Transaction;

    const MAX_TX_BYTES: usize = 1232;

    fn size(curve_sweep: bool, pool_sweep: bool, shareholders: usize, v2: bool) -> usize {
        let payer = Pubkey::new_unique();
        let mut pool = recorded_pool_sweep::pool();
        pool.quote_mint = WRAPPED_SOL_MINT;
        pool.quote_token_program = spl_token::ID;
        let mint = pool.base_mint;
        let config = sharing_config_pda(&mint);
        pool.coin_creator = config;
        let holders: Vec<Pubkey> = (0..shareholders).map(|_| Pubkey::new_unique()).collect();
        let quote = (&WRAPPED_SOL_MINT, &spl_token::ID);

        let mut ixs = vec![
            ComputeBudgetInstruction::set_compute_unit_limit(500_000),
            ComputeBudgetInstruction::set_compute_unit_price(50_000),
        ];
        if curve_sweep {
            ixs.push(
                sweep_bonding_curve_creator_fee_instruction(
                    &payer, &mint, &config, quote.0, quote.1,
                )
                .expect("builds"),
            );
        }
        if pool_sweep {
            ixs.push(sweep_pool_creator_fee_instruction(&payer, &pool).expect("builds"));
        }
        if v2 {
            ixs.push(
                transfer_creator_fees_to_pump_v2_instruction(&payer, &config, quote.0, quote.1)
                    .expect("builds"),
            );
            ixs.push(
                distribute_creator_fees_v2_instruction(
                    &payer, &mint, quote.0, quote.1, true, &holders,
                )
                .expect("builds"),
            );
        } else {
            ixs.push(transfer_creator_fees_to_pump_instruction(&config).expect("builds"));
            ixs.push(distribute_creator_fees_instruction(&mint, &holders).expect("builds"));
        }

        let tx = Transaction::new_unsigned(Message::new(&ixs, Some(&payer)));
        bincode::serde::encode_to_vec(&tx, bincode::config::legacy())
            .expect("encodes")
            .len()
    }

    #[test]
    fn the_largest_payout_fits_with_the_one_sweep_it_normally_needs() {
        for (curve, pool) in [(true, false), (false, true)] {
            let bytes = size(curve, pool, 10, false);
            assert!(bytes <= MAX_TX_BYTES, "curve={curve} pool={pool}: {bytes}");
        }
    }

    /// A graduated coin can still have a fee on its curve. Both sweeps then fit
    /// up to nine shareholders; at ten the sweeps go in their own transaction
    /// first, which is safe because that curve takes no more trades.
    #[test]
    fn both_sweeps_fit_up_to_nine_shareholders() {
        let bytes = size(true, true, 9, false);
        assert!(bytes <= MAX_TX_BYTES, "{bytes}");
        assert!(size(true, true, 10, false) > MAX_TX_BYTES);
    }

    #[test]
    fn the_largest_v2_payout_fits_with_one_sweep() {
        for (curve, pool) in [(true, false), (false, true)] {
            let bytes = size(curve, pool, 10, true);
            assert!(bytes <= MAX_TX_BYTES, "curve={curve} pool={pool}: {bytes}");
        }
    }
}

mod live {
    use super::*;
    use pump_swap_sdk::sweep_bonding_curve_creator_fee_instruction;
    use pump_swap_sdk::util::sharing_config_pda;
    use pump_swap_sdk::{PUMPFUN_PROGRAM, WRAPPED_SOL_MINT};
    use solana_client::nonblocking::rpc_client::RpcClient;
    use solana_client::rpc_config::RpcSimulateTransactionConfig;
    use solana_commitment_config::CommitmentConfig;
    use solana_sdk::message::Message;
    use solana_sdk::transaction::Transaction;

    /// A fee-sharing coin, and a funded wallet that has paid out for it.
    /// Simulation only reads the balance; nothing is signed or submitted.
    const SHARED_MINT: Pubkey = pubkey!("57KoEZXm2mJwFqbB7fvcgZmmjc9mivFmKhXA45H3pump");
    const PAYER: Pubkey = pubkey!("Ao6bNP1o68EzVKy296rR5Jg6iAUUQNU2FA65Wvs7aR2U");

    fn rpc() -> RpcClient {
        let url = std::env::var("RPC_URL")
            .unwrap_or_else(|_| "https://api.mainnet-beta.solana.com".to_string());
        RpcClient::new_with_commitment(url, CommitmentConfig::confirmed())
    }

    async fn assert_simulates_clean(ix: Instruction, payer: &Pubkey) {
        let tx = Transaction::new_unsigned(Message::new(&[ix], Some(payer)));
        let result = rpc()
            .simulate_transaction_with_config(
                &tx,
                RpcSimulateTransactionConfig {
                    sig_verify: false,
                    replace_recent_blockhash: true,
                    commitment: Some(CommitmentConfig::confirmed()),
                    ..Default::default()
                },
            )
            .await
            .expect("simulate")
            .value;
        let logs = result.logs.unwrap_or_default().join("\n");
        assert!(result.err.is_none(), "{:?}\n{logs}", result.err);
        assert!(
            logs.contains("Instruction: SweepCreatorFee"),
            "program did not reach the handler:\n{logs}"
        );
    }

    #[tokio::test]
    #[ignore = "hits mainnet RPC"]
    async fn live_pool_sweep_simulates() {
        let ix = sweep_pool_creator_fee_instruction(
            &recorded_pool_sweep::PAYER,
            &recorded_pool_sweep::pool(),
        )
        .expect("builds");
        assert_simulates_clean(ix, &recorded_pool_sweep::PAYER).await;
    }

    #[tokio::test]
    #[ignore = "hits mainnet RPC"]
    async fn live_bonding_curve_sweep_simulates() {
        let ix = sweep_bonding_curve_creator_fee_instruction(
            &PAYER,
            &SHARED_MINT,
            &sharing_config_pda(&SHARED_MINT),
            &WRAPPED_SOL_MINT,
            &spl_token::ID,
        )
        .expect("builds");
        assert_eq!(ix.program_id, PUMPFUN_PROGRAM);
        assert_simulates_clean(ix, &PAYER).await;
    }
}
