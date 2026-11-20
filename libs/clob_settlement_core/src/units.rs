//! Typed integer units for the CLOB (#881).
//!
//! Every quantity in matching code is one of these newtypes, so base atoms,
//! quote atoms, lots, ticks and sequence numbers cannot be mixed up silently.
//! There are no `From` impls between units and no unchecked arithmetic. Unit
//! changes go through the explicit conversions in this module, and arithmetic
//! returns `Option`. Nothing here uses floating point.
//!
//! # Widths and overflow analysis
//!
//! - Token amounts ([`BaseAtoms`], [`QuoteAtoms`]) are `i128`, matching the
//!   Soroban token interface. Both are non-negative by construction.
//! - Lots, ticks and sequence numbers are `u64`, the width the read API already
//!   uses for price levels and order ids.
//! - Decimals are capped at [`MAX_TOKEN_DECIMALS`] = 18. Stellar Asset Contracts
//!   use 7, so 18 is a generous ceiling. `10^18 * 10^18 = 10^36` is below
//!   `i128::MAX` (about 1.7 * 10^38), so one product of two scaled values fits
//!   before any check is needed.
//! - A lot is `lot_size` base atoms with `lot_size >= 1`. With `lot_size = 1` a
//!   lot count can reach `i128::MAX`, which exceeds `u64::MAX`. The conversion
//!   from atoms to lots therefore returns `None` instead of truncating.
//! - Sequence numbers are `u64`. Exhausting them would take more than 500 years
//!   at one order per nanosecond, but [`SequenceNumber::next`] still checks.

/// Largest token decimal count a market may list. See the module overflow analysis.
pub const MAX_TOKEN_DECIMALS: u32 = 18;

/// Validates a token's decimal count against [`MAX_TOKEN_DECIMALS`].
pub fn validate_decimals(decimals: u32) -> Result<(), &'static str> {
    if decimals > MAX_TOKEN_DECIMALS {
        return Err("token decimals exceed MAX_TOKEN_DECIMALS");
    }
    Ok(())
}

/// Non-negative amount of base token, in its smallest unit.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd)]
pub struct BaseAtoms(i128);

impl BaseAtoms {
    pub const ZERO: Self = Self(0);

    /// Returns `None` for negative input.
    pub fn new(value: i128) -> Option<Self> {
        (value >= 0).then_some(Self(value))
    }

    pub const fn get(self) -> i128 {
        self.0
    }

    pub fn checked_add(self, rhs: Self) -> Option<Self> {
        self.0.checked_add(rhs.0).map(Self)
    }

    /// Returns `None` if the result would be negative.
    pub fn checked_sub(self, rhs: Self) -> Option<Self> {
        self.0.checked_sub(rhs.0).filter(|v| *v >= 0).map(Self)
    }
}

/// Non-negative amount of quote token, in its smallest unit.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd)]
pub struct QuoteAtoms(i128);

impl QuoteAtoms {
    pub const ZERO: Self = Self(0);

    /// Returns `None` for negative input.
    pub fn new(value: i128) -> Option<Self> {
        (value >= 0).then_some(Self(value))
    }

    pub const fn get(self) -> i128 {
        self.0
    }

    pub fn checked_add(self, rhs: Self) -> Option<Self> {
        self.0.checked_add(rhs.0).map(Self)
    }

    /// Returns `None` if the result would be negative.
    pub fn checked_sub(self, rhs: Self) -> Option<Self> {
        self.0.checked_sub(rhs.0).filter(|v| *v >= 0).map(Self)
    }
}

/// Count of base lots. One lot is `lot_size` base atoms.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd)]
pub struct BaseLots(u64);

impl BaseLots {
    pub const ZERO: Self = Self(0);
    pub const MAX: Self = Self(u64::MAX);

    pub const fn new(value: u64) -> Self {
        Self(value)
    }

    pub const fn get(self) -> u64 {
        self.0
    }

    pub fn checked_add(self, rhs: Self) -> Option<Self> {
        self.0.checked_add(rhs.0).map(Self)
    }

    pub fn checked_sub(self, rhs: Self) -> Option<Self> {
        self.0.checked_sub(rhs.0).map(Self)
    }

    /// Converts to base atoms. Returns `None` if `lot_size` is not positive or
    /// the product overflows.
    pub fn to_atoms(self, lot_size: BaseAtoms) -> Option<BaseAtoms> {
        if lot_size.get() <= 0 {
            return None;
        }
        i128::from(self.0)
            .checked_mul(lot_size.get())
            .map(BaseAtoms)
    }

