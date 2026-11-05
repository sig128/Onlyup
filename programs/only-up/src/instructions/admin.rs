use anchor_lang::prelude::*;

use crate::constants::*;
use crate::errors::OnlyUpError;
use crate::events::{AdminRenounced, BuysPausedSet, TreasurySet};

/// The admin can only pause buying and change the treasury. The admin cannot
/// touch the vault, change fees, mint tokens or stop anyone from selling.
#[derive(Accounts)]
pub struct AdminOnly<'info> {
    pub admin: Signer<'info>,

    #[account(
        mut,
        seeds = [MARKET_SEED, market.mint.as_ref()],
        bump = market.bump,
        has_one = admin,
    )]
    pub market: Account<'info, crate::state::Market>,
}

#[derive(Accounts)]
pub struct SetTreasury<'info> {
    pub admin: Signer<'info>,

    #[account(
        mut,
        seeds = [MARKET_SEED, market.mint.as_ref()],
        bump = market.bump,
        has_one = admin,
    )]
    pub market: Account<'info, crate::state::Market>,

    /// The new treasury: an ordinary wallet, so creator fees can always be claimed.
    pub new_treasury: SystemAccount<'info>,
}

pub fn handle_set_buys_paused(ctx: Context<AdminOnly>, paused: bool) -> Result<()> {
    ctx.accounts.market.buys_paused = paused;
    emit!(BuysPausedSet {
        market: ctx.accounts.market.key(),
        paused,
    });
    Ok(())
}

pub fn handle_set_treasury(ctx: Context<SetTreasury>) -> Result<()> {
    let treasury = ctx.accounts.new_treasury.key();
    require_keys_neq!(treasury, Pubkey::default(), OnlyUpError::BadTreasury);
    ctx.accounts.market.treasury = treasury;
    emit!(TreasurySet {
        market: ctx.accounts.market.key(),
        treasury,
    });
    Ok(())
}

/// Gives up admin rights forever. Buying is switched back on so it can never
/// be left paused with nobody able to resume it.
pub fn handle_renounce_admin(ctx: Context<AdminOnly>) -> Result<()> {
    ctx.accounts.market.admin = Pubkey::default();
    ctx.accounts.market.buys_paused = false;
    emit!(AdminRenounced {
        market: ctx.accounts.market.key(),
    });
    Ok(())
}
