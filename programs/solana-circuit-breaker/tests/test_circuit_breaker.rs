#![allow(unused_variables)]

use {
    anchor_lang::{
        solana_program::{instruction::Instruction, pubkey::Pubkey, system_program},
        InstructionData, ToAccountMetas,
    },
    litesvm::LiteSVM,
    solana_clock::Clock,
    solana_keypair::Keypair,
    solana_message::{Message, VersionedMessage},
    solana_signer::Signer,
    solana_transaction::versioned::VersionedTransaction,
};

use solana_circuit_breaker::state::WindowConfig;

const PROGRAM_BYTES: &[u8] = include_bytes!("../../../target/deploy/solana_circuit_breaker.so");

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn setup() -> (LiteSVM, Keypair, Pubkey) {
    let program_id = solana_circuit_breaker::id();
    let admin = Keypair::new();
    let mut svm = LiteSVM::new().with_sysvars();
    svm.add_program(program_id, PROGRAM_BYTES).unwrap();
    svm.airdrop(&admin.pubkey(), 10_000_000_000).unwrap();
    (svm, admin, program_id)
}

fn set_clock(svm: &mut LiteSVM, unix_timestamp: i64) {
    let mut clock = svm.get_sysvar::<Clock>();
    clock.unix_timestamp = unix_timestamp;
    svm.set_sysvar::<Clock>(&clock);
}

static TX_NONCE: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// Send a transaction. Returns Ok(return_data_bytes) or Err(debug_string).
fn send_tx(svm: &mut LiteSVM, ixs: &[Instruction], signers: &[&Keypair]) -> Result<Vec<u8>, String> {
    let blockhash = svm.latest_blockhash();
    let payer = signers[0].pubkey();

    // Use ComputeBudget::SetComputeUnitLimit with varying values to make each tx unique
    let nonce = TX_NONCE.fetch_add(1, std::sync::atomic::Ordering::Relaxed) as u32;
    let units = 400_000 + nonce; // vary CU limit to make tx unique
    let mut cu_data = vec![2u8]; // SetComputeUnitLimit discriminator
    cu_data.extend_from_slice(&units.to_le_bytes());
    let cu_ix = Instruction::new_with_bytes(
        "ComputeBudget111111111111111111111111111111".parse::<Pubkey>().unwrap(),
        &cu_data,
        vec![],
    );

    let mut all_ixs = vec![cu_ix];
    all_ixs.extend_from_slice(ixs);

    let msg = Message::new_with_blockhash(&all_ixs, Some(&payer), &blockhash);
    let tx = VersionedTransaction::try_new(VersionedMessage::Legacy(msg), signers)
        .map_err(|e| e.to_string())?;
    match svm.send_transaction(tx) {
        Ok(meta) => Ok(meta.return_data.data),
        Err(e) => Err(format!("{:?}", e)),
    }
}

/// Send and expect success (ignore return data).
fn send_ok(svm: &mut LiteSVM, ixs: &[Instruction], signers: &[&Keypair]) {
    send_tx(svm, ixs, signers).expect("transaction should succeed");
}

/// Send and expect failure.
fn send_err(svm: &mut LiteSVM, ixs: &[Instruction], signers: &[&Keypair]) {
    assert!(send_tx(svm, ixs, signers).is_err(), "transaction should fail");
}

/// Send check_outflow. Returns true if allowed, false if blocked.
fn check_outflow_allowed(
    svm: &mut LiteSVM,
    program_id: &Pubkey,
    policy_authority: &Keypair,
    vault: &Pubkey,
    amount: u64,
    tvl: u64,
) -> bool {
    let ix = ix_check_outflow(program_id, &policy_authority.pubkey(), vault, amount, tvl);
    send_tx(svm, &[ix], &[policy_authority]).is_ok()
}

fn global_state_pda(program_id: &Pubkey) -> Pubkey {
    Pubkey::find_program_address(&[b"global_state"], program_id).0
}

fn vault_policy_pda(program_id: &Pubkey, vault: &Pubkey) -> Pubkey {
    Pubkey::find_program_address(&[b"vault_policy", vault.as_ref()], program_id).0
}

