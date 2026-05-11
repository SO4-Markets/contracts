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
            q.saturating_mul(b).saturating_add(r.saturating_mul(b) / denominator)
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
    result.to_i128().unwrap_or(if a > 0 { i128::MAX } else { i128::MIN })
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
    let mut y = (x + 1) / 2;
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
    if value < 0 { value.saturating_neg() } else { value }
}

pub fn min(a: i128, b: i128) -> i128 {
    if a < b { a } else { b }
}

pub fn max(a: i128, b: i128) -> i128 {
    if a > b { a } else { b }
}

/// Clamp value to [0, ∞) — used for pool amounts that can't go negative.
pub fn bound_above_zero(value: i128) -> i128 {
    if value < 0 { 0 } else { value }
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
}
