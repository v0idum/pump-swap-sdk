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
    /// Quote-side liquidity the program prices against but the pool's quote
    /// token account does not hold.
    ///
    /// A `u64` stored immediately after `is_cashback_coin`, present in the
    /// live account layout but **absent from the published IDL**, so
    /// [`Pool`] does not carry it. pump-amm adds it to the quote token
    /// account balance both when stepping the constant product and when
    /// computing the pool's market cap for fee-tier selection; ignoring it
    /// misprices affected pools by orders of magnitude. Zero for pools that
    /// predate the field or never accrued any.
    ///
    /// See [`PoolInfo::effective_reserves`].
    pub virtual_quote_reserves: u64,
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
    ///
    /// These are the raw token-account balances. To price a swap, use
    /// [`Self::effective_reserves`] first — the program prices against more
    /// quote than the token account holds whenever
    /// [`Self::virtual_quote_reserves`] is non-zero.
    pub fn orient_reserves(&self, base_reserve: u64, quote_reserve: u64) -> (u64, u64) {
        match self.token_side() {
            TokenSide::Base => (quote_reserve, base_reserve),
            TokenSide::Quote => (base_reserve, quote_reserve),
        }
    }

    /// Decode a pool account into a [`PoolInfo`].
    ///
    /// `data` is the raw account, discriminator included. The token programs
    /// are supplied by the caller because they live on the mint accounts, not
    /// on the pool; [`load_pool`](crate::util::load_pool) reads them for you.
    ///
    /// Accounts longer than the struct are accepted — the live allocation is
    /// over-sized — and accounts shorter than the current layout decode with
    /// [`Self::virtual_quote_reserves`] at zero, which is what the program
    /// does for pools that predate the field.
    pub fn from_account_data(
        pool: Pubkey,
        data: &[u8],
        base_token_program: Pubkey,
        quote_token_program: Pubkey,
    ) -> Result<Self> {
        let pool_size = size_of::<Pool>();
        if data.len() < pool_size + 8 {
            anyhow::bail!(
                "Pool account too short: expected at least {}, got {}",
                pool_size + 8,
                data.len()
            );
        }
        let pool_data = *bytemuck::from_bytes::<Pool>(&data[8..pool_size + 8]);

        // A `u64` the live layout carries immediately after
        // `is_cashback_coin`, absent from the published IDL and from older,
        // shorter accounts. See [`Self::virtual_quote_reserves`].
        let virtual_quote_reserves = data
            .get(pool_size + 8..pool_size + 16)
            .and_then(|bytes| <[u8; 8]>::try_from(bytes).ok())
            .map(u64::from_le_bytes)
            .unwrap_or(0);

        Ok(Self {
            pool,
            pool_account_data_len: data.len(),
            base_mint: pool_data.base_mint,
            quote_mint: pool_data.quote_mint,
            lp_mint: pool_data.lp_mint,
            pool_base_token_account: pool_data.pool_base_token_account,
            pool_quote_token_account: pool_data.pool_quote_token_account,
            creator: pool_data.creator,
            coin_creator: pool_data.coin_creator,
            is_mayhem_mode: pool_data.is_mayhem_mode != 0,
            is_cashback_coin: pool_data.is_cashback_coin != 0,
            virtual_quote_reserves,
            base_token_program,
            quote_token_program,
        })
    }

    /// The `(base, quote)` reserves pump-amm actually prices against: raw
    /// token-account balances with [`Self::virtual_quote_reserves`] added to
    /// the quote side.
    pub fn effective_reserves(&self, base_reserve: u64, quote_reserve: u64) -> (u64, u64) {
        (
            base_reserve,
            quote_reserve.saturating_add(self.virtual_quote_reserves),
        )
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

/// Quote mints the fee program prices off `stable_fee_tiers`.
///
/// Undetermined, and deliberately left that way. Of 136 pools observed on
/// mainnet only three are ladder-priced without a SOL quote, and they do not
/// agree with each other: one lands exactly on the stable ladder, the other
/// two on neither ladder (one of them at 325 bps, past the top of both). The
/// membership rule is therefore not something a single live sample settles,
/// so [`crate::math::can_quote_fees`] rejects those pools outright instead of
/// this function guessing at them. It returns `false` so a caller reaching
/// [`FeeConfig::fees_for_pool`] directly gets the standard ladder, which at a
/// given market cap is the more expensive of the two — the safe direction for
/// a `min_out`.
fn is_stable_quote_mint(_quote_mint: &Pubkey) -> bool {
    false
}

/// `ceil(amount * bps / 10_000)` in u128, saturating back into a `u64`.
fn ceil_div_bps(amount: u64, bps: u64) -> u64 {
    let numerator = amount as u128 * bps as u128;
    u64::try_from(numerator.div_ceil(10_000)).unwrap_or(u64::MAX)
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

    /// The exact fee the program deducts from `amount`.
    ///
    /// **Not** `amount * total_bps / 10_000`: pump-amm rounds each component
    /// up separately, so a three-way split can charge up to two lamports more
    /// than a single rounded-up calculation on the combined rate.
    pub fn fee_on(&self, amount: u64) -> u64 {
        [self.lp_fee_bps, self.protocol_fee_bps, self.creator_fee_bps]
            .into_iter()
            .map(|bps| ceil_div_bps(amount, bps))
            .fold(0u64, u64::saturating_add)
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
/// pump-amm does not call the `get_fees` instruction in the published IDL. On
/// every swap it CPIs `GetFeesWithQuoteMint`, which takes
/// `(is_pump_pool: bool, market_cap_lamports: u128, quote_mint: Pubkey)` and
/// no trade size — confirmed by decoding the inner instruction of live swaps
/// (mainnet, 2026-09-19). The ladder choice is therefore:
///
/// * `is_pump_pool == false` (non-SOL-quoted pools) → [`Self::flat_fees`]
/// * otherwise → [`Self::fee_tiers`] indexed by market cap, or
///   [`Self::stable_fee_tiers`] for a stablecoin quote mint
///
/// This type exposes the decoded ladders so callers can price a swap without
/// a CPI. See [`FeeConfig::fees_for_pool`].
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
    /// above `market_cap_lamports`. Neither happens on the live account,
    /// whose base rung has a threshold of `0`.
    ///
    /// **Do not fall back to [`FeeConfig::flat_fees`] here.** `flat_fees`
    /// totals 30 bps and applies to pools the program flags as non-pump; the
    /// ladder's base rung is 125 bps. Treating "no tier matched" as `flat_fees`
    /// would under-charge a low-market-cap pool by 95 bps and produce a
    /// `min_out` the program rejects. An empty ladder means the decoded
    /// account is not what this SDK expects, so callers should surface that
    /// rather than guess — [`FeeConfig::fees_for_pool`] returns the base rung.
    pub fn fee_tier_for_market_cap(&self, market_cap_lamports: u128) -> Option<&FeeTier> {
        Self::tier_for_market_cap(&self.fee_tiers, market_cap_lamports)
    }

    /// Same as [`FeeConfig::fee_tier_for_market_cap`], against the stable-pool
    /// ladder.
    pub fn stable_fee_tier_for_market_cap(&self, market_cap_lamports: u128) -> Option<&FeeTier> {
        Self::tier_for_market_cap(&self.stable_fee_tiers, market_cap_lamports)
    }

    /// The fee schedule pump-amm's fee program returns for a pool, mirroring
    /// its `GetFeesWithQuoteMint` instruction.
    ///
    /// `is_tiered` is the program's `is_pump_pool` argument — see
    /// [`is_tiered_fee_pool`](crate::math::is_tiered_fee_pool). `quote_mint`
    /// selects between the standard and stable ladders.
    ///
    /// When the ladder is empty (which the live account never is) this falls
    /// back to the ladder's most expensive schedule rather than to
    /// [`Self::flat_fees`], so a decoding surprise cannot silently under-quote
    /// the fee.
    pub fn fees_for_pool(
        &self,
        is_tiered: bool,
        market_cap_lamports: u128,
        quote_mint: &Pubkey,
    ) -> Fees {
        if !is_tiered {
            return self.flat_fees;
        }
        let ladder = if is_stable_quote_mint(quote_mint) {
            &self.stable_fee_tiers
        } else {
            &self.fee_tiers
        };
        Self::tier_for_market_cap(ladder, market_cap_lamports)
            .or_else(|| ladder.first())
            .map(|tier| tier.fees)
            .unwrap_or(self.flat_fees)
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
