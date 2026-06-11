use crate::constants::{
    GLOBAL_VOLUME_ACCUMULATOR, POOL_ACCOUNT_NEW_SIZE, PUMP_SWAP_PROGRAM_ID, WRAPPED_SOL_MINT,
};
use crate::instruction::{
    create_pool_instruction_with_options, distribute_creator_fees_instruction,
    make_buy_exact_quote_in_instruction, make_buy_instruction, make_claim_cashback_instruction,
    make_claim_token_incentives_instruction, make_close_user_volume_accumulator_instruction,
    make_collect_coin_creator_fee_instruction, make_deposit_instruction,
    make_extend_account_instruction, make_init_user_volume_accumulator_instruction,
    make_sell_instruction, make_sync_user_volume_accumulator_instruction,
    transfer_creator_fees_to_pump_instruction, withdraw_instruction,
};
use crate::math::calc_amount_out;
use crate::state::PoolInfo;
use crate::util::{
    calc_lp_mint_pda, calc_pool_pda_with_index, calc_user_pool_token_account,
    create_ata_token_or_not_with_program, gen_pubkey_with_seed, load_pool,
};
use anyhow::{Result, anyhow};
use log::{debug, info};
use solana_client::nonblocking::rpc_client::RpcClient;
use solana_sdk::compute_budget::ComputeBudgetInstruction;
use solana_sdk::instruction::Instruction;
use solana_sdk::pubkey::Pubkey;
use solana_sdk::rent::Rent;
use solana_sdk::signature::{Keypair, Signer};
use solana_sdk::system_instruction;
use solana_sdk::transaction::Transaction;
use spl_associated_token_account::instruction::create_associated_token_account_idempotent;
use spl_token::instruction::{close_account as spl_token_close_account, initialize_account};
use spl_token::solana_program::program_pack::Pack;
use std::ops::Deref;
use tokio::time::{Duration, sleep};

/// Async client wrapping an [`RpcClient`] with high-level pump-amm helpers.
///
/// `load_pool` auto-detects each side's token program from the on-chain mint
/// accounts, so callers don't need to know whether a given pool lives on the
/// classic `spl_token` program or `spl_token_2022` — pass any pool pubkey
/// and the SDK routes the right program through every downstream operation.
#[derive(Clone)]
pub struct PumpSwapClient<T: Deref<Target = RpcClient>> {
    pub rpc: T,
    pub program_id: Pubkey,
}

impl<T: Deref<Target = RpcClient>> PumpSwapClient<T> {
    pub fn new(rpc: T) -> Self {
        Self {
            rpc,
            program_id: PUMP_SWAP_PROGRAM_ID,
        }
    }

    /// Fetch a pool and decode it into a [`PoolInfo`] (auto-detects the
    /// base/quote token programs by reading each mint account's owner).
    pub async fn load_pool(&self, pool_pubkey: &Pubkey) -> Result<PoolInfo> {
        load_pool(pool_pubkey, &self.rpc).await
    }

    /// Fetch raw base/quote reserves (in token base units) for a pool.
    pub async fn fetch_pool_reserves(&self, pool: &PoolInfo) -> Result<(u64, u64)> {
        let base_reserve = self
            .rpc
            .get_token_account_balance(&pool.pool_base_token_account)
            .await?;
        let quote_reserve = self
            .rpc
            .get_token_account_balance(&pool.pool_quote_token_account)
            .await?;
        Ok((base_reserve.amount.parse()?, quote_reserve.amount.parse()?))
    }

    /// Fetch pool reserves with both UI and base-unit amounts. Retries with
    /// exponential backoff up to ~6s; returns an error if the RPC remains
    /// unavailable.
    pub async fn fetch_ui_pooled_amounts(&self, pool: &PoolInfo) -> Result<(f64, f64, (u64, u64))> {
        let mut delay = Duration::from_millis(200);

        for _ in 0..6 {
            match async {
                let base = self
                    .rpc
                    .get_token_account_balance(&pool.pool_base_token_account)
                    .await?;
                let quote = self
                    .rpc
                    .get_token_account_balance(&pool.pool_quote_token_account)
                    .await?;
                Ok::<_, anyhow::Error>((
                    base.ui_amount
                        .ok_or_else(|| anyhow!("base ui_amount None"))?,
                    quote
                        .ui_amount
                        .ok_or_else(|| anyhow!("quote ui_amount None"))?,
                    (base.amount.parse()?, quote.amount.parse()?),
                ))
            }
            .await
            {
                Ok(v) => return Ok(v),
                Err(_) => {
                    sleep(delay).await;
                    delay = (delay * 2).min(Duration::from_secs(3));
                }
            }
        }
        Err(anyhow!("fetch_ui_pooled_amounts failed after retries"))
    }

