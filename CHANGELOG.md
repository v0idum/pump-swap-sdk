# Changelog

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
