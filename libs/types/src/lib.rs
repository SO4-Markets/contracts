#![no_std]
#![allow(dependency_on_unit_never_type_fallback)]

use soroban_sdk::{contracttype, Address, Vec};

// ─── Price ───────────────────────────────────────────────────────────────────

/// USD price with min/max spread; 30-decimal FLOAT_PRECISION. Mirrors GMX's Price.Props.
#[contracttype]
pub struct PriceProps {
    pub min: i128,
    pub max: i128,
}

impl PriceProps {
    pub fn is_empty(&self) -> bool {
        self.min == 0 || self.max == 0
    }

    pub fn mid_price(&self) -> i128 {
        (self.max + self.min) / 2
    }

    pub fn pick_price(&self, maximize: bool) -> i128 {
        if maximize {
            self.max
        } else {
            self.min
        }
    }

    /// Longs profit from higher prices, shorts from lower.
    /// maximize=true → worst-case PnL for the LP / best-case for the trader.
    pub fn pick_price_for_pnl(&self, is_long: bool, maximize: bool) -> i128 {
        match (is_long, maximize) {
            (true, true) => self.max,
            (true, false) => self.min,
            (false, true) => self.min,
            (false, false) => self.max,
        }
    }
}

// ─── Market ──────────────────────────────────────────────────────────────────

/// Mirrors GMX's Market.Props.
#[contracttype]
pub struct MarketProps {
    pub market_token: Address,
    pub index_token: Address,
    pub long_token: Address,
    pub short_token: Address,
}

impl MarketProps {
    /// Construct a `MarketProps` from borrowed addresses.
    ///
    /// Issue #248: `MarketProps` carries only `Address` fields, every one of which
    /// is required and has no meaningful zero value (Soroban `Address` cannot be
    /// constructed without an `Env`, so a `Default` impl is not possible). The
    /// per-field struct literal — repeated verbatim across handlers, libs, and
    /// every test — is the actual boilerplate. This constructor collapses that
    /// six-line literal into a single call and clones internally so callers keep
    /// ownership of their addresses, which is the common case when the same
    /// addresses are reused to build other structs in the same scope.
    pub fn new(
        market_token: &Address,
        index_token: &Address,
        long_token: &Address,
        short_token: &Address,
    ) -> Self {
        Self {
            market_token: market_token.clone(),
            index_token: index_token.clone(),
            long_token: long_token.clone(),
            short_token: short_token.clone(),
        }
    }
}

// ─── Position ────────────────────────────────────────────────────────────────

/// Mirrors GMX's Position.Props.
/// Field name abbreviations (30-char limit in #[contracttype]):
///   long_claim_fnd_per_size  = longTokenClaimableFundingAmountPerSize
///   short_claim_fnd_per_size = shortTokenClaimableFundingAmountPerSize
#[contracttype]
pub struct PositionProps {
    pub account: Address,
    pub market: Address,
    pub collateral_token: Address,
    pub size_in_usd: i128,
    pub size_in_tokens: i128,
    pub collateral_amount: i128,
    pub pending_impact_amount: i128,
    pub borrowing_factor: i128,
    pub funding_fee_amount_per_size: i128,
    pub long_claim_fnd_per_size: i128,
    pub short_claim_fnd_per_size: i128,
    pub increased_at_time: u64,
    pub decreased_at_time: u64,
    pub is_long: bool,
}

// ─── Orders ──────────────────────────────────────────────────────────────────

/// Mirrors GMX's Order.OrderType.
#[contracttype]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum OrderType {
    MarketSwap,
    LimitSwap,
    MarketIncrease,
    LimitIncrease,
    MarketDecrease,
    LimitDecrease,
    StopLossDecrease,
    Liquidation,
    StopIncrease,
}

/// Mirrors GMX's Order.Props.
#[contracttype]
pub struct OrderProps {
    pub account: Address,
    pub receiver: Address,
    pub market: Address,
    pub initial_collateral_token: Address,
    pub swap_path: Vec<Address>,
    pub size_delta_usd: i128,
    pub collateral_delta_amount: i128,
    pub trigger_price: i128,
    pub acceptable_price: i128,
    pub execution_fee: i128,
    pub min_output_amount: i128,
    pub order_type: OrderType,
    pub is_long: bool,
    pub updated_at_time: u64,
}

// ─── Handler create-params (shared so router doesn't depend on handler crates) ─