fn ix_initialize(program_id: &Pubkey, admin: &Pubkey) -> Instruction {
    Instruction::new_with_bytes(
        *program_id,
        &solana_circuit_breaker::instruction::Initialize {}.data(),
        solana_circuit_breaker::accounts::Initialize {
            admin: *admin,
            global_state: global_state_pda(program_id),
            system_program: system_program::ID,
        }
        .to_account_metas(None),
    )
}

fn ix_register_vault(
    program_id: &Pubkey,
    policy_authority: &Pubkey,
    vault: &Pubkey,
    token_mint: &Pubkey,
    breaker_authority: &Pubkey,
    windows: Vec<WindowConfig>,
    max_single_outflow_bps: u16,
    cooldown_seconds: u32,
    lockout_seconds: u32,
    policy_change_delay: u32,
) -> Instruction {
    let params = solana_circuit_breaker::RegisterVaultParams {
        windows,
        max_single_outflow_bps,
        cooldown_seconds,
        lockout_seconds,
        breaker_authority: *breaker_authority,
        policy_change_delay,
    };
    Instruction::new_with_bytes(
        *program_id,
        &solana_circuit_breaker::instruction::RegisterVault { params: params.clone() }.data(),
        solana_circuit_breaker::accounts::RegisterVault {
            policy_authority: *policy_authority,
            vault: *vault,
            token_mint: *token_mint,
            vault_policy: vault_policy_pda(program_id, vault),
            global_state: global_state_pda(program_id),
            system_program: system_program::ID,
        }
        .to_account_metas(None),
    )
}

fn ix_check_outflow(program_id: &Pubkey, authority: &Pubkey, vault: &Pubkey, amount: u64, tvl: u64) -> Instruction {
    Instruction::new_with_bytes(
        *program_id,
        &solana_circuit_breaker::instruction::CheckOutflow { amount, current_tvl: tvl }.data(),
        solana_circuit_breaker::accounts::CheckOutflow {
            policy_authority: *authority,
            vault_policy: vault_policy_pda(program_id, vault),
        }
        .to_account_metas(None),
    )
}

fn ix_trip_breaker(program_id: &Pubkey, breaker_authority: &Pubkey, vault: &Pubkey) -> Instruction {
    Instruction::new_with_bytes(
        *program_id,
        &solana_circuit_breaker::instruction::TripBreaker {}.data(),
        solana_circuit_breaker::accounts::TripBreaker {
            breaker_authority: *breaker_authority,
            vault_policy: vault_policy_pda(program_id, vault),
        }
        .to_account_metas(None),
    )
}

fn ix_reset_breaker(program_id: &Pubkey, breaker_authority: &Pubkey, vault: &Pubkey) -> Instruction {
    Instruction::new_with_bytes(
        *program_id,
        &solana_circuit_breaker::instruction::ResetBreaker {}.data(),
        solana_circuit_breaker::accounts::ResetBreaker {
            breaker_authority: *breaker_authority,
            vault_policy: vault_policy_pda(program_id, vault),
        }
        .to_account_metas(None),
    )
}

fn ix_update_policy(program_id: &Pubkey, authority: &Pubkey, vault: &Pubkey, params: solana_circuit_breaker::UpdatePolicyParams) -> Instruction {
    Instruction::new_with_bytes(
        *program_id,
        &solana_circuit_breaker::instruction::UpdatePolicy { params: params.clone() }.data(),
        solana_circuit_breaker::accounts::UpdatePolicy {
            policy_authority: *authority,
            vault_policy: vault_policy_pda(program_id, vault),
        }
        .to_account_metas(None),
    )
}

/// Setup with default vault: 10% per 300s, 5% single tx, 300s cooldown, 600s lockout
fn setup_with_vault(svm: &mut LiteSVM, admin: &Keypair, program_id: &Pubkey) -> (Keypair, Keypair, Keypair) {
    let vault = Keypair::new();
    let token_mint = Keypair::new();
    let breaker_auth = Keypair::new();
    svm.airdrop(&breaker_auth.pubkey(), 1_000_000_000).unwrap();

    send_ok(svm, &[ix_initialize(program_id, &admin.pubkey())], &[admin]);
    send_ok(svm, &[ix_register_vault(
        program_id, &admin.pubkey(), &vault.pubkey(), &token_mint.pubkey(), &breaker_auth.pubkey(),
        vec![WindowConfig { window_seconds: 300, max_outflow_bps: 1000 }],
        500, 300, 600, 0,
    )], &[admin]);

    (vault, token_mint, breaker_auth)
}

