pub const GLOBAL_STATE_SEED: &[u8] = b"global_state";
pub const VAULT_POLICY_SEED: &[u8] = b"vault_policy";
pub const OUTFLOW_WINDOW_SEED: &[u8] = b"outflow_window";

/// Basis points denominator (100% = 10_000 BPS)
pub const BPS_DENOMINATOR: u64 = 10_000;

/// Maximum number of time windows a vault can track simultaneously
pub const MAX_WINDOWS: usize = 3;

/// Default cooldown after breaker trips (5 minutes)
pub const DEFAULT_COOLDOWN_SECONDS: u32 = 300;

/// Maximum single outflow as % of TVL (default 5% = 500 BPS)
pub const DEFAULT_MAX_SINGLE_OUTFLOW_BPS: u16 = 500;
