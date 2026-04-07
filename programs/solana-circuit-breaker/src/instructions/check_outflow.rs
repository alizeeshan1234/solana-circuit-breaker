use anchor_lang::prelude::*;

use crate::constants::{BPS_DENOMINATOR, VAULT_POLICY_SEED};
use crate::error::CircuitBreakerError;
use crate::state::{BreakerTrippedEvent, OutflowCheckedEvent, VaultPolicy};

/// Return codes from check_outflow:
/// 0 = allowed, outflow recorded
/// 1 = blocked — breaker was already tripped
/// 2 = blocked — single tx limit exceeded, breaker now tripped
/// 3 = blocked — window rate limit exceeded, breaker now tripped
/// 4 = blocked — vault is paused
pub const OUTFLOW_ALLOWED: u8 = 0;
pub const OUTFLOW_BLOCKED_TRIPPED: u8 = 1;
pub const OUTFLOW_BLOCKED_SINGLE_TX: u8 = 2;
pub const OUTFLOW_BLOCKED_WINDOW: u8 = 3;
pub const OUTFLOW_BLOCKED_PAUSED: u8 = 4;

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

/// Check if an outflow is allowed. Always succeeds so trip state persists on-chain.
/// Returns a code: 0 = allowed, non-zero = blocked (see constants above).
/// Protocols MUST check the return value and abort the withdrawal if non-zero.
pub fn handler(ctx: Context<CheckOutflow>, amount: u64, current_tvl: u64) -> Result<u8> {
    let policy = &mut ctx.accounts.vault_policy;
    let clock = Clock::get()?;
    let now = clock.unix_timestamp;

    // Check if vault is admin-paused
    if policy.paused {
        return Ok(OUTFLOW_BLOCKED_PAUSED);
    }

    // Check if breaker is tripped
    if policy.tripped {
        let elapsed = now.saturating_sub(policy.tripped_at);

        // For auto-trips, enforce lockout period
        if policy.auto_tripped && elapsed < policy.lockout_seconds as i64 {
            return Ok(OUTFLOW_BLOCKED_TRIPPED);
        }

        // After lockout (or for manual trips), check cooldown
        if elapsed < policy.cooldown_seconds as i64 {
            return Ok(OUTFLOW_BLOCKED_TRIPPED);
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

    if amount as u128 > max_single {
        policy.tripped = true;
        policy.tripped_at = now;
        policy.auto_tripped = true;
        policy.trip_count = policy.trip_count.saturating_add(1);

        emit!(BreakerTrippedEvent {
            vault: policy.vault,
            protocol_authority: policy.policy_authority,
            outflow_amount: amount,
            tvl: current_tvl,
            window_index: u8::MAX,
            timestamp: now,
        });

        return Ok(OUTFLOW_BLOCKED_SINGLE_TX);
    }

    // Check 2: Rolling window rate limits
    let num_windows = policy.windows.len();
    for i in 0..num_windows {
        let window_seconds = policy.windows[i].window_seconds;
        let max_outflow_bps = policy.windows[i].max_outflow_bps;

        let window_elapsed = now.saturating_sub(policy.window_states[i].window_start);

        if window_elapsed >= window_seconds as i64 {
            policy.window_states[i].cumulative_outflow = 0;
            policy.window_states[i].window_start = now;
        }

        let new_cumulative = policy.window_states[i]
            .cumulative_outflow
            .checked_add(amount)
            .ok_or(CircuitBreakerError::MathOverflow)?;

        let max_window_outflow = (current_tvl as u128)
            .checked_mul(max_outflow_bps as u128)
            .ok_or(CircuitBreakerError::MathOverflow)?
            / BPS_DENOMINATOR as u128;

        if new_cumulative as u128 > max_window_outflow {
            policy.tripped = true;
            policy.tripped_at = now;
            policy.auto_tripped = true;
            policy.trip_count = policy.trip_count.saturating_add(1);

            emit!(BreakerTrippedEvent {
                vault: policy.vault,
                protocol_authority: policy.policy_authority,
                outflow_amount: amount,
                tvl: current_tvl,
                window_index: i as u8,
                timestamp: now,
            });

            return Ok(OUTFLOW_BLOCKED_WINDOW);
        }

        policy.window_states[i].cumulative_outflow = new_cumulative;
    }

    emit!(OutflowCheckedEvent {
        vault: policy.vault,
        amount,
        tvl: current_tvl,
        timestamp: now,
    });

    Ok(OUTFLOW_ALLOWED)
}