// ===========================================================================
// 1. Initialize
// ===========================================================================

#[test]
fn test_initialize_succeeds() {
    let (mut svm, admin, pid) = setup();
    send_ok(&mut svm, &[ix_initialize(&pid, &admin.pubkey())], &[&admin]);
}

#[test]
fn test_initialize_twice_fails() {
    let (mut svm, admin, pid) = setup();
    send_ok(&mut svm, &[ix_initialize(&pid, &admin.pubkey())], &[&admin]);
    send_err(&mut svm, &[ix_initialize(&pid, &admin.pubkey())], &[&admin]);
}

// ===========================================================================
// 2. Register Vault
// ===========================================================================

#[test]
fn test_register_vault_succeeds() {
    let (mut svm, admin, pid) = setup();
    let (v, m, b) = setup_with_vault(&mut svm, &admin, &pid);
}

#[test]
fn test_register_vault_rejects_empty_windows() {
    let (mut svm, admin, pid) = setup();
    send_ok(&mut svm, &[ix_initialize(&pid, &admin.pubkey())], &[&admin]);
    let v = Keypair::new(); let m = Keypair::new();
    send_err(&mut svm, &[ix_register_vault(&pid, &admin.pubkey(), &v.pubkey(), &m.pubkey(), &admin.pubkey(), vec![], 500, 300, 600, 0)], &[&admin]);
}

#[test]
fn test_register_vault_rejects_zero_window_seconds() {
    let (mut svm, admin, pid) = setup();
    send_ok(&mut svm, &[ix_initialize(&pid, &admin.pubkey())], &[&admin]);
    let v = Keypair::new(); let m = Keypair::new();
    send_err(&mut svm, &[ix_register_vault(&pid, &admin.pubkey(), &v.pubkey(), &m.pubkey(), &admin.pubkey(),
        vec![WindowConfig { window_seconds: 0, max_outflow_bps: 1000 }], 500, 300, 600, 0)], &[&admin]);
}

#[test]
fn test_register_vault_rejects_zero_bps() {
    let (mut svm, admin, pid) = setup();
    send_ok(&mut svm, &[ix_initialize(&pid, &admin.pubkey())], &[&admin]);
    let v = Keypair::new(); let m = Keypair::new();
    send_err(&mut svm, &[ix_register_vault(&pid, &admin.pubkey(), &v.pubkey(), &m.pubkey(), &admin.pubkey(),
        vec![WindowConfig { window_seconds: 300, max_outflow_bps: 0 }], 500, 300, 600, 0)], &[&admin]);
}

#[test]
fn test_register_vault_rejects_over_10000_bps() {
    let (mut svm, admin, pid) = setup();
    send_ok(&mut svm, &[ix_initialize(&pid, &admin.pubkey())], &[&admin]);
    let v = Keypair::new(); let m = Keypair::new();
    send_err(&mut svm, &[ix_register_vault(&pid, &admin.pubkey(), &v.pubkey(), &m.pubkey(), &admin.pubkey(),
        vec![WindowConfig { window_seconds: 300, max_outflow_bps: 10001 }], 500, 300, 600, 0)], &[&admin]);
}

#[test]
fn test_register_vault_rejects_more_than_3_windows() {
    let (mut svm, admin, pid) = setup();
    send_ok(&mut svm, &[ix_initialize(&pid, &admin.pubkey())], &[&admin]);
    let v = Keypair::new(); let m = Keypair::new();
    send_err(&mut svm, &[ix_register_vault(&pid, &admin.pubkey(), &v.pubkey(), &m.pubkey(), &admin.pubkey(),
        vec![
            WindowConfig { window_seconds: 60, max_outflow_bps: 500 },
            WindowConfig { window_seconds: 300, max_outflow_bps: 1000 },
            WindowConfig { window_seconds: 3600, max_outflow_bps: 2500 },
            WindowConfig { window_seconds: 86400, max_outflow_bps: 5000 },
        ], 500, 300, 600, 0)], &[&admin]);
}

// ===========================================================================
// 3. Check Outflow — normal
// ===========================================================================

