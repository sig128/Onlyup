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

/// Defensive check after every SOL movement: the vault must hold at least its
/// rent-exempt minimum, the reserve and the unclaimed creator fees.
pub(crate) fn check_vault_covers(vault_lamports: u64, reserve: u64, team_fees_owed: u64) -> Result<()> {
    let rent_min = Rent::get()?.minimum_balance(0);
    let needed = rent_min
        .checked_add(reserve)
        .and_then(|v| v.checked_add(team_fees_owed))
        .ok_or(OnlyUpError::MathOverflow)?;
    require!(vault_lamports >= needed, OnlyUpError::VaultShortfall);
    Ok(())
}
