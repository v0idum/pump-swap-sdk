use crate::constants::{
    EVENT_AUTHORITY, FEE_PROGRAM, GLOBAL_CONFIG, GLOBAL_VOLUME_ACCUMULATOR, PUMP_CREATOR_VAULT,
    PUMP_SWAP_PROGRAM_ID, PUMPFUN_EVENT_AUTHORITY, PUMPFUN_PROGRAM, WRAPPED_SOL_MINT,
};
use crate::state::PoolInfo;
use crate::util::{
    calc_lp_mint_pda, calc_user_pool_token_account, fee_config_pda, find_coin_creator_vault_ata,
    find_coin_creator_vault_authority, find_user_vol_accumulator, pick_buyback_fee_recipient,
    pick_protocol_fee_recipient_for_pool, pool_v2_pda, user_volume_accumulator_quote_ata,
};
use anyhow::Result;
use bytemuck::{Pod, Zeroable};
use solana_sdk::instruction::{AccountMeta, Instruction};
use solana_sdk::pubkey::Pubkey;
use solana_system_interface::program as system_program;

/// Common trait for serializing instructions to `Vec<u8>`.
pub trait ToInstructionBytes {
    fn to_vec(&self) -> Vec<u8>;
}

impl<T: Pod> ToInstructionBytes for T {
    fn to_vec(&self) -> Vec<u8> {
        bytemuck::bytes_of(self).to_vec()
    }
}

/// pump-amm `buy` instruction args — buy an *exact* amount of base out by
/// spending up to `max_quote_amount_in` of quote.
///
/// On-chain layout: `[discriminator(8) | base_amount_out(8) |
/// max_quote_amount_in(8) | track_volume(1)]` = 25 bytes.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct BuyInstruction {
    pub base_amount_out: u64,
    pub max_quote_amount_in: u64,
    /// Tells the program whether to write this trade's quote-in into the
    /// caller's `user_volume_accumulator` for cashback / incentive tracking.
    pub track_volume: bool,
}

impl BuyInstruction {
    pub const DISCRIMINATOR: [u8; 8] = [102, 6, 61, 18, 1, 218, 235, 234];

    pub fn new(base_amount_out: u64, max_quote_amount_in: u64, track_volume: bool) -> Self {
        Self {
            base_amount_out,
            max_quote_amount_in,
            track_volume,
        }
    }

    pub fn to_vec(&self) -> Vec<u8> {
        let mut buf = Vec::with_capacity(25);
        buf.extend_from_slice(&Self::DISCRIMINATOR);
        buf.extend_from_slice(&self.base_amount_out.to_le_bytes());
        buf.extend_from_slice(&self.max_quote_amount_in.to_le_bytes());
        buf.push(self.track_volume as u8);
        buf
    }
}

/// pump-amm `buy_exact_quote_in` instruction args — spend an *exact* amount
/// of quote, getting at least `min_base_amount_out` of base.
///
/// On-chain layout: `[discriminator(8) | spendable_quote_in(8) |
/// min_base_amount_out(8) | track_volume(1)]` = 25 bytes.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct BuyExactQuoteInInstruction {
    pub spendable_quote_in: u64,
    pub min_base_amount_out: u64,
    pub track_volume: bool,
}

impl BuyExactQuoteInInstruction {
    pub const DISCRIMINATOR: [u8; 8] = [198, 46, 21, 82, 180, 217, 232, 112];

    pub fn new(spendable_quote_in: u64, min_base_amount_out: u64, track_volume: bool) -> Self {
        Self {
            spendable_quote_in,
            min_base_amount_out,
            track_volume,
        }
    }

