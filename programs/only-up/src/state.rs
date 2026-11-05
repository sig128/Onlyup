use anchor_lang::prelude::*;

/// One market per ONLY UP mint. PDA: [MARKET_SEED, mint].
#[account]
#[derive(InitSpace, Debug)]
pub struct Market {
    /// Layout version, for future migrations.
    pub version: u8,
    pub bump: u8,
    pub vault_bump: u8,
    pub locked_bump: u8,
    /// The ONLY UP token mint. Its mint authority is this account.
    pub mint: Pubkey,
    /// Can only pause buys and change the treasury. Zero once renounced.
    pub admin: Pubkey,
    /// Receives the creator's fee share through `claim_team_fees`.
    pub treasury: Pubkey,
    /// Lamports backing the price. Price = reserve / token supply.
    pub reserve: u64,
    /// Creator fees waiting in the vault. Not part of the reserve.
    pub team_fees_owed: u64,
    /// Fee kept for holders on every buy, in basis points. Goes straight
    /// into the reserve, so the price rises the instant a trade lands.
    pub buy_fee_bps: u16,
    /// Fee kept for holders on every sell, in basis points.
    pub sell_fee_bps: u16,
    /// Creator share of every buy and sell, in basis points.
    pub team_fee_bps: u16,
    pub buys_paused: bool,
    pub created_at: i64,
    pub trade_count: u64,
    pub volume_lamports: u64,
}
