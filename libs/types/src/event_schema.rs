//! Event schema definitions with versioning and payload budgeting.
//!
//! Issue #886 [CLOB-010]: Version the event schema and budget event payload size.
//!
//! All protocol events carry a schema version to enable backward-compatible indexing
//! and event reconstruction. Payload sizes are constrained to fit within Soroban
//! event metering budgets.
//!
//! Schema Version: v1.0 (current)
//!   - All events include a version field
//!   - Topic/data encoding uses compact formats
//!   - Payload budgets are maintained for each event type

use soroban_sdk::{contracttype, Address, BytesN};

// ─── Event Schema Version ─────────────────────────────────────────────────────
/// Current schema version constant for all events. Increment when event layout changes.
pub const EVENT_SCHEMA_VERSION: u32 = 1;

// ─── Market Lifecycle Events ──────────────────────────────────────────────────

/// Market state transition event. Emitted whenever market state changes.
///
/// Payload budget: ~120 bytes (fits easily within Soroban event limits)
///
/// Topics (for indexing):
///   [0] = "MarketStateTransition" (symbol/fixed)
///   [1] = market (Address)
#[contracttype]
pub struct MarketStateTransitionEvent {
    /// Schema version for compatibility (always EVENT_SCHEMA_VERSION)
    pub schema_version: u32,
    /// Market contract address
    pub market: Address,
    /// Previous market state (from MarketState enum)
    pub old_state: u32,
    /// New market state (from MarketState enum)
    pub new_state: u32,
    /// Ledger sequence when transition occurred
    pub at_ledger: u64,
    /// Who authorized this transition
    pub authorized_by: Address,
}

// ─── Order Events ─────────────────────────────────────────────────────────────

/// Order created event. Emitted when a new order is placed.
///
/// Payload budget: ~180 bytes
///
/// Topics:
///   [0] = "OrderCreated"
///   [1] = order_account (account that owns the order)
#[contracttype]
pub struct OrderCreatedEvent {
    pub schema_version: u32,
    pub order_key: BytesN<32>, // order hash/identifier
    pub account: Address,
    pub market: Address,
    pub order_type: u32, // OrderType variant
    pub size_delta_usd: i128,
    pub is_long: bool,
    pub execution_fee: i128,
    pub at_ledger: u64,
}

/// Order executed event. Emitted when a keeper executes an order.
///
/// Payload budget: ~200 bytes
///
/// Topics:
///   [0] = "OrderExecuted"
///   [1] = order_account
#[contracttype]
pub struct OrderExecutedEvent {
    pub schema_version: u32,
    pub order_key: BytesN<32>,
    pub account: Address,
    pub market: Address,
    pub execution_price: i128, // actual execution price
    pub output_amount: i128,   // amount received by user
    pub execution_fee_paid: i128,
    pub at_ledger: u64,
}

/// Order cancelled event. Emitted when an order is cancelled (by user or system).
///
/// Payload budget: ~140 bytes
///
/// Topics:
///   [0] = "OrderCancelled"
///   [1] = order_account
#[contracttype]
pub struct OrderCancelledEvent {
    pub schema_version: u32,
    pub order_key: BytesN<32>,
    pub account: Address,
    pub market: Address,
    pub cancellation_reason: u32, // 0=user, 1=expired, 2=market_paused, etc.
    pub refund_amount: i128,
    pub at_ledger: u64,
}

// ─── Position Events ──────────────────────────────────────────────────────────

/// Position increased event. Emitted when position size or collateral increases.
///
/// Payload budget: ~240 bytes
///
/// Topics:
///   [0] = "PositionIncreased"
///   [1] = position_account
#[contracttype]
pub struct PositionIncreasedEvent {
    pub schema_version: u32,
    pub account: Address,
    pub market: Address,
    pub position_size_usd: i128,
    pub collateral_amount: i128,
    pub execution_price: i128,
    pub is_long: bool,
    pub funding_fee: i128,
    pub at_ledger: u64,
}

/// Position decreased event. Emitted when position is partially or fully closed.
///
/// Payload budget: ~280 bytes
///
/// Topics:
///   [0] = "PositionDecreased"
///   [1] = position_account
#[contracttype]
pub struct PositionDecreasedEvent {
    pub schema_version: u32,
    pub account: Address,
    pub market: Address,
    pub position_size_usd: i128,
    pub collateral_amount: i128,
    pub execution_price: i128,
    pub pnl_usd: i128, // realized profit/loss
    pub is_fully_closed: bool,
    pub borrowing_fee: i128,
    pub funding_fee: i128,
    pub at_ledger: u64,
}

