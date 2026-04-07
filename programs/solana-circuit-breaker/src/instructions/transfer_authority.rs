use anchor_lang::prelude::*;

use crate::constants::VAULT_POLICY_SEED;
use crate::error::CircuitBreakerError;
use crate::state::{AuthorityTransferredEvent, VaultPolicy};

#[derive(Accounts)]
pub struct TransferAuthority<'info> {
    /// Current policy_authority must sign to transfer either authority.
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

pub fn handler(
    ctx: Context<TransferAuthority>,
    new_policy_authority: Option<Pubkey>,
    new_breaker_authority: Option<Pubkey>,
) -> Result<()> {
    let policy = &mut ctx.accounts.vault_policy;
    let old_policy = policy.policy_authority;
    let old_breaker = policy.breaker_authority;

    if let Some(new_policy) = new_policy_authority {
        require!(
            new_policy != Pubkey::default(),
            CircuitBreakerError::InvalidAuthority
        );
        msg!(
            "Policy authority transferred: {} → {}",
            policy.policy_authority,
            new_policy
        );
        policy.policy_authority = new_policy;
    }

    if let Some(new_breaker) = new_breaker_authority {
        require!(
            new_breaker != Pubkey::default(),
            CircuitBreakerError::InvalidAuthority
        );
        msg!(
            "Breaker authority transferred: {} → {}",
            policy.breaker_authority,
            new_breaker
        );
        policy.breaker_authority = new_breaker;
    }

    emit!(AuthorityTransferredEvent {
        vault: policy.vault,
        old_policy_authority: old_policy,
        new_policy_authority: policy.policy_authority,
        old_breaker_authority: old_breaker,
        new_breaker_authority: policy.breaker_authority,
        timestamp: Clock::get()?.unix_timestamp,
    });

    Ok(())
}

