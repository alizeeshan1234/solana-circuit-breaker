use anchor_lang::prelude::*;

use crate::constants::GLOBAL_STATE_SEED;
use crate::state::GlobalState;

#[derive(Accounts)]
pub struct Initialize<'info> {
    #[account(mut)]
    pub admin: Signer<'info>,

    #[account(
        init,
        payer = admin,
        space = 8 + GlobalState::INIT_SPACE,
        seeds = [GLOBAL_STATE_SEED],
        bump,
    )]
    pub global_state: Account<'info, GlobalState>,

    pub system_program: Program<'info, System>,
}

pub fn handler(ctx: Context<Initialize>) -> Result<()> {
    let state = &mut ctx.accounts.global_state;
    state.admin = ctx.accounts.admin.key();
    state.vault_count = 0;
    state.total_trips = 0;
    state.bump = ctx.bumps.global_state;

    msg!("Circuit breaker protocol initialized");
    Ok(())
}