/// User-supplied parameters for creating a deposit.
#[contracttype]
pub struct CreateDepositParams {
    pub receiver: Address,
    pub market: Address,
    pub initial_long_token: Address,
    pub initial_short_token: Address,
    pub long_token_amount: i128,
    pub short_token_amount: i128,
    pub min_market_tokens: i128,
    pub execution_fee: i128,
}

/// User-supplied parameters for creating a withdrawal.
#[contracttype]
pub struct CreateWithdrawalParams {
    pub receiver: Address,
    pub market: Address,
    pub market_token_amount: i128,
    pub min_long_token_amount: i128,
    pub min_short_token_amount: i128,
    pub execution_fee: i128,
}

/// User-supplied parameters for creating an order. Mirrors GMX BaseOrderUtils.CreateOrderParams.
#[contracttype]
pub struct CreateOrderParams {
    pub receiver: Address,
    pub market: Address,
    pub initial_collateral_token: Address,
    pub swap_path: Vec<Address>,
    pub size_delta_usd: i128,
    pub collateral_delta_amount: i128,
    pub trigger_price: i128,
    pub acceptable_price: i128,
    pub execution_fee: i128,
    pub min_output_amount: i128,
    pub order_type: OrderType,
    pub is_long: bool,
}

// ─── Deposits / Withdrawals ───────────────────────────────────────────────────

/// Mirrors GMX's Deposit.Props.
#[contracttype]
pub struct DepositProps {
    pub account: Address,
    pub receiver: Address,
    pub market: Address,
    pub initial_long_token: Address,
    pub initial_short_token: Address,
    pub long_token_amount: i128,
    pub short_token_amount: i128,
    pub min_market_tokens: i128,
    pub execution_fee: i128,
    pub updated_at_time: u64,
}

/// Mirrors GMX's Withdrawal.Props.
#[contracttype]
pub struct WithdrawalProps {
    pub account: Address,
    pub receiver: Address,
    pub market: Address,
    pub market_token_amount: i128,
    pub min_long_token_amount: i128,
    pub min_short_token_amount: i128,
    pub execution_fee: i128,
    pub updated_at_time: u64,
}

// ─── Oracle ───────────────────────────────────────────────────────────────────

/// Used by keepers to submit prices to the oracle contract.
#[contracttype]
pub struct TokenPrice {
    pub token: Address,
    pub min: i128,
    pub max: i128,
}

// ─── Market utils output types ────────────────────────────────────────────────

/// Full pool value breakdown returned by market_utils::get_pool_value.
#[contracttype]
pub struct PoolValueInfo {
    pub pool_value: i128,
    pub long_pnl: i128,
    pub short_pnl: i128,
    pub net_pnl: i128,
    pub long_token_amount: i128,
    pub short_token_amount: i128,
    pub long_token_usd: i128,
    pub short_token_usd: i128,
    pub total_borrowing_fees: i128,
    pub impact_pool_amount: i128,
}

/// Aggregate funding information for a market (used by Reader).
#[contracttype]
pub struct FundingInfo {
    pub funding_factor_per_second: i128,
    pub long_funding_amount_per_size: i128,
    pub short_funding_amount_per_size: i128,
}

/// Detailed per-hour funding rate view for frontend display (issue #207).
#[contracttype]
pub struct FundingRateInfo {
    pub long_funding_rate_per_hour: i128,
    pub short_funding_rate_per_hour: i128,
    pub long_funding_amount_per_size: i128,
    pub short_funding_amount_per_size: i128,
    pub funding_updated_at_ledger: u64,
    pub long_open_interest_usd: u128,
    pub short_open_interest_usd: u128,
}

/// Position fee breakdown.
#[contracttype]
pub struct PositionFees {
    pub borrowing_fee_amount: i128,
    pub funding_fee_amount: i128,
    pub position_fee_amount: i128,
    pub total_cost_amount: i128,
}

/// Result of executing a position decrease (partial or full close).
#[contracttype]
pub struct DecreasePositionResult {
    pub execution_price: i128,         // FLOAT_PRECISION per whole token
    pub pnl_usd: i128,                 // realised PnL (positive = profit, negative = loss)
    pub output_amount: i128,           // collateral token amount sent to receiver
    pub secondary_output_amount: i128, // optional second token (e.g. from swap-on-close)
    pub remaining_collateral: i128,    // collateral left in position after fees & pnl
    pub is_fully_closed: bool,
}

