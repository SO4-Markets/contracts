//! Tests for Issue #916 [CLOB-040]:
//! Avoid renewing abandoned state and test archive/restore scenarios.
//!
//! Validates:
//! 1. Zero-balance trader entries are NOT renewed by unrelated activity or sweeps.
//! 2. Active non-zero balance entries are renewed on operations.
//! 3. Archive/restore lifecycle preserves exact balance reconciliation without fund loss.

#![cfg(test)]

use gmx_types::storage_classification::{
    filter_renewable_accounts, is_abandoned_zero_balance, should_renew_order,
    should_renew_trader_balance,
};
use gmx_types::token_support::SafeCustodyLedger;
use soroban_sdk::{testutils::Address as _, Address, Env, Map};

#[test]
fn test_unrelated_activity_does_not_renew_zero_balance_traders() {
    let env = Env::default();
    let active_trader = Address::generate(&env);
    let dead_trader = Address::generate(&env);

    // Mock storage map representing ledger state
    let mut ledger_balances: Map<Address, (i128, i128)> = Map::new(&env);
    ledger_balances.set(active_trader.clone(), (5000, 0)); // Active funds
    ledger_balances.set(dead_trader.clone(), (0, 0)); // Abandoned account

    // Unrelated activity: bulk sweep or market fill
    let batch = [
        (active_trader.clone(), 5000, 0),
        (dead_trader.clone(), 0, 0),
    ];

    let renewable = filter_renewable_accounts(&env, &batch);

    // Assert that active trader is scheduled for renewal, but dead trader is completely skipped
    assert_eq!(renewable.len(), 1);
    assert_eq!(renewable.get(0).unwrap(), active_trader);
    assert!(!renewable.contains(dead_trader.clone()));
}

#[test]
fn test_balance_depletion_transitions_to_non_renewable() {
    // 1. Trader starts with funds: renewable
    let mut available = 1000i128;
    let mut locked = 0i128;
    assert!(should_renew_trader_balance(available, locked));
    assert!(!is_abandoned_zero_balance(available, locked));

    // 2. Trader places order: locked funds, still renewable
    available = 0;
    locked = 1000;
    assert!(should_renew_trader_balance(available, locked));
    assert!(!is_abandoned_zero_balance(available, locked));

    // 3. Order fills and trader withdraws full proceeds: balance drops to (0, 0)
    available = 0;
    locked = 0;

    // 4. Now abandoned: must NOT be renewed
    assert!(!should_renew_trader_balance(available, locked));
    assert!(is_abandoned_zero_balance(available, locked));
}

#[test]
fn test_archive_and_restore_preserves_custody_reconciliation() {
    let env = Env::default();
    let token = Address::generate(&env);
    let mut custody = SafeCustodyLedger::new(token);

    // Trader deposits 5000 units
    custody.checked_deposit(5000, 0, 5000).unwrap();
    custody.checked_lock(2000).unwrap();
    let vault_balance = 5000i128;

    assert_eq!(custody.available, 3000);
    assert_eq!(custody.locked, 2000);
    assert!(custody.reconcile_with_vault(vault_balance).is_ok());

    // ── Simulate Archival ──
    // The trader becomes inactive and the persistent entry expires and is archived.
    // While archived, active queries cannot find the entry directly without restoration.
    let archived_available = custody.available;
    let archived_locked = custody.locked;
    let archived_fees = custody.protocol_fees;

    // ── Simulate Restoration ──
    // Trader submits a restore footprint transaction to revive the entry.
    // Restored state is unmarshalled:
    let restored_custody = SafeCustodyLedger {
        token: custody.token.clone(),
        available: archived_available,
        locked: archived_locked,
        protocol_fees: archived_fees,
    };

    // Full custody reconciliation succeeds with zero loss of funds
    assert_eq!(restored_custody.available, 3000);
    assert_eq!(restored_custody.locked, 2000);
    assert_eq!(restored_custody.total_accounted().unwrap(), 5000);
    assert!(restored_custody.reconcile_with_vault(vault_balance).is_ok());
}

#[test]
fn test_order_lifecycle_renewal_invariants() {
    let expiry = 10_000u64;

    // 1. Order open at ledger 100: renewable
    assert!(should_renew_order(100, false, 100, expiry));

    // 2. Order partially filled (50 remaining) at ledger 500: renewable
    assert!(should_renew_order(50, false, 500, expiry));

    // 3. Order fully filled (0 remaining) at ledger 600: NOT renewable
    assert!(!should_renew_order(0, false, 600, expiry));

    // 4. Order cancelled at ledger 200: NOT renewable
    assert!(!should_renew_order(50, true, 200, expiry));

    // 5. Order reaches expiry at ledger 10_001: NOT renewable
    assert!(!should_renew_order(50, false, 10_001, expiry));
}