    /// Simulate a sell against the connected RPC. Logs the simulation result.
    ///
    /// Convenience wrapper that uses **1% slippage** by default. For custom
    /// slippage / compute-budget, build the instructions yourself with
    /// [`Self::build_sell_ixs`] and submit them through your own simulation
    /// or send path.
    pub async fn simulate_sell(
        &self,
        pool_info: &PoolInfo,
        amount_in: u64,
        keypair: &Keypair,
    ) -> Result<()> {
        let (base_reserve, quote_reserve) = self.fetch_pool_reserves(pool_info).await?;
        let amount_out = calc_amount_out(amount_in, base_reserve, quote_reserve, 0.01);
        debug!("Sell amount out: {}", amount_out);
        let mut tx = Transaction::new_with_payer(
            &self.build_sell_ixs(amount_in, amount_out, pool_info, &keypair.pubkey(), true)?,
            Some(&keypair.pubkey()),
        );
        tx.sign(&[keypair], self.rpc.get_latest_blockhash().await?);
        let result = self.rpc.simulate_transaction(&tx).await?;
        info!("Simulation result: {:?}", result);
        Ok(())
    }

    /// Simulate a buy against the connected RPC. Logs the simulation result.
    ///
    /// Convenience wrapper that uses **1% slippage** by default. For custom
    /// slippage / compute-budget, build the instructions yourself with
    /// [`Self::build_buy_ixs`] and submit them through your own simulation
    /// or send path.
    pub async fn simulate_buy(
        &self,
        pool_info: &PoolInfo,
        amount_in: u64,
        keypair: &Keypair,
    ) -> Result<()> {
        let (base_reserve, quote_reserve) = self.fetch_pool_reserves(pool_info).await?;
        let base_amount_out = calc_amount_out(amount_in, quote_reserve, base_reserve, 0.01);
        debug!("Buy base amount out: {}", base_amount_out);
        let mut tx = Transaction::new_with_payer(
            &self.build_buy_ixs(
                base_amount_out,
                amount_in,
                true, // track_volume
                pool_info,
                &keypair.pubkey(),
                true,
            )?,
            Some(&keypair.pubkey()),
        );
        tx.sign(&[keypair], self.rpc.get_latest_blockhash().await?);
        let result = self.rpc.simulate_transaction(&tx).await?;
        info!("Simulation result: {:?}", result);
        Ok(())
    }

    /// Create a new WSOL-quoted pool. Funds the new WSOL ATA with
    /// `quote_amount_in` lamports and seeds the pool.
    pub async fn create_wsol_pool(
        &self,
        base_amount_in: u64,
        quote_amount_in: u64,
        coin_creator: &Pubkey,
        base_mint: &Pubkey,
        payer: &Keypair,
    ) -> Result<()> {
        self.create_wsol_pool_with_options(
            0,
            base_amount_in,
            quote_amount_in,
            coin_creator,
            base_mint,
            &spl_token::ID,
            false,
            false,
            payer,
        )
        .await
    }

