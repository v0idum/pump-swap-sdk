use crate::constants::{
    EVENT_AUTHORITY, FEE_PROGRAM, GLOBAL_CONFIG, GLOBAL_VOLUME_ACCUMULATOR, PUMP_CREATOR_VAULT,
    PUMP_SWAP_PROGRAM_ID, PUMPFUN_EVENT_AUTHORITY, PUMPFUN_PROGRAM, WRAPPED_SOL_MINT,
};
use crate::state::PoolInfo;
use crate::util::{
    calc_lp_mint_pda, calc_user_pool_token_account, fee_config_pda, find_coin_creator_vault_ata,
    find_coin_creator_vault_authority, find_user_vol_accumulator, pick_buyback_fee_recipient,
    pick_protocol_fee_recipient, pool_v2_pda, user_volume_accumulator_quote_ata,
};
use anyhow::Result;
use bytemuck::{Pod, Zeroable};
use solana_sdk::instruction::{AccountMeta, Instruction};
use solana_sdk::pubkey::Pubkey;
use solana_sdk::system_program;

/// Common trait for serializing instructions to `Vec<u8>`.
pub trait ToInstructionBytes {
    fn to_vec(&self) -> Vec<u8>;
}

impl<T: Pod> ToInstructionBytes for T {
    fn to_vec(&self) -> Vec<u8> {
        bytemuck::bytes_of(self).to_vec()
    }
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Pod, Zeroable)]
pub struct BuyInstruction {
    pub discriminator: [u8; 8],
    pub base_amount_out: u64,
    pub max_quote_amount_in: u64,
}

impl BuyInstruction {
    pub fn new(base_amount_out: u64, max_quote_amount_in: u64) -> Self {
        Self {
            discriminator: [102, 6, 61, 18, 1, 218, 235, 234],
            base_amount_out,
            max_quote_amount_in,
        }
    }
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Pod, Zeroable)]
pub struct SellInstruction {
    pub discriminator: [u8; 8],
    pub base_amount_in: u64,
    pub min_quote_amount_out: u64,
}

impl SellInstruction {
    pub fn new(base_amount_in: u64, min_quote_amount_out: u64) -> Self {
        Self {
            discriminator: [51, 230, 133, 164, 1, 127, 131, 173],
            base_amount_in,
            min_quote_amount_out,
        }
    }
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Zeroable)]
pub struct CreatePoolInstruction {
    pub discriminator: [u8; 8],
    pub index: u16,
    pub base_amount_in: u64,
    pub quote_amount_in: u64,
    pub coin_creator: Pubkey,
}

impl CreatePoolInstruction {
    pub fn new(base_amount_in: u64, quote_amount_in: u64, coin_creator: Pubkey) -> Self {
        Self {
            discriminator: [233, 146, 209, 142, 207, 104, 64, 188],
            index: 0,
            base_amount_in,
            quote_amount_in,
            coin_creator,
        }
    }

    #[allow(clippy::wrong_self_convention)]
    fn to_vec(&self) -> Vec<u8> {
        let mut buf: Vec<u8> = Vec::with_capacity(size_of::<Self>());
        buf.extend_from_slice(&self.discriminator);
        buf.extend_from_slice(&self.index.to_le_bytes());
        buf.extend_from_slice(&self.base_amount_in.to_le_bytes());
        buf.extend_from_slice(&self.quote_amount_in.to_le_bytes());
        buf.extend_from_slice(&self.coin_creator.to_bytes());
        buf
    }
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Pod, Zeroable)]
pub struct WithdrawInstruction {
    pub discriminator: [u8; 8],
    pub lp_token_amount_in: u64,
    pub min_base_amount_out: u64,
    pub min_quote_amount_out: u64,
}

impl WithdrawInstruction {
    pub fn new(
        lp_token_amount_in: u64,
        min_base_amount_out: u64,
        min_quote_amount_out: u64,
    ) -> Self {
        Self {
            discriminator: [183, 18, 70, 156, 148, 109, 161, 34],
            lp_token_amount_in,
            min_base_amount_out,
            min_quote_amount_out,
        }
    }
}

