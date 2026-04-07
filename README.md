# Solana Circuit Breaker

An on-chain circuit breaker program for Solana DeFi protocols. It monitors token outflows from vaults and automatically halts operations when anomalous withdrawal patterns are detected — like an electrical circuit breaker that cuts power when too much current flows.

## The Problem

When a DeFi protocol gets exploited, the attacker drains the entire vault in seconds. By the time the team notices, the funds are gone. Every major DeFi hack could have been mitigated if there was an automatic system that detected unusual outflows and froze the vault before it was fully drained.

## How It Works

Any DeFi protocol can register its vaults with the circuit breaker. Before every withdrawal, the protocol makes a CPI call to `check_outflow`. The circuit breaker tracks outflows across rolling time windows and automatically trips (freezes all outflows) if limits are exceeded.

```
Normal day:
  Withdrawal 1:  2% of TVL  → total last 5 min: 2%  → ok
  Withdrawal 2:  3% of TVL  → total last 5 min: 5%  → ok
  Withdrawal 3:  3% of TVL  → total last 5 min: 8%  → ok

Attack day:
  Withdrawal 1:  2% of TVL  → total last 5 min: 2%  → ok
  Withdrawal 2:  3% of TVL  → total last 5 min: 5%  → ok
  Withdrawal 3:  6% of TVL  → total last 5 min: 11% → TRIPPED (over 10% limit)
  Withdrawal 4:  any amount  → BLOCKED (breaker is tripped)
```

## Features

- **Rolling time windows** — Up to 3 independent rate limit windows per vault (e.g. 10% in 5 min, 25% in 1 hour, 50% in 24 hours)
- **Single-transaction limit** — Blocks any single withdrawal that exceeds X% of TVL
- **Auto-trip** — Breaker trips automatically when limits are exceeded, no human needed
- **Lockout mode** — When auto-tripped, nobody can reset the breaker for a configurable period (default 1 hour), even with admin keys. Protects against compromised admin scenarios.
- **Role separation** — Two separate authorities: `policy_authority` (manages config) and `breaker_authority` (trips/resets). Compromising one key doesn't give full control.
- **Manual panic button** — `breaker_authority` can trip the breaker instantly in an emergency
- **Auto-reset** — After cooldown expires, the breaker resets automatically
- **Events** — Emits events on every trip, reset, and successful outflow check for off-chain monitoring

## Architecture

### Accounts

**GlobalState** — Protocol-wide state (one per deployment)
- `admin` — Protocol admin
- `vault_count` — Number of registered vaults
- `total_trips` — Total trips across all vaults

**VaultPolicy** — Per-vault configuration and runtime state (PDA seeded by vault address)
- `policy_authority` — Can update config (windows, limits, cooldown, lockout)
- `breaker_authority` — Can manually trip/reset the breaker (should be a different key or multisig)
- `windows` — Rate limit window configurations
- `window_states` — Rolling outflow tracking per window
- `lockout_seconds` — Minimum time after auto-trip where nobody can reset
- `tripped` / `auto_tripped` — Current breaker state

### Instructions

| Instruction | Who calls it | What it does |
|---|---|---|
| `initialize` | Deployer (once) | Creates the global state |
| `register_vault` | Protocol admin | Registers a vault with rate limit windows and authorities |
| `check_outflow` | Protocol (every withdrawal) | Checks if a withdrawal is allowed. Auto-trips if limits exceeded. |
| `trip_breaker` | `breaker_authority` | Manual emergency freeze |
| `reset_breaker` | `breaker_authority` | Manual reset (blocked during lockout for auto-trips) |
| `update_policy` | `policy_authority` | Update windows, limits, cooldown, lockout, or pause |

### Security Model

**Two-key separation:**
```
policy_authority  → manages config (windows, limits, cooldown)
breaker_authority → controls breaker (trip, reset)
```
Attacker steals the policy key? They can change config but can't reset a tripped breaker.
Attacker steals the breaker key? They can reset but can't loosen the limits.
Need BOTH keys to fully compromise.

**Lockout protection:**
```
Auto-trip (exploit detected)  → locked for 1 hour, nobody can reset
Manual trip (panic button)    → breaker_authority can reset immediately
```
If admin keys are compromised and the attacker drains 10%, the breaker auto-trips and locks for 1 hour. The attacker can't reset it even with all the keys. The team has 1 hour to respond.

## Integration Guide

### Step 1: Add the circuit breaker as a dependency

In your protocol's `Cargo.toml`:

```toml
[dependencies]
# Anchor 1.0.0 (solana 3.x)
solana-circuit-breaker = { git = "https://github.com/alizeeshan1234/solana-circuit-breaker", branch = "main", features = ["cpi"] }

# Anchor 0.32.1 (solana 2.x)
solana-circuit-breaker = { git = "https://github.com/alizeeshan1234/solana-circuit-breaker", branch = "anchor-032", features = ["cpi"] }

# Or local path (for development)
# solana-circuit-breaker = { path = "../solana-circuit-breaker/programs/solana-circuit-breaker", features = ["cpi"] }
```

The program code is identical across both branches — only the Anchor/Solana dependency versions differ. Pick the branch that matches your project's Anchor version.

### Step 2: Register your vault

Call `register_vault` once per vault during your protocol's setup:

