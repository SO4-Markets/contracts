//! Decrease position utilities — partial or full close of a long/short position.
//! Mirrors GMX's DecreasePositionUtils.sol.
//!
//! Flow:
//!   1. Update market funding and borrowing state.
//!   2. Settle claimable funding for this position.
//!   3. Compute price impact and execution price.
//!   4. Realise PnL for the closing slice.
//!   5. Deduct fees from remaining collateral.
//!   6. Update position size, tokens, and trackers.
//!   7. Apply OI deltas and pool updates.
//!   8. Validate (if partial) or remove (if fully closed) position.
//!   9. Transfer output tokens to receiver (or swap to requested token).
#![no_std]
#![allow(dependency_on_unit_never_type_fallback)]

use gmx_keys::{
    account_position_list_key, claimable_fee_amount_key, collateral_sum_key,
    cumulative_borrowing_factor_key, funding_amount_per_size_key, position_key, position_list_key,
};
use gmx_market_utils::{
    apply_delta_to_open_interest, apply_delta_to_open_interest_in_tokens,
    apply_delta_to_pool_amount, update_cumulative_borrowing_factor, update_funding_state,
};
use gmx_math::{mul_div_wide, TOKEN_PRECISION};
use gmx_position_utils::{
    get_position_fees, get_position_pnl_usd, settle_funding_fees, validate_position,
};
use gmx_pricing_utils::{
    apply_position_impact_value, get_execution_price, get_position_price_impact,
};
use gmx_swap_utils::swap_with_path;
use gmx_types::{DecreasePositionResult, MarketProps, PositionProps, PriceProps};
use soroban_sdk::{contracttype, Address, BytesN, Env, Vec};

#[allow(dead_code)]
#[soroban_sdk::contractclient(name = "DataStoreClient")]
trait IDataStore {
    fn get_u128(env: Env, key: BytesN<32>) -> u128;
    fn get_i128(env: Env, key: BytesN<32>) -> i128;
    fn set_u128(env: Env, caller: Address, key: BytesN<32>, value: u128) -> u128;
    fn apply_delta_to_u128(env: Env, caller: Address, key: BytesN<32>, delta: i128) -> u128;
    fn apply_delta_to_i128(env: Env, caller: Address, key: BytesN<32>, delta: i128) -> i128;
    fn get_address(env: Env, key: BytesN<32>) -> Option<Address>;
    fn remove_bytes32_from_set(env: Env, caller: Address, set_key: BytesN<32>, value: BytesN<32>);
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

// ─── Position storage key ──────────────────────────────────────────────────────

#[contracttype]
enum PositionKey {
    Position(BytesN<32>),
}

// ─── Params ───────────────────────────────────────────────────────────────────

pub struct DecreasePositionParams<'a> {
    pub data_store: &'a Address,
    pub caller: &'a Address,   // handler contract address (has CONTROLLER)
    pub account: &'a Address,  // position owner
    pub receiver: &'a Address, // where output tokens are sent
    pub market: &'a MarketProps,
    pub collateral_token: &'a Address,
    pub size_delta_usd: i128,   // USD value of the slice being closed
    pub acceptable_price: i128, // FLOAT_PRECISION; 0 = no slippage check
    pub is_long: bool,
    pub index_token_price: &'a PriceProps,
    pub collateral_price: i128, // FLOAT_PRECISION
    pub current_time: u64,
    /// Swap path for the output token. Empty = return collateral token; non-empty = swap to
    /// the requested output token via the given market hops.
    pub swap_path: Vec<Address>,
    /// Oracle address, used when swap_path is non-empty.
    pub oracle: &'a Address,
}

// ─── Main entry ───────────────────────────────────────────────────────────────

