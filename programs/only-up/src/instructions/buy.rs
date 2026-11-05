use anchor_lang::prelude::*;
use anchor_lang::system_program::{self, Transfer};
use anchor_spl::token::{self, Mint, MintTo, Token, TokenAccount};

use crate::constants::*;
use crate::errors::OnlyUpError;
use crate::events::Traded;
use crate::instructions::check_vault_covers;
use crate::math;

#[derive(Accounts)]
pub struct Buy<'info> {
    #[account(mut)]
    pub buyer: Signer<'info>,

    #[account(mut)]
    pub mint: Account<'info, Mint>,

    #[account(
        mut,
        seeds = [MARKET_SEED, mint.key().as_ref()],
        bump = market.bump,
        has_one = mint,
    )]
    pub market: Account<'info, crate::state::Market>,

    #[account(mut, seeds = [VAULT_SEED, market.key().as_ref()], bump = market.vault_bump)]
    pub vault: SystemAccount<'info>,

    /// Receives the new tokens. Any token account of this mint.
    #[account(mut, token::mint = mint)]
    pub buyer_tokens: Account<'info, TokenAccount>,

    pub token_program: Program<'info, Token>,
    pub system_program: Program<'info, System>,
}

pub fn handle_buy(ctx: Context<Buy>, sol_in: u64, min_tokens_out: u64) -> Result<()> {
    require!(sol_in > 0, OnlyUpError::ZeroAmount);
    require!(!ctx.accounts.market.buys_paused, OnlyUpError::BuysPaused);

    let m = &ctx.accounts.market;
    let reserve = m.reserve;
    let supply = ctx.accounts.mint.supply;
    let quote = math::quote_buy(sol_in, reserve, supply, m.buy_fee_bps, m.team_fee_bps)
        .ok_or(OnlyUpError::MathOverflow)?;
    require!(quote.tokens_out > 0, OnlyUpError::ZeroOutput);
    require!(quote.tokens_out >= min_tokens_out, OnlyUpError::SlippageExceeded);

    // The holder part goes straight into the reserve, so the price rises now.
    let new_reserve = reserve.checked_add(quote.to_reserve).ok_or(OnlyUpError::MathOverflow)?;
    let new_supply = supply.checked_add(quote.tokens_out).ok_or(OnlyUpError::MathOverflow)?;
    let new_owed = m.team_fees_owed.checked_add(quote.team_fee).ok_or(OnlyUpError::MathOverflow)?;
    require!(
        math::price_not_lower(reserve, supply, new_reserve, new_supply),
        OnlyUpError::PriceWouldDrop
    );
    let market_bump = m.bump;

    // All of the SOL goes into the vault. The creator share is booked separately.
    system_program::transfer(
        CpiContext::new(
            ctx.accounts.system_program.to_account_info(),
            Transfer {
                from: ctx.accounts.buyer.to_account_info(),
                to: ctx.accounts.vault.to_account_info(),
            },
        ),
        sol_in,
    )?;

    let mint_key = ctx.accounts.mint.key();
    let bump = [market_bump];
    let signer_seeds: &[&[&[u8]]] = &[&[MARKET_SEED, mint_key.as_ref(), &bump]];
    token::mint_to(
        CpiContext::new_with_signer(
            ctx.accounts.token_program.to_account_info(),
            MintTo {
                mint: ctx.accounts.mint.to_account_info(),
                to: ctx.accounts.buyer_tokens.to_account_info(),
                authority: ctx.accounts.market.to_account_info(),
            },
            signer_seeds,
        ),
        quote.tokens_out,
    )?;

    ctx.accounts.mint.reload()?;
    require!(ctx.accounts.mint.supply == new_supply, OnlyUpError::SupplyMismatch);
    check_vault_covers(ctx.accounts.vault.lamports(), new_reserve, new_owed)?;

    let now = Clock::get()?.unix_timestamp;
    let market_key = ctx.accounts.market.key();
    let market = &mut ctx.accounts.market;
    market.reserve = new_reserve;
    market.team_fees_owed = new_owed;
    market.trade_count = market.trade_count.saturating_add(1);
    market.volume_lamports = market.volume_lamports.saturating_add(sol_in);

    emit!(Traded {
        market: market_key,
        trader: ctx.accounts.buyer.key(),
        is_buy: true,
        sol_amount: sol_in,
        token_amount: quote.tokens_out,
        team_fee: quote.team_fee,
        reserve_after: new_reserve,
        supply_after: new_supply,
        timestamp: now,
    });
    Ok(())
}
