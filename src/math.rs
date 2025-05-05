pub fn calc_amount_out(amount_in: u64, reserve_in: u64, reserve_out: u64) -> u64 {
    let amount_in = amount_in as f64;
    let reserve_in = reserve_in as f64;
    let reserve_out = reserve_out as f64;

    let product = reserve_in * reserve_out;
    let new_in_reserve = reserve_in + amount_in;
    let new_out_reserve = product / new_in_reserve + 1.0;
    let result = reserve_out - new_out_reserve;
    result.max(0.0).round() as u64
}