/// Decrease or fully close a position. Returns a `DecreasePositionResult`.
pub fn decrease_position(env: &Env, p: &DecreasePositionParams) -> DecreasePositionResult {
    let pos_key = position_key(
        env,
        p.account,
        &p.market.market_token,
        p.collateral_token,
        p.is_long,
    );
    let storage_key = PositionKey::Position(pos_key.clone());

    // 1. Load position
    let mut position: PositionProps = env
        .storage()
        .persistent()
        .get(&storage_key)
        .expect("position not found");

    // Clamp size_delta to full close if needed
    let size_delta_usd = p.size_delta_usd.min(position.size_in_usd);

    // 2. Update market funding + borrowing state
    let index_price_mid = p.index_token_price.mid_price();
    update_funding_state(
        env,
        p.data_store,
        p.caller,
        p.market,
        index_price_mid,
        index_price_mid,
        p.current_time,
    );
    update_cumulative_borrowing_factor(
        env,
        p.data_store,
        p.caller,
        p.market,
        p.is_long,
        p.current_time,
    );

    // 3. Settle pending funding for this position
    settle_funding_fees(env, p.data_store, p.caller, p.market, &mut position);

    // 4. Price impact (decrease: is_increase = false)
    let impact_usd = get_position_price_impact(
        env,
        p.data_store,
        p.market,
        p.is_long,
        size_delta_usd,
        false,
        index_price_mid,
    );
    apply_position_impact_value(
        env,
        p.data_store,
        p.caller,
        p.market,
        impact_usd,
        index_price_mid,
    );

    // 5. Execution price
    let execution_price = get_execution_price(
        env,
        index_price_mid,
        size_delta_usd,
        impact_usd,
        p.is_long,
        false,
    );
    if p.acceptable_price != 0 {
        if p.is_long && execution_price < p.acceptable_price {
            soroban_sdk::panic_with_error!(env, soroban_sdk::Error::from_contract_error(1u32));
        }
        if !p.is_long && execution_price > p.acceptable_price {
            soroban_sdk::panic_with_error!(env, soroban_sdk::Error::from_contract_error(2u32));
        }
    }

    // 6. Size delta in tokens (proportional to position)
    let size_delta_in_tokens = if position.size_in_usd > 0 {
        mul_div_wide(
            env,
            size_delta_usd,
            position.size_in_tokens,
            position.size_in_usd,
        )
    } else {
        0
    };

    // 7. Realise PnL for the closing slice
    let (pnl_usd, _) = get_position_pnl_usd(env, &position, p.index_token_price, size_delta_usd);
    let pnl_token_amount = if p.collateral_price > 0 {
        mul_div_wide(env, pnl_usd, TOKEN_PRECISION, p.collateral_price)
    } else {
        0
    };

    // Settle PnL with the pool:
    //   trader profit → pool shrinks (pool pays trader)
    //   trader loss   → pool grows  (trader pays pool)
    if pnl_token_amount > 0 {
        apply_delta_to_pool_amount(
            env,
            p.data_store,
            p.caller,
            p.market,
            p.collateral_token,
            -pnl_token_amount,
        );
    } else if pnl_token_amount < 0 {
        apply_delta_to_pool_amount(
            env,
            p.data_store,
            p.caller,
            p.market,
            p.collateral_token,
            -pnl_token_amount,
        ); // negative delta = pool grows
    }

    // 8. Position fees
    let for_positive_impact = impact_usd >= 0;
    let fees = get_position_fees(
        env,
        p.data_store,
        p.market,
        &position,
        p.collateral_price,
        size_delta_usd,
        for_positive_impact,
    );
    // Fee income goes to pool; also track in claimable_fee_amount_key so
    // fee_handler.claim_fees can sweep it consistently across all fee paths.
    apply_delta_to_pool_amount(
        env,
        p.data_store,
        p.caller,
        p.market,
        p.collateral_token,
        fees.total_cost_amount,
    );
    if fees.total_cost_amount > 0 {
        DataStoreClient::new(env, p.data_store).apply_delta_to_u128(
            p.caller,
            &claimable_fee_amount_key(env, &p.market.market_token, p.collateral_token),
            &(fees.total_cost_amount as i128),
        );
    }

    // 9. Compute output amount
    // For a partial close, we return the collateral proportional to the size delta
    let collateral_delta = if position.size_in_usd > 0 {
        mul_div_wide(
            env,
            position.collateral_amount,
            size_delta_usd,
            position.size_in_usd,
        )
    } else {
        position.collateral_amount
    };

    let raw_output = collateral_delta + pnl_token_amount - fees.total_cost_amount;
    let output_amount = raw_output.max(0);

    // 10. Update position size fields
    position.size_in_usd -= size_delta_usd;
    position.size_in_tokens -= size_delta_in_tokens;
    position.collateral_amount -= collateral_delta;
    position.decreased_at_time = p.current_time;

    // Sync trackers
    let cum_borrow_key = cumulative_borrowing_factor_key(env, &p.market.market_token, p.is_long);
    position.borrowing_factor =
        DataStoreClient::new(env, p.data_store).get_u128(&cum_borrow_key) as i128;

    let fnd_key =
        funding_amount_per_size_key(env, &p.market.market_token, p.collateral_token, p.is_long);
    position.funding_fee_amount_per_size =
        DataStoreClient::new(env, p.data_store).get_i128(&fnd_key);

    // 11. Open interest deltas
    apply_delta_to_open_interest(
        env,
        p.data_store,
        p.caller,
        p.market,
        p.collateral_token,
        p.is_long,
        -size_delta_usd,
    );
    apply_delta_to_open_interest_in_tokens(
        env,
        p.data_store,
        p.caller,
        p.market,
        p.collateral_token,
        p.is_long,
        -size_delta_in_tokens,
    );

    // 12. Collateral sum
    let col_sum_key =
        collateral_sum_key(env, &p.market.market_token, p.collateral_token, p.is_long);
    DataStoreClient::new(env, p.data_store).apply_delta_to_u128(
        p.caller,
        &col_sum_key,
        &(-output_amount),
    );

    // 13. Persist or remove position
    let is_fully_closed = position.size_in_usd == 0;
    let remaining_collateral = position.collateral_amount;

    if is_fully_closed {
        env.storage().persistent().remove(&storage_key);
        let ds = DataStoreClient::new(env, p.data_store);
        ds.remove_bytes32_from_set(p.caller, &position_list_key(env), &pos_key);
        ds.remove_bytes32_from_set(
            p.caller,
            &account_position_list_key(env, p.account),
            &pos_key,
        );
    } else {
        validate_position(
            env,
            p.data_store,
            &position,
            p.market,
            p.collateral_price,
            p.index_token_price,
        );
        env.storage().persistent().set(&storage_key, &position);
    }

    // 14. Transfer output to receiver, optionally swapping to requested token
    let mut secondary_output_amount: i128 = 0;
    if output_amount > 0 {
        if p.swap_path.is_empty() {
            // No swap: return collateral token directly to receiver
            MarketTokenClient::new(env, &p.market.market_token).withdraw_from_pool(
                p.caller,
                p.collateral_token,
                p.receiver,
                &output_amount,
            );
        } else {
            // Swap path: route collateral through the path to get the requested output token
            let first_market = p.swap_path.get(0).unwrap();
            MarketTokenClient::new(env, &p.market.market_token).withdraw_from_pool(
                p.caller,
                p.collateral_token,
                &first_market,
                &output_amount,
            );
            let (_out_token, swapped) = swap_with_path(
                env,
                p.data_store,
                p.caller,
                p.oracle,
                p.collateral_token,
                output_amount,
                &p.swap_path,
                p.receiver,
            );
            secondary_output_amount = swapped;
        }
    }

    env.events().publish(
        (soroban_sdk::symbol_short!("pos_dec"),),
        (
            pos_key,
            p.account.clone(),
            size_delta_usd,
            execution_price,
            pnl_usd,
        ),
    );

    DecreasePositionResult {
        execution_price,
        pnl_usd,
        output_amount,
        secondary_output_amount,
        remaining_collateral,
        is_fully_closed,
    }
}

