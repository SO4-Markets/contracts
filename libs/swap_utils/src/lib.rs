//! Swap utilities — single-hop and multi-hop token swaps through GMX markets.
//! Mirrors GMX's SwapUtils.sol.
//!
//! Each swap hop:
//!   - Computes price impact and swap fees.
//!   - Updates pool amounts for both tokens.
//!   - Updates the swap impact pool.
//!   - Transfers output tokens to receiver (or next hop).
#![no_std]
#![allow(dependency_on_unit_never_type_fallback)]

use soroban_sdk::{Address, BytesN, Env, Vec, token};
use gmx_types::MarketProps;
use gmx_math::{TOKEN_PRECISION, mul_div_wide};
use gmx_keys::{
    market_long_token_key, market_short_token_key, market_index_token_key,
};
use gmx_market_utils::apply_delta_to_pool_amount;
use gmx_pricing_utils::{
    get_swap_output_amount, apply_swap_impact_value,
};

#[soroban_sdk::contractclient(name = "DataStoreClient")]
trait IDataStore {
    fn get_u128(env: Env, key: BytesN<32>) -> u128;
    fn apply_delta_to_u128(env: Env, caller: Address, key: BytesN<32>, delta: i128) -> u128;
    fn get_address(env: Env, key: BytesN<32>) -> Option<Address>;
}

#[soroban_sdk::contractclient(name = "OracleClient")]
trait IOracle {
    fn get_primary_price(env: Env, token: Address) -> gmx_types::PriceProps;
}

// ─── Single-hop swap ──────────────────────────────────────────────────────────

/// Execute one swap hop: `token_in → token_out` through a single market.
///
/// `amount_in` must already be sitting in the market_token contract (the pool).
/// Returns the net output amount of `token_out`.
pub fn swap(
    env: &Env,
    data_store: &Address,
    caller: &Address,
    oracle: &Address,
    market: &MarketProps,
    token_in: &Address,
    amount_in: i128,
    receiver: &Address,
) -> (Address, i128) {
    // TODO: (mirrors GMX SwapUtils._swap)
    //
    // 1. Determine token_out (the other token in the market):
    //    if token_in == market.long_token  → token_out = market.short_token
    //    if token_in == market.short_token → token_out = market.long_token
    //    else → panic "token_in not in market"
    //
    // 2. Read prices from oracle:
    //    price_in  = oracle.get_primary_price(token_in).mid_price()
    //    price_out = oracle.get_primary_price(token_out).mid_price()
    //
    // 3. Determine if impact is positive (for fee factor selection):
    //    for_positive_impact = (price impact would improve pool balance)
    //    compute this by checking whether the swap reduces |pool_in - pool_out|
    //
    // 4. Compute output and fee:
    //    (amount_out, fee_amount) = get_swap_output_amount(
    //        env, ds, market, token_in, token_out,
    //        amount_in, price_in, price_out, for_positive_impact
    //    )
    //
    // 5. Apply swap impact to the impact pool (token_out denomination):
    //    apply_swap_impact_value(env, ds, caller, market, token_out, price_out, impact_usd)
    //
    // 6. Update pool amounts in data_store:
    //    apply_delta_to_pool_amount(env, ds, caller, market, token_in,  +amount_in)
    //    apply_delta_to_pool_amount(env, ds, caller, market, token_out, -amount_out)
    //    (pool grows by what came in, shrinks by what goes out)
    //
    // 7. Transfer token_out from market_token contract to receiver:
    //    MarketTokenClient::new(env, &market.market_token)
    //        .withdraw_from_pool(caller, token_out, receiver, amount_out)
    //    (withdraw_from_pool is the CONTROLLER-gated egress added in Phase 4)
    //
    // Returns (token_out, amount_out)
    todo!()
}

// ─── Multi-hop swap ───────────────────────────────────────────────────────────

/// Execute a swap across a path of markets.
///
/// `path` is a Vec<Address> of market_token addresses (each is one hop).
/// The first token must match the input side of path[0];
/// the final output goes to `receiver`.
///
/// Max path length enforced by MAX_SWAP_PATH_LENGTH config in data_store (default 3).
pub fn swap_with_path(
    env: &Env,
    data_store: &Address,
    caller: &Address,
    oracle: &Address,
    token_in: &Address,
    amount_in: i128,
    path: &Vec<Address>,   // Vec of market_token addresses
    receiver: &Address,
) -> (Address, i128) {
    // TODO: (mirrors GMX SwapUtils.swap path loop)
    //
    // 1. Validate path length:
    //    max_len = data_store.get_u128(max_swap_path_length_key(env)) as usize  (default 3)
    //    if path.len() > max_len → panic "swap path too long"
    //
    // 2. Deduplicate check: no market should appear twice in the path
    //    (store visited market_tokens in a temp set and check each hop)
    //
    // 3. Iterate hops:
    //    current_token  = token_in
    //    current_amount = amount_in
    //
    //    for each market_token_addr in path:
    //        Load MarketProps from data_store (index/long/short token keys)
    //        Determine next_receiver:
    //            if last hop → receiver
    //            else        → path[i+1] market_token address (tokens sit in next pool)
    //
    //        // Transfer current_amount of current_token INTO the next market pool
    //        // (caller must have arranged the transfer before this call, OR
    //        //  for intermediate hops the tokens just stay in the market contract)
    //
    //        (current_token, current_amount) = swap(
    //            env, ds, caller, oracle,
    //            &market_props, &current_token, current_amount, &next_receiver
    //        )
    //
    // 4. Return (current_token, current_amount) — final output
    todo!()
}
