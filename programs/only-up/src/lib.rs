//! ONLY UP: a token whose price in SOL can never go down.
//!
//! The program itself is the market. Buying mints new tokens at the vault
//! price plus a fee. Selling burns tokens and pays out the vault price minus
//! a fee. The holder fee stays in the vault and joins the price on the spot,
//! so the SOL backing each token only ever grows.
//!
//! Price = SOL reserve in the vault / token supply.
//!
//! Every buy and sell re-checks that the price did not fall, and the whole
//! transaction is cancelled if it would have. There is no instruction that
//! takes reserve SOL out of the vault except paying sellers.
use anchor_lang::prelude::*;

pub mod constants;
pub mod errors;
pub mod events;
pub mod instructions;
pub mod math;
pub mod state;

pub use instructions::*;

declare_id!("3ooGGJTLiadrijsQPW9AfTQLrgawhSkEYJPrPQpjnGxS");

#[program]
pub mod only_up {
    use super::*;

    /// Creates the market for a fresh mint. The caller must be the mint's
    /// mint authority; the authority is handed to the market in this same
    /// instruction. Locks the seed SOL and seed tokens forever.
    pub fn initialize(ctx: Context<Initialize>, params: InitParams) -> Result<()> {
        instructions::initialize::handle_initialize(ctx, params)
    }

    /// Pays `sol_in` lamports and receives newly minted tokens at the vault
    /// price plus the buy fee. Fails if fewer than `min_tokens_out` would arrive.
    pub fn buy(ctx: Context<Buy>, sol_in: u64, min_tokens_out: u64) -> Result<()> {
        instructions::buy::handle_buy(ctx, sol_in, min_tokens_out)
    }

    /// Burns `tokens_in` and receives SOL at the vault price minus the sell
    /// fee. Fails if less than `min_sol_out` would arrive. Can never be paused.
    pub fn sell(ctx: Context<Sell>, tokens_in: u64, min_sol_out: u64) -> Result<()> {
        instructions::sell::handle_sell(ctx, tokens_in, min_sol_out)
    }

    /// Adds SOL straight into the reserve without minting anything, which
    /// raises the price for every holder.
    pub fn donate(ctx: Context<Donate>, lamports: u64) -> Result<()> {
        instructions::donate::handle_donate(ctx, lamports)
    }

    /// Sends the creator's accumulated fee share to the treasury. Anyone can
    /// call it; the money can only go to the stored treasury address.
    pub fn claim_team_fees(ctx: Context<ClaimTeamFees>) -> Result<()> {
        instructions::claim_team_fees::handle_claim_team_fees(ctx)
    }

    /// Admin only. Pauses or resumes buying. Selling can never be paused.
    pub fn set_buys_paused(ctx: Context<AdminOnly>, paused: bool) -> Result<()> {
        instructions::admin::handle_set_buys_paused(ctx, paused)
    }

    /// Admin only. Changes where the creator's fee share is sent. The new
    /// treasury must be an ordinary wallet.
    pub fn set_treasury(ctx: Context<SetTreasury>) -> Result<()> {
        instructions::admin::handle_set_treasury(ctx)
    }

    /// Admin only. Gives up admin rights forever and resumes buying.
    pub fn renounce_admin(ctx: Context<AdminOnly>) -> Result<()> {
        instructions::admin::handle_renounce_admin(ctx)
    }
}
