// Issues #915 [CLOB-039] & #916 [CLOB-040]:
// Classify storage entries, define TTL extension thresholds, and avoid renewing abandoned zero-balance state.
//! Storage classification, TTL thresholds, and zero-balance renewal avoidance.
//!
//! Persistent Soroban state requires rent to stay alive. Permanent-critical protocol
//! state and live user balances must not disappear, while dead or abandoned state
//! (such as zero-balance accounts) must NOT be renewed at protocol expense.

use soroban_sdk::{contracttype, Address, Env, Vec};

// Target TTL and threshold constants (measured in 5-second ledger sequences).

/// Tier 1: Permanent-Critical threshold (~30 days).
pub const PERMANENT_CRITICAL_THRESHOLD: u32 = 518_400;
/// Tier 1: Permanent-Critical extension target (~60 days).
pub const PERMANENT_CRITICAL_TARGET: u32 = 1_036_800;

/// Tier 2: Live-While-Active threshold (~15 days).
pub const LIVE_ACTIVE_THRESHOLD: u32 = 259_200;
/// Tier 2: Live-While-Active extension target (~30 days).
pub const LIVE_ACTIVE_TARGET: u32 = 518_400;

/// Tier 3: Temporary price TTL (~10 minutes).
pub const TEMPORARY_PRICE_TTL: u32 = 120;

/// Storage entry classification tier.
#[contracttype]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum StorageTier {
    /// Permanent-Critical: Market config, registry, admin/roles, fee pools.
    PermanentCritical,
    /// Live-While-Active: Non-zero user balances, open resting orders, active order queues.
    LiveWhileActive,
    /// Disposable / Indexable: Zero-balance trader entries, settled/cancelled order records.
    DisposableIndexable,
}

/// TTL extension parameters for a storage tier.
#[contracttype]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TtlConfig {
    /// Renew if remaining TTL drops below this ledger threshold.
    pub threshold_ledgers: u32,
    /// Extend TTL to this target ledger count.
    pub extend_to_ledgers: u32,
}

impl StorageTier {
    /// Get the standard TTL configuration for this tier.
    /// Returns None for DisposableIndexable (which should not be renewed).
    pub fn ttl_config(&self) -> Option<TtlConfig> {
        match self {
            StorageTier::PermanentCritical => Some(TtlConfig {
                threshold_ledgers: PERMANENT_CRITICAL_THRESHOLD,
                extend_to_ledgers: PERMANENT_CRITICAL_TARGET,
            }),
            StorageTier::LiveWhileActive => Some(TtlConfig {
                threshold_ledgers: LIVE_ACTIVE_THRESHOLD,
                extend_to_ledgers: LIVE_ACTIVE_TARGET,
            }),
            StorageTier::DisposableIndexable => None,
        }
    }
}

/// Evaluates whether a trader balance entry qualifies for TTL renewal.
///
/// Issue #916: Zero-balance trader entries (`available == 0 && locked == 0`)
/// MUST NOT be renewed by unrelated activity or bulk sweeps. Only accounts
/// with real economic interest (`available > 0 || locked > 0`) are renewed.
pub fn should_renew_trader_balance(available: i128, locked: i128) -> bool {
    // Accounts with active funds or open order liabilities qualify for renewal.
    available > 0 || locked > 0
}

/// Returns true if an account is an abandoned zero-balance entry.
pub fn is_abandoned_zero_balance(available: i128, locked: i128) -> bool {
    available == 0 && locked == 0
}

/// Evaluates whether a resting order qualifies for TTL renewal.
///
/// An order should be renewed only if it is active, unfilled, and not expired.
pub fn should_renew_order(
    remaining_base_lots: u64,
    is_cancelled: bool,
    current_ledger: u64,
    expiry_ledger: u64,
) -> bool {
    !is_cancelled && remaining_base_lots > 0 && current_ledger <= expiry_ledger
}

/// Filter a batch of accounts, returning only those with non-zero balances eligible for renewal.
///
/// Used by bulk maintenance operations, fee sweeps, and keeper scripts to prevent
/// spending protocol gas/rent renewing dead zero-balance accounts.
pub fn filter_renewable_accounts(
    env: &Env,
    accounts: &[(Address, i128, i128)], // (address, available, locked)
) -> Vec<Address> {
    let mut renewable = Vec::new(env);
    for (addr, available, locked) in accounts.iter() {
        if should_renew_trader_balance(*available, *locked) {
            renewable.push_back(addr.clone());
        }
    }
    renewable
}

#[cfg(test)]
mod tests {
    use super::*;
    use soroban_sdk::testutils::Address as _;

    #[test]
    fn test_storage_tier_ttl_configs() {
        let perm = StorageTier::PermanentCritical.ttl_config().unwrap();
        assert_eq!(perm.threshold_ledgers, PERMANENT_CRITICAL_THRESHOLD);
        assert_eq!(perm.extend_to_ledgers, PERMANENT_CRITICAL_TARGET);

        let live = StorageTier::LiveWhileActive.ttl_config().unwrap();
        assert_eq!(live.threshold_ledgers, LIVE_ACTIVE_THRESHOLD);
        assert_eq!(live.extend_to_ledgers, LIVE_ACTIVE_TARGET);

        let disp = StorageTier::DisposableIndexable.ttl_config();
        assert!(disp.is_none());
    }

    #[test]
    fn test_zero_balance_is_not_renewed() {
        // Zero balance: neither available nor locked
        assert!(!should_renew_trader_balance(0, 0));
        assert!(is_abandoned_zero_balance(0, 0));
    }

    #[test]
    fn test_nonzero_available_is_renewed() {
        assert!(should_renew_trader_balance(100, 0));
        assert!(!is_abandoned_zero_balance(100, 0));
    }

    #[test]
    fn test_nonzero_locked_is_renewed() {
        assert!(should_renew_trader_balance(0, 50));
        assert!(!is_abandoned_zero_balance(0, 50));
    }

    #[test]
    fn test_order_renewal_conditions() {
        // Active unfilled order before expiry -> renew
        assert!(should_renew_order(10, false, 100, 200));

        // Fully filled order (0 remaining lots) -> do NOT renew
        assert!(!should_renew_order(0, false, 100, 200));

        // Cancelled order -> do NOT renew
        assert!(!should_renew_order(10, true, 100, 200));

        // Expired order (current ledger > expiry) -> do NOT renew
        assert!(!should_renew_order(10, false, 250, 200));
    }

    #[test]
    fn test_filter_renewable_accounts_prunes_zero_balances() {
        let env = Env::default();
        let a1 = Address::generate(&env);
        let a2 = Address::generate(&env);
        let a3 = Address::generate(&env);

        let accounts = [
            (a1.clone(), 100i128, 0i128), // active
            (a2.clone(), 0i128, 0i128),   // zero-balance / dead
            (a3.clone(), 0i128, 50i128),  // locked
        ];

        let renewable = filter_renewable_accounts(&env, &accounts);
        assert_eq!(renewable.len(), 2);
        assert!(renewable.contains(a1));
        assert!(!renewable.contains(a2)); // a2 pruned
        assert!(renewable.contains(a3));
    }
}
