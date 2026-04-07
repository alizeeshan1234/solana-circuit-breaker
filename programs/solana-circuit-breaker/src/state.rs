use anchor_lang::prelude::*;

use crate::constants::MAX_WINDOWS;

/// Global state for the circuit breaker protocol.
#[account]
#[derive(InitSpace)]
pub struct GlobalState {
    /// Protocol admin who can update global settings
    pub admin: Pubkey,
    /// Total number of vaults registered
    pub vault_count: u64,
    /// Total number of times any breaker has tripped (protocol-wide metric)
    pub total_trips: u64,
    /// PDA bump
    pub bump: u8,
}

/// Configuration for a single time window rate limit.
/// Example: max 10% outflow in 5 minutes.
#[derive(AnchorSerialize, AnchorDeserialize, Clone, Copy, InitSpace, Default, Debug)]
pub struct WindowConfig {
    /// Duration of the rolling window in seconds
    pub window_seconds: u32,
    /// Maximum outflow as basis points of TVL (e.g. 1000 = 10%)
    pub max_outflow_bps: u16,
}

/// Tracks cumulative outflow for a single time window.
#[derive(AnchorSerialize, AnchorDeserialize, Clone, Copy, InitSpace, Default, Debug)]
pub struct WindowState {
    /// Cumulative outflow in this window (token amount, not USD)
    pub cumulative_outflow: u64,
    /// Slot at which this window started
    pub window_start: i64,
}

/// Per-vault policy and state. Each vault a protocol registers gets one of these.
#[account]
#[derive(InitSpace)]
pub struct VaultPolicy {
    /// Authority that can update policy (windows, limits, cooldown).
    /// This is the "admin" role — manages configuration.
    pub policy_authority: Pubkey,
    /// Authority that can manually trip/reset the breaker.
    /// Should be a DIFFERENT key (or multisig) from policy_authority.
    pub breaker_authority: Pubkey,
    /// The vault (token account) this policy protects
    pub vault: Pubkey,
    /// Token mint of the vault
    pub token_mint: Pubkey,

    // --- Rate limit config ---
    /// Up to MAX_WINDOWS independent time windows
    #[max_len(MAX_WINDOWS)]
    pub windows: Vec<WindowConfig>,
    /// Maximum single-transaction outflow in BPS of TVL
    pub max_single_outflow_bps: u16,

    // --- Cooldown config ---
    /// How long the breaker stays tripped (seconds)
    pub cooldown_seconds: u32,
    /// Minimum lockout after an AUTO-trip (seconds). Nobody can reset during this period.
    /// Protects against compromised admin — even with all keys, must wait.
    pub lockout_seconds: u32,

    // --- Runtime state ---
    /// Rolling window tracking (parallel to windows config)
    #[max_len(MAX_WINDOWS)]
    pub window_states: Vec<WindowState>,
    /// Whether the breaker is currently tripped
    pub tripped: bool,
    /// When the breaker was tripped (unix timestamp)
    pub tripped_at: i64,
    /// Whether the trip was automatic (rate limit) or manual (panic button)
    pub auto_tripped: bool,
    /// Total times this vault's breaker has tripped
    pub trip_count: u64,
    /// Admin can pause vault manually
    pub paused: bool,

    /// PDA bump
    pub bump: u8,
}

/// Event emitted when a breaker trips.
#[event]
pub struct BreakerTrippedEvent {
    pub vault: Pubkey,
    pub protocol_authority: Pubkey,
    pub outflow_amount: u64,
    pub tvl: u64,
    pub window_index: u8,
    pub timestamp: i64,
}

/// Event emitted when a breaker is reset.
#[event]
pub struct BreakerResetEvent {
    pub vault: Pubkey,
    pub protocol_authority: Pubkey,
    pub timestamp: i64,
}

/// Event emitted when an outflow check passes.
#[event]
pub struct OutflowCheckedEvent {
    pub vault: Pubkey,
    pub amount: u64,
    pub tvl: u64,
    pub timestamp: i64,
}
