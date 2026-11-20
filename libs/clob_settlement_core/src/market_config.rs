// CLOB market configuration validation
// Addresses issue #900: one validation path shared by market creation and
// permitted configuration updates.

use soroban_sdk::Address;

use crate::{quote_atoms, MarketFeeConfig, RoundingMode, MAX_FEE_RATE};

/// Upper bound on `max_open_orders_per_trader`. Cancelling or expiring a
/// trader's orders walks this many entries, so it bounds that work. The value is
/// provisional until the benchmark harness produces resource evidence.
pub const MAX_OPEN_ORDERS_PER_TRADER_LIMIT: u32 = 1_024;

/// Trading status of a market. Every status is valid both at creation and after
/// an update, so a status change can never produce a configuration that
/// creation would reject.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u32)]
pub enum MarketStatus {
    /// Placement, matching, and cancellation are open.
    Active = 0,
    /// Only cancellation and expiry run; no new orders.
    CancelOnly = 1,
    /// No order activity. Deposits and withdrawals are unaffected.
    Halted = 2,
}

/// The field an update tried to change although it is fixed at creation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u32)]
pub enum ImmutableField {
    Base = 0,
    Quote = 1,
    LotSize = 2,
    TickSize = 3,
    MaxTick = 4,
    TakerFeeRate = 5,
    MakerFeeRate = 6,
}

/// Every way a market configuration can be invalid. Discriminants are stable so
/// a contract can surface them as error codes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u32)]
pub enum MarketConfigError {
    /// Base and quote are the same token contract (spec section 3).
    IdenticalTokens = 1,
    LotSizeZero = 2,
    TickSizeZero = 3,
    MaxTickZero = 4,
    /// `max_tick * tick_size` overflows `u64`, so some ticks have no valid price.
    TickRangeOverflow = 5,
    TakerFeeOutOfRange = 6,
    MakerFeeOutOfRange = 7,
    MinOrderLotsZero = 8,
    /// `min_order_lots * lot_size` overflows `u64`.
    MinOrderSizeOverflow = 9,
    /// The smallest order at the lowest tick credits the seller zero quote.
    MinOrderBelowQuoteGranularity = 10,
    MaxOpenOrdersZero = 11,
    MaxOpenOrdersAboveLimit = 12,
    /// An update changed a field that is fixed at creation.
    ImmutableFieldChanged(ImmutableField) = 13,
}

/// Full configuration of one market. Pair identity (`base`, `quote`), the price
/// grid (`lot_size`, `tick_size`, `max_tick`), and the fee rates are fixed at
/// creation (spec section 3, `docs/clob-state-ownership.md` section 4). The
/// remaining fields may be changed by a permitted update.
#[derive(Clone)]
pub struct MarketConfig {
    pub base: Address,
    pub quote: Address,
    /// Base atoms per lot.
    pub lot_size: u64,
    /// Price increment, scaled by `PRICE_SCALE`.
    pub tick_size: u64,
    /// Largest valid tick index for this market.
    pub max_tick: u64,
    pub fees: MarketFeeConfig,
    /// Smallest order size, in lots. Being in lots keeps it on the lot grid.
    pub min_order_lots: u64,
    pub max_open_orders_per_trader: u32,
    pub status: MarketStatus,
}

impl MarketConfig {
    /// Checks every field. Market creation calls this directly and
    /// `validate_update` calls it on the proposed configuration, so there is
    /// one set of rules for both paths.
    pub fn validate(&self) -> Result<(), MarketConfigError> {
        if self.base == self.quote {
            return Err(MarketConfigError::IdenticalTokens);
        }
        if self.lot_size == 0 {
            return Err(MarketConfigError::LotSizeZero);
        }
        if self.tick_size == 0 {
            return Err(MarketConfigError::TickSizeZero);
        }
        if self.max_tick == 0 {
            return Err(MarketConfigError::MaxTickZero);
        }
        // Spec section 4.1 rejects an order whose scaled price overflows; a
        // market must not list ticks that could never be priced.
        self.max_tick
            .checked_mul(self.tick_size)
            .ok_or(MarketConfigError::TickRangeOverflow)?;
        if !fee_rate_in_range(self.fees.taker_fee_rate) {
            return Err(MarketConfigError::TakerFeeOutOfRange);
        }
        if !fee_rate_in_range(self.fees.maker_fee_rate) {
            return Err(MarketConfigError::MakerFeeOutOfRange);
        }
        self.validate_min_order()?;
        if self.max_open_orders_per_trader == 0 {
            return Err(MarketConfigError::MaxOpenOrdersZero);
        }
        if self.max_open_orders_per_trader > MAX_OPEN_ORDERS_PER_TRADER_LIMIT {
            return Err(MarketConfigError::MaxOpenOrdersAboveLimit);
        }
        Ok(())
    }

