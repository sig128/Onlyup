use anchor_lang::prelude::*;

/// Emitted once, when the market is created.
#[event]
pub struct MarketCreated {
    pub market: Pubkey,
    pub mint: Pubkey,
    pub admin: Pubkey,
    pub treasury: Pubkey,
    pub reserve: u64,
    pub supply: u64,
    pub buy_fee_bps: u16,
    pub sell_fee_bps: u16,
    pub team_fee_bps: u16,
    pub timestamp: i64,
}

/// Emitted on every buy and sell. Price after the trade = reserve_after / supply_after.
#[event]
pub struct Traded {
    pub market: Pubkey,
    pub trader: Pubkey,
    pub is_buy: bool,
    /// Lamports paid in (buy) or paid out to the seller (sell).
    pub sol_amount: u64,
    /// Tokens minted (buy) or burned (sell), in base units.
    pub token_amount: u64,
    /// Creator share taken from this trade, in lamports.
    pub team_fee: u64,
    pub reserve_after: u64,
    pub supply_after: u64,
    pub timestamp: i64,
}

#[event]
pub struct Donated {
    pub market: Pubkey,
    pub donor: Pubkey,
    pub lamports: u64,
    pub reserve_after: u64,
    pub timestamp: i64,
}

#[event]
pub struct TeamFeesClaimed {
    pub market: Pubkey,
    pub treasury: Pubkey,
    pub lamports: u64,
}

#[event]
pub struct BuysPausedSet {
    pub market: Pubkey,
    pub paused: bool,
}

#[event]
pub struct TreasurySet {
    pub market: Pubkey,
    pub treasury: Pubkey,
}

#[event]
pub struct AdminRenounced {
    pub market: Pubkey,
}
