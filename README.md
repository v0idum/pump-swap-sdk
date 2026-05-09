# pump-swap-sdk

Rust SDK for the [PumpSwap (pump-amm)](https://pump.fun) AMM on Solana.

Builds buy / sell / create-pool / withdraw / creator-fee-distribution
instructions, derives pool and fee PDAs, simulates or submits swaps via an
`RpcClient`, and bundles transactions through Jito.

The SDK targets the current pump-amm IDL: Buy uses the 23-account canonical
layout, Sell uses 21. On top of those it appends the live program's required
`remaining_accounts` (cashback ATA when the pool is a cashback coin, the
`pool-v2` PDA when `pool.coin_creator != default`, plus a randomly-selected
buyback fee recipient and its quote-mint ATA). Both classic SPL Token and
SPL Token-2022 base/quote mints are supported — `load_pool` detects each
side's token program from the mint account's owner and the instruction
builders use the right program for every ATA derivation.

You only supply the pool, the user, and the user's token accounts.

## Install

```toml
[dependencies]
pump-swap-sdk = "0.2"
```

Requires Rust 1.85+ (edition 2024).

## Quickstart

```rust
use std::sync::Arc;
use std::str::FromStr;
use solana_client::nonblocking::rpc_client::RpcClient;
use solana_sdk::commitment_config::CommitmentConfig;
use solana_sdk::pubkey::Pubkey;
use pump_swap_sdk::PumpSwapClient;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let rpc_url = std::env::var("RPC_URL")?;
    let pool = std::env::var("POOL")?;

    let rpc = Arc::new(RpcClient::new_with_commitment(
        rpc_url,
        CommitmentConfig::confirmed(),
    ));
    let client = PumpSwapClient::new(rpc); // auto-detect token programs
    let pool_info = client.load_pool(&Pubkey::from_str(&pool)?).await?;
    let (base, quote) = client.fetch_pool_reserves(&pool_info).await?;
    println!("reserves: base={}, quote={}", base, quote);
    Ok(())
}
```

### Token-2022 support is automatic

`load_pool` (and `client.load_pool`) read each mint's program owner and
populate `PoolInfo.base_token_program` / `PoolInfo.quote_token_program`
automatically. Every downstream operation in the SDK
(`make_buy_instruction`, `make_sell_instruction`, `build_buy_ixs`,
`build_sell_ixs`) reads those fields and routes to the right program — pass
any pool pubkey and the SDK handles the rest. Mixed-mode pools (base on one
program, quote on the other) work transparently.

## Examples

Every example is env-var driven — no config files to wrangle. Run with
`cargo run --example <name>`:

| Example | What it does |
|---|---|
| `load_pool` | Fetch and print a pool's `PoolInfo` and current reserves. |
| `simulate_buy` | Build and simulate a buy via `PumpSwapClient::simulate_buy`. |
| `verify_layout` | Sanity-check that Buy/Sell account layouts execute against the live program (uses `simulateTransaction` with `sigVerify=false`, so only a user pubkey is needed — no signing keypair). |
| `batch_buy` | Send a paced batch of buys across N wallets. Optional Jito-bundle submission via `JITO_ENDPOINTS=`. |
| `creator_fees_runner` | One cycle of the creator-fee claim flow: claim accrued fees, sweep surplus SOL to a target wallet. |

Each example prints the env vars it expects in its module-level docstring.

## Building instructions directly

```rust
use pump_swap_sdk::{make_buy_instruction, load_pool};
// pool_info from load_pool, user pubkey, user token accounts
let ix = make_buy_instruction(
    base_amount_out,
    max_quote_amount_in,
    &pool_info,
    &user,
    &user_base_ata,
    &user_quote_ata,
)?;
```

`make_buy_instruction` / `make_sell_instruction` derive every PDA they need
(`coin_creator_vault_authority`, `coin_creator_vault_ata`,
`user_volume_accumulator`, `fee_config`, plus the protocol-fee-recipient ATA
for the pool's quote mint).

## Public API

- `PumpSwapClient` — high-level wrapper around `RpcClient` with `simulate_buy`,
  `simulate_sell`, `buy`, `sell`, `build_buy_ixs`, `build_sell_ixs`,
  `create_wsol_pool`, `withdraw_from_wsol_pool`, `withdraw_creator_fees`.
- `make_buy_instruction`, `make_sell_instruction` — raw instruction builders
  for the canonical 23-account Buy and 21-account Sell layouts plus the
  cashback / buyback `remaining_accounts` the live program requires.
- `create_pool_instruction`, `withdraw_instruction`,
  `transfer_creator_fees_to_pump_instruction`,
  `distribute_creator_fees_instruction` — additional instruction builders.
- `load_pool` — fetch and decode a pool account into `PoolInfo`.
- `calc_amount_out`, `buy_amount_out`, `sell_amount_out` — constant-product math.
- `find_coin_creator_vault_authority`, `find_coin_creator_vault_ata`,
  `find_user_vol_accumulator`, `fee_config_pda`, `calc_pool_pda`,
  `calc_lp_mint_pda`, `calc_user_pool_token_account` — PDA helpers.
- `JitoPool`, `send_jito_bundle`, `send_bundle_with_retry` — Jito bundle
  helpers (round-robin pool with per-region cooldown).

## License

MIT
