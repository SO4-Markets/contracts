#![no_std]

use soroban_sdk::{Address, Bytes, BytesN, Env};

// ─── Internal key builder ─────────────────────────────────────────────────────
//
// Each component is length-prefixed (2-byte BE length + bytes) so that
// keys("A", "BC") ≠ keys("AB", "C").  Separating components prevents
// accidental collisions between different arities of the same tag.
//
// Address → strkey string bytes (G... or C...), deterministic & unique.
// bool    → 1 byte: 0x00 or 0x01
// u64     → 8 bytes big-endian
// i128    → 16 bytes big-endian

fn push_str(buf: &mut Bytes, env: &Env, s: &str) {
    let b = Bytes::from_slice(env, s.as_bytes());
    let len = b.len() as u16;
    buf.append(&Bytes::from_slice(env, &len.to_be_bytes()));
    buf.append(&b);
}

fn push_addr(buf: &mut Bytes, env: &Env, addr: &Address) {
    // Address::to_string() returns the strkey (G... or C...) as a soroban String.
    // From<soroban_sdk::String> for Bytes is implemented in soroban-sdk.
    let s: soroban_sdk::String = addr.to_string();
    let b: Bytes = s.into();
    let len = b.len() as u16;
    buf.append(&Bytes::from_slice(env, &len.to_be_bytes()));
    buf.append(&b);
}

fn push_bool(buf: &mut Bytes, env: &Env, v: bool) {
    buf.append(&Bytes::from_slice(env, &[if v { 1u8 } else { 0u8 }]));
}

fn push_u64(buf: &mut Bytes, env: &Env, v: u64) {
    buf.append(&Bytes::from_slice(env, &v.to_be_bytes()));
}

fn sha256(env: &Env, buf: &Bytes) -> BytesN<32> {
    env.crypto().sha256(buf).into()
}

// ─── Role keys ────────────────────────────────────────────────────────────────

pub mod roles {
    use super::*;

    pub fn role_admin(env: &Env) -> BytesN<32> {
        let mut b = Bytes::new(env);
        push_str(&mut b, env, "ROLE_ADMIN");
        sha256(env, &b)
    }

    pub fn controller(env: &Env) -> BytesN<32> {
        let mut b = Bytes::new(env);
        push_str(&mut b, env, "CONTROLLER");
        sha256(env, &b)
    }

    pub fn market_keeper(env: &Env) -> BytesN<32> {
        let mut b = Bytes::new(env);
        push_str(&mut b, env, "MARKET_KEEPER");
        sha256(env, &b)
    }

    pub fn order_keeper(env: &Env) -> BytesN<32> {
        let mut b = Bytes::new(env);
        push_str(&mut b, env, "ORDER_KEEPER");
        sha256(env, &b)
    }

    pub fn liquidation_keeper(env: &Env) -> BytesN<32> {
        let mut b = Bytes::new(env);
        push_str(&mut b, env, "LIQUIDATION_KEEPER");
        sha256(env, &b)
    }

    pub fn adl_keeper(env: &Env) -> BytesN<32> {
        let mut b = Bytes::new(env);
        push_str(&mut b, env, "ADL_KEEPER");
        sha256(env, &b)
    }

    pub fn fee_keeper(env: &Env) -> BytesN<32> {
        let mut b = Bytes::new(env);
        push_str(&mut b, env, "FEE_KEEPER");
        sha256(env, &b)
    }
}

// ─── Market keys ─────────────────────────────────────────────────────────────

/// sha256("MARKET" ‖ market_address)
pub fn market_key(env: &Env, market: &Address) -> BytesN<32> {
    let mut b = Bytes::new(env);
    push_str(&mut b, env, "MARKET");
    push_addr(&mut b, env, market);
    sha256(env, &b)
}

pub fn market_index_token_key(env: &Env, market: &Address) -> BytesN<32> {
    let mut b = Bytes::new(env);
    push_str(&mut b, env, "MARKET_INDEX_TOKEN");
    push_addr(&mut b, env, market);
    sha256(env, &b)
}

pub fn market_long_token_key(env: &Env, market: &Address) -> BytesN<32> {
    let mut b = Bytes::new(env);
    push_str(&mut b, env, "MARKET_LONG_TOKEN");
    push_addr(&mut b, env, market);
    sha256(env, &b)
}

