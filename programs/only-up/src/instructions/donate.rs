use anchor_lang::prelude::*;
use anchor_lang::system_program::{self, Transfer};

use crate::constants::*;
use crate::errors::OnlyUpError;
use crate::events::Donated;
use crate::instructions::check_vault_covers;

#[derive(Accounts)]
pub struct Donate<'info> {
    #[account(mut)]
    pub donor: Signer<'info>,

    #[account(mut, seeds = [MARKET_SEED, market.mint.as_ref()], bump = market.bump)]
    pub market: Account<'info, crate::state::Market>,

    #[account(mut, seeds = [VAULT_SEED, market.key().as_ref()], bump = market.vault_bump)]
    pub vault: SystemAccount<'info>,

    pub system_program: Program<'info, System>,
}

/// Adds SOL straight into the floor reserve without minting anything, so the
/// floor rises for every holder. Useful for routing outside revenue into the
/// token. Adding to the floor only ever raises floor_reserve / supply.
pub fn handle_donate(ctx: Context<Donate>, lamports: u64) -> Result<()> {
    require!(lamports > 0, OnlyUpError::ZeroAmount);

    system_program::transfer(
        CpiContext::new(
            ctx.accounts.system_program.to_account_info(),
            Transfer {
                from: ctx.accounts.donor.to_account_info(),
                to: ctx.accounts.vault.to_account_info(),
            },
        ),
        lamports,
    )?;

    let m = &ctx.accounts.market;
    let new_floor = m.floor_reserve.checked_add(lamports).ok_or(OnlyUpError::MathOverflow)?;
    check_vault_covers(ctx.accounts.vault.lamports(), m.curve_sol, new_floor, m.creator_fees_owed)?;
    let now = Clock::get()?.unix_timestamp;
    let market_key = ctx.accounts.market.key();
    ctx.accounts.market.floor_reserve = new_floor;

    emit!(Donated {
        market: market_key,
        donor: ctx.accounts.donor.key(),
        lamports,
        floor_reserve_after: new_floor,
        timestamp: now,
    });
    Ok(())
}
