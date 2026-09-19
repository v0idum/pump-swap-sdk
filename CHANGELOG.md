# Changelog

## Unreleased

### Added

- **Volume-accumulator read API.** `UserVolumeAccumulator` and
  `GlobalVolumeAccumulator` deserializers in `state.rs`, plus
  `PumpSwapClient::fetch_user_volume_accumulator` and
  `PumpSwapClient::fetch_global_volume_accumulator`. The SDK could already
  build the instructions that maintain these accounts
  (`init_user_volume_accumulator`, `sync_user_volume_accumulator`,
  `close_user_volume_accumulator`, `claim_cashback`,
  `claim_token_incentives`) but could not read the resulting state, so
  "how much cashback have I earned?" needed hand-rolled account parsing.

  `fetch_user_volume_accumulator` returns `Option<UserVolumeAccumulator>`:
  a user who has never traded has no PDA, and one closed through
  `close_user_volume_accumulator` no longer has one. Neither is an error.
  `UserVolumeAccumulator::empty(user)` is the all-zero substitute for
  callers that would rather branch on the numbers than on the `Option`.

  Both decoders reuse the bounds-checked `Reader` and validate the account
  discriminator, so a truncated account or the wrong account type fails
  loudly instead of decoding into plausible-looking garbage.
  `GlobalVolumeAccumulator::day_index` maps a timestamp onto the 30 day
  buckets, and `VOLUME_ACCUMULATOR_DAYS` is the bucket count.

- `tests/volume_accumulator.rs` with mainnet account fixtures for both
  accounts under `tests/fixtures/`, so the layout assertions run offline;
  the live-RPC checks are `#[ignore]`d.

  The token-incentive half of both accounts is dormant on mainnet as of
  2026-09-20: the global accumulator is zeroed past its discriminator, and
  every user accumulator sampled has zero token counters. Cashback is not
  dormant — of 8,319,867 live `UserVolumeAccumulator` accounts, 1,255,705
  carry non-zero cashback counters. The user fixture is one of them, taken
  from a real `claim_cashback` transaction; the global fixture records the
  zeroed account as it stands, and the `#[ignore]`d tests are the alarm for
  the day that changes.
- **`SharingConfig` decoding: a coin's creator fees split across several
  addresses by basis points.** New `SharingConfig`, `Shareholder` and
  `ConfigStatus` types with a `SharingConfig::from_account_data` decoder,
  `util::sharing_config_pda` for the address, and
  `PumpSwapClient::fetch_sharing_config`, which returns `Ok(None)` for the
  common case of a coin with no split.

  The account is owned by the fee program, not pump-amm, and lives at
  `["sharing-config", mint]` under it — note the hyphen, where `fee_config`
  uses an underscore. pump-amm declares it in its own IDL because
  `migrate_pool_coin_creator` reads it: that instruction takes the pool and
  this config as its only non-fixed accounts and repoints the pool's
  `coin_creator` at the config, which is also what the pump-amm error set
  describes (`CoinCreatorMigratedToSharingConfig`,
  `CreatorVaultMigratedToSharingConfig`). 664_802 of these accounts were live
  on mainnet at slot 448_520_505 (2026-09-20), across both `ConfigStatus`
  variants.

  The account is allocated at a fixed 1024 bytes and a shrunk shareholder list
  leaves its old bytes in place, so the decoder stops at the end of the
  `shareholders` vector and ignores the remainder. Covered by two byte-for-byte
  mainnet fixtures under `tests/fixtures/sharing_configs/` — a four-way active
  split and a paused config with a revoked admin and stale trailing bytes —
  plus `#[ignore]`d tests against live chain state.

  This release decodes the account only; routing creator fees through a split
  when `pool.coin_creator` is a `SharingConfig` PDA is not yet wired into the
  fee-collection builders.

### Changed