/// Build a pump-amm Buy instruction for the current canonical IDL plus
/// cashback / buyback `remaining_accounts`.
///
/// All non-input accounts are derived internally from `pool_info` and `user`:
/// the protocol-fee-recipient ATA, creator-vault PDAs, volume-accumulator
/// PDAs, fee-config PDA, plus the trailing `remaining_accounts` required by
/// the live program (cashback ATA when `pool_info.is_cashback_coin`,
/// `pool-v2` PDA when `pool_info.coin_creator != default`, and a randomly-
/// selected buyback fee recipient + its ATA).
///
/// `pool_info.base_token_program` and `pool_info.quote_token_program` decide
/// whether each side uses `spl_token` or `spl_token_2022`.
pub fn make_buy_instruction(
    base_amount_out: u64,
    max_quote_amount_in: u64,
    pool_info: &PoolInfo,
    user: &Pubkey,
    user_base_token_account: &Pubkey,
    user_quote_token_account: &Pubkey,
) -> Result<Instruction> {
    let data = BuyInstruction::new(base_amount_out, max_quote_amount_in).to_vec();

    let protocol_fee_recipient = pick_protocol_fee_recipient();
    let protocol_fee_recipient_ta =
        spl_associated_token_account::get_associated_token_address_with_program_id(
            &protocol_fee_recipient,
            &pool_info.quote_mint,
            &pool_info.quote_token_program,
        );
    let creator_vault_authority = find_coin_creator_vault_authority(&pool_info.coin_creator);
    let creator_vault_ata = find_coin_creator_vault_ata(
        &creator_vault_authority,
        &pool_info.quote_token_program,
        &pool_info.quote_mint,
    );
    let user_vol_acc = find_user_vol_accumulator(user);
    let fee_config = fee_config_pda();

    let mut accounts = vec![
        AccountMeta::new(pool_info.pool, false),
        AccountMeta::new(*user, true),
        AccountMeta::new_readonly(GLOBAL_CONFIG, false),
        AccountMeta::new_readonly(pool_info.base_mint, false),
        AccountMeta::new_readonly(pool_info.quote_mint, false),
        AccountMeta::new(*user_base_token_account, false),
        AccountMeta::new(*user_quote_token_account, false),
        AccountMeta::new(pool_info.pool_base_token_account, false),
        AccountMeta::new(pool_info.pool_quote_token_account, false),
        AccountMeta::new_readonly(protocol_fee_recipient, false),
        AccountMeta::new(protocol_fee_recipient_ta, false),
        AccountMeta::new_readonly(pool_info.base_token_program, false),
        AccountMeta::new_readonly(pool_info.quote_token_program, false),
        AccountMeta::new_readonly(system_program::ID, false),
        AccountMeta::new_readonly(spl_associated_token_account::ID, false),
        AccountMeta::new_readonly(EVENT_AUTHORITY, false),
        AccountMeta::new_readonly(PUMP_SWAP_PROGRAM_ID, false),
        AccountMeta::new(creator_vault_ata, false),
        AccountMeta::new_readonly(creator_vault_authority, false),
        AccountMeta::new_readonly(GLOBAL_VOLUME_ACCUMULATOR, false),
        AccountMeta::new(user_vol_acc, false),
        AccountMeta::new_readonly(fee_config, false),
        AccountMeta::new_readonly(FEE_PROGRAM, false),
    ];

    append_swap_remaining_accounts(&mut accounts, pool_info, user, /* is_sell */ false);

    Ok(Instruction {
        program_id: PUMP_SWAP_PROGRAM_ID,
        accounts,
        data,
    })
}

