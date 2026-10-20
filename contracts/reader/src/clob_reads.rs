//! CLOB Read API Extensions — Phase 7
//!
//! Provides bounded read helpers for L2 order book depth, top-of-book quotes,
//! and trader balance exposure. Implements specifications from issues
//! #1019, #1018, #1020.
//!
//! These functions are read-only and respect resource budgets via pagination
//! and bounded traversal.

use soroban_sdk::{contracttype, Address, Env, Vec};

// ─── Data Types ───────────────────────────────────────────────────────────────

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

// ─── Read Functions (Stub Implementations) ────────────────────────────────────

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
