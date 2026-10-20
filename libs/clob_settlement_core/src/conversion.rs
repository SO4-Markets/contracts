//! Checked tick and lot to quote conversions with documented rounding (#882).
//!
//! # Price model
//!
//! A market has one [`TickSize`]. A [`Tick`] `t` prices at `t * tick_size`
//! in quote atoms per base atom, scaled by [`PRICE_SCALE`] (10^8). That scaled
//! price is an exact integer. The only division in this module converts it
//! back to quote atoms:
//!
//! ```text
//! quote_atoms = base_atoms * (tick * tick_size) / PRICE_SCALE
//! ```
//!
//! # Rounding policy
//!
//! Every helper that divides takes a [`Rounding`]. There is no default. Choose
//! the direction by who is paying:
//!
//! - [`Rounding::Up`] when the trader pays quote in: buy-side payments and
//!   reserves. The trader never pays less than the exact amount.
//! - [`Rounding::Down`] when the protocol pays quote out: sell-side proceeds.
//!   The trader never receives more than the exact amount.
//!
//! The two directions differ by at most one quote atom. No path uses floating
//! point, and every operation is checked: any overflow returns `None`.

use crate::units::{BaseAtoms, BaseLots, QuoteAtoms, Tick};

/// Fixed-point scale of the price: a scaled price of `PRICE_SCALE` is one quote
/// atom per base atom.
pub const PRICE_SCALE: i128 = 100_000_000;

/// Scaled price increment per tick. Always positive.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TickSize(u64);

impl TickSize {
    /// Returns `None` for zero.
    pub fn new(value: u64) -> Option<Self> {
        (value > 0).then_some(Self(value))
    }

    pub const fn get(self) -> u64 {
        self.0
    }
}

/// Direction for the single division in a conversion.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Rounding {
    /// Round toward zero. Use for amounts the protocol pays out.
    Down,
    /// Round away from zero. Use for amounts the trader pays in.
    Up,
}

/// Scaled price of `tick`: `tick * tick_size`, exact.
///
/// No rounding. Returns `None` if the product exceeds `i128::MAX`, which can
/// happen only when both factors are near `u64::MAX`.
pub fn scaled_price(tick: Tick, tick_size: TickSize) -> Option<i128> {
    i128::from(tick.get()).checked_mul(i128::from(tick_size.get()))
}

/// Quote amount for `base` atoms at `scaled_price`.
///
/// Rounds per `rounding`. Returns `None` on overflow of the product or, for
/// [`Rounding::Up`], of the round-up adjustment.
pub fn quote_for_base_atoms(
    base: BaseAtoms,
    scaled_price: i128,
    rounding: Rounding,
) -> Option<QuoteAtoms> {
    let product = base.get().checked_mul(scaled_price)?;
    let quote = match rounding {
        Rounding::Down => product / PRICE_SCALE,
        Rounding::Up => product.checked_add(PRICE_SCALE - 1)? / PRICE_SCALE,
    };
    QuoteAtoms::new(quote)
}

