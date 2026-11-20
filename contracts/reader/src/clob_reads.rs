//! CLOB Contract Extensions — Deposits, Withdrawals, and Read APIs
//!
//! Provides custody operations (deposits/withdrawals), authorization checks,
//! and bounded read helpers for L2 order book depth, top-of-book quotes,
//! and trader balance exposure. Implements specifications from issues
//! #903, #904, #905, #906 (deposits/withdrawals) and #1019, #1018, #1020 (reads).
//!
//! Custody operations authenticate users, validate input, and maintain atomic
//! balance updates. Read functions are read-only and respect resource budgets
//! via pagination and bounded traversal.

use soroban_sdk::{contracttype, Address, Env, Vec, token};

// ─── Data Types ───────────────────────────────────────────────────────────────

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MarketRegistry {
    /// Market ID (contract address)
    pub id: Address,
    
    /// Base asset token
    pub base: Address,
    
    /// Quote asset token
    pub quote: Address,
    
    /// Ledger at which the market was created
    pub created_ledger: u32,
    
    /// Ledger at which the market was marked closed/deleted (if any)
    pub deleted_ledger: Option<u32>,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MarketCreatedEvent {
    /// Market ID
    pub market_id: Address,
    
    /// Base token
    pub base: Address,
    
    /// Quote token
    pub quote: Address,
    
    /// Base atoms per lot (immutable)
    pub lot_size: u64,
    
    /// Price tick increment (immutable)
    pub tick_size: u64,
    
    /// Maximum tick (immutable)
    pub max_tick: u64,
    
    /// Taker fee rate (immutable)
    pub taker_fee_rate: i128,
    
    /// Maker fee rate (immutable)
    pub maker_fee_rate: i128,
    
    /// Minimum order lots (mutable but set at creation)
    pub min_order_lots: u64,
    
    /// Max open orders per trader (mutable but set at creation)
    pub max_open_orders_per_trader: u32,
    
    /// Ledger at which the market was created
    pub created_ledger: u32,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PriceLevel {
    /// Price tick (normalized to market tick size)
    pub price_tick: u64,
    
    /// Aggregate remaining base asset lots at this level
    pub base_lots: u64,
    
    /// Number of orders at this level
    pub order_count: u32,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct L2Depth {
    /// Bid levels (descending price)
    pub bids: Vec<PriceLevel>,
    
    /// Ask levels (ascending price)
    pub asks: Vec<PriceLevel>,
    
    /// Cursor for next page: (is_bid, price_tick)
    /// None if no more data available
    pub next_cursor: Option<(bool, u64)>,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TopOfBook {
    /// Best bid level (highest buy price)
    pub best_bid: Option<PriceLevel>,
    
    /// Best ask level (lowest sell price)
    pub best_ask: Option<PriceLevel>,
    
    /// Spread in ticks (ask - bid), None if one-sided market
    pub spread_ticks: Option<u64>,
    
    /// Mid-price in ticks ((bid + ask) / 2), None if one-sided or overflow
    pub mid_price_ticks: Option<u64>,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MarketRegistryPage {
    /// Markets on this page
    pub markets: Vec<MarketRegistry>,
    
    /// Page index (0-based)
    pub page: u32,
    
    /// Total number of active markets (excluding deleted)
    pub total_markets: u32,
    
    /// Whether more pages exist
    pub has_next: bool,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TraderBalance {
    /// Total balance deposited
    pub total: i128,
    
    /// Available for withdrawal
    pub available: i128,
    
    /// Locked in open orders
    pub locked: i128,
    
    /// Pending settlement (fills not yet settled)
    pub pending: i128,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OpenOrderSummary {
    /// Number of active orders
    pub order_count: u32,
    
    /// Cursor for pagination (last order ID)
    pub cursor: Option<u64>,
}

// ─── Custody and Deposit Functions ────────────────────────────────────────

/// Validate and authorize a deposit request
///
/// # Arguments
/// * `depositor` - The account depositing tokens
/// * `asset` - The token address to deposit
/// * `amount` - The number of token units to deposit (must be positive)
///
/// # Authorization
/// Requires depositor.require_auth() to prevent unauthorized deposits
///
/// # Errors
/// - "deposit: unauthorized" if depositor has not signed the transaction
/// - "deposit: amount must be positive" if amount <= 0
/// - "deposit: unsupported asset" if asset is not in the market's supported tokens
///
/// # Implementation Status
/// TODO: Issue #903 - Validate against market token whitelist
pub fn validate_deposit(
    _env: &Env,
    depositor: Address,
    _asset: Address,
    amount: i128,
) -> Result<(), &'static str> {
    depositor.require_auth();

    if amount <= 0 {
        return Err("deposit: amount must be positive");
    }

    Ok(())
}

/// Transfer tokens into custody and credit available balance
///
/// # Arguments
/// * `trader` - The trader receiving custody credit
/// * `token_contract` - The token contract address
/// * `custody_address` - The authoritative custody contract/address
/// * `amount` - Amount to transfer (previously validated)
///
/// # Atomicity
/// Token transfer and internal credit are atomic - if transfer fails,
/// no internal balance update occurs. If internal credit fails after
/// successful transfer, the deposit is rejected.
///
/// # Events
/// Emits deposit event with (trader, token, amount, available_after_deposit)
///
/// # Implementation Status
/// TODO: Issue #904 - Integrate actual token transfer
/// TODO: Issue #904 - Emit deposit events
pub fn deposit_with_transfer(
    _env: &Env,
    _trader: Address,
    _token_contract: Address,
    _custody_address: Address,
    amount: i128,
) -> Result<(), &'static str> {
    if amount <= 0 {
        return Err("deposit: invalid amount after validation");
    }

    // Stub: Would transfer tokens to custody_address
    // Then credit trader's internal balance
    Ok(())
}

/// Determine custody ledger topology for multi-market deposits
///
/// # Returns
/// The scope at which balances are keyed:
/// - true if (trader, token) - global per-trader-per-token
/// - false if (market, trader, token) - per-market isolation
///
/// # Specification
/// Issue #905 requires explicit topology decision to prevent
/// double-counting tokens across markets.
///
/// # Implementation Status
/// TODO: Issue #905 - Load and return configured topology
pub fn get_custody_scope(_env: &Env) -> bool {
    true
}

/// Prevent double-counting of same token across markets
///
/// # Arguments
/// * `trader` - The trader account
/// * `asset` - The token being shared
/// * `markets` - Markets that may share this asset
///
/// # Validation
/// Ensures total available across all markets equals the sum of
/// individual market balances (no double-counting).
///
/// # Implementation Status
/// TODO: Issue #905 - Validate balance consistency across markets
pub fn validate_shared_asset_no_double_count(
    _env: &Env,
    _trader: Address,
    _asset: Address,
    _markets: &Vec<Address>,
) -> Result<(), &'static str> {
    Ok(())
}

// ─── Custody and Withdrawal Functions ─────────────────────────────────────

/// Validate and authorize a withdrawal request
///
/// # Arguments
/// * `trader` - The trader requesting withdrawal
/// * `asset` - The token to withdraw
/// * `amount` - The number of token units to withdraw
/// * `delegate` - Optional approved delegate (if trader uses delegation)
///
/// # Authorization
/// - If no delegate: requires trader.require_auth()
/// - If delegate present: requires delegate.require_auth()
///
/// # Validation
/// - trader must have amount available (not locked in orders)
/// - amount must be positive
/// - withdrawal fails if amount > available_balance
///
/// # Errors
/// - "withdrawal: unauthorized" if neither trader nor delegate signed
/// - "withdrawal: amount must be positive" if amount <= 0
/// - "withdrawal: insufficient available balance" if amount > available
///
/// # Implementation Status
/// TODO: Issue #906 - Connect to delegation registry (if needed)
pub fn validate_withdrawal(
    _env: &Env,
    trader: Address,
    _asset: Address,
    amount: i128,
    delegate: Option<Address>,
) -> Result<(), &'static str> {
    if amount <= 0 {
        return Err("withdrawal: amount must be positive");
    }

    match delegate {
        Some(d) => d.require_auth(),
        None => trader.require_auth(),
    }

    Ok(())
}

/// Check available balance and prevent over-withdrawal
///
/// # Arguments
/// * `available_balance` - Currently available (not locked) balance
/// * `amount` - Amount requested for withdrawal
///
/// # Validation
/// Ensures withdrawal respects the fully-collateralized-offer principle:
/// locked tokens in open orders cannot be withdrawn, even if total
/// (available + locked) would be sufficient.
///
/// # Implementation Status
/// TODO: Issue #906 - Load trader's available balance from storage
pub fn check_available_balance(
    available_balance: i128,
    amount: i128,
) -> Result<(), &'static str> {
    if amount > available_balance {
        return Err("withdrawal: insufficient available balance");
    }
    Ok(())
}

// ─── Market Creation ──────────────────────────────────────────────────────────

/// Create and initialize a new market with full configuration
///
/// # Arguments
/// * `base` - Base token address
/// * `quote` - Quote token address
/// * `lot_size` - Base atoms per lot (immutable)
/// * `tick_size` - Price increment in scaled units (immutable)
/// * `max_tick` - Maximum valid tick (immutable)
/// * `taker_fee_rate` - Taker fee in basis points (immutable)
/// * `maker_fee_rate` - Maker fee in basis points (immutable)
/// * `min_order_lots` - Minimum order size in lots (mutable)
/// * `max_open_orders_per_trader` - Per-trader limit (mutable)
///
/// # Authorization
/// Requires admin signature via address.require_auth()
///
/// # Returns
/// MarketCreatedEvent with full immutable configuration
///
/// # Errors
/// - "market: identical tokens" if base == quote
/// - "market: already exists" if market for this pair already created
/// - "market: invalid configuration" if any parameter is invalid
/// - "market: deployment failed" if contract instantiation fails
///
/// # Resource Bounds
/// - CPU: O(1) — deterministic ID derivation + storage write
/// - Memory: O(1)
/// - Storage writes: 2 (market metadata + registry entry)
///
/// # Guarantees
/// - Market ID is canonical and deterministic from (base, quote)
/// - Configuration is immutable after creation
/// - Event contains full configuration for indexer
/// - Idempotent: same market cannot be created twice
///
/// # Implementation Status
/// TODO: Issue #014 - Implement deterministic market deployment
/// TODO: Issue #014 - Emit MarketCreated event to contract ledger
pub fn create_market(
    _env: Env,
    _base: Address,
    _quote: Address,
    _lot_size: u64,
    _tick_size: u64,
    _max_tick: u64,
    _taker_fee_rate: i128,
    _maker_fee_rate: i128,
    _min_order_lots: u64,
    _max_open_orders_per_trader: u32,
) -> Result<MarketCreatedEvent, &'static str> {
    // Stub: Return placeholder
    // Real implementation will:
    // 1. Validate inputs via MarketConfig::validate()
    // 2. Derive canonical MarketId from (base, quote)
    // 3. Check if market already exists (prevent duplicates)
    // 4. Deploy/initialize market contract with config
    // 5. Store market metadata in registry
    // 6. Emit MarketCreated event with full configuration
    // 7. Return event for client confirmation
    
    Ok(MarketCreatedEvent {
        market_id: _base.clone(), // Placeholder
        base: _base,
        quote: _quote,
        lot_size: _lot_size,
        tick_size: _tick_size,
        max_tick: _max_tick,
        taker_fee_rate: _taker_fee_rate,
        maker_fee_rate: _maker_fee_rate,
        min_order_lots: _min_order_lots,
        max_open_orders_per_trader: _max_open_orders_per_trader,
        created_ledger: 0,
    })
}

// ─── Read Functions (Stub Implementations) ────────────────────────────────────

/// Enumerate markets in the registry with pagination
///
/// # Arguments
/// * `page` - Page number (0-based, capped at registry size / 100)
/// * `include_deleted` - Whether to include soft-deleted markets
///
/// # Returns
/// One page of up to 100 markets with pagination metadata
///
/// # Guarantees
/// - Enumeration never reads an unbounded list
/// - O(1) lookup per market (individual metadata loads)
/// - Exactly `MARKETS_PER_PAGE` = 100 per page bounds resource usage
/// - Total pages = ceil(total_markets / 100)
///
/// # Resource Bounds
/// - CPU: O(markets_on_page) — bounded by page size
/// - Memory: O(100) — fixed per-page allocation
/// - Storage reads: O(1 + markets_on_page) — one index read + market loads
///
/// # Implementation Status
/// TODO: Issue #015 - Load market registry pages from storage
pub fn enumerate_markets(
    _env: Env,
    page: u32,
    include_deleted: bool,
) -> Result<MarketRegistryPage, &'static str> {
    // Stub: Return empty page
    // Real implementation will:
    // 1. Load total market count
    // 2. Validate page number
    // 3. Load market IDs for this page from paginated storage
    // 4. Load metadata for each market ID
    // 5. Filter by deleted_ledger if include_deleted is false
    // 6. Return page with has_next = true if more markets exist
    
    Ok(MarketRegistryPage {
        markets: Vec::new(&_env),
        page,
        total_markets: 0,
        has_next: false,
    })
}

