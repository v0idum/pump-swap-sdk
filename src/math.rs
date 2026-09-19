//! Swap pricing that reproduces pump-amm's own arithmetic.
//!
//! Every rule here was read off mainnet swap events rather than inferred:
//! see `tests/swap_math.rs`, which replays byte-for-byte `Program data:`
//! payloads through these functions.

use crate::constants::WRAPPED_SOL_MINT;
use crate::state::{Fees, PoolInfo};
use solana_sdk::pubkey::Pubkey;

/// Constant-product output with no fee accounting, reduced by `slippage`.
///
/// **This is not what the program pays out.** pump-amm charges a
/// market-cap-tiered fee (30–125 bps) on top of the curve, so a `min_out`
/// derived from this function is too optimistic by the fee and the swap
/// reverts with `ExceededSlippage` unless `slippage` is wide enough to absorb
/// it. Use [`quote_sell`], [`quote_buy_exact_quote_in`] or
/// [`quote_buy_exact_base_out`] to price a real swap; this function remains
/// for callers that want the raw curve.
pub fn calc_amount_out(amount_in: u64, reserve_in: u64, reserve_out: u64, slippage: f64) -> u64 {
    let amount_in = amount_in as f64;
    let reserve_in = reserve_in as f64;
    let reserve_out = reserve_out as f64;

    let product = reserve_in * reserve_out;
    let new_in_reserve = reserve_in + amount_in;
    let new_out_reserve = product / new_in_reserve + 1.0;
    let result = (reserve_out - new_out_reserve) * (1.0 - slippage);
    result.max(0.0).round() as u64
}

/// `floor(reserve_out * amount_in / (reserve_in + amount_in))` — the exact
/// integer constant-product step the program takes, in `u128` so a large pool
/// can't overflow the intermediate product.
pub fn constant_product_out(amount_in: u64, reserve_in: u64, reserve_out: u64) -> u64 {
    let numerator = reserve_out as u128 * amount_in as u128;
    let denominator = reserve_in as u128 + amount_in as u128;
    if denominator == 0 {
        return 0;
    }
    u64::try_from(numerator / denominator).unwrap_or(u64::MAX)
}

/// `ceil(reserve_in * amount_out / (reserve_out - amount_out))` — the input
/// the program requires for an exact output. Returns `None` when
/// `amount_out` would drain the reserve.
fn constant_product_in(amount_out: u64, reserve_in: u64, reserve_out: u64) -> Option<u64> {
    let remaining = reserve_out.checked_sub(amount_out)?;
    if remaining == 0 {
        return None;
    }
    let numerator = reserve_in as u128 * amount_out as u128;
    Some(u64::try_from(numerator.div_ceil(remaining as u128)).unwrap_or(u64::MAX))
}

fn apply_slippage(amount: u64, slippage: f64) -> u64 {
    let factor = (1.0 - slippage).clamp(0.0, 1.0);
    (amount as f64 * factor).floor().max(0.0) as u64
}

/// A swap priced the way the program prices it, for the directions that take
/// an exact input and pay out a minimum.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SwapQuote {
    /// Curve output before fees — the event's `quote_amount_out` on a sell.
    /// On an exact-quote-in buy this is the same as [`Self::amount_out`],
    /// because there the fee is taken off the input instead.
    pub gross_amount_out: u64,
    /// Total fee the program takes for this swap.
    pub fee: u64,
    /// Exactly what the program credits the caller, fees already deducted.
    pub amount_out: u64,
    /// [`Self::amount_out`] reduced by `slippage`. This is the value to pass
    /// as the instruction's `min_*_amount_out`.
    pub min_amount_out: u64,
}

/// A swap priced for the `buy` instruction, which takes an exact base output
/// and a spending cap.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct BuyExactOutQuote {
    /// Quote the curve consumes, before fees.
    pub quote_amount_in: u64,
    /// Total fee, charged on top of [`Self::quote_amount_in`].
    pub fee: u64,
    /// What the caller actually spends: curve input plus fee.
    pub total_quote_in: u64,
    /// [`Self::total_quote_in`] raised by `slippage`. Pass as
    /// `max_quote_amount_in`.
    pub max_quote_in: u64,
}

/// Price a pump-amm `sell`: exact `base_amount_in`, minimum quote out.
///
/// The program takes the curve output and subtracts each fee component,
/// rounded up individually:
///
/// ```text
/// gross = floor(quote_reserve * base_in / (base_reserve + base_in))
/// out   = gross - ceil(gross*lp/1e4) - ceil(gross*protocol/1e4) - ceil(gross*creator/1e4)
/// ```
///
/// `quote_reserve` must be the *effective* reserve — see
/// [`PoolInfo::effective_reserves`].
pub fn quote_sell(
    base_amount_in: u64,
    base_reserve: u64,
    quote_reserve: u64,
    fees: Fees,
    slippage: f64,
) -> SwapQuote {
    let gross = constant_product_out(base_amount_in, base_reserve, quote_reserve);
    let fee = fees.fee_on(gross);
    let amount_out = gross.saturating_sub(fee);
    SwapQuote {
        gross_amount_out: gross,
        fee,
        amount_out,
        min_amount_out: apply_slippage(amount_out, slippage),
    }
}

