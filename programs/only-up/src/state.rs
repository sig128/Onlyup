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
    /// Receives the team's fee share through `claim_team_fees`.
    pub treasury: Pubkey,
    /// Lamports backing the price. Price = reserve / token supply.
    pub reserve: u64,
    /// Fees paid but not yet dripped into the reserve.
    pub pending: u64,
    /// Team fees waiting in the vault. Not part of the reserve or pending.
    pub team_fees_owed: u64,
    /// Decaying sum of recent buy volume, in lamports. Drives the hype fee,
    /// which applies to buys only.
    pub heat: u64,
    pub heat_updated_at: i64,
    /// When pending fees last dripped into the reserve.
    pub last_drip_at: i64,
    /// Fee kept for holders on every buy, in basis points.
    pub buy_fee_bps: u16,
    /// Fee kept for holders on every sell, in basis points.
    pub sell_fee_bps: u16,
    /// Team share of every buy and sell, in basis points.
    pub team_fee_bps: u16,
    /// Hype fee ceiling, in basis points.
    pub max_hype_bps: u16,
    /// Heat, as a share of the reserve in basis points, that reaches the ceiling.
    pub hype_full_bps: u16,
    /// Seconds for the heat to halve.
    pub hype_half_life_secs: u32,
    /// Seconds for pending fees to drip into the reserve.
    pub drip_secs: u32,
    pub buys_paused: bool,
    pub created_at: i64,
    pub trade_count: u64,
    pub volume_lamports: u64,
}
