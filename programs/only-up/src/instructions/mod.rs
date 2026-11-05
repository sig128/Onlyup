pub mod admin;
pub mod buy;
pub mod claim_creator_fees;
pub mod donate;
pub mod initialize;
pub mod sell;

pub use admin::*;
pub use buy::*;
pub use claim_creator_fees::*;
pub use donate::*;
pub use initialize::*;
pub use sell::*;

use anchor_lang::prelude::*;

use crate::errors::OnlyUpError;

/// Defensive check after every SOL movement: the vault must hold at least its
/// rent-exempt minimum plus the curve reserve, the floor reserve and the
/// unclaimed creator fees. The only SOL that ever leaves the vault is a payout
/// to a seller or a creator-fee claim.
pub(crate) fn check_vault_covers(
    vault_lamports: u64,
    curve_sol: u64,
    floor_reserve: u64,
    creator_fees_owed: u64,
) -> Result<()> {
    let rent_min = Rent::get()?.minimum_balance(0);
    let needed = rent_min
        .checked_add(curve_sol)
        .and_then(|v| v.checked_add(floor_reserve))
        .and_then(|v| v.checked_add(creator_fees_owed))
        .ok_or(OnlyUpError::MathOverflow)?;
    require!(vault_lamports >= needed, OnlyUpError::VaultShortfall);
    Ok(())
}
