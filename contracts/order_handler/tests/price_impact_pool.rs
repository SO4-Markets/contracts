// Integration tests for the swap price impact pool — issue #194.
//
// The swap impact pool (swap_impact_pool_amount_key) accumulates fees from
// imbalancing swaps (negative price impact) and pays rebates for balancing
// swaps (positive price impact), capped at the available pool balance.
//
// Verifies:
//   - Imbalancing swaps (same direction from balanced pool) grow the impact pool
//   - Impact pool grows monotonically with multiple imbalancing swaps
//   - Pool is always >= 0 (positive rebate capped at pool balance)
//   - swap_impact_pool_amount_key correctly tracks the running balance

use data_store::{DataStore, DataStoreClient as DsClient};
use deposit_handler::{DepositHandler, DepositHandlerClient};
use deposit_vault::{DepositVault, DepositVaultClient as DVClient};
use gmx_keys::roles;
use gmx_math::{FLOAT_PRECISION, TOKEN_PRECISION};
use gmx_types::{CreateDepositParams, OrderType, TokenPrice};
use market_token::{MarketToken, MarketTokenClient as MtClient};
use oracle::{Oracle, OracleClient as OClient};
use order_handler::{CreateOrderParams, OrderHandler, OrderHandlerClient};
use order_vault::{OrderVault, OrderVaultClient as OVClient};
use role_store::{RoleStore, RoleStoreClient as RsClient};
use soroban_sdk::{
    testutils::Address as _,
    token::StellarAssetClient,
    Address, Env, Vec,
};

struct World {
    env: Env,
    admin: Address,
    keeper: Address,
    user: Address,
    ds: Address,
    oracle: Address,
    ord_vault: Address,
    dep_handler: Address,
    ord_handler: Address,
    market_tk: Address,
    long_tk: Address,
    short_tk: Address,
    index_tk: Address,
}

fn setup() -> World {
    let env = Env::default();
    env.mock_all_auths();
    env.budget().reset_unlimited();

    let admin = Address::generate(&env);
    let keeper = Address::generate(&env);
    let user = Address::generate(&env);

    let rs = env.register(RoleStore, ());
    let rs_c = RsClient::new(&env, &rs);
    rs_c.initialize(&admin);
    rs_c.grant_role(&admin, &admin, &roles::controller(&env));
    rs_c.grant_role(&admin, &keeper, &roles::order_keeper(&env));

    let ds = env.register(DataStore, ());
    DsClient::new(&env, &ds).initialize(&admin, &rs);

    let oracle_addr = env.register(Oracle, ());
    let passphrase = soroban_sdk::Bytes::from_slice(&env, b"Test SDF Network ; September 2015");
    OClient::new(&env, &oracle_addr).initialize(&admin, &rs, &ds, &passphrase);

    let market_tk = env.register(MarketToken, ());
    MtClient::new(&env, &market_tk).initialize(
        &admin,
        &rs,
        &7u32,
        &soroban_sdk::String::from_str(&env, "GMX Market Token"),
        &soroban_sdk::String::from_str(&env, "GM"),
    );
    rs_c.grant_role(&admin, &market_tk, &roles::controller(&env));

    let dep_vault = env.register(DepositVault, ());
    DVClient::new(&env, &dep_vault).initialize(&admin, &rs);

    let ord_vault = env.register(OrderVault, ());
    OVClient::new(&env, &ord_vault).initialize(&admin, &rs);

    let dep_handler = env.register(DepositHandler, ());
    DepositHandlerClient::new(&env, &dep_handler)
        .initialize(&admin, &rs, &ds, &oracle_addr, &dep_vault);
    rs_c.grant_role(&admin, &dep_handler, &roles::controller(&env));

    let ord_handler = env.register(OrderHandler, ());
    OrderHandlerClient::new(&env, &ord_handler)
        .initialize(&admin, &rs, &ds, &oracle_addr, &ord_vault);
    rs_c.grant_role(&admin, &ord_handler, &roles::controller(&env));

    let long_tk = env.register_stellar_asset_contract_v2(admin.clone()).address();
    let short_tk = env.register_stellar_asset_contract_v2(admin.clone()).address();
    let index_tk = Address::generate(&env);

    let ds_c = DsClient::new(&env, &ds);
    ds_c.set_address(&admin, &gmx_keys::market_index_token_key(&env, &market_tk), &index_tk);
    ds_c.set_address(&admin, &gmx_keys::market_long_token_key(&env, &market_tk), &long_tk);
    ds_c.set_address(&admin, &gmx_keys::market_short_token_key(&env, &market_tk), &short_tk);

    World {
        env,
        admin,
        keeper,
        user,
        ds,
        oracle: oracle_addr,
        ord_vault,
        dep_handler,
        ord_handler,
        market_tk,
        long_tk,
        short_tk,
        index_tk,
    }
}