    /// Floors `atoms` to whole lots. Returns the lot count and the dust left
    /// over, which is always less than `lot_size`.
    ///
    /// Rounds down. Returns `None` if `lot_size` is not positive or the lot
    /// count does not fit in `u64`.
    pub fn from_atoms_floor(atoms: BaseAtoms, lot_size: BaseAtoms) -> Option<(Self, BaseAtoms)> {
        if lot_size.get() <= 0 {
            return None;
        }
        let lots = u64::try_from(atoms.get() / lot_size.get()).ok()?;
        let dust = BaseAtoms(atoms.get() % lot_size.get());
        Some((Self(lots), dust))
    }

    /// Converts `atoms` to lots only when it is an exact multiple of `lot_size`.
    /// Returns `None` if there is any dust or the conversion is out of range.
    pub fn from_atoms_exact(atoms: BaseAtoms, lot_size: BaseAtoms) -> Option<Self> {
        match Self::from_atoms_floor(atoms, lot_size)? {
            (lots, dust) if dust == BaseAtoms::ZERO => Some(lots),
            _ => None,
        }
    }
}

/// Count of quote lots. One lot is `lot_size` quote atoms.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd)]
pub struct QuoteLots(u64);

impl QuoteLots {
    pub const ZERO: Self = Self(0);
    pub const MAX: Self = Self(u64::MAX);

    pub const fn new(value: u64) -> Self {
        Self(value)
    }

    pub const fn get(self) -> u64 {
        self.0
    }

    pub fn checked_add(self, rhs: Self) -> Option<Self> {
        self.0.checked_add(rhs.0).map(Self)
    }

    pub fn checked_sub(self, rhs: Self) -> Option<Self> {
        self.0.checked_sub(rhs.0).map(Self)
    }
}

/// Price level on the book. Price in quote terms is derived from the tick by
/// `conversion::scaled_price`.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd)]
pub struct Tick(u64);

impl Tick {
    pub const ZERO: Self = Self(0);
    pub const MAX: Self = Self(u64::MAX);

    pub const fn new(value: u64) -> Self {
        Self(value)
    }

    pub const fn get(self) -> u64 {
        self.0
    }
}

/// Per-market order sequence number. Assigned at acceptance and used as the
/// stable order identifier in events.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd)]
pub struct SequenceNumber(u64);

impl SequenceNumber {
    pub const MAX: Self = Self(u64::MAX);

    pub const fn new(value: u64) -> Self {
        Self(value)
    }

    pub const fn get(self) -> u64 {
        self.0
    }