pub fn market_short_token_key(env: &Env, market: &Address) -> BytesN<32> {
    let mut b = Bytes::new(env);
    push_str(&mut b, env, "MARKET_SHORT_TOKEN");
    push_addr(&mut b, env, market);
    sha256(env, &b)
}

/// sha256("POOL_AMOUNT" ‖ market ‖ token)
pub fn pool_amount_key(env: &Env, market: &Address, token: &Address) -> BytesN<32> {
    let mut b = Bytes::new(env);
    push_str(&mut b, env, "POOL_AMOUNT");
    push_addr(&mut b, env, market);
    push_addr(&mut b, env, token);
    sha256(env, &b)
}

/// sha256("SWAP_IMPACT_POOL_AMOUNT" ‖ market ‖ token)
pub fn swap_impact_pool_amount_key(env: &Env, market: &Address, token: &Address) -> BytesN<32> {
    let mut b = Bytes::new(env);
    push_str(&mut b, env, "SWAP_IMPACT_POOL_AMOUNT");
    push_addr(&mut b, env, market);
    push_addr(&mut b, env, token);
    sha256(env, &b)
}

/// sha256("POSITION_IMPACT_POOL_AMOUNT" ‖ market)
pub fn position_impact_pool_amount_key(env: &Env, market: &Address) -> BytesN<32> {
    let mut b = Bytes::new(env);
    push_str(&mut b, env, "POSITION_IMPACT_POOL_AMOUNT");
    push_addr(&mut b, env, market);
    sha256(env, &b)
}

/// sha256("OPEN_INTEREST" ‖ market ‖ collateral_token ‖ is_long)
pub fn open_interest_key(
    env: &Env,
    market: &Address,
    collateral_token: &Address,
    is_long: bool,
) -> BytesN<32> {
    let mut b = Bytes::new(env);
    push_str(&mut b, env, "OPEN_INTEREST");
    push_addr(&mut b, env, market);
    push_addr(&mut b, env, collateral_token);
    push_bool(&mut b, env, is_long);
    sha256(env, &b)
}

/// sha256("OPEN_INTEREST_IN_TOKENS" ‖ market ‖ collateral_token ‖ is_long)
pub fn open_interest_in_tokens_key(
    env: &Env,
    market: &Address,
    collateral_token: &Address,
    is_long: bool,
) -> BytesN<32> {
    let mut b = Bytes::new(env);
    push_str(&mut b, env, "OPEN_INTEREST_IN_TOKENS");
    push_addr(&mut b, env, market);
    push_addr(&mut b, env, collateral_token);
    push_bool(&mut b, env, is_long);
    sha256(env, &b)
}

/// sha256("COLLATERAL_SUM" ‖ market ‖ collateral_token ‖ is_long)
pub fn collateral_sum_key(
    env: &Env,
    market: &Address,
    collateral_token: &Address,
    is_long: bool,
) -> BytesN<32> {
    let mut b = Bytes::new(env);
    push_str(&mut b, env, "COLLATERAL_SUM");
    push_addr(&mut b, env, market);
    push_addr(&mut b, env, collateral_token);
    push_bool(&mut b, env, is_long);
    sha256(env, &b)
}

// ─── Position keys ────────────────────────────────────────────────────────────

/// sha256("POSITION" ‖ account ‖ market ‖ collateral_token ‖ is_long)
pub fn position_key(
    env: &Env,
    account: &Address,
    market: &Address,
    collateral_token: &Address,
    is_long: bool,
) -> BytesN<32> {
    let mut b = Bytes::new(env);
    push_str(&mut b, env, "POSITION");
    push_addr(&mut b, env, account);
    push_addr(&mut b, env, market);
    push_addr(&mut b, env, collateral_token);
    push_bool(&mut b, env, is_long);
    sha256(env, &b)
}

// ─── Order / deposit / withdrawal keys ───────────────────────────────────────

/// sha256("ORDER" ‖ nonce)
pub fn order_key(env: &Env, nonce: u64) -> BytesN<32> {
    let mut b = Bytes::new(env);
    push_str(&mut b, env, "ORDER");
    push_u64(&mut b, env, nonce);
    sha256(env, &b)
}

/// sha256("DEPOSIT" ‖ nonce)
pub fn deposit_key(env: &Env, nonce: u64) -> BytesN<32> {
    let mut b = Bytes::new(env);
    push_str(&mut b, env, "DEPOSIT");
    push_u64(&mut b, env, nonce);
    sha256(env, &b)
}

