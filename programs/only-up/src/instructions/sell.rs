use anchor_lang::prelude::*;
use anchor_lang::system_program::{self, Transfer};
use anchor_spl::token::{self, Burn, Mint, Token, TokenAccount};

use crate::constants::*;
use crate::errors::OnlyUpError;
use crate::events::Traded;
use crate::instructions::check_vault_covers;
use crate::math;

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
    pub market: Account<'info, crate::state::Market>,

    #[account(mut, seeds = [VAULT_SEED, market.key().as_ref()], bump = market.vault_bump)]
    pub vault: SystemAccount<'info>,

    /// The seller's tokens. They are burned.
    #[account(mut, token::mint = mint, token::authority = seller)]
    pub seller_tokens: Account<'info, TokenAccount>,

    pub token_program: Program<'info, Token>,
    pub system_program: Program<'info, System>,
}

/// Selling has no pause switch and no admin check: it always works. The seller
/// is paid the better of the curve price or the floor; a floor redemption can
/// never lower the floor for everyone else.
pub fn handle_sell(ctx: Context<Sell>, tokens_in: u64, min_sol_out: u64) -> Result<()> {
    require!(tokens_in > 0, OnlyUpError::ZeroAmount);

    let m = &ctx.accounts.market;
    let curve_sol = m.curve_sol;
    let floor_reserve = m.floor_reserve;
    let supply = ctx.accounts.mint.supply;
    require!(tokens_in < supply, OnlyUpError::SellTooLarge);
    let tc = m.total_supply - supply; // virtual token reserve

    let quote = math::quote_sell(tokens_in, curve_sol, tc, floor_reserve, supply)
        .ok_or(OnlyUpError::MathOverflow)?;
    require!(quote.user_out > 0, OnlyUpError::ZeroOutput);
    require!(quote.user_out >= min_sol_out, OnlyUpError::SlippageExceeded);

    let new_curve_sol = quote.new_curve_sol;
    let new_floor = quote.new_floor_reserve;
    let new_supply = supply - tokens_in;
    let new_owed = m.creator_fees_owed.checked_add(quote.creator_fee).ok_or(OnlyUpError::MathOverflow)?;

    require!(
        math::floor_not_lower(floor_reserve, supply, new_floor, new_supply),
        OnlyUpError::FloorWouldDrop
    );
    // The reserve the payout comes out of must drop by the gross; the 2.5%
    // floor fee then flows back into the floor, so the floor only ever grows.
    if quote.from_floor {
        let expected_floor = floor_reserve
            .checked_sub(quote.gross).ok_or(OnlyUpError::FloorShortfall)?
            .checked_add(quote.floor_fee).ok_or(OnlyUpError::MathOverflow)?;
        require!(new_floor == expected_floor, OnlyUpError::FloorShortfall);
        require!(new_curve_sol == curve_sol, OnlyUpError::MathOverflow);
    } else {
        require!(new_curve_sol == curve_sol.checked_sub(quote.gross).ok_or(OnlyUpError::VaultShortfall)?, OnlyUpError::VaultShortfall);
        require!(new_floor == floor_reserve.checked_add(quote.floor_fee).ok_or(OnlyUpError::MathOverflow)?, OnlyUpError::MathOverflow);
    }
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
    check_vault_covers(ctx.accounts.vault.lamports(), new_curve_sol, new_floor, new_owed)?;

    let now = Clock::get()?.unix_timestamp;
    let market = &mut ctx.accounts.market;
    market.curve_sol = new_curve_sol;
    market.floor_reserve = new_floor;
    market.creator_fees_owed = new_owed;
    market.trade_count = market.trade_count.saturating_add(1);
    market.volume_lamports = market.volume_lamports.saturating_add(quote.gross);

    emit!(Traded {
        market: market_key,
        trader: ctx.accounts.seller.key(),
        is_buy: false,
        floor_redeem: quote.from_floor,
        sol_amount: quote.user_out,
        token_amount: tokens_in,
        creator_fee: quote.creator_fee,
        curve_sol_after: new_curve_sol,
        floor_reserve_after: new_floor,
        supply_after: new_supply,
        timestamp: now,
    });
    Ok(())
}