/// Build a pump-amm Sell instruction for the current canonical IDL plus
/// cashback / buyback `remaining_accounts`.
///
/// Sell omits the global/user volume accumulators that Buy carries (those
/// move into `remaining_accounts` only when `pool_info.is_cashback_coin`).
pub fn make_sell_instruction(
    base_amount_in: u64,
    min_quote_amount_out: u64,
    pool_info: &PoolInfo,
    user: &Pubkey,
    user_base_token_account: &Pubkey,
    user_quote_token_account: &Pubkey,
) -> Result<Instruction> {
    let data = SellInstruction::new(base_amount_in, min_quote_amount_out).to_vec();

    let protocol_fee_recipient = pick_protocol_fee_recipient();
    let protocol_fee_recipient_ta =
        spl_associated_token_account::get_associated_token_address_with_program_id(
            &protocol_fee_recipient,
            &pool_info.quote_mint,
            &pool_info.quote_token_program,
        );
    let creator_vault_authority = find_coin_creator_vault_authority(&pool_info.coin_creator);
    let creator_vault_ata = find_coin_creator_vault_ata(
        &creator_vault_authority,
        &pool_info.quote_token_program,
        &pool_info.quote_mint,
    );
    let fee_config = fee_config_pda();

    let mut accounts = vec![
        AccountMeta::new(pool_info.pool, false),
        AccountMeta::new(*user, true),
        AccountMeta::new_readonly(GLOBAL_CONFIG, false),
        AccountMeta::new_readonly(pool_info.base_mint, false),
        AccountMeta::new_readonly(pool_info.quote_mint, false),
        AccountMeta::new(*user_base_token_account, false),
        AccountMeta::new(*user_quote_token_account, false),
        AccountMeta::new(pool_info.pool_base_token_account, false),
        AccountMeta::new(pool_info.pool_quote_token_account, false),
        AccountMeta::new_readonly(protocol_fee_recipient, false),
        AccountMeta::new(protocol_fee_recipient_ta, false),
        AccountMeta::new_readonly(pool_info.base_token_program, false),
        AccountMeta::new_readonly(pool_info.quote_token_program, false),
        AccountMeta::new_readonly(system_program::ID, false),
        AccountMeta::new_readonly(spl_associated_token_account::ID, false),
        AccountMeta::new_readonly(EVENT_AUTHORITY, false),
        AccountMeta::new_readonly(PUMP_SWAP_PROGRAM_ID, false),
        AccountMeta::new(creator_vault_ata, false),
        AccountMeta::new_readonly(creator_vault_authority, false),
        AccountMeta::new_readonly(fee_config, false),
        AccountMeta::new_readonly(FEE_PROGRAM, false),
    ];

    append_swap_remaining_accounts(&mut accounts, pool_info, user, /* is_sell */ true);

    Ok(Instruction {
        program_id: PUMP_SWAP_PROGRAM_ID,
        accounts,
        data,
    })
}

/// Append the live program's `remaining_accounts` for Buy / Sell. Mirrors
/// the pump-fun npm SDK `offlinePumpAmm` logic.
///
/// Order:
/// 1. (if cashback) `user_volume_accumulator_quote_ata` (writable). Sell also
///    pushes `user_volume_accumulator` (writable) immediately after.
/// 2. (if `pool.coin_creator != default`) `pool_v2_pda(base_mint)` (read-only).
/// 3. Always: `buyback_fee_recipient` (read-only),
///    `buyback_fee_recipient_quote_ata` (writable).
fn append_swap_remaining_accounts(
    accounts: &mut Vec<AccountMeta>,
    pool_info: &PoolInfo,
    user: &Pubkey,
    is_sell: bool,
) {
    if pool_info.is_cashback_coin {
        let cashback_ata = user_volume_accumulator_quote_ata(
            user,
            &pool_info.quote_mint,
            &pool_info.quote_token_program,
        );
        accounts.push(AccountMeta::new(cashback_ata, false));
        if is_sell {
            accounts.push(AccountMeta::new(find_user_vol_accumulator(user), false));
        }
    }

    if pool_info.coin_creator != Pubkey::default() {
        accounts.push(AccountMeta::new_readonly(
            pool_v2_pda(&pool_info.base_mint),
            false,
        ));
    }

    let buyback_recipient = pick_buyback_fee_recipient();
    let buyback_recipient_ta =
        spl_associated_token_account::get_associated_token_address_with_program_id(
            &buyback_recipient,
            &pool_info.quote_mint,
            &pool_info.quote_token_program,
        );
    accounts.push(AccountMeta::new_readonly(buyback_recipient, false));
    accounts.push(AccountMeta::new(buyback_recipient_ta, false));
}