/// sha256("WITHDRAWAL" ‖ nonce)
pub fn withdrawal_key(env: &Env, nonce: u64) -> BytesN<32> {
    let mut b = Bytes::new(env);
    push_str(&mut b, env, "WITHDRAWAL");
    push_u64(&mut b, env, nonce);
    sha256(env, &b)
}

// ─── Borrowing keys ───────────────────────────────────────────────────────────

/// sha256("CUMULATIVE_BORROWING_FACTOR" ‖ market ‖ is_long)
pub fn cumulative_borrowing_factor_key(
    env: &Env,
    market: &Address,
    is_long: bool,
) -> BytesN<32> {
    let mut b = Bytes::new(env);
    push_str(&mut b, env, "CUMULATIVE_BORROWING_FACTOR");
    push_addr(&mut b, env, market);
    push_bool(&mut b, env, is_long);
    sha256(env, &b)
}

/// sha256("CUMULATIVE_BORROWING_FACTOR_UPDATED_AT" ‖ market ‖ is_long)
pub fn cumulative_borrowing_factor_updated_at_key(
    env: &Env,
    market: &Address,
    is_long: bool,
) -> BytesN<32> {
    let mut b = Bytes::new(env);
    push_str(&mut b, env, "CUMULATIVE_BORROWING_FACTOR_UPDATED_AT");
    push_addr(&mut b, env, market);
    push_bool(&mut b, env, is_long);
    sha256(env, &b)
}

// ─── Funding keys ────────────────────────────────────────────────────────────

/// sha256("FUNDING_AMOUNT_PER_SIZE" ‖ market ‖ collateral_token ‖ is_long)
pub fn funding_amount_per_size_key(
    env: &Env,
    market: &Address,
    collateral_token: &Address,
    is_long: bool,
) -> BytesN<32> {
    let mut b = Bytes::new(env);
    push_str(&mut b, env, "FUNDING_AMOUNT_PER_SIZE");
    push_addr(&mut b, env, market);
    push_addr(&mut b, env, collateral_token);
    push_bool(&mut b, env, is_long);
    sha256(env, &b)
}

/// sha256("CLAIMABLE_FUNDING_AMOUNT" ‖ market ‖ token ‖ account)
pub fn claimable_funding_amount_key(
    env: &Env,
    market: &Address,
    token: &Address,
    account: &Address,
) -> BytesN<32> {
    let mut b = Bytes::new(env);
    push_str(&mut b, env, "CLAIMABLE_FUNDING_AMOUNT");
    push_addr(&mut b, env, market);
    push_addr(&mut b, env, token);
    push_addr(&mut b, env, account);
    sha256(env, &b)
}

/// sha256("CLAIMABLE_FEE_AMOUNT" ‖ market ‖ token)
pub fn claimable_fee_amount_key(env: &Env, market: &Address, token: &Address) -> BytesN<32> {
    let mut b = Bytes::new(env);
    push_str(&mut b, env, "CLAIMABLE_FEE_AMOUNT");
    push_addr(&mut b, env, market);
    push_addr(&mut b, env, token);
    sha256(env, &b)
}

/// sha256("FUNDING_UPDATED_AT" ‖ market)
pub fn funding_updated_at_key(env: &Env, market: &Address) -> BytesN<32> {
    let mut b = Bytes::new(env);
    push_str(&mut b, env, "FUNDING_UPDATED_AT");
    push_addr(&mut b, env, market);
    sha256(env, &b)
}

/// sha256("SAVED_FUNDING_FACTOR_PER_SECOND" ‖ market)
pub fn saved_funding_factor_per_second_key(env: &Env, market: &Address) -> BytesN<32> {
    let mut b = Bytes::new(env);
    push_str(&mut b, env, "SAVED_FUNDING_FACTOR_PER_SECOND");
    push_addr(&mut b, env, market);
    sha256(env, &b)
}

/// sha256("FUNDING_INCREASE_FACTOR_PER_SECOND" ‖ market)
pub fn funding_increase_factor_per_second_key(env: &Env, market: &Address) -> BytesN<32> {
    let mut b = Bytes::new(env);
    push_str(&mut b, env, "FUNDING_INCREASE_FACTOR_PER_SECOND");
    push_addr(&mut b, env, market);
    sha256(env, &b)
}