- **Dependencies brought up to their latest stable majors.** `solana-client`
  2.1.5 → 4.3.0, `solana-sdk` 2.1.5 → 4.1.0, `spl-token` 7 → 9,
  `spl-token-2022` 7 → 11, `spl-associated-token-account` 6 → 8, `bincode`
  1.3.3 → 2.0.1, `base64` 0.21.7 → 0.23.1, `rand` 0.9 → 0.10.2, `bytemuck`
  1.20 → 1.25.2. `jito-sdk-rust` was already current at 0.3.2.

  The `solana-sdk` 2.x line has since been split into granular `solana-*`
  crates, and four items this SDK uses left the monolith. They are now direct
  dependencies: `ComputeBudgetInstruction` from `solana-compute-budget-interface`,
  and `system_instruction` / `system_program` from `solana-system-interface`.
  `CommitmentConfig` (`solana-commitment-config`) and `LAMPORTS_PER_SOL`
  (`solana-native-token`) moved out too; both are only used by the examples and
  tests, so they are dev-dependencies.

- **MSRV raised from 1.85 to 1.97.1.** Required by the `solana-client` 4.3.0
  stack, which declares `rust-version = "1.97.1"` across ~40 crates. This is the
  highest MSRV in the resolved graph; the build and full test suite are verified
  against exactly that toolchain. `edition` stays at `2024`.

- **`send_jito_bundle` and the system-instruction decode moved to the `bincode`
  2 API.** `bincode::serialize` / `deserialize` became
  `bincode::serde::encode_to_vec` / `decode_from_slice` with
  `bincode::config::legacy()`, which is the configuration that reproduces
  bincode 1.3's format (little-endian, fixed-int). Verified byte-for-byte: a
  signed `Transaction` encodes to the same 260 bytes under bincode 1.3.3 and
  bincode 2.0.1 `legacy()`. `tests/wire_format.rs` pins that encoding to a
  golden vector captured from bincode 1.3.3 so a future bump cannot move it
  silently.

- Example and README amounts that used `solana_sdk::native_token::sol_to_lamports`
  now derive from `LAMPORTS_PER_SOL` or parse with `sol_str_to_lamports`.
  `solana-native-token` 3.0 dropped the lossy `f64` converters
  (`sol_to_lamports` / `lamports_to_sol`). The constants are unchanged:
  `sol_to_lamports(0.001)` was exactly `1_000_000`.

### Held back

- **`bincode` stays on 2.0.1, not 3.0.0.** bincode 3.0.0 is not a usable
  release — its entire source is `compile_error!("https://xkcd.com/2347/")`.
  2.0.1 is the latest version that builds.

- **`solana-sdk` stays on 4.1.0, not 5.0.0.** `solana-sdk` 5.0.0 depends on
  `solana-transaction` 5.x and `solana-message` 5.x, while the latest stable
  `solana-client` (4.3.0) is built on the 4.x line. Pairing them puts two
  incompatible `Transaction` types in the graph and
  `solana_sdk::transaction::Transaction` stops satisfying the
  `SerializableTransaction` bound every `send_*` / `simulate_*` call needs.
  There is no stable `solana-client` 5.x yet (only `4.4.0-alpha.5`).
  `solana-sdk` 4.1.0 is the newest release whose granular dependencies unify
  with `solana-client` 4.3.0.

### Fixed

- `Cargo.lock` pins `five8_core` to 1.0.0. `five8` 1.0.0 requests
  `five8_core >=0.1.1, <2` and cargo would otherwise select 0.1.2, whose
  `DecodeError` predates the `core::error::Error` impl that
  `solana-keypair` 3.1.2 requires — the dependency graph does not compile
  without the pin.

- **Two advisories cleared out of `Cargo.lock`.** Surfaced by the new
  `cargo audit` job, both semver-compatible and both transitive:
  `crossbeam-epoch` 0.9.18 -> 0.9.21 (RUSTSEC-2026-0204, invalid pointer
  dereference in the `fmt::Pointer` impl for `Atomic` / `Shared`; reached via
  `rayon` under `solana-streamer`) and `time` 0.3.41 -> 0.3.55
  (RUSTSEC-2026-0009, denial of service via stack exhaustion; reached via
  `x509-parser` under `solana-tls-utils`). No manifest requirement changed and
  the resolved versions are still MSRV-compatible.

