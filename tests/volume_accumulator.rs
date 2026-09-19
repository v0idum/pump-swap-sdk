//! Decoding tests for pump-amm's `GlobalVolumeAccumulator` and
//! `UserVolumeAccumulator` accounts.
//!
//! The offline tests run against byte-for-byte mainnet fixtures under
//! `tests/fixtures/`, captured at slot 448520197 (2026-09-20), so CI never
//! depends on RPC availability. The `#[ignore]`d tests re-run the same
//! assertions against live chain state:
//!
//! ```text
//! cargo test --test volume_accumulator -- --ignored
//! RPC_URL=https://my-private-rpc cargo test --test volume_accumulator -- --ignored
//! ```

use pump_swap_sdk::{
    GlobalVolumeAccumulator, UserVolumeAccumulator, VOLUME_ACCUMULATOR_DAYS,
    find_user_vol_accumulator,
};
use solana_sdk::pubkey;
use solana_sdk::pubkey::Pubkey;

const GLOBAL_FIXTURE: &[u8] = include_bytes!("fixtures/global_volume_accumulator.bin");
const USER_FIXTURE: &[u8] = include_bytes!("fixtures/user_volume_accumulator.bin");

/// The trader whose accumulator `user_volume_accumulator.bin` captures, and
/// the PDA the fixture was read from.
///
/// Picked from `claim_cashback` transaction
/// `jXWaHahomWjQ9fQn4LuBnXYf9m3Wci89RAoV8hxf8zcNQnN6ibhBTyNefKVZH9AEbzGLf4Xag582zYm6PFmBQDA`
/// (slot 439611975), so the account has real cashback counters rather than
/// the zeros most live accumulators carry.
const FIXTURE_USER: Pubkey = pubkey!("YbBux65QrEjMj3UiraGgxBtyd6Jfioi5ofLd3M7kFC6");
const FIXTURE_ACCUMULATOR_PDA: Pubkey = pubkey!("112P247K33FPVxY8PHrT4u5dowFibaKhYcoForDK1FP");

fn global() -> GlobalVolumeAccumulator {
    GlobalVolumeAccumulator::from_account_data(GLOBAL_FIXTURE).expect("fixture decodes")
}

fn user() -> UserVolumeAccumulator {
    UserVolumeAccumulator::from_account_data(USER_FIXTURE).expect("fixture decodes")
}

#[test]
fn global_accumulator_has_thirty_day_buckets() {
    let accumulator = global();
    assert_eq!(accumulator.total_token_supply.len(), 30);
    assert_eq!(accumulator.sol_volumes.len(), 30);
    assert_eq!(VOLUME_ACCUMULATOR_DAYS, 30);
}

/// The live account is 600 bytes against a 544-byte layout, and the whole
/// layout decodes without running off the end.
#[test]
fn global_accumulator_fixture_has_the_expected_shape() {
    assert_eq!(GLOBAL_FIXTURE.len(), 600);
    assert!(GLOBAL_FIXTURE.len() > GlobalVolumeAccumulator::ENCODED_LEN);
    assert_eq!(GlobalVolumeAccumulator::ENCODED_LEN, 544);
}

/// The token-incentive program is dormant: the live account is zeroed past
/// its discriminator. This is real state, not a decode failure — the
/// discriminator check is what separates the two — so pin it, and let the
/// `#[ignore]`d live test below be the alarm for the day it changes.
#[test]
fn global_accumulator_is_dormant_on_mainnet() {
    let accumulator = global();
    assert_eq!(accumulator.start_time, 0);
    assert_eq!(accumulator.end_time, 0);
    assert_eq!(accumulator.seconds_in_a_day, 0);
    assert_eq!(accumulator.mint, Pubkey::default());
    assert!(accumulator.total_token_supply.iter().all(|x| *x == 0));
    assert!(accumulator.sol_volumes.iter().all(|x| *x == 0));
    assert!(!accumulator.is_active_at(i64::MAX));
    assert_eq!(accumulator.day_index(0), None);
}

#[test]
fn day_index_buckets_by_day_within_the_program_window() {
    let accumulator = GlobalVolumeAccumulator {
        start_time: 1_000,
        end_time: 1_000 + 86_400 * 30,
        seconds_in_a_day: 86_400,
        ..global()
    };

    assert_eq!(accumulator.day_index(999), None);
    assert_eq!(accumulator.day_index(1_000), Some(0));
    assert_eq!(accumulator.day_index(1_000 + 86_399), Some(0));
    assert_eq!(accumulator.day_index(1_000 + 86_400), Some(1));
    assert_eq!(accumulator.day_index(1_000 + 86_400 * 29), Some(29));
    // Exactly `end_time` is past the window, so there is no 30th bucket.
    assert_eq!(accumulator.day_index(1_000 + 86_400 * 30), None);
}

/// A non-positive `seconds_in_a_day` would divide by zero or run backwards.
#[test]
fn day_index_rejects_a_non_positive_day_length() {
    let accumulator = GlobalVolumeAccumulator {
        start_time: 0,
        end_time: i64::MAX,
        seconds_in_a_day: 0,
        ..global()
    };
    assert_eq!(accumulator.day_index(1_000), None);
}

/// `user` is the accumulator's only PDA seed, so a correct decode of the
/// first field round-trips back to the account the fixture was read from.
#[test]
fn user_accumulator_field_round_trips_to_its_own_pda() {
    assert_eq!(user().user, FIXTURE_USER);
    assert_eq!(
        find_user_vol_accumulator(&FIXTURE_USER),
        FIXTURE_ACCUMULATOR_PDA
    );
}

