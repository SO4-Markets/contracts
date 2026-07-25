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

use gmx_keys::{
    claimable_fee_amount_key, market_index_token_key, market_long_token_key,
    market_short_token_key, max_swap_path_length_key,
};
use gmx_market_utils::apply_delta_to_pool_amount;
use gmx_pricing_utils::{apply_swap_impact_value, get_swap_output_amount, get_swap_price_impact};
use gmx_types::{MarketProps, PriceProps};
use soroban_sdk::{Address, BytesN, Env, Map, Vec};

#[allow(dead_code)]
#[soroban_sdk::contractclient(name = "DataStoreClient")]
trait IDataStore {
    fn get_u128(env: Env, key: BytesN<32>) -> u128;
    fn apply_delta_to_u128(env: Env, caller: Address, key: BytesN<32>, delta: i128) -> u128;
    fn get_address(env: Env, key: BytesN<32>) -> Option<Address>;
}

#[allow(dead_code)]
#[soroban_sdk::contractclient(name = "OracleClient")]
trait IOracle {
    fn get_primary_price(env: Env, token: Address) -> PriceProps;
}

#[allow(dead_code)]
#[soroban_sdk::contractclient(name = "MarketTokenClient")]
trait IMarketToken {
    fn withdraw_from_pool(
        env: Env,
        caller: Address,
        pool_token: Address,
        receiver: Address,
        amount: i128,
    );
}

// ─── Single-hop swap (with pre-fetched prices) ────────────────────────────────

/// Execute one swap hop using caller-supplied prices.
///
/// This is the canonical inner implementation. Both the public single-hop
/// `swap()` and the multi-hop `swap_with_path()` delegate here after
/// obtaining prices — the former via a direct oracle call, the latter via
/// the pre-fetched price cache built once at the start of the path.
#[allow(clippy::too_many_arguments)]
fn swap_with_prices(
    env: &Env,
    data_store: &Address,
    caller: &Address,
    market: &MarketProps,
    token_in: &Address,
    amount_in: i128,
    receiver: &Address,
    price_in: i128,
    price_out: i128,
    token_out: &Address,
) -> (Address, i128) {
    // 1. Determine if this swap improves pool balance (for fee factor selection)
    let impact_usd = get_swap_price_impact(
        env, data_store, market, token_in, token_out, amount_in, price_in, price_out,
    );
    let for_positive_impact = impact_usd >= 0;

    // 2. Compute output and fee
    let (amount_out, fee_amount) = get_swap_output_amount(
        env,
        data_store,
        market,
        token_in,
        token_out,
        amount_in,
        price_in,
        price_out,
        for_positive_impact,
    );

    // The caller already physically transferred `amount_in` of `token_in` into
    // this market before invoking this function (see the token-movement notes
    // above). If a hop's amount_out rounds to zero — easily reached with small
    // amounts, since fee rounding always rounds a nonzero fee up to at least 1
    // unit — silently returning here would leave those tokens un-credited to
    // pool_amount forever, even though the order-level min_output_amount check
    // (or an intermediate hop with no such check at all) would not catch it.
    // Revert instead so no on-chain transfer executes with unaccounted-for input.
    if amount_out == 0 {
        soroban_sdk::panic_with_error!(env, soroban_sdk::Error::from_contract_error(4u32));
    }

    // 3. Apply swap impact to impact pool (denominated in token_out)
    apply_swap_impact_value(
        env, data_store, caller, market, token_out, price_out, impact_usd,
    );

    // 4. Update pool amounts; track swap fee in claimable_fee_amount_key so
    //    fee_handler.claim_fees sweeps all fee paths consistently.
    apply_delta_to_pool_amount(env, data_store, caller, market, token_in, amount_in);
    apply_delta_to_pool_amount(env, data_store, caller, market, token_out, -amount_out);
    if fee_amount > 0 {
        DataStoreClient::new(env, data_store).apply_delta_to_u128(
            caller,
            &claimable_fee_amount_key(env, &market.market_token, token_out),
            &fee_amount,
        );
    }

    // 5. Transfer token_out from market_token pool → receiver
    MarketTokenClient::new(env, &market.market_token).withdraw_from_pool(
        caller,
        token_out,
        receiver,
        &amount_out,
    );

    (token_out.clone(), amount_out)
}