- **`POOL_ACCOUNT_NEW_SIZE` corrected from 300 to 301.** Live mainnet pool
  accounts allocate 301 bytes, not 300 — the three current-layout fixtures in
  `tests/fixtures/pools/` are all exactly that long. The `extend_account` gate
  compares `pool_account_data_len < POOL_ACCOUNT_NEW_SIZE`, so the wrong value
  only mattered for a pool at exactly 300 bytes: it was classified as
  current-layout and left un-extended. No account of that length is known on
  mainnet, so no caller-visible behaviour changes. `src/client.rs` gained a
  test module pinning the gate one byte below the boundary and at it, plus the
  same decision against the real 271-byte legacy and 301-byte current
  fixtures.

  The pool decode path is untouched. `PoolInfo::from_account_data` reads
  `virtual_quote_reserves` at `size_of::<Pool>() + 8` = 245, an offset derived
  from the `Pool` struct and not from this constant, and still accepts
  accounts on either side of it; `tests/wire_format.rs` continues to pin
  `size_of::<Pool>() == 237`.

### Added

- **CI runs `cargo audit`.** A dedicated `audit` job installs `cargo-audit`
  and audits `Cargo.lock` against the RustSec advisory database, so a
  vulnerable transitive dependency fails the build instead of going unnoticed.

  It runs with `--ignore RUSTSEC-2026-0258` (h2 0.3.26, unbounded empty DATA
  frames). That one is patched in h2 0.4.16 and reaches the graph only via
  `jito-sdk-rust` 0.3.2 -> `reqwest` 0.11.27 -> `hyper` 0.14 -> `h2` 0.3, so
  it cannot be resolved here without `jito-sdk-rust` moving to `reqwest` 0.12.
  The advisory concerns an HTTP/2 endpoint accepting inbound connections; this
  SDK uses `reqwest` only as a client to POST Jito bundles and serves nothing.
  The flag carries the same note in `ci.yml` and should be dropped when
  `jito-sdk-rust` ships a `reqwest` 0.12 release.

- **CI verifies the declared MSRV.** A new `msrv` job reads `rust-version`
  out of `Cargo.toml` with `cargo metadata` instead of hardcoding it, installs
  that exact toolchain, and runs `cargo build --all-targets --locked` and
  `cargo test --all-targets --locked`. The MSRV raised to 1.97.1 above is now
  tested rather than asserted, and the declared value cannot drift away from
  the toolchain it is checked against.

### Removed

- `build.log` — two stray lines of `cargo run` output from an old
  `verify_layout` run — is no longer committed, and `.gitignore` gained a
  `*.log` rule so it does not come back.

## 0.5.0 - 2026-09-20

### Fixed

- **Swap quotes are fee-aware.** `src/math.rs` priced swaps as pure constant
  product with no fee accounting, so every quote was too optimistic by the
  pool's fee and callers passing a realistic slippage got a `min_out` the
  program rejected with `ExceededSlippage` (`Custom(6004)`). A 0.001 SOL buy
  on `2TrDYCA4kEKTGXs622pENzQADGydndXM8zKdDn3x3DeV` needed 0.3% slippage to
  pass; it now passes at 0% (verified on mainnet, 2026-09-19). `slippage` is a
  slippage budget again.
- **Pool decoding reads the pool's virtual quote liquidity.** Pool accounts
  carry a `u64` immediately after `is_cashback_coin` that is absent from the
  published IDL. pump-amm adds it to the quote token-account balance both when
  stepping the constant product and when computing market cap for fee-tier
  selection. Pools carrying it were mispriced by orders of magnitude — on one
  live pool, by 52x. Exposed as `PoolInfo::virtual_quote_reserves` and applied
  by `PoolInfo::effective_reserves`.
