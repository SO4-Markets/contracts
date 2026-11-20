//! Typed protocol error model for SO4 CLOB.
//!
//! Issue #884 [CLOB-008]: Defines stable error codes for validation, authorization,
//! balance, book, matching, lifecycle, resource-bound, and configuration failures.
//!
//! Every public entrypoint failure path maps to a typed error where practical.
//! Error codes are reserved in ranges to avoid accidental reuse:
//!   - Validation errors:      1-999
//!   - Authorization errors:   1000-1999
//!   - Balance/token errors:   2000-2999
//!   - Book/order errors:      3000-3999
//!   - Matching errors:        4000-4999
//!   - Market lifecycle:       5000-5999
//!   - Resource bounds:        6000-6999
//!   - Configuration errors:   7000-7999
//!   - Future/reserved:        8000-9999

use soroban_sdk::contracterror;

#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
#[repr(u32)]
pub enum ProtocolError {
    // ─── Validation errors (1-999) ───────────────────────────────────────────
    /// Invalid or null address provided
    InvalidAddress = 1,
    /// Invalid order parameters (e.g., zero size, negative amount)
    InvalidOrderParams = 2,
    /// Invalid market or token configuration
    InvalidMarketConfig = 3,
    /// Price or value out of acceptable range
    InvalidPrice = 4,
    /// Timestamp or sequence number out of valid range
    InvalidTimestamp = 5,
    /// Zero or negative amount provided where positive is required
    ZeroAmount = 6,
    /// Collateral or fee amount insufficient for operation
    InsufficientAmount = 7,
    /// Swap path is empty or contains invalid tokens
    InvalidSwapPath = 8,
    /// Execution fee does not meet minimum requirement
    ExecutionFeeTooLow = 9,
    /// Trigger price invalid for order type
    InvalidTriggerPrice = 10,

    // ─── Authorization errors (1000-1999) ────────────────────────────────────
    /// Caller is not authorized to perform this action
    Unauthorized = 1000,
    /// Caller is not in the required role
    RoleRequired = 1001,
    /// Only admin can call this function
    AdminRequired = 1002,
    /// Only keeper can call this function
    KeeperRequired = 1003,
    /// Order does not belong to caller
    OrderNotOwnedByAccount = 1004,
    /// Position does not belong to caller
    PositionNotOwnedByAccount = 1005,
    /// Deposit does not belong to caller
    DepositNotOwnedByAccount = 1006,
    /// Withdrawal does not belong to caller
    WithdrawalNotOwnedByAccount = 1007,
    /// Caller is not the position manager authorized by owner
    NotPositionManager = 1008,

    // ─── Balance and token errors (2000-2999) ────────────────────────────────
    /// Account balance insufficient for operation
    InsufficientBalance = 2000,
    /// Token contract call failed
    TokenOperationFailed = 2001,
    /// Transfer of funds failed
    TransferFailed = 2002,
    /// Token decimals mismatch or invalid
    InvalidTokenDecimals = 2003,
    /// Pool does not have sufficient liquidity
    InsufficientLiquidity = 2004,
    /// Impact pool balance is insufficient
    ImpactPoolInsufficient = 2005,
    /// Fee balance is insufficient for sweep
    FeeBalanceInsufficient = 2006,

    // ─── Book and order errors (3000-3999) ───────────────────────────────────
    /// Order not found
    OrderNotFound = 3000,
    /// Order has already been executed
    OrderAlreadyExecuted = 3001,
    /// Order has already been cancelled
    OrderAlreadyCancelled = 3002,
    /// Order has expired (past expiry ledger)
    OrderExpired = 3003,
    /// Acceptable price for execution not met (slippage)
    SlippageExceeded = 3004,
    /// Order already exists with this key
    OrderAlreadyExists = 3005,
    /// Collateral token for order is invalid
    InvalidCollateralToken = 3006,
    /// Too many orders in batch (exceeds max batch size)
    OrderBatchTooLarge = 3007,