// ─── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use data_store::{DataStore, DataStoreClient as DsClient};
    use gmx_keys::roles;
    use gmx_math::{FLOAT_PRECISION, TOKEN_PRECISION};
    use gmx_types::{PositionProps, PriceProps};
    use market_token::{MarketToken, MarketTokenClient as MtClient};
    use role_store::{RoleStore, RoleStoreClient as RsClient};
    use soroban_sdk::{testutils::Address as _, token::StellarAssetClient, Env};

    const ONE_TOKEN: i128 = 10_000_000; // 7-decimal Stellar precision
    const FP: i128 = FLOAT_PRECISION;

    struct World {
        env: Env,
        admin: Address,
        caller: Address,
        user: Address,
        ds: Address,
        market_tk: Address,
        long_tk: Address,
        short_tk: Address,
        index_tk: Address,
    }

    #[soroban_sdk::contract]
    pub struct DummyContract;
    #[soroban_sdk::contractimpl]
    impl DummyContract {}

    fn setup() -> World {
        let env = Env::default();
        env.mock_all_auths();

        let admin = Address::generate(&env);
        let caller = env.register(DummyContract, ());
        let user = Address::generate(&env);

        let rs = env.register(RoleStore, ());
        let rs_c = RsClient::new(&env, &rs);
        rs_c.initialize(&admin);
        rs_c.grant_role(&admin, &admin, &roles::controller(&env));
        rs_c.grant_role(&admin, &caller, &roles::controller(&env));

        let ds = env.register(DataStore, ());
        DsClient::new(&env, &ds).initialize(&admin, &rs);

        let market_tk = env.register(MarketToken, ());
        MtClient::new(&env, &market_tk).initialize(
            &admin,
            &rs,
            &7u32,
            &soroban_sdk::String::from_str(&env, "SO4 Market"),
            &soroban_sdk::String::from_str(&env, "GM"),
        );
        rs_c.grant_role(&admin, &market_tk, &roles::controller(&env));

        let long_tk = env
            .register_stellar_asset_contract_v2(admin.clone())
            .address();
        let short_tk = env
            .register_stellar_asset_contract_v2(admin.clone())
            .address();
        let index_tk = Address::generate(&env);

        let ds_c = DsClient::new(&env, &ds);
        ds_c.set_address(
            &admin,
            &gmx_keys::market_index_token_key(&env, &market_tk),
            &index_tk,
        );
        ds_c.set_address(
            &admin,
            &gmx_keys::market_long_token_key(&env, &market_tk),
            &long_tk,
        );
        ds_c.set_address(
            &admin,
            &gmx_keys::market_short_token_key(&env, &market_tk),
            &short_tk,
        );

        World {
            env,
            admin,
            caller,
            user,
            ds,
            market_tk,
            long_tk,
            short_tk,
            index_tk,
        }
    }

    fn configure_market(w: &World, fee_bps: i128) {
        let ds_c = DsClient::new(&w.env, &w.ds);
        let fee_factor = fee_bps * FP / 10_000;
        ds_c.set_u128(
            &w.admin,
            &gmx_keys::position_fee_factor_key(&w.env, &w.market_tk, true),
            &(fee_factor as u128),
        );
        ds_c.set_u128(
            &w.admin,
            &gmx_keys::position_fee_factor_key(&w.env, &w.market_tk, false),
            &(fee_factor as u128),
        );
        ds_c.set_u128(
            &w.admin,
            &gmx_keys::max_leverage_key(&w.env, &w.market_tk),
            &(50 * FP as u128),
        );
        // Seed pool so withdraw_from_pool succeeds
        ds_c.set_u128(
            &w.admin,
            &gmx_keys::pool_amount_key(&w.env, &w.market_tk, &w.long_tk),
            &(10_000 * ONE_TOKEN as u128),
        );
        ds_c.set_u128(
            &w.admin,
            &gmx_keys::pool_amount_key(&w.env, &w.market_tk, &w.short_tk),
            &(10_000 * ONE_TOKEN as u128),
        );
    }

    /// Store a synthetic position in the env's persistent storage so `decrease_position` can load it.
    fn plant_position(
        w: &World,
        size_usd: i128,
        collateral: i128,
        index_price: i128,
    ) -> PositionProps {
        let size_in_tokens = gmx_math::mul_div_wide(&w.env, size_usd, TOKEN_PRECISION, index_price);
        let position = PositionProps {
            account: w.user.clone(),
            market: w.market_tk.clone(),
            collateral_token: w.long_tk.clone(),
            size_in_usd: size_usd,
            size_in_tokens,
            collateral_amount: collateral,
            pending_impact_amount: 0,
            borrowing_factor: 0,
            funding_fee_amount_per_size: 0,
            long_claim_fnd_per_size: 0,
            short_claim_fnd_per_size: 0,
            increased_at_time: 1_000,
            decreased_at_time: 0,
            is_long: true,
        };
        let pos_key = gmx_keys::position_key(&w.env, &w.user, &w.market_tk, &w.long_tk, true);
        w.env.as_contract(&w.caller, || {
            w.env
                .storage()
                .persistent()
                .set(&PositionKey::Position(pos_key), &position);
        });

        // Seed Open Interest and Collateral Sum in DataStore to prevent underflows on decrease
        let ds_c = DsClient::new(&w.env, &w.ds);
        let oi_key = gmx_keys::open_interest_key(&w.env, &w.market_tk, &w.long_tk, true);
        ds_c.set_u128(&w.admin, &oi_key, &(size_usd as u128));

        let oi_tokens_key =
            gmx_keys::open_interest_in_tokens_key(&w.env, &w.market_tk, &w.long_tk, true);
        ds_c.set_u128(&w.admin, &oi_tokens_key, &(size_in_tokens as u128));

        let col_sum_key = gmx_keys::collateral_sum_key(&w.env, &w.market_tk, &w.long_tk, true);
        ds_c.set_u128(&w.admin, &col_sum_key, &(collateral as u128));

        // Mint enough tokens into the market pool so withdraw_from_pool can transfer
        StellarAssetClient::new(&w.env, &w.long_tk)
            .mint(&w.market_tk, &(collateral + 5 * ONE_TOKEN));
        position
    }

    fn make_market(w: &World) -> MarketProps {
        // Issue #248: build via the shared constructor instead of a per-field literal.
        MarketProps::new(&w.market_tk, &w.index_tk, &w.long_tk, &w.short_tk)
    }

    // ── Task 1: fee delta, PnL, output verification ───────────────────────────

    /// Partial close: position_fee amount matches the expected formula.
    #[test]
    fn partial_close_position_fee_matches_formula() {
        let w = setup();
        let index_price = 2_000 * FP;
        let fee_bps: i128 = 10; // 10 bps = 0.1%
        configure_market(&w, fee_bps);

        let size_usd = 1_000 * FP; // $1 000 position
        let collateral = ONE_TOKEN * 10;
        plant_position(&w, size_usd, collateral, index_price);

        let market = make_market(&w);
        let price = PriceProps {
            min: index_price,
            max: index_price,
        };
        let size_delta = size_usd / 2; // close 50%

        let result = w.env.as_contract(&w.caller, || {
            decrease_position(
                &w.env,
                &DecreasePositionParams {
                    data_store: &w.ds,
                    caller: &w.caller,
                    account: &w.user,
                    receiver: &w.user,
                    market: &market,
                    collateral_token: &w.long_tk,
                    size_delta_usd: size_delta,
                    acceptable_price: 0,
                    is_long: true,
                    index_token_price: &price,
                    collateral_price: index_price,
                    current_time: 2_000,
                    swap_path: Vec::new(&w.env),
                    oracle: &w.admin, // unused; no swap path
                },
            )
        });

        // Expected position fee: size_delta × fee_factor / FLOAT_PRECISION / collateral_price × TOKEN_PRECISION
        let fee_factor = fee_bps * FP / 10_000;
        let fee_usd = gmx_math::mul_div_wide(&w.env, size_delta, fee_factor, FP);
        let expected_fee_tok =
            gmx_math::mul_div_wide(&w.env, fee_usd, TOKEN_PRECISION, index_price);
        let collateral_delta = collateral / 2; // proportional to size_delta / size_usd
        let expected_output = collateral_delta - expected_fee_tok;

        // No underflow: output is non-negative
        assert!(
            result.output_amount >= 0,
            "output must be non-negative, got {}",
            result.output_amount
        );

        // Fee is non-zero
        assert!(expected_fee_tok > 0, "expected_fee must be non-zero");

        // Output matches expected (allow ±2 for rounding)
        let diff = (result.output_amount - expected_output).abs();
        assert!(
            diff <= 2,
            "output_amount={} expected={} diff={}",
            result.output_amount,
            expected_output,
            diff
        );

        // Partial close: position still open, remaining collateral is positive
        assert!(!result.is_fully_closed, "should be partial close");
        assert!(
            result.remaining_collateral > 0,
            "remaining collateral must be positive after partial close"
        );
    }

    /// Full close: output is non-negative and position is marked fully closed.
    #[test]
    fn full_close_output_non_negative_and_fully_closed() {
        let w = setup();
        let index_price = 2_000 * FP;
        configure_market(&w, 10);

        let size_usd = 500 * FP;
        let collateral = ONE_TOKEN * 5;
        plant_position(&w, size_usd, collateral, index_price);

        let market = make_market(&w);
        let price = PriceProps {
            min: index_price,
            max: index_price,
        };

        let result = w.env.as_contract(&w.caller, || {
            decrease_position(
                &w.env,
                &DecreasePositionParams {
                    data_store: &w.ds,
                    caller: &w.caller,
                    account: &w.user,
                    receiver: &w.user,
                    market: &market,
                    collateral_token: &w.long_tk,
                    size_delta_usd: size_usd, // full close
                    acceptable_price: 0,
                    is_long: true,
                    index_token_price: &price,
                    collateral_price: index_price,
                    current_time: 2_000,
                    swap_path: Vec::new(&w.env),
                    oracle: &w.admin,
                },
            )
        });

        assert!(
            result.output_amount >= 0,
            "output must be non-negative on full close"
        );
        assert!(result.is_fully_closed, "position must be fully closed");
        assert_eq!(
            result.remaining_collateral, 0,
            "no collateral remains after full close"
        );
    }

    /// When fees exceed the raw output (large loss), output is clamped to zero (no underflow).
    #[test]
    fn output_clamped_to_zero_on_loss_exceeding_collateral() {
        let w = setup();
        let entry_price = 2_000 * FP;
        let close_price = 1_000 * FP; // 50% drop — big loss
        configure_market(&w, 100); // 100 bps fee to make it even worse

        let size_usd = 1_000 * FP;
        let collateral = ONE_TOKEN; // tiny collateral relative to loss
        plant_position(&w, size_usd, collateral, entry_price);

        let market = make_market(&w);
        let price = PriceProps {
            min: close_price,
            max: close_price,
        };

        let result = w.env.as_contract(&w.caller, || {
            decrease_position(
                &w.env,
                &DecreasePositionParams {
                    data_store: &w.ds,
                    caller: &w.caller,
                    account: &w.user,
                    receiver: &w.user,
                    market: &market,
                    collateral_token: &w.long_tk,
                    size_delta_usd: size_usd,
                    acceptable_price: 0,
                    is_long: true,
                    index_token_price: &price,
                    collateral_price: close_price,
                    current_time: 2_000,
                    swap_path: Vec::new(&w.env),
                    oracle: &w.admin,
                },
            )
        });

        // Output must never go negative
        assert!(
            result.output_amount >= 0,
            "output_amount must not underflow; got {}",
            result.output_amount
        );
    }

    /// Partial close: claimable funding amount accumulates in DataStore for the position owner.
    #[test]
    fn partial_close_residual_claimable_funding_is_correct() {
        let w = setup();
        let index_price = 2_000 * FP;
        configure_market(&w, 10);

        // Seed a positive claimable-funding-per-size for the position owner's side.
        // A positive `tracker - latest` in settle_funding_fees means the position is owed funding.
        let ds_c = DsClient::new(&w.env, &w.ds);
        let fnd_key = gmx_keys::funding_amount_per_size_key(&w.env, &w.market_tk, &w.long_tk, true);
        // Set global funding-per-size to 0 (default).
        // The position's long_claim_fnd_per_size starts at 0 (from plant_position).
        // If we set the global to a NEGATIVE value, tracker(0) - latest(neg) = positive → owed.
        let funding_per_size: i128 = -(10 * FP / 1_000_000); // small negative → position is owed
        ds_c.apply_delta_to_i128(&w.admin, &fnd_key, &funding_per_size);

        let size_usd = 1_000 * FP;
        let collateral = ONE_TOKEN * 10;
        let pos = plant_position(&w, size_usd, collateral, index_price);

        // Manually set the position's tracker to 0 so delta = 0 - funding_per_size > 0
        // (plant_position already sets long_claim_fnd_per_size = 0, which is > latest negative)
        // So claimable_per_size = 0 - funding_per_size = 0 - neg = positive
        let _ = pos;

        let market = make_market(&w);
        let price = PriceProps {
            min: index_price,
            max: index_price,
        };
        let size_delta = size_usd / 2;

        w.env.as_contract(&w.caller, || {
            decrease_position(
                &w.env,
                &DecreasePositionParams {
                    data_store: &w.ds,
                    caller: &w.caller,
                    account: &w.user,
                    receiver: &w.user,
                    market: &market,
                    collateral_token: &w.long_tk,
                    size_delta_usd: size_delta,
                    acceptable_price: 0,
                    is_long: true,
                    index_token_price: &price,
                    collateral_price: index_price,
                    current_time: 2_000,
                    swap_path: Vec::new(&w.env),
                    oracle: &w.admin,
                },
            )
        });

        // The claimable funding amount for user should be positive
        let claim_key =
            gmx_keys::claimable_funding_amount_key(&w.env, &w.market_tk, &w.long_tk, &w.user);
        let claimable = ds_c.get_u128(&claim_key);
        assert!(
            claimable > 0,
            "claimable funding must be positive after partial close with owed funding"
        );
    }
}