    /// Create a new WSOL-quoted pool with the current create-pool options.
    #[allow(clippy::too_many_arguments)]
    pub async fn create_wsol_pool_with_options(
        &self,
        index: u16,
        base_amount_in: u64,
        quote_amount_in: u64,
        coin_creator: &Pubkey,
        base_mint: &Pubkey,
        base_token_program: &Pubkey,
        is_mayhem_mode: bool,
        is_cashback_coin: bool,
        payer: &Keypair,
    ) -> Result<()> {
        let wallet = payer.pubkey();
        let pool = calc_pool_pda_with_index(index, &wallet, base_mint, &WRAPPED_SOL_MINT).0;
        info!("Creating pool: {}", pool);

        let base_ata = spl_associated_token_account::get_associated_token_address_with_program_id(
            &wallet,
            base_mint,
            base_token_program,
        );
        let wsol_ata =
            spl_associated_token_account::get_associated_token_address(&wallet, &WRAPPED_SOL_MINT);
        let pool_base_ata =
            spl_associated_token_account::get_associated_token_address_with_program_id(
                &pool,
                base_mint,
                base_token_program,
            );
        let pool_quote_ata =
            spl_associated_token_account::get_associated_token_address(&pool, &WRAPPED_SOL_MINT);

        let wsol_ata_ix = create_associated_token_account_idempotent(
            &wallet,
            &wallet,
            &WRAPPED_SOL_MINT,
            &spl_token::ID,
        );
        let pool_base_ata_ix = create_associated_token_account_idempotent(
            &wallet,
            &pool,
            base_mint,
            base_token_program,
        );
        let pool_quote_ata_ix = create_associated_token_account_idempotent(
            &wallet,
            &pool,
            &WRAPPED_SOL_MINT,
            &spl_token::ID,
        );
        let create_ix = create_pool_instruction_with_options(
            index,
            base_amount_in,
            quote_amount_in,
            is_mayhem_mode,
            is_cashback_coin,
            &pool,
            &wallet,
            coin_creator,
            base_mint,
            &WRAPPED_SOL_MINT,
            &base_ata,
            &wsol_ata,
            &pool_base_ata,
            &pool_quote_ata,
            base_token_program,
            &spl_token::ID,
        )?;

        let instructions = vec![
            wsol_ata_ix,
            system_instruction::transfer(&wallet, &wsol_ata, quote_amount_in),
            spl_token::instruction::sync_native(&spl_token::ID, &wsol_ata)?,
            pool_base_ata_ix,
            pool_quote_ata_ix,
            create_ix,
            spl_token_close_account(&spl_token::ID, &wsol_ata, &wallet, &wallet, &[])?,
        ];

        let mut tx = Transaction::new_with_payer(&instructions, Some(&wallet));
        tx.sign(&[payer], self.rpc.get_latest_blockhash().await?);
        let result = self.rpc.send_and_confirm_transaction(&tx).await?;
        info!("create_pool tx: {}", result);
        Ok(())
    }

    /// Withdraw a fraction of LP from a WSOL-quoted pool.
    pub async fn withdraw_from_wsol_pool(
        &self,
        pool: &Pubkey,
        base_mint: &Pubkey,
        payer: &Keypair,
        lp_token_percent: f64,
        min_base_amount_out: u64,
        min_quote_amount_out: u64,
    ) -> Result<()> {
        let wallet = payer.pubkey();
        let base_ata =
            spl_associated_token_account::get_associated_token_address(&wallet, base_mint);
        let wsol_ata =
            spl_associated_token_account::get_associated_token_address(&wallet, &WRAPPED_SOL_MINT);

        let base_ata_ix =
            create_associated_token_account_idempotent(&wallet, &wallet, base_mint, &spl_token::ID);
        let wsol_ata_ix = create_associated_token_account_idempotent(
            &wallet,
            &wallet,
            &WRAPPED_SOL_MINT,
            &spl_token::ID,
        );

        let lp_mint = calc_lp_mint_pda(pool).0;
        let user_pool_token_account = calc_user_pool_token_account(&wallet, &lp_mint).0;
        let lp_balance = self
            .rpc
            .get_token_account_balance(&user_pool_token_account)
            .await?;
        let balance: u64 = lp_balance.amount.parse()?;
        if balance == 0 {
            return Err(anyhow!("Zero LP token balance"));
        }
        info!("LP token balance: {}", balance);

        let withdraw_ix = withdraw_instruction(
            pool,
            &wallet,
            base_mint,
            &base_ata,
            &wsol_ata,
            (lp_token_percent * balance as f64) as u64,
            min_base_amount_out,
            min_quote_amount_out,
        )?;

        let instructions = vec![
            base_ata_ix,
            wsol_ata_ix,
            withdraw_ix,
            spl_token_close_account(&spl_token::ID, &wsol_ata, &wallet, &wallet, &[])?,
        ];

        let mut tx = Transaction::new_with_payer(&instructions, Some(&wallet));
        tx.sign(&[payer], self.rpc.get_latest_blockhash().await?);
        let result = self.rpc.send_and_confirm_transaction(&tx).await?;
        info!("withdraw tx: {}", result);
        Ok(())
    }

