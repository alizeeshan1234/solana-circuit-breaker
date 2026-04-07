use anchor_lang::prelude::*;

use crate::constants::{BPS_DENOMINATOR, VAULT_POLICY_SEED};
use crate::error::CircuitBreakerError;
use crate::state::{OutflowCheckedEvent, VaultPolicy};

#[derive(Accounts)]
pub struct CheckOutflow<'info> {
    /// Protocol calling the circuit breaker before an outflow.
    /// Must be the policy_authority that registered the vault.
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

/// Check if an outflow is allowed. Errors when blocked — protocols cannot ignore it.
/// On success, records the outflow in the rolling window.
/// On failure (limits exceeded or breaker tripped), the tx rolls back — no state changes.
pub fn handler(ctx: Context<CheckOutflow>, amount: u64, current_tvl: u64) -> Result<()> {
    let policy = &mut ctx.accounts.vault_policy;
    let clock = Clock::get()?;
    let now = clock.unix_timestamp;

    // Check if vault is admin-paused
    require!(!policy.paused, CircuitBreakerError::VaultPaused);

    // Check if breaker is tripped (via manual trip or previous auto-trip that persisted)
    if policy.tripped {
        let elapsed = now.saturating_sub(policy.tripped_at);

        // For auto-trips, enforce lockout period
        if policy.auto_tripped && elapsed < policy.lockout_seconds as i64 {
            return Err(CircuitBreakerError::BreakerTripped.into());
        }

        // After lockout (or for manual trips), check cooldown
        if elapsed < policy.cooldown_seconds as i64 {
            return Err(CircuitBreakerError::BreakerTripped.into());
        }

        // Cooldown expired — auto-reset
        policy.tripped = false;
        policy.tripped_at = 0;
        policy.auto_tripped = false;
        for ws in policy.window_states.iter_mut() {
            ws.cumulative_outflow = 0;
            ws.window_start = now;
        }
    }

    require!(amount > 0, CircuitBreakerError::ZeroOutflow);
    require!(current_tvl > 0, CircuitBreakerError::ZeroTvl);

    // Check 1: Single transaction limit
    let max_single = (current_tvl as u128)
        .checked_mul(policy.max_single_outflow_bps as u128)
        .ok_or(CircuitBreakerError::MathOverflow)?
        / BPS_DENOMINATOR as u128;

    require!(
        (amount as u128) <= max_single,
        CircuitBreakerError::MaxSingleOutflowExceeded
    );

    // Check 2: Rolling window rate limits
    // First pass: validate all windows BEFORE updating any state.
    // This ensures no partial state changes if a later window fails.
    let num_windows = policy.windows.len();
    let mut new_cumulatives = Vec::with_capacity(num_windows);

    for i in 0..num_windows {
        let window_seconds = policy.windows[i].window_seconds;
        let max_outflow_bps = policy.windows[i].max_outflow_bps;

        let window_elapsed = now.saturating_sub(policy.window_states[i].window_start);
        let current_cumulative = if window_elapsed >= window_seconds as i64 {
            0 // window expired, will reset
        } else {
            policy.window_states[i].cumulative_outflow
        };

        let new_cumulative = current_cumulative
            .checked_add(amount)
            .ok_or(CircuitBreakerError::MathOverflow)?;

        let max_window_outflow = (current_tvl as u128)
            .checked_mul(max_outflow_bps as u128)
            .ok_or(CircuitBreakerError::MathOverflow)?
            / BPS_DENOMINATOR as u128;

        require!(
            (new_cumulative as u128) <= max_window_outflow,
            CircuitBreakerError::RateLimitExceeded
        );

        new_cumulatives.push(new_cumulative);
    }

    // Second pass: all windows passed — commit the state changes
    for i in 0..num_windows {
        let window_seconds = policy.windows[i].window_seconds;
        let window_elapsed = now.saturating_sub(policy.window_states[i].window_start);

        if window_elapsed >= window_seconds as i64 {
            policy.window_states[i].window_start = now;
        }
        policy.window_states[i].cumulative_outflow = new_cumulatives[i];
    }

    emit!(OutflowCheckedEvent {
        vault: policy.vault,
        amount,
        tvl: current_tvl,
        timestamp: now,
    });

    Ok(())
}
