//! Shared CLOB arithmetic rules — single authoritative source (#1180).
//!
//! Both `tests/clob_model.rs` and `tests/clob_properties.rs` previously
//! maintained independent copies of these rules, two of which were known to
//! disagree on fee scale and maker rounding. This crate is the single
//! canonical statement; both test files import it.
//!
//! When `libs/clob_settlement_core` gains a Cargo.toml and compiles, re-export
//! this crate from there so the on-chain logic shares the same arithmetic.
//!
//! # Published rules (all integer, no floating point)
//!
//! - `notional = price * qty`  (price in ticks, qty in lots), computed in `u128`.
//! - taker fee = `ceil(notional * taker_bps / 10_000)`
//! - maker fee = `ceil(notional * maker_bps / 10_000)`
//!
//! Both fees round UP. This matches `libs/clob_settlement_core`'s `fee_on_quote`
//! and `required_bid_reservation`'s worst-case lock (#1175). An earlier version
//! of this crate used floor for the maker fee; that version was wrong.
//! - A fill executes at the resting (maker) price. A buying taker escrows its
//!   limit price and is refunded `(limit_price - fill_price) * qty`.
//!
//! # Protocol maximums
//!
//! Enforced before any arithmetic so no checked-multiplication is needed at
//! call sites.
//!
//! - `MAX_PRICE`   = 10^12 ticks
//! - `MAX_QTY`     = `u64::MAX / 2` lots
//! - `MAX_FEE_BPS` = 10 000 basis points (= 100 %)

/// Basis-point denominator.
pub const BPS: u128 = 10_000;

/// Maximum valid price in ticks.
pub const MAX_PRICE: u64 = 1_000_000_000_000;

/// Maximum valid quantity in lots.
pub const MAX_QTY: u64 = u64::MAX / 2;

/// Maximum fee rate in basis points.
pub const MAX_FEE_BPS: u32 = 10_000;

/// Errors returned by arithmetic helpers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ArithError {
    /// `price` or `qty` exceeds the protocol maximum.
    OutOfRange,
    /// An intermediate multiplication overflowed `u128`.
    Overflow,
    /// Fee rate exceeds `MAX_FEE_BPS`.
    InvalidFeeRate,
}

/// `notional = price × qty`, checked against protocol maximums.
///
/// Returns `Err(OutOfRange)` when either argument exceeds its maximum.
/// Returns `Err(Overflow)` when the product overflows `u128` (unreachable
/// given the maximums, but enforced defensively).
pub fn notional(price: u64, qty: u64) -> Result<u128, ArithError> {
    if price > MAX_PRICE {
        return Err(ArithError::OutOfRange);
    }
    if qty > MAX_QTY {
        return Err(ArithError::OutOfRange);
    }
    (price as u128)
        .checked_mul(qty as u128)
        .ok_or(ArithError::Overflow)
}

/// Taker fee = `ceil(notional × bps / 10_000)`.
///
/// Returns `Err(InvalidFeeRate)` when `bps > MAX_FEE_BPS`.
/// Returns `Err(Overflow)` when `notional × bps` overflows `u128`.
pub fn taker_fee(n: u128, bps: u32) -> Result<u128, ArithError> {
    if bps > MAX_FEE_BPS {
        return Err(ArithError::InvalidFeeRate);
    }
    let p = n.checked_mul(bps as u128).ok_or(ArithError::Overflow)?;
    Ok(p / BPS + u128::from(p % BPS != 0))
}

/// Maker fee = `ceil(notional × bps / 10_000)`.
///
/// Both taker and maker fees round UP, matching `libs/clob_settlement_core`'s
/// `fee_on_quote` (#1175). Using floor here would cause a one-atom conservation
/// error per fill where the division is inexact.
///
/// Returns `Err(InvalidFeeRate)` when `bps > MAX_FEE_BPS`.
/// Returns `Err(Overflow)` when `notional × bps` overflows `u128`.
pub fn maker_fee(n: u128, bps: u32) -> Result<u128, ArithError> {
    if bps > MAX_FEE_BPS {
        return Err(ArithError::InvalidFeeRate);
    }
    let p = n.checked_mul(bps as u128).ok_or(ArithError::Overflow)?;
    Ok(p / BPS + u128::from(p % BPS != 0))
}

