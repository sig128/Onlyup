use anchor_lang::prelude::*;
use anchor_lang::system_program::{self, Transfer};
use anchor_spl::token::{self, Burn, Mint, Token, TokenAccount};

use crate::constants::*;
use crate::errors::OnlyUpError;
use crate::events::Traded;
use crate::instructions::{apply_drip, check_vault_covers};
use crate::math;
use crate::state::Market;

#[derive(Accounts)]
pub struct Sell<'info> {
    #[account(mut)]
    pub seller: Signer<'info>,

    #[account(mut)]
    pub mint: Account<'info, Mint>,

    #[account(
        mut,
        seeds = [MARKET_SEED, mint.key().as_ref()],
        bump = market.bump,
        has_one = mint,
    )]
    pub market: Account<'info, Market>,

    #[account(mut, seeds = [VAULT_SEED, market.key().as_ref()], bump = market.vault_bump)]
    pub vault: SystemAccount<'info>,

    /// The seller's tokens. They are burned.
    #[account(mut, token::mint = mint, token::authority = seller)]
    pub seller_tokens: Account<'info, TokenAccount>,

    pub token_program: Program<'info, Token>,
    pub system_program: Program<'info, System>,
}

/// Selling has no pause switch, no admin check and no hype fee: it always
/// works and always costs the same fixed fee.
pub fn handle_sell(ctx: Context<Sell>, tokens_in: u64, min_sol_out: u64) -> Result<()> {
    require!(tokens_in > 0, OnlyUpError::ZeroAmount);

    let now = Clock::get()?.unix_timestamp;
    apply_drip(&mut ctx.accounts.market, now)?;

    let m = &ctx.accounts.market;
    let reserve = m.reserve;
    let supply = ctx.accounts.mint.supply;
    require!(tokens_in < supply, OnlyUpError::SellTooLarge);
    let quote = math::quote_sell(tokens_in, reserve, supply, m.sell_fee_bps, m.team_fee_bps, 0)
        .ok_or(OnlyUpError::MathOverflow)?;
    require!(quote.user_out > 0, OnlyUpError::ZeroOutput);
    require!(quote.user_out >= min_sol_out, OnlyUpError::SlippageExceeded);

    let new_reserve = reserve.checked_sub(quote.gross).ok_or(OnlyUpError::MathOverflow)?;
    let new_pending = m.pending.checked_add(quote.to_pending).ok_or(OnlyUpError::MathOverflow)?;
    let new_supply = supply - tokens_in;
    let new_owed = m
        .team_fees_owed
        .checked_add(quote.team_fee)
        .ok_or(OnlyUpError::MathOverflow)?;
    require!(
        math::price_not_lower(reserve, supply, new_reserve, new_supply),
        OnlyUpError::PriceWouldDrop
    );
    let vault_bump = m.vault_bump;

    token::burn(
        CpiContext::new(
            ctx.accounts.token_program.to_account_info(),
            Burn {
                mint: ctx.accounts.mint.to_account_info(),
                from: ctx.accounts.seller_tokens.to_account_info(),
                authority: ctx.accounts.seller.to_account_info(),
            },
        ),
        tokens_in,
    )?;
    ctx.accounts.mint.reload()?;
    require!(ctx.accounts.mint.supply == new_supply, OnlyUpError::SupplyMismatch);

    let market_key = ctx.accounts.market.key();
    let bump = [vault_bump];
    let signer_seeds: &[&[&[u8]]] = &[&[VAULT_SEED, market_key.as_ref(), &bump]];
    system_program::transfer(
        CpiContext::new_with_signer(
            ctx.accounts.system_program.to_account_info(),
            Transfer {
                from: ctx.accounts.vault.to_account_info(),
                to: ctx.accounts.seller.to_account_info(),
            },
            signer_seeds,
        ),
        quote.user_out,
    )?;
    check_vault_covers(ctx.accounts.vault.lamports(), new_reserve, new_pending, new_owed)?;

    let market = &mut ctx.accounts.market;
    market.reserve = new_reserve;
    market.pending = new_pending;
    market.team_fees_owed = new_owed;
    market.trade_count = market.trade_count.saturating_add(1);
    market.volume_lamports = market.volume_lamports.saturating_add(quote.gross);

    emit!(Traded {
        market: market_key,
        trader: ctx.accounts.seller.key(),
        is_buy: false,
        sol_amount: quote.user_out,
        token_amount: tokens_in,
        hype_bps: 0,
        team_fee: quote.team_fee,
        reserve_after: new_reserve,
        pending_after: new_pending,
        supply_after: new_supply,
        timestamp: now,
    });
    Ok(())
}