    /// Submit a buy transaction (full send-and-confirm).
    ///
    /// Convenience wrapper using **5% slippage**, **1,000,000 CU limit**, and
    /// **100,000 micro-lamport CU price**. For custom values, compose your
    /// own transaction from [`Self::build_buy_ixs`] plus your own
    /// `ComputeBudgetInstruction`s.
    pub async fn buy(&self, amount_in: u64, pool_info: &PoolInfo, payer: &Keypair) -> Result<()> {
        let (base_reserve, quote_reserve) = self.fetch_pool_reserves(pool_info).await?;
        let amount_out = calc_amount_out(amount_in, quote_reserve, base_reserve, 0.05);
        debug!("Buying amount out: {}", amount_out);

        let mut instructions: Vec<Instruction> = vec![
            ComputeBudgetInstruction::set_compute_unit_limit(1_000_000),
            ComputeBudgetInstruction::set_compute_unit_price(100_000),
        ];
        instructions.extend(self.build_buy_ixs(
            amount_out,
            amount_in,
            true, // track_volume
            pool_info,
            &payer.pubkey(),
            false,
        )?);

        let mut tx = Transaction::new_with_payer(&instructions, Some(&payer.pubkey()));
        tx.sign(&[payer], self.rpc.get_latest_blockhash().await?);
        let result = self.rpc.send_and_confirm_transaction(&tx).await?;
        info!("Buy tx: {}", result);
        Ok(())
    }

    /// Submit a `buy_exact_quote_in` transaction (full send-and-confirm).
    ///
    /// "Spend exactly `spendable_quote_in` of quote (e.g. SOL), accept any
    /// base amount ≥ `min_base_amount_out`." Convenience wrapper using
    /// **1,000,000 CU limit** and **100,000 micro-lamport CU price**, with
    /// `track_volume = true` so the trade counts toward cashback. For custom
    /// values, compose via [`Self::build_buy_exact_quote_in_ixs`].
    pub async fn buy_exact_quote_in(
        &self,
        spendable_quote_in: u64,
        min_base_amount_out: u64,
        pool_info: &PoolInfo,
        payer: &Keypair,
    ) -> Result<()> {
        debug!(
            "buy_exact_quote_in: spend={} min_out={}",
            spendable_quote_in, min_base_amount_out
        );
        let mut instructions: Vec<Instruction> = vec![
            ComputeBudgetInstruction::set_compute_unit_limit(1_000_000),
            ComputeBudgetInstruction::set_compute_unit_price(100_000),
        ];
        instructions.extend(self.build_buy_exact_quote_in_ixs(
            spendable_quote_in,
            min_base_amount_out,
            true, // track_volume
            pool_info,
            &payer.pubkey(),
            false,
        )?);
        let mut tx = Transaction::new_with_payer(&instructions, Some(&payer.pubkey()));
        tx.sign(&[payer], self.rpc.get_latest_blockhash().await?);
        let result = self.rpc.send_and_confirm_transaction(&tx).await?;
        info!("buy_exact_quote_in tx: {}", result);
        Ok(())
    }

    /// Simulate a `buy_exact_quote_in` against the connected RPC.
    ///
    /// Uses `track_volume = true` to match real on-chain behavior; pass a
    /// `min_base_amount_out` of 0 to ignore slippage in simulation.
    pub async fn simulate_buy_exact_quote_in(
        &self,
        spendable_quote_in: u64,
        min_base_amount_out: u64,
        pool_info: &PoolInfo,
        keypair: &Keypair,
    ) -> Result<()> {
        let mut tx = Transaction::new_with_payer(
            &self.build_buy_exact_quote_in_ixs(
                spendable_quote_in,
                min_base_amount_out,
                true,
                pool_info,
                &keypair.pubkey(),
                true,
            )?,
            Some(&keypair.pubkey()),
        );
        tx.sign(&[keypair], self.rpc.get_latest_blockhash().await?);
        let result = self.rpc.simulate_transaction(&tx).await?;
        info!("Simulation result: {:?}", result);
        Ok(())
    }

    /// Claim accrued cashback for `payer`. The user's
    /// `user_volume_accumulator` and its quote-mint ATA must already exist
    /// (they're created by the buy / buy_exact_quote_in flow when
    /// `track_volume = true`).
    pub async fn claim_cashback(
        &self,
        quote_mint: &Pubkey,
        quote_token_program: &Pubkey,
        payer: &Keypair,
    ) -> Result<()> {
        let ix = make_claim_cashback_instruction(&payer.pubkey(), quote_mint, quote_token_program)?;
        let mut tx = Transaction::new_with_payer(&[ix], Some(&payer.pubkey()));
        tx.sign(&[payer], self.rpc.get_latest_blockhash().await?);
        let result = self.rpc.send_and_confirm_transaction(&tx).await?;
        info!("claim_cashback tx: {}", result);
        Ok(())
    }