/// Build a pump-amm `create_pool` instruction for a WSOL-quoted pool.
#[allow(clippy::too_many_arguments)]
pub fn create_pool_instruction(
    base_amount_in: u64,
    quote_amount_in: u64,
    pool: &Pubkey,
    creator: &Pubkey,
    coin_creator: &Pubkey,
    base_mint: &Pubkey,
    quote_mint: &Pubkey,
    user_base_token_account: &Pubkey,
    user_quote_token_account: &Pubkey,
    pool_base_token_account: &Pubkey,
    pool_quote_token_account: &Pubkey,
) -> Result<Instruction> {
    let data = CreatePoolInstruction::new(base_amount_in, quote_amount_in, *coin_creator).to_vec();

    let lp_mint = calc_lp_mint_pda(pool).0;

    let accounts = vec![
        AccountMeta::new(*pool, false),
        AccountMeta::new_readonly(GLOBAL_CONFIG, false),
        AccountMeta::new(*creator, true),
        AccountMeta::new_readonly(*base_mint, false),
        AccountMeta::new_readonly(*quote_mint, false),
        AccountMeta::new(lp_mint, false),
        AccountMeta::new(*user_base_token_account, false),
        AccountMeta::new(*user_quote_token_account, false),
        AccountMeta::new(calc_user_pool_token_account(creator, &lp_mint).0, false),
        AccountMeta::new(*pool_base_token_account, false),
        AccountMeta::new(*pool_quote_token_account, false),
        AccountMeta::new_readonly(system_program::ID, false),
        AccountMeta::new_readonly(spl_token_2022::ID, false),
        AccountMeta::new_readonly(spl_token::ID, false),
        AccountMeta::new_readonly(spl_token::ID, false),
        AccountMeta::new_readonly(spl_associated_token_account::ID, false),
        AccountMeta::new_readonly(EVENT_AUTHORITY, false),
        AccountMeta::new_readonly(PUMP_SWAP_PROGRAM_ID, false),
    ];

    Ok(Instruction {
        program_id: PUMP_SWAP_PROGRAM_ID,
        accounts,
        data,
    })
}

/// Build a pump-amm `withdraw` instruction for a WSOL-quoted pool.
#[allow(clippy::too_many_arguments)]
pub fn withdraw_instruction(
    pool: &Pubkey,
    user: &Pubkey,
    base_mint: &Pubkey,
    base_ata: &Pubkey,
    quote_ata: &Pubkey,
    lp_token_amount_in: u64,
    min_base_amount_out: u64,
    min_quote_amount_out: u64,
) -> Result<Instruction> {
    let data = WithdrawInstruction::new(
        lp_token_amount_in,
        min_base_amount_out,
        min_quote_amount_out,
    )
    .to_vec();

    let lp_mint = calc_lp_mint_pda(pool).0;
    let user_pool_token_account = calc_user_pool_token_account(user, &lp_mint).0;

    let pool_base_ata = spl_associated_token_account::get_associated_token_address(pool, base_mint);
    let pool_quote_ata =
        spl_associated_token_account::get_associated_token_address(pool, &WRAPPED_SOL_MINT);

    let accounts = vec![
        AccountMeta::new(*pool, false),
        AccountMeta::new_readonly(GLOBAL_CONFIG, false),
        AccountMeta::new_readonly(*user, true),
        AccountMeta::new_readonly(*base_mint, false),
        AccountMeta::new_readonly(WRAPPED_SOL_MINT, false),
        AccountMeta::new(lp_mint, false),
        AccountMeta::new(*base_ata, false),
        AccountMeta::new(*quote_ata, false),
        AccountMeta::new(user_pool_token_account, false),
        AccountMeta::new(pool_base_ata, false),
        AccountMeta::new(pool_quote_ata, false),
        AccountMeta::new_readonly(spl_token::ID, false),
        AccountMeta::new_readonly(spl_token_2022::ID, false),
        AccountMeta::new_readonly(EVENT_AUTHORITY, false),
        AccountMeta::new_readonly(PUMP_SWAP_PROGRAM_ID, false),
    ];

    Ok(Instruction {
        program_id: PUMP_SWAP_PROGRAM_ID,
        accounts,
        data,
    })
}

