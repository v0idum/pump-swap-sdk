use anyhow::{Result, anyhow};
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

/// Anchor account discriminator for pump-amm's `GlobalConfig`.
const GLOBAL_CONFIG_DISCRIMINATOR: [u8; 8] = [0x95, 0x08, 0x9c, 0xca, 0xa0, 0xfc, 0xb0, 0xd9];

/// Anchor account discriminator for the fee program's `FeeConfig`.
const FEE_CONFIG_DISCRIMINATOR: [u8; 8] = [0x8f, 0x34, 0x92, 0xbb, 0xdb, 0x7b, 0x4c, 0x9b];

/// Sequential borsh reader over raw account bytes.
///
/// Every read is bounds-checked, so a truncated or unexpected account fails
/// with an error instead of decoding into plausible-looking garbage.
struct Reader<'a> {
    data: &'a [u8],
    pos: usize,
}

impl<'a> Reader<'a> {
    /// Opens `data` past an 8-byte Anchor discriminator, rejecting anything
    /// that isn't `expected`.
    fn after_discriminator(data: &'a [u8], expected: &[u8; 8], account: &str) -> Result<Self> {
        let found = data
            .get(..8)
            .ok_or_else(|| anyhow!("{account}: account data too short for a discriminator"))?;
        if found != expected {
            anyhow::bail!(
                "{account}: unexpected account discriminator {found:02x?}, expected {expected:02x?}"
            );
        }
        Ok(Self { data, pos: 8 })
    }

    fn take(&mut self, len: usize) -> Result<&'a [u8]> {
        let end = self
            .pos
            .checked_add(len)
            .ok_or_else(|| anyhow!("account data offset overflow"))?;
        let slice = self.data.get(self.pos..end).ok_or_else(|| {
            anyhow!(
                "account data truncated: need {end} bytes, got {}",
                self.data.len()
            )
        })?;
        self.pos = end;
        Ok(slice)
    }

    fn u8(&mut self) -> Result<u8> {
        Ok(self.take(1)?[0])
    }

    fn bool(&mut self) -> Result<bool> {
        match self.u8()? {
            0 => Ok(false),
            1 => Ok(true),
            other => Err(anyhow!("invalid borsh bool byte {other}")),
        }
    }

    fn u32(&mut self) -> Result<u32> {
        Ok(u32::from_le_bytes(self.take(4)?.try_into()?))
    }

    fn u64(&mut self) -> Result<u64> {
        Ok(u64::from_le_bytes(self.take(8)?.try_into()?))
    }

    fn u128(&mut self) -> Result<u128> {
        Ok(u128::from_le_bytes(self.take(16)?.try_into()?))
    }

    fn pubkey(&mut self) -> Result<Pubkey> {
        Ok(Pubkey::new_from_array(self.take(32)?.try_into()?))
    }

    fn pubkey_array<const N: usize>(&mut self) -> Result<[Pubkey; N]> {
        let mut out = [Pubkey::default(); N];
        for slot in out.iter_mut() {
            *slot = self.pubkey()?;
        }
        Ok(out)
    }

    /// Reads a borsh `Vec<T>`: a `u32` length prefix followed by that many
    /// elements. The length is sanity-checked against the remaining bytes so a
    /// corrupt prefix can't trigger a huge allocation.
    fn vec<T>(
        &mut self,
        min_element_len: usize,
        mut read: impl FnMut(&mut Self) -> Result<T>,
    ) -> Result<Vec<T>> {
        let len = self.u32()? as usize;
        let remaining = self.data.len().saturating_sub(self.pos);
        if len.saturating_mul(min_element_len) > remaining {
            anyhow::bail!("vector length {len} exceeds {remaining} remaining account bytes");
        }
        (0..len).map(|_| read(self)).collect()
    }
}

/// Fee split applied to a swap, in basis points of the quote amount.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
pub struct Fees {
    pub lp_fee_bps: u64,
    pub protocol_fee_bps: u64,
    pub creator_fee_bps: u64,
}

impl Fees {
    /// Total fee charged on a swap, in basis points.
    ///
    /// Saturates instead of wrapping; the live values are well under 10_000.
    pub fn total_bps(&self) -> u64 {
        self.lp_fee_bps
            .saturating_add(self.protocol_fee_bps)
            .saturating_add(self.creator_fee_bps)
    }

    fn read(reader: &mut Reader<'_>) -> Result<Self> {
        Ok(Self {
            lp_fee_bps: reader.u64()?,
            protocol_fee_bps: reader.u64()?,
            creator_fee_bps: reader.u64()?,
        })
    }
}

/// One rung of the market-cap fee ladder: the fees that apply once a pool's
/// market cap reaches `market_cap_lamports_threshold`.
///
/// Note the threshold is `u128`, matching the on-chain IDL. Decoding it as a
/// `u64` yields plausible-looking values rather than an error.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
pub struct FeeTier {
    pub market_cap_lamports_threshold: u128,
    pub fees: Fees,
}

impl FeeTier {
    fn read(reader: &mut Reader<'_>) -> Result<Self> {
        Ok(Self {
            market_cap_lamports_threshold: reader.u128()?,
            fees: Fees::read(reader)?,
        })
    }

    /// Minimum encoded size of a `FeeTier` (u128 threshold + three u64 fees).
    const ENCODED_LEN: usize = 16 + 24;
}

