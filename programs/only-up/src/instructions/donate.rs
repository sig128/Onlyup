use anchor_lang::prelude::*;
use anchor_lang::system_program::{self, Transfer};

use crate::constants::*;
use crate::errors::OnlyUpError;
use crate::events::Donated;
use crate::instructions::{apply_drip, check_vault_covers};
use crate::state::Market;

#[derive(Accounts)]
pub struct Donate<'info> {
    #[account(mut)]
    pub donor: Signer<'info>,

    #[account(mut, seeds = [MARKET_SEED, market.mint.as_ref()], bump = market.bump)]
    pub market: Account<'info, Market>,

    #[account(mut, seeds = [VAULT_SEED, market.key().as_ref()], bump = market.vault_bump)]
    pub vault: SystemAccount<'info>,

    pub system_program: Program<'info, System>,
}

/// Adds SOL without minting anything. It drips into the reserve like a fee,
/// so the price rises for every holder over the drip window. Useful for
/// routing outside revenue into the token.
pub fn handle_donate(ctx: Context<Donate>, lamports: u64) -> Result<()> {
    require!(lamports > 0, OnlyUpError::ZeroAmount);
    let now = Clock::get()?.unix_timestamp;
    apply_drip(&mut ctx.accounts.market, now)?;

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
    let new_pending = m.pending.checked_add(lamports).ok_or(OnlyUpError::MathOverflow)?;
    check_vault_covers(ctx.accounts.vault.lamports(), m.reserve, new_pending, m.team_fees_owed)?;
    let market_key = ctx.accounts.market.key();
    ctx.accounts.market.pending = new_pending;

    emit!(Donated {
        market: market_key,
        donor: ctx.accounts.donor.key(),
        lamports,
        pending_after: new_pending,
        timestamp: now,
    });
    Ok(())
}
