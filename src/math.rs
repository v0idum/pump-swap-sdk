use crate::constants::WRAPPED_SOL_MINT;
use crate::state::PoolInfo;

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

pub fn buy_amount_out(amount_in: u64, reserves: (u64, u64), pool: &PoolInfo, slippage: f64) -> u64 {
    if pool.base_mint != WRAPPED_SOL_MINT {
        calc_amount_out(amount_in, reserves.1, reserves.0, slippage)
    } else {
        calc_amount_out(amount_in, reserves.0, reserves.1, slippage)
    }
}

pub fn sell_amount_out(amount_in: u64, reserves: (u64, u64), pool: &PoolInfo, slippage: f64) -> u64 {
    if pool.base_mint != WRAPPED_SOL_MINT {
        calc_amount_out(amount_in, reserves.0, reserves.1, slippage)
    } else {
        calc_amount_out(amount_in, reserves.1, reserves.0, slippage)
    }
}
