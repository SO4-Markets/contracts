//! Benchmarks for a full order execution cycle.
//!
//! Run with:
//!   cargo bench -p order-handler --bench order_execution
//!
//! Three scenarios are measured:
//!   1. create_order   — user creates a market-increase order
//!   2. execute_order  — keeper executes the order (full position open)
//!   3. full_cycle     — create_order + execute_order + market-decrease (close)
//!
//! Instruction counts from the Soroban budget are printed to stderr after each
//! group so they can be pasted into docs/performance.md.

use criterion::{criterion_group, criterion_main, BatchSize, Criterion};
use soroban_sdk::{
    testutils::Address as _,
    token::StellarAssetClient,
    Address, Bytes, Env, Vec,
};

use data_store::{DataStore, DataStoreClient};
use market_token::{MarketToken, MarketTokenClient};
use oracle::{Oracle, OracleClient};
use order_handler::{CreateOrderParams, OrderHandler, OrderHandlerClient};
use order_vault::{OrderVault, OrderVaultClient};
use role_store::{RoleStore, RoleStoreClient};

use gmx_keys::roles;
use gmx_types::{OrderType, SwapPath, TokenPrice};

const FLOAT_PRECISION: i128 = 1_000_000_000_000_000_000_000_000_000_000;
const TOKEN_PRECISION: i128 = 10_000_000;
const INDEX_PRICE: i128 = 2_000 * TOKEN_PRECISION;
const COLLATERAL_PRICE: i128 = TOKEN_PRECISION; // stable = $1
const COLLATERAL_AMOUNT: i128 = 100 * TOKEN_PRECISION; // 100 tokens
const SIZE_USD: i128 = 500 * TOKEN_PRECISION; // 5× leverage

struct World {
    env: Env,
    keeper: Address,
    handler: Address,
    ds: Address,
    market_tk: Address,
    long_tk: Address,
    short_tk: Address,
    index_tk: Address,
    oracle: Address,
}

fn setup() -> World {
    let env = Env::default();
    env.mock_all_auths();
    env.cost_estimate().budget().reset_unlimited();

    let admin = Address::generate(&env);
    let keeper = Address::generate(&env);

    let rs = env.register(RoleStore, ());
    RoleStoreClient::new(&env, &rs).initialize(&admin);
    let rs_c = RoleStoreClient::new(&env, &rs);
    rs_c.grant_role(&admin, &admin, &roles::controller(&env));
    rs_c.grant_role(&admin, &keeper, &roles::order_keeper(&env));

    let ds = env.register(DataStore, ());
    DataStoreClient::new(&env, &ds).initialize(&admin, &rs);

    let oracle_addr = env.register(Oracle, ());
    let passphrase = Bytes::from_slice(&env, b"Test SDF Network ; September 2015");
    OracleClient::new(&env, &oracle_addr).initialize(&admin, &rs, &ds, &passphrase);

    let vault = env.register(OrderVault, ());
    OrderVaultClient::new(&env, &vault).initialize(&admin, &rs);

    let market_tk = env.register(MarketToken, ());
    MarketTokenClient::new(&env, &market_tk).initialize(
        &admin,
        &rs,
        &7u32,
        &soroban_sdk::String::from_str(&env, "GMX Market Token"),
        &soroban_sdk::String::from_str(&env, "GM"),
    );

    let handler = env.register(OrderHandler, ());
    OrderHandlerClient::new(&env, &handler).initialize(
        &admin, &rs, &ds, &oracle_addr, &vault,
    );

    rs_c.grant_role(&admin, &handler, &roles::controller(&env));

    let long_tk = env.register_stellar_asset_contract_v2(admin.clone()).address();
    let short_tk = env.register_stellar_asset_contract_v2(admin.clone()).address();
    let index_tk = Address::generate(&env);

    let ds_c = DataStoreClient::new(&env, &ds);
    ds_c.set_address(&handler, &gmx_keys::market_index_token_key(&env, &market_tk), &index_tk);
    ds_c.set_address(&handler, &gmx_keys::market_long_token_key(&env, &market_tk), &long_tk);
    ds_c.set_address(&handler, &gmx_keys::market_short_token_key(&env, &market_tk), &short_tk);

    // max leverage: 50× (in FLOAT_PRECISION units)
    ds_c.set_u128(
        &handler,
        &gmx_keys::max_leverage_key(&env, &market_tk),
        &(50 * FLOAT_PRECISION as u128),
    );

    // oracle prices
    let prices = Vec::from_array(
        &env,
        [
            TokenPrice { token: index_tk.clone(), min: INDEX_PRICE, max: INDEX_PRICE },
            TokenPrice { token: long_tk.clone(),  min: TOKEN_PRECISION, max: TOKEN_PRECISION },
            TokenPrice { token: short_tk.clone(), min: TOKEN_PRECISION, max: TOKEN_PRECISION },
        ],
    );
    OracleClient::new(&env, &oracle_addr).set_prices_simple(&admin, &prices);

    World { env, keeper, handler, ds, market_tk, long_tk, short_tk, index_tk, oracle: oracle_addr }
}