/// sha256("FUNDING_DECREASE_FACTOR_PER_SECOND" ‖ market)
pub fn funding_decrease_factor_per_second_key(env: &Env, market: &Address) -> BytesN<32> {
    let mut b = Bytes::new(env);
    push_str(&mut b, env, "FUNDING_DECREASE_FACTOR_PER_SECOND");
    push_addr(&mut b, env, market);
    sha256(env, &b)
}

/// sha256("MIN_FUNDING_FACTOR_PER_SECOND" ‖ market)
pub fn min_funding_factor_per_second_key(env: &Env, market: &Address) -> BytesN<32> {
    let mut b = Bytes::new(env);
    push_str(&mut b, env, "MIN_FUNDING_FACTOR_PER_SECOND");
    push_addr(&mut b, env, market);
    sha256(env, &b)
}

/// sha256("MAX_FUNDING_FACTOR_PER_SECOND" ‖ market)
pub fn max_funding_factor_per_second_key(env: &Env, market: &Address) -> BytesN<32> {
    let mut b = Bytes::new(env);
    push_str(&mut b, env, "MAX_FUNDING_FACTOR_PER_SECOND");
    push_addr(&mut b, env, market);
    sha256(env, &b)
}

/// sha256("FUNDING_EXPONENT_FACTOR" ‖ market)
pub fn funding_exponent_factor_key(env: &Env, market: &Address) -> BytesN<32> {
    let mut b = Bytes::new(env);
    push_str(&mut b, env, "FUNDING_EXPONENT_FACTOR");
    push_addr(&mut b, env, market);
    sha256(env, &b)
}

// ─── List / set keys ──────────────────────────────────────────────────────────

pub fn market_list_key(env: &Env) -> BytesN<32> {
    let mut b = Bytes::new(env);
    push_str(&mut b, env, "MARKET_LIST");
    sha256(env, &b)
}

pub fn position_list_key(env: &Env) -> BytesN<32> {
    let mut b = Bytes::new(env);
    push_str(&mut b, env, "POSITION_LIST");
    sha256(env, &b)
}

pub fn account_position_list_key(env: &Env, account: &Address) -> BytesN<32> {
    let mut b = Bytes::new(env);
    push_str(&mut b, env, "ACCOUNT_POSITION_LIST");
    push_addr(&mut b, env, account);
    sha256(env, &b)
}

pub fn order_list_key(env: &Env) -> BytesN<32> {
    let mut b = Bytes::new(env);
    push_str(&mut b, env, "ORDER_LIST");
    sha256(env, &b)
}

pub fn account_order_list_key(env: &Env, account: &Address) -> BytesN<32> {
    let mut b = Bytes::new(env);
    push_str(&mut b, env, "ACCOUNT_ORDER_LIST");
    push_addr(&mut b, env, account);
    sha256(env, &b)
}

pub fn deposit_list_key(env: &Env) -> BytesN<32> {
    let mut b = Bytes::new(env);
    push_str(&mut b, env, "DEPOSIT_LIST");
    sha256(env, &b)
}

pub fn account_deposit_list_key(env: &Env, account: &Address) -> BytesN<32> {
    let mut b = Bytes::new(env);
    push_str(&mut b, env, "ACCOUNT_DEPOSIT_LIST");
    push_addr(&mut b, env, account);
    sha256(env, &b)
}

pub fn withdrawal_list_key(env: &Env) -> BytesN<32> {
    let mut b = Bytes::new(env);
    push_str(&mut b, env, "WITHDRAWAL_LIST");
    sha256(env, &b)
}

pub fn account_withdrawal_list_key(env: &Env, account: &Address) -> BytesN<32> {
    let mut b = Bytes::new(env);
    push_str(&mut b, env, "ACCOUNT_WITHDRAWAL_LIST");
    push_addr(&mut b, env, account);
    sha256(env, &b)
}

pub fn nonce_key(env: &Env) -> BytesN<32> {
    let mut b = Bytes::new(env);
    push_str(&mut b, env, "NONCE");
    sha256(env, &b)
}

// ─── Configuration parameter keys ────────────────────────────────────────────

pub fn max_pool_amount_key(env: &Env, market: &Address, token: &Address) -> BytesN<32> {
    let mut b = Bytes::new(env);
    push_str(&mut b, env, "MAX_POOL_AMOUNT");
    push_addr(&mut b, env, market);
    push_addr(&mut b, env, token);
    sha256(env, &b)
}