/// Escrow amount for a resting order.
///
/// - Buy  side: escrows `price × qty` quote tokens.
/// - Sell side: escrows `qty` base tokens (returned as a notional amount for
///   uniformity; callers may interpret the unit as base lots).
pub fn escrow(side_is_buy: bool, price: u64, qty: u64) -> Result<u128, ArithError> {
    if side_is_buy {
        notional(price, qty)
    } else {
        if qty > MAX_QTY {
            return Err(ArithError::OutOfRange);
        }
        Ok(qty as u128)
    }
}

/// Refund for a buying taker whose limit price exceeds the fill price.
///
/// `refund = (limit_price - fill_price) × qty`
///
/// Returns `Err(Overflow)` on `u128` overflow (unreachable with protocol
/// maximums, but checked defensively).
/// Returns `Ok(0)` when `fill_price >= limit_price` (no overpayment).
pub fn buyer_refund(limit_price: u64, fill_price: u64, qty: u64) -> Result<u128, ArithError> {
    if fill_price >= limit_price {
        return Ok(0);
    }
    let diff = (limit_price - fill_price) as u128;
    diff.checked_mul(qty as u128).ok_or(ArithError::Overflow)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn notional_basic() {
        assert_eq!(notional(100, 5).unwrap(), 500);
        assert_eq!(notional(0, 5).unwrap(), 0);
    }

    #[test]
    fn notional_rejects_out_of_range() {
        assert_eq!(notional(MAX_PRICE + 1, 1), Err(ArithError::OutOfRange));
        assert_eq!(notional(1, MAX_QTY + 1), Err(ArithError::OutOfRange));
    }

    #[test]
    fn taker_fee_is_ceil() {
        // 10_001 * 30 / 10_000 = 30.003 → ceil → 31
        let n = 10_001u128;
        assert_eq!(taker_fee(n, 30).unwrap(), 31);
    }

    #[test]
    fn maker_fee_is_ceil() {
        // 10_001 * 10 / 10_000 = 10.001 → ceil → 11  (#1175: maker rounds UP like taker)
        let n = 10_001u128;
        assert_eq!(maker_fee(n, 10).unwrap(), 11);
    }

    #[test]
    fn exact_bps_boundary_no_rounding() {
        // 10_000 * 30 / 10_000 = 30 exactly — both ceil and floor give 30
        assert_eq!(taker_fee(10_000, 30).unwrap(), 30);
        assert_eq!(maker_fee(10_000, 10).unwrap(), 10);
    }

    #[test]
    fn zero_fee_bps() {
        assert_eq!(taker_fee(999_999, 0).unwrap(), 0);
        assert_eq!(maker_fee(999_999, 0).unwrap(), 0);
    }

    #[test]
    fn invalid_fee_rate_rejected() {
        assert_eq!(taker_fee(100, MAX_FEE_BPS + 1), Err(ArithError::InvalidFeeRate));
        assert_eq!(maker_fee(100, MAX_FEE_BPS + 1), Err(ArithError::InvalidFeeRate));
    }

    #[test]
    fn buyer_refund_basic() {
        // limit 110, fill 100, qty 3 → refund = 30
        assert_eq!(buyer_refund(110, 100, 3).unwrap(), 30);
    }

    #[test]
    fn buyer_refund_no_overpayment() {
        assert_eq!(buyer_refund(100, 100, 5).unwrap(), 0);
        assert_eq!(buyer_refund(99, 100, 5).unwrap(), 0);
    }

    #[test]
    fn escrow_buy_equals_notional() {
        assert_eq!(escrow(true, 50, 4).unwrap(), notional(50, 4).unwrap());
    }

    #[test]
    fn escrow_sell_equals_qty() {
        assert_eq!(escrow(false, 50, 4).unwrap(), 4);
    }
}