    /// Checks that `next` may replace `self`: no immutable field changes, and
    /// `next` passes the same validation as a newly created market.
    pub fn validate_update(&self, next: &MarketConfig) -> Result<(), MarketConfigError> {
        let changed = if next.base != self.base {
            Some(ImmutableField::Base)
        } else if next.quote != self.quote {
            Some(ImmutableField::Quote)
        } else if next.lot_size != self.lot_size {
            Some(ImmutableField::LotSize)
        } else if next.tick_size != self.tick_size {
            Some(ImmutableField::TickSize)
        } else if next.max_tick != self.max_tick {
            Some(ImmutableField::MaxTick)
        } else if next.fees.taker_fee_rate != self.fees.taker_fee_rate {
            Some(ImmutableField::TakerFeeRate)
        } else if next.fees.maker_fee_rate != self.fees.maker_fee_rate {
            Some(ImmutableField::MakerFeeRate)
        } else {
            None
        };
        if let Some(field) = changed {
            return Err(MarketConfigError::ImmutableFieldChanged(field));
        }
        next.validate()
    }

    fn validate_min_order(&self) -> Result<(), MarketConfigError> {
        if self.min_order_lots == 0 {
            return Err(MarketConfigError::MinOrderLotsZero);
        }
        let min_atoms = self
            .min_order_lots
            .checked_mul(self.lot_size)
            .ok_or(MarketConfigError::MinOrderSizeOverflow)?;
        // The cheapest fill is the minimum size at tick 1. If the seller's
        // rounded-down credit there is zero, the market accepts trades that
        // give away base for nothing. u64 * u64 can exceed i128, so the
        // product itself is checked too.
        let credit = quote_atoms(
            min_atoms as i128,
            self.tick_size as i128,
            RoundingMode::Down,
        )
        .map_err(|_| MarketConfigError::MinOrderSizeOverflow)?;
        if credit == 0 {
            return Err(MarketConfigError::MinOrderBelowQuoteGranularity);
        }
        Ok(())
    }
}