/// The fee program's `FeeConfig` account, owned by
/// [`FEE_PROGRAM`](crate::constants::FEE_PROGRAM) and stored at
/// [`fee_config_pda`](crate::util::fee_config_pda).
///
/// The on-chain `get_fees` instruction picks between `flat_fees`, `fee_tiers`
/// and `stable_fee_tiers` from the pool's market cap, trade size and mint;
/// this type exposes the decoded ladders so callers can price a swap without
/// a CPI. See [`FeeConfig::fee_tier_for_market_cap`].
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct FeeConfig {
    pub bump: u8,
    pub admin: Pubkey,
    /// Fees charged when no market-cap tier applies.
    pub flat_fees: Fees,
    /// Market-cap ladder for standard pools, ascending by threshold.
    pub fee_tiers: Vec<FeeTier>,
    /// Market-cap ladder for stable pools, ascending by threshold.
    pub stable_fee_tiers: Vec<FeeTier>,
}

impl FeeConfig {
    /// Decode a `FeeConfig` from raw account data, discriminator included.
    ///
    /// Trailing bytes past the encoded layout are ignored: the live account is
    /// over-allocated to leave room for more fee tiers.
    pub fn from_account_data(data: &[u8]) -> Result<Self> {
        let mut reader = Reader::after_discriminator(data, &FEE_CONFIG_DISCRIMINATOR, "FeeConfig")?;
        Ok(Self {
            bump: reader.u8()?,
            admin: reader.pubkey()?,
            flat_fees: Fees::read(&mut reader)?,
            fee_tiers: reader.vec(FeeTier::ENCODED_LEN, FeeTier::read)?,
            stable_fee_tiers: reader.vec(FeeTier::ENCODED_LEN, FeeTier::read)?,
        })
    }

    /// The standard-pool tier that applies at `market_cap_lamports`: the
    /// highest tier whose threshold is at or below it.
    ///
    /// Returns `None` only when the ladder is empty or every threshold sits
    /// above `market_cap_lamports`; callers should fall back to
    /// [`FeeConfig::flat_fees`].
    pub fn fee_tier_for_market_cap(&self, market_cap_lamports: u128) -> Option<&FeeTier> {
        Self::tier_for_market_cap(&self.fee_tiers, market_cap_lamports)
    }

    /// Same as [`FeeConfig::fee_tier_for_market_cap`], against the stable-pool
    /// ladder.
    pub fn stable_fee_tier_for_market_cap(&self, market_cap_lamports: u128) -> Option<&FeeTier> {
        Self::tier_for_market_cap(&self.stable_fee_tiers, market_cap_lamports)
    }

    fn tier_for_market_cap(tiers: &[FeeTier], market_cap_lamports: u128) -> Option<&FeeTier> {
        tiers
            .iter()
            .rev()
            .find(|tier| tier.market_cap_lamports_threshold <= market_cap_lamports)
    }
}

/// pump-amm's `GlobalConfig` account, stored at
/// [`GLOBAL_CONFIG`](crate::constants::GLOBAL_CONFIG).
///
/// Field order matches the live IDL. Note the asymmetry the SDK's
/// [`RESERVED_FEE_RECIPIENTS`](crate::constants::RESERVED_FEE_RECIPIENTS)
/// constant flattens away: on chain there is one `reserved_fee_recipient`
/// plus an array of seven more. Use
/// [`GlobalConfig::all_reserved_fee_recipients`] for the flattened view.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub struct GlobalConfig {
    pub admin: Pubkey,
    pub lp_fee_basis_points: u64,
    pub protocol_fee_basis_points: u64,
    pub disable_flags: u8,
    pub protocol_fee_recipients: [Pubkey; 8],
    pub coin_creator_fee_basis_points: u64,
    pub admin_set_coin_creator_authority: Pubkey,
    pub whitelist_pda: Pubkey,
    /// First reserved (Mayhem-mode) fee recipient; see
    /// [`GlobalConfig::all_reserved_fee_recipients`].
    pub reserved_fee_recipient: Pubkey,
    pub mayhem_mode_enabled: bool,
    /// The remaining seven reserved fee recipients.
    pub reserved_fee_recipients: [Pubkey; 7],
    pub is_cashback_enabled: bool,
    pub buyback_fee_recipients: [Pubkey; 8],
    pub buyback_basis_points: u64,
}

impl GlobalConfig {
    /// Decode a `GlobalConfig` from raw account data, discriminator included.
    ///
    /// Trailing bytes past the encoded layout are ignored: the live account is
    /// 949 bytes against a 907-byte layout.
    pub fn from_account_data(data: &[u8]) -> Result<Self> {
        let mut reader =
            Reader::after_discriminator(data, &GLOBAL_CONFIG_DISCRIMINATOR, "GlobalConfig")?;
        Ok(Self {
            admin: reader.pubkey()?,
            lp_fee_basis_points: reader.u64()?,
            protocol_fee_basis_points: reader.u64()?,
            disable_flags: reader.u8()?,
            protocol_fee_recipients: reader.pubkey_array()?,
            coin_creator_fee_basis_points: reader.u64()?,
            admin_set_coin_creator_authority: reader.pubkey()?,
            whitelist_pda: reader.pubkey()?,
            reserved_fee_recipient: reader.pubkey()?,
            mayhem_mode_enabled: reader.bool()?,
            reserved_fee_recipients: reader.pubkey_array()?,
            is_cashback_enabled: reader.bool()?,
            buyback_fee_recipients: reader.pubkey_array()?,
            buyback_basis_points: reader.u64()?,
        })
    }

    /// `reserved_fee_recipient` followed by `reserved_fee_recipients`, in the
    /// same order as
    /// [`RESERVED_FEE_RECIPIENTS`](crate::constants::RESERVED_FEE_RECIPIENTS).
    pub fn all_reserved_fee_recipients(&self) -> [Pubkey; 8] {
        let mut out = [self.reserved_fee_recipient; 8];
        out[1..].copy_from_slice(&self.reserved_fee_recipients);
        out
    }
}