- Corrected two claims carried over from the fee-state work: pump-amm does not
  call the IDL's `get_fees`, it CPIs `GetFeesWithQuoteMint`, which takes
  `(is_pump_pool, market_cap_lamports, quote_mint)` and **no trade size**. The
  flag tracks the pool's `coin_creator`, not its quote mint, which is how a
  pump pool quoted in another token still lands on the ladder; and
  `FeeConfig::flat_fees` is the schedule for pools the program does not price
  off the ladder, not a fallback for "no market-cap tier matched". The old
  fallback erred 95 bps in the direction that makes swaps revert, and the
  `fetch_fee_state` rustdoc no longer demonstrates it.

### Known limitation

- A ladder-priced pool quoted in something other than SOL (about 2% of the
  pools observed) is charged a schedule matching neither published ladder —
  live swaps show 30 to 325 bps against a ladder that tops out at 125.
  `fetch_pool_fees` returns an error for those rather than quoting low, and
  `can_quote_fees` reports it up front. Such pools are outside the trader API
  anyway, whose `sol_in` / `min_sol_out` assume one side of the pair is WSOL.

### Changed

- **Breaking:** `buy_amount_out` and `sell_amount_out` take a `Fees` argument
  and are now oriented around trader intent (SOL in → tokens, tokens in → SOL),
  matching `build_token_buy_ixs` / `build_token_sell_ixs`. `calc_amount_out`
  keeps its signature and its pure-curve behaviour, now documented as not
  pricing a real swap.
- **Breaking:** `PoolInfo` gained a `virtual_quote_reserves` field, so struct
  literals need updating. `PoolInfo::from_account_data` is the supported way to
  build one from account bytes.
- The client's convenience methods (`buy`, `sell`, `simulate_buy`,
  `simulate_sell`) now quote fees explicitly and share `DEFAULT_SLIPPAGE`
  (0.5%). They previously used 1%, 5% and 10% to absorb an unaccounted fee —
  and 1% was below the 125 bps charged on low-market-cap pools, so buys there
  failed outright.
- `examples/sim_trade_flow.rs` prices both legs with the fee-aware quotes and
  drops its slippage from 20% to 0.1%. Its sell leg now creates the token ATA
  first, so a clean account layout surfaces as insufficient funds inside the
  token transfer rather than a missing account.

### Added

- `quote_sell`, `quote_buy_exact_quote_in` and `quote_buy_exact_base_out`,
  reproducing pump-amm's arithmetic exactly — integer constant product,
  per-component rounded-up fees, and the program's own off-by-one on the
  exact-quote-in curve input. `token_buy_quote` / `token_sell_quote` wrap them
  for either pool orientation, returning a `SwapQuote` that carries the fee,
  the expected fill and the `min_out` to pass.
- `PumpSwapClient::quote_token_buy` / `quote_token_sell` (live quote in one
  call), `fetch_pool_fees` (the pool's tiered schedule in one
  `getMultipleAccounts`) and `fees_from_state` for callers that already hold
  the fee config.
- `FeeConfig::fees_for_pool`, mirroring the fee program's
  `GetFeesWithQuoteMint`; `Fees::fee_on`; `market_cap_lamports`;
  `constant_product_out`; `is_tiered_fee_pool`; `can_quote_fees`.
- `PoolInfo::from_account_data`, so a pool can be decoded from raw bytes
  without an RPC round trip. `load_pool` now goes through it.
- `tests/swap_math.rs` with mainnet swap-event and pool-account fixtures, so
  the pricing rules are asserted byte-for-byte offline, plus `#[ignore]`d live
  simulations covering a flat-fee pool and a 125 bps base-tier pool.

- `GlobalConfig`, `FeeConfig`, `Fees`, and `FeeTier` types with deserializers
  matching the live pump-amm and fee-program IDLs (verified on mainnet
  2026-09-19). `GlobalConfig` mirrors the on-chain one
  `reserved_fee_recipient` plus seven `reserved_fee_recipients` split, with
  `all_reserved_fee_recipients()` for the flattened view used by
  `RESERVED_FEE_RECIPIENTS`. `FeeTier::market_cap_lamports_threshold` is a
  `u128`, as on chain.
- `PumpSwapClient::fetch_global_config`, `fetch_fee_config`, and
  `fetch_fee_state` (both accounts in one `getMultipleAccounts` call).
