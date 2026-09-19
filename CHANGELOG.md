# Changelog

## Unreleased

### Added

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