/// Aggregated protocol-wide statistics across a set of markets (returned by Reader).
///
/// Issue #251: lets the frontend fetch headline numbers (TVL, OI, accumulated
/// fees) in a single call instead of N per-market round-trips. All USD figures
/// use 30-decimal FLOAT_PRECISION and reflect the oracle prices at the ledger
/// in which the call executes — the result is a snapshot, valid only for
/// `computed_at_ledger`.
#[contracttype]
pub struct ProtocolStats {
    pub total_pool_value_usd: i128,       // sum of get_pool_value across all markets
    pub total_long_open_interest_usd: i128,
    pub total_short_open_interest_usd: i128,
    pub total_accumulated_fees_usd: i128, // sum of unclaimed fee balances, in USD
    pub market_count: u32,                // number of markets aggregated
    pub computed_at_ledger: u64,          // ledger sequence the snapshot was taken at
}

/// Liveness status for a keeper role (issue #249), returned by Reader.
///
/// A keeper that has gone permanently offline leaves orders unexecuted. This
/// gives the admin an on-chain signal: when `is_stale` is true the gap since the
/// last successful execution has exceeded the configured heartbeat timeout, and
/// the keeper's role can be revoked.
#[contracttype]
pub struct KeeperHeartbeatStatus {
    pub last_active_ledger: u64,
    pub ledgers_since_last_activity: u64,
    pub is_stale: bool, // gap > keeper_heartbeat_timeout(role)
}

/// Rich position info including computed PnL and fees (returned by Reader).
#[contracttype]
pub struct PositionInfo {
    pub position: PositionProps,
    pub pnl_usd: i128,
    pub uncapped_pnl_usd: i128,
    pub borrowing_fee_usd: i128,
    pub funding_fee_usd: i128,
    pub position_fee_usd: i128,
    pub liquidation_price: i128,
}

/// ADL-eligible position candidate for auto-deleveraging.
/// Returned by reader::get_adl_eligible_positions.
#[contracttype]
pub struct AdlCandidate {
    pub key: BytesN<32>,                  // position key for order_handler::get_position
    pub owner: Address,                   // position owner
    pub size_usd: u128,                   // position size in USD (absolute value)
    pub unrealised_pnl_usd: u128,         // positive PnL only (loss positions are filtered)
    pub pnl_to_size_ratio_bps: u32,       // unrealised_pnl / size in basis points (sort key)
}

/// Estimated swap output for dry-run queries.
/// Returned by reader::estimate_swap_output.
#[contracttype]
pub struct SwapEstimate {
    pub token_out: Address,               // output token after full swap path
    pub amount_out: u128,                 // estimated output amount
    pub price_impact_usd: i128,           // cumulative impact (negative = cost, positive = rebate)
    pub execution_price: u128,            // effective rate for the full swap path
    pub reverts_if_executed: bool,        // true if any market is paused or insufficient liquidity
}

/// Position leverage breakdown returned by reader::get_position_leverage.
/// leverage_bps = size_usd * 100 / net_collateral_usd (e.g. 2000 = 20×).
/// If net_collateral_usd == 0, effective_leverage_bps == u32::MAX.
#[contracttype]
pub struct PositionLeverage {
    pub effective_leverage_bps: u32,
    pub net_collateral_usd: u128,
    pub position_size_usd: u128,
    pub is_liquidatable: bool,
}

/// Lightweight pending-order summary returned by reader::get_pending_orders (issue #202).
///
/// Carries only the fields a keeper bot needs to decide execution order:
/// owner, type, size, fee, last-update time, and direction.
/// Full details can always be fetched via reader::get_order if needed.
#[contracttype]
pub struct PendingOrder {
    pub owner: Address,
    pub market: Address,
    pub order_type: OrderType,
    pub size_delta_usd: i128,
    pub execution_fee: i128,
    pub updated_at_time: u64,
    pub is_long: bool,
}

/// Liquidatable position entry returned by `reader::get_liquidatable_positions` (issue #283).
///
/// `health_factor_bps < 10000` means the position is below the min-collateral threshold
/// (i.e., currently eligible for liquidation). Sorted ascending so the most
/// under-collateralised positions appear first.
#[contracttype]
pub struct LiquidatablePosition {
    pub key: BytesN<32>,           // canonical position key for order_handler::get_position
    pub owner: Address,
    pub size_usd: u128,
    pub collateral_usd: u128,
    pub health_factor_bps: u32,    // collateral_usd * 10000 / size_usd; < 10000 = liquidatable
}

use soroban_sdk::BytesN;
