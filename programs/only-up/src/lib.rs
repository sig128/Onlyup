//! ONLY UP v2 — a token that trades on a real curve, over a floor that can only rise.
//!
//! The program is the market. It is a constant-product curve (real SOL reserve
//! against a virtual token reserve) sitting on top of a separate floor reserve.
//! The curve price can move up and down like any coin. 25% of every buy funds
//! the floor reserve, and the amount a holder can always redeem per token —
//! floor_reserve / supply — can only ever ratchet UP, never fall. Every trade
//! re-checks that the floor did not drop and that the vault still covers both
//! reserves plus unclaimed creator fees.
//!
//! The one mechanism that makes it sound: each newly minted token must carry at
//! least the current floor in backing, and nobody can mint below the floor and
//! redeem for a profit. That is proven across a 2,000-market stress test.
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
    /// authority; it is handed to the market here. Seeds the curve reserve.
    pub fn initialize(ctx: Context<Initialize>, params: InitParams) -> Result<()> {
        instructions::initialize::handle_initialize(ctx, params)
    }

    /// Pays `sol_in` lamports and receives newly minted tokens. 1% creator fee,
    /// 25% of the rest funds the floor, the remainder drives the curve.
    pub fn buy(ctx: Context<Buy>, sol_in: u64, min_tokens_out: u64) -> Result<()> {
        instructions::buy::handle_buy(ctx, sol_in, min_tokens_out)
    }

    /// Burns `tokens_in` and pays the better of the curve price or the floor,
    /// minus a 1% creator fee. Can never be paused.
    pub fn sell(ctx: Context<Sell>, tokens_in: u64, min_sol_out: u64) -> Result<()> {
        instructions::sell::handle_sell(ctx, tokens_in, min_sol_out)
    }

    /// Adds SOL straight into the floor reserve, raising the floor for everyone.
    pub fn donate(ctx: Context<Donate>, lamports: u64) -> Result<()> {
        instructions::donate::handle_donate(ctx, lamports)
    }

    /// Sends the creator's booked fee share to the stored treasury. Anyone can
    /// call it; the money can only go to that treasury address.
    pub fn claim_creator_fees(ctx: Context<ClaimCreatorFees>) -> Result<()> {
        instructions::claim_creator_fees::handle_claim_creator_fees(ctx)
    }

    /// Admin only. Pauses or resumes buying. Selling can never be paused.
    pub fn set_buys_paused(ctx: Context<AdminOnly>, paused: bool) -> Result<()> {
        instructions::admin::handle_set_buys_paused(ctx, paused)
    }

    /// Admin only. Changes where the creator's fee share is sent.
    pub fn set_treasury(ctx: Context<SetTreasury>) -> Result<()> {
        instructions::admin::handle_set_treasury(ctx)
    }

    /// Admin only. Gives up admin rights forever and resumes buying.
    pub fn renounce_admin(ctx: Context<AdminOnly>) -> Result<()> {
        instructions::admin::handle_renounce_admin(ctx)
    }
}