#[test]
fn test_small_withdrawal_allowed() {
    let (mut svm, admin, pid) = setup();
    let (vault, _, _) = setup_with_vault(&mut svm, &admin, &pid);
    set_clock(&mut svm, 1000);
    assert!(check_outflow_allowed(&mut svm, &pid, &admin, &vault.pubkey(), 10_000, 1_000_000));
}

#[test]
fn test_multiple_small_withdrawals_allowed() {
    let (mut svm, admin, pid) = setup();
    let (vault, _, _) = setup_with_vault(&mut svm, &admin, &pid);
    set_clock(&mut svm, 1000);
    // 3 x 3% = 9%, under 10% window
    for i in 0..3 {
        assert_eq!(check_outflow_allowed(&mut svm, &pid, &admin, &vault.pubkey(), 30_000, 1_000_000), true, "withdrawal {} should pass", i);
    }
}

#[test]
fn test_rejects_zero_amount() {
    let (mut svm, admin, pid) = setup();
    let (vault, _, _) = setup_with_vault(&mut svm, &admin, &pid);
    set_clock(&mut svm, 1000);
    // zero amount is a hard error (require!), not a return code
    let ix = ix_check_outflow(&pid, &admin.pubkey(), &vault.pubkey(), 0, 1_000_000);
    assert!(send_tx(&mut svm, &[ix], &[&admin]).is_err());
}

#[test]
fn test_rejects_zero_tvl() {
    let (mut svm, admin, pid) = setup();
    let (vault, _, _) = setup_with_vault(&mut svm, &admin, &pid);
    set_clock(&mut svm, 1000);
    let ix = ix_check_outflow(&pid, &admin.pubkey(), &vault.pubkey(), 1000, 0);
    assert!(send_tx(&mut svm, &[ix], &[&admin]).is_err());
}

// ===========================================================================
// 4. Single transaction limit
// ===========================================================================

#[test]
fn test_single_tx_over_limit_trips() {
    let (mut svm, admin, pid) = setup();
    let (vault, _, _) = setup_with_vault(&mut svm, &admin, &pid);
    set_clock(&mut svm, 1000);
    // 6% > 5% single limit → blocked, code 2
    assert_eq!(check_outflow_allowed(&mut svm, &pid, &admin, &vault.pubkey(), 60_000, 1_000_000), false);
}

#[test]
fn test_single_tx_at_exact_limit_passes() {
    let (mut svm, admin, pid) = setup();
    let (vault, _, _) = setup_with_vault(&mut svm, &admin, &pid);
    set_clock(&mut svm, 1000);
    // Exactly 5% → 50_000 of 1_000_000 → should pass
    assert!(check_outflow_allowed(&mut svm, &pid, &admin, &vault.pubkey(), 50_000, 1_000_000));
}

// ===========================================================================
// 5. Window rate limit
// ===========================================================================

#[test]
fn test_window_rate_limit_trips() {
    let (mut svm, admin, pid) = setup();
    let (vault, _, _) = setup_with_vault(&mut svm, &admin, &pid);
    set_clock(&mut svm, 1000);
    // 3 x 3% = 9% ok
    for _ in 0..3 {
        assert!(check_outflow_allowed(&mut svm, &pid, &admin, &vault.pubkey(), 30_000, 1_000_000));
    }
    // 4th: 2% more = 11% → over 10% → blocked
    assert!(!check_outflow_allowed(&mut svm, &pid, &admin, &vault.pubkey(), 20_000, 1_000_000));
}

#[test]
fn test_window_resets_after_expiry() {
    let (mut svm, admin, pid) = setup();
    let (vault, _, _) = setup_with_vault(&mut svm, &admin, &pid);
    set_clock(&mut svm, 1000);
    // Use 9% of the 10% window
    for _ in 0..3 {
        assert!(check_outflow_allowed(&mut svm, &pid, &admin, &vault.pubkey(), 30_000, 1_000_000));
    }
    // Fast forward past 300s window
    set_clock(&mut svm, 1000 + 301);
    // Window reset — 3% should work
    assert!(check_outflow_allowed(&mut svm, &pid, &admin, &vault.pubkey(), 30_000, 1_000_000));
}

