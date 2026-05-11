#![no_std]

use soroban_sdk::Address;

/// Price struct with min and max values (30-decimal precision)
#[derive(Clone, Debug)]
pub struct PriceProps {
    pub min: i128,
    pub max: i128,
}

impl PriceProps {
    /// Check if price is empty/invalid
    pub fn is_empty(&self) -> bool {
        self.min == 0 || self.max == 0
    }

    /// Get midpoint price
    pub fn mid_price(&self) -> i128 {
        (self.max + self.min) / 2
    }

    /// Pick min or max based on maximize flag
    pub fn pick_price(&self, maximize: bool) -> i128 {
        if maximize {
            self.max
        } else {
            self.min
        }
    }

    /// Pick price for PnL calculation
    pub fn pick_price_for_pnl(&self, is_long: bool, maximize: bool) -> i128 {
        if is_long {
            if maximize {
                self.max
            } else {
                self.min
            }
        } else {
            if maximize {
                self.min
            } else {
                self.max
            }
        }
    }
}

/// Market properties
#[derive(Clone, Debug)]
pub struct MarketProps {
    pub market_token: Address,
    pub index_token: Address,
    pub long_token: Address,
    pub short_token: Address,
}

/// Position properties
#[derive(Clone, Debug)]
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
    pub long_token_claimable_funding_per_size: i128,
    pub short_token_claimable_funding_per_size: i128,
    pub increased_at_time: u64,
    pub decreased_at_time: u64,
    pub is_long: bool,
}

/// Order type enum
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum OrderType {
    MarketSwap = 0,
    LimitSwap = 1,
    MarketIncrease = 2,
    LimitIncrease = 3,
    MarketDecrease = 4,
    LimitDecrease = 5,
    StopLossDecrease = 6,
    Liquidation = 7,
    StopIncrease = 8,
}

/// Order properties
#[derive(Clone, Debug)]
pub struct OrderProps {
    pub account: Address,
    pub receiver: Address,
    pub market: Address,
    pub initial_collateral_token: Address,
    pub size_delta_usd: i128,
    pub initial_collateral_delta_amount: i128,
    pub trigger_price: i128,
    pub acceptable_price: i128,
    pub execution_fee: i128,
    pub min_output_amount: i128,
    pub order_type: OrderType,
    pub is_long: bool,
    pub updated_at_time: u64,
}

/// Deposit properties
#[derive(Clone, Debug)]
pub struct DepositProps {
    pub account: Address,
    pub receiver: Address,
    pub market: Address,
    pub initial_long_token: Address,
    pub initial_short_token: Address,
    pub long_token_amount: i128,
    pub short_token_amount: i128,
    pub min_market_tokens: i128,
    pub updated_at_time: u64,
}

/// Withdrawal properties
#[derive(Clone, Debug)]
pub struct WithdrawalProps {
    pub account: Address,
    pub receiver: Address,
    pub market: Address,
    pub market_token_amount: i128,
    pub min_long_token_amount: i128,
    pub min_short_token_amount: i128,
    pub updated_at_time: u64,
}

/// Funding info result
#[derive(Clone, Debug)]
pub struct FundingInfo {
    pub funding_factor_per_second: i128,
    pub funding_amount_per_size_delta: (i128, i128), // (long, short)
}

/// Position info for reader
#[derive(Clone, Debug)]
pub struct PositionInfo {
    pub position: PositionProps,
    pub pnl_usd: i128,
    pub uncapped_pnl_usd: i128,
    pub borrowing_fees: i128,
    pub funding_fees: i128,
    pub position_fees: i128,
}