    // ─── Matching and position errors (4000-4999) ────────────────────────────
    /// Position not found
    PositionNotFound = 4000,
    /// Position is already open
    PositionAlreadyOpen = 4001,
    /// Cannot close position that is not open
    PositionNotOpen = 4002,
    /// Insufficient position collateral for operation
    InsufficientPositionCollateral = 4003,
    /// Position size would exceed maximum allowed
    PositionSizeExceeded = 4004,
    /// Leverage exceeds maximum allowed
    LeverageTooHigh = 4005,
    /// Position is eligible for liquidation
    PositionLiquidatable = 4006,
    /// Liquidation price calculation failed
    LiquidationPriceCalcFailed = 4007,
    /// Position is in reduce-only mode; increase not allowed
    ReduceOnlyModeViolation = 4008,

    // ─── Market lifecycle errors (5000-5999) ──────────────────────────────────
    /// Market does not exist
    MarketNotFound = 5000,
    /// Market is in Pending state; operations not yet allowed
    MarketPending = 5001,
    /// Market is Paused; new orders not accepted
    MarketPaused = 5002,
    /// Market is in ReduceOnly state; only decreases allowed
    MarketReduceOnly = 5003,
    /// Market is in CancelOnly state; only cancellations allowed
    MarketCancelOnly = 5004,
    /// Market is Closed; no operations allowed
    MarketClosed = 5005,
    /// Market already exists
    MarketAlreadyExists = 5006,
    /// Invalid state transition requested
    InvalidStateTransition = 5007,
    /// Market state not initialized
    MarketStateNotInitialized = 5008,

    // ─── Resource bound errors (6000-6999) ────────────────────────────────────
    /// Unbounded iteration would exceed resource limits
    UnboundedIterationDetected = 6000,
    /// Vector or collection exceeds maximum allowed size
    CollectionTooLarge = 6001,
    /// Recursion depth exceeded
    RecursionDepthExceeded = 6002,
    /// Storage quota exceeded
    StorageQuotaExceeded = 6003,
    /// Operation would require excessive gas
    GasLimitExceeded = 6004,

    // ─── Configuration errors (7000-7999) ──────────────────────────────────────
    /// Configuration value is out of range or invalid
    InvalidConfiguration = 7000,
    /// Configuration not initialized
    ConfigurationNotInitialized = 7001,
    /// Parameter was already set and cannot be changed
    ParameterImmutable = 7002,
    /// WASM hash not set (required for deployment)
    WasmHashNotSet = 7003,
    /// Dependency contract not found or invalid
    DependencyContractNotFound = 7004,
    /// Duplicate configuration entry
    DuplicateEntry = 7005,

    // ─── Oracle and pricing errors (7100-7199) ────────────────────────────────
    /// Oracle price not available
    OraclePriceNotAvailable = 7100,
    /// Oracle price is stale
    OraclePriceStale = 7101,
    /// Oracle price feed misconfigured
    OraclePriceFeedMisconfigured = 7102,

    // ─── Storage and data errors (7200-7299) ───────────────────────────────────
    /// Data store operation failed
    DataStorageError = 7200,
    /// Role store operation failed
    RoleStorageError = 7201,
    /// Invalid storage key
    InvalidStorageKey = 7202,
    /// Storage entry not found
    StorageEntryNotFound = 7203,

    // ─── Internal/fallback error ──────────────────────────────────────────────
    /// Internal error; contact protocol maintainers
    InternalError = 9000,
}

