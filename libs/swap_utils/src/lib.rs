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
use soroban_sdk::{Address, BytesN, Env, Vec};

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

// ─── Single-hop swap ──────────────────────────────────────────────────────────

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
    // 1. Determine token_out
    let token_out = if token_in == &market.long_token {
        market.short_token.clone()
    } else if token_in == &market.short_token {
        market.long_token.clone()
    } else {
        soroban_sdk::panic_with_error!(env, soroban_sdk::Error::from_contract_error(1u32));
    };

    // 2. Read prices from oracle
    let oracle_client = OracleClient::new(env, oracle);
    let price_in_props = oracle_client.get_primary_price(token_in);
    let price_out_props = oracle_client.get_primary_price(&token_out);
    let price_in = price_in_props.mid_price();
    let price_out = price_out_props.mid_price();

    // 3. Determine if this swap improves pool balance (for fee factor selection)
    let impact_usd = get_swap_price_impact(
        env, data_store, market, token_in, &token_out, amount_in, price_in, price_out,
    );
    let for_positive_impact = impact_usd >= 0;

    // 4. Compute output and fee
    let (amount_out, fee_amount) = get_swap_output_amount(
        env,
        data_store,
        market,
        token_in,
        &token_out,
        amount_in,
        price_in,
        price_out,
        for_positive_impact,
    );

    if amount_out == 0 {
        return (token_out, 0);
    }

    // 5. Apply swap impact to impact pool (denominated in token_out)
    apply_swap_impact_value(
        env, data_store, caller, market, &token_out, price_out, impact_usd,
    );

    // 6. Update pool amounts; track swap fee in claimable_fee_amount_key so
    //    fee_handler.claim_fees sweeps all fee paths consistently.
    apply_delta_to_pool_amount(env, data_store, caller, market, token_in, amount_in);
    apply_delta_to_pool_amount(env, data_store, caller, market, &token_out, -amount_out);
    if fee_amount > 0 {
        DataStoreClient::new(env, data_store).apply_delta_to_u128(
            caller,
            &claimable_fee_amount_key(env, &market.market_token, &token_out),
            &fee_amount,
        );
    }

    // 7. Transfer token_out from market_token pool → receiver
    MarketTokenClient::new(env, &market.market_token).withdraw_from_pool(
        caller,
        &token_out,
        receiver,
        &amount_out,
    );

    (token_out, amount_out)
}

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

    // 1. Validate path length
    let max_len = {
        let raw =
            DataStoreClient::new(env, data_store).get_u128(&max_swap_path_length_key(env)) as usize;
        if raw == 0 {
            3
        } else {
            raw
        } // default to 3 if not configured
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

    // 3. Walk the path — tokens move physically on every hop (see module comment).
    let mut current_token = token_in.clone();
    let mut current_amount = amount_in;

    for i in 0..path_len {
        let market_token_addr = path.get(i).unwrap();

        // Load market props from data_store
        let ds = DataStoreClient::new(env, data_store);
        let index_token = ds
            .get_address(&market_index_token_key(env, &market_token_addr))
            .expect("market index token not found");
        let long_token = ds
            .get_address(&market_long_token_key(env, &market_token_addr))
            .expect("market long token not found");
        let short_token = ds
            .get_address(&market_short_token_key(env, &market_token_addr))
            .expect("market short token not found");

        let market_props = MarketProps {
            market_token: market_token_addr.clone(),
            index_token,
            long_token,
            short_token,
        };

        // For intermediate hops the output token is physically transferred to the
        // next market's contract address (it becomes that pool's input for the
        // following hop).  For the final hop the output goes to the caller's receiver.
        let next_receiver = if i + 1 == path_len {
            receiver.clone()
        } else {
            // Tokens physically move from this market to the next market pool.
            // The next hop will find them already in the target pool contract.
            path.get(i + 1).unwrap()
        };

        let (out_token, out_amount) = swap(
            env,
            data_store,
            caller,
            oracle,
            &market_props,
            &current_token,
            current_amount,
            &next_receiver,
        );

        current_token = out_token;
        current_amount = out_amount;
    }

    (current_token, current_amount)
}
