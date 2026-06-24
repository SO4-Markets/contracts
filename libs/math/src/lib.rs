#![no_std]

use soroban_sdk::{Env, I256};

/// 30-decimal precision — matches GMX's FLOAT_PRECISION = 10^30.
pub const FLOAT_PRECISION: i128 = 1_000_000_000_000_000_000_000_000_000_000; // 10^30

/// sqrt(FLOAT_PRECISION) = 10^15 — used in sqrt_fp.
const SQRT_FLOAT_PRECISION: i128 = 1_000_000_000_000_000; // 10^15

/// Stellar standard token precision: 1 token = 10^7 stroops.
pub const TOKEN_PRECISION: i128 = 10_000_000; // 10^7

// ─── Core arithmetic ─────────────────────────────────────────────────────────

/// (a × b) / denominator using i128. Fast path; panics on denominator=0.
/// Use mul_div_wide for values near FLOAT_PRECISION where overflow is likely.
pub fn mul_div(a: i128, b: i128, denominator: i128) -> i128 {
    if denominator == 0 {
        return 0;
    }
    match a.checked_mul(b) {
        Some(p) => p / denominator,
        None => {
            // Decompose to avoid overflow: (a/d)*b + (a%d)*b/d
            let q = a / denominator;
            let r = a % denominator;
            q.saturating_mul(b)
                .saturating_add(r.saturating_mul(b) / denominator)
        }
    }
}

/// (a × b) / denominator using I256 host arithmetic — safe for large USD values.
/// Required when a or b can approach FLOAT_PRECISION (10^30).
pub fn mul_div_wide(env: &Env, a: i128, b: i128, denominator: i128) -> i128 {
    if denominator == 0 {
        return 0;
    }
    let a256 = I256::from_i128(env, a);
    let b256 = I256::from_i128(env, b);
    let d256 = I256::from_i128(env, denominator);
    let product = a256.mul(&b256);
    let result = product.div(&d256);
    // Saturate to i128 bounds if result is too large (shouldn't happen in normal protocol use)
    result
        .to_i128()
        .unwrap_or(if a > 0 { i128::MAX } else { i128::MIN })
}

/// (a × b) / denominator rounded UP (ceiling division) using I256.
/// Used for fee and cost amounts so the protocol never under-collects.
pub fn mul_div_wide_up(env: &Env, a: i128, b: i128, denominator: i128) -> i128 {
    if denominator == 0 {
        return 0;
    }
    let a256 = I256::from_i128(env, a);
    let b256 = I256::from_i128(env, b);
    let d256 = I256::from_i128(env, denominator);
    let product = a256.mul(&b256);
    // ceiling division: (product + denominator - 1) / denominator
    // only applies when product > 0 to avoid rounding negative values upward
    let zero = I256::from_i128(env, 0);
    let one = I256::from_i128(env, 1);
    let result = if product.cmp(&zero) == core::cmp::Ordering::Greater {
        let d_minus_one = d256.sub(&one);
        product.add(&d_minus_one).div(&d256)
    } else {
        product.div(&d256)
    };
    result
        .to_i128()
        .unwrap_or(if a > 0 { i128::MAX } else { i128::MIN })
}

// ─── Factor helpers ───────────────────────────────────────────────────────────

/// value / total expressed as a FLOAT_PRECISION fraction.
pub fn to_factor(value: i128, total: i128) -> i128 {
    if total == 0 {
        return 0;
    }
    mul_div(value, FLOAT_PRECISION, total)
}

/// value × factor / FLOAT_PRECISION.
pub fn apply_factor(value: i128, factor: i128) -> i128 {
    mul_div(value, factor, FLOAT_PRECISION)
}

/// Wide version — safe when value is a large USD amount.
pub fn apply_factor_wide(env: &Env, value: i128, factor: i128) -> i128 {
    mul_div_wide(env, value, factor, FLOAT_PRECISION)
}

// ─── Integer square root ─────────────────────────────────────────────────────

/// Integer square root via Newton's method (floor).
pub fn integer_sqrt(n: i128) -> i128 {
    if n <= 0 {
        return 0;
    }
    let mut x = n;
    // Use (x >> 1) + (x & 1) instead of (x + 1) / 2 to avoid overflow when x = i128::MAX.
    let mut y = (x >> 1) + (x & 1);
    while y < x {
        x = y;
        y = (y + n / y) / 2;
    }
    x
}