    /// Returns `None` at `u64::MAX`.
    pub fn next(self) -> Option<Self> {
        self.0.checked_add(1).map(Self)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base_atoms_rejects_negative() {
        assert_eq!(BaseAtoms::new(-1), None);
        assert_eq!(BaseAtoms::new(0), Some(BaseAtoms::ZERO));
        assert_eq!(BaseAtoms::new(i128::MAX).map(BaseAtoms::get), Some(i128::MAX));
    }

    #[test]
    fn quote_atoms_rejects_negative() {
        assert_eq!(QuoteAtoms::new(-1), None);
        assert_eq!(QuoteAtoms::new(0), Some(QuoteAtoms::ZERO));
    }

    #[test]
    fn atoms_checked_sub_cannot_go_negative() {
        let five = BaseAtoms::new(5).unwrap();
        let six = BaseAtoms::new(6).unwrap();

        assert_eq!(five.checked_sub(six), None);
        assert_eq!(six.checked_sub(five), BaseAtoms::new(1));
    }

    #[test]
    fn atoms_checked_add_overflow_is_none() {
        let max = BaseAtoms::new(i128::MAX).unwrap();
        let one = BaseAtoms::new(1).unwrap();

        assert_eq!(max.checked_add(one), None);
    }

    #[test]
    fn lots_checked_add_at_max_is_none() {
        assert_eq!(BaseLots::MAX.checked_add(BaseLots::new(1)), None);
        assert_eq!(QuoteLots::MAX.checked_add(QuoteLots::new(1)), None);
    }

    #[test]
    fn lots_checked_sub_below_zero_is_none() {
        assert_eq!(BaseLots::ZERO.checked_sub(BaseLots::new(1)), None);
        assert_eq!(QuoteLots::ZERO.checked_sub(QuoteLots::new(1)), None);
    }

    #[test]
    fn floor_has_no_dust_at_exact_multiple() {
        let lot = BaseAtoms::new(10).unwrap();

        let (lots, dust) = BaseLots::from_atoms_floor(BaseAtoms::new(30).unwrap(), lot).unwrap();

        assert_eq!(lots, BaseLots::new(3));
        assert_eq!(dust, BaseAtoms::ZERO);
    }

    #[test]
    fn floor_one_unit_below_a_lot_is_all_dust() {
        let lot = BaseAtoms::new(10).unwrap();

        let (lots, dust) = BaseLots::from_atoms_floor(BaseAtoms::new(9).unwrap(), lot).unwrap();

        assert_eq!(lots, BaseLots::ZERO);
        assert_eq!(dust, BaseAtoms::new(9).unwrap());
    }

    #[test]
    fn floor_one_unit_above_a_lot_leaves_one_unit_dust() {
        let lot = BaseAtoms::new(10).unwrap();

        let (lots, dust) = BaseLots::from_atoms_floor(BaseAtoms::new(11).unwrap(), lot).unwrap();

        assert_eq!(lots, BaseLots::new(1));
        assert_eq!(dust, BaseAtoms::new(1).unwrap());
    }

    #[test]
    fn floor_rejects_non_positive_lot_size() {
        let amount = BaseAtoms::new(10).unwrap();

        assert_eq!(BaseLots::from_atoms_floor(amount, BaseAtoms::ZERO), None);
    }

    #[test]
    fn floor_at_u64_max_lots_fits() {
        let lot = BaseAtoms::new(1).unwrap();
        let atoms = BaseAtoms::new(u64::MAX as i128).unwrap();

        let (lots, dust) = BaseLots::from_atoms_floor(atoms, lot).unwrap();

        assert_eq!(lots, BaseLots::MAX);
        assert_eq!(dust, BaseAtoms::ZERO);
    }

    #[test]
    fn floor_one_lot_past_u64_max_is_rejected() {
        let lot = BaseAtoms::new(1).unwrap();
        let atoms = BaseAtoms::new(u64::MAX as i128 + 1).unwrap();

        assert_eq!(BaseLots::from_atoms_floor(atoms, lot), None);
    }

    #[test]
    fn floor_of_i128_max_with_unit_lot_is_rejected() {
        let lot = BaseAtoms::new(1).unwrap();
        let atoms = BaseAtoms::new(i128::MAX).unwrap();

        assert_eq!(BaseLots::from_atoms_floor(atoms, lot), None);
    }

    #[test]
    fn exact_accepts_only_multiples() {
        let lot = BaseAtoms::new(10).unwrap();

        assert_eq!(BaseLots::from_atoms_exact(BaseAtoms::new(9).unwrap(), lot), None);
        assert_eq!(BaseLots::from_atoms_exact(BaseAtoms::new(10).unwrap(), lot), Some(BaseLots::new(1)));
        assert_eq!(BaseLots::from_atoms_exact(BaseAtoms::new(11).unwrap(), lot), None);
    }

    #[test]
    fn lots_to_atoms_round_trips_exactly() {
        let lot = BaseAtoms::new(10).unwrap();

        let atoms = BaseLots::new(7).to_atoms(lot).unwrap();

        assert_eq!(atoms, BaseAtoms::new(70).unwrap());
        assert_eq!(BaseLots::from_atoms_exact(atoms, lot), Some(BaseLots::new(7)));
    }

    #[test]
    fn lots_to_atoms_overflow_is_none() {
        let lot = BaseAtoms::new(i128::MAX).unwrap();

        assert_eq!(BaseLots::MAX.to_atoms(lot), None);
    }

    #[test]
    fn lots_to_atoms_rejects_non_positive_lot_size() {
        assert_eq!(BaseLots::new(1).to_atoms(BaseAtoms::ZERO), None);
    }

    #[test]
    fn sequence_number_advances_and_stops_at_max() {
        assert_eq!(SequenceNumber::new(4).next(), Some(SequenceNumber::new(5)));
        assert_eq!(SequenceNumber::MAX.next(), None);
    }

    #[test]
    fn decimals_at_cap_accepted_above_rejected() {
        assert!(validate_decimals(MAX_TOKEN_DECIMALS).is_ok());
        assert!(validate_decimals(MAX_TOKEN_DECIMALS + 1).is_err());
    }
}