/// Field values as of slot 448520197, right after the `claim_cashback` the
/// fixture was picked from. `cashback_earned` and `total_cashback_claimed`
/// are the only non-zero counters, and the claimed total is the larger of the
/// two — which is why `UserVolumeAccumulator` does not subtract them into a
/// "claimable" number.
#[test]
fn user_accumulator_decodes_its_cashback_counters() {
    let accumulator = user();

    assert_eq!(accumulator.cashback_earned, 89_356_599);
    assert_eq!(accumulator.total_cashback_claimed, 249_797_601);
    assert!(accumulator.total_cashback_claimed > accumulator.cashback_earned);

    assert!(!accumulator.needs_claim);
    assert_eq!(accumulator.total_unclaimed_tokens, 0);
    assert_eq!(accumulator.total_claimed_tokens, 0);
    assert_eq!(accumulator.current_sol_volume, 0);
    assert_eq!(accumulator.last_update_timestamp, 0);
    assert!(!accumulator.has_total_claimed_tokens);
}

/// A `u64`-width or offset slip in the tail fields would still produce
/// plausible-looking numbers, so pin the raw little-endian bytes the two
/// cashback counters are read from.
#[test]
fn user_accumulator_cashback_counters_sit_where_the_idl_puts_them() {
    assert_eq!(
        &USER_FIXTURE[74..82],
        &89_356_599u64.to_le_bytes(),
        "cashback_earned offset"
    );
    assert_eq!(
        &USER_FIXTURE[82..90],
        &249_797_601u64.to_le_bytes(),
        "total_cashback_claimed offset"
    );
}

/// The live account is 137 bytes against a 90-byte layout.
#[test]
fn user_accumulator_fixture_has_the_expected_shape() {
    assert_eq!(USER_FIXTURE.len(), 137);
    assert!(USER_FIXTURE.len() > UserVolumeAccumulator::ENCODED_LEN);
    assert_eq!(UserVolumeAccumulator::ENCODED_LEN, 90);
}

#[test]
fn empty_accumulator_carries_the_user_and_nothing_else() {
    let user = Pubkey::new_unique();
    let empty = UserVolumeAccumulator::empty(user);
    assert_eq!(empty.user, user);
    assert!(!empty.needs_claim);
    assert_eq!(empty.total_unclaimed_tokens, 0);
    assert_eq!(empty.total_claimed_tokens, 0);
    assert_eq!(empty.current_sol_volume, 0);
    assert_eq!(empty.last_update_timestamp, 0);
    assert!(!empty.has_total_claimed_tokens);
    assert_eq!(empty.cashback_earned, 0);
    assert_eq!(empty.total_cashback_claimed, 0);
}

#[test]
fn decoding_rejects_a_foreign_discriminator() {
    let mut data = USER_FIXTURE.to_vec();
    data[0] ^= 0xff;
    let err = UserVolumeAccumulator::from_account_data(&data).expect_err("bad discriminator");
    assert!(
        err.to_string().contains("discriminator"),
        "unexpected error: {err}"
    );

    // The two accounts must not decode as each other.
    assert!(UserVolumeAccumulator::from_account_data(GLOBAL_FIXTURE).is_err());
    assert!(GlobalVolumeAccumulator::from_account_data(USER_FIXTURE).is_err());
}

#[test]
fn decoding_rejects_truncated_accounts() {
    assert!(UserVolumeAccumulator::from_account_data(&USER_FIXTURE[..50]).is_err());
    assert!(GlobalVolumeAccumulator::from_account_data(&GLOBAL_FIXTURE[..50]).is_err());
    assert!(UserVolumeAccumulator::from_account_data(&[]).is_err());
}

/// Both accounts are over-allocated past the IDL layout, so decoding must not
/// require an exact length.
#[test]
fn decoding_tolerates_trailing_account_slack() {
    let mut data = USER_FIXTURE.to_vec();
    data.extend_from_slice(&[0xab; 64]);
    assert_eq!(
        UserVolumeAccumulator::from_account_data(&data).expect("decodes"),
        user()
    );

    let mut data = GLOBAL_FIXTURE.to_vec();
    data.extend_from_slice(&[0xab; 64]);
    assert_eq!(
        GlobalVolumeAccumulator::from_account_data(&data).expect("decodes"),
        global()
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
    async fn live_global_volume_accumulator_decodes() {
        let accumulator = client()
            .fetch_global_volume_accumulator()
            .await
            .expect("fetch global volume accumulator");

        assert_eq!(accumulator.total_token_supply.len(), 30);
        assert_eq!(accumulator.sol_volumes.len(), 30);
    }

    #[tokio::test]
    #[ignore = "hits mainnet RPC"]
    async fn live_user_volume_accumulator_decodes() {
        let accumulator = client()
            .fetch_user_volume_accumulator(&FIXTURE_USER)
            .await
            .expect("fetch user volume accumulator")
            .expect("fixture user has an accumulator");

        assert_eq!(accumulator.user, FIXTURE_USER);
    }

    /// The case the API exists for: a user who has never traded has no PDA,
    /// and that must not be an error.
    #[tokio::test]
    #[ignore = "hits mainnet RPC"]
    async fn live_missing_user_volume_accumulator_is_none() {
        let never_traded = Pubkey::new_unique();
        let accumulator = client()
            .fetch_user_volume_accumulator(&never_traded)
            .await
            .expect("missing account is not an error");

        assert!(accumulator.is_none());
    }
}