/// Look up a single market by ID
///
/// # Arguments
/// * `market_id` - The market address to look up
///
/// # Returns
/// Market metadata including base/quote tokens and creation time
///
/// # Guarantees
/// - O(1) lookup: direct storage access by market ID
/// - Returns None if market does not exist or is not yet synced
///
/// # Resource Bounds
/// - CPU: O(1)
/// - Memory: O(1)
/// - Storage reads: 1 (single metadata lookup)
///
/// # Implementation Status
/// TODO: Issue #015 - Load individual market metadata from storage
pub fn lookup_market(
    _env: Env,
    market_id: Address,
) -> Result<Option<MarketRegistry>, &'static str> {
    // Stub: Return None
    // Real implementation will:
    // 1. Load market metadata by ID from storage
    // 2. Return None if not found
    // 3. Return Some(metadata) if found
    
    Ok(None)
}

/// Read L2 order book depth with bounded pagination
///
/// # Arguments
/// * `market_id` - Market address
/// * `max_levels` - Maximum levels per side (capped at 50)
/// * `cursor` - Optional continuation: (is_bid, price_tick)
///
/// # Returns
/// Aggregated price levels with pagination cursor
///
/// # Resource Bounds
/// - CPU: O(max_levels * orders_per_level)
/// - Memory: O(max_levels)
/// - Storage reads: O(max_levels)
///
/// # Implementation Status
/// TODO: Issue #1019 - Connect to actual order book storage
pub fn read_l2_depth(
    _env: Env,
    _market_id: Address,
    max_levels: u32,
    _cursor: Option<(bool, u64)>,
) -> L2Depth {
    let max = max_levels.min(50); // Resource limit
    
    // Stub: Return empty depth
    // Real implementation will traverse price levels and aggregate orders
    L2Depth {
        bids: Vec::new(&_env),
        asks: Vec::new(&_env),
        next_cursor: if max > 0 { Some((true, 0)) } else { None },
    }
}