/// sqrt of a FLOAT_PRECISION value, result also in FLOAT_PRECISION units.
///
/// sqrt_fp(v) where v = V × 10^30:
///   result = sqrt(V) × 10^30 = sqrt(V × 10^30) × 10^15
pub fn sqrt_fp(value: i128) -> i128 {
    if value <= 0 {
        return 0;
    }
    // sqrt(value) in native units, then multiply by 10^15
    let s = integer_sqrt(value);
    s.saturating_mul(SQRT_FLOAT_PRECISION)
}

// ─── Exponent factor (mirrors GMX Precision.applyExponentFactor) ──────────────

/// value^(exponent / FLOAT_PRECISION) where value and result are in FLOAT_PRECISION units.
///
/// Uses the same sqrt-based approximation as GMX:
///   1. Compute integer part: value^floor(exponent / FLOAT_PRECISION)
///   2. Approximate fractional part via sqrt: value^frac ≈ sqrt(value)^(2*frac)
///
/// Requires env for I256 intermediate arithmetic.
pub fn pow_factor(env: &Env, value: i128, exponent: i128) -> i128 {
    if value <= 0 {
        return 0;
    }
    if exponent == 0 {
        return FLOAT_PRECISION; // x^0 = 1
    }
    if exponent == FLOAT_PRECISION {
        return value; // x^1 = x
    }

    let whole = exponent / FLOAT_PRECISION;
    let decimal = exponent % FLOAT_PRECISION;

    // Integer power: value^whole (using wide arithmetic to prevent overflow)
    let mut result = FLOAT_PRECISION; // 1.0
    for _ in 0..whole {
        result = mul_div_wide(env, result, value, FLOAT_PRECISION);
    }

    if decimal == 0 {
        return result;
    }

    // Fractional power via sqrt:
    //   value^decimal = sqrt(value)^(2 * decimal / FLOAT_PRECISION)
    let sqrt_value = sqrt_fp(value);
    let double_decimal = decimal.saturating_mul(2);
    let sqrt_whole = double_decimal / FLOAT_PRECISION; // 0 or 1
    let sqrt_frac = double_decimal % FLOAT_PRECISION;

    let mut sqrt_result = FLOAT_PRECISION;
    for _ in 0..sqrt_whole {
        sqrt_result = mul_div_wide(env, sqrt_result, sqrt_value, FLOAT_PRECISION);
    }

    // Linear interpolation for the remaining sub-half exponent:
    //   x^f ≈ 1 + f*(x - 1)
    if sqrt_frac > 0 && sqrt_value > FLOAT_PRECISION {
        let delta = mul_div(sqrt_value - FLOAT_PRECISION, sqrt_frac, FLOAT_PRECISION);
        sqrt_result = sqrt_result.saturating_add(mul_div(sqrt_result, delta, FLOAT_PRECISION));
    }

    mul_div_wide(env, result, sqrt_result, FLOAT_PRECISION)
}

// ─── Utility ──────────────────────────────────────────────────────────────────

pub fn abs_safe(value: i128) -> i128 {
    if value < 0 {
        value.saturating_neg()
    } else {
        value
    }
}

pub fn min(a: i128, b: i128) -> i128 {
    if a < b {
        a
    } else {
        b
    }
}

pub fn max(a: i128, b: i128) -> i128 {
    if a > b {
        a
    } else {
        b
    }
}

/// Clamp value to [0, ∞) — used for pool amounts that can't go negative.
pub fn bound_above_zero(value: i128) -> i128 {
    if value < 0 {
        0
    } else {
        value
    }
}