/// Price a pump-amm `buy_exact_quote_in`: exact quote spent, minimum base out.
///
/// The program divides the spend back out of the fee-inclusive rate to get the
/// amount it charges fees on, subtracts those fees from the spend, and feeds
/// the curve **one unit less** than the remainder:
///
/// ```text
/// charged = floor(quote_in * 10_000 / (10_000 + total_bps))
/// net     = quote_in - fee_on(charged)
/// out     = floor(base_reserve * (net - 1) / (quote_reserve + net - 1))
/// ```
///
/// The `net - 1` is the program's own rounding, not a safety margin; it is
/// reproduced here because a quote that is one curve step too high is exactly
/// what makes a zero-slippage swap revert.
pub fn quote_buy_exact_quote_in(
    quote_amount_in: u64,
    base_reserve: u64,
    quote_reserve: u64,
    fees: Fees,
    slippage: f64,
) -> SwapQuote {
    let denominator = 10_000u128 + fees.total_bps() as u128;
    let charged = u64::try_from(quote_amount_in as u128 * 10_000 / denominator).unwrap_or(u64::MAX);
    let fee = fees.fee_on(charged);
    let net = quote_amount_in.saturating_sub(fee);
    let curve_in = net.saturating_sub(1);
    let amount_out = constant_product_out(curve_in, quote_reserve, base_reserve);
    SwapQuote {
        gross_amount_out: amount_out,
        fee,
        amount_out,
        min_amount_out: apply_slippage(amount_out, slippage),
    }
}

/// Price a pump-amm `buy`: exact `base_amount_out`, capped quote in.
///
/// ```text
/// quote_in = ceil(quote_reserve * base_out / (base_reserve - base_out))
/// spent    = quote_in + fee_on(quote_in)
/// ```
///
/// Returns `None` when `base_amount_out` is at or beyond the pool's base
/// reserve, which the curve cannot price.
pub fn quote_buy_exact_base_out(
    base_amount_out: u64,
    base_reserve: u64,
    quote_reserve: u64,
    fees: Fees,
    slippage: f64,
) -> Option<BuyExactOutQuote> {
    let quote_amount_in = constant_product_in(base_amount_out, quote_reserve, base_reserve)?;
    let fee = fees.fee_on(quote_amount_in);
    let total_quote_in = quote_amount_in.saturating_add(fee);
    let factor = (1.0 + slippage.max(0.0)).min(u32::MAX as f64);
    let max_quote_in = (total_quote_in as f64 * factor).ceil() as u64;
    Some(BuyExactOutQuote {
        quote_amount_in,
        fee,
        total_quote_in,
        max_quote_in,
    })
}

/// Quote "spend exactly `sol_in` lamports, receive tokens", matching
/// [`build_token_buy_ixs`](crate::client::PumpSwapClient::build_token_buy_ixs)
/// on either pool orientation.
///
/// `reserves` are the raw `(base_reserve, quote_reserve)` token-account
/// balances; the pool's virtual quote liquidity is added internally.
pub fn token_buy_quote(
    sol_in: u64,
    reserves: (u64, u64),
    pool: &PoolInfo,
    fees: Fees,
    slippage: f64,
) -> SwapQuote {
    let (base_reserve, quote_reserve) = pool.effective_reserves(reserves.0, reserves.1);
    if pool.sol_is_base() {
        // SOL is the base side: buying the token is a `sell` of base.
        quote_sell(sol_in, base_reserve, quote_reserve, fees, slippage)
    } else {
        quote_buy_exact_quote_in(sol_in, base_reserve, quote_reserve, fees, slippage)
    }
}

/// Quote "spend exactly `tokens_in` base units, receive lamports", matching
/// [`build_token_sell_ixs`](crate::client::PumpSwapClient::build_token_sell_ixs)
/// on either pool orientation.
pub fn token_sell_quote(
    tokens_in: u64,
    reserves: (u64, u64),
    pool: &PoolInfo,
    fees: Fees,
    slippage: f64,
) -> SwapQuote {
    let (base_reserve, quote_reserve) = pool.effective_reserves(reserves.0, reserves.1);
    if pool.sol_is_base() {
        // SOL is the base side: selling the token buys base with exact quote in.
        quote_buy_exact_quote_in(tokens_in, base_reserve, quote_reserve, fees, slippage)
    } else {
        quote_sell(tokens_in, base_reserve, quote_reserve, fees, slippage)
    }
}

/// Fee-aware replacement for the old pure-curve helper: tokens received for
/// `amount_in` lamports of SOL, minus fees and slippage.
///
/// Mirrors [`token_buy_quote`] and returns only the `min_out` value.
pub fn buy_amount_out(
    amount_in: u64,
    reserves: (u64, u64),
    pool: &PoolInfo,
    fees: Fees,
    slippage: f64,
) -> u64 {
    token_buy_quote(amount_in, reserves, pool, fees, slippage).min_amount_out
}