// ─── Public single-hop swap ───────────────────────────────────────────────────

#[allow(clippy::too_many_arguments)]
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
    // Determine token_out
    let token_out = if token_in == &market.long_token {
        market.short_token.clone()
    } else if token_in == &market.short_token {
        market.long_token.clone()
    } else {
        soroban_sdk::panic_with_error!(env, soroban_sdk::Error::from_contract_error(1u32));
    };

    // Fetch prices from oracle (single-hop path; no pre-fetch possible here)
    let oracle_client = OracleClient::new(env, oracle);
    let price_in = oracle_client.get_primary_price(token_in).mid_price();
    let price_out = oracle_client.get_primary_price(&token_out).mid_price();

    swap_with_prices(
        env, data_store, caller, market, token_in, amount_in, receiver,
        price_in, price_out, &token_out,
    )
}

/// Absolute upper bound on swap path length (number of market hops).
///
/// Used as the fallback cap when `max_swap_path_length_key` is not configured
/// in DataStore (returns 0). Enforced both at order creation time (#300) and
/// at swap execution time so that neither path can be bypassed.
pub const MAX_SWAP_PATH_LENGTH: usize = 5;

// ─── Multi-hop swap ───────────────────────────────────────────────────────────
//
// # Token movement semantics (issue #57)
//
// Tokens move **physically** between pools on every hop.  There is no virtual
// accounting shortcut — each intermediate transfer is an actual SEP-41 token
// transfer on-chain.  The flow for a two-hop path A→B→C is:
//
//   Before first hop : order_handler has already transferred `token_A` from the
//                      order_vault into `market_1`'s contract address.
//
//   Hop 1 (market_1, A→B):
//     • pool_1 amount_A  += input_amount    (DataStore record)
//     • pool_1 amount_B  -= output_amount   (DataStore record)
//     • SEP-41 transfer  : token_B moves from market_1 → market_2 (physical)
//
//   Hop 2 (market_2, B→C):
//     • pool_2 amount_B  += intermediate_amount   (DataStore record, tokens
//                                                   already physically present)
//     • pool_2 amount_C  -= output_amount          (DataStore record)
//     • SEP-41 transfer  : token_C moves from market_2 → final_receiver (physical)
//
// Pool balance invariant (after full execution):
//   market_1 on-chain balance of token_B  == recorded pool_1 amount_B
//   market_2 on-chain balance of token_C  == recorded pool_2 amount_C
//
// # Duplicate market guard (issue #56)
//
// A swap path with repeated market addresses would cause the same pool's state
// to be mutated twice inside one transaction, double-counting both the pool
// amounts and the price-impact pool.  This function rejects any path that
// contains a duplicate market address before any state is touched.
//
// # Oracle pre-fetch optimisation (issue #298)
//
// In a N-hop swap the oracle was previously queried twice per hop (once for
// token_in, once for token_out), giving 2N cross-contract oracle calls.
// Since oracle prices are ledger-scoped and constant within a single
// transaction, prices are now collected once before the hop loop:
//
//   1. First pass: load market props (long_token + short_token) for every
//      market in the path from DataStore.
//   2. Pre-fetch prices for every unique token seen (≤ N+1 unique tokens
//      for an N-hop path).
//   3. Main loop: execute each hop with prices looked up from the cache —
//      zero additional oracle calls.
//
// Net oracle calls: O(unique tokens) ≤ O(N+1)  vs  O(2N) previously.

