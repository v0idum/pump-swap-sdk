use solana_sdk::pubkey;
use solana_sdk::pubkey::Pubkey;

/// pump-amm AMM program (PumpSwap).
pub const PUMP_SWAP_PROGRAM_ID: Pubkey = pubkey!("pAMMBay6oceH9fJKBRHGP5D4bD4sWpmSwMn52FMfXEA");

/// pump.fun bonding-curve program — used for creator-fee distribution.
pub const PUMPFUN_PROGRAM: Pubkey = pubkey!("6EF8rrecthR5Dkzon8Nwu78hRvfCKubJ14M5uBEwF6P");

/// pump.fun event-authority PDA — used by the bonding-curve program's
/// `distribute_creator_fees` instruction.
pub const PUMPFUN_EVENT_AUTHORITY: Pubkey = pubkey!("Ce6TQqeHC9p8KetsN6JsjHK7UTZk7nasjjnr7XxXp9F1");

/// pump.fun creator-vault account that receives accrued creator fees from
/// the pump-amm `transfer_creator_fees_to_pump` instruction before they're
/// distributed via `distribute_creator_fees`.
pub const PUMP_CREATOR_VAULT: Pubkey = pubkey!("8CoWk2ZYjsBZEy8yLqWEK9mtZ8tkAbAFrbmtyyuhEqGg");

/// pump fee program — owns fee_config and exposes `get_fees`.
pub const FEE_PROGRAM: Pubkey = pubkey!("pfeeUxB6jkeY1Hxd7CsFCAjcbHA9rWtchMGdZ6VojVZ");

/// pump-amm event-authority PDA (`["__event_authority"]` under pump-amm).
pub const EVENT_AUTHORITY: Pubkey = pubkey!("GS4CU59F31iL7aR2Q8zVS8DRrcRnXX1yjQ66TqNVQnaR");

/// pump-amm GlobalConfig account.
pub const GLOBAL_CONFIG: Pubkey = pubkey!("ADyA8hdefvWN2dbGGWFotbzWxrAvLW83WG6QCVXvJKqw");

/// Size in bytes of a current pool account, Anchor discriminator included.
///
/// Live mainnet pools allocate 301 bytes: an 8-byte discriminator, the
/// 237-byte [`Pool`](crate::Pool) struct, the 8-byte undocumented
/// `virtual_quote_reserves` that follows it, and trailing reserved space.
/// Pools allocated before the layout was extended are shorter — the
/// `pool_legacy_short.bin` fixture is a real 271-byte one — and the
/// high-level build helpers prepend `extend_account` for them.
///
/// This is a threshold, not a decode bound:
/// [`PoolInfo::from_account_data`](crate::PoolInfo::from_account_data) reads
/// fields at fixed offsets and accepts accounts on either side of it.
pub const POOL_ACCOUNT_NEW_SIZE: usize = 301;

/// Global volume accumulator account (PDA `["global_volume_accumulator"]` under
/// pump-amm; address is stable, pinned as a constant for convenience).
pub const GLOBAL_VOLUME_ACCUMULATOR: Pubkey =
    pubkey!("C2aFPdENg4A2HQsmrd5rTw5TaYBX5Ku887cWjbFKtZpw");

/// Wrapped-SOL native mint.
pub const WRAPPED_SOL_MINT: Pubkey = pubkey!("So11111111111111111111111111111111111111112");

/// pump-amm GlobalConfig.protocol_fee_recipients (size 8). The pool's swap
/// program accepts any of these; the SDK selects one per call.
pub const PROTOCOL_FEE_RECIPIENTS: [Pubkey; 8] = [
    pubkey!("62qc2CNXwrYqQScmEdiZFFAnJR262PxWEuNQtxfafNgV"),
    pubkey!("7VtfL8fvgNfhz17qKRMjzQEXgbdpnHHHQRh54R9jP2RJ"),
    pubkey!("7hTckgnGnLQR6sdH7YkqFTAA7VwTfYFaZ6EhEsU3saCX"),
    pubkey!("9rPYyANsfQZw3DnDmKE3YCQF5E8oD89UXoHn9JFEhJUz"),
    pubkey!("AVmoTthdrX6tKt4nDjco2D775W2YK3sDhxPcMmzUAmTY"),
    pubkey!("FWsW1xNtWscwNmKv6wVsU1iTzRN6wmmk3MjxRP5tT7hz"),
    pubkey!("G5UZAVbAf46s7cKWoyKu8kYTip9DGTpbLZ2qa9Aq69dP"),
    pubkey!("JCRGumoE9Qi5BBgULTgdgTLjSgkCMSbF62ZZfGs84JeU"),
];

/// pump-amm GlobalConfig reserved fee recipients used for Mayhem-mode pools.
///
/// The live config stores one `reserved_fee_recipient` plus seven
/// `reserved_fee_recipients`; the program accepts any of these when the pool
/// is in Mayhem mode.
pub const RESERVED_FEE_RECIPIENTS: [Pubkey; 8] = [
    pubkey!("GesfTA3X2arioaHp8bbKdjG9vJtskViWACZoYvxp4twS"),
    pubkey!("4budycTjhs9fD6xw62VBducVTNgMgJJ5BgtKq7mAZwn6"),
    pubkey!("8SBKzEQU4nLSzcwF4a74F2iaUDQyTfjGndn6qUWBnrpR"),
    pubkey!("4UQeTP1T39KZ9Sfxzo3WR5skgsaP6NZa87BAkuazLEKH"),
    pubkey!("8sNeir4QsLsJdYpc9RZacohhK1Y5FLU3nC5LXgYB4aa6"),
    pubkey!("Fh9HmeLNUMVCvejxCtCL2DbYaRyBFVJ5xrWkLnMH6fdk"),
    pubkey!("463MEnMeGyJekNZFQSTUABBEbLnvMTALbT6ZmsxAbAdq"),
    pubkey!("6AUH3WEHucYZyC61hqpqYUWVto5qA5hjHuNQ32GNnNxA"),
];

/// pump-amm GlobalConfig.buyback_fee_recipients (size 8). PDAs owned by the
/// fee_program. The SDK appends one of these (and its quote ATA) as
/// `remaining_accounts` on every Buy/Sell.
pub const BUYBACK_FEE_RECIPIENTS: [Pubkey; 8] = [
    pubkey!("5YxQFdt3Tr9zJLvkFccqXVUwhdTWJQc1fFg2YPbxvxeD"),
    pubkey!("9M4giFFMxmFGXtc3feFzRai56WbBqehoSeRE5GK7gf7"),
    pubkey!("GXPFM2caqTtQYC2cJ5yJRi9VDkpsYZXzYdwYpGnLmtDL"),
    pubkey!("3BpXnfJaUTiwXnJNe7Ej1rcbzqTTQUvLShZaWazebsVR"),
    pubkey!("5cjcW9wExnJJiqgLjq7DEG75Pm6JBgE1hNv4B2vHXUW6"),
    pubkey!("EHAAiTxcdDwQ3U4bU6YcMsQGaekdzLS3B5SmYo46kJtL"),
    pubkey!("5eHhjP8JaYkz83CWwvGU2uMUXefd3AazWGx4gpcuEEYD"),
    pubkey!("A7hAgCzFw14fejgCp387JUJRMNyz4j89JKnhtKU8piqW"),
];