/// Fee-aware replacement for the old pure-curve helper: lamports received for
/// `amount_in` base units of the token, minus fees and slippage.
pub fn sell_amount_out(
    amount_in: u64,
    reserves: (u64, u64),
    pool: &PoolInfo,
    fees: Fees,
    slippage: f64,
) -> u64 {
    token_sell_quote(amount_in, reserves, pool, fees, slippage).min_amount_out
}

/// The pool's market cap in lamports, the way pump-amm computes it before
/// asking the fee program which tier applies.
///
/// ```text
/// market_cap = base_mint_supply * effective_quote_reserve / base_reserve
/// ```
///
/// Only meaningful for pools quoted in SOL; the program passes `0` for the
/// rest. Reserves must already include the pool's virtual quote liquidity.
pub fn market_cap_lamports(base_supply: u64, base_reserve: u64, quote_reserve: u64) -> u128 {
    if base_reserve == 0 {
        return 0;
    }
    base_supply as u128 * quote_reserve as u128 / base_reserve as u128
}

/// Whether the fee program prices this pool off the market-cap ladder rather
/// than [`FeeConfig::flat_fees`](crate::state::FeeConfig::flat_fees).
///
/// pump-amm passes this as `is_pump_pool` to the fee program's
/// `GetFeesWithQuoteMint`. It tracks the pool's coin creator: across 614
/// mainnet swaps over 136 pools (2026-09-19), `coin_creator != Pubkey::default()`
/// agreed with the flag on every swap. That also matches the program's own
/// `set_coin_creator`, which will only write the field for a mint that has a
/// pump.fun bonding curve.
///
/// A tempting alternative, "the pool is SOL-quoted", is wrong: three of those
/// pools are quoted in another mint and are still priced off the ladder.
pub fn is_tiered_fee_pool(pool: &PoolInfo) -> bool {
    pool.coin_creator != Pubkey::default()
}

/// Whether this SDK can reproduce the fee the program will charge.
///
/// Returns `false` for a [tiered](is_tiered_fee_pool) pool quoted in something
/// other than SOL. Those exist — roughly 2% of the pools observed — and the
/// fee program prices them off neither published ladder: live swaps show
/// schedules from 30 bps up to 325 bps, against a ladder that tops out at 125.
/// The SDK does not guess at them, because guessing low is what produces a
/// `min_out` the program rejects.
///
/// Such a pool is outside this SDK's trading API anyway: `sol_in` /
/// `min_sol_out` assume one side of the pair is WSOL.
pub fn can_quote_fees(pool: &PoolInfo) -> bool {
    !is_tiered_fee_pool(pool) || pool.quote_mint == WRAPPED_SOL_MINT
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fees(lp: u64, protocol: u64, creator: u64) -> Fees {
        Fees {
            lp_fee_bps: lp,
            protocol_fee_bps: protocol,
            creator_fee_bps: creator,
        }
    }

    #[test]
    fn fee_rounds_each_component_up_separately() {
        // 30 bps split three ways on an amount whose every share has a
        // remainder: one combined ceil would charge 3, three charge 3 x 1.
        assert_eq!(fees(10, 10, 10).fee_on(1), 3);
        assert_eq!(fees(30, 0, 0).fee_on(1), 1);
    }

    #[test]
    fn zero_fee_quote_still_subtracts_nothing() {
        let quote = quote_sell(1_000, 10_000, 10_000, Fees::default(), 0.0);
        assert_eq!(quote.fee, 0);
        assert_eq!(quote.amount_out, quote.gross_amount_out);
    }

    #[test]
    fn slippage_only_reduces_the_minimum_not_the_expected_fill() {
        let quote = quote_sell(1_000, 10_000, 10_000, fees(20, 5, 5), 0.01);
        assert!(quote.min_amount_out < quote.amount_out);
        assert_eq!(
            quote.min_amount_out,
            (quote.amount_out as f64 * 0.99) as u64
        );
    }

    #[test]
    fn exact_base_out_quote_rejects_draining_the_reserve() {
        assert!(quote_buy_exact_base_out(10_000, 10_000, 10_000, Fees::default(), 0.0).is_none());
        assert!(quote_buy_exact_base_out(10_001, 10_000, 10_000, Fees::default(), 0.0).is_none());
        assert!(quote_buy_exact_base_out(9_999, 10_000, 10_000, Fees::default(), 0.0).is_some());
    }

    #[test]
    fn constant_product_out_is_exact_at_the_boundary() {
        // 100 * 10 / (10 + 10) = 50 exactly, no rounding either way.
        assert_eq!(constant_product_out(10, 10, 100), 50);
        // 100 * 3 / (10 + 3) = 23.07..., floored.
        assert_eq!(constant_product_out(3, 10, 100), 23);
    }
}