/// Build the pump-amm instruction that moves accrued creator fees from the
/// coin-creator vault back into the pump.fun program's creator vault, so they
/// can subsequently be distributed via [`distribute_creator_fees_instruction`].
pub fn transfer_creator_fees_to_pump_instruction(coin_creator: &Pubkey) -> Result<Instruction> {
    let coin_creator_vault_authority = find_coin_creator_vault_authority(coin_creator);
    let coin_creator_vault_ata = find_coin_creator_vault_ata(
        &coin_creator_vault_authority,
        &spl_token::ID,
        &WRAPPED_SOL_MINT,
    );

    let accounts = vec![
        AccountMeta::new_readonly(WRAPPED_SOL_MINT, false),
        AccountMeta::new_readonly(spl_token::ID, false),
        AccountMeta::new_readonly(system_program::ID, false),
        AccountMeta::new_readonly(spl_associated_token_account::ID, false),
        AccountMeta::new_readonly(*coin_creator, false),
        AccountMeta::new(coin_creator_vault_authority, false),
        AccountMeta::new(coin_creator_vault_ata, false),
        AccountMeta::new(PUMP_CREATOR_VAULT, false),
        AccountMeta::new_readonly(EVENT_AUTHORITY, false),
        AccountMeta::new_readonly(PUMP_SWAP_PROGRAM_ID, false),
    ];

    Ok(Instruction {
        program_id: PUMP_SWAP_PROGRAM_ID,
        accounts,
        data: vec![139, 52, 134, 85, 228, 229, 108, 241],
    })
}

/// pump.fun `distribute_creator_fees` instruction — pays out fees from the pump
/// program's creator vault to the admin/sharing destinations.
pub fn distribute_creator_fees_instruction(
    mint: &Pubkey,
    bonding_curve: &Pubkey,
    sharing_config: &Pubkey,
    admin_account: &Pubkey,
) -> Result<Instruction> {
    let accounts = vec![
        AccountMeta::new_readonly(*mint, false),
        AccountMeta::new_readonly(*bonding_curve, false),
        AccountMeta::new_readonly(*sharing_config, false),
        AccountMeta::new(PUMP_CREATOR_VAULT, false),
        AccountMeta::new_readonly(system_program::ID, false),
        AccountMeta::new_readonly(PUMPFUN_EVENT_AUTHORITY, false),
        AccountMeta::new_readonly(PUMPFUN_PROGRAM, false),
        AccountMeta::new(*admin_account, true),
    ];

    Ok(Instruction {
        program_id: PUMPFUN_PROGRAM,
        accounts,
        data: vec![165, 114, 103, 0, 121, 206, 247, 81],
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pk(byte: u8) -> Pubkey {
        Pubkey::new_from_array([byte; 32])
    }

    fn pool_info(coin_creator: Pubkey, is_cashback_coin: bool) -> PoolInfo {
        PoolInfo {
            pool: pk(1),
            base_mint: pk(2),
            quote_mint: WRAPPED_SOL_MINT,
            lp_mint: pk(3),
            pool_base_token_account: pk(4),
            pool_quote_token_account: pk(5),
            creator: pk(6),
            coin_creator,
            is_cashback_coin,
            base_token_program: spl_token::ID,
            quote_token_program: spl_token::ID,
        }
    }

    #[test]
    fn swap_instruction_account_counts_include_required_remaining_accounts() {
        let user = pk(7);
        let user_base_ata = pk(8);
        let user_quote_ata = pk(9);

        let classic = pool_info(Pubkey::default(), false);
        assert_eq!(
            make_buy_instruction(1, 2, &classic, &user, &user_base_ata, &user_quote_ata)
                .unwrap()
                .accounts
                .len(),
            25
        );
        assert_eq!(
            make_sell_instruction(1, 2, &classic, &user, &user_base_ata, &user_quote_ata)
                .unwrap()
                .accounts
                .len(),
            23
        );

        let creator_pool = pool_info(pk(10), false);
        assert_eq!(
            make_buy_instruction(1, 2, &creator_pool, &user, &user_base_ata, &user_quote_ata)
                .unwrap()
                .accounts
                .len(),
            26
        );
        assert_eq!(
            make_sell_instruction(1, 2, &creator_pool, &user, &user_base_ata, &user_quote_ata)
                .unwrap()
                .accounts
                .len(),
            24
        );

        let cashback_creator_pool = pool_info(pk(10), true);
        assert_eq!(
            make_buy_instruction(
                1,
                2,
                &cashback_creator_pool,
                &user,
                &user_base_ata,
                &user_quote_ata,
            )
            .unwrap()
            .accounts
            .len(),
            27
        );
        assert_eq!(
            make_sell_instruction(
                1,
                2,
                &cashback_creator_pool,
                &user,
                &user_base_ata,
                &user_quote_ata,
            )
            .unwrap()
            .accounts
            .len(),
            26
        );
    }
}