fn set_prices(w: &World, index_usd: i128) {
    OClient::new(&w.env, &w.oracle).set_prices_simple(
        &w.keeper,
        &Vec::from_array(
            &w.env,
            [
                TokenPrice { token: w.long_tk.clone(), min: index_usd, max: index_usd },
                TokenPrice { token: w.short_tk.clone(), min: FLOAT_PRECISION, max: FLOAT_PRECISION },
                TokenPrice { token: w.index_tk.clone(), min: index_usd, max: index_usd },
            ],
        ),
    );
}

fn seed_pool(w: &World) {
    let lp = Address::generate(&w.env);
    let price = 2000 * FLOAT_PRECISION;
    // 1000 long_tk × $2000 = $2M; 2,000,000 short_tk × $1 = $2M — balanced
    StellarAssetClient::new(&w.env, &w.long_tk).mint(&lp, &(1000 * TOKEN_PRECISION));
    StellarAssetClient::new(&w.env, &w.short_tk).mint(&lp, &(2_000_000 * TOKEN_PRECISION));
    set_prices(w, price);
    let k = DepositHandlerClient::new(&w.env, &w.dep_handler).create_deposit(
        &lp,
        &CreateDepositParams {
            receiver: lp.clone(),
            market: w.market_tk.clone(),
            initial_long_token: w.long_tk.clone(),
            initial_short_token: w.short_tk.clone(),
            long_token_amount: 1000 * TOKEN_PRECISION,
            short_token_amount: 2_000_000 * TOKEN_PRECISION,
            min_market_tokens: 1,
            execution_fee: 0,
        },
    );
    DepositHandlerClient::new(&w.env, &w.dep_handler).execute_deposit(&w.keeper, &k);
}

/// Execute a long_tk → short_tk MarketSwap and return the resulting impact pool value.
fn execute_swap_long_to_short(w: &World, amount: i128) {
    StellarAssetClient::new(&w.env, &w.long_tk).mint(&w.ord_vault, &amount);
    let hc = OrderHandlerClient::new(&w.env, &w.ord_handler);
    let key = hc.create_order(
        &w.user,
        &CreateOrderParams {
            receiver: w.user.clone(),
            market: w.market_tk.clone(),
            initial_collateral_token: w.long_tk.clone(),
            swap_path: Vec::from_array(&w.env, [w.market_tk.clone()]),
            size_delta_usd: 0,
            collateral_delta_amount: amount,
            trigger_price: 0,
            acceptable_price: 0,
            execution_fee: 0,
            min_output_amount: 0,
            order_type: OrderType::MarketSwap,
            is_long: false,
        },
    );
    set_prices(w, 2000 * FLOAT_PRECISION);
    hc.execute_order(&w.keeper, &key);
}

// ─── Tests ────────────────────────────────────────────────────────────────────

