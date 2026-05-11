#![no_std]

/// 30-decimal precision constant (1 USD)
pub const FLOAT_PRECISION: i128 = 1_000_000_000_000_000_000_000_000_000_000; // 10^30
pub const WEI_PRECISION: i128 = 1_000_000_000_000_000_000; // 10^18

/// Multiply then divide with rounding
/// TODO: implement precise mul_div
pub fn mul_div(a: i128, b: i128, denominator: i128) -> i128 {
    if denominator == 0 {
        return 0;
    }
    let result = a.checked_mul(b).unwrap_or(i128::MAX);
    result / denominator
}

/// Convert value to factor (returns as i128 in FLOAT_PRECISION units)
/// TODO: implement to_factor with precision handling
pub fn to_factor(value: i128, total: i128) -> i128 {
    if total == 0 {
        return 0;
    }
    mul_div(value, FLOAT_PRECISION, total)
}

/// Apply factor to value
/// TODO: implement apply_factor with precision handling
pub fn apply_factor(value: i128, factor: i128) -> i128 {
    mul_div(value, factor, FLOAT_PRECISION)
}

/// Power factor calculation (for price impact curve)
/// TODO: implement pow_factor with exponent support
pub fn pow_factor(value: i128, exponent: i128) -> i128 {
    value.saturating_pow(exponent as u32)
}

/// Safe multiplication with overflow checks
/// TODO: implement safe_mul_checked
pub fn safe_mul(a: i128, b: i128) -> Option<i128> {
    a.checked_mul(b)
}

/// Safe division with zero check
/// TODO: implement safe_div_checked
pub fn safe_div(a: i128, b: i128) -> Option<i128> {
    if b == 0 {
        None
    } else {
        Some(a / b)
    }
}

/// Absolute value with overflow handling
/// TODO: implement abs_safe
pub fn abs_safe(value: i128) -> i128 {
    if value < 0 {
        value.saturating_neg()
    } else {
        value
    }
}

/// Min of two values
pub fn min(a: i128, b: i128) -> i128 {
    if a < b { a } else { b }
}

/// Max of two values
pub fn max(a: i128, b: i128) -> i128 {
    if a > b { a } else { b }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_mul_div() {
        // TODO: implement tests
    }

    #[test]
    fn test_apply_factor() {
        // TODO: implement tests
    }
}