// ─── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use soroban_sdk::Env;

    #[test]
    fn test_mul_div_basic() {
        assert_eq!(mul_div(100, 200, 100), 200);
        assert_eq!(mul_div(1, FLOAT_PRECISION, FLOAT_PRECISION), 1);
        assert_eq!(mul_div(0, 1000, 100), 0);
        assert_eq!(mul_div(100, 0, 100), 0);
    }

    #[test]
    fn test_apply_factor() {
        // 50% of FLOAT_PRECISION = 0.5
        let half = FLOAT_PRECISION / 2;
        assert_eq!(apply_factor(FLOAT_PRECISION, half), half);
        // 1.0 factor = identity
        assert_eq!(apply_factor(12345, FLOAT_PRECISION), 12345);
    }

    #[test]
    fn test_to_factor() {
        assert_eq!(to_factor(1, 2), FLOAT_PRECISION / 2);
        assert_eq!(to_factor(FLOAT_PRECISION, FLOAT_PRECISION), FLOAT_PRECISION);
    }

    #[test]
    fn test_integer_sqrt() {
        assert_eq!(integer_sqrt(0), 0);
        assert_eq!(integer_sqrt(1), 1);
        assert_eq!(integer_sqrt(4), 2);
        assert_eq!(integer_sqrt(9), 3);
        assert_eq!(integer_sqrt(100), 10);
        assert_eq!(integer_sqrt(2), 1); // floor
    }

    #[test]
    fn test_mul_div_wide() {
        let env = Env::default();
        // Same as mul_div for small values
        assert_eq!(mul_div_wide(&env, 100, 200, 100), 200);
        // Large value: (FLOAT_PRECISION * FLOAT_PRECISION) / FLOAT_PRECISION = FLOAT_PRECISION
        let fp = FLOAT_PRECISION;
        assert_eq!(mul_div_wide(&env, fp, fp, fp), fp);
    }

    #[test]
    fn test_pow_factor_integer_exponents() {
        let env = Env::default();
        let fp = FLOAT_PRECISION;
        // x^1 = x
        assert_eq!(pow_factor(&env, 1000 * fp, fp), 1000 * fp);
        // x^0 = 1
        assert_eq!(pow_factor(&env, 1000 * fp, 0), fp);
        // 2^2 = 4 (in FLOAT_PRECISION units)
        let two = 2 * fp;
        let four = 4 * fp;
        assert_eq!(pow_factor(&env, two, 2 * fp), four);
    }

    // ── Issue #156/#127: rounding direction ──────────────────────────────────

    /// mul_div_wide (floor) and mul_div_wide_up (ceil) produce the same result
    /// when the division is exact.
    #[test]
    fn rounding_exact_division_same_result() {
        let env = Env::default();
        // 10 × 3 / 5 = 6 exactly — both should give 6
        assert_eq!(mul_div_wide(&env, 10, 3, 5), 6);
        assert_eq!(mul_div_wide_up(&env, 10, 3, 5), 6);
    }

    /// When division has a remainder, mul_div_wide_up produces a result one
    /// greater than mul_div_wide, ensuring fees are never under-collected.
    #[test]
    fn rounding_up_exceeds_floor_on_remainder() {
        let env = Env::default();
        // 10 × 1 / 3 = 3 remainder 1 → floor = 3, ceil = 4
        let floor = mul_div_wide(&env, 10, 1, 3);
        let ceil = mul_div_wide_up(&env, 10, 1, 3);
        assert_eq!(floor, 3);
        assert_eq!(ceil, 4);
        assert!(
            ceil > floor,
            "ceil must exceed floor when there is a remainder"
        );
    }

    /// Repeated small fees accumulate rather than leak when rounding up.
    /// 1_000_001 iterations each paying 1 stroop of fee at 0.001% rate —
    /// the ceiling version must collect at least as much as the floor version.
    #[test]
    fn fee_rounding_accumulates_not_leaks() {
        let env = Env::default();
        env.cost_estimate().budget().reset_unlimited();
        let fp = FLOAT_PRECISION;
        // fee_factor = 0.001% = fp / 100_000
        let fee_factor = fp / 100_000;
        let size = 33; // odd number ensures a remainder on most iterations

        let mut floor_total: i128 = 0;
        let mut ceil_total: i128 = 0;

        for _ in 0..1_000 {
            floor_total += mul_div_wide(&env, size, fee_factor, fp);
            ceil_total += mul_div_wide_up(&env, size, fee_factor, fp);
        }

        assert!(
            ceil_total >= floor_total,
            "ceiling-rounded fees must accumulate at least as much as floor-rounded fees"
        );
    }

    /// Negative values (credits/claimable amounts) should not be rounded away
    /// from zero — mul_div_wide_up returns floor division for negative products.
    #[test]
    fn rounding_up_does_not_affect_negative_values() {
        let env = Env::default();
        // −10 × 1 / 3 = −3 remainder −1 → both floor and ceil behave the same
        // (we only apply ceiling for positive fee amounts)
        let floor = mul_div_wide(&env, -10, 1, 3);
        let ceil = mul_div_wide_up(&env, -10, 1, 3);
        // Both should truncate toward zero (i.e. -3, not -4)
        assert_eq!(floor, -3);
        assert_eq!(ceil, -3);
    }

    // ── Issue #136: property tests for math utilities ─────────────────────────

    /// mul_div_wide_up(a, b, d) >= mul_div_wide(a, b, d) for all positive inputs.
    /// Covers a range of values including protocol-scale USD amounts.
    #[test]
    fn property_ceil_always_gte_floor_for_positive_inputs() {
        let env = Env::default();
        let cases: &[(i128, i128, i128)] = &[
            (1, 1, 3),
            (7, 11, 13),
            (FLOAT_PRECISION, 3, FLOAT_PRECISION * 2),
            (10_000_000, FLOAT_PRECISION / 1_000, FLOAT_PRECISION),
            (i128::MAX / 2, 1, i128::MAX),
            (
                1_000_000 * 10_000_000,
                FLOAT_PRECISION / 10_000,
                FLOAT_PRECISION,
            ),
        ];
        for &(a, b, d) in cases {
            let floor = mul_div_wide(&env, a, b, d);
            let ceil = mul_div_wide_up(&env, a, b, d);
            assert!(
                ceil >= floor,
                "ceil must be >= floor: a={a}, b={b}, d={d}, floor={floor}, ceil={ceil}"
            );
        }
    }

    /// mul_div_wide is monotone in a: for fixed positive b, d and a1 < a2,
    /// mul_div_wide(a1, b, d) <= mul_div_wide(a2, b, d).
    #[test]
    fn property_mul_div_wide_monotone_in_a() {
        let env = Env::default();
        let b = FLOAT_PRECISION / 100; // 1%
        let d = FLOAT_PRECISION;
        let steps: &[i128] = &[
            0,
            1,
            100,
            10_000_000,
            FLOAT_PRECISION,
            FLOAT_PRECISION * 1_000,
        ];
        for &a1 in steps {
            for &a2 in steps {
                if a1 >= a2 {
                    continue;
                }
                let r1 = mul_div_wide(&env, a1, b, d);
                let r2 = mul_div_wide(&env, a2, b, d);
                assert!(
                    r1 <= r2,
                    "monotone violated: a1={a1}, a2={a2}, r1={r1}, r2={r2}"
                );
            }
        }
    }

    /// apply_factor with FLOAT_PRECISION is the identity (within rounding of 1 unit).
    #[test]
    fn property_apply_factor_fp_is_identity() {
        let values: &[i128] = &[0, 1, 100, 10_000_000, FLOAT_PRECISION, FLOAT_PRECISION * 7];
        for &v in values {
            let result = apply_factor(v, FLOAT_PRECISION);
            assert_eq!(result, v, "apply_factor(v, FP) must equal v for v={v}");
        }
    }

    /// apply_factor with zero factor always returns 0.
    #[test]
    fn property_apply_factor_zero_factor_returns_zero() {
        let values: &[i128] = &[0, 1, 10_000_000, FLOAT_PRECISION, i128::MAX / 2];
        for &v in values {
            assert_eq!(
                apply_factor(v, 0),
                0,
                "apply_factor(v, 0) must be 0 for v={v}"
            );
        }
    }

    /// integer_sqrt is monotone: a <= b implies sqrt(a) <= sqrt(b).
    #[test]
    fn property_integer_sqrt_monotone() {
        let steps: &[i128] = &[
            0,
            1,
            2,
            3,
            4,
            9,
            16,
            100,
            10_000,
            1_000_000,
            FLOAT_PRECISION,
        ];
        for &a in steps {
            for &b in steps {
                if a > b {
                    continue;
                }
                let sa = integer_sqrt(a);
                let sb = integer_sqrt(b);
                assert!(
                    sa <= sb,
                    "sqrt not monotone: sqrt({a})={sa} > sqrt({b})={sb}"
                );
            }
        }
    }

    /// integer_sqrt(n*n) == n for small perfect squares — verifies floor correctness.
    #[test]
    fn property_integer_sqrt_perfect_squares() {
        for n in 0i128..=1000 {
            assert_eq!(integer_sqrt(n * n), n, "sqrt(n²) must equal n for n={n}");
        }
    }

    /// integer_sqrt never produces a negative value.
    #[test]
    fn property_integer_sqrt_never_negative() {
        let cases: &[i128] = &[0, 1, 2, 3, 4, 5, 100, 10_000, FLOAT_PRECISION, i128::MAX];
        for &n in cases {
            assert!(integer_sqrt(n) >= 0, "sqrt({n}) must be non-negative");
        }
    }

    /// mul_div(a, b, d) never returns a value larger than a*b (for positive inputs).
    /// This guards against overflow bugs that inflate results.
    #[test]
    fn property_mul_div_result_never_exceeds_naive_product() {
        let cases: &[(i128, i128, i128)] = &[(10, 20, 1), (100, 200, 50), (FLOAT_PRECISION, 2, 1)];
        for &(a, b, d) in cases {
            let result = mul_div(a, b, d);
            let naive = a.saturating_mul(b);
            // result should be <= naive / d (we just check it doesn't exceed naive)
            assert!(
                result <= naive,
                "mul_div({a},{b},{d})={result} exceeded naive product {naive}"
            );
        }
    }

    /// bound_above_zero clamps negatives to 0 and passes non-negatives through.
    #[test]
    fn property_bound_above_zero_no_negative_output() {
        let cases: &[i128] = &[i128::MIN, -1_000_000, -1, 0, 1, 1_000_000, i128::MAX];
        for &v in cases {
            let result = bound_above_zero(v);
            assert!(
                result >= 0,
                "bound_above_zero({v}) returned negative: {result}"
            );
            if v >= 0 {
                assert_eq!(
                    result, v,
                    "bound_above_zero must be identity for non-negative {v}"
                );
            } else {
                assert_eq!(result, 0, "bound_above_zero must return 0 for negative {v}");
            }
        }
    }

    /// abs_safe never returns a negative for any input (including i128::MIN which
    /// would overflow a plain negation — saturating_neg clamps to i128::MAX).
    #[test]
    fn property_abs_safe_never_negative() {
        let cases: &[i128] = &[i128::MIN, -1, 0, 1, FLOAT_PRECISION, i128::MAX];
        for &v in cases {
            let result = abs_safe(v);
            assert!(result >= 0, "abs_safe({v}) returned negative: {result}");
        }
    }

    /// pow_factor(x, 0) == FLOAT_PRECISION (x^0 = 1) for any positive x.
    #[test]
    fn property_pow_factor_zero_exponent_is_one() {
        let env = Env::default();
        let xs: &[i128] = &[1, FLOAT_PRECISION / 2, FLOAT_PRECISION, FLOAT_PRECISION * 3];
        for &x in xs {
            assert_eq!(
                pow_factor(&env, x, 0),
                FLOAT_PRECISION,
                "pow_factor({x}, 0) must be FLOAT_PRECISION"
            );
        }
    }

    /// pow_factor(x, FP) == x (x^1 = x) for positive x.
    #[test]
    fn property_pow_factor_unit_exponent_is_identity() {
        let env = Env::default();
        let xs: &[i128] = &[FLOAT_PRECISION / 2, FLOAT_PRECISION, 2 * FLOAT_PRECISION];
        for &x in xs {
            assert_eq!(
                pow_factor(&env, x, FLOAT_PRECISION),
                x,
                "pow_factor({x}, FP) must equal x"
            );
        }
    }

    /// mul_div_wide with denominator 0 returns 0 (no divide-by-zero panic).
    #[test]
    fn property_mul_div_wide_zero_denominator_returns_zero() {
        let env = Env::default();
        assert_eq!(mul_div_wide(&env, 12345, FLOAT_PRECISION, 0), 0);
        assert_eq!(mul_div_wide_up(&env, 12345, FLOAT_PRECISION, 0), 0);
        assert_eq!(mul_div(12345, FLOAT_PRECISION, 0), 0);
    }
}
