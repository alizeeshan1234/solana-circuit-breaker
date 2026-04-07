use anchor_lang::prelude::*;

#[error_code]
pub enum CircuitBreakerError {
    #[msg("Circuit breaker is tripped — outflows halted")]
    BreakerTripped,
    #[msg("Outflow exceeds rate limit for this time window")]
    RateLimitExceeded,
    #[msg("Single transaction exceeds maximum allowed outflow")]
    MaxSingleOutflowExceeded,
    #[msg("Cooldown period has not elapsed")]
    CooldownNotElapsed,
    #[msg("Invalid authority")]
    InvalidAuthority,
    #[msg("Invalid rate limit basis points (must be 1-10000)")]
    InvalidRateLimit,
    #[msg("Invalid time window (must be > 0)")]
    InvalidTimeWindow,
    #[msg("TVL must be greater than zero")]
    ZeroTvl,
    #[msg("Arithmetic overflow")]
    MathOverflow,
    #[msg("Breaker is not tripped")]
    BreakerNotTripped,
    #[msg("Vault is paused by admin")]
    VaultPaused,
    #[msg("Outflow amount must be greater than zero")]
    ZeroOutflow,
    #[msg("Lockout period active — breaker cannot be reset yet")]
    LockoutActive,
}