```rust
// In your protocol's initialization or admin instruction
use solana_circuit_breaker::cpi::accounts::RegisterVault;
use solana_circuit_breaker::cpi::register_vault;
use solana_circuit_breaker::RegisterVaultParams;
use solana_circuit_breaker::state::WindowConfig;

let params = RegisterVaultParams {
    windows: vec![
        WindowConfig { window_seconds: 300, max_outflow_bps: 1000 },   // 10% per 5 min
        WindowConfig { window_seconds: 3600, max_outflow_bps: 2500 },  // 25% per 1 hour
        WindowConfig { window_seconds: 86400, max_outflow_bps: 5000 }, // 50% per 24 hours
    ],
    max_single_outflow_bps: 500,  // max 5% per single transaction
    cooldown_seconds: 300,         // 5 min cooldown after trip
    lockout_seconds: 3600,         // 1 hour lockout on auto-trip
    breaker_authority: your_multisig_key, // separate key for breaker control
};

let cpi_accounts = RegisterVault {
    policy_authority: ctx.accounts.admin.to_account_info(),
    vault: ctx.accounts.your_vault.to_account_info(),
    token_mint: ctx.accounts.token_mint.to_account_info(),
    vault_policy: ctx.accounts.vault_policy.to_account_info(),
    global_state: ctx.accounts.circuit_breaker_global.to_account_info(),
    system_program: ctx.accounts.system_program.to_account_info(),
};

let cpi_ctx = CpiContext::new(ctx.accounts.circuit_breaker_program.to_account_info(), cpi_accounts);
register_vault(cpi_ctx, params)?;
```

### Step 3: Add the check before every withdrawal

This is the core integration — one CPI call before any outflow:

```rust
use solana_circuit_breaker::cpi::accounts::CheckOutflow;
use solana_circuit_breaker::cpi::check_outflow;

// In your withdrawal/remove_liquidity/swap instruction:
pub fn handle_withdrawal(ctx: Context<YourWithdrawal>, amount: u64) -> Result<()> {
    // Get current TVL of the vault
    let current_tvl = ctx.accounts.vault.amount;

    // Ask circuit breaker: is this withdrawal safe?
    let cpi_accounts = CheckOutflow {
        policy_authority: ctx.accounts.your_authority.to_account_info(),
        vault_policy: ctx.accounts.vault_policy.to_account_info(),
    };
    let cpi_ctx = CpiContext::new(
        ctx.accounts.circuit_breaker_program.to_account_info(),
        cpi_accounts,
    );
    check_outflow(cpi_ctx, amount, current_tvl)?;

    // If we reach here, the withdrawal is allowed
    // ... proceed with your normal withdrawal logic ...
    transfer_tokens(amount)?;

    Ok(())
}
```

If `check_outflow` returns an error, the entire transaction fails and no tokens move. The circuit breaker auto-trips and blocks all subsequent withdrawals until the cooldown expires or `breaker_authority` resets it.

### Step 4: Add accounts to your instruction context

You'll need to pass the circuit breaker accounts in your withdrawal instruction:

```rust
#[derive(Accounts)]
pub struct YourWithdrawal<'info> {
    // ... your existing accounts ...

    /// CHECK: Circuit breaker vault policy PDA
    #[account(mut)]
    pub vault_policy: UncheckedAccount<'info>,

    /// Circuit breaker program
    pub circuit_breaker_program: Program<'info, SolanaCircuitBreaker>,
}
```

The `vault_policy` PDA is derived from: `seeds = [b"vault_policy", your_vault_pubkey.as_ref()]` using the circuit breaker program ID.

### Step 5: Set up monitoring (recommended)

Listen for `BreakerTrippedEvent` events from the circuit breaker program to get instant alerts when a breaker trips:

```typescript
program.addEventListener("BreakerTrippedEvent", (event) => {
  console.log(`ALERT: Breaker tripped on vault ${event.vault}`);
  console.log(`Outflow: ${event.outflowAmount}, TVL: ${event.tvl}`);
  console.log(`Window: ${event.windowIndex}, Time: ${event.timestamp}`);
  // Send alert to Discord, Telegram, PagerDuty, etc.
});
```

## Tuning Guidelines

| Protocol Type | Single TX Limit | 5 min Window | 1 hour Window | Lockout |
|---|---|---|---|---|
| Small DEX pool ($500k) | 5% (500 bps) | 10% (1000 bps) | 25% (2500 bps) | 1 hour |
| Large lending protocol ($100M) | 2% (200 bps) | 5% (500 bps) | 15% (1500 bps) | 2 hours |
| Stablecoin vault | 10% (1000 bps) | 20% (2000 bps) | 40% (4000 bps) | 30 min |
| Treasury/multisig | 1% (100 bps) | 3% (300 bps) | 10% (1000 bps) | 4 hours |

Start conservative and loosen based on real usage data. A few false alarms (legit traders waiting 5 minutes) is better than missing an exploit.

## What it doesn't protect against

- **Compromised program upgrade authority** — If the attacker can upgrade the protocol's program, they can remove the circuit breaker call entirely. Use immutable programs or multisig upgrade authority.
- **Slow drains** — An attacker who drains small amounts over days/weeks stays under the rate limits. Use off-chain monitoring for this.
- **Inflow attacks** — The circuit breaker only monitors outflows. Oracle manipulation or price attacks that inflate balances before withdrawing need separate protection.

## License

MIT
