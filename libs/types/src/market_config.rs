// Issue #883 [CLOB-007]: Specify tick-size and lot-size rules for every market
//! Market configuration with consensus-critical tick and lot size rules.
//!
//! Tick and lot sizes are not UI hints; they are normalization rules that keep order keys
//! compact and arithmetic deterministic. All price and amount validation enforces these rules.

use soroban_sdk::contracttype;

/// Market configuration with tick and lot size rules.
/// Consensus-critical: changes require protocol coordination.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MarketConfig {
    /// Tick size in atomic units of the quote asset.
    /// All prices must be a multiple of tick_size.
    /// Must be > 0; enforced on market creation.
    pub tick_size: u128,

    /// Base lot size in atomic units of the base asset.
    /// Order amounts must be a multiple of base_lot_size.
    /// Must be > 0; enforced on market creation.
    pub base_lot_size: u128,

    /// Minimum notional value (price * amount in quote units).
    /// Orders below this threshold are rejected.
    /// Protects against dust orders and extreme spreads.
    pub min_notional: u128,
}

impl MarketConfig {
    /// Create a new market configuration.
    /// Returns None if tick_size or base_lot_size is zero.
    pub fn new(tick_size: u128, base_lot_size: u128, min_notional: u128) -> Option<Self> {
        if tick_size == 0 || base_lot_size == 0 {
            return None;
        }
        Some(MarketConfig {
            tick_size,
            base_lot_size,
            min_notional,
        })
    }

    /// Validate that a price is aligned to tick size.
    pub fn validate_price(&self, price: u128) -> Result<(), &'static str> {
        if price == 0 {
            return Err("Price must be non-zero");
        }
        if price % self.tick_size != 0 {
            return Err("Price not aligned to tick size");
        }
        Ok(())
    }

    /// Validate that an amount is aligned to lot size.
    pub fn validate_amount(&self, amount: u128) -> Result<(), &'static str> {
        if amount == 0 {
            return Err("Amount must be non-zero");
        }
        if amount % self.base_lot_size != 0 {
            return Err("Amount not aligned to lot size");
        }
        Ok(())
    }

    /// Validate notional value (price * amount / decimals_adjustment).
    /// decimal_adjustment accounts for base/quote precision difference.
    pub fn validate_notional(
        &self,
        price: u128,
        amount: u128,
        decimal_adjustment: u128,
    ) -> Result<(), &'static str> {
        let notional = price
            .checked_mul(amount)
            .and_then(|v| v.checked_div(decimal_adjustment))
            .ok_or("Notional overflow")?;

        if notional < self.min_notional {
            return Err("Notional value below minimum");
        }
        Ok(())
    }

    /// Round an amount up to the nearest valid lot size.
    pub fn round_amount_up(&self, amount: u128) -> Option<u128> {
        if amount == 0 {
            return Some(self.base_lot_size);
        }
        let remainder = amount % self.base_lot_size;
        if remainder == 0 {
            Some(amount)
        } else {
            amount.checked_add(self.base_lot_size - remainder)
        }
    }

    /// Round a price up to the nearest valid tick.
    pub fn round_price_up(&self, price: u128) -> Option<u128> {
        if price == 0 {
            return Some(self.tick_size);
        }
        let remainder = price % self.tick_size;
        if remainder == 0 {
            Some(price)
        } else {
            price.checked_add(self.tick_size - remainder)
        }
    }

    /// Round a price down to the nearest valid tick.
    pub fn round_price_down(&self, price: u128) -> Option<u128> {
        if price == 0 {
            return None;
        }
        Some(price - (price % self.tick_size))
    }
}

/// Errors for market configuration validation
#[derive(Debug, Eq, PartialEq)]
pub enum ConfigError {
    ZeroTickSize,
    ZeroLotSize,
    PriceOffTick,
    AmountOffLot,
    BelowMinNotional,
    InvalidPrice,
    InvalidAmount,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_market_config_creation() {
        let cfg = MarketConfig::new(100, 1000, 10000);
        assert!(cfg.is_some());

        let none_tick = MarketConfig::new(0, 1000, 10000);
        assert!(none_tick.is_none());

        let none_lot = MarketConfig::new(100, 0, 10000);
        assert!(none_lot.is_none());
    }

    #[test]
    fn test_price_validation() {
        let cfg = MarketConfig::new(100, 1000, 10000).unwrap();

        // Valid prices (multiples of 100)
        assert!(cfg.validate_price(100).is_ok());
        assert!(cfg.validate_price(200).is_ok());
        assert!(cfg.validate_price(50000).is_ok());

        // Invalid prices
        assert!(cfg.validate_price(0).is_err()); // zero
        assert!(cfg.validate_price(50).is_err()); // not multiple of 100
        assert!(cfg.validate_price(250).is_err()); // not multiple of 100
    }

    #[test]
    fn test_amount_validation() {
        let cfg = MarketConfig::new(100, 1000, 10000).unwrap();

        // Valid amounts (multiples of 1000)
        assert!(cfg.validate_amount(1000).is_ok());
        assert!(cfg.validate_amount(2000).is_ok());

        // Invalid amounts
        assert!(cfg.validate_amount(0).is_err()); // zero
        assert!(cfg.validate_amount(500).is_err()); // not multiple of 1000
        assert!(cfg.validate_amount(1500).is_err()); // not multiple of 1000
    }

    #[test]
    fn test_notional_validation() {
        let cfg = MarketConfig::new(100, 1000, 100_000).unwrap();

        // price=100, amount=1000, decimal_adjustment=1 → notional=100_000 ✓
        assert!(cfg.validate_notional(100, 1000, 1).is_ok());

        // price=50, amount=1000, decimal_adjustment=1 → notional=50_000 ✗
        assert!(cfg.validate_notional(50, 1000, 1).is_err());
    }

    #[test]
    fn test_rounding() {
        let cfg = MarketConfig::new(100, 1000, 10000).unwrap();

        // Round price up
        assert_eq!(cfg.round_price_up(150), Some(200));
        assert_eq!(cfg.round_price_up(100), Some(100));
        assert_eq!(cfg.round_price_up(0), Some(100));

        // Round price down
        assert_eq!(cfg.round_price_down(150), Some(100));
        assert_eq!(cfg.round_price_down(100), Some(100));
        assert_eq!(cfg.round_price_down(50), Some(0));

        // Round amount up
        assert_eq!(cfg.round_amount_up(1500), Some(2000));
        assert_eq!(cfg.round_amount_up(1000), Some(1000));
    }

    #[test]
    fn test_decimal_precision_handling() {
        // 6-decimal base, 8-decimal quote: adjustment = 1e8 / 1e6 = 100
        let cfg = MarketConfig::new(100, 1000, 100_000).unwrap();

        // price=1e6 (1 quote per base), amount=1000 (0.001 base)
        // notional = 1e6 * 1000 / 100 = 1e7 = 10_000_000 quote atoms
        assert!(cfg.validate_notional(1_000_000, 1000, 100).is_ok());
    }
}