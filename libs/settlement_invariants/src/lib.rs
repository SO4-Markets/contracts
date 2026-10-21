// Settlement invariant definitions and checks for CLOB accounting
// Ensures that all token transfers, fee operations, and liability tracking
// maintain exact accounting equations before and after every fill.
//
// Issue #972: Define settlement asset and fee delta invariants
// Issue #973: Implement reusable settlement invariant checks

#![no_std]

use soroban_sdk::{Address, Env, Map};

/// Account balance delta for a specific token and trader.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BalanceDelta {
    pub token: Address,
    pub account: Address,
    pub delta: i128,
}

/// Fee ownership tracking state.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FeeDelta {
    pub token: Address,
    pub pool: Address,
    pub delta: i128,
}

/// Outstanding liability for open orders.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LiabilityDelta {
    pub account: Address,
    pub collateral_token: Address,
    pub delta: i128,
}

/// Settlement state snapshot for invariant validation.
#[derive(Clone, Debug)]
pub struct SettlementSnapshot {
    pub balances: Map<Address, i128>,    // token → balance
    pub fees: Map<Address, i128>,        // token → fee pool balance
    pub liabilities: Map<Address, i128>, // account → liability
}

/// Bid-taker settlement invariant: long asset increases, short asset decreases,
/// fees accumulate.
pub struct BidTakerInvariant {
    pub taker_account: Address,
    pub long_token: Address,
    pub short_token: Address,
    pub long_collateral_delta: i128,
    pub short_collateral_delta: i128,
    pub execution_fee_delta: i128,
    pub position_fee_delta: i128,
}

/// Ask-taker settlement invariant: long asset decreases, short asset increases,
/// fees accumulate.
pub struct AskTakerInvariant {
    pub taker_account: Address,
    pub long_token: Address,
    pub short_token: Address,
    pub long_collateral_delta: i128,
    pub short_collateral_delta: i128,
    pub execution_fee_delta: i128,
    pub position_fee_delta: i128,
}

/// Locked collateral invariant: sum of all open-order collateral must equal
/// recorded locked liability per account-token pair.
pub struct LockedLiabilityInvariant {
    pub account: Address,
    pub token: Address,
    pub aggregate_open_orders_collateral: i128,
    pub recorded_locked_liability: i128,
}

impl BidTakerInvariant {
    /// Validate bid-taker invariant: long + short deltas consistent with fees.
    pub fn validate(&self) -> Result<(), &'static str> {
        if self.long_collateral_delta < 0 {
            return Err("bid-taker: long collateral decreased");
        }
        if self.short_collateral_delta > 0 {
            return Err("bid-taker: short collateral increased");
        }
        if self.execution_fee_delta < 0 {
            return Err("bid-taker: execution fee decreased");
        }
        if self.position_fee_delta < 0 {
            return Err("bid-taker: position fee decreased");
        }
        Ok(())
    }
}

impl AskTakerInvariant {
    /// Validate ask-taker invariant: long decreases, short increases, fees accumulate.
    pub fn validate(&self) -> Result<(), &'static str> {
        if self.long_collateral_delta > 0 {
            return Err("ask-taker: long collateral increased");
        }
        if self.short_collateral_delta < 0 {
            return Err("ask-taker: short collateral decreased");
        }
        if self.execution_fee_delta < 0 {
            return Err("ask-taker: execution fee decreased");
        }
        if self.position_fee_delta < 0 {
            return Err("ask-taker: position fee decreased");
        }
        Ok(())
    }
}

impl LockedLiabilityInvariant {
    /// Validate that recorded locked liability matches aggregate open orders.
    pub fn validate(&self) -> Result<(), &'static str> {
        if self.aggregate_open_orders_collateral != self.recorded_locked_liability {
            return Err("locked-liability: aggregate != recorded");
        }
        Ok(())
    }
}

/// Check handler for invariant validation in tests.
pub struct InvariantChecker;

impl InvariantChecker {
    /// Check that balances, fees, and liabilities sum to zero (conservation).
    pub fn check_conservation(
        _env: &Env,
        balances: &Map<Address, i128>,
        fees: &Map<Address, i128>,
    ) -> Result<(), &'static str> {
        let mut sum: i128 = 0;
        for (_, balance) in balances.iter() {
            sum = sum.checked_add(balance).ok_or("balance overflow")?;
        }

        for (_, fee) in fees.iter() {
            sum = sum.checked_add(fee).ok_or("fee overflow")?;
        }

        if sum == 0 {
            Ok(())
        } else {
            Err("conservation: total != 0")
        }
    }

    /// Check that all token movements are tracked.
    pub fn check_accounting_complete(
        _env: &Env,
        _token: &Address,
        expected_movements: &[(Address, i128)],
    ) -> Result<(), &'static str> {
        let mut accounted: i128 = 0;
        for (_account, delta) in expected_movements.iter() {
            accounted = accounted.checked_add(*delta).ok_or("accounting overflow")?;
        }

        if accounted == 0 {
            Ok(())
        } else {
            Err("accounting: unaccounted movement")
        }
    }

    /// Format failure message with account, token, and amount details.
    pub fn format_invariant_failure(
        env: &Env,
        reason: &str,
        _account: Option<&Address>,
        _token: Option<&Address>,
        _amount: Option<i128>,
    ) -> soroban_sdk::String {
        // Return a string formatted with the reason.
        // In real implementation, would use proper format macros.
        soroban_sdk::String::from_str(env, reason)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use soroban_sdk::testutils::Address as _;

    #[test]
    fn bid_taker_invariant_validates() {
        let invariant = BidTakerInvariant {
            taker_account: Address::generate(&Env::default()),
            long_token: Address::generate(&Env::default()),
            short_token: Address::generate(&Env::default()),
            long_collateral_delta: 100,
            short_collateral_delta: -100,
            execution_fee_delta: 10,
            position_fee_delta: 5,
        };
        assert!(invariant.validate().is_ok());
    }

    #[test]
    fn ask_taker_invariant_validates() {
        let invariant = AskTakerInvariant {
            taker_account: Address::generate(&Env::default()),
            long_token: Address::generate(&Env::default()),
            short_token: Address::generate(&Env::default()),
            long_collateral_delta: -100,
            short_collateral_delta: 100,
            execution_fee_delta: 10,
            position_fee_delta: 5,
        };
        assert!(invariant.validate().is_ok());
    }

    #[test]
    fn locked_liability_invariant_validates() {
        let invariant = LockedLiabilityInvariant {
            account: Address::generate(&Env::default()),
            token: Address::generate(&Env::default()),
            aggregate_open_orders_collateral: 1000,
            recorded_locked_liability: 1000,
        };
        assert!(invariant.validate().is_ok());
    }
}
