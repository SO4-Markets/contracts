//! Integration tests for Issue #914 [CLOB-038]:
//! Define supported token behavior, reject unsafe token assumptions, and verify custody invariants.
//!
//! Covers:
//! 1. Non-standard failure behavior: failed transfers, fee-on-transfer under-delivery.
//! 2. Internal custody accounting never credits a failed transfer.
//! 3. Custody equation: available + locked + protocol_fees == actual token custody.
//! 4. Unexpected balance drops (simulating clawback/freeze) trigger reconciliation errors.

#![cfg(test)]

use gmx_types::token_support::{SafeCustodyLedger, TokenListingPolicy};
use soroban_sdk::{testutils::Address as _, Address, Env};

#[test]
fn test_token_listing_policy_rejects_unsafe_tokens() {
    let policy = TokenListingPolicy::standard();

    // 1. Valid token passes
    assert!(policy
        .validate_token(true, 7, false, false, false, false)
        .is_ok());

    // 2. Non-standard decimals rejected
    assert_eq!(
        policy
            .validate_token(true, 6, false, false, false, false)
            .unwrap_err(),
        "Token decimals do not match market policy"
    );

    // 3. Rebasing token rejected
    assert_eq!(
        policy
            .validate_token(true, 7, false, false, true, false)
            .unwrap_err(),
        "Rebasing tokens are strictly unsupported"
    );

    // 4. Fee-on-transfer rejected
    assert_eq!(
        policy
            .validate_token(true, 7, false, false, false, true)
            .unwrap_err(),
        "Fee-on-transfer tokens are strictly unsupported"
    );

    // 5. Clawback-capable rejected in standard market
    assert_eq!(
        policy
            .validate_token(true, 7, true, false, false, false)
            .unwrap_err(),
        "Clawback-capable tokens not permitted in standard markets"
    );

    // 6. Freeze-capable rejected in standard market
    assert_eq!(
        policy
            .validate_token(true, 7, false, true, false, false)
            .unwrap_err(),
        "Freeze-capable tokens not permitted in standard markets"
    );

    // 7. Non-compliant interface rejected
    assert_eq!(
        policy
            .validate_token(false, 7, false, false, false, false)
            .unwrap_err(),
        "Token does not implement required SEP-41/SAC interface"
    );
}

#[test]
fn test_custody_never_credits_failed_token_transfer() {
    let env = Env::default();
    let token = Address::generate(&env);
    let mut custody = SafeCustodyLedger::new(token);

    let declared_deposit = 50_000_000i128; // 5 tokens
    let initial_vault_balance = 100_000_000i128;

    // Simulate mock transfer failure where vault balance did not increase
    let result = custody.checked_deposit(
        declared_deposit,
        initial_vault_balance,
        initial_vault_balance, // post balance == pre balance (transfer failed)
    );

    assert!(result.is_err());
    assert_eq!(
        result.unwrap_err(),
        "Transfer failed: no tokens received by vault"
    );

    // Accounting remains strictly 0 - no phantom credit
    assert_eq!(custody.available, 0);
    assert_eq!(custody.locked, 0);
    assert_eq!(custody.protocol_fees, 0);
    assert_eq!(custody.total_accounted().unwrap(), 0);
}

#[test]
fn test_custody_rejects_fee_on_transfer_under_delivery() {
    let env = Env::default();
    let token = Address::generate(&env);
    let mut custody = SafeCustodyLedger::new(token);

    let declared_deposit = 10_000_000i128; // 1 token
    let pre_balance = 0i128;
    // Malicious or fee-on-transfer token only delivered 9.5M (burned 5%)
    let post_balance = 9_500_000i128;

    let result = custody.checked_deposit(declared_deposit, pre_balance, post_balance);
    assert!(result.is_err());
    assert_eq!(
        result.unwrap_err(),
        "Fee-on-transfer detected: actual received < declared amount"
    );

    // Available balance is not credited
    assert_eq!(custody.available, 0);
}

#[test]
fn test_custody_conservation_across_lifecycle() {
    let env = Env::default();
    let token = Address::generate(&env);
    let mut custody = SafeCustodyLedger::new(token);

    // 1. Trader deposits 100 units
    let mut vault_balance = 0i128;
    custody
        .checked_deposit(100, vault_balance, vault_balance + 100)
        .unwrap();
    vault_balance += 100;
    assert!(custody.reconcile_with_vault(vault_balance).is_ok());

    // 2. Trader places order locking 60 units (40 remaining available)
    custody.checked_lock(60).unwrap();
    assert_eq!(custody.available, 40);
    assert_eq!(custody.locked, 60);
    assert!(custody.reconcile_with_vault(vault_balance).is_ok());

    // 3. Cannot lock more than available (attempting to lock 50 when only 40 available)
    assert!(custody.checked_lock(50).is_err());

    // 4. Order partially fills: 40 locked consumed, 2 fee to protocol, 3 refund to trader
    // (net 35 was swapped out to counterparty)
    custody.checked_fill_settlement(40, 2, 3).unwrap();
    vault_balance -= 35; // 35 delivered out to taker
    assert_eq!(custody.locked, 20); // 60 - 40
    assert_eq!(custody.protocol_fees, 2);
    assert_eq!(custody.available, 43); // 40 + 3
    assert_eq!(custody.total_accounted().unwrap(), 65);
    assert_eq!(vault_balance, 65);
    assert!(custody.reconcile_with_vault(vault_balance).is_ok());

    // 5. Remaining order is cancelled, unlocking 20 back to available
    custody.checked_unlock(20).unwrap();
    assert_eq!(custody.locked, 0);
    assert_eq!(custody.available, 63);
    assert!(custody.reconcile_with_vault(vault_balance).is_ok());

    // 6. Trader withdraws 50 units
    custody
        .checked_withdraw(50, vault_balance, vault_balance - 50)
        .unwrap();
    vault_balance -= 50;
    assert_eq!(custody.available, 13);
    assert_eq!(custody.protocol_fees, 2);
    assert_eq!(vault_balance, 15);
    assert!(custody.reconcile_with_vault(vault_balance).is_ok());
}

#[test]
fn test_custody_detects_clawback_anomaly() {
    let env = Env::default();
    let token = Address::generate(&env);
    let mut custody = SafeCustodyLedger::new(token);

    custody.checked_deposit(1_000, 0, 1_000).unwrap();
    let vault_balance = 1_000;
    assert!(custody.reconcile_with_vault(vault_balance).is_ok());

    // Simulate issuer clawback or uncoordinated transfer reducing vault to 800
    let compromised_vault_balance = 800;
    let err = custody
        .reconcile_with_vault(compromised_vault_balance)
        .unwrap_err();
    assert_eq!(
        err,
        "Custody reconciliation failure: total_accounted != actual_vault_balance"
    );
}
