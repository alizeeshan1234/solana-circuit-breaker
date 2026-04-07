use anchor_lang::prelude::*;

use crate::constants::VAULT_POLICY_SEED;
use crate::error::CircuitBreakerError;
use crate::state::{PolicyUpdatedEvent, VaultPolicy};

/// Execute a pending policy change after the timelock delay has elapsed.
#[derive(Accounts)]
pub struct ExecutePendingPolicy<'info> {
    pub policy_authority: Signer<'info>,

    #[account(
        mut,
        seeds = [VAULT_POLICY_SEED, vault_policy.vault.as_ref()],
        bump = vault_policy.bump,
        constraint = vault_policy.policy_authority == policy_authority.key()
            @ CircuitBreakerError::InvalidAuthority,
    )]
    pub vault_policy: Account<'info, VaultPolicy>,
}

pub fn handler(ctx: Context<ExecutePendingPolicy>) -> Result<()> {
    let policy = &mut ctx.accounts.vault_policy;
    let now = Clock::get()?.unix_timestamp;

    // Must have a pending change
    require!(
        policy.pending_change_at > 0,
        CircuitBreakerError::NoPendingChange
    );

    // Delay must have elapsed
    let elapsed = now.saturating_sub(policy.pending_change_at);
    require!(
        elapsed >= policy.policy_change_delay as i64,
        CircuitBreakerError::PolicyChangeDelayNotElapsed
    );

    // Apply pending changes
    if policy.pending_max_single_outflow_bps > 0 {
        msg!(
            "Applying pending max_single_outflow_bps: {} → {}",
            policy.max_single_outflow_bps,
            policy.pending_max_single_outflow_bps
        );
        policy.max_single_outflow_bps = policy.pending_max_single_outflow_bps;
        policy.pending_max_single_outflow_bps = 0;
    }

    if policy.pending_cooldown_seconds > 0 {
        msg!(
            "Applying pending cooldown: {} → {}",
            policy.cooldown_seconds,
            policy.pending_cooldown_seconds
        );
        policy.cooldown_seconds = policy.pending_cooldown_seconds;
        policy.pending_cooldown_seconds = 0;
    }

    policy.pending_change_at = 0;

    emit!(PolicyUpdatedEvent {
        vault: policy.vault,
        policy_authority: policy.policy_authority,
        max_single_outflow_bps: policy.max_single_outflow_bps,
        cooldown_seconds: policy.cooldown_seconds,
        lockout_seconds: policy.lockout_seconds,
        paused: policy.paused,
        timestamp: now,
    });

    Ok(())
}