fn do_create_order(w: &World, user: &Address) -> soroban_sdk::BytesN<32> {
    StellarAssetClient::new(&w.env, &w.long_tk).mint(user, &COLLATERAL_AMOUNT);
    OrderVaultClient::new(&w.env, &w.handler)
        .record_transfer_in(&w.long_tk); // simulate vault receipt

    OrderHandlerClient::new(&w.env, &w.handler).create_order(
        user,
        &CreateOrderParams {
            receiver: user.clone(),
            market: w.market_tk.clone(),
            initial_collateral_token: w.long_tk.clone(),
            swap_path: SwapPath::new(),
            size_delta_usd: SIZE_USD,
            collateral_delta_amount: COLLATERAL_AMOUNT,
            trigger_price: 0,
            acceptable_price: INDEX_PRICE * 2,
            execution_fee: 0,
            min_output_amount: 0,
            order_type: OrderType::MarketIncrease,
            is_long: true,
        },
    )
}

// ── Scenario 1: create_order ──────────────────────────────────────────────────

fn bench_create_order(c: &mut Criterion) {
    let mut group = c.benchmark_group("create_order");
    group.bench_function("market_increase", |b| {
        b.iter_batched(
            || {
                let w = setup();
                let user = Address::generate(&w.env);
                (w, user)
            },
            |(w, user)| {
                do_create_order(&w, &user)
            },
            BatchSize::SmallInput,
        )
    });

    // Print instruction count once for the docs table
    let w = setup();
    let user = Address::generate(&w.env);
    w.env.cost_estimate().budget().reset_default();
    do_create_order(&w, &user);
    let cpu = w.env.cost_estimate().budget().cpu_instruction_cost();
    let mem = w.env.cost_estimate().budget().memory_bytes_cost();
    eprintln!("\n[bench] create_order  cpu={cpu}  mem={mem}");

    group.finish();
}

// ── Scenario 2: execute_order ─────────────────────────────────────────────────

fn bench_execute_order(c: &mut Criterion) {
    let mut group = c.benchmark_group("execute_order");
    group.bench_function("market_increase", |b| {
        b.iter_batched(
            || {
                let w = setup();
                let user = Address::generate(&w.env);
                let key = do_create_order(&w, &user);
                (w, key)
            },
            |(w, key)| {
                OrderHandlerClient::new(&w.env, &w.handler)
                    .execute_order(&w.keeper, &key)
            },
            BatchSize::SmallInput,
        )
    });

    let w = setup();
    let user = Address::generate(&w.env);
    let key = do_create_order(&w, &user);
    w.env.cost_estimate().budget().reset_default();
    OrderHandlerClient::new(&w.env, &w.handler).execute_order(&w.keeper, &key);
    let cpu = w.env.cost_estimate().budget().cpu_instruction_cost();
    let mem = w.env.cost_estimate().budget().memory_bytes_cost();
    eprintln!("\n[bench] execute_order cpu={cpu}  mem={mem}");

    group.finish();
}

// ── Scenario 3: full_cycle (create + execute + decrease/close) ────────────────

fn bench_full_cycle(c: &mut Criterion) {
    let mut group = c.benchmark_group("full_cycle");
    group.bench_function("open_then_close", |b| {
        b.iter_batched(
            setup,
            |w| {
                let user = Address::generate(&w.env);
                // Open
                let key = do_create_order(&w, &user);
                OrderHandlerClient::new(&w.env, &w.handler).execute_order(&w.keeper, &key);

                // Close via market decrease
                let close_key = OrderHandlerClient::new(&w.env, &w.handler).create_order(
                    &user,
                    &CreateOrderParams {
                        receiver: user.clone(),
                        market: w.market_tk.clone(),
                        initial_collateral_token: w.long_tk.clone(),
                        swap_path: SwapPath::new(),
                        size_delta_usd: SIZE_USD,
                        collateral_delta_amount: 0,
                        trigger_price: 0,
                        acceptable_price: 0,
                        execution_fee: 0,
                        min_output_amount: 0,
                        order_type: OrderType::MarketDecrease,
                        is_long: true,
                    },
                );
                OrderHandlerClient::new(&w.env, &w.handler)
                    .execute_order(&w.keeper, &close_key)
            },
            BatchSize::SmallInput,
        )
    });

    let w = setup();
    let user = Address::generate(&w.env);
    let key = do_create_order(&w, &user);
    OrderHandlerClient::new(&w.env, &w.handler).execute_order(&w.keeper, &key);
    let close_key = OrderHandlerClient::new(&w.env, &w.handler).create_order(
        &user,
        &CreateOrderParams {
            receiver: user.clone(),
            market: w.market_tk.clone(),
            initial_collateral_token: w.long_tk.clone(),
            swap_path: SwapPath::new(),
            size_delta_usd: SIZE_USD,
            collateral_delta_amount: 0,
            trigger_price: 0,
            acceptable_price: 0,
            execution_fee: 0,
            min_output_amount: 0,
            order_type: OrderType::MarketDecrease,
            is_long: true,
        },
    );
    w.env.cost_estimate().budget().reset_default();
    OrderHandlerClient::new(&w.env, &w.handler).execute_order(&w.keeper, &close_key);
    let cpu = w.env.cost_estimate().budget().cpu_instruction_cost();
    let mem = w.env.cost_estimate().budget().memory_bytes_cost();
    eprintln!("\n[bench] full_cycle    cpu={cpu}  mem={mem}");

    group.finish();
}

criterion_group!(benches, bench_create_order, bench_execute_order, bench_full_cycle);
criterion_main!(benches);
