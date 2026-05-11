#![no_std]

use soroban_sdk::{BytesN, Env, Address};

/// Hash a string tag to create a base key
/// TODO: implement key generation with sha256
pub fn hash_key(env: &Env, tag: &str) -> BytesN<32> {
    let data = tag.as_bytes();
    let hash = env.crypto().sha256(&soroban_sdk::Bytes::from_slice(env, data));
    BytesN::from_array(env, &hash.into())
}

/// Market key: MARKET_{market_address}
/// TODO: implement market_key
pub fn market_key(env: &Env, _market: &Address) -> BytesN<32> {
    // TODO: derive from market address
    BytesN::from_array(env, &[0u8; 32])
}

/// Pool amount key: POOL_AMOUNT_{market}_{token}
/// TODO: implement pool_amount_key
pub fn pool_amount_key(env: &Env, _market: &Address, _token: &Address) -> BytesN<32> {
    // TODO: derive from market and token
    BytesN::from_array(env, &[0u8; 32])
}

/// Open interest key: OPEN_INTEREST_{market}_{collateral}_{is_long}
/// TODO: implement open_interest_key
pub fn open_interest_key(env: &Env, _market: &Address, _collateral: &Address, _is_long: bool) -> BytesN<32> {
    // TODO: derive from market, collateral, and direction
    BytesN::from_array(env, &[0u8; 32])
}

/// Position key: POSITION_{account}_{market}_{collateral}_{is_long}
/// TODO: implement position_key
pub fn position_key(env: &Env, _account: &Address, _market: &Address, _collateral: &Address, _is_long: bool) -> BytesN<32> {
    // TODO: derive from account, market, collateral, and direction
    BytesN::from_array(env, &[0u8; 32])
}

/// Order key: ORDER_{nonce}
/// TODO: implement order_key
pub fn order_key(env: &Env, _nonce: u64) -> BytesN<32> {
    // TODO: derive from nonce
    BytesN::from_array(env, &[0u8; 32])
}

/// Deposit key: DEPOSIT_{nonce}
/// TODO: implement deposit_key
pub fn deposit_key(env: &Env, _nonce: u64) -> BytesN<32> {
    // TODO: derive from nonce
    BytesN::from_array(env, &[0u8; 32])
}

/// Withdrawal key: WITHDRAWAL_{nonce}
/// TODO: implement withdrawal_key
pub fn withdrawal_key(env: &Env, _nonce: u64) -> BytesN<32> {
    // TODO: derive from nonce
    BytesN::from_array(env, &[0u8; 32])
}

/// Cumulative borrowing factor key
/// TODO: implement cumulative_borrowing_factor_key
pub fn cumulative_borrowing_factor_key(env: &Env, _market: &Address, _is_long: bool) -> BytesN<32> {
    // TODO: derive from market and direction
    BytesN::from_array(env, &[0u8; 32])
}

/// Funding amount per size key
/// TODO: implement funding_amount_per_size_key
pub fn funding_amount_per_size_key(env: &Env, _market: &Address, _collateral: &Address, _is_long: bool) -> BytesN<32> {
    // TODO: derive from market, collateral, and direction
    BytesN::from_array(env, &[0u8; 32])
}

/// Claimable funding amount key
/// TODO: implement claimable_funding_amount_key
pub fn claimable_funding_amount_key(env: &Env, _market: &Address, _token: &Address, _account: &Address) -> BytesN<32> {
    // TODO: derive from market, token, and account
    BytesN::from_array(env, &[0u8; 32])
}

/// Market list key
/// TODO: implement market_list_key
pub fn market_list_key(env: &Env) -> BytesN<32> {
    // TODO: return constant key for market list
    BytesN::from_array(env, &[0u8; 32])
}

/// Position list key
/// TODO: implement position_list_key
pub fn position_list_key(env: &Env) -> BytesN<32> {
    // TODO: return constant key for position list
    BytesN::from_array(env, &[0u8; 32])
}

/// Account position list key
/// TODO: implement account_position_list_key
pub fn account_position_list_key(env: &Env, _account: &Address) -> BytesN<32> {
    // TODO: derive from account
    BytesN::from_array(env, &[0u8; 32])
}

/// Order list key
/// TODO: implement order_list_key
pub fn order_list_key(env: &Env) -> BytesN<32> {
    // TODO: return constant key for order list
    BytesN::from_array(env, &[0u8; 32])
}

/// Account order list key
/// TODO: implement account_order_list_key
pub fn account_order_list_key(env: &Env, _account: &Address) -> BytesN<32> {
    // TODO: derive from account
    BytesN::from_array(env, &[0u8; 32])
}

/// Deposit list key
/// TODO: implement deposit_list_key
pub fn deposit_list_key(env: &Env) -> BytesN<32> {
    // TODO: return constant key for deposit list
    BytesN::from_array(env, &[0u8; 32])
}

/// Withdrawal list key
/// TODO: implement withdrawal_list_key
pub fn withdrawal_list_key(env: &Env) -> BytesN<32> {
    // TODO: return constant key for withdrawal list
    BytesN::from_array(env, &[0u8; 32])
}

/// Nonce key
/// TODO: implement nonce_key
pub fn nonce_key(env: &Env) -> BytesN<32> {
    // TODO: return constant key for nonce
    BytesN::from_array(env, &[0u8; 32])
}

/// Configuration parameter keys
/// TODO: implement config_key_* functions
pub fn max_pool_amount_key(env: &Env, _market: &Address, _token: &Address) -> BytesN<32> {
    BytesN::from_array(env, &[0u8; 32])
}

pub fn max_open_interest_key(env: &Env, _market: &Address, _is_long: bool) -> BytesN<32> {
    BytesN::from_array(env, &[0u8; 32])
}

pub fn min_collateral_factor_key(env: &Env, _market: &Address) -> BytesN<32> {
    BytesN::from_array(env, &[0u8; 32])
}

pub fn position_fee_factor_key(env: &Env) -> BytesN<32> {
    BytesN::from_array(env, &[0u8; 32])
}

pub fn borrowing_factor_key(env: &Env, _market: &Address, _is_long: bool) -> BytesN<32> {
    BytesN::from_array(env, &[0u8; 32])
}

pub fn funding_factor_key(env: &Env, _market: &Address) -> BytesN<32> {
    BytesN::from_array(env, &[0u8; 32])
}

pub fn price_impact_factor_key(env: &Env, _market: &Address) -> BytesN<32> {
    BytesN::from_array(env, &[0u8; 32])
}
