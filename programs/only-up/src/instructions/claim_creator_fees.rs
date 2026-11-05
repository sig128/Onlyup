use anchor_lang::prelude::*;
use anchor_lang::system_program::{self, Transfer};

use crate::constants::*;
use crate::errors::OnlyUpError;
use crate::events::CreatorFeesClaimed;
use crate::instructions::check_vault_covers;

#[derive(Accounts)]
pub struct ClaimCreatorFees<'info> {
    #[account(mut, seeds = [MARKET_SEED, market.mint.as_ref()], bump = market.bump)]
    pub market: Account<'info, crate::state::Market>,

    #[account(mut, seeds = [VAULT_SEED, market.key().as_ref()], bump = market.vault_bump)]
    pub vault: SystemAccount<'info>,

    /// CHECK: must be the treasury stored in the market; it only receives SOL.
    #[account(mut, address = market.treasury)]
    pub treasury: UncheckedAccount<'info>,

    pub system_program: Program<'info, System>,
}

/// Pays out only the creator's booked fee share. Neither reserve is touched.
/// Anyone can trigger it, but the SOL only ever goes to the stored treasury.
pub fn handle_claim_creator_fees(ctx: Context<ClaimCreatorFees>) -> Result<()> {
    let amount = ctx.accounts.market.creator_fees_owed;
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
    check_vault_covers(ctx.accounts.vault.lamports(), m.curve_sol, m.floor_reserve, 0)?;
    ctx.accounts.market.creator_fees_owed = 0;

    emit!(CreatorFeesClaimed {
        market: market_key,
        treasury: ctx.accounts.treasury.key(),
        lamports: amount,
    });
    Ok(())
}
