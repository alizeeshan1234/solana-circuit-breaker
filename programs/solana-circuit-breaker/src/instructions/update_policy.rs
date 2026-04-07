use anchor_lang::prelude::*;

use crate::constants::{BPS_DENOMINATOR, VAULT_POLICY_SEED};
use crate::error::CircuitBreakerError;
use crate::state::{VaultPolicy, WindowConfig, WindowState};

#[derive(AnchorSerialize, AnchorDeserialize, Clone)]
pub struct UpdatePolicyParams {
    pub windows: Option<Vec<WindowConfig>>,
    pub max_single_outflow_bps: Option<u16>,
    pub cooldown_seconds: Option<u32>,
    pub lockout_seconds: Option<u32>,
    pub paused: Option<bool>,
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

pub fn handler(ctx: Context<UpdatePolicy>, params: UpdatePolicyParams) -> Result<()> {
    let policy = &mut ctx.accounts.vault_policy;
    let now = Clock::get()?.unix_timestamp;

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
        // Reset window states when config changes
        policy.window_states = windows
            .iter()
            .map(|_| WindowState {
                cumulative_outflow: 0,
                window_start: now,
            })
            .collect();
        policy.windows = windows;
    }

    if let Some(max_single) = params.max_single_outflow_bps {
        require!(
            max_single > 0 && max_single <= BPS_DENOMINATOR as u16,
            CircuitBreakerError::InvalidRateLimit
        );
        policy.max_single_outflow_bps = max_single;
    }

    if let Some(cooldown) = params.cooldown_seconds {
        policy.cooldown_seconds = cooldown;
    }

    if let Some(lockout) = params.lockout_seconds {
        policy.lockout_seconds = lockout;
    }

    if let Some(paused) = params.paused {
        policy.paused = paused;
    }

    msg!("Policy updated for vault {}", policy.vault);
    Ok(())
}