    pub fn to_vec(&self) -> Vec<u8> {
        let mut buf = Vec::with_capacity(25);
        buf.extend_from_slice(&Self::DISCRIMINATOR);
        buf.extend_from_slice(&self.spendable_quote_in.to_le_bytes());
        buf.extend_from_slice(&self.min_base_amount_out.to_le_bytes());
        buf.push(self.track_volume as u8);
        buf
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

/// pump-amm `deposit` instruction args — add liquidity to a pool, minting
/// `lp_token_amount_out` LP tokens in exchange for up to `max_base_amount_in`
/// base and `max_quote_amount_in` quote.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Pod, Zeroable)]
pub struct DepositInstruction {
    pub discriminator: [u8; 8],
    pub lp_token_amount_out: u64,
    pub max_base_amount_in: u64,
    pub max_quote_amount_in: u64,
}

impl DepositInstruction {
    pub fn new(
        lp_token_amount_out: u64,
        max_base_amount_in: u64,
        max_quote_amount_in: u64,
    ) -> Self {
        Self {
            discriminator: [242, 35, 198, 137, 82, 225, 242, 182],
            lp_token_amount_out,
            max_base_amount_in,
            max_quote_amount_in,
        }
    }
}

/// pump-amm `claim_cashback` instruction args — takes no arguments; the
/// `user_volume_accumulator` PDA tells the program how much to pay out.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ClaimCashbackInstruction;

impl ClaimCashbackInstruction {
    pub const DISCRIMINATOR: [u8; 8] = [37, 58, 35, 126, 190, 53, 228, 197];

    pub fn to_vec(&self) -> Vec<u8> {
        Self::DISCRIMINATOR.to_vec()
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
    pub is_mayhem_mode: u8,
    pub is_cashback_coin: u8,
}

impl CreatePoolInstruction {
    pub fn new(base_amount_in: u64, quote_amount_in: u64, coin_creator: Pubkey) -> Self {
        Self::new_with_options(
            0,
            base_amount_in,
            quote_amount_in,
            coin_creator,
            false,
            false,
        )
    }

    pub fn new_with_options(
        index: u16,
        base_amount_in: u64,
        quote_amount_in: u64,
        coin_creator: Pubkey,
        is_mayhem_mode: bool,
        is_cashback_coin: bool,
    ) -> Self {
        Self {
            discriminator: [233, 146, 209, 142, 207, 104, 64, 188],
            index,
            base_amount_in,
            quote_amount_in,
            coin_creator,
            is_mayhem_mode: is_mayhem_mode as u8,
            is_cashback_coin: is_cashback_coin as u8,
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
        buf.push(self.is_mayhem_mode);
        buf.push(self.is_cashback_coin);
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

/// Build a pump-amm `buy` instruction (exact-base-out) for the current
/// canonical IDL plus cashback / buyback `remaining_accounts`.
///
/// `track_volume = true` makes the program write this trade's quote-in to
/// the caller's `user_volume_accumulator` PDA, which is required for any
/// future `claim_cashback` payout to include it.
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
    track_volume: bool,
    pool_info: &PoolInfo,
    user: &Pubkey,
    user_base_token_account: &Pubkey,
    user_quote_token_account: &Pubkey,
) -> Result<Instruction> {
    let data = BuyInstruction::new(base_amount_out, max_quote_amount_in, track_volume).to_vec();
    Ok(Instruction {
        program_id: PUMP_SWAP_PROGRAM_ID,
        accounts: swap_accounts(
            pool_info,
            user,
            user_base_token_account,
            user_quote_token_account,
            SwapKind::Buy,
        ),
        data,
    })
}

/// Build a pump-amm `buy_exact_quote_in` instruction (exact-quote-in) — the
/// "spend exactly X SOL, get at least Y tokens" entry point most trader
/// bots want. Account layout is identical to [`make_buy_instruction`];
/// only the instruction data (discriminator + args) differs.
pub fn make_buy_exact_quote_in_instruction(
    spendable_quote_in: u64,
    min_base_amount_out: u64,
    track_volume: bool,
    pool_info: &PoolInfo,
    user: &Pubkey,
    user_base_token_account: &Pubkey,
    user_quote_token_account: &Pubkey,
) -> Result<Instruction> {
    let data =
        BuyExactQuoteInInstruction::new(spendable_quote_in, min_base_amount_out, track_volume)
            .to_vec();
    Ok(Instruction {
        program_id: PUMP_SWAP_PROGRAM_ID,
        accounts: swap_accounts(
            pool_info,
            user,
            user_base_token_account,
            user_quote_token_account,
            SwapKind::Buy,
        ),
        data,
    })
}

#[derive(Clone, Copy)]
enum SwapKind {
    Buy,
    Sell,
}

/// Account list shared by `buy` / `buy_exact_quote_in` (23 IDL + 1–4
/// `remaining_accounts`) and `sell` (21 IDL + 1–3 `remaining_accounts`).
fn swap_accounts(
    pool_info: &PoolInfo,
    user: &Pubkey,
    user_base_token_account: &Pubkey,
    user_quote_token_account: &Pubkey,
    kind: SwapKind,
) -> Vec<AccountMeta> {
    let protocol_fee_recipient = pick_protocol_fee_recipient_for_pool(pool_info.is_mayhem_mode);
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
    ];

    if matches!(kind, SwapKind::Buy) {
        accounts.push(AccountMeta::new_readonly(GLOBAL_VOLUME_ACCUMULATOR, false));
        accounts.push(AccountMeta::new(find_user_vol_accumulator(user), false));
    }

    accounts.push(AccountMeta::new_readonly(fee_config, false));
    accounts.push(AccountMeta::new_readonly(FEE_PROGRAM, false));

    let is_sell = matches!(kind, SwapKind::Sell);
    append_swap_remaining_accounts(&mut accounts, pool_info, user, is_sell);
    accounts
}

/// Build a pump-amm `sell` instruction for the current canonical IDL plus
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
    Ok(Instruction {
        program_id: PUMP_SWAP_PROGRAM_ID,
        accounts: swap_accounts(
            pool_info,
            user,
            user_base_token_account,
            user_quote_token_account,
            SwapKind::Sell,
        ),
        data,
    })
}

/// Build a pump-amm `deposit` instruction — add liquidity, minting LP tokens.
///
/// The user must pre-create their LP-token ATA (`user_pool_token_account`)
/// for `lp_mint = calc_lp_mint_pda(pool)`. Use
/// [`PumpSwapClient::deposit_into_wsol_pool`](crate::client::PumpSwapClient::deposit_into_wsol_pool)
/// for the full convenience flow including ATA creation.
#[allow(clippy::too_many_arguments)]
pub fn make_deposit_instruction(
    lp_token_amount_out: u64,
    max_base_amount_in: u64,
    max_quote_amount_in: u64,
    pool_info: &PoolInfo,
    user: &Pubkey,
    user_base_token_account: &Pubkey,
    user_quote_token_account: &Pubkey,
    user_pool_token_account: &Pubkey,
) -> Result<Instruction> {
    let data =
        DepositInstruction::new(lp_token_amount_out, max_base_amount_in, max_quote_amount_in)
            .to_vec();

    let accounts = vec![
        AccountMeta::new(pool_info.pool, false),
        AccountMeta::new_readonly(GLOBAL_CONFIG, false),
        AccountMeta::new_readonly(*user, true),
        AccountMeta::new_readonly(pool_info.base_mint, false),
        AccountMeta::new_readonly(pool_info.quote_mint, false),
        AccountMeta::new(pool_info.lp_mint, false),
        AccountMeta::new(*user_base_token_account, false),
        AccountMeta::new(*user_quote_token_account, false),
        AccountMeta::new(*user_pool_token_account, false),
        AccountMeta::new(pool_info.pool_base_token_account, false),
        AccountMeta::new(pool_info.pool_quote_token_account, false),
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

/// Build a pump-amm `claim_cashback` instruction — pays the caller their
/// accrued cashback from the `user_volume_accumulator` PDA, denominated in
/// the given `quote_mint` (typically WSOL).
///
/// The caller's `user_volume_accumulator` and its quote-mint ATA must
/// already exist — they're created lazily by the `buy` / `buy_exact_quote_in`
/// flow when `track_volume = true`.
pub fn make_claim_cashback_instruction(
    user: &Pubkey,
    quote_mint: &Pubkey,
    quote_token_program: &Pubkey,
) -> Result<Instruction> {
    let user_volume_accumulator = find_user_vol_accumulator(user);
    let user_volume_accumulator_quote_ta =
        spl_associated_token_account::get_associated_token_address_with_program_id(
            &user_volume_accumulator,
            quote_mint,
            quote_token_program,
        );
    let user_quote_ta = spl_associated_token_account::get_associated_token_address_with_program_id(
        user,
        quote_mint,
        quote_token_program,
    );

    // Per IDL, `user` is mut but NOT marked signer — the program looks the
    // user up via PDA seeds, so anyone can trigger a cashback claim that
    // pays out to that user's wsol ATA.
    let accounts = vec![
        AccountMeta::new(*user, false),
        AccountMeta::new(user_volume_accumulator, false),
        AccountMeta::new_readonly(*quote_mint, false),
        AccountMeta::new_readonly(*quote_token_program, false),
        AccountMeta::new(user_volume_accumulator_quote_ta, false),
        AccountMeta::new(user_quote_ta, false),
        AccountMeta::new_readonly(system_program::ID, false),
        AccountMeta::new_readonly(EVENT_AUTHORITY, false),
        AccountMeta::new_readonly(PUMP_SWAP_PROGRAM_ID, false),
    ];

    Ok(Instruction {
        program_id: PUMP_SWAP_PROGRAM_ID,
        accounts,
        data: ClaimCashbackInstruction.to_vec(),
    })
}

/// Build a pump-amm `extend_account` instruction. The official SDK prepends
/// this before swap / liquidity instructions when an older pool account is
/// smaller than the current pool allocation of
/// [`POOL_ACCOUNT_NEW_SIZE`](crate::POOL_ACCOUNT_NEW_SIZE) bytes.
pub fn make_extend_account_instruction(account: &Pubkey, user: &Pubkey) -> Result<Instruction> {
    let accounts = vec![
        AccountMeta::new(*account, false),
        AccountMeta::new_readonly(*user, true),
        AccountMeta::new_readonly(system_program::ID, false),
        AccountMeta::new_readonly(EVENT_AUTHORITY, false),
        AccountMeta::new_readonly(PUMP_SWAP_PROGRAM_ID, false),
    ];

    Ok(Instruction {
        program_id: PUMP_SWAP_PROGRAM_ID,
        accounts,
        data: vec![234, 102, 194, 203, 150, 72, 62, 229],
    })
}

/// Build a pump-amm `init_user_volume_accumulator` instruction for a user.
pub fn make_init_user_volume_accumulator_instruction(
    payer: &Pubkey,
    user: &Pubkey,
) -> Result<Instruction> {
    let accounts = vec![
        AccountMeta::new(*payer, true),
        AccountMeta::new_readonly(*user, false),
        AccountMeta::new(find_user_vol_accumulator(user), false),
        AccountMeta::new_readonly(system_program::ID, false),
        AccountMeta::new_readonly(EVENT_AUTHORITY, false),
        AccountMeta::new_readonly(PUMP_SWAP_PROGRAM_ID, false),
    ];

    Ok(Instruction {
        program_id: PUMP_SWAP_PROGRAM_ID,
        accounts,
        data: vec![94, 6, 202, 115, 255, 96, 232, 183],
    })
}

/// Build a pump-amm `sync_user_volume_accumulator` instruction. This moves a
/// user's current-day volume into claimable incentive state when the global
/// incentive day advances.
pub fn make_sync_user_volume_accumulator_instruction(user: &Pubkey) -> Result<Instruction> {
    let accounts = vec![
        AccountMeta::new_readonly(*user, false),
        AccountMeta::new_readonly(GLOBAL_VOLUME_ACCUMULATOR, false),
        AccountMeta::new(find_user_vol_accumulator(user), false),
        AccountMeta::new_readonly(EVENT_AUTHORITY, false),
        AccountMeta::new_readonly(PUMP_SWAP_PROGRAM_ID, false),
    ];

    Ok(Instruction {
        program_id: PUMP_SWAP_PROGRAM_ID,
        accounts,
        data: vec![86, 31, 192, 87, 163, 87, 79, 238],
    })
}

/// Build a pump-amm `close_user_volume_accumulator` instruction.
pub fn make_close_user_volume_accumulator_instruction(user: &Pubkey) -> Result<Instruction> {
    let accounts = vec![
        AccountMeta::new(*user, true),
        AccountMeta::new(find_user_vol_accumulator(user), false),
        AccountMeta::new_readonly(EVENT_AUTHORITY, false),
        AccountMeta::new_readonly(PUMP_SWAP_PROGRAM_ID, false),
    ];

    Ok(Instruction {
        program_id: PUMP_SWAP_PROGRAM_ID,
        accounts,
        data: vec![249, 69, 164, 218, 150, 103, 84, 138],
    })
}

/// Build a pump-amm `claim_token_incentives` instruction.
///
/// `user_token_account` is the user's ATA for `mint`; `global_incentive_token_account`
/// is the ATA for `(GLOBAL_VOLUME_ACCUMULATOR, mint, token_program)`.
pub fn make_claim_token_incentives_instruction(
    user: &Pubkey,
    payer: &Pubkey,
    mint: &Pubkey,
    token_program: &Pubkey,
    user_token_account: &Pubkey,
    global_incentive_token_account: &Pubkey,
) -> Result<Instruction> {
    let accounts = vec![
        AccountMeta::new_readonly(*user, false),
        AccountMeta::new(*user_token_account, false),
        AccountMeta::new_readonly(GLOBAL_VOLUME_ACCUMULATOR, false),
        AccountMeta::new(*global_incentive_token_account, false),
        AccountMeta::new(find_user_vol_accumulator(user), false),
        AccountMeta::new_readonly(*mint, false),
        AccountMeta::new_readonly(*token_program, false),
        AccountMeta::new_readonly(system_program::ID, false),
        AccountMeta::new_readonly(spl_associated_token_account::ID, false),
        AccountMeta::new_readonly(EVENT_AUTHORITY, false),
        AccountMeta::new_readonly(PUMP_SWAP_PROGRAM_ID, false),
        AccountMeta::new(*payer, true),
    ];

    Ok(Instruction {
        program_id: PUMP_SWAP_PROGRAM_ID,
        accounts,
        data: vec![16, 4, 71, 28, 204, 1, 40, 27],
    })
}

/// Build a pump-amm `collect_coin_creator_fee` instruction. This is the
/// current direct collect path from the coin-creator vault ATA into the coin
/// creator's quote token account.
pub fn make_collect_coin_creator_fee_instruction(
    coin_creator: &Pubkey,
    quote_mint: &Pubkey,
    quote_token_program: &Pubkey,
    coin_creator_token_account: &Pubkey,
) -> Result<Instruction> {
    let coin_creator_vault_authority = find_coin_creator_vault_authority(coin_creator);
    let coin_creator_vault_ata = find_coin_creator_vault_ata(
        &coin_creator_vault_authority,
        quote_token_program,
        quote_mint,
    );
    let accounts = vec![
        AccountMeta::new_readonly(*quote_mint, false),
        AccountMeta::new_readonly(*quote_token_program, false),
        AccountMeta::new_readonly(*coin_creator, false),
        AccountMeta::new_readonly(coin_creator_vault_authority, false),
        AccountMeta::new(coin_creator_vault_ata, false),
        AccountMeta::new(*coin_creator_token_account, false),
        AccountMeta::new_readonly(EVENT_AUTHORITY, false),
        AccountMeta::new_readonly(PUMP_SWAP_PROGRAM_ID, false),
    ];

    Ok(Instruction {
        program_id: PUMP_SWAP_PROGRAM_ID,
        accounts,
        data: vec![160, 57, 89, 42, 181, 139, 43, 66],
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
    create_pool_instruction_with_options(
        0,
        base_amount_in,
        quote_amount_in,
        false,
        false,
        pool,
        creator,
        coin_creator,
        base_mint,
        quote_mint,
        user_base_token_account,
        user_quote_token_account,
        pool_base_token_account,
        pool_quote_token_account,
        &spl_token::ID,
        &spl_token::ID,
    )
}

/// Build a pump-amm `create_pool` instruction with the current IDL args:
/// `index`, `coin_creator`, `is_mayhem_mode`, and `is_cashback_coin`.
#[allow(clippy::too_many_arguments)]
pub fn create_pool_instruction_with_options(
    index: u16,
    base_amount_in: u64,
    quote_amount_in: u64,
    is_mayhem_mode: bool,
    is_cashback_coin: bool,
    pool: &Pubkey,
    creator: &Pubkey,
    coin_creator: &Pubkey,
    base_mint: &Pubkey,
    quote_mint: &Pubkey,
    user_base_token_account: &Pubkey,
    user_quote_token_account: &Pubkey,
    pool_base_token_account: &Pubkey,
    pool_quote_token_account: &Pubkey,
    base_token_program: &Pubkey,
    quote_token_program: &Pubkey,
) -> Result<Instruction> {
    let data = CreatePoolInstruction::new_with_options(
        index,
        base_amount_in,
        quote_amount_in,
        *coin_creator,
        is_mayhem_mode,
        is_cashback_coin,
    )
    .to_vec();

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
        AccountMeta::new_readonly(*base_token_program, false),
        AccountMeta::new_readonly(*quote_token_program, false),
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
            pool_account_data_len: crate::constants::POOL_ACCOUNT_NEW_SIZE,
            base_mint: pk(2),
            quote_mint: WRAPPED_SOL_MINT,
            lp_mint: pk(3),
            pool_base_token_account: pk(4),
            pool_quote_token_account: pk(5),
            creator: pk(6),
            coin_creator,
            is_mayhem_mode: false,
            is_cashback_coin,
            virtual_quote_reserves: 0,
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
            make_buy_instruction(1, 2, true, &classic, &user, &user_base_ata, &user_quote_ata)
                .unwrap()
                .accounts
                .len(),
            25
        );
        assert_eq!(
            make_buy_exact_quote_in_instruction(
                1,
                2,
                true,
                &classic,
                &user,
                &user_base_ata,
                &user_quote_ata,
            )
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
            make_buy_instruction(
                1,
                2,
                true,
                &creator_pool,
                &user,
                &user_base_ata,
                &user_quote_ata
            )
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
                true,
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

    #[test]
    fn buy_instruction_data_layout_matches_idl() {
        let bytes = BuyInstruction::new(50_000_000, 564_953_706, true).to_vec();
        assert_eq!(bytes.len(), 25, "data must be exactly 25 bytes");
        assert_eq!(&bytes[0..8], &BuyInstruction::DISCRIMINATOR);
        assert_eq!(
            u64::from_le_bytes(bytes[8..16].try_into().unwrap()),
            50_000_000
        );
        assert_eq!(
            u64::from_le_bytes(bytes[16..24].try_into().unwrap()),
            564_953_706
        );
        assert_eq!(bytes[24], 1);

        let bytes_off = BuyInstruction::new(1, 2, false).to_vec();
        assert_eq!(bytes_off[24], 0);
    }

    #[test]
    fn buy_exact_quote_in_data_layout_matches_idl() {
        // Reproduces the exact on-chain instruction data observed in tx
        // 5EogpJNF...WoqH (spend 0.05 SOL, min 564_953_706 base out, track on).
        let bytes = BuyExactQuoteInInstruction::new(50_000_000, 564_953_706, true).to_vec();
        assert_eq!(bytes.len(), 25);
        assert_eq!(&bytes[0..8], &BuyExactQuoteInInstruction::DISCRIMINATOR);
        assert_eq!(
            u64::from_le_bytes(bytes[8..16].try_into().unwrap()),
            50_000_000
        );
        assert_eq!(
            u64::from_le_bytes(bytes[16..24].try_into().unwrap()),
            564_953_706
        );
        assert_eq!(bytes[24], 1);
    }

    #[test]
    fn deposit_instruction_account_count() {
        let pool = pool_info(Pubkey::default(), false);
        let user = pk(7);
        assert_eq!(
            make_deposit_instruction(1, 2, 3, &pool, &user, &pk(8), &pk(9), &pk(10))
                .unwrap()
                .accounts
                .len(),
            15
        );
    }

    #[test]
    fn claim_cashback_instruction_account_count() {
        let user = pk(7);
        assert_eq!(
            make_claim_cashback_instruction(&user, &WRAPPED_SOL_MINT, &spl_token::ID)
                .unwrap()
                .accounts
                .len(),
            9
        );
    }

    #[test]
    fn create_pool_instruction_data_layout_includes_current_flags() {
        let ix = CreatePoolInstruction::new_with_options(7, 1, 2, pk(3), true, false).to_vec();

        assert_eq!(ix.len(), 60);
        assert_eq!(&ix[0..8], &[233, 146, 209, 142, 207, 104, 64, 188]);
        assert_eq!(u16::from_le_bytes(ix[8..10].try_into().unwrap()), 7);
        assert_eq!(u64::from_le_bytes(ix[10..18].try_into().unwrap()), 1);
        assert_eq!(u64::from_le_bytes(ix[18..26].try_into().unwrap()), 2);
        assert_eq!(&ix[26..58], pk(3).as_ref());
        assert_eq!(ix[58], 1);
        assert_eq!(ix[59], 0);
    }

    #[test]
    fn current_auxiliary_instruction_builders_match_idl_account_counts() {
        let user = pk(7);
        let payer = pk(8);
        let mint = WRAPPED_SOL_MINT;
        let token_program = spl_token::ID;
        let user_ata = spl_associated_token_account::get_associated_token_address(&user, &mint);
        let global_ata = spl_associated_token_account::get_associated_token_address(
            &GLOBAL_VOLUME_ACCUMULATOR,
            &mint,
        );

        let extend_ix = make_extend_account_instruction(&pk(1), &user).unwrap();
        assert_eq!(extend_ix.data, vec![234, 102, 194, 203, 150, 72, 62, 229]);
        assert_eq!(extend_ix.accounts.len(), 5);

        assert_eq!(
            make_init_user_volume_accumulator_instruction(&payer, &user)
                .unwrap()
                .accounts
                .len(),
            6
        );
        assert_eq!(
            make_sync_user_volume_accumulator_instruction(&user)
                .unwrap()
                .accounts
                .len(),
            5
        );
        assert_eq!(
            make_close_user_volume_accumulator_instruction(&user)
                .unwrap()
                .accounts
                .len(),
            4
        );
        assert_eq!(
            make_claim_token_incentives_instruction(
                &user,
                &payer,
                &mint,
                &token_program,
                &user_ata,
                &global_ata,
            )
            .unwrap()
            .accounts
            .len(),
            12
        );
        assert_eq!(
            make_collect_coin_creator_fee_instruction(&user, &mint, &token_program, &user_ata,)
                .unwrap()
                .accounts
                .len(),
            8
        );
    }
}
