use anchor_lang::prelude::*;

/// One market per ONLY UP mint. PDA: [MARKET_SEED, mint].
///
/// The market is a constant-product curve (curve_sol against a virtual token
/// reserve) with a separate, unpierceable floor reserve. Circulating supply is
/// read from the mint itself; the virtual token reserve is `total_supply - supply`.
#[account]
#[derive(InitSpace, Debug)]
pub struct Market {
    /// Layout version, for future migrations.
    pub version: u8,
    pub bump: u8,
    pub vault_bump: u8,
    /// The ONLY UP token mint. The program is its mint authority.
    pub mint: Pubkey,
    /// Can only pause buys and change the treasury. Zero once renounced.
    pub admin: Pubkey,
    /// Receives the creator's fee share through `claim_creator_fees`.
    pub treasury: Pubkey,
    /// Real SOL in the constant-product curve reserve (lamports). The curve
    /// price is curve_sol / (total_supply - supply).
    pub curve_sol: u64,
    /// Real SOL in the floor reserve (lamports). Any holder can always redeem a
    /// token for floor_reserve / supply. This ratio can only ever rise.
    pub floor_reserve: u64,
    /// Virtual token reserve at launch, and the asymptotic supply cap. The
    /// constant-product token side is `total_supply - supply`, always > 0.
    pub total_supply: u64,
    /// Creator fees waiting in the vault. Not part of either reserve.
    pub creator_fees_owed: u64,
    pub buys_paused: bool,
    pub created_at: i64,
    pub trade_count: u64,
    pub volume_lamports: u64,
}
