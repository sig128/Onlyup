pub mod admin;
pub mod buy;
pub mod claim_team_fees;
pub mod donate;
pub mod initialize;
pub mod sell;

pub use admin::*;
pub use buy::*;
pub use claim_team_fees::*;
pub use donate::*;
pub use initialize::*;
pub use sell::*;

use anchor_lang::prelude::*;

use crate::errors::OnlyUpError;
use crate::math;
use crate::state::Market;

/// Lets pending fees drip into the reserve up to `now`. Runs at the start of
/// every buy, sell and donation, so the price everyone trades at is current.
pub(crate) fn apply_drip(market: &mut Market, now: i64) -> Result<()> {
    let vested = math::vest(market.pending, now.saturating_sub(market.last_drip_at), market.drip_secs);
    market.reserve = market.reserve.checked_add(vested).ok_or(OnlyUpError::MathOverflow)?;
    market.pending -= vested;
    market.last_drip_at = market.last_drip_at.max(now);
    Ok(())
}

/// Defensive check after every SOL movement: the vault must hold at least its
/// rent-exempt minimum, the reserve, the pending fees and the unclaimed team fees.
pub(crate) fn check_vault_covers(vault_lamports: u64, reserve: u64, pending: u64, team_fees_owed: u64) -> Result<()> {
    let rent_min = Rent::get()?.minimum_balance(0);
    let needed = rent_min
        .checked_add(reserve)
        .and_then(|v| v.checked_add(pending))
        .and_then(|v| v.checked_add(team_fees_owed))
        .ok_or(OnlyUpError::MathOverflow)?;
    require!(vault_lamports >= needed, OnlyUpError::VaultShortfall);
    Ok(())
}
