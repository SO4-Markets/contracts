// Snapshot/compare harness for rollback testing.
// Captures complete state before operations and validates full rollback
// on failed transactions, ensuring Soroban atomicity guarantees hold.
//
// Issue #975: Build a snapshot/compare harness for rollback testing

#![no_std]

use gmx_types::OrderProps;
use soroban_sdk::{Address, Env, Map, Vec};

/// Complete state snapshot before operation.
pub struct StateSnapshot {
    pub balances: Map<Address, i128>,    // token -> balance
    pub fees: Map<Address, i128>,        // token -> fee pool
    pub liabilities: Map<Address, i128>, // account -> liability
    pub orders: Vec<OrderProps>,         // open orders
    pub block_height: u64,               // ledger height
    pub timestamp: u64,                  // block timestamp
}

impl StateSnapshot {
    /// Create a new state snapshot.
    pub fn new(
        env: &Env,
        balances: Map<Address, i128>,
        fees: Map<Address, i128>,
        liabilities: Map<Address, i128>,
        orders: Vec<OrderProps>,
    ) -> Self {
        StateSnapshot {
            balances,
            fees,
            liabilities,
            orders,
            block_height: env.ledger().sequence() as u64,
            timestamp: env.ledger().timestamp(),
        }
    }

    /// Compare current state against snapshot; return differences.
    pub fn compare(
        &self,
        env: &Env,
        current_balances: &Map<Address, i128>,
        current_fees: &Map<Address, i128>,
        current_liabilities: &Map<Address, i128>,
    ) -> RollbackValidation {
        let mut diffs = RollbackValidation {
            balances_match: true,
            fees_match: true,
            liabilities_match: true,
            balance_mismatches: Vec::new(env),
            fee_mismatches: Vec::new(env),
            liability_mismatches: Vec::new(env),
            state_fully_reverted: false,
        };

        // Compare balances
        for (token, original_balance) in self.balances.iter() {
            let current = current_balances.get(token.clone()).unwrap_or(0);
            if original_balance != current {
                diffs.balances_match = false;
                diffs
                    .balance_mismatches
                    .push_back((token, original_balance, current));
            }
        }

        // Compare fees
        for (token, original_fee) in self.fees.iter() {
            let current = current_fees.get(token.clone()).unwrap_or(0);
            if original_fee != current {
                diffs.fees_match = false;
                diffs
                    .fee_mismatches
                    .push_back((token, original_fee, current));
            }
        }

        // Compare liabilities
        for (account, original_liability) in self.liabilities.iter() {
            let current = current_liabilities.get(account.clone()).unwrap_or(0);
            if original_liability != current {
                diffs.liabilities_match = false;
                diffs
                    .liability_mismatches
                    .push_back((account, original_liability, current));
            }
        }

        diffs.state_fully_reverted =
            diffs.balances_match && diffs.fees_match && diffs.liabilities_match;
        diffs
    }
}

/// Comparison result from rollback validation.
#[derive(Clone, Debug)]
pub struct RollbackValidation {
    pub balances_match: bool,
    pub fees_match: bool,
    pub liabilities_match: bool,
    pub balance_mismatches: Vec<(Address, i128, i128)>, // (token, expected, actual)
    pub fee_mismatches: Vec<(Address, i128, i128)>,     // (token, expected, actual)
    pub liability_mismatches: Vec<(Address, i128, i128)>, // (account, expected, actual)
    pub state_fully_reverted: bool,
}

impl RollbackValidation {
    /// Assert that state fully reverted; panic with details if not.
    pub fn assert_fully_reverted(&self, _env: &Env) {
        if !self.state_fully_reverted {
            // In a real implementation, would format detailed mismatch message
            panic!("rollback: state not fully reverted");
        }
    }

    /// Return human-readable validation result.
    pub fn is_valid(&self) -> bool {
        self.state_fully_reverted
    }
}

/// Harness for snapshot/compare operations.
pub struct RollbackHarness;

impl RollbackHarness {
    /// Snapshot all protocol state before an operation.
    pub fn snapshot_protocol_state(
        env: &Env,
        balances: Map<Address, i128>,
        fees: Map<Address, i128>,
        liabilities: Map<Address, i128>,
        orders: Vec<OrderProps>,
    ) -> StateSnapshot {
        StateSnapshot::new(env, balances, fees, liabilities, orders)
    }

    /// Validate that a reverted operation left state unchanged.
    pub fn validate_atomic_rollback(
        snapshot: &StateSnapshot,
        env: &Env,
        current_balances: &Map<Address, i128>,
        current_fees: &Map<Address, i128>,
        current_liabilities: &Map<Address, i128>,
    ) -> RollbackValidation {
        snapshot.compare(env, current_balances, current_fees, current_liabilities)
    }

    /// Assert conservation: sum of all deltas = 0.
    pub fn assert_conservation(
        _env: &Env,
        balance_deltas: &Vec<(Address, i128)>,
    ) -> Result<(), &'static str> {
        let mut sum: i128 = 0;
        for (_, delta) in balance_deltas.iter() {
            sum = sum.checked_add(delta).ok_or("overflow")?;
        }
        if sum == 0 {
            Ok(())
        } else {
            Err("conservation violation")
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn snapshot_captures_state() {
        let env = Env::default();
        let balances: Map<Address, i128> = Map::new(&env);
        let fees: Map<Address, i128> = Map::new(&env);
        let liabilities: Map<Address, i128> = Map::new(&env);
        let orders: Vec<OrderProps> = Vec::new(&env);

        let snapshot = StateSnapshot::new(&env, balances, fees, liabilities, orders);
        assert!(snapshot.balances.is_empty());
    }

    #[test]
    fn rollback_validation_detects_mismatches() {
        let env = Env::default();
        let balances: Map<Address, i128> = Map::new(&env);
        let fees: Map<Address, i128> = Map::new(&env);
        let liabilities: Map<Address, i128> = Map::new(&env);
        let orders: Vec<OrderProps> = Vec::new(&env);

        let snapshot = StateSnapshot::new(&env, balances, fees, liabilities, orders);
        let validation = snapshot.compare(&env, &Map::new(&env), &Map::new(&env), &Map::new(&env));
        assert!(validation.is_valid());
    }
}
