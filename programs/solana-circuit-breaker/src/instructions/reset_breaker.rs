use anchor_lang::prelude::*;

use crate::constants::VAULT_POLICY_SEED;
use crate::error::CircuitBreakerError;
use crate::state::{BreakerResetEvent, VaultPolicy};

/// Manual reset — requires breaker_authority.
/// If the trip was automatic (rate limit detected), lockout period must have elapsed first.
/// This prevents a compromised admin from immediately resetting after an exploit detection.
#[derive(Accounts)]
pub struct ResetBreaker<'info> {
    pub breaker_authority: Signer<'info>,

    #[account(
        mut,
        seeds = [VAULT_POLICY_SEED, vault_policy.vault.as_ref()],
        bump = vault_policy.bump,
        constraint = vault_policy.breaker_authority == breaker_authority.key()
            @ CircuitBreakerError::InvalidAuthority,
    )]
    pub vault_policy: Account<'info, VaultPolicy>,
}

pub fn handler(ctx: Context<ResetBreaker>) -> Result<()> {
    let policy = &mut ctx.accounts.vault_policy;
    let now = Clock::get()?.unix_timestamp;

    require!(policy.tripped, CircuitBreakerError::BreakerNotTripped);

    // If auto-tripped, enforce lockout — nobody can reset during this period
    if policy.auto_tripped {
        let elapsed = now.saturating_sub(policy.tripped_at);
        require!(
            elapsed >= policy.lockout_seconds as i64,
            CircuitBreakerError::LockoutActive
        );
    }

    policy.tripped = false;
    policy.tripped_at = 0;
    policy.auto_tripped = false;

    // Reset all window states
    for ws in policy.window_states.iter_mut() {
        ws.cumulative_outflow = 0;
        ws.window_start = now;
    }

    emit!(BreakerResetEvent {
        vault: policy.vault,
        protocol_authority: policy.breaker_authority,
        timestamp: now,
    });

    msg!("Breaker reset for vault {}", policy.vault);
    Ok(())
}
