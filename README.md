# pump-swap-sdk

[![Crates.io](https://img.shields.io/crates/v/pump-swap-sdk.svg)](https://crates.io/crates/pump-swap-sdk)
[![Docs.rs](https://docs.rs/pump-swap-sdk/badge.svg)](https://docs.rs/pump-swap-sdk)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)
[![MSRV](https://img.shields.io/badge/MSRV-1.97.1-blue.svg)](Cargo.toml)

Rust SDK for [Pump.fun's PumpSwap (pump-amm)](https://swap.pump.fun/) AMM on
Solana.

A small, focused library for building Solana applications on top of
PumpSwap — trading, providing liquidity, claiming creator fees, anything
the protocol supports.

- **Focused** — one library, one protocol; every public API maps to a
  pump-amm concept.
- **Current** — tracks the live program's account layout, not a snapshot
  of last year's IDL.
- **Ergonomic** — pass a pool pubkey, get back a working transaction.
- **Composable** — high-level convenience methods or raw instruction
  builders; bring your own `RpcClient`.

## Features

- Fetching and decoding pump-amm pool accounts into `PoolInfo`.
- Automatic SPL Token vs SPL Token-2022 detection for each pool mint.
- Buy / `buy_exact_quote_in` / sell / deposit / withdraw / claim-cashback
  instruction builders for the current pump-amm account layouts.
- Pool creation and creator-fee distribution instruction helpers.
- Current pool flags and account maintenance helpers, including Mayhem-mode
  fee recipient selection, `extend_account`, user volume accumulators, token
  incentives, and direct coin-creator fee collection.
- Reserve-aware buy/sell quote helpers.
- Live fee-state decoding: pump-amm `GlobalConfig` and the fee program's
  `FeeConfig` (flat fees plus the standard and stable market-cap fee
  ladders), fetched in one RPC call.
- Volume-accumulator decoding: a user's `UserVolumeAccumulator` (cashback and
  token-incentive counters) and the program-wide `GlobalVolumeAccumulator`.
- High-level `PumpSwapClient` for loading pools, simulating swaps,
  building transaction instruction sets, and submitting convenience
  buys/sells.
- Jito bundle submission helpers with a round-robin endpoint pool.

## Install

Install from crates.io:

```toml
[dependencies]
pump-swap-sdk = "0.5.0"
```

Requires Rust 1.97.1+.

Or install directly from Git:

```toml
[dependencies]
pump-swap-sdk = { git = "https://github.com/v0idum/pump-swap-sdk", branch = "master" }
```

For the quickstart or examples below, a binary crate will also need the Solana
client types and an async runtime:

```toml
[dependencies]
pump-swap-sdk = "0.5.0"
anyhow = "1"
tokio = { version = "1", features = ["macros", "rt-multi-thread"] }
solana-client = "4"
solana-sdk = "4"
solana-commitment-config = "3"
```

`CommitmentConfig` now lives in its own crate: the `solana-sdk` 2.x → 4.x split
moved it (and `system_instruction`, `compute_budget`, `system_program`) out of
the monolithic crate.

## Quickstart

This first example is read-only. It loads a pool, detects the token programs,
and prints current reserves.

```rust
use std::str::FromStr;
use std::sync::Arc;

use pump_swap_sdk::PumpSwapClient;
use solana_client::nonblocking::rpc_client::RpcClient;
use solana_commitment_config::CommitmentConfig;
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
    println!("mayhem:     {}", pool_info.is_mayhem_mode);
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
use solana_native_token::LAMPORTS_PER_SOL;
use solana_sdk::signature::Signer;

let amount_in = LAMPORTS_PER_SOL / 100; // 0.01 SOL
// Subtracts the pool's live, market-cap-tiered fee before applying slippage,
// so 0.1% here is a slippage budget and nothing else.
let quote = client.quote_token_buy(amount_in, &pool_info, 0.001).await?;
println!("fee {} lamports, expect {} tokens", quote.fee, quote.amount_out);

let instructions = client.build_buy_ixs(
    quote.min_amount_out,
    amount_in,
    true, // track_volume — count toward cashback accumulator
    &pool_info,
    &payer.pubkey(),
    true, // create the user's token ATA if missing
)?;
```

> Quoting a swap as pure constant product is what makes the program reject
> `min_out` with `ExceededSlippage` (`Custom(6004)`): pump-amm charges 30–125
> bps depending on the pool's market cap, on top of the curve. `calc_amount_out`
> is still there for the raw curve, but it does not price a real swap.

### Spend-exact-quote buys (`buy_exact_quote_in`)

For "spend exactly N SOL, accept ≥ min base out" semantics — what most
trader bots want — use the `buy_exact_quote_in` family:

```rust
use solana_native_token::LAMPORTS_PER_SOL;

let spend = LAMPORTS_PER_SOL / 100; // 0.01 SOL
let instructions = client.build_buy_exact_quote_in_ixs(
    spend,
    1, // min_base_amount_out (program rejects 0)
    true, // track_volume
    &pool_info,
    &payer.pubkey(),
    true,
)?;

// Or the full convenience: client.buy_exact_quote_in(spend, min_out, &pool_info, &payer).await?;
```

### Build sell instructions

Sell amounts are token base units. `build_sell_ixs` creates the ephemeral WSOL
account for quote output, builds the pump-amm Sell instruction, and closes the
WSOL account after the swap.

```rust
use solana_sdk::signature::Signer;

let base_amount_in = 1_000_000;
let quote = client.quote_token_sell(base_amount_in, &pool_info, 0.001).await?;

let instructions = client.build_sell_ixs(
    base_amount_in,
    quote.min_amount_out,
    &pool_info,
    &payer.pubkey(),
    false, // close the user's token ATA only when intentionally emptying it
)?;
```

### How swaps are priced

`quote_token_buy` / `quote_token_sell` reproduce pump-amm's own arithmetic, so
`SwapQuote::amount_out` is what the program credits and `min_amount_out` is a
floor it will accept. Three things the raw constant product misses:

- **A market-cap-tiered fee.** On every swap pump-amm CPIs the fee program's
  `GetFeesWithQuoteMint` with `(is_pump_pool, market_cap_lamports, quote_mint)`.
  Pools carrying a `coin_creator` — pump.fun graduates — are priced off a
  25-rung ladder running from 125 bps at the bottom to 30 bps at the top;
  everything else pays the flat 30 bps schedule. `flat_fees` is **not** a
  fallback for "no tier matched" — treating it as one under-charges a
  low-market-cap pool by 95 bps. A ladder-priced pool quoted in something other
  than SOL is priced by a rule this SDK has not established, so
  `fetch_pool_fees` refuses it rather than guessing low; see `can_quote_fees`.
- **Per-component rounding.** The lp, protocol and creator shares are each
  rounded up separately, so the fee is up to two units more than one rounded-up
  calculation on the combined rate.
- **Virtual quote liquidity.** Pool accounts carry a `u64` past
  `is_cashback_coin` that the published IDL does not list, and the program adds
  it to the quote token-account balance both when stepping the curve and when
  computing market cap. `PoolInfo::virtual_quote_reserves` exposes it and
  `effective_reserves` applies it; a pool carrying it is mispriced by orders of
  magnitude without it.

Fee schedule and pricing rules are pinned by `tests/swap_math.rs`, which
replays byte-for-byte mainnet swap events through this math offline. The
`#[ignore]`d tests in that file re-check them against live chain state:

```bash
RPC_URL=https://my-private-rpc cargo test --test swap_math -- --ignored
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
    true, // track_volume
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
client.simulate_buy(&pool_info, LAMPORTS_PER_SOL / 1_000, &payer).await?; // 0.001 SOL
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

### Mayhem mode and old pools

`load_pool` records the pool's `is_mayhem_mode` flag and raw account data
length. Swap builders use reserved fee recipients for Mayhem pools, and
`build_buy_ixs`, `build_buy_exact_quote_in_ixs`, `build_sell_ixs`, and
`deposit_into_wsol_pool` prepend `extend_account` when a loaded pool account is
smaller than the current pump-amm allocation.

## API overview

Generate local API docs with:

```sh
cargo doc --open
```

- `PumpSwapClient`: high-level RPC client wrapper. Exposes `buy`, `sell`,
  `buy_exact_quote_in`, `simulate_*`, `deposit_into_wsol_pool`,
  `withdraw_from_wsol_pool`, `claim_cashback`, `withdraw_creator_fees`, and
  `create_wsol_pool` convenience methods plus their `build_*_ixs` counterparts.
- `make_buy_instruction`, `make_buy_exact_quote_in_instruction`,
  `make_sell_instruction`: raw swap instruction builders. The buy variants
  take a `track_volume: bool` that accrues cashback eligibility on the
  caller's `user_volume_accumulator` PDA.
- `make_deposit_instruction`, `withdraw_instruction`: LP-side builders.
- `make_claim_cashback_instruction`: pulls accrued cashback for a user from
  their volume-accumulator PDA.
- `make_claim_token_incentives_instruction`,
  `make_init_user_volume_accumulator_instruction`,
  `make_sync_user_volume_accumulator_instruction`,
  `make_close_user_volume_accumulator_instruction`,
  `make_extend_account_instruction`,
  `make_collect_coin_creator_fee_instruction`: current auxiliary pump-amm
  instruction builders.
- `create_pool_instruction`,
  `create_pool_instruction_with_options`,
  `transfer_creator_fees_to_pump_instruction`,
  `distribute_creator_fees_instruction`: additional pump-amm and pump.fun
  instruction builders.
- `load_pool`, `load_pool_with_token_program`: pool account decoding helpers.
- `GlobalConfig`, `FeeConfig`, `Fees`, `FeeTier`: decoded on-chain fee state.
  `GlobalConfig::from_account_data` and `FeeConfig::from_account_data` decode
  raw account bytes; `PumpSwapClient::fetch_global_config`,
  `fetch_fee_config`, and `fetch_fee_state` fetch and decode them.
  `FeeConfig::fee_tier_for_market_cap` (and its `stable_` counterpart) picks
  the applicable tier, and `Fees::total_bps()` sums the lp/protocol/creator
  split.
- `UserVolumeAccumulator`, `GlobalVolumeAccumulator`: decoded volume-accumulator
  state. `PumpSwapClient::fetch_user_volume_accumulator` returns `None` for a
  user who has never traded (no PDA) rather than an error;
  `fetch_global_volume_accumulator` reads the program-wide account. Note that
  `cashback_earned` and `total_cashback_claimed` are independent counters and
  do not subtract into a claimable balance — see the type docs.
- `SharingConfig`, `Shareholder`, `ConfigStatus`: a coin's creator-fee split,
  owned by the fee program at `sharing_config_pda(mint)`
  (`["sharing-config", mint]`). `SharingConfig::from_account_data` decodes raw
  bytes; `PumpSwapClient::fetch_sharing_config` fetches one, returning
  `Ok(None)` for a coin with no split. Decoding only — the fee-collection
  builders do not yet route through a split.
- `calc_amount_out`, `buy_amount_out`, `sell_amount_out`: quote math helpers.
- PDA helpers such as `calc_pool_pda`, `calc_lp_mint_pda`,
  `find_coin_creator_vault_authority`, `find_coin_creator_vault_ata`,
  `find_user_vol_accumulator`, `fee_config_pda`, and `sharing_config_pda`.
- `JitoPool`, `send_jito_bundle`, `send_bundle_with_retry`: Jito bundle
  helpers.

## Safety

This SDK can build and submit mainnet transactions. Treat every example that
uses a keypair as capable of spending real funds.

- This project is not audited.
- This project is not an official Pump.fun product.
- Simulate transactions before sending them.
- Do not commit `.env` files, keypair JSON, or base58 secret keys.
- The convenience helpers apply `DEFAULT_SLIPPAGE` (0.5%) on a fee-aware
  quote. They used 1–10% only because the quote ignored the pool's fee.
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

Tests that hit mainnet RPC are `#[ignore]`d so CI never depends on RPC
availability. Run them explicitly — against a private endpoint when iterating,
since `api.mainnet-beta.solana.com` rate-limits to HTTP 429 quickly:

```sh
RPC_URL=https://my-private-rpc cargo test -- --ignored
```

`tests/fee_state.rs` keeps byte-for-byte mainnet fixtures of `GlobalConfig` and
`FeeConfig` under `tests/fixtures/`, so the layout assertions run offline. Its
`#[ignore]`d live tests re-run them against chain state and double as a drift
alarm for the hardcoded fee-recipient tables in `constants.rs`.

`tests/volume_accumulator.rs` does the same for the `UserVolumeAccumulator` and
`GlobalVolumeAccumulator` accounts.

## License

MIT