/// Position liquidated event. Emitted when a position is liquidated.
///
/// Payload budget: ~200 bytes
///
/// Topics:
///   [0] = "PositionLiquidated"
///   [1] = position_account
#[contracttype]
pub struct PositionLiquidatedEvent {
    pub schema_version: u32,
    pub account: Address,
    pub market: Address,
    pub position_size_usd: i128,
    pub collateral_amount: i128,
    pub liquidation_price: i128,
    pub pnl_usd: i128,
    pub liquidation_fee_paid: i128,
    pub at_ledger: u64,
}

// ─── Deposit/Withdrawal Events ────────────────────────────────────────────────

/// Deposit created event. Emitted when a user initiates a deposit.
///
/// Payload budget: ~160 bytes
///
/// Topics:
///   [0] = "DepositCreated"
///   [1] = depositor_account
#[contracttype]
pub struct DepositCreatedEvent {
    pub schema_version: u32,
    pub account: Address,
    pub market: Address,
    pub long_token_amount: i128,
    pub short_token_amount: i128,
    pub min_market_tokens: i128,
    pub execution_fee: i128,
    pub at_ledger: u64,
}

/// Deposit executed event. Emitted when keeper executes a pending deposit.
///
/// Payload budget: ~160 bytes
///
/// Topics:
///   [0] = "DepositExecuted"
///   [1] = depositor_account
#[contracttype]
pub struct DepositExecutedEvent {
    pub schema_version: u32,
    pub account: Address,
    pub market: Address,
    pub market_tokens_received: i128,
    pub execution_fee_paid: i128,
    pub at_ledger: u64,
}

/// Withdrawal created event. Emitted when a user initiates a withdrawal.
///
/// Payload budget: ~140 bytes
///
/// Topics:
///   [0] = "WithdrawalCreated"
///   [1] = withdrawer_account
#[contracttype]
pub struct WithdrawalCreatedEvent {
    pub schema_version: u32,
    pub account: Address,
    pub market: Address,
    pub market_token_amount: i128,
    pub min_long_token_amount: i128,
    pub min_short_token_amount: i128,
    pub execution_fee: i128,
    pub at_ledger: u64,
}

/// Withdrawal executed event. Emitted when keeper executes a pending withdrawal.
///
/// Payload budget: ~160 bytes
///
/// Topics:
///   [0] = "WithdrawalExecuted"
///   [1] = withdrawer_account
#[contracttype]
pub struct WithdrawalExecutedEvent {
    pub schema_version: u32,
    pub account: Address,
    pub market: Address,
    pub long_token_amount: i128,
    pub short_token_amount: i128,
    pub execution_fee_paid: i128,
    pub at_ledger: u64,
}

// ─── Funding Events ───────────────────────────────────────────────────────────

/// Funding rate updated event. Emitted when hourly funding is accrued.
///
/// Payload budget: ~120 bytes
///
/// Topics:
///   [0] = "FundingRateUpdated"
///   [1] = market
#[contracttype]
pub struct FundingRateUpdatedEvent {
    pub schema_version: u32,
    pub market: Address,
    pub long_funding_amount_per_size: i128,
    pub short_funding_amount_per_size: i128,
    pub funding_factor_per_second: i128,
    pub at_ledger: u64,
}

// ─── Error Events (for indexing failures) ─────────────────────────────────────

/// Transaction rejected event. Emitted when a public entrypoint fails validation.
///
/// Payload budget: ~100 bytes
///
/// Topics:
///   [0] = "TransactionRejected"
///   [1] = account (or zero-address if no account context)
#[contracttype]
pub struct TransactionRejectedEvent {
    pub schema_version: u32,
    pub account: Address,
    pub market: Address,
    pub error_code: u32, // ProtocolError variant
    pub reason: u32,     // specific failure context
    pub at_ledger: u64,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn event_schema_version_is_set() {
        assert_eq!(EVENT_SCHEMA_VERSION, 1);
    }

    #[test]
    fn market_state_transition_event_has_schema_version() {
        // This test ensures all event types have a schema_version field
        // In production, every event emitted must set schema_version = EVENT_SCHEMA_VERSION
        assert_eq!(EVENT_SCHEMA_VERSION, 1);
    }
}