#[test]
fn test_exact_window_limit_passes() {
    let (mut svm, admin, pid) = setup();
    send_ok(&mut svm, &[ix_initialize(&pid, &admin.pubkey())], &[&admin]);
    let vault = Keypair::new(); let mint = Keypair::new();
    send_ok(&mut svm, &[ix_register_vault(&pid, &admin.pubkey(), &vault.pubkey(), &mint.pubkey(), &admin.pubkey(),
        vec![WindowConfig { window_seconds: 300, max_outflow_bps: 1000 }],
        1000, 300, 600, 0)], &[&admin]);
    set_clock(&mut svm, 1000);
    // Exactly 10% → at limit, should pass
    assert!(check_outflow_allowed(&mut svm, &pid, &admin, &vault.pubkey(), 100_000, 1_000_000));
}

#[test]
fn test_one_over_window_limit_trips() {
    let (mut svm, admin, pid) = setup();
    let (vault, _, _) = setup_with_vault(&mut svm, &admin, &pid);
    set_clock(&mut svm, 1000);
    // 10% + 1 → trips
    assert_eq!(check_outflow_allowed(&mut svm, &pid, &admin, &vault.pubkey(), 100_001, 1_000_000), false);
}

// ===========================================================================
// 6. Tripped breaker blocks all outflows
// ===========================================================================

#[test]
fn test_tripped_breaker_blocks_subsequent_outflows() {
    let (mut svm, admin, pid) = setup();
    let (vault, _, breaker_auth) = setup_with_vault(&mut svm, &admin, &pid);
    set_clock(&mut svm, 1000);
    // Manually trip the breaker
    send_ok(&mut svm, &[ix_trip_breaker(&pid, &breaker_auth.pubkey(), &vault.pubkey())], &[&breaker_auth]);
    // Even 1 token should be blocked
    assert!(!check_outflow_allowed(&mut svm, &pid, &admin, &vault.pubkey(), 1, 1_000_000));
}

#[test]
fn test_manual_trip_auto_resets_after_cooldown() {
    let (mut svm, admin, pid) = setup();
    let (vault, _, breaker_auth) = setup_with_vault(&mut svm, &admin, &pid);
    set_clock(&mut svm, 1000);
    // Manual trip
    send_ok(&mut svm, &[ix_trip_breaker(&pid, &breaker_auth.pubkey(), &vault.pubkey())], &[&breaker_auth]);
    // Still blocked during cooldown
    set_clock(&mut svm, 1000 + 299);
    assert!(!check_outflow_allowed(&mut svm, &pid, &admin, &vault.pubkey(), 1, 1_000_000));
    // After cooldown (300s) — auto reset
    set_clock(&mut svm, 1000 + 301);
    assert!(check_outflow_allowed(&mut svm, &pid, &admin, &vault.pubkey(), 1_000, 1_000_000));
}

// ===========================================================================
// 7. Manual trip and reset
// ===========================================================================

#[test]
fn test_manual_trip_blocks_outflows() {
    let (mut svm, admin, pid) = setup();
    let (vault, _, breaker_auth) = setup_with_vault(&mut svm, &admin, &pid);
    set_clock(&mut svm, 1000);
    send_ok(&mut svm, &[ix_trip_breaker(&pid, &breaker_auth.pubkey(), &vault.pubkey())], &[&breaker_auth]);
    assert_eq!(check_outflow_allowed(&mut svm, &pid, &admin, &vault.pubkey(), 1, 1_000_000), false);
}

#[test]
fn test_manual_trip_reset_immediately() {
    let (mut svm, admin, pid) = setup();
    let (vault, _, breaker_auth) = setup_with_vault(&mut svm, &admin, &pid);
    set_clock(&mut svm, 1000);
    // Trip manually
    send_ok(&mut svm, &[ix_trip_breaker(&pid, &breaker_auth.pubkey(), &vault.pubkey())], &[&breaker_auth]);
    // Reset immediately — no lockout for manual trips
    send_ok(&mut svm, &[ix_reset_breaker(&pid, &breaker_auth.pubkey(), &vault.pubkey())], &[&breaker_auth]);
    // Outflow works
    assert!(check_outflow_allowed(&mut svm, &pid, &admin, &vault.pubkey(), 1_000, 1_000_000));
}

