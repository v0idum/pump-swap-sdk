# Changelog

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
