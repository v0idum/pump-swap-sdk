use bytemuck::{Pod, Zeroable};
use serde::Serialize;
use solana_sdk::pubkey::Pubkey;

#[repr(C, packed)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Pod, Zeroable, Serialize)]
pub struct Pool {
    pub pool_bump: u8,
    pub index: u16,
    pub creator: Pubkey,
    pub base_mint: Pubkey,
    pub quote_mint: Pubkey,
    pub lp_mint: Pubkey,
    pub pool_base_token_account: Pubkey,
    pub pool_quote_token_account: Pubkey,
    pub lp_supply: u64,
    pub coin_creator: Pubkey,
    pub is_mayhem_mode: u8,
    pub is_cashback_coin: u8,
}

#[derive(Debug, Clone, Serialize)]
pub struct PoolInfo {
    pub pool: Pubkey,
    /// Raw on-chain pool account data length, including the Anchor discriminator.
    pub pool_account_data_len: usize,
    pub base_mint: Pubkey,
    pub quote_mint: Pubkey,
    pub lp_mint: Pubkey,
    pub pool_base_token_account: Pubkey,
    pub pool_quote_token_account: Pubkey,
    pub creator: Pubkey,
    pub coin_creator: Pubkey,
    pub is_mayhem_mode: bool,
    pub is_cashback_coin: bool,
    /// SPL Token program that owns the base mint (`spl_token::ID` or `spl_token_2022::ID`).
    pub base_token_program: Pubkey,
    /// SPL Token program that owns the quote mint (`spl_token::ID` or `spl_token_2022::ID`).
    pub quote_token_program: Pubkey,
}
