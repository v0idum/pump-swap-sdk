use crate::{
    constants::PUMP_SWAP_PROGRAM_ID,
    pool::{Pool, load_pool},
    math::calc_amount_out,
};
use solana_sdk::{pubkey::Pubkey};
use solana_client::nonblocking::rpc_client::RpcClient;
use anyhow::Result;

pub struct PumpSwapClient {
    pub rpc: RpcClient,
    pub program_id: Pubkey,
}

impl PumpSwapClient {
    pub fn new(rpc: RpcClient) -> Self {
        Self {
            rpc,
            program_id: PUMP_SWAP_PROGRAM_ID,
        }
    }

    pub async fn load_pool(&self, pool_pubkey: Pubkey) -> Result<Pool> {
        let data = self.rpc.get_account_data(&pool_id).await?;
        if data.len() < size_of::<Pool>() {
            anyhow::bail!("Data too short: expected at least {}, got {}", size_of::<Pool>(), data.len());
        }
        let pool = from_bytes::<Pool>(&data[8..]);
        Ok(*pool)
    }

    pub async fn simulate_sell(&self, pool: Pool, amount_in: u64) -> Result<u64> {
        Ok(calc_amount_out(amount_in, pool.reserve_in(), pool.reserve_out()))
    }

    pub async fn simulate_buy(&self, pool: Pool, amount_in: u64) -> Result<u64> {
        // If buy logic is different from sell, customize here
        Ok(calc_amount_out(amount_in, pool.reserve_out(), pool.reserve_in()))
    }

}
