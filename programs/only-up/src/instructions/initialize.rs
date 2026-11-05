use anchor_lang::prelude::*;
use anchor_lang::system_program::{self, Transfer};
use anchor_spl::token::spl_token::instruction::AuthorityType;
use anchor_spl::token::{self, Mint, SetAuthority, Token};

use crate::constants::*;
use crate::errors::OnlyUpError;
use crate::events::MarketCreated;
use crate::instructions::check_vault_covers;
use crate::state::Market;

#[derive(AnchorSerialize, AnchorDeserialize, Clone, Debug)]
pub struct InitParams {
    /// Real SOL seeded into the curve reserve (lamports). Locked: the curve
    /// reserve never falls below it, since the virtual token side is capped.
    pub seed_lamports: u64,
    /// Virtual token reserve at launch, and the asymptotic supply cap
    /// (base units). Starting curve price = seed_lamports / seed_tokens.
    pub seed_tokens: u64,
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

    /// Holds the SOL (curve reserve + floor reserve + unclaimed creator fees).
    /// Only this program can sign for it.
    #[account(mut, seeds = [VAULT_SEED, market.key().as_ref()], bump)]
    pub vault: SystemAccount<'info>,

    /// An ordinary wallet that may receive the creator's fee share.
    pub treasury: SystemAccount<'info>,

    pub token_program: Program<'info, Token>,
    pub system_program: Program<'info, System>,
}

pub fn handle_initialize(ctx: Context<Initialize>, p: InitParams) -> Result<()> {
    require!(p.seed_lamports >= MIN_SEED_LAMPORTS, OnlyUpError::SeedTooSmall);
    require!(p.seed_tokens >= MIN_SEED_TOKENS, OnlyUpError::SeedTooSmall);
    require!(p.seed_tokens <= MAX_SEED_TOKENS, OnlyUpError::SeedTooLarge);
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

    // Hand the mint authority to the market, for good. From here, tokens are
    // only ever minted against a paid buy and burned on a sell.
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

    check_vault_covers(ctx.accounts.vault.lamports(), p.seed_lamports, 0, 0)?;

    let now = Clock::get()?.unix_timestamp;
    let treasury = ctx.accounts.treasury.key();
    ctx.accounts.market.set_inner(Market {
        version: MARKET_VERSION,
        bump: ctx.bumps.market,
        vault_bump: ctx.bumps.vault,
        mint: mint_key,
        admin,
        treasury,
        curve_sol: p.seed_lamports,
        floor_reserve: 0,
        total_supply: p.seed_tokens,
        creator_fees_owed: 0,
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
        curve_sol: p.seed_lamports,
        total_supply: p.seed_tokens,
        timestamp: now,
    });
    Ok(())
}
