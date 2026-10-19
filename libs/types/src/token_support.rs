// Issue #914 [CLOB-038]: Define supported token behavior and reject unsafe token assumptions
//! Supported token capabilities, listing policy validation, and safe custody invariants.
//!
//! Follows the fully-collateralized-offer principle: an accepted resting order must
//! always be backed 100% by already-reserved funds. Internal custody accounting
//! (`available + locked + protocol_fees == actual_vault_balance`) must never credit
//! a failed transfer or accept non-standard tokens that violate linear conservation.

use soroban_sdk::{contracttype, Address};

/// Capabilities and properties of a token evaluated for CLOB listing.
#[contracttype]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TokenCapability {
    /// Standard Stellar Asset Contract (SAC) with linear transfer semantics.
    StandardSac,
    /// Custom SEP-41 compliant Soroban token with standard transfer semantics.
    CustomSep41,
    /// Token issuer has administrative clawback capability enabled.
    ClawbackCapable,
    /// Token issuer has administrative freeze capability enabled.
    FreezeCapable,
    /// Token exhibits supply or balance rebasing (unsupported).
    Rebasing,
    /// Token deducts a fee or burn on transfer (unsupported).
    FeeOnTransfer,
}

/// Listing validation policy for CLOB markets.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TokenListingPolicy {
    /// Whether markets under this policy permit clawback-capable assets.
    pub allow_clawback: bool,
    /// Whether markets under this policy permit freeze-capable assets.
    pub allow_freeze: bool,
    /// Expected decimal places (default 7 for Stellar stroop alignment).
    pub expected_decimals: u32,
}

impl Default for TokenListingPolicy {
    fn default() -> Self {
        Self::standard()
    }
}

impl TokenListingPolicy {
    /// Standard permissionless market policy: no clawback, no freeze, 7 decimals.
    pub const fn standard() -> Self {
        Self {
            allow_clawback: false,
            allow_freeze: false,
            expected_decimals: 7,
        }
    }

    /// Regulated market policy: allows issuer clawback/freeze when disclosed.
    pub const fn regulated() -> Self {
        Self {
            allow_clawback: true,
            allow_freeze: true,
            expected_decimals: 7,
        }
    }

    /// Validate a token against this listing policy.
    pub fn validate_token(
        &self,
        is_compliant_interface: bool,
        decimals: u32,
        is_clawback: bool,
        is_freeze: bool,
        is_rebasing: bool,
        is_fee_on_transfer: bool,
    ) -> Result<(), &'static str> {
        if !is_compliant_interface {
            return Err("Token does not implement required SEP-41/SAC interface");
        }
        if is_rebasing {
            return Err("Rebasing tokens are strictly unsupported");
        }
        if is_fee_on_transfer {
            return Err("Fee-on-transfer tokens are strictly unsupported");
        }
        if decimals != self.expected_decimals {
            return Err("Token decimals do not match market policy");
        }
        if is_clawback && !self.allow_clawback {
            return Err("Clawback-capable tokens not permitted in standard markets");
        }
        if is_freeze && !self.allow_freeze {
            return Err("Freeze-capable tokens not permitted in standard markets");
        }
        Ok(())
    }
}

/// Safe custody balance ledger for an asset within a market or exchange vault.
///
/// Invariant:
/// `available + locked + protocol_fees == actual_vault_balance`
#[contracttype]
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct SafeCustodyLedger {
    pub token: Option<Address>,
    /// Spendable balance owned by traders and not locked in orders.
    pub available: i128,
    /// Collateral reserved for active resting orders.
    pub locked: i128,
    /// Accumulated protocol fees not yet swept.
    pub protocol_fees: i128,
}

impl SafeCustodyLedger {
    /// Create a new empty custody ledger for a token.
    pub fn new(token: Address) -> Self {
        Self {
            token: Some(token),
            available: 0,
            locked: 0,
            protocol_fees: 0,
        }
    }

