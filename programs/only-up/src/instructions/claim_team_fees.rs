use anchor_lang::prelude::*;
use anchor_lang::system_program::{self, Transfer};

use crate::constants::*;
use crate::errors::OnlyUpError;
use crate::events::TeamFeesClaimed;
use crate::instructions::check_vault_covers;
use crate::state::Market;

#[derive(Accounts)]
pub struct ClaimTeamFees<'info> {
    #[account(mut, seeds = [MARKET_SEED, market.mint.as_ref()], bump = market.bump)]
    pub market: Account<'info, Market>,

    #[account(mut, seeds = [VAULT_SEED, market.key().as_ref()], bump = market.vault_bump)]
    pub vault: SystemAccount<'info>,

    /// CHECK: must be the treasury stored in the market; it only receives SOL.
    #[account(mut, address = market.treasury)]
    pub treasury: UncheckedAccount<'info>,

    pub system_program: Program<'info, System>,
}

/// Pays out only the team's booked fee share. The reserve and the pending
/// fees are never touched.
pub fn handle_claim_team_fees(ctx: Context<ClaimTeamFees>) -> Result<()> {
    let amount = ctx.accounts.market.team_fees_owed;
    require!(amount > 0, OnlyUpError::NothingToClaim);

    let market_key = ctx.accounts.market.key();
    let bump = [ctx.accounts.market.vault_bump];
    let signer_seeds: &[&[&[u8]]] = &[&[VAULT_SEED, market_key.as_ref(), &bump]];
    system_program::transfer(
        CpiContext::new_with_signer(
            ctx.accounts.system_program.to_account_info(),
            Transfer {
                from: ctx.accounts.vault.to_account_info(),
                to: ctx.accounts.treasury.to_account_info(),
            },
            signer_seeds,
        ),
        amount,
    )?;
    let m = &ctx.accounts.market;
    check_vault_covers(ctx.accounts.vault.lamports(), m.reserve, m.pending, 0)?;
    ctx.accounts.market.team_fees_owed = 0;

    emit!(TeamFeesClaimed {
        market: market_key,
        treasury: ctx.accounts.treasury.key(),
        lamports: amount,
    });
    Ok(())
}
