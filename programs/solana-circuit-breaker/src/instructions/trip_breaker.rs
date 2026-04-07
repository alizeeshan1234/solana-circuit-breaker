use anchor_lang::prelude::*;

use crate::constants::VAULT_POLICY_SEED;
use crate::error::CircuitBreakerError;
use crate::state::{BreakerTrippedEvent, VaultPolicy};

/// Manual emergency trip — requires breaker_authority (separate from policy_authority).
#[derive(Accounts)]
pub struct TripBreaker<'info> {
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

pub fn handler(ctx: Context<TripBreaker>) -> Result<()> {
    let policy = &mut ctx.accounts.vault_policy;
    let now = Clock::get()?.unix_timestamp;

    policy.tripped = true;
    policy.tripped_at = now;
    policy.auto_tripped = false; // manual trip — no lockout enforced
    policy.trip_count = policy.trip_count.saturating_add(1);

    emit!(BreakerTrippedEvent {
        vault: policy.vault,
        protocol_authority: policy.breaker_authority,
        outflow_amount: 0,
        tvl: 0,
        window_index: u8::MAX,
        timestamp: now,
    });

    msg!("Breaker manually tripped for vault {}", policy.vault);
    Ok(())
}
