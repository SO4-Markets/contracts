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

use soroban_sdk::{Address, BytesN, Env, Vec};
use gmx_types::{MarketProps, PriceProps};
use gmx_math::TOKEN_PRECISION;
use gmx_keys::{
    market_long_token_key, market_short_token_key,
    market_index_token_key, max_swap_path_length_key,
};
use gmx_market_utils::apply_delta_to_pool_amount;
use gmx_pricing_utils::{
    get_swap_output_amount, apply_swap_impact_value, get_swap_price_impact,
};

#[soroban_sdk::contractclient(name = "DataStoreClient")]
trait IDataStore {
    fn get_u128(env: Env, key: BytesN<32>) -> u128;
    fn apply_delta_to_u128(env: Env, caller: Address, key: BytesN<32>, delta: i128) -> u128;
    fn get_address(env: Env, key: BytesN<32>) -> Option<Address>;
}

#[soroban_sdk::contractclient(name = "OracleClient")]
trait IOracle {
    fn get_primary_price(env: Env, token: Address) -> PriceProps;
}

#[soroban_sdk::contractclient(name = "MarketTokenClient")]
trait IMarketToken {
    fn withdraw_from_pool(env: Env, caller: Address, pool_token: Address, receiver: Address, amount: i128);
}

// ─── Single-hop swap ──────────────────────────────────────────────────────────

/// Execute one swap hop: `token_in → token_out` through a single market.
///
/// `amount_in` must already be sitting in the market_token contract (the pool).
/// Returns (token_out, net_output_amount).
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
    // 1. Determine token_out
    let token_out = if token_in == &market.long_token {
        market.short_token.clone()
    } else if token_in == &market.short_token {
        market.long_token.clone()
    } else {
        panic!("token_in not in market")
    };

    // 2. Read prices from oracle
    let oracle_client = OracleClient::new(env, oracle);
    let price_in_props  = oracle_client.get_primary_price(token_in);
    let price_out_props = oracle_client.get_primary_price(&token_out);
    let price_in  = price_in_props.mid_price();
    let price_out = price_out_props.mid_price();

    // 3. Determine if this swap improves pool balance (for fee factor selection)
    let impact_usd = get_swap_price_impact(
        env, data_store, market,
        token_in, &token_out,
        amount_in, price_in, price_out,
    );
    let for_positive_impact = impact_usd >= 0;

    // 4. Compute output and fee
    let (amount_out, _fee_amount) = get_swap_output_amount(
        env, data_store, market,
        token_in, &token_out,
        amount_in, price_in, price_out,
        for_positive_impact,
    );

    if amount_out == 0 {
        return (token_out, 0);
    }

    // 5. Apply swap impact to impact pool (denominated in token_out)
    apply_swap_impact_value(env, data_store, caller, market, &token_out, price_out, impact_usd);

    // 6. Update pool amounts
    apply_delta_to_pool_amount(env, data_store, caller, market, token_in,   amount_in);
    apply_delta_to_pool_amount(env, data_store, caller, market, &token_out, -amount_out);

    // 7. Transfer token_out from market_token pool → receiver
    MarketTokenClient::new(env, &market.market_token)
        .withdraw_from_pool(caller, &token_out, receiver, &amount_out);

    (token_out, amount_out)
}

// ─── Multi-hop swap ───────────────────────────────────────────────────────────

/// Execute a swap across a path of markets.
///
/// `path` is a Vec<Address> of market_token addresses (each is one hop).
/// Max path length read from data_store (default 3).
pub fn swap_with_path(
    env: &Env,
    data_store: &Address,
    caller: &Address,
    oracle: &Address,
    token_in: &Address,
    amount_in: i128,
    path: &Vec<Address>,
    receiver: &Address,
) -> (Address, i128) {
    // 1. Validate path length
    let max_len = {
        let raw = DataStoreClient::new(env, data_store)
            .get_u128(&max_swap_path_length_key(env)) as usize;
        if raw == 0 { 3 } else { raw } // default to 3 if not configured
    };
    if path.len() as usize > max_len {
        panic!("swap path too long");
    }

    // 2. Walk the path
    let mut current_token = token_in.clone();
    let mut current_amount = amount_in;
    let path_len = path.len();

    for i in 0..path_len {
        let market_token_addr = path.get(i).unwrap();

        // Load market props from data_store
        let ds = DataStoreClient::new(env, data_store);
        let index_token = ds.get_address(&market_index_token_key(env, &market_token_addr))
            .unwrap_or_else(|| panic!("market index token not found"));
        let long_token  = ds.get_address(&market_long_token_key(env, &market_token_addr))
            .unwrap_or_else(|| panic!("market long token not found"));
        let short_token = ds.get_address(&market_short_token_key(env, &market_token_addr))
            .unwrap_or_else(|| panic!("market short token not found"));

        let market_props = MarketProps {
            market_token: market_token_addr.clone(),
            index_token,
            long_token,
            short_token,
        };

        // For non-final hops: output stays in the next pool (tokens don't move to receiver yet).
        // For the final hop: send to receiver.
        let next_receiver = if (i + 1) as u32 == path_len {
            receiver.clone()
        } else {
            // Output stays in this market's pool for the next hop to read
            // (we don't actually move tokens between pools mid-path;
            //  instead we carry the amount and re-apply on next hop)
            market_token_addr.clone()
        };

        let (out_token, out_amount) = swap(
            env, data_store, caller, oracle,
            &market_props, &current_token, current_amount, &next_receiver,
        );

        current_token  = out_token;
        current_amount = out_amount;
    }

    (current_token, current_amount)
}
