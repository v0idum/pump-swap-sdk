# pump-swap-sdk

[![CI](https://github.com/v0idum/pump-swap-sdk/actions/workflows/ci.yml/badge.svg)](https://github.com/v0idum/pump-swap-sdk/actions/workflows/ci.yml)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)
[![MSRV](https://img.shields.io/badge/MSRV-1.85-blue.svg)](Cargo.toml)

Rust SDK for building and submitting [PumpSwap (pump-amm)](https://swap.pump.fun/)
AMM instructions on Solana.

This crate focuses on the moving parts that are easy to get wrong when talking
to the live pump-amm program: account layouts, PDA derivation, Token-2022
associated token accounts, creator-fee accounts, cashback/buyback accounts,
quote math, and optional Jito bundle submission.

## What it supports

- Fetching and decoding pump-amm pool accounts into `PoolInfo`.
- Automatic SPL Token vs SPL Token-2022 detection for each pool mint.
- Buy and sell instruction builders for the current pump-amm account layouts.
- Pool creation, withdrawal, and creator-fee distribution instruction helpers.
- Reserve-aware buy/sell quote helpers.
- High-level `PumpSwapClient` helpers for loading pools, simulating swaps,
  building transaction instruction sets, and submitting convenience buys/sells.
- Jito bundle submission helpers with a round-robin endpoint pool.

## Install

Install from crates.io:

```toml
[dependencies]
pump-swap-sdk = "0.1.0"
```

Requires Rust 1.85+.

Or install directly from Git:

```toml
[dependencies]
pump-swap-sdk = { git = "https://github.com/v0idum/pump-swap-sdk", branch = "master" }
```

For the quickstart or examples below, a binary crate will also need the Solana
client types and an async runtime:

```toml
[dependencies]
pump-swap-sdk = "0.1.0"
anyhow = "1"
tokio = { version = "1", features = ["macros", "rt-multi-thread"] }
solana-client = "2"
solana-sdk = "2"
```

## Quickstart

This first example is read-only. It loads a pool, detects the token programs,
and prints current reserves.

```rust
use std::str::FromStr;
use std::sync::Arc;

use pump_swap_sdk::PumpSwapClient;
use solana_client::nonblocking::rpc_client::RpcClient;
use solana_sdk::commitment_config::CommitmentConfig;
use solana_sdk::pubkey::Pubkey;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let rpc_url = std::env::var("RPC_URL")?;
    let pool = Pubkey::from_str(&std::env::var("POOL")?)?;

    let rpc = Arc::new(RpcClient::new_with_commitment(
        rpc_url,
        CommitmentConfig::confirmed(),
    ));
    let client = PumpSwapClient::new(rpc);

    let pool_info = client.load_pool(&pool).await?;
    let (base_reserve, quote_reserve) = client.fetch_pool_reserves(&pool_info).await?;

    println!("base_mint:  {}", pool_info.base_mint);
    println!("quote_mint: {}", pool_info.quote_mint);
    println!("base_token_program:  {}", pool_info.base_token_program);
    println!("quote_token_program: {}", pool_info.quote_token_program);
    println!("reserves: base={base_reserve}, quote={quote_reserve}");

    Ok(())
}
```

Run it with:

```sh
RPC_URL=https://api.mainnet-beta.solana.com \
POOL=<pool_pubkey> \
cargo run
```

## Usage

The high-level client gives you two levels of control:

- `buy`, `sell`, `simulate_buy`, and `simulate_sell` are convenience helpers.
- `build_buy_ixs` and `build_sell_ixs` return instructions so you can set your
  own slippage, compute budget, priority fee, signing, retry, and confirmation
  policy.

### Build buy instructions

For common WSOL-quoted pools, `build_buy_ixs` creates the ephemeral WSOL
account, optionally creates the user's token ATA, builds the pump-amm Buy
instruction, and closes the WSOL account back to the payer.

```rust
use pump_swap_sdk::buy_amount_out;
use solana_sdk::native_token::sol_to_lamports;
use solana_sdk::signature::Signer;

let (base_reserve, quote_reserve) = client.fetch_pool_reserves(&pool_info).await?;
let amount_in = sol_to_lamports(0.01);
let base_amount_out = buy_amount_out(
    amount_in,
    (base_reserve, quote_reserve),
    &pool_info,
    0.01, // 1% slippage
);

let instructions = client.build_buy_ixs(
    base_amount_out,
    amount_in,
    &pool_info,
    &payer.pubkey(),
    true, // create the user's token ATA if missing
)?;
```

### Build sell instructions

Sell amounts are token base units. `build_sell_ixs` creates the ephemeral WSOL
account for quote output, builds the pump-amm Sell instruction, and closes the
WSOL account after the swap.

```rust
use pump_swap_sdk::sell_amount_out;
use solana_sdk::signature::Signer;

let (base_reserve, quote_reserve) = client.fetch_pool_reserves(&pool_info).await?;
let base_amount_in = 1_000_000;
let min_quote_amount_out = sell_amount_out(
    base_amount_in,
    (base_reserve, quote_reserve),
    &pool_info,
    0.01, // 1% slippage
);

let instructions = client.build_sell_ixs(
    base_amount_in,
    min_quote_amount_out,
    &pool_info,
    &payer.pubkey(),
    false, // close the user's token ATA only when intentionally emptying it
)?;
```

### Customize transaction options

Use the instruction builders when you need explicit compute budget, priority
fee, signing, or send behavior.

```rust
use solana_sdk::compute_budget::ComputeBudgetInstruction;
use solana_sdk::signature::Signer;
use solana_sdk::transaction::Transaction;

let mut instructions = vec![
    ComputeBudgetInstruction::set_compute_unit_limit(1_000_000),
    ComputeBudgetInstruction::set_compute_unit_price(100_000),
];

instructions.extend(client.build_buy_ixs(
    base_amount_out,
    amount_in,
    &pool_info,
    &payer.pubkey(),
    true,
)?);

let mut tx = Transaction::new_with_payer(&instructions, Some(&payer.pubkey()));
tx.sign(&[&payer], client.rpc.get_latest_blockhash().await?);

let sig = client.rpc.send_and_confirm_transaction(&tx).await?;
println!("tx: {sig}");
```

### Simulate before sending

```rust
client.simulate_buy(&pool_info, sol_to_lamports(0.001), &payer).await?;
client.simulate_sell(&pool_info, base_amount_in, &payer).await?;
```

### Send through Jito

```rust
use std::sync::Arc;
use std::time::Duration;

use pump_swap_sdk::{send_jito_bundle, JitoPool};

let jito_pool = Arc::new(JitoPool::new(
    &["https://mainnet.block-engine.jito.wtf"],
    None,
    Duration::from_millis(200),
)?);

send_jito_bundle(vec![tx], jito_pool.next_client().await).await?;
```

## Examples

Every example is driven by environment variables. The examples print useful
status to stdout and return errors for missing or malformed inputs.

### Basic examples

| Example | Purpose | Command |
|---|---|---|
| `load_pool` | Fetch pool metadata and reserves. | `RPC_URL=<url> POOL=<pool> cargo run --example load_pool` |
| `verify_layout` | Simulate Buy/Sell account layouts without signing. | `RPC_URL=<url> POOL=<pool> USER=<funded_pubkey> cargo run --example verify_layout` |
| `simulate_buy` | Build and simulate a buy transaction. | `RPC_URL=<url> POOL=<pool> KEYPAIR=<base58_secret> AMOUNT_SOL=0.001 cargo run --example simulate_buy` |

### Advanced examples

| Example | Purpose | Command |
|---|---|---|
| `batch_buy` | Send paced buys across multiple wallets, optionally via Jito. | `RPC_URL=<url> POOL=<pool> KEYPAIRS=<secret1>,<secret2> AMOUNT_LAMPORTS=500000 cargo run --example batch_buy` |
| `creator_fees_runner` | Claim creator fees and sweep surplus SOL. | See the module docstring for required creator-fee accounts. |

Start with `load_pool` or `verify_layout`. They are the safest way to validate
pool decoding and account layout compatibility before signing a transaction.

## Important behavior

### Token programs

`load_pool` and `PumpSwapClient::load_pool` read the owner of each mint account
and populate:

- `PoolInfo.base_token_program`
- `PoolInfo.quote_token_program`

Instruction builders use those fields when deriving ATAs and adding token
program accounts. Mixed SPL Token / Token-2022 pools are supported.

### Buy and sell direction

The raw instruction names follow the pump-amm IDL:

- Buy means quote -> base.
- Sell means base -> quote.

For common WSOL-quoted pools, this maps to SOL -> token and token -> SOL. If
you work with a pool whose WSOL side is `base_mint`, be explicit about reserve
ordering and simulate before sending.

### Protocol compatibility

The swap builders include the live program accounts required for current
pump-amm pools, including cashback, creator, fee, and buyback-related accounts.
Most callers should not add those accounts manually; pass `PoolInfo`, the user,
and the user's token accounts or use `build_buy_ixs` / `build_sell_ixs`.

## API overview

Generate local API docs with:

```sh
cargo doc --open
```

- `PumpSwapClient`: high-level RPC client wrapper.
- `make_buy_instruction`, `make_sell_instruction`: raw swap instruction
  builders.
- `create_pool_instruction`, `withdraw_instruction`,
  `transfer_creator_fees_to_pump_instruction`,
  `distribute_creator_fees_instruction`: additional pump-amm and pump.fun
  instruction builders.
- `load_pool`, `load_pool_with_token_program`: pool account decoding helpers.
- `calc_amount_out`, `buy_amount_out`, `sell_amount_out`: quote math helpers.
- PDA helpers such as `calc_pool_pda`, `calc_lp_mint_pda`,
  `find_coin_creator_vault_authority`, `find_coin_creator_vault_ata`,
  `find_user_vol_accumulator`, and `fee_config_pda`.
- `JitoPool`, `send_jito_bundle`, `send_bundle_with_retry`: Jito bundle
  helpers.

## Safety

This SDK can build and submit mainnet transactions. Treat every example that
uses a keypair as capable of spending real funds.

- This project is not audited.
- This project is not an official Pump.fun product.
- Simulate transactions before sending them.
- Do not commit `.env` files, keypair JSON, or base58 secret keys.
- Review default slippage, compute budget, and priority fee values before using
  the convenience `buy` / `sell` methods in production.

## Development

This section is for maintainers and contributors. Run the check set before
opening a pull request:

```sh
cargo fmt --all -- --check
cargo clippy --all-targets -- -D warnings
cargo test --all-targets
cargo test --doc
RUSTDOCFLAGS="-D warnings" cargo doc --no-deps
cargo package
```

## License

MIT
