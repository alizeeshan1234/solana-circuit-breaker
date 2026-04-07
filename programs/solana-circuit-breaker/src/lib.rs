pub mod constants;
pub mod error;
pub mod instructions;
pub mod state;

use anchor_lang::prelude::*;

pub use constants::*;
pub use instructions::*;
pub use state::*;

declare_id!("3fxSA4BZSks8jcyxg11kkrjEhBePtHoXEoxVSryD5VUc");

#[program]
pub mod solana_circuit_breaker {
    use super::*;

    /// Initialize the circuit breaker protocol.
    pub fn initialize(ctx: Context<Initialize>) -> Result<()> {
        initialize::handler(ctx)
    }

    /// Register a vault to be protected by the circuit breaker.
    pub fn register_vault(
        ctx: Context<RegisterVault>,
        params: RegisterVaultParams,
    ) -> Result<()> {
        register_vault::handler(ctx, params)
    }

    /// Check if an outflow is allowed. Errors when blocked — protocols cannot ignore.
    /// On success, records the outflow in the rolling window.
    pub fn check_outflow(
        ctx: Context<CheckOutflow>,
        amount: u64,
        current_tvl: u64,
    ) -> Result<()> {
        check_outflow::handler(ctx, amount, current_tvl)
    }

    /// Manually trip the breaker (emergency pause).
    pub fn trip_breaker(ctx: Context<TripBreaker>) -> Result<()> {
        trip_breaker::handler(ctx)
    }

    /// Manually reset a tripped breaker.
    pub fn reset_breaker(ctx: Context<ResetBreaker>) -> Result<()> {
        reset_breaker::handler(ctx)
    }

    /// Update vault policy. Tightening applies immediately, loosening is queued.
    pub fn update_policy(
        ctx: Context<UpdatePolicy>,
        params: UpdatePolicyParams,
    ) -> Result<()> {
        update_policy::handler(ctx, params)
    }

    /// Transfer policy_authority and/or breaker_authority to new keys.
    pub fn transfer_authority(
        ctx: Context<TransferAuthority>,
        new_policy_authority: Option<Pubkey>,
        new_breaker_authority: Option<Pubkey>,
    ) -> Result<()> {
        transfer_authority::handler(ctx, new_policy_authority, new_breaker_authority)
    }

    /// Execute a queued policy loosening after the timelock delay.
    pub fn execute_pending_policy(ctx: Context<ExecutePendingPolicy>) -> Result<()> {
        execute_pending_policy::handler(ctx)
    }
}