    /// Total protocol-accounted funds for this asset.
    pub fn total_accounted(&self) -> Result<i128, &'static str> {
        let sum = self
            .available
            .checked_add(self.locked)
            .ok_or("Custody overflow in available + locked")?
            .checked_add(self.protocol_fees)
            .ok_or("Custody overflow in total + protocol_fees")?;
        Ok(sum)
    }

    /// Reconcile internal accounting against observed token vault balance.
    ///
    /// Reverts if any mismatch is detected, preventing insolvent execution.
    pub fn reconcile_with_vault(&self, actual_vault_balance: i128) -> Result<(), &'static str> {
        let accounted = self.total_accounted()?;
        if accounted != actual_vault_balance {
            return Err("Custody reconciliation failure: total_accounted != actual_vault_balance");
        }
        Ok(())
    }

    /// Process a token deposit with transfer verification.
    ///
    /// Guarantees:
    /// 1. Failed or simulated transfers with 0 delta are never credited.
    /// 2. Under-delivering tokens (fee-on-transfer) are detected and rejected.
    /// 3. Internal ledger is updated only upon verified vault balance increase.
    pub fn checked_deposit(
        &mut self,
        declared_amount: i128,
        pre_vault_balance: i128,
        post_vault_balance: i128,
    ) -> Result<(), &'static str> {
        if declared_amount <= 0 {
            return Err("Deposit amount must be positive");
        }

        let actual_received = post_vault_balance
            .checked_sub(pre_vault_balance)
            .ok_or("Vault balance arithmetic underflow")?;

        if actual_received <= 0 {
            return Err("Transfer failed: no tokens received by vault");
        }

        if actual_received < declared_amount {
            return Err("Fee-on-transfer detected: actual received < declared amount");
        }

        if actual_received > declared_amount {
            return Err("Unexpected balance delta: actual received > declared amount");
        }

        self.available = self
            .available
            .checked_add(actual_received)
            .ok_or("Available balance overflow")?;

        Ok(())
    }

    /// Lock collateral for an accepted resting order.
    ///
    /// Follows fully-collateralized-offer principle: cannot lock more than available.
    pub fn checked_lock(&mut self, amount: i128) -> Result<(), &'static str> {
        if amount <= 0 {
            return Err("Lock amount must be positive");
        }
        if amount > self.available {
            return Err("Insufficient available balance to lock order collateral");
        }

        self.available = self
            .available
            .checked_sub(amount)
            .ok_or("Available balance underflow")?;
        self.locked = self
            .locked
            .checked_add(amount)
            .ok_or("Locked balance overflow")?;

        Ok(())
    }

    /// Unlock collateral when an order is cancelled or expires.
    pub fn checked_unlock(&mut self, amount: i128) -> Result<(), &'static str> {
        if amount <= 0 {
            return Err("Unlock amount must be positive");
        }
        if amount > self.locked {
            return Err("Cannot unlock more than locked collateral");
        }

        self.locked = self
            .locked
            .checked_sub(amount)
            .ok_or("Locked balance underflow")?;
        self.available = self
            .available
            .checked_add(amount)
            .ok_or("Available balance overflow")?;

        Ok(())
    }

    /// Consume locked collateral on order fill and credit protocol fees.
    pub fn checked_fill_settlement(
        &mut self,
        locked_consumed: i128,
        fee_to_protocol: i128,
        trader_refund: i128,
    ) -> Result<(), &'static str> {
        if locked_consumed <= 0 {
            return Err("Locked consumed must be positive");
        }
        if locked_consumed > self.locked {
            return Err("Cannot consume more than locked collateral");
        }

        let total_out = fee_to_protocol
            .checked_add(trader_refund)
            .ok_or("Settlement delta overflow")?;

        if total_out > locked_consumed {
            return Err("Settlement exceeds locked collateral");
        }

        self.locked = self
            .locked
            .checked_sub(locked_consumed)
            .ok_or("Locked balance underflow")?;

        if fee_to_protocol > 0 {
            self.protocol_fees = self
                .protocol_fees
                .checked_add(fee_to_protocol)
                .ok_or("Protocol fee overflow")?;
        }

        if trader_refund > 0 {
            self.available = self
                .available
                .checked_add(trader_refund)
                .ok_or("Trader refund available overflow")?;
        }

        Ok(())
    }

    /// Process a withdrawal, reducing available balance.
    pub fn checked_withdraw(
        &mut self,
        amount: i128,
        pre_vault_balance: i128,
        post_vault_balance: i128,
    ) -> Result<(), &'static str> {
        if amount <= 0 {
            return Err("Withdrawal amount must be positive");
        }
        if amount > self.available {
            return Err("Insufficient available balance for withdrawal");
        }

        let actual_dispatched = pre_vault_balance
            .checked_sub(post_vault_balance)
            .ok_or("Vault balance arithmetic underflow")?;

        if actual_dispatched != amount {
            return Err("Vault balance dispatch mismatch on withdrawal");
        }

        self.available = self
            .available
            .checked_sub(amount)
            .ok_or("Available balance underflow")?;

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_standard_policy_validates_clean_token() {
        let policy = TokenListingPolicy::standard();
        assert!(policy
            .validate_token(true, 7, false, false, false, false)
            .is_ok());
    }

    #[test]
    fn test_standard_policy_rejects_rebasing_token() {
        let policy = TokenListingPolicy::standard();
        let err = policy
            .validate_token(true, 7, false, false, true, false)
            .unwrap_err();
        assert_eq!(err, "Rebasing tokens are strictly unsupported");
    }

    #[test]
    fn test_standard_policy_rejects_fee_on_transfer() {
        let policy = TokenListingPolicy::standard();
        let err = policy
            .validate_token(true, 7, false, false, false, true)
            .unwrap_err();
        assert_eq!(err, "Fee-on-transfer tokens are strictly unsupported");
    }

    #[test]
    fn test_standard_policy_rejects_clawback_capable_token() {
        let policy = TokenListingPolicy::standard();
        let err = policy
            .validate_token(true, 7, true, false, false, false)
            .unwrap_err();
        assert_eq!(
            err,
            "Clawback-capable tokens not permitted in standard markets"
        );
    }

    #[test]
    fn test_regulated_policy_permits_clawback_capable_token() {
        let policy = TokenListingPolicy::regulated();
        assert!(policy
            .validate_token(true, 7, true, true, false, false)
            .is_ok());
    }

    #[test]
    fn test_custody_deposit_and_reconcile() {
        let mut ledger = SafeCustodyLedger::default();
        let declared = 10_000_000i128; // 1 token at 7 decimals

        // Pre = 0, post = 10_000_000
        assert!(ledger.checked_deposit(declared, 0, 10_000_000).is_ok());
        assert_eq!(ledger.available, 10_000_000);
        assert_eq!(ledger.locked, 0);
        assert_eq!(ledger.protocol_fees, 0);

        // Vault reconciliation passes
        assert!(ledger.reconcile_with_vault(10_000_000).is_ok());
    }

    #[test]
    fn test_custody_never_credits_failed_transfer() {
        let mut ledger = SafeCustodyLedger::default();
        let declared = 5_000_000i128;

        // Simulated failed transfer: post == pre
        let res = ledger.checked_deposit(declared, 1_000_000, 1_000_000);
        assert!(res.is_err());
        assert_eq!(res.unwrap_err(), "Transfer failed: no tokens received by vault");
        assert_eq!(ledger.available, 0);
        assert_eq!(ledger.total_accounted().unwrap(), 0);
    }

    #[test]
    fn test_custody_rejects_fee_on_transfer_underdelivery() {
        let mut ledger = SafeCustodyLedger::default();
        let declared = 10_000_000i128;

        // Token took 5% fee: received only 9_500_000
        let res = ledger.checked_deposit(declared, 0, 9_500_000);
        assert!(res.is_err());
        assert_eq!(
            res.unwrap_err(),
            "Fee-on-transfer detected: actual received < declared amount"
        );
        assert_eq!(ledger.available, 0);
    }

    #[test]
    fn test_custody_lock_and_unlock_invariants() {
        let mut ledger = SafeCustodyLedger::default();
        ledger.checked_deposit(1_000, 0, 1_000).unwrap();

        // Cannot lock more than available
        assert!(ledger.checked_lock(1_001).is_err());

        // Lock 400
        assert!(ledger.checked_lock(400).is_ok());
        assert_eq!(ledger.available, 600);
        assert_eq!(ledger.locked, 400);
        assert_eq!(ledger.total_accounted().unwrap(), 1_000);
        assert!(ledger.reconcile_with_vault(1_000).is_ok());

        // Cannot unlock more than locked
        assert!(ledger.checked_unlock(401).is_err());

        // Unlock 150
        assert!(ledger.checked_unlock(150).is_ok());
        assert_eq!(ledger.available, 750);
        assert_eq!(ledger.locked, 250);
        assert_eq!(ledger.total_accounted().unwrap(), 1_000);
        assert!(ledger.reconcile_with_vault(1_000).is_ok());
    }

    #[test]
    fn test_custody_fill_settlement_and_fees() {
        let mut ledger = SafeCustodyLedger::default();
        ledger.checked_deposit(1_000, 0, 1_000).unwrap();
        ledger.checked_lock(1_000).unwrap();

        // Fill with price improvement refund:
        // locked_consumed = 1000, fee = 30, refund = 70 (net traded = 900)
        assert!(ledger.checked_fill_settlement(1_000, 30, 70).is_ok());
        assert_eq!(ledger.locked, 0);
        assert_eq!(ledger.protocol_fees, 30);
        assert_eq!(ledger.available, 70);

        // 900 left the vault to taker: actual vault balance is now 100
        assert_eq!(ledger.total_accounted().unwrap(), 100);
        assert!(ledger.reconcile_with_vault(100).is_ok());
    }

    #[test]
    fn test_reconciliation_fails_on_clawback_or_leak() {
        let mut ledger = SafeCustodyLedger::default();
        ledger.checked_deposit(1_000, 0, 1_000).unwrap();

        // If vault balance drops to 700 without an exchange operation (e.g. issuer clawback)
        assert!(ledger.reconcile_with_vault(700).is_err());
    }
}
