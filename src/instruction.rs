#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct BuyInstruction {
    pub base_amount_out: u64,
    pub max_quote_amount_in: u64,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct SellInstruction {
    pub base_amount_in: u64,
    pub min_quote_amount_out: u64,
}