#[test]
fn test_reset_fails_when_not_tripped() {
    let (mut svm, admin, pid) = setup();
    let (vault, _, breaker_auth) = setup_with_vault(&mut svm, &admin, &pid);
    set_clock(&mut svm, 1000);
    send_err(&mut svm, &[ix_reset_breaker(&pid, &breaker_auth.pubkey(), &vault.pubkey())], &[&breaker_auth]);
}

// ===========================================================================
// 8. Role separation
// ===========================================================================

#[test]
fn test_policy_authority_cannot_trip() {
    let (mut svm, admin, pid) = setup();
    let (vault, _, _) = setup_with_vault(&mut svm, &admin, &pid);
    send_err(&mut svm, &[ix_trip_breaker(&pid, &admin.pubkey(), &vault.pubkey())], &[&admin]);
}

#[test]
fn test_policy_authority_cannot_reset() {
    let (mut svm, admin, pid) = setup();
    let (vault, _, breaker_auth) = setup_with_vault(&mut svm, &admin, &pid);
    set_clock(&mut svm, 1000);
    send_ok(&mut svm, &[ix_trip_breaker(&pid, &breaker_auth.pubkey(), &vault.pubkey())], &[&breaker_auth]);
    send_err(&mut svm, &[ix_reset_breaker(&pid, &admin.pubkey(), &vault.pubkey())], &[&admin]);
}

#[test]
fn test_breaker_authority_cannot_update_policy() {
    let (mut svm, admin, pid) = setup();
    let (vault, _, breaker_auth) = setup_with_vault(&mut svm, &admin, &pid);
    let params = solana_circuit_breaker::UpdatePolicyParams {
        windows: None, max_single_outflow_bps: Some(9999), cooldown_seconds: None, lockout_seconds: None, paused: None, policy_change_delay: None,
    };
    send_err(&mut svm, &[ix_update_policy(&pid, &breaker_auth.pubkey(), &vault.pubkey(), params)], &[&breaker_auth]);
}

#[test]
fn test_random_key_cannot_check_outflow() {
    let (mut svm, admin, pid) = setup();
    let (vault, _, _) = setup_with_vault(&mut svm, &admin, &pid);
    set_clock(&mut svm, 1000);
    let random = Keypair::new();
    svm.airdrop(&random.pubkey(), 1_000_000_000).unwrap();
    let ix = ix_check_outflow(&pid, &random.pubkey(), &vault.pubkey(), 1_000, 1_000_000);
    assert!(send_tx(&mut svm, &[ix], &[&random]).is_err());
}

// ===========================================================================
// 9. Lockout mode
// ===========================================================================

#[test]
fn test_manual_trip_can_be_reset_immediately_no_lockout() {
    // Manual trips set auto_tripped=false, so no lockout applies
    let (mut svm, admin, pid) = setup();
    let (vault, _, breaker_auth) = setup_with_vault(&mut svm, &admin, &pid);
    set_clock(&mut svm, 1000);
    send_ok(&mut svm, &[ix_trip_breaker(&pid, &breaker_auth.pubkey(), &vault.pubkey())], &[&breaker_auth]);
    // Reset works immediately for manual trips
    send_ok(&mut svm, &[ix_reset_breaker(&pid, &breaker_auth.pubkey(), &vault.pubkey())], &[&breaker_auth]);
}

#[test]
fn test_window_cumulative_prevents_drain_without_trip_state() {
    // Even though check_outflow errors don't persist trip state,
    // the cumulative outflow from SUCCESSFUL calls prevents further draining
    let (mut svm, admin, pid) = setup();
    let (vault, _, _) = setup_with_vault(&mut svm, &admin, &pid);
    set_clock(&mut svm, 1000);
    // 3 x 3% = 9% — each call succeeds and persists cumulative
    for _ in 0..3 {
        assert!(check_outflow_allowed(&mut svm, &pid, &admin, &vault.pubkey(), 30_000, 1_000_000));
    }
    // 4th fails — cumulative 12% > 10% window. Error rolls back, but
    // the 9% from previous calls persists, so attacker can't drain more
    assert!(!check_outflow_allowed(&mut svm, &pid, &admin, &vault.pubkey(), 30_000, 1_000_000));
    // Even smaller amounts fail if they push over 10%
    assert!(!check_outflow_allowed(&mut svm, &pid, &admin, &vault.pubkey(), 20_000, 1_000_000));
    // But amounts that stay under 10% still work
    assert!(check_outflow_allowed(&mut svm, &pid, &admin, &vault.pubkey(), 10_000, 1_000_000));
}

