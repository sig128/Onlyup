use anchor_lang::prelude::*;
use anchor_lang::system_program::{self, Transfer};
use anchor_spl::token::spl_token::instruction::AuthorityType;
use anchor_spl::token::{self, Mint, MintTo, SetAuthority, Token, TokenAccount};

use crate::constants::*;
use crate::errors::OnlyUpError;
use crate::events::MarketCreated;
use crate::instructions::check_vault_covers;
use crate::state::Market;

#[derive(AnchorSerialize, AnchorDeserialize, Clone, Debug)]
pub struct InitParams {
    /// Lamports put in the reserve and locked forever.
    pub seed_lamports: u64,
    /// Token base units minted into the locked account forever.
    /// Starting price = seed_lamports / seed_tokens.
    pub seed_tokens: u64,
    /// Holder fee on buys, in basis points (e.g. 250 = 2.5%).
    pub buy_fee_bps: u16,
    /// Holder fee on sells, in basis points.
    pub sell_fee_bps: u16,
    /// Creator share of every buy and sell, in basis points (e.g. 50 = 0.5%).
    pub team_fee_bps: u16,
}

#[derive(Accounts)]
pub struct Initialize<'info> {
    /// Must be the mint's current mint authority.
    #[account(mut)]
    pub admin: Signer<'info>,

    /// A fresh mint: zero supply, 6 decimals, no freeze authority.
    #[account(mut)]
    pub mint: Account<'info, Mint>,

    #[account(
        init,
        payer = admin,
        space = 8 + Market::INIT_SPACE,
        seeds = [MARKET_SEED, mint.key().as_ref()],
        bump,
    )]
    pub market: Account<'info, Market>,

    /// Holds the SOL. Only this program can sign for it.
    #[account(mut, seeds = [VAULT_SEED, market.key().as_ref()], bump)]
    pub vault: SystemAccount<'info>,

    /// Holds the seed tokens forever. No instruction ever moves them.
    #[account(
        init,
        payer = admin,
        seeds = [LOCKED_SEED, market.key().as_ref()],
        bump,
        token::mint = mint,
        token::authority = market,
    )]
    pub locked_tokens: Account<'info, TokenAccount>,

    /// An ordinary wallet that may receive the creator's fee share.
    pub treasury: SystemAccount<'info>,

    pub token_program: Program<'info, Token>,
    pub system_program: Program<'info, System>,
}

pub fn handle_initialize(ctx: Context<Initialize>, p: InitParams) -> Result<()> {
    require!(
        (MIN_FEE_BPS..=MAX_BUY_FEE_BPS).contains(&p.buy_fee_bps)
            && (MIN_FEE_BPS..=MAX_SELL_FEE_BPS).contains(&p.sell_fee_bps)
            && p.team_fee_bps <= MAX_TEAM_FEE_BPS,
        OnlyUpError::BadFeeSettings
    );
    // A buy lifts the price by at most the buy fee, so an exit fee at least as
    // large means a round trip around another trader can never turn a profit.
    // This is what keeps buys from being sandwiched, with fees landing instantly.
    require!(p.sell_fee_bps >= p.buy_fee_bps, OnlyUpError::LopsidedFees);
    require!(
        p.seed_lamports >= MIN_SEED_LAMPORTS && p.seed_tokens >= MIN_SEED_TOKENS,
        OnlyUpError::SeedTooSmall
    );
    require!(
        p.seed_tokens / p.seed_lamports <= MAX_SEED_TOKENS_PER_LAMPORT,
        OnlyUpError::StartPriceTooLow
    );
    require_keys_neq!(ctx.accounts.treasury.key(), Pubkey::default(), OnlyUpError::BadTreasury);

    let admin = ctx.accounts.admin.key();
    let market_key = ctx.accounts.market.key();
    let mint_key = ctx.accounts.mint.key();
    {
        let mint = &ctx.accounts.mint;
        require!(mint.supply == 0, OnlyUpError::MintNotEmpty);
        require!(mint.decimals == DECIMALS, OnlyUpError::WrongDecimals);
        // Only whoever controls the mint can create its market, so nobody can
        // front-run the launch with their own settings.
        require!(
            Option::<Pubkey>::from(mint.mint_authority) == Some(admin),
            OnlyUpError::BadMintAuthority
        );
        require!(mint.freeze_authority.is_none(), OnlyUpError::FreezeAuthoritySet);
    }

    // Hand the mint authority to the market, for good.
    token::set_authority(
        CpiContext::new(
            ctx.accounts.token_program.to_account_info(),
            SetAuthority {
                current_authority: ctx.accounts.admin.to_account_info(),
                account_or_mint: ctx.accounts.mint.to_account_info(),
            },
        ),
        AuthorityType::MintTokens,
        Some(market_key),
    )?;

    // Fund the vault with its rent-exempt minimum plus the seed reserve.
    // Anything someone sent to the vault address earlier simply stays there.
    let rent_min = Rent::get()?.minimum_balance(0);
    let top_up = rent_min
        .saturating_sub(ctx.accounts.vault.lamports())
        .checked_add(p.seed_lamports)
        .ok_or(OnlyUpError::MathOverflow)?;
    system_program::transfer(
        CpiContext::new(
            ctx.accounts.system_program.to_account_info(),
            Transfer {
                from: ctx.accounts.admin.to_account_info(),
                to: ctx.accounts.vault.to_account_info(),
            },
        ),
        top_up,
    )?;

    // Mint the seed tokens into the locked account.
    let bump = [ctx.bumps.market];
    let signer_seeds: &[&[&[u8]]] = &[&[MARKET_SEED, mint_key.as_ref(), &bump]];
    token::mint_to(
        CpiContext::new_with_signer(
            ctx.accounts.token_program.to_account_info(),
            MintTo {
                mint: ctx.accounts.mint.to_account_info(),
                to: ctx.accounts.locked_tokens.to_account_info(),
                authority: ctx.accounts.market.to_account_info(),
            },
            signer_seeds,
        ),
        p.seed_tokens,
    )?;

    check_vault_covers(ctx.accounts.vault.lamports(), p.seed_lamports, 0)?;

    let now = Clock::get()?.unix_timestamp;
    let treasury = ctx.accounts.treasury.key();
    ctx.accounts.market.set_inner(Market {
        version: MARKET_VERSION,
        bump: ctx.bumps.market,
        vault_bump: ctx.bumps.vault,
        locked_bump: ctx.bumps.locked_tokens,
        mint: mint_key,
        admin,
        treasury,
        reserve: p.seed_lamports,
        team_fees_owed: 0,
        buy_fee_bps: p.buy_fee_bps,
        sell_fee_bps: p.sell_fee_bps,
        team_fee_bps: p.team_fee_bps,
        buys_paused: false,
        created_at: now,
        trade_count: 0,
        volume_lamports: 0,
    });

    emit!(MarketCreated {
        market: market_key,
        mint: mint_key,
        admin,
        treasury,
        reserve: p.seed_lamports,
        supply: p.seed_tokens,
        buy_fee_bps: p.buy_fee_bps,
        sell_fee_bps: p.sell_fee_bps,
        team_fee_bps: p.team_fee_bps,
        timestamp: now,
    });
    Ok(())
}
