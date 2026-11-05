/// Seeds for the market account: [MARKET_SEED, mint].
pub const MARKET_SEED: &[u8] = b"market";
/// Seeds for the SOL vault: [VAULT_SEED, market].
pub const VAULT_SEED: &[u8] = b"vault";
/// Seeds for the token account holding the locked seed tokens: [LOCKED_SEED, market].
pub const LOCKED_SEED: &[u8] = b"locked";

/// Layout version of the `Market` account.
pub const MARKET_VERSION: u8 = 1;

/// The token always has 6 decimals.
pub const DECIMALS: u8 = 6;

// Hard limits, fixed in code, so no deployment can turn the token into a trap.

/// Fee kept for holders, on each side. Every trade pays at least 0.1%, so
/// every trade lifts the price, and at most 5%, so getting out stays cheap.
pub const MIN_FEE_BPS: u16 = 10;
pub const MAX_BUY_FEE_BPS: u16 = 500;
pub const MAX_SELL_FEE_BPS: u16 = 500;
/// Creator share of each trade: at most 2%.
pub const MAX_TEAM_FEE_BPS: u16 = 200;

/// Minimum seed: 0.01 SOL and 1,000,000 base units of the token.
pub const MIN_SEED_LAMPORTS: u64 = 10_000_000;
pub const MIN_SEED_TOKENS: u64 = 1_000_000;
/// The starting price may not be absurdly low: at most 1,000 token base units
/// per seed lamport. With 6 decimals that is 0.000001 SOL per token. The price
/// only rises from there, so token amounts stay far below the u64 limit.
pub const MAX_SEED_TOKENS_PER_LAMPORT: u64 = 1_000;
