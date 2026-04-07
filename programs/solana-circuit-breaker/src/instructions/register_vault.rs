use anchor_lang::prelude::*;

use crate::constants::{
    BPS_DENOMINATOR, DEFAULT_COOLDOWN_SECONDS,
    GLOBAL_STATE_SEED, VAULT_POLICY_SEED,
};
use crate::error::CircuitBreakerError;
use crate::state::{GlobalState, VaultPolicy, WindowConfig, WindowState};

/// Default lockout after auto-trip: 1 hour. Nobody can reset during this period.
const DEFAULT_LOCKOUT_SECONDS: u32 = 3600;

#[derive(AnchorSerialize, AnchorDeserialize, Clone)]
pub struct RegisterVaultParams {
    pub windows: Vec<WindowConfig>,
    pub max_single_outflow_bps: u16,
    pub cooldown_seconds: u32,
    /// Minimum lockout after auto-trip. 0 = use default (1 hour).
    pub lockout_seconds: u32,
    /// Separate key for breaker reset/trip. If Pubkey::default(), uses policy_authority.
    pub breaker_authority: Pubkey,
    /// Delay before policy loosening takes effect. 0 = no timelock.
    pub policy_change_delay: u32,
}

#[derive(Accounts)]
pub struct RegisterVault<'info> {
    /// Policy authority — manages config (windows, limits, cooldown)
    #[account(mut)]
    pub policy_authority: Signer<'info>,

    /// The vault token account being protected
    /// CHECK: Any token account — protocol is responsible for passing the right one
    pub vault: UncheckedAccount<'info>,

    /// Token mint of the vault
    /// CHECK: Validated by the protocol, not by us
    pub token_mint: UncheckedAccount<'info>,

    #[account(
        init,
        payer = policy_authority,
        space = 8 + VaultPolicy::INIT_SPACE,
        seeds = [VAULT_POLICY_SEED, vault.key().as_ref()],
        bump,
    )]
    pub vault_policy: Account<'info, VaultPolicy>,

    #[account(
        mut,
        seeds = [GLOBAL_STATE_SEED],
        bump = global_state.bump,
    )]
    pub global_state: Account<'info, GlobalState>,

    pub system_program: Program<'info, System>,
}

pub fn handler(ctx: Context<RegisterVault>, params: RegisterVaultParams) -> Result<()> {
    // Validate windows
    require!(
        !params.windows.is_empty() && params.windows.len() <= 3,
        CircuitBreakerError::InvalidTimeWindow
    );
    for w in &params.windows {
        require!(w.window_seconds > 0, CircuitBreakerError::InvalidTimeWindow);
        require!(
            w.max_outflow_bps > 0 && w.max_outflow_bps <= BPS_DENOMINATOR as u16,
            CircuitBreakerError::InvalidRateLimit
        );
    }
    require!(
        params.max_single_outflow_bps > 0
            && params.max_single_outflow_bps <= BPS_DENOMINATOR as u16,
        CircuitBreakerError::InvalidRateLimit
    );

    let policy_key = ctx.accounts.policy_authority.key();
    let breaker_key = if params.breaker_authority == Pubkey::default() {
        policy_key
    } else {
        params.breaker_authority
    };

    let policy = &mut ctx.accounts.vault_policy;
    policy.policy_authority = policy_key;
    policy.breaker_authority = breaker_key;
    policy.vault = ctx.accounts.vault.key();
    policy.token_mint = ctx.accounts.token_mint.key();
    policy.max_single_outflow_bps = params.max_single_outflow_bps;
    policy.cooldown_seconds = if params.cooldown_seconds > 0 {
        params.cooldown_seconds
    } else {
        DEFAULT_COOLDOWN_SECONDS
    };
    policy.lockout_seconds = if params.lockout_seconds > 0 {
        params.lockout_seconds
    } else {
        DEFAULT_LOCKOUT_SECONDS
    };
    policy.tripped = false;
    policy.tripped_at = 0;
    policy.auto_tripped = false;
    policy.trip_count = 0;
    policy.paused = false;
    policy.policy_change_delay = params.policy_change_delay;
    policy.pending_max_single_outflow_bps = 0;
    policy.pending_cooldown_seconds = 0;
    policy.pending_change_at = 0;
    policy.bump = ctx.bumps.vault_policy;

    // Initialize window configs and states
    policy.windows = params.windows.clone();
    policy.window_states = params
        .windows
        .iter()
        .map(|_| WindowState {
            cumulative_outflow: 0,
            window_start: 0,
        })
        .collect();

    // Increment global vault count
    let state = &mut ctx.accounts.global_state;
    state.vault_count = state
        .vault_count
        .checked_add(1)
        .ok_or(CircuitBreakerError::MathOverflow)?;

    msg!(
        "Vault registered: {} | policy_authority: {} | breaker_authority: {} | lockout: {}s",
        ctx.accounts.vault.key(),
        policy_key,
        breaker_key,
        policy.lockout_seconds,
    );

    Ok(())
}
