use anchor_lang::prelude::*;

#[error_code]
pub enum OnlyUpError {
    #[msg("Amount must be greater than zero")]
    ZeroAmount,
    #[msg("Trade too small: it would return nothing")]
    ZeroOutput,
    #[msg("You would receive less than the minimum you set")]
    SlippageExceeded,
    #[msg("Buying is paused")]
    BuysPaused,
    #[msg("This trade would lower the floor, so it was cancelled")]
    FloorWouldDrop,
    #[msg("Math overflow")]
    MathOverflow,
    #[msg("Seed SOL or seed tokens are below the minimum")]
    SeedTooSmall,
    #[msg("Seed tokens are above the maximum")]
    SeedTooLarge,
    #[msg("Starting price is too low: too many seed tokens per lamport")]
    StartPriceTooLow,
    #[msg("The mint must have zero supply")]
    MintNotEmpty,
    #[msg("The mint must have 6 decimals")]
    WrongDecimals,
    #[msg("Only the mint's current mint authority can create its market")]
    BadMintAuthority,
    #[msg("The mint must not have a freeze authority")]
    FreezeAuthoritySet,
    #[msg("Cannot sell the whole supply")]
    SellTooLarge,
    #[msg("Treasury cannot be the default address")]
    BadTreasury,
    #[msg("No creator fees to claim")]
    NothingToClaim,
    #[msg("Token supply changed unexpectedly")]
    SupplyMismatch,
    #[msg("Vault balance is lower than the books say")]
    VaultShortfall,
    #[msg("Floor reserve is lower than the books say")]
    FloorShortfall,
    #[msg("Caller is not the admin")]
    NotAdmin,
}
