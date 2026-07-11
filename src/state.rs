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

/// Which side of the pair is the traded token (the non-SOL asset).
///
/// pump.fun graduations currently deploy pools with **SOL as base** (~85% of
/// live volume); direct launches are typically token-base. The pump-amm
/// `buy`/`sell` instructions are defined over base/quote, so trader intent
/// ("buy the token") maps to a different instruction per orientation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TokenSide {
    /// base = token, quote = WSOL (canonical / direct-launch style).
    Base,
    /// base = WSOL, quote = token (current pump.fun-graduation style).
    Quote,
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

impl PoolInfo {
    /// True when the pool stores WSOL on the base side (pump.fun-graduation
    /// orientation). See [`TokenSide`].
    pub fn sol_is_base(&self) -> bool {
        self.base_mint == crate::constants::WRAPPED_SOL_MINT
    }

    /// Which side of the pair holds the traded (non-SOL) token.
    pub fn token_side(&self) -> TokenSide {
        if self.sol_is_base() {
            TokenSide::Quote
        } else {
            TokenSide::Base
        }
    }

    /// Mint of the traded (non-SOL) token.
    pub fn token_mint(&self) -> Pubkey {
        match self.token_side() {
            TokenSide::Base => self.base_mint,
            TokenSide::Quote => self.quote_mint,
        }
    }

    /// SPL Token program owning the traded token's mint.
    pub fn token_program(&self) -> Pubkey {
        match self.token_side() {
            TokenSide::Base => self.base_token_program,
            TokenSide::Quote => self.quote_token_program,
        }
    }

    /// Pool reserves as `(sol_reserve, token_reserve)` given raw
    /// `(base_reserve, quote_reserve)` amounts.
    pub fn orient_reserves(&self, base_reserve: u64, quote_reserve: u64) -> (u64, u64) {
        match self.token_side() {
            TokenSide::Base => (quote_reserve, base_reserve),
            TokenSide::Quote => (base_reserve, quote_reserve),
        }
    }
}