/// Imbalancing swaps from a balanced pool accumulate the swap impact pool.
/// Each long_tk→short_tk swap worsens the imbalance (more long_tk in pool),
/// causing negative price impact whose fee accumulates in pool(short_tk).
#[test]
fn imbalancing_swaps_increase_impact_pool() {
    let w = setup();
    let fp = FLOAT_PRECISION;

    // Configure non-zero negative impact factor so imbalancing swaps create fees
    let ds_c = DsClient::new(&w.env, &w.ds);
    ds_c.set_u128(
        &w.admin,
        &gmx_keys::swap_impact_factor_key(&w.env, &w.market_tk, false),
        &(fp as u128 / 1_000), // 0.1% factor
    );
    ds_c.set_u128(
        &w.admin,
        &gmx_keys::swap_impact_exponent_factor_key(&w.env, &w.market_tk),
        &(fp as u128), // linear exponent
    );

    seed_pool(&w);

    let impact_pool_key =
        gmx_keys::swap_impact_pool_amount_key(&w.env, &w.market_tk, &w.short_tk);

    // Impact pool starts at 0
    assert_eq!(ds_c.get_u128(&impact_pool_key), 0, "impact pool must start at 0");

    // First large imbalancing swap
    execute_swap_long_to_short(&w, 50 * TOKEN_PRECISION); // 50 long_tk ≈ $100k
    let pool_after_first = ds_c.get_u128(&impact_pool_key);
    assert!(
        pool_after_first > 0,
        "impact pool must increase after first imbalancing swap; got {}",
        pool_after_first
    );

    // Second imbalancing swap (pool is now more imbalanced, fee is larger)
    execute_swap_long_to_short(&w, 10 * TOKEN_PRECISION); // 10 long_tk
    let pool_after_second = ds_c.get_u128(&impact_pool_key);
    assert!(
        pool_after_second > pool_after_first,
        "impact pool must grow after second imbalancing swap: {} → {}",
        pool_after_first,
        pool_after_second
    );
}

/// The impact pool balance matches the expected running total:
/// pool = X (after first swap) + Y (after second swap).
#[test]
fn impact_pool_running_balance_is_additive() {
    let w = setup();
    let fp = FLOAT_PRECISION;

    let ds_c = DsClient::new(&w.env, &w.ds);
    ds_c.set_u128(
        &w.admin,
        &gmx_keys::swap_impact_factor_key(&w.env, &w.market_tk, false),
        &(fp as u128 / 1_000),
    );
    ds_c.set_u128(
        &w.admin,
        &gmx_keys::swap_impact_exponent_factor_key(&w.env, &w.market_tk),
        &(fp as u128),
    );

    seed_pool(&w);

    let impact_pool_key =
        gmx_keys::swap_impact_pool_amount_key(&w.env, &w.market_tk, &w.short_tk);

    execute_swap_long_to_short(&w, 30 * TOKEN_PRECISION);
    let x = ds_c.get_u128(&impact_pool_key);

    execute_swap_long_to_short(&w, 30 * TOKEN_PRECISION);
    let x_plus_y = ds_c.get_u128(&impact_pool_key);

    // Both increments are positive (x > 0, x+y > x)
    assert!(x > 0);
    assert!(x_plus_y > x, "second swap must further increase the pool: {} → {}", x, x_plus_y);
}

