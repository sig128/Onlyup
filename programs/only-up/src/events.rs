use anchor_lang::prelude::*;

#[event]
pub struct MarketCreated {
    pub market: Pubkey,
    pub mint: Pubkey,
    pub admin: Pubkey,
    pub treasury: Pubkey,
    pub curve_sol: u64,
    pub total_supply: u64,
    pub timestamp: i64,
}

#[event]
pub struct Traded {
    pub market: Pubkey,
    pub trader: Pubkey,
    pub is_buy: bool,
    /// True when a sell was paid from the floor reserve instead of the curve.
    pub floor_redeem: bool,
    /// SOL the trader paid (buy) or received (sell), in lamports.
    pub sol_amount: u64,
    /// Tokens received (buy) or sold (sell), in base units.
    pub token_amount: u64,
    pub creator_fee: u64,
    pub curve_sol_after: u64,
    pub floor_reserve_after: u64,
    pub supply_after: u64,
    pub timestamp: i64,
}

#[event]
pub struct Donated {
    pub market: Pubkey,
    pub donor: Pubkey,
    pub lamports: u64,
    pub floor_reserve_after: u64,
    pub timestamp: i64,
}

#[event]
pub struct CreatorFeesClaimed {
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