    /// Build a `claim_token_incentives` instruction set for `user`.
    ///
    /// The user's incentive-token ATA is created idempotently when
    /// `create_user_ata` is true. The global incentive token account is owned
    /// by `GLOBAL_VOLUME_ACCUMULATOR` and must already be funded by the
    /// protocol.
    pub fn build_claim_token_incentives_ixs(
        &self,
        user: &Pubkey,
        payer: &Pubkey,
        mint: &Pubkey,
        token_program: &Pubkey,
        create_user_ata: bool,
    ) -> Result<Vec<Instruction>> {
        let mut instructions = Vec::new();
        let user_token_account =
            spl_associated_token_account::get_associated_token_address_with_program_id(
                user,
                mint,
                token_program,
            );
        if create_user_ata {
            instructions.push(create_associated_token_account_idempotent(
                payer,
                user,
                mint,
                token_program,
            ));
        }
        let global_incentive_token_account =
            spl_associated_token_account::get_associated_token_address_with_program_id(
                &GLOBAL_VOLUME_ACCUMULATOR,
                mint,
                token_program,
            );
        instructions.push(make_claim_token_incentives_instruction(
            user,
            payer,
            mint,
            token_program,
            &user_token_account,
            &global_incentive_token_account,
        )?);
        Ok(instructions)
    }

    /// Build a `collect_coin_creator_fee` instruction set. Creates the coin
    /// creator's quote-token ATA idempotently when requested.
    pub fn build_collect_coin_creator_fee_ixs(
        &self,
        coin_creator: &Pubkey,
        payer: &Pubkey,
        quote_mint: &Pubkey,
        quote_token_program: &Pubkey,
        create_coin_creator_ata: bool,
    ) -> Result<Vec<Instruction>> {
        let mut instructions = Vec::new();
        let coin_creator_token_account =
            spl_associated_token_account::get_associated_token_address_with_program_id(
                coin_creator,
                quote_mint,
                quote_token_program,
            );
        if create_coin_creator_ata {
            instructions.push(create_associated_token_account_idempotent(
                payer,
                coin_creator,
                quote_mint,
                quote_token_program,
            ));
        }
        instructions.push(make_collect_coin_creator_fee_instruction(
            coin_creator,
            quote_mint,
            quote_token_program,
            &coin_creator_token_account,
        )?);
        Ok(instructions)
    }

    /// Build an `extend_account` instruction for older pool accounts.
    pub fn build_extend_account_ix(&self, account: &Pubkey, user: &Pubkey) -> Result<Instruction> {
        make_extend_account_instruction(account, user)
    }

    /// Build an `init_user_volume_accumulator` instruction.
    pub fn build_init_user_volume_accumulator_ix(
        &self,
        payer: &Pubkey,
        user: &Pubkey,
    ) -> Result<Instruction> {
        make_init_user_volume_accumulator_instruction(payer, user)
    }

    /// Build a `sync_user_volume_accumulator` instruction.
    pub fn build_sync_user_volume_accumulator_ix(&self, user: &Pubkey) -> Result<Instruction> {
        make_sync_user_volume_accumulator_instruction(user)
    }

    /// Build a `close_user_volume_accumulator` instruction.
    pub fn build_close_user_volume_accumulator_ix(&self, user: &Pubkey) -> Result<Instruction> {
        make_close_user_volume_accumulator_instruction(user)
    }

    /// Deposit liquidity into a pool, minting `lp_token_amount_out` LP
    /// tokens. The caller's base, quote, and pool-LP ATAs are created
    /// idempotently before the deposit instruction.
    ///
    /// Convenience wrapper for WSOL-quoted pools — for other quote mints
    /// compose via [`make_deposit_instruction`].
    #[allow(clippy::too_many_arguments)]
    pub async fn deposit_into_wsol_pool(
        &self,
        pool_info: &PoolInfo,
        payer: &Keypair,
        lp_token_amount_out: u64,
        max_base_amount_in: u64,
        max_quote_amount_in: u64,
    ) -> Result<()> {
        let wallet = payer.pubkey();
        let base_ata = spl_associated_token_account::get_associated_token_address_with_program_id(
            &wallet,
            &pool_info.base_mint,
            &pool_info.base_token_program,
        );
        let wsol_ata =
            spl_associated_token_account::get_associated_token_address(&wallet, &WRAPPED_SOL_MINT);
        let user_lp_ata = calc_user_pool_token_account(&wallet, &pool_info.lp_mint).0;

        let base_ata_ix = create_associated_token_account_idempotent(
            &wallet,
            &wallet,
            &pool_info.base_mint,
            &pool_info.base_token_program,
        );
        let wsol_ata_ix = create_associated_token_account_idempotent(
            &wallet,
            &wallet,
            &WRAPPED_SOL_MINT,
            &spl_token::ID,
        );
        let lp_ata_ix = create_associated_token_account_idempotent(
            &wallet,
            &wallet,
            &pool_info.lp_mint,
            &spl_token_2022::ID,
        );

        let deposit_ix = make_deposit_instruction(
            lp_token_amount_out,
            max_base_amount_in,
            max_quote_amount_in,
            pool_info,
            &wallet,
            &base_ata,
            &wsol_ata,
            &user_lp_ata,
        )?;

        let mut instructions = Vec::new();
        if let Some(ix) = Self::maybe_extend_pool_ix(pool_info, &wallet)? {
            instructions.push(ix);
        }
        instructions.extend(vec![
            base_ata_ix,
            wsol_ata_ix,
            system_instruction::transfer(&wallet, &wsol_ata, max_quote_amount_in),
            spl_token::instruction::sync_native(&spl_token::ID, &wsol_ata)?,
            lp_ata_ix,
            deposit_ix,
            spl_token_close_account(&spl_token::ID, &wsol_ata, &wallet, &wallet, &[])?,
        ]);

        let mut tx = Transaction::new_with_payer(&instructions, Some(&wallet));
        tx.sign(&[payer], self.rpc.get_latest_blockhash().await?);
        let result = self.rpc.send_and_confirm_transaction(&tx).await?;
        info!("deposit tx: {}", result);
        Ok(())
    }