pub fn max_open_interest_key(env: &Env, market: &Address, is_long: bool) -> BytesN<32> {
    let mut b = Bytes::new(env);
    push_str(&mut b, env, "MAX_OPEN_INTEREST");
    push_addr(&mut b, env, market);
    push_bool(&mut b, env, is_long);
    sha256(env, &b)
}

pub fn min_collateral_factor_key(env: &Env, market: &Address) -> BytesN<32> {
    let mut b = Bytes::new(env);
    push_str(&mut b, env, "MIN_COLLATERAL_FACTOR");
    push_addr(&mut b, env, market);
    sha256(env, &b)
}

pub fn max_leverage_key(env: &Env, market: &Address) -> BytesN<32> {
    let mut b = Bytes::new(env);
    push_str(&mut b, env, "MAX_LEVERAGE");
    push_addr(&mut b, env, market);
    sha256(env, &b)
}

pub fn position_fee_factor_key(env: &Env, market: &Address, for_positive_impact: bool) -> BytesN<32> {
    let mut b = Bytes::new(env);
    push_str(&mut b, env, "POSITION_FEE_FACTOR");
    push_addr(&mut b, env, market);
    push_bool(&mut b, env, for_positive_impact);
    sha256(env, &b)
}

pub fn swap_fee_factor_key(env: &Env, market: &Address, for_positive_impact: bool) -> BytesN<32> {
    let mut b = Bytes::new(env);
    push_str(&mut b, env, "SWAP_FEE_FACTOR");
    push_addr(&mut b, env, market);
    push_bool(&mut b, env, for_positive_impact);
    sha256(env, &b)
}

pub fn borrowing_factor_key(env: &Env, market: &Address, is_long: bool) -> BytesN<32> {
    let mut b = Bytes::new(env);
    push_str(&mut b, env, "BORROWING_FACTOR");
    push_addr(&mut b, env, market);
    push_bool(&mut b, env, is_long);
    sha256(env, &b)
}

pub fn borrowing_exponent_factor_key(env: &Env, market: &Address, is_long: bool) -> BytesN<32> {
    let mut b = Bytes::new(env);
    push_str(&mut b, env, "BORROWING_EXPONENT_FACTOR");
    push_addr(&mut b, env, market);
    push_bool(&mut b, env, is_long);
    sha256(env, &b)
}

pub fn funding_factor_key(env: &Env, market: &Address) -> BytesN<32> {
    let mut b = Bytes::new(env);
    push_str(&mut b, env, "FUNDING_FACTOR");
    push_addr(&mut b, env, market);
    sha256(env, &b)
}

/// Positive price impact factor (trades that improve pool balance)
pub fn position_impact_factor_key(env: &Env, market: &Address, is_positive: bool) -> BytesN<32> {
    let mut b = Bytes::new(env);
    push_str(&mut b, env, "POSITION_IMPACT_FACTOR");
    push_addr(&mut b, env, market);
    push_bool(&mut b, env, is_positive);
    sha256(env, &b)
}

pub fn position_impact_exponent_factor_key(env: &Env, market: &Address) -> BytesN<32> {
    let mut b = Bytes::new(env);
    push_str(&mut b, env, "POSITION_IMPACT_EXPONENT_FACTOR");
    push_addr(&mut b, env, market);
    sha256(env, &b)
}

pub fn swap_impact_factor_key(env: &Env, market: &Address, is_positive: bool) -> BytesN<32> {
    let mut b = Bytes::new(env);
    push_str(&mut b, env, "SWAP_IMPACT_FACTOR");
    push_addr(&mut b, env, market);
    push_bool(&mut b, env, is_positive);
    sha256(env, &b)
}

pub fn swap_impact_exponent_factor_key(env: &Env, market: &Address) -> BytesN<32> {
    let mut b = Bytes::new(env);
    push_str(&mut b, env, "SWAP_IMPACT_EXPONENT_FACTOR");
    push_addr(&mut b, env, market);
    sha256(env, &b)
}

pub fn max_pnl_factor_key(env: &Env, pnl_factor_type: &BytesN<32>, market: &Address, is_long: bool) -> BytesN<32> {
    let mut b = Bytes::new(env);
    push_str(&mut b, env, "MAX_PNL_FACTOR");
    b.extend_from_array(&pnl_factor_type.to_array());
    push_addr(&mut b, env, market);
    push_bool(&mut b, env, is_long);
    sha256(env, &b)
}