#[allow(clippy::too_many_arguments)]
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
    let path_len = path.len();

    // 1. Validate path length against the DataStore-configured cap (or the compile-time constant).
    let max_len = {
        let raw =
            DataStoreClient::new(env, data_store).get_u128(&max_swap_path_length_key(env)) as usize;
        if raw == 0 { MAX_SWAP_PATH_LENGTH } else { raw }
    };
    if path_len as usize > max_len {
        soroban_sdk::panic_with_error!(env, soroban_sdk::Error::from_contract_error(2u32));
    }

    // 2. Reject duplicate market addresses in path (issue #56).
    //    Any repeated market would double-mutate pool state and corrupt
    //    price-impact accounting; revert before any state change.
    {
        let mut i = 0u32;
        while i < path_len {
            let mut j = i + 1;
            while j < path_len {
                if path.get(i).unwrap() == path.get(j).unwrap() {
                    // Error code 3 = DuplicateMarketInPath
                    soroban_sdk::panic_with_error!(
                        env,
                        soroban_sdk::Error::from_contract_error(3u32)
                    );
                }
                j += 1;
            }
            i += 1;
        }
    }

    // 3. First pass: load all market props and collect unique token addresses.
    //    Doing this upfront lets us pre-fetch all oracle prices in one batch
    //    (issue #298) without redundant cross-contract calls in the main loop.
    let ds = DataStoreClient::new(env, data_store);
    let mut market_props_cache: Vec<MarketProps> = Vec::new(env);
    // Price cache: token address → PriceProps.  Populated below, used in main loop.
    let mut price_cache: Map<Address, PriceProps> = Map::new(env);

    // Seed with the initial token_in so it is always in the cache.
    let oracle_client = OracleClient::new(env, oracle);
    price_cache.set(token_in.clone(), oracle_client.get_primary_price(token_in));

    let mut i = 0u32;
    while i < path_len {
        let market_token_addr = path.get(i).unwrap();
        let index_token = ds
            .get_address(&market_index_token_key(env, &market_token_addr))
            .expect("market index token not found");
        let long_token = ds
            .get_address(&market_long_token_key(env, &market_token_addr))
            .expect("market long token not found");
        let short_token = ds
            .get_address(&market_short_token_key(env, &market_token_addr))
            .expect("market short token not found");

        // Pre-fetch long_token price if not already cached.
        if price_cache.get(long_token.clone()).is_none() {
            price_cache.set(
                long_token.clone(),
                oracle_client.get_primary_price(&long_token),
            );
        }
        // Pre-fetch short_token price if not already cached.
        if price_cache.get(short_token.clone()).is_none() {
            price_cache.set(
                short_token.clone(),
                oracle_client.get_primary_price(&short_token),
            );
        }

        market_props_cache.push_back(MarketProps {
            market_token: market_token_addr,
            index_token,
            long_token,
            short_token,
        });
        i += 1;
    }

    // 4. Main loop — execute each hop using cached market props and pre-fetched prices.
    //    No oracle calls occur here (issue #298).
    let mut current_token = token_in.clone();
    let mut current_amount = amount_in;

    let mut hop = 0u32;
    while hop < path_len {
        let market_props = market_props_cache.get(hop).unwrap();

        // Determine token_out from market configuration.
        let token_out = if current_token == market_props.long_token {
            market_props.short_token.clone()
        } else if current_token == market_props.short_token {
            market_props.long_token.clone()
        } else {
            soroban_sdk::panic_with_error!(env, soroban_sdk::Error::from_contract_error(1u32));
        };

        // Look up pre-fetched prices (guaranteed present from first pass above).
        let price_in = price_cache
            .get(current_token.clone())
            .expect("price_in not in cache")
            .mid_price();
        let price_out = price_cache
            .get(token_out.clone())
            .expect("price_out not in cache")
            .mid_price();

        // Intermediate hops send output to the next market pool; final hop to receiver.
        let next_receiver = if hop + 1 == path_len {
            receiver.clone()
        } else {
            path.get(hop + 1).unwrap()
        };

        let (out_token, out_amount) = swap_with_prices(
            env,
            data_store,
            caller,
            &market_props,
            &current_token,
            current_amount,
            &next_receiver,
            price_in,
            price_out,
            &token_out,
        );

        current_token = out_token;
        current_amount = out_amount;
        hop += 1;
    }

    (current_token, current_amount)
}