    /// Submit a sell transaction (full send-and-confirm).
    ///
    /// Convenience wrapper using **10% slippage**, **1,000,000 CU limit**, and
    /// **100,000 micro-lamport CU price**. For custom values, compose your
    /// own transaction from [`Self::build_sell_ixs`] plus your own
    /// `ComputeBudgetInstruction`s.
    pub async fn sell(&self, amount_in: u64, pool_info: &PoolInfo, payer: &Keypair) -> Result<()> {
        let (base_reserve, quote_reserve) = self.fetch_pool_reserves(pool_info).await?;
        let amount_out = calc_amount_out(amount_in, base_reserve, quote_reserve, 0.1);
        debug!("Sell amount out: {}", amount_out);

        let mut instructions: Vec<Instruction> = vec![
            ComputeBudgetInstruction::set_compute_unit_limit(1_000_000),
            ComputeBudgetInstruction::set_compute_unit_price(100_000),
        ];
        instructions.extend(self.build_sell_ixs(
            amount_in,
            amount_out,
            pool_info,
            &payer.pubkey(),
            true,
        )?);

        let mut tx = Transaction::new_with_payer(&instructions, Some(&payer.pubkey()));
        tx.sign(&[payer], self.rpc.get_latest_blockhash().await?);
        let result = self.rpc.send_and_confirm_transaction(&tx).await?;
        info!("Sell tx: {}", result);
        Ok(())
    }

    /// Withdraw accrued creator fees: transfers the coin-creator vault balance
    /// into the pump.fun program's creator vault, then triggers a distribution.
    ///
    /// Convenience wrapper using **500,000 CU limit** and **50,000
    /// micro-lamport CU price**. For custom values, compose your own
    /// transaction from [`Self::build_creator_fee_withdraw_ixs`].
    pub async fn withdraw_creator_fees(
        &self,
        admin: &Keypair,
        coin_creator: &Pubkey,
        token_mint: &Pubkey,
        bonding_curve: &Pubkey,
        sharing_config: &Pubkey,
    ) -> Result<()> {
        info!("Withdrawing creator fees for: {}", admin.pubkey());
        let mut instructions: Vec<Instruction> = vec![
            ComputeBudgetInstruction::set_compute_unit_limit(500_000),
            ComputeBudgetInstruction::set_compute_unit_price(50_000),
        ];
        instructions.extend(self.build_creator_fee_withdraw_ixs(
            coin_creator,
            token_mint,
            bonding_curve,
            sharing_config,
            &admin.pubkey(),
        )?);
        let mut tx = Transaction::new_with_payer(&instructions, Some(&admin.pubkey()));
        tx.sign(&[admin], self.rpc.get_latest_blockhash().await?);
        let result = self.rpc.send_and_confirm_transaction(&tx).await?;
        info!("withdraw_creator_fees tx: {}", result);
        Ok(())
    }

    /// Compose the two-instruction creator-fee withdraw flow.
    pub fn build_creator_fee_withdraw_ixs(
        &self,
        coin_creator: &Pubkey,
        token_mint: &Pubkey,
        bonding_curve: &Pubkey,
        sharing_config: &Pubkey,
        admin_account: &Pubkey,
    ) -> Result<Vec<Instruction>> {
        Ok(vec![
            transfer_creator_fees_to_pump_instruction(coin_creator)?,
            distribute_creator_fees_instruction(
                token_mint,
                bonding_curve,
                sharing_config,
                admin_account,
            )?,
        ])
    }