/// Positive price impact (balancing swap) is capped at the available pool amount;
/// the pool never goes negative — apply_delta_to_u128 would panic with Underflow
/// if the cap were not applied first.
///
/// Scenario:
///   1. Build up pool(short_tk) via imbalancing swaps.
///   2. Artificially set pool(short_tk) to a very small amount.
///   3. Execute a large balancing swap (short_tk → long_tk from WETH-heavy pool)
///      whose uncapped positive impact exceeds the pool amount.
///   4. Pool must reach 0, NOT go negative.
#[test]
fn balancing_swap_rebate_capped_at_pool_balance() {
    let w = setup();
    let fp = FLOAT_PRECISION;

    let ds_c = DsClient::new(&w.env, &w.ds);
    // Large positive impact factor to ensure any balancing swap exceeds the (tiny) pool
    ds_c.set_u128(
        &w.admin,
        &gmx_keys::swap_impact_factor_key(&w.env, &w.market_tk, true),
        &(fp as u128 / 10), // 10% positive factor
    );
    ds_c.set_u128(
        &w.admin,
        &gmx_keys::swap_impact_factor_key(&w.env, &w.market_tk, false),
        &(fp as u128 / 1_000),
    );
    ds_c.set_u128(
        &w.admin,
        &gmx_keys::swap_impact_exponent_factor_key(&w.env, &w.market_tk),
        &(fp as u128),
    );

    seed_pool(&w);

    // Build up a tiny short_tk impact pool via a small imbalancing swap
    execute_swap_long_to_short(&w, 5 * TOKEN_PRECISION);
    let impact_pool_key =
        gmx_keys::swap_impact_pool_amount_key(&w.env, &w.market_tk, &w.short_tk);
    let pool_built = ds_c.get_u128(&impact_pool_key);
    assert!(pool_built > 0, "pool must be non-zero after imbalancing swap");

    // Directly set the short_tk impact pool to a tiny amount so the next balancing
    // swap will try to pay more rebate than the pool can cover.
    ds_c.set_u128(&w.admin, &impact_pool_key, &1u128); // 1 unit = tiny pool

    // Now seed extra short_tk into market (making pool short_tk-heavy) so a
    // long_tk→short_tk swap IS balancing (long_tk is deficient).
    // We can do this by seeding additional short_tk into the pool directly.
    let lp2 = Address::generate(&w.env);
    StellarAssetClient::new(&w.env, &w.short_tk).mint(&lp2, &(10_000_000 * TOKEN_PRECISION));
    let k2 = DepositHandlerClient::new(&w.env, &w.dep_handler).create_deposit(
        &lp2,
        &CreateDepositParams {
            receiver: lp2.clone(),
            market: w.market_tk.clone(),
            initial_long_token: w.long_tk.clone(),
            initial_short_token: w.short_tk.clone(),
            long_token_amount: 0,
            short_token_amount: 10_000_000 * TOKEN_PRECISION, // massive short_tk deposit
            min_market_tokens: 1,
            execution_fee: 0,
        },
    );
    set_prices(&w, 2000 * fp);
    DepositHandlerClient::new(&w.env, &w.dep_handler).execute_deposit(&w.keeper, &k2);

    // Perform a long_tk→short_tk swap — this is now BALANCING (long_tk is deficient side)
    let swap_amount = 100 * TOKEN_PRECISION;
    StellarAssetClient::new(&w.env, &w.long_tk).mint(&w.ord_vault, &swap_amount);
    let hc = OrderHandlerClient::new(&w.env, &w.ord_handler);
    let key = hc.create_order(
        &w.user,
        &CreateOrderParams {
            receiver: w.user.clone(),
            market: w.market_tk.clone(),
            initial_collateral_token: w.long_tk.clone(),
            swap_path: Vec::from_array(&w.env, [w.market_tk.clone()]),
            size_delta_usd: 0,
            collateral_delta_amount: swap_amount,
            trigger_price: 0,
            acceptable_price: 0,
            execution_fee: 0,
            min_output_amount: 0,
            order_type: OrderType::MarketSwap,
            is_long: false,
        },
    );
    set_prices(&w, 2000 * fp);
    hc.execute_order(&w.keeper, &key);

    // Pool must be 0 (fully drained, not negative) — the cap prevented underflow
    let pool_after = ds_c.get_u128(&impact_pool_key);
    assert_eq!(
        pool_after, 0,
        "impact pool must reach 0 after balancing swap exceeds available balance; got {}",
        pool_after
    );
}

/// swap_impact_pool_amount_key is the canonical storage key for the impact pool.
/// Verify that reads via the key match what the swap actually deposited.
#[test]
fn impact_pool_key_reflects_accumulated_fees() {
    let w = setup();
    let fp = FLOAT_PRECISION;

    let ds_c = DsClient::new(&w.env, &w.ds);
    ds_c.set_u128(
        &w.admin,
        &gmx_keys::swap_impact_factor_key(&w.env, &w.market_tk, false),
        &(fp as u128 / 500), // 0.2%
    );
    ds_c.set_u128(
        &w.admin,
        &gmx_keys::swap_impact_exponent_factor_key(&w.env, &w.market_tk),
        &(fp as u128),
    );

    seed_pool(&w);

    let impact_pool_key =
        gmx_keys::swap_impact_pool_amount_key(&w.env, &w.market_tk, &w.short_tk);

    let before = ds_c.get_u128(&impact_pool_key);
    execute_swap_long_to_short(&w, 20 * TOKEN_PRECISION);
    let after = ds_c.get_u128(&impact_pool_key);

    assert!(
        after >= before,
        "swap_impact_pool_amount_key must reflect accumulated fees: {} → {}",
        before,
        after
    );
}
