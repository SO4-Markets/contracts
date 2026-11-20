//! CLOB settlement core library.
//!
//! All fallible functions return `Result<_, SettlementError>` so errors can be
//! encoded into `ScError` at every contract boundary without a string-match
//! translation layer. See issue #1127.
#![no_std]

use soroban_sdk::contracterror;

// ─── Error enum ──────────────────────────────────────────────────────────────

/// Typed error variants for every failure case in the settlement pipeline.
///
/// Discriminant ranges:
///   1–49   validation errors (tick, amount, price, invariant)
///  50–99   fee / quote arithmetic errors
/// 100–149  fill / settlement errors
/// 150–199  deposit / withdrawal flow errors
/// 200–249  market-config errors  (replaces MarketConfigError)
/// 250–299  registry errors       (replaces RegistryError)
/// 300–349  deployment errors     (replaces DeploymentError)
///
/// Values are stable — never reorder or renumber after a crate version has
/// been deployed, as on-chain callers may compare the numeric code directly.
#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
#[repr(u32)]
pub enum SettlementError {
    // ── Validation (1–49) ───────────────────────────────────────────────────
    /// Tick value is outside the allowed grid.
    InvalidTick = 1,
    /// Amount must be strictly positive.
    AmountNotPositive = 2,
    /// Price is zero or negative.
    InvalidPrice = 3,
    /// Trader-account invariant violated (e.g. negative balance after fill).
    TraderInvariantViolated = 4,
    /// Order size exceeds the per-order maximum.
    OrderSizeExceedsMax = 5,
    /// Order side is not recognised (must be bid or ask).
    InvalidSide = 6,
    /// Sequence number is not monotonically increasing.
    InvalidSequenceNumber = 7,
    /// Order has already been filled or cancelled.
    OrderAlreadyConsumed = 8,

    // ── Fee / quote arithmetic (50–99) ───────────────────────────────────────
    /// Fee-on-quote calculation overflowed.
    FeeOverflow = 50,
    /// Quote atoms calculation produced a negative result.
    QuoteAtomsNegative = 51,
    /// Fee rate exceeds the protocol maximum (10 000 bps).
    FeeRateExceedsMax = 52,
    /// Maker rebate exceeds the taker fee, leaving a negative net fee.
    RebateExceedsFee = 53,

    // ── Fill / settlement (100–149) ──────────────────────────────────────────
    /// Fill quantity exceeds the remaining order quantity.
    FillExceedsOrder = 100,
    /// Counterparty does not have enough base tokens to deliver.
    InsufficientBaseTokens = 101,
    /// Counterparty does not have enough quote tokens to deliver.
    InsufficientQuoteTokens = 102,
    /// Legs in a multi-leg fill do not net to zero.
    FillLegsImbalanced = 103,
    /// Settlement token transfer failed (token contract returned an error).
    TransferFailed = 104,
    /// Self-trade detected; fill rejected per self-trade prevention policy.
    SelfTrade = 105,

    // ── Deposit / withdrawal (150–199) ───────────────────────────────────────
    /// Deposit amount is zero.
    DepositAmountZero = 150,
    /// Deposit would push the total vault balance above the per-asset cap.
    DepositExceedsCap = 151,
    /// Withdrawal amount exceeds the trader's available balance.
    WithdrawalExceedsBalance = 152,
    /// Withdrawal is locked because an open order references this balance.
    WithdrawalLockedByOrder = 153,
    /// Withdrawal destination address is not set.
    WithdrawalAddressNotSet = 154,

    // ── Market-config errors (200–249) — replaces MarketConfigError ──────────
    /// Market token address is not registered.
    MarketNotFound = 200,
    /// Long-token and short-token are the same address.
    SameTokenPair = 201,
    /// Market is currently halted for new orders.
    MarketHalted = 202,
    /// Tick size is zero or not a power of ten.
    InvalidTickSize = 203,
    /// Lot size is zero.
    InvalidLotSize = 204,
    /// Requested market already exists.
    MarketAlreadyExists = 205,

    // ── Registry errors (250–299) — replaces RegistryError ───────────────────
    /// Contract address is not registered in the protocol registry.
    ContractNotRegistered = 250,
    /// Caller is not authorised to update the registry.
    RegistryUnauthorized = 251,
    /// Registry is frozen; no further registrations are allowed.
    RegistryFrozen = 252,

    // ── Deployment errors (300–349) — replaces DeploymentError ───────────────
    /// Wasm hash does not match the expected deployment hash.
    WasmHashMismatch = 300,
    /// Contract was already deployed at this address.
    AlreadyDeployed = 301,
    /// Deployer is not on the allow-list.
    DeployerUnauthorized = 302,
    /// Salt produced an address collision.
    SaltCollision = 303,
}
