use anchor_lang::prelude::*;

use crate::constants::{BPS_DENOMINATOR, VAULT_POLICY_SEED};
use crate::error::CircuitBreakerError;
use crate::state::{PolicyUpdatedEvent, VaultPolicy, WindowConfig, WindowState};

#[derive(AnchorSerialize, AnchorDeserialize, Clone)]
pub struct UpdatePolicyParams {
    /// New windows config. Only applied if tightening (lower bps). Loosening is rejected — use propose/execute.
    pub windows: Option<Vec<WindowConfig>>,
    /// New single tx limit. Applied immediately if tightening, queued if loosening.
    pub max_single_outflow_bps: Option<u16>,
    /// New cooldown. Applied immediately if tightening (shorter), queued if loosening.
    pub cooldown_seconds: Option<u32>,
    pub lockout_seconds: Option<u32>,
    pub paused: Option<bool>,
    pub policy_change_delay: Option<u32>,
}

#[derive(Accounts)]
pub struct UpdatePolicy<'info> {
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

/// Returns true if the new bps value is "loosening" (making limits less restrictive).
fn is_loosening_bps(current: u16, new: u16) -> bool {
    new > current
}

/// Returns true if the new cooldown is "loosening" (shorter cooldown = less restrictive).
fn is_loosening_cooldown(current: u32, new: u32) -> bool {
    new < current
}

pub fn handler(ctx: Context<UpdatePolicy>, params: UpdatePolicyParams) -> Result<()> {
    let policy = &mut ctx.accounts.vault_policy;
    let now = Clock::get()?.unix_timestamp;

    // Windows: only allow tightening immediately. Reject loosening entirely — re-register instead.
    if let Some(windows) = params.windows {
        require!(
            !windows.is_empty() && windows.len() <= 3,
            CircuitBreakerError::InvalidTimeWindow
        );
        for w in &windows {
            require!(w.window_seconds > 0, CircuitBreakerError::InvalidTimeWindow);
            require!(
                w.max_outflow_bps > 0 && w.max_outflow_bps <= BPS_DENOMINATOR as u16,
                CircuitBreakerError::InvalidRateLimit
            );
        }

        // Check that all windows are tightening (lower or equal bps)
        for (i, new_w) in windows.iter().enumerate() {
            if i < policy.windows.len() {
                require!(
                    new_w.max_outflow_bps <= policy.windows[i].max_outflow_bps,
                    CircuitBreakerError::PolicyChangeDelayNotElapsed
                );
            }
        }

        policy.window_states = windows
            .iter()
            .map(|_| WindowState {
                cumulative_outflow: 0,
                window_start: now,
            })
            .collect();
        policy.windows = windows;
    }

    // Single tx limit
    if let Some(max_single) = params.max_single_outflow_bps {
        require!(
            max_single > 0 && max_single <= BPS_DENOMINATOR as u16,
            CircuitBreakerError::InvalidRateLimit
        );

        if policy.policy_change_delay > 0 && is_loosening_bps(policy.max_single_outflow_bps, max_single) {
            // Queue the change
            policy.pending_max_single_outflow_bps = max_single;
            policy.pending_change_at = now;
            msg!("Loosening queued: max_single_outflow_bps {} → {} (effective after {}s)",
                policy.max_single_outflow_bps, max_single, policy.policy_change_delay);
        } else {
            // Tightening or no delay — apply immediately
            policy.max_single_outflow_bps = max_single;
        }
    }

    // Cooldown
    if let Some(cooldown) = params.cooldown_seconds {
        if policy.policy_change_delay > 0 && is_loosening_cooldown(policy.cooldown_seconds, cooldown) {
            // Queue the change
            policy.pending_cooldown_seconds = cooldown;
            if policy.pending_change_at == 0 {
                policy.pending_change_at = now;
            }
            msg!("Loosening queued: cooldown {} → {} (effective after {}s)",
                policy.cooldown_seconds, cooldown, policy.policy_change_delay);
        } else {
            policy.cooldown_seconds = cooldown;
        }
    }

    if let Some(lockout) = params.lockout_seconds {
        policy.lockout_seconds = lockout;
    }

    if let Some(paused) = params.paused {
        policy.paused = paused;
    }

    if let Some(delay) = params.policy_change_delay {
        policy.policy_change_delay = delay;
    }

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