pub fn min_market_tokens_for_first_deposit_key(env: &Env, market: &Address) -> BytesN<32> {
    let mut b = Bytes::new(env);
    push_str(&mut b, env, "MIN_MARKET_TOKENS_FOR_FIRST_DEPOSIT");
    push_addr(&mut b, env, market);
    sha256(env, &b)
}

/// Stable price for stablecoins (bypasses oracle spread)
pub fn stable_price_key(env: &Env, token: &Address) -> BytesN<32> {
    let mut b = Bytes::new(env);
    push_str(&mut b, env, "STABLE_PRICE");
    push_addr(&mut b, env, token);
    sha256(env, &b)
}

/// Token decimals stored per-token (used in USD conversion)
pub fn token_decimals_key(env: &Env, token: &Address) -> BytesN<32> {
    let mut b = Bytes::new(env);
    push_str(&mut b, env, "TOKEN_DECIMALS");
    push_addr(&mut b, env, token);
    sha256(env, &b)
}

/// Keeper public keys for ed25519 signature verification
pub fn keeper_public_key_prefix(env: &Env) -> BytesN<32> {
    let mut b = Bytes::new(env);
    push_str(&mut b, env, "KEEPER_PUBLIC_KEY");
    sha256(env, &b)
}

/// Market token wasm hash (for factory to deploy LP tokens)
pub fn market_token_wasm_hash_key(env: &Env) -> BytesN<32> {
    let mut b = Bytes::new(env);
    push_str(&mut b, env, "MARKET_TOKEN_WASM_HASH");
    sha256(env, &b)
}

// ─── PnL factor type constants ────────────────────────────────────────────────

/// sha256("MAX_PNL_FACTOR_FOR_TRADERS") — used in pool value calculation
pub fn max_pnl_factor_for_traders_key(env: &Env) -> BytesN<32> {
    let mut b = Bytes::new(env);
    push_str(&mut b, env, "MAX_PNL_FACTOR_FOR_TRADERS");
    sha256(env, &b)
}

/// sha256("MAX_PNL_FACTOR_FOR_DEPOSITS") — used during LP deposits
pub fn max_pnl_factor_for_deposits_key(env: &Env) -> BytesN<32> {
    let mut b = Bytes::new(env);
    push_str(&mut b, env, "MAX_PNL_FACTOR_FOR_DEPOSITS");
    sha256(env, &b)
}

/// sha256("MAX_PNL_FACTOR_FOR_WITHDRAWALS") — used during LP withdrawals
pub fn max_pnl_factor_for_withdrawals_key(env: &Env) -> BytesN<32> {
    let mut b = Bytes::new(env);
    push_str(&mut b, env, "MAX_PNL_FACTOR_FOR_WITHDRAWALS");
    sha256(env, &b)
}

// ─── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use soroban_sdk::{testutils::Address as _, Env};

    #[test]
    fn test_keys_are_distinct() {
        let env = Env::default();
        let market = Address::generate(&env);
        let token_a = Address::generate(&env);
        let token_b = Address::generate(&env);

        let k1 = pool_amount_key(&env, &market, &token_a);
        let k2 = pool_amount_key(&env, &market, &token_b);
        let k3 = pool_amount_key(&env, &token_a, &market); // swapped order
        let k4 = max_pool_amount_key(&env, &market, &token_a); // different tag

        assert_ne!(k1, k2, "different tokens must yield different keys");
        assert_ne!(k1, k3, "argument order must matter");
        assert_ne!(k1, k4, "different tags must yield different keys");
    }

    #[test]
    fn test_keys_are_deterministic() {
        let env = Env::default();
        let market = Address::generate(&env);
        let token = Address::generate(&env);

        let k1 = pool_amount_key(&env, &market, &token);
        let k2 = pool_amount_key(&env, &market, &token);
        assert_eq!(k1, k2, "same inputs must produce same key");
    }

    #[test]
    fn test_bool_direction_matters() {
        let env = Env::default();
        let market = Address::generate(&env);

        let long_key = cumulative_borrowing_factor_key(&env, &market, true);
        let short_key = cumulative_borrowing_factor_key(&env, &market, false);
        assert_ne!(long_key, short_key);
    }

    #[test]
    fn test_role_keys_distinct() {
        let env = Env::default();
        let admin = roles::role_admin(&env);
        let ctrl = roles::controller(&env);
        let mkeeper = roles::market_keeper(&env);
        assert_ne!(admin, ctrl);
        assert_ne!(ctrl, mkeeper);
    }
}