- `FeeConfig::fee_tier_for_market_cap` / `stable_fee_tier_for_market_cap` and
  `Fees::total_bps()`, so callers can price a swap's fee without a CPI to the
  fee program's `get_fees`.
- `tests/fee_state.rs` with mainnet account fixtures for offline layout
  assertions, plus `#[ignore]`d live tests that act as a drift alarm for the
  hardcoded fee-recipient tables in `constants.rs`.

### Compatibility

- **Minor bump, not a patch: this release breaks source compatibility.** Two
  changes need code edits. `buy_amount_out` / `sell_amount_out` take a `Fees`
  argument and are oriented around trader intent rather than base/quote, and
  `PoolInfo` gained the `virtual_quote_reserves` field, so struct literals
  must be updated — `PoolInfo::from_account_data` is the supported
  constructor.
- **Quotes change value, by design.** Anything comparing a `min_out` against a
  previously recorded number will see a different figure: the old one was too
  optimistic by the pool's fee. Callers passing a slippage wide enough to
  absorb the fee (5% or more) can narrow it; `DEFAULT_SLIPPAGE` is 0.5%.
- `calc_amount_out` keeps its signature and its pure-curve behaviour, so
  existing call sites compile unchanged — but it does not price a real swap
  and should be replaced with `quote_token_buy` / `quote_token_sell`.
- Unchanged: instruction layouts, account orders, PDA derivations, and every
  instruction builder. This release does not alter a single byte sent on
  chain.
- MSRV stays at 1.85.

## 0.4.0 - 2026-07-11

### Fixed

- **User swap accounts are now oriented per pool layout.** Pools created by
  current pump.fun graduations store WSOL on the **base** side (~85% of live
  PumpSwap volume); `build_buy_ixs`, `build_buy_exact_quote_in_ixs`, and
  `build_sell_ixs` previously passed the user token ATA / WSOL account in
  fixed (base, quote) order and funded the ephemeral WSOL account on the wrong
  side, so every swap on a SOL-base pool failed with a token-mint mismatch
  (`Custom(3)`). The builders now map accounts and WSOL funding from the pool's
  actual mint orientation.

### Added

- Orientation-aware, trader-intent API: `build_token_buy_ixs` ("spend exactly
  N lamports of SOL, receive >= M tokens") and `build_token_sell_ixs` ("spend
  exactly N tokens, receive >= M lamports"), selecting the correct pump-amm
  instruction per pool orientation (`buy_exact_quote_in`/`sell` on token-base
  pools, `sell`/`buy_exact_quote_in` on SOL-base pools).
- `PoolInfo::sol_is_base()`, `token_side()`, `token_mint()`, `token_program()`,
  and `orient_reserves()` plus the `TokenSide` enum.
- `examples/sim_trade_flow.rs`: verifies a full token buy + sell against the
  live program via `simulateTransaction` on either pool orientation (no keys
  required).

### Compatibility

- Existing method signatures are unchanged; behavior on token-base (canonical)
  pools is identical. On SOL-base pools the legacy builders now produce
  *working* instructions (previously guaranteed-failing), including the
  corrected WSOL funding side.

## 0.3.0 - 2026-06-11

### Added

- Current PumpSwap `create_pool` arguments: pool `index`, `is_mayhem_mode`, and
  `is_cashback_coin`.
- Indexed pool PDA helper, while keeping index `0` as the default for existing
  callers.
- Mayhem-mode reserved fee recipient selection.
- Automatic `extend_account` prepending for older pool accounts in high-level
  swap and deposit builders.
- Builders for user volume accumulator maintenance, token incentive claims, and
  direct coin creator fee collection.

### Changed

- `PoolInfo` now exposes the loaded pool account data length and Mayhem flag so
  builders can choose the correct protocol accounts.
- `CreatePoolInstruction` serializes the current 60-byte PumpSwap layout.

### Compatibility

- Existing constructor and high-level client methods keep their previous default
  behavior.
- Apps that construct `PoolInfo` or `CreatePoolInstruction` with struct literals
  must add the new public fields.
