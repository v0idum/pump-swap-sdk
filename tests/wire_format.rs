//! Invariants that a dependency bump must not move.
//!
//! These lock down two byte-level contracts that the rest of the suite cannot
//! see fail: the in-memory size of [`Pool`], which fixes the offset that
//! `PoolInfo::from_account_data` reads `virtual_quote_reserves` from, and the
//! `bincode` encoding of a signed [`Transaction`], which is what
//! `send_jito_bundle` puts on the wire.

use pump_swap_sdk::Pool;
use solana_sdk::hash::Hash;
use solana_sdk::instruction::{AccountMeta, Instruction};
use solana_sdk::message::Message;
use solana_sdk::pubkey::Pubkey;
use solana_sdk::signature::{Keypair, Signer};
use solana_sdk::transaction::Transaction;

/// `1 + 2 + 6*32 + 8 + 32 + 1 + 1`, with `repr(C, packed)` adding no padding.
///
/// `PoolInfo::from_account_data` reads `virtual_quote_reserves` at
/// `size_of::<Pool>() + 8`. If a `bytemuck` or `solana-pubkey` bump changed
/// `Pubkey`'s size or the derive's padding behaviour, that offset would move
/// silently and mispriced pools would be the only symptom.
#[test]
fn pool_layout_is_237_packed_bytes() {
    assert_eq!(size_of::<Pool>(), 237, "Pool layout moved");
    assert_eq!(align_of::<Pool>(), 1, "Pool is no longer packed");
}

/// Captured from `bincode` 1.3.3's `serialize`, the version this SDK shipped
/// with before the move to `bincode` 2's `serde` module. `bincode::config::legacy()`
/// is the configuration that reproduces it (little-endian, fixed-int).
const GOLDEN_TX: [u8; 260] = [
    1, 5, 59, 157, 241, 65, 108, 204, 27, 249, 140, 235, 244, 28, 3, 124, 194, 40, 160, 17, 109,
    31, 184, 170, 72, 124, 101, 186, 208, 9, 89, 182, 205, 63, 156, 118, 229, 147, 185, 214, 223,
    15, 224, 54, 43, 39, 154, 40, 157, 77, 215, 181, 92, 29, 156, 248, 230, 77, 37, 99, 232, 254,
    18, 235, 15, 1, 0, 2, 4, 237, 73, 40, 198, 40, 209, 194, 198, 234, 233, 3, 56, 144, 89, 149,
    97, 41, 89, 39, 58, 92, 99, 249, 54, 54, 193, 70, 20, 172, 135, 55, 209, 9, 9, 9, 9, 9, 9, 9,
    9, 9, 9, 9, 9, 9, 9, 9, 9, 9, 9, 9, 9, 9, 9, 9, 9, 9, 9, 9, 9, 9, 9, 9, 9, 0, 0, 0, 0, 0, 0, 0,
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 5, 5, 5, 5, 5, 5, 5,
    5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 7, 7, 7, 7, 7, 7, 7,
    7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 2, 3, 2, 0, 1, 8, 1,
    2, 3, 4, 5, 6, 7, 8, 2, 2, 0, 1, 12, 2, 0, 0, 0, 135, 214, 18, 0, 0, 0, 0, 0,
];

fn golden_transaction() -> Transaction {
    let payer = Keypair::new_from_array([3u8; 32]);
    let dest = Pubkey::new_from_array([9u8; 32]);
    let program = Pubkey::new_from_array([5u8; 32]);
    let ix = Instruction::new_with_bytes(
        program,
        &[1u8, 2, 3, 4, 5, 6, 7, 8],
        vec![
            AccountMeta::new(payer.pubkey(), true),
            AccountMeta::new_readonly(dest, false),
        ],
    );
    let sys = solana_system_interface::instruction::transfer(&payer.pubkey(), &dest, 1_234_567);
    let msg = Message::new(&[ix, sys], Some(&payer.pubkey()));
    let mut tx = Transaction::new_unsigned(msg);
    tx.sign(&[&payer], Hash::new_from_array([7u8; 32]));
    tx
}

/// `send_jito_bundle` base64-encodes this; a change here silently corrupts
/// every bundle the SDK submits.
#[test]
fn bincode_legacy_transaction_encoding_is_unchanged() {
    let encoded =
        bincode::serde::encode_to_vec(golden_transaction(), bincode::config::legacy()).unwrap();
    assert_eq!(encoded, GOLDEN_TX, "Transaction wire encoding changed");
}

/// The other direction: `build_*_ixs` funding assertions decode the system
/// instruction back out of the built instruction data.
#[test]
fn bincode_legacy_decodes_system_instructions() {
    use solana_system_interface::instruction::SystemInstruction;

    let payer = Pubkey::new_from_array([3u8; 32]);
    let base = Pubkey::new_from_array([9u8; 32]);
    let owner = Pubkey::new_from_array([5u8; 32]);
    let ix = solana_system_interface::instruction::create_account_with_seed(
        &payer, &base, &payer, "seed", 2_039_280, 165, &owner,
    );

    let (decoded, _) = bincode::serde::decode_from_slice::<SystemInstruction, _>(
        &ix.data,
        bincode::config::legacy(),
    )
    .unwrap();

    match decoded {
        SystemInstruction::CreateAccountWithSeed {
            lamports, space, ..
        } => {
            assert_eq!(lamports, 2_039_280);
            assert_eq!(space, 165);
        }
        other => panic!("unexpected variant: {other:?}"),
    }
}