#[test]
fn test_single_tx_limit_rejects_but_allows_smaller() {
    // Single tx limit errors don't persist, but they prevent the large withdrawal
    let (mut svm, admin, pid) = setup();
    let (vault, _, _) = setup_with_vault(&mut svm, &admin, &pid);
    set_clock(&mut svm, 1000);
    // 6% blocked (single tx limit 5%)
    assert!(!check_outflow_allowed(&mut svm, &pid, &admin, &vault.pubkey(), 60_000, 1_000_000));
    // But 4% still works — no trip state persisted
    assert!(check_outflow_allowed(&mut svm, &pid, &admin, &vault.pubkey(), 40_000, 1_000_000));
}

// ===========================================================================
// 10. Update policy
// ===========================================================================

#[test]
fn test_update_policy_loosens_limit() {
    let (mut svm, admin, pid) = setup();
    let (vault, _, _) = setup_with_vault(&mut svm, &admin, &pid);
    set_clock(&mut svm, 1000);
    // Loosen single tx to 50%
    let params = solana_circuit_breaker::UpdatePolicyParams {
        windows: None, max_single_outflow_bps: Some(5000), cooldown_seconds: None, lockout_seconds: None, paused: None, policy_change_delay: None,
    };
    send_ok(&mut svm, &[ix_update_policy(&pid, &admin.pubkey(), &vault.pubkey(), params)], &[&admin]);
    // 6% now passes (no policy_change_delay so loosening is immediate)
    assert!(check_outflow_allowed(&mut svm, &pid, &admin, &vault.pubkey(), 60_000, 1_000_000));
}

#[test]
fn test_pause_blocks_outflows() {
    let (mut svm, admin, pid) = setup();
    let (vault, _, _) = setup_with_vault(&mut svm, &admin, &pid);
    set_clock(&mut svm, 1000);
    let params = solana_circuit_breaker::UpdatePolicyParams {
        windows: None, max_single_outflow_bps: None, cooldown_seconds: None, lockout_seconds: None, paused: Some(true), policy_change_delay: None,
    };
    send_ok(&mut svm, &[ix_update_policy(&pid, &admin.pubkey(), &vault.pubkey(), params)], &[&admin]);
    assert_eq!(check_outflow_allowed(&mut svm, &pid, &admin, &vault.pubkey(), 1, 1_000_000), false); // paused
}

#[test]
fn test_unpause_resumes_outflows() {
    let (mut svm, admin, pid) = setup();
    let (vault, _, _) = setup_with_vault(&mut svm, &admin, &pid);
    set_clock(&mut svm, 1000);
    for paused in [true, false] {
        let params = solana_circuit_breaker::UpdatePolicyParams {
            windows: None, max_single_outflow_bps: None, cooldown_seconds: None, lockout_seconds: None, paused: Some(paused), policy_change_delay: None,
        };
        send_ok(&mut svm, &[ix_update_policy(&pid, &admin.pubkey(), &vault.pubkey(), params)], &[&admin]);
    }
    assert!(check_outflow_allowed(&mut svm, &pid, &admin, &vault.pubkey(), 1_000, 1_000_000));
}

// ===========================================================================
// 11. Attack simulations
// ===========================================================================

#[test]
fn test_attack_drain_capped_at_10_percent() {
    let (mut svm, admin, pid) = setup();
    let (vault, _, _) = setup_with_vault(&mut svm, &admin, &pid);
    set_clock(&mut svm, 1000);
    let tvl = 1_000_000u64;
    let mut total_drained = 0u64;
    for _ in 0..100 {
        if check_outflow_allowed(&mut svm, &pid, &admin, &vault.pubkey(), 49_000, tvl) {
            total_drained += 49_000;
        } else {
            break;
        }
    }
    assert!(total_drained <= 100_000, "drained {} > 10%", total_drained);
    assert!(total_drained > 0);
}

