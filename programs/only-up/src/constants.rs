/// Seeds for the market account: [MARKET_SEED, mint].
pub const MARKET_SEED: &[u8] = b"market";
/// Seeds for the SOL vault: [VAULT_SEED, market]. Holds the curve reserve, the
/// floor reserve and unclaimed creator fees. Only the program can sign for it.
pub const VAULT_SEED: &[u8] = b"vault";

/// Layout version of the `Market` account.
pub const MARKET_VERSION: u8 = 2;

/// The token always has 6 decimals.
pub const DECIMALS: u8 = 6;

/// Basis-point denominator.
pub const BPS: u128 = 10_000;

/// Creator fee taken from every buy and every sell, in basis points. Fixed in
/// code at 0.5% so no deployment can turn it into a honeypot.
pub const CREATOR_FEE_BPS: u128 = 50;

/// Floor fee taken from every buy and every sell, in basis points. Fixed at
/// 2.5%: it goes straight into the floor reserve, so holders earn on every
/// trade. Together with the creator fee this is a flat 3% per side.
pub const FLOOR_FEE_BPS: u128 = 250;

/// Share of each buy (after the 3% fee) routed into the floor reserve, in
/// basis points. Fixed at 40% — the "dial" that trades curve upside for a
/// stronger, faster-rising floor. This is on top of the 2.5% floor fee.
pub const FLOOR_BPS: u128 = 4_000;

// ---- launch bounds, fixed in code ----

/// Minimum curve seed: 0.02 SOL of real liquidity.
pub const MIN_SEED_LAMPORTS: u64 = 20_000_000;
/// Minimum / maximum virtual token reserve (also the asymptotic supply cap).
/// 1e6 .. 1e18 base units (with 6 decimals: 1 .. 1,000,000,000,000 whole tokens).
pub const MIN_SEED_TOKENS: u64 = 1_000_000;
pub const MAX_SEED_TOKENS: u64 = 1_000_000_000_000_000_000;
/// The starting curve price may not be absurdly low: at most this many token
/// base units per seed lamport. Keeps token amounts far below the u64 ceiling.
pub const MAX_SEED_TOKENS_PER_LAMPORT: u64 = 1_000_000;