/// Read top-of-book market data (best bid/ask, spread, mid-price)
///
/// # Arguments
/// * `market_id` - Market address
///
/// # Returns
/// Best bid/ask with derived spread and mid-price
/// - Returns None for spread/mid on one-sided markets
/// - Uses overflow-safe arithmetic
///
/// # Resource Bounds
/// - CPU: O(1) - direct pointer reads
/// - Memory: O(1)
/// - Storage reads: 2 (best bid + best ask pointers)
///
/// # Implementation Status
/// TODO: Issue #1018 - Connect to market state storage
pub fn read_top_of_book(_env: Env, _market_id: Address) -> TopOfBook {
    // Stub: Return empty book
    // Real implementation will read best_bid_tick and best_ask_tick from market state
    TopOfBook {
        best_bid: None,
        best_ask: None,
        spread_ticks: None,
        mid_price_ticks: None,
    }
}

/// Read trader balance breakdown for a specific asset
///
/// # Arguments
/// * `market_id` - Market address
/// * `trader` - Trader address (must match caller)
/// * `asset` - Asset address (base or quote token)
///
/// # Returns
/// Balance breakdown: total, available, locked, pending
///
/// # Authorization
/// Requires trader.require_auth() - only trader can read their own balance
///
/// # Resource Bounds
/// - CPU: O(1)
/// - Memory: O(1)
/// - Storage reads: 1 (account state)
///
/// # Implementation Status
/// TODO: Issue #1020 - Connect to account custody storage
pub fn read_trader_balance(
    _env: Env,
    _market_id: Address,
    trader: Address,
    _asset: Address,
) -> TraderBalance {
    trader.require_auth();
    
    // Stub: Return zero balance
    // Real implementation will load account state and compute buckets
    TraderBalance {
        total: 0,
        available: 0,
        locked: 0,
        pending: 0,
    }
}

