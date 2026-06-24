#![no_std]
#![allow(dependency_on_unit_never_type_fallback)]

use soroban_sdk::{contracttype, Address};

pub const MAX_SWAP_PATH_LENGTH: usize = 5;

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SwapPath {
    pub hop0: Option<Address>,
    pub hop1: Option<Address>,
    pub hop2: Option<Address>,
    pub hop3: Option<Address>,
    pub hop4: Option<Address>,
}

impl SwapPath {
    pub fn new() -> Self {
        Self {
            hop0: None,
            hop1: None,
            hop2: None,
            hop3: None,
            hop4: None,
        }
    }

    pub fn from_array<const N: usize>(hops: [Address; N]) -> Self {
        if N > MAX_SWAP_PATH_LENGTH {
            panic!("swap path too long");
        }

        let mut path = Self::new();
        let mut i = 0usize;
        while i < N {
            path.set(i, hops[i].clone());
            i += 1;
        }
        path
    }

    pub fn len(&self) -> u32 {
        let mut count = 0u32;
        while count < MAX_SWAP_PATH_LENGTH as u32 {
            if self.get(count).is_none() {
                break;
            }
            count += 1;
        }
        count
    }

    pub fn is_empty(&self) -> bool {
        self.hop0.is_none()
    }

    pub fn get(&self, index: u32) -> Option<Address> {
        match index {
            0 => self.hop0.clone(),
            1 => self.hop1.clone(),
            2 => self.hop2.clone(),
            3 => self.hop3.clone(),
            4 => self.hop4.clone(),
            _ => None,
        }
    }

    fn set(&mut self, index: usize, address: Address) {
        match index {
            0 => self.hop0 = Some(address),
            1 => self.hop1 = Some(address),
            2 => self.hop2 = Some(address),
            3 => self.hop3 = Some(address),
            4 => self.hop4 = Some(address),
            _ => panic!("swap path index out of bounds"),
        }
    }
}

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
    pub swap_path: SwapPath,
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
    pub swap_path: SwapPath,
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