/// Quote amount for `lots` lots at `tick`.
///
/// The lot to base-atom step is exact, since `lot_size` is an integer and the
/// product is checked. Rounding happens only at the price step, per `rounding`.
/// Returns `None` if `lot_size` is not positive or any step overflows.
pub fn quote_for_lots(
    lots: BaseLots,
    lot_size: BaseAtoms,
    tick: Tick,
    tick_size: TickSize,
    rounding: Rounding,
) -> Option<QuoteAtoms> {
    let base = lots.to_atoms(lot_size)?;
    let price = scaled_price(tick, tick_size)?;
    quote_for_base_atoms(base, price, rounding)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn atoms(value: i128) -> BaseAtoms {
        BaseAtoms::new(value).unwrap()
    }

    #[test]
    fn tick_size_rejects_zero() {
        assert_eq!(TickSize::new(0), None);
        assert_eq!(TickSize::new(1).map(TickSize::get), Some(1));
    }

    #[test]
    fn scaled_price_is_exact_product() {
        let price = scaled_price(Tick::new(7), TickSize::new(3).unwrap());

        assert_eq!(price, Some(21));
    }

    #[test]
    fn scaled_price_at_u64_max_times_one_fits() {
        let price = scaled_price(Tick::MAX, TickSize::new(1).unwrap());

        assert_eq!(price, Some(u64::MAX as i128));
    }

    #[test]
    fn scaled_price_both_factors_at_max_overflows() {
        // (2^64 - 1)^2 is about 3.4 * 10^38, above i128::MAX (about 1.7 * 10^38).
        let price = scaled_price(Tick::MAX, TickSize::new(u64::MAX).unwrap());

        assert_eq!(price, None);
    }

    #[test]
    fn exact_quote_has_no_rounding_difference() {
        // One quote atom per base atom: quote equals base in both directions.
        let price = PRICE_SCALE;

        assert_eq!(quote_for_base_atoms(atoms(100), price, Rounding::Down), QuoteAtoms::new(100));
        assert_eq!(quote_for_base_atoms(atoms(100), price, Rounding::Up), QuoteAtoms::new(100));
    }

    #[test]
    fn zero_base_quotes_zero_in_both_directions() {
        assert_eq!(quote_for_base_atoms(atoms(0), 12_345, Rounding::Down), Some(QuoteAtoms::ZERO));
        assert_eq!(quote_for_base_atoms(atoms(0), 12_345, Rounding::Up), Some(QuoteAtoms::ZERO));
    }

    #[test]
    fn one_unit_product_rounds_down_to_zero_and_up_to_one() {
        // product = 1, which is less than PRICE_SCALE.
        assert_eq!(quote_for_base_atoms(atoms(1), 1, Rounding::Down), Some(QuoteAtoms::ZERO));
        assert_eq!(quote_for_base_atoms(atoms(1), 1, Rounding::Up), QuoteAtoms::new(1));
    }

    #[test]
    fn one_unit_below_a_whole_quote_atom_boundary() {
        // product = PRICE_SCALE - 1 sits just under one quote atom.
        let base = atoms(PRICE_SCALE - 1);

        assert_eq!(quote_for_base_atoms(base, 1, Rounding::Down), Some(QuoteAtoms::ZERO));
        assert_eq!(quote_for_base_atoms(base, 1, Rounding::Up), QuoteAtoms::new(1));
    }

    #[test]
    fn one_unit_above_a_whole_quote_atom_boundary() {
        // product = PRICE_SCALE sits exactly on one quote atom: no rounding.
        let base = atoms(PRICE_SCALE);

        assert_eq!(quote_for_base_atoms(base, 1, Rounding::Down), QuoteAtoms::new(1));
        assert_eq!(quote_for_base_atoms(base, 1, Rounding::Up), QuoteAtoms::new(1));
    }

    #[test]
    fn rounding_directions_bracket_the_exact_value() {
        for base in 0..=40i128 {
            for price in 0..=40i128 {
                let down = quote_for_base_atoms(atoms(base), price, Rounding::Down)
                    .unwrap()
                    .get();
                let up = quote_for_base_atoms(atoms(base), price, Rounding::Up)
                    .unwrap()
                    .get();
                let product = base * price;

                assert!(down * PRICE_SCALE <= product, "down too high: {base} * {price}");
                assert!(product < (down + 1) * PRICE_SCALE, "down too low: {base} * {price}");
                assert!(up - down <= 1, "directions differ by more than one atom");
                assert!(up * PRICE_SCALE >= product, "up too low: {base} * {price}");
            }
        }
    }

    #[test]
    fn quote_for_lots_is_exact_when_tick_size_is_scale() {
        // 3 lots of 10 atoms = 30 base atoms, at tick 2 with tick_size 1e8 = 2 quote per base.
        let quote = quote_for_lots(
            BaseLots::new(3),
            atoms(10),
            Tick::new(2),
            TickSize::new(PRICE_SCALE as u64).unwrap(),
            Rounding::Down,
        );

        assert_eq!(quote, QuoteAtoms::new(60));
    }

    #[test]
    fn quote_for_lots_rounds_only_at_price_step() {
        // 3 lots of 10 atoms = 30 base atoms. Scaled price 2 * 5 = 10. Product 300 < 1e8.
        let lots = BaseLots::new(3);
        let lot = atoms(10);
        let tick = Tick::new(2);
        let tick_size = TickSize::new(5).unwrap();

        assert_eq!(quote_for_lots(lots, lot, tick, tick_size, Rounding::Down), Some(QuoteAtoms::ZERO));
        assert_eq!(quote_for_lots(lots, lot, tick, tick_size, Rounding::Up), QuoteAtoms::new(1));
    }

    #[test]
    fn quote_for_lots_rejects_non_positive_lot_size() {
        let quote = quote_for_lots(
            BaseLots::new(1),
            BaseAtoms::ZERO,
            Tick::new(1),
            TickSize::new(1).unwrap(),
            Rounding::Up,
        );

        assert_eq!(quote, None);
    }

    #[test]
    fn quote_overflow_is_none() {
        assert_eq!(quote_for_base_atoms(atoms(i128::MAX), 2, Rounding::Down), None);
        assert_eq!(quote_for_base_atoms(atoms(i128::MAX), 2, Rounding::Up), None);
    }

    #[test]
    fn round_up_adjustment_overflow_is_none() {
        // product = i128::MAX, so adding PRICE_SCALE - 1 overflows, but rounding down is fine.
        assert_eq!(quote_for_base_atoms(atoms(i128::MAX), 1, Rounding::Up), None);
        assert!(quote_for_base_atoms(atoms(i128::MAX), 1, Rounding::Down).is_some());
    }
}