/// Read open order summary for trader
///
/// # Arguments
/// * `market_id` - Market address
/// * `trader` - Trader address (must match caller)
/// * `cursor` - Optional continuation (last order ID)
/// * `limit` - Maximum orders to return (capped at 100)
///
/// # Returns
/// (order_ids, summary) tuple with pagination cursor
///
/// # Authorization
/// Requires trader.require_auth()
///
/// # Resource Bounds
/// - CPU: O(limit)
/// - Memory: O(limit)
/// - Storage reads: 1 + limit
///
/// # Implementation Status
/// TODO: Issue #1020 - Connect to trader order index
pub fn read_open_orders(
    _env: Env,
    _market_id: Address,
    trader: Address,
    _cursor: Option<u64>,
    limit: u32,
) -> (Vec<u64>, OpenOrderSummary) {
    trader.require_auth();
    
    let max_limit = limit.min(100); // Resource cap
    
    // Stub: Return empty result
    // Real implementation will load trader's order list and paginate
    let order_ids = Vec::new(&_env);
    let summary = OpenOrderSummary {
        order_count: 0,
        cursor: if max_limit > 0 { Some(0) } else { None },
    };
    
    (order_ids, summary)
}

// ─── Helper Functions ─────────────────────────────────────────────────────────

/// Aggregate all orders at a single price level
///
/// # Implementation Note
/// This is a helper for read_l2_depth and read_top_of_book.
/// Real implementation will:
/// 1. Load order IDs from PriceLevelOrders storage
/// 2. Load each order and sum remaining_lots
/// 3. Return None if level is empty
#[allow(dead_code)]
fn aggregate_price_level(
    _env: &Env,
    _market_id: &Address,
    _is_bid: bool,
    price_tick: u64,
) -> Option<PriceLevel> {
    // Stub: Return dummy level
    Some(PriceLevel {
        price_tick,
        base_lots: 0,
        order_count: 0,
    })
}

/// Compute mid-price with overflow-safe arithmetic
///
/// # Returns
/// None if:
/// - Either bid or ask is None (one-sided market)
/// - (bid + ask) would overflow u64
///
/// # Formula
/// mid = (bid + ask) / 2
#[allow(dead_code)]
fn safe_mid_price(best_bid: Option<u64>, best_ask: Option<u64>) -> Option<u64> {
    match (best_bid, best_ask) {
        (Some(bid), Some(ask)) => {
            // checked_add prevents overflow
            bid.checked_add(ask).map(|sum| sum / 2)
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_safe_mid_price_normal() {
        assert_eq!(safe_mid_price(Some(100), Some(110)), Some(105));
    }
    
    #[test]
    fn test_safe_mid_price_one_sided() {
        assert_eq!(safe_mid_price(Some(100), None), None);
        assert_eq!(safe_mid_price(None, Some(110)), None);
    }
    
    #[test]
    fn test_safe_mid_price_overflow() {
        let max = u64::MAX;
        assert_eq!(safe_mid_price(Some(max - 10), Some(max - 5)), None);
    }
}