impl ProtocolError {
    /// Return a human-readable message for SDK consumption.
    pub fn message(&self) -> &'static str {
        match self {
            // Validation
            Self::InvalidAddress => "Invalid or null address provided",
            Self::InvalidOrderParams => "Invalid order parameters",
            Self::InvalidMarketConfig => "Invalid market configuration",
            Self::InvalidPrice => "Price or value out of acceptable range",
            Self::InvalidTimestamp => "Timestamp or sequence number out of valid range",
            Self::ZeroAmount => "Amount must be positive",
            Self::InsufficientAmount => "Amount is insufficient for operation",
            Self::InvalidSwapPath => "Swap path is empty or contains invalid tokens",
            Self::ExecutionFeeTooLow => "Execution fee below minimum requirement",
            Self::InvalidTriggerPrice => "Trigger price invalid for order type",

            // Authorization
            Self::Unauthorized => "Caller is not authorized for this action",
            Self::RoleRequired => "Caller must have required role",
            Self::AdminRequired => "Only admin can call this function",
            Self::KeeperRequired => "Only keeper can call this function",
            Self::OrderNotOwnedByAccount => "Order does not belong to caller",
            Self::PositionNotOwnedByAccount => "Position does not belong to caller",
            Self::DepositNotOwnedByAccount => "Deposit does not belong to caller",
            Self::WithdrawalNotOwnedByAccount => "Withdrawal does not belong to caller",
            Self::NotPositionManager => "Caller is not authorized position manager",

            // Balance/token
            Self::InsufficientBalance => "Account balance insufficient",
            Self::TokenOperationFailed => "Token contract operation failed",
            Self::TransferFailed => "Fund transfer failed",
            Self::InvalidTokenDecimals => "Token decimals invalid or mismatched",
            Self::InsufficientLiquidity => "Insufficient pool liquidity",
            Self::ImpactPoolInsufficient => "Impact pool balance insufficient",
            Self::FeeBalanceInsufficient => "Fee balance insufficient for sweep",

            // Book/order
            Self::OrderNotFound => "Order not found",
            Self::OrderAlreadyExecuted => "Order already executed",
            Self::OrderAlreadyCancelled => "Order already cancelled",
            Self::OrderExpired => "Order has expired",
            Self::SlippageExceeded => "Price slippage exceeds acceptable range",
            Self::OrderAlreadyExists => "Order with this key already exists",
            Self::InvalidCollateralToken => "Invalid collateral token for order",
            Self::OrderBatchTooLarge => "Order batch exceeds maximum size",

            // Matching/position
            Self::PositionNotFound => "Position not found",
            Self::PositionAlreadyOpen => "Position is already open",
            Self::PositionNotOpen => "Position is not open",
            Self::InsufficientPositionCollateral => "Insufficient position collateral",
            Self::PositionSizeExceeded => "Position size exceeds maximum",
            Self::LeverageTooHigh => "Leverage exceeds maximum allowed",
            Self::PositionLiquidatable => "Position is liquidatable",
            Self::LiquidationPriceCalcFailed => "Liquidation price calculation failed",
            Self::ReduceOnlyModeViolation => "Market is reduce-only; increase not allowed",

            // Market lifecycle
            Self::MarketNotFound => "Market not found",
            Self::MarketPending => "Market pending; operations not allowed",
            Self::MarketPaused => "Market paused; new orders not accepted",
            Self::MarketReduceOnly => "Market reduce-only; only decreases allowed",
            Self::MarketCancelOnly => "Market cancel-only; only cancellations allowed",
            Self::MarketClosed => "Market closed; no operations allowed",
            Self::MarketAlreadyExists => "Market already exists",
            Self::InvalidStateTransition => "Invalid market state transition",
            Self::MarketStateNotInitialized => "Market state not initialized",

            // Resource bounds
            Self::UnboundedIterationDetected => "Unbounded iteration would exceed limits",
            Self::CollectionTooLarge => "Collection exceeds maximum size",
            Self::RecursionDepthExceeded => "Recursion depth exceeded",
            Self::StorageQuotaExceeded => "Storage quota exceeded",
            Self::GasLimitExceeded => "Operation requires excessive gas",

            // Configuration
            Self::InvalidConfiguration => "Invalid configuration value",
            Self::ConfigurationNotInitialized => "Configuration not initialized",
            Self::ParameterImmutable => "Parameter was already set and is immutable",
            Self::WasmHashNotSet => "WASM hash not set",
            Self::DependencyContractNotFound => "Dependency contract not found",
            Self::DuplicateEntry => "Duplicate configuration entry",

            // Oracle/pricing
            Self::OraclePriceNotAvailable => "Oracle price not available",
            Self::OraclePriceStale => "Oracle price is stale",
            Self::OraclePriceFeedMisconfigured => "Oracle price feed misconfigured",

            // Storage
            Self::DataStorageError => "Data store operation failed",
            Self::RoleStorageError => "Role store operation failed",
            Self::InvalidStorageKey => "Invalid storage key",
            Self::StorageEntryNotFound => "Storage entry not found",

            // Internal
            Self::InternalError => "Internal error; contact protocol maintainers",
        }
    }
}