    fn maybe_extend_pool_ix(pool_info: &PoolInfo, payer: &Pubkey) -> Result<Option<Instruction>> {
        if pool_info.pool_account_data_len < POOL_ACCOUNT_NEW_SIZE {
            Ok(Some(make_extend_account_instruction(
                &pool_info.pool,
                payer,
            )?))
        } else {
            Ok(None)
        }
    }

    /// Build the instruction sequence for a buy (exact-base-out): a fresh
    /// seed-derived WSOL account, optional ATA creation for the base mint,
    /// the buy ix, and a close of the WSOL account back to lamports.
    ///
    /// `track_volume = true` makes the trade count toward the caller's
    /// cashback accumulator — set `false` only if you explicitly don't want
    /// it tracked.
    pub fn build_buy_ixs(
        &self,
        base_amount_out: u64,
        max_quote_amount_in: u64,
        track_volume: bool,
        pool_info: &PoolInfo,
        payer: &Pubkey,
        create_ata: bool,
    ) -> Result<Vec<Instruction>> {
        let mut instructions: Vec<Instruction> = Vec::new();
        if let Some(ix) = Self::maybe_extend_pool_ix(pool_info, payer)? {
            instructions.push(ix);
        }
        let (wsol_acc, seed) = gen_pubkey_with_seed(payer)?;
        let span = spl_token::state::Account::LEN;
        let min_rent_exempt = Rent::default().minimum_balance(span);

        // Token side (base mint when quote is WSOL, otherwise quote mint).
        let (token_mint, wsol_mint, token_program) = if pool_info.base_mint != WRAPPED_SOL_MINT {
            (
                pool_info.base_mint,
                pool_info.quote_mint,
                pool_info.base_token_program,
            )
        } else {
            (
                pool_info.quote_mint,
                pool_info.base_mint,
                pool_info.quote_token_program,
            )
        };
        instructions.extend(vec![
            system_instruction::create_account_with_seed(
                payer,
                &wsol_acc,
                payer,
                &seed,
                min_rent_exempt + max_quote_amount_in,
                span as u64,
                &spl_token::ID,
            ),
            initialize_account(&spl_token::ID, &wsol_acc, &wsol_mint, payer)?,
        ]);

        let (ata_acc, ata_acc_inst) = create_ata_token_or_not_with_program(
            payer,
            &token_mint,
            payer,
            &token_program,
            create_ata,
        );
        if let Some(ata_inst) = ata_acc_inst {
            instructions.push(ata_inst);
        }

        instructions.push(make_buy_instruction(
            base_amount_out,
            max_quote_amount_in,
            track_volume,
            pool_info,
            payer,
            &ata_acc,
            &wsol_acc,
        )?);
        instructions.push(spl_token_close_account(
            &spl_token::ID,
            &wsol_acc,
            payer,
            payer,
            &[],
        )?);
        Ok(instructions)
    }

    /// Build the instruction sequence for a `buy_exact_quote_in` (exact-quote-in):
    /// fresh WSOL account funded with `spendable_quote_in`, optional base-mint
    /// ATA creation, the buy ix, and a close of the WSOL account. Most trader
    /// bots want this entry point — "spend exactly N quote, accept ≥ min base".
    pub fn build_buy_exact_quote_in_ixs(
        &self,
        spendable_quote_in: u64,
        min_base_amount_out: u64,
        track_volume: bool,
        pool_info: &PoolInfo,
        payer: &Pubkey,
        create_ata: bool,
    ) -> Result<Vec<Instruction>> {
        let mut instructions: Vec<Instruction> = Vec::new();
        if let Some(ix) = Self::maybe_extend_pool_ix(pool_info, payer)? {
            instructions.push(ix);
        }
        let (wsol_acc, seed) = gen_pubkey_with_seed(payer)?;
        let span = spl_token::state::Account::LEN;
        let min_rent_exempt = Rent::default().minimum_balance(span);

        let (token_mint, wsol_mint, token_program) = if pool_info.base_mint != WRAPPED_SOL_MINT {
            (
                pool_info.base_mint,
                pool_info.quote_mint,
                pool_info.base_token_program,
            )
        } else {
            (
                pool_info.quote_mint,
                pool_info.base_mint,
                pool_info.quote_token_program,
            )
        };
        instructions.extend(vec![
            system_instruction::create_account_with_seed(
                payer,
                &wsol_acc,
                payer,
                &seed,
                min_rent_exempt + spendable_quote_in,
                span as u64,
                &spl_token::ID,
            ),
            initialize_account(&spl_token::ID, &wsol_acc, &wsol_mint, payer)?,
        ]);

        let (ata_acc, ata_acc_inst) = create_ata_token_or_not_with_program(
            payer,
            &token_mint,
            payer,
            &token_program,
            create_ata,
        );
        if let Some(ata_inst) = ata_acc_inst {
            instructions.push(ata_inst);
        }

        instructions.push(make_buy_exact_quote_in_instruction(
            spendable_quote_in,
            min_base_amount_out,
            track_volume,
            pool_info,
            payer,
            &ata_acc,
            &wsol_acc,
        )?);
        instructions.push(spl_token_close_account(
            &spl_token::ID,
            &wsol_acc,
            payer,
            payer,
            &[],
        )?);
        Ok(instructions)
    }

