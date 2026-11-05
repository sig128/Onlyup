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
    #[msg("This trade would lower the price, so it was cancelled")]
    PriceWouldDrop,
    #[msg("Math overflow")]
    MathOverflow,
    #[msg("Fee settings are outside the allowed range")]
    BadFeeSettings,
    #[msg("Hype settings are outside the allowed range")]
    BadHypeSettings,
    #[msg("Seed SOL or seed tokens are below the minimum")]
    SeedTooSmall,
    #[msg("The mint must have zero supply")]
    MintNotEmpty,
    #[msg("Only the mint's current mint authority can create its market")]
    BadMintAuthority,
    #[msg("The mint must not have a freeze authority")]
    FreezeAuthoritySet,
    #[msg("Cannot sell the whole supply")]
    SellTooLarge,
    #[msg("Treasury cannot be the default address")]
    BadTreasury,
    #[msg("No team fees to claim")]
    NothingToClaim,
    #[msg("Token supply changed unexpectedly")]
    SupplyMismatch,
    #[msg("Vault balance is lower than the books say")]
    VaultShortfall,
    #[msg("Starting price is too low: too many seed tokens per lamport")]
    StartPriceTooLow,
    #[msg("The mint must have 6 decimals")]
    WrongDecimals,
    #[msg("Drip window is outside the allowed range")]
    BadDripSettings,
}