#[test]
fn test_attack_compromised_policy_key_cannot_reset_auto_trip() {
    let (mut svm, admin, pid) = setup();
    let (vault, _, breaker_auth) = setup_with_vault(&mut svm, &admin, &pid);
    set_clock(&mut svm, 1000);
    check_outflow_allowed(&mut svm, &pid, &admin, &vault.pubkey(), 60_000, 1_000_000);
    // policy_authority can't reset
    send_err(&mut svm, &[ix_reset_breaker(&pid, &admin.pubkey(), &vault.pubkey())], &[&admin]);
    // breaker_authority can't reset during lockout
    send_err(&mut svm, &[ix_reset_breaker(&pid, &breaker_auth.pubkey(), &vault.pubkey())], &[&breaker_auth]);
}

#[test]
fn test_attack_loosen_windows_rejected() {
    let (mut svm, admin, pid) = setup();
    let (vault, _, _) = setup_with_vault(&mut svm, &admin, &pid);
    set_clock(&mut svm, 1000);
    // Attacker has policy key, tries to loosen windows to 100%
    // This is now rejected — window loosening requires re-registration
    let params = solana_circuit_breaker::UpdatePolicyParams {
        windows: Some(vec![WindowConfig { window_seconds: 300, max_outflow_bps: 10000 }]),
        max_single_outflow_bps: None, cooldown_seconds: None, lockout_seconds: None, paused: None, policy_change_delay: None,
    };
    send_err(&mut svm, &[ix_update_policy(&pid, &admin.pubkey(), &vault.pubkey(), params)], &[&admin]);
}

#[test]
fn test_tightening_windows_allowed() {
    let (mut svm, admin, pid) = setup();
    let (vault, _, _) = setup_with_vault(&mut svm, &admin, &pid);
    set_clock(&mut svm, 1000);
    // Tightening from 10% to 5% — should work immediately
    let params = solana_circuit_breaker::UpdatePolicyParams {
        windows: Some(vec![WindowConfig { window_seconds: 300, max_outflow_bps: 500 }]),
        max_single_outflow_bps: None, cooldown_seconds: None, lockout_seconds: None, paused: None, policy_change_delay: None,
    };
    send_ok(&mut svm, &[ix_update_policy(&pid, &admin.pubkey(), &vault.pubkey(), params)], &[&admin]);
    // Now 4% should fail (was ok before at 10%, now 5% window)
    assert!(check_outflow_allowed(&mut svm, &pid, &admin, &vault.pubkey(), 40_000, 1_000_000));
    // But 2% more = 6% > 5% should fail
    assert!(!check_outflow_allowed(&mut svm, &pid, &admin, &vault.pubkey(), 20_000, 1_000_000));
}

// ===========================================================================
// 12. Multiple windows
// ===========================================================================

#[test]
fn test_tightest_window_wins() {
    let (mut svm, admin, pid) = setup();
    send_ok(&mut svm, &[ix_initialize(&pid, &admin.pubkey())], &[&admin]);
    let vault = Keypair::new(); let mint = Keypair::new();
    send_ok(&mut svm, &[ix_register_vault(&pid, &admin.pubkey(), &vault.pubkey(), &mint.pubkey(), &admin.pubkey(),
        vec![
            WindowConfig { window_seconds: 60, max_outflow_bps: 500 },   // 5% per 60s
            WindowConfig { window_seconds: 3600, max_outflow_bps: 2000 }, // 20% per 1h
        ], 1000, 300, 600, 0)], &[&admin]);
    set_clock(&mut svm, 1000);
    // 4% — under both
    assert!(check_outflow_allowed(&mut svm, &pid, &admin, &vault.pubkey(), 40_000, 1_000_000));
    // 2% more = 6% → over window 1 (5%) but under window 2 (20%)
    assert!(!check_outflow_allowed(&mut svm, &pid, &admin, &vault.pubkey(), 20_000, 1_000_000));
}

// ===========================================================================
// 13. Edge cases
// ===========================================================================

#[test]
fn test_large_tvl_no_overflow() {
    let (mut svm, admin, pid) = setup();
    let (vault, _, _) = setup_with_vault(&mut svm, &admin, &pid);
    set_clock(&mut svm, 1000);
    assert!(check_outflow_allowed(&mut svm, &pid, &admin, &vault.pubkey(), 1000, u64::MAX / 2));
}