    /// Build the instruction sequence for a sell: a fresh seed-derived WSOL
    /// account, the sell ix, a close of the WSOL account, and optionally a
    /// close of the user's base ATA when emptying a position.
    pub fn build_sell_ixs(
        &self,
        base_amount_in: u64,
        min_quote_amount_out: u64,
        pool_info: &PoolInfo,
        payer: &Pubkey,
        close_spl_acc: bool,
    ) -> Result<Vec<Instruction>> {
        let mut instructions: Vec<Instruction> = Vec::new();
        if let Some(ix) = Self::maybe_extend_pool_ix(pool_info, payer)? {
            instructions.push(ix);
        }
        let (wsol_acc, seed) = gen_pubkey_with_seed(payer)?;
        let span = spl_token::state::Account::LEN;
        let min_rent_exempt = Rent::default().minimum_balance(span);

        let (token_mint, wsol_mint, token_program) = if pool_info.base_mint != WRAPPED_SOL_MINT {
            (
                pool_info.base_mint,
                pool_info.quote_mint,
                pool_info.base_token_program,
            )
        } else {
            (
                pool_info.quote_mint,
                pool_info.base_mint,
                pool_info.quote_token_program,
            )
        };
        instructions.extend(vec![
            system_instruction::create_account_with_seed(
                payer,
                &wsol_acc,
                payer,
                &seed,
                min_rent_exempt,
                span as u64,
                &spl_token::ID,
            ),
            initialize_account(&spl_token::ID, &wsol_acc, &wsol_mint, payer)?,
        ]);

        let token_ata_in =
            spl_associated_token_account::get_associated_token_address_with_program_id(
                payer,
                &token_mint,
                &token_program,
            );

        instructions.push(make_sell_instruction(
            base_amount_in,
            min_quote_amount_out,
            pool_info,
            payer,
            &token_ata_in,
            &wsol_acc,
        )?);
        instructions.push(spl_token_close_account(
            &spl_token::ID,
            &wsol_acc,
            payer,
            payer,
            &[],
        )?);
        if close_spl_acc {
            instructions.push(spl_token_close_account(
                &token_program,
                &token_ata_in,
                payer,
                payer,
                &[],
            )?);
        }
        Ok(instructions)
    }
}

/// Read the SPL-Token balance for `(owner, mint)` under `token_program`.
///
/// Returns:
/// - `Ok(Some(amount))` when the ATA exists and the balance was read.
/// - `Ok(None)` when the ATA does not exist on chain.
/// - `Err(_)` when the RPC call fails (network error, malformed response,
///   non-numeric amount, etc.) so callers can distinguish "no balance" from
///   "I couldn't reach the RPC".
pub async fn get_token_balance(
    rpc: &RpcClient,
    pubkey: &Pubkey,
    mint: &Pubkey,
    token_program: &Pubkey,
) -> Result<Option<u64>> {
    let token_account = spl_associated_token_account::get_associated_token_address_with_program_id(
        pubkey,
        mint,
        token_program,
    );
    match rpc.get_token_account_balance(&token_account).await {
        Ok(balance) => Ok(Some(balance.amount.parse()?)),
        Err(solana_client::client_error::ClientError {
            kind:
                solana_client::client_error::ClientErrorKind::RpcError(
                    solana_client::rpc_request::RpcError::ForUser(msg),
                ),
            ..
        }) if msg.contains("AccountNotFound") || msg.contains("could not find account") => Ok(None),
        Err(e) => Err(e.into()),
    }
}