fn fee_rate_in_range(rate: i128) -> bool {
    (0..=MAX_FEE_RATE).contains(&rate)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{FEE_RATE_SCALE, PRICE_SCALE};
    use soroban_sdk::{testutils::Address as _, Env};

    fn valid(env: &Env) -> MarketConfig {
        MarketConfig {
            base: Address::generate(env),
            quote: Address::generate(env),
            lot_size: 100,
            tick_size: 100_000,
            max_tick: 1_000_000,
            fees: MarketFeeConfig {
                taker_fee_rate: 100_000,
                maker_fee_rate: 50_000,
            },
            // 10 lots * 100 atoms at tick 1 credits exactly 1 quote atom.
            min_order_lots: 10,
            max_open_orders_per_trader: 100,
            status: MarketStatus::Active,
        }
    }

    /// Deterministic xorshift64 so the property tests need no extra crate and
    /// any failure reproduces from its seed.
    struct Rng(u64);

    impl Rng {
        fn next(&mut self) -> u64 {
            let mut x = self.0;
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            self.0 = x;
            x
        }

        fn below(&mut self, n: u64) -> u64 {
            self.next() % n
        }
    }

    #[test]
    fn valid_config_is_accepted() {
        let env = Env::default();
        assert_eq!(valid(&env).validate(), Ok(()));
    }

    type Mutation = fn(&mut MarketConfig);

    #[test]
    fn every_invalid_field_has_its_own_error() {
        let env = Env::default();
        let base = valid(&env);
        let cases: [(Mutation, MarketConfigError); 12] = [
            (
                |c| c.quote = c.base.clone(),
                MarketConfigError::IdenticalTokens,
            ),
            (|c| c.lot_size = 0, MarketConfigError::LotSizeZero),
            (|c| c.tick_size = 0, MarketConfigError::TickSizeZero),
            (|c| c.max_tick = 0, MarketConfigError::MaxTickZero),
            (
                |c| c.max_tick = u64::MAX,
                MarketConfigError::TickRangeOverflow,
            ),
            (
                |c| c.fees.taker_fee_rate = -1,
                MarketConfigError::TakerFeeOutOfRange,
            ),
            (
                |c| c.fees.maker_fee_rate = MAX_FEE_RATE + 1,
                MarketConfigError::MakerFeeOutOfRange,
            ),
            (
                |c| c.min_order_lots = 0,
                MarketConfigError::MinOrderLotsZero,
            ),
            (
                |c| c.min_order_lots = u64::MAX,
                MarketConfigError::MinOrderSizeOverflow,
            ),
            (
                |c| c.min_order_lots = 1,
                MarketConfigError::MinOrderBelowQuoteGranularity,
            ),
            (
                |c| c.max_open_orders_per_trader = 0,
                MarketConfigError::MaxOpenOrdersZero,
            ),
            (
                |c| c.max_open_orders_per_trader = MAX_OPEN_ORDERS_PER_TRADER_LIMIT + 1,
                MarketConfigError::MaxOpenOrdersAboveLimit,
            ),
        ];
        for (mutate, expected) in cases {
            let mut config = base.clone();
            mutate(&mut config);
            assert_eq!(config.validate(), Err(expected));
        }
    }

    #[test]
    fn fee_bounds_are_inclusive() {
        let env = Env::default();
        let mut config = valid(&env);
        config.fees = MarketFeeConfig {
            taker_fee_rate: 0,
            maker_fee_rate: MAX_FEE_RATE,
        };
        assert_eq!(config.validate(), Ok(()));
        config.fees.taker_fee_rate = FEE_RATE_SCALE + 1;
        assert_eq!(
            config.validate(),
            Err(MarketConfigError::TakerFeeOutOfRange)
        );
    }

    #[test]
    fn min_order_granularity_boundary() {
        let env = Env::default();
        let mut config = valid(&env);
        config.lot_size = 1;
        config.tick_size = 1;
        // At one scaled-price unit, PRICE_SCALE base atoms earn one quote atom.
        config.min_order_lots = PRICE_SCALE as u64;
        assert_eq!(config.validate(), Ok(()));
        config.min_order_lots -= 1;
        assert_eq!(
            config.validate(),
            Err(MarketConfigError::MinOrderBelowQuoteGranularity)
        );
    }

    #[test]
    fn permitted_fields_can_be_updated() {
        let env = Env::default();
        let current = valid(&env);
        let mut next = current.clone();
        next.status = MarketStatus::Halted;
        next.min_order_lots = 5_000;
        next.max_open_orders_per_trader = MAX_OPEN_ORDERS_PER_TRADER_LIMIT;
        assert_eq!(current.validate_update(&next), Ok(()));
    }

    #[test]
    fn immutable_fields_cannot_be_updated() {
        let env = Env::default();
        let current = valid(&env);
        let other = Address::generate(&env);
        let cases: [(MarketConfig, ImmutableField); 7] = [
            (
                MarketConfig {
                    base: other.clone(),
                    ..current.clone()
                },
                ImmutableField::Base,
            ),
            (
                MarketConfig {
                    quote: other,
                    ..current.clone()
                },
                ImmutableField::Quote,
            ),
            (
                MarketConfig {
                    lot_size: 200,
                    ..current.clone()
                },
                ImmutableField::LotSize,
            ),
            (
                MarketConfig {
                    tick_size: 200_000,
                    ..current.clone()
                },
                ImmutableField::TickSize,
            ),
            (
                MarketConfig {
                    max_tick: 10,
                    ..current.clone()
                },
                ImmutableField::MaxTick,
            ),
            (
                MarketConfig {
                    fees: MarketFeeConfig {
                        taker_fee_rate: 0,
                        ..current.fees
                    },
                    ..current.clone()
                },
                ImmutableField::TakerFeeRate,
            ),
            (
                MarketConfig {
                    fees: MarketFeeConfig {
                        maker_fee_rate: 0,
                        ..current.fees
                    },
                    ..current.clone()
                },
                ImmutableField::MakerFeeRate,
            ),
        ];
        for (next, field) in cases {
            assert_eq!(
                current.validate_update(&next),
                Err(MarketConfigError::ImmutableFieldChanged(field))
            );
        }
    }

    #[test]
    fn update_rejects_what_creation_rejects() {
        let env = Env::default();
        let current = valid(&env);
        let mut next = current.clone();
        next.min_order_lots = 0;
        assert_eq!(
            current.validate_update(&next),
            Err(MarketConfigError::MinOrderLotsZero)
        );
        next.min_order_lots = current.min_order_lots;
        next.max_open_orders_per_trader = 0;
        assert_eq!(
            current.validate_update(&next),
            Err(MarketConfigError::MaxOpenOrdersZero)
        );
    }

    /// Applies one random invalidating change. Several are combined per case,
    /// so a config can be invalid in more than one way at once.
    fn break_one(rng: &mut Rng, config: &mut MarketConfig) {
        match rng.below(10) {
            0 => config.quote = config.base.clone(),
            1 => config.lot_size = 0,
            2 => config.tick_size = 0,
            3 => config.max_tick = 0,
            4 => {
                config.max_tick = u64::MAX - rng.below(1_000);
                config.tick_size = config.tick_size.max(2);
            }
            5 => config.fees.taker_fee_rate = MAX_FEE_RATE + 1 + rng.below(1 << 40) as i128,
            6 => config.fees.maker_fee_rate = -1 - rng.below(1 << 40) as i128,
            7 => config.min_order_lots = 0,
            8 => config.max_open_orders_per_trader = 0,
            _ => {
                config.max_open_orders_per_trader =
                    MAX_OPEN_ORDERS_PER_TRADER_LIMIT + 1 + rng.below(1 << 20) as u32
            }
        }
    }

    #[test]
    fn property_random_invalid_combinations_are_rejected() {
        let env = Env::default();
        let base = valid(&env);
        let mut rng = Rng(0x9E37_79B9_7F4A_7C15);
        for _ in 0..2_000 {
            let mut config = base.clone();
            for _ in 0..=rng.below(4) {
                break_one(&mut rng, &mut config);
            }
            assert!(config.validate().is_err());
            // The same invalid values must be rejected when proposed as an update.
            assert!(base.validate_update(&config).is_err());
        }
    }

    #[test]
    fn property_accepted_updates_always_pass_creation_validation() {
        let env = Env::default();
        let current = valid(&env);
        let other = Address::generate(&env);
        let statuses = [
            MarketStatus::Active,
            MarketStatus::CancelOnly,
            MarketStatus::Halted,
        ];
        let mut rng = Rng(0xD1B5_4A32_D192_ED03);
        let mut accepted = 0;
        for _ in 0..2_000 {
            let mut next = current.clone();
            next.status = statuses[rng.below(3) as usize];
            // Values span both sides of every bound.
            next.min_order_lots = match rng.below(3) {
                0 => rng.below(2_000),
                1 => rng.next(),
                _ => current.min_order_lots,
            };
            next.max_open_orders_per_trader =
                rng.below(MAX_OPEN_ORDERS_PER_TRADER_LIMIT as u64 * 2) as u32;
            if rng.below(4) == 0 {
                match rng.below(4) {
                    0 => next.base = other.clone(),
                    1 => next.lot_size = rng.next(),
                    2 => next.tick_size = rng.next(),
                    _ => next.fees.taker_fee_rate = rng.next() as i128,
                }
            }
            if current.validate_update(&next).is_ok() {
                accepted += 1;
                assert_eq!(next.validate(), Ok(()));
            }
        }
        // Guard against a generator that never produces an accepted update.
        assert!(accepted > 0);
    }
}
