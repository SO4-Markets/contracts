//! Resource benchmark harness for CI regression reporting (#1064).
//!
//! Runs representative user and keeper flows and records the Soroban resources
//! metered for each top-level invocation: CPU instructions, memory, ledger
//! entry reads/writes, read/write bytes, contract-event size, and the simulated
//! fee (SDK pubnet fee snapshot).
//!
//! The numbers come from the Soroban cost model, not wall-clock time, so they
//! are deterministic for a given SDK version and code: the same tree produces
//! the same output on every machine, which is what makes trend comparison and
//! CI gating possible.
//!
//! Limitation: contracts are registered natively (`env.register`), so Wasm VM
//! instantiation/execution and code-entry reads are not included. Treat the
//! values as a stable relative signal, not absolute on-chain cost.
//!
//! Normal `cargo test` runs this as a test (it checks determinism and that
//! every scenario stays inside mainnet invocation limits). With
//! `RESOURCE_BENCH_OUT=<path>` set it also writes the results as JSON, which
//! `scripts/bench_compare.py` compares against `docs/benchmarks/resource-baseline.json`.
//! Run `make bench-resources` (compare) or `make bench-baseline RATIONALE=...` (update).

#![cfg(test)]

use std::collections::BTreeMap;

use data_store::{DataStore, DataStoreClient as DsClient};
use deposit_handler::{DepositHandler, DepositHandlerClient as DHClient};
use deposit_vault::{DepositVault, DepositVaultClient as DVClient};
use fee_handler::{FeeHandler, FeeHandlerClient as FHClient};
use gmx_keys::{market_index_token_key, market_long_token_key, market_short_token_key, roles};
use gmx_math::FLOAT_PRECISION;
use gmx_types::{
    CreateDepositParams, CreateOrderParams, CreateWithdrawalParams, OrderType, TokenPrice,
};
use market_token::{MarketToken, MarketTokenClient as MtClient};
use oracle::{Oracle, OracleClient as OClient};
use order_handler::{OrderHandler, OrderHandlerClient as OHClient, MAX_ORDER_BATCH_SIZE};
use order_vault::{OrderVault, OrderVaultClient as OVClient};
use role_store::{RoleStore, RoleStoreClient as RsClient};
use soroban_sdk::{testutils::Address as _, token::StellarAssetClient, Address, Env, Vec};
use withdrawal_handler::{WithdrawalHandler, WithdrawalHandlerClient as WHClient};
use withdrawal_vault::{WithdrawalVault, WithdrawalVaultClient as WVClient};

const ONE_TOKEN: i128 = 10_000_000;
const ONE_USD: i128 = FLOAT_PRECISION;

/// Bumped when the output format changes; the compare script rejects a mismatch.
const SCHEMA_VERSION: u32 = 1;

/// Mainnet per-invocation limits (soroban-sdk `InvocationResourceLimits::mainnet()`).
const MAINNET_INSTRUCTIONS: i64 = 600_000_000;
const MAINNET_DISK_READ_ENTRIES: u32 = 100;
const MAINNET_WRITE_ENTRIES: u32 = 50;
const MAINNET_EVENTS_BYTES: u32 = 16_384;

/// Resources metered for one top-level invocation.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Metrics {
    instructions: i64,
    mem_bytes: i64,
    disk_read_entries: u32,
    memory_read_entries: u32,
    write_entries: u32,
    disk_read_bytes: u32,
    write_bytes: u32,
    contract_events_size_bytes: u32,
    fee_total_stroops: i64,
}

impl Metrics {
    fn of_last_invocation(env: &Env) -> Self {
        let r = env.cost_estimate().resources();
        let fee = env.cost_estimate().fee();
        Metrics {
            instructions: r.instructions,
            mem_bytes: r.mem_bytes,
            disk_read_entries: r.disk_read_entries,
            memory_read_entries: r.memory_read_entries,
            write_entries: r.write_entries,
            disk_read_bytes: r.disk_read_bytes,
            write_bytes: r.write_bytes,
            contract_events_size_bytes: r.contract_events_size_bytes,
            fee_total_stroops: fee.total,
        }
    }

    fn fields(&self) -> [(&'static str, i64); 9] {
        [
            ("instructions", self.instructions),
            ("mem_bytes", self.mem_bytes),
            ("disk_read_entries", self.disk_read_entries as i64),
            ("memory_read_entries", self.memory_read_entries as i64),
            ("write_entries", self.write_entries as i64),
            ("disk_read_bytes", self.disk_read_bytes as i64),
            ("write_bytes", self.write_bytes as i64),
            (
                "contract_events_size_bytes",
                self.contract_events_size_bytes as i64,
            ),
            ("fee_total_stroops", self.fee_total_stroops),
        ]
    }
}

#[allow(dead_code)]
struct World {
    env: Env,
    admin: Address,
    keeper: Address,
    rs: Address,
    ds: Address,
    oracle: Address,
    ord_vault: Address,
    dep_handler: Address,
    wth_handler: Address,
    ord_handler: Address,
    market_tk: Address,
    long_tk: Address,
    short_tk: Address,
    index_tk: Address,
}

fn setup() -> World {
    let env = Env::default();
    env.mock_all_auths();
    env.cost_estimate().budget().reset_unlimited();

    let admin = Address::generate(&env);
    let keeper = Address::generate(&env);

    let rs = env.register(RoleStore, ());
    let rs_c = RsClient::new(&env, &rs);
    rs_c.initialize(&admin);
    rs_c.grant_role(&admin, &admin, &roles::controller(&env));
    rs_c.grant_role(&admin, &keeper, &roles::order_keeper(&env));

    let ds = env.register(DataStore, ());
    DsClient::new(&env, &ds).initialize(&admin, &rs);

    let oracle = env.register(Oracle, ());
    let passphrase = soroban_sdk::Bytes::from_slice(&env, b"Test SDF Network ; September 2015");
    OClient::new(&env, &oracle).initialize(&admin, &rs, &ds, &passphrase);

    let dep_vault = env.register(DepositVault, ());
    DVClient::new(&env, &dep_vault).initialize(&admin, &rs);
    let wth_vault = env.register(WithdrawalVault, ());
    WVClient::new(&env, &wth_vault).initialize(&admin, &rs);
    let ord_vault = env.register(OrderVault, ());
    OVClient::new(&env, &ord_vault).initialize(&admin, &rs);

    let long_tk = env
        .register_stellar_asset_contract_v2(admin.clone())
        .address();
    let short_tk = env
        .register_stellar_asset_contract_v2(admin.clone())
        .address();
    let index_tk = Address::generate(&env);

    let market_tk = env.register(MarketToken, ());
    MtClient::new(&env, &market_tk).initialize(
        &admin,
        &rs,
        &7u32,
        &soroban_sdk::String::from_str(&env, "GMX ETH/USD Market"),
        &soroban_sdk::String::from_str(&env, "GM"),
        &long_tk,
        &short_tk,
    );

    let dep_handler = env.register(DepositHandler, ());
    DHClient::new(&env, &dep_handler).initialize(&admin, &rs, &ds, &oracle, &dep_vault);
    let wth_handler = env.register(WithdrawalHandler, ());
    WHClient::new(&env, &wth_handler).initialize(&admin, &rs, &ds, &oracle, &wth_vault);
    let ord_handler = env.register(OrderHandler, ());
    OHClient::new(&env, &ord_handler).initialize(&admin, &rs, &ds, &oracle, &ord_vault);

    rs_c.grant_role(&admin, &dep_handler, &roles::controller(&env));
    rs_c.grant_role(&admin, &wth_handler, &roles::controller(&env));
    rs_c.grant_role(&admin, &ord_handler, &roles::controller(&env));

    let ds_c = DsClient::new(&env, &ds);
    ds_c.set_address(&admin, &market_index_token_key(&env, &market_tk), &index_tk);
    ds_c.set_address(&admin, &market_long_token_key(&env, &market_tk), &long_tk);
    ds_c.set_address(&admin, &market_short_token_key(&env, &market_tk), &short_tk);

    World {
        env,
        admin,
        keeper,
        rs,
        ds,
        oracle,
        ord_vault,
        dep_handler,
        wth_handler,
        ord_handler,
        market_tk,
        long_tk,
        short_tk,
        index_tk,
    }
}

fn set_prices(w: &World, eth_usd: i128) {
    OClient::new(&w.env, &w.oracle).set_prices_simple(
        &w.keeper,
        &Vec::from_array(
            &w.env,
            [
                TokenPrice {
                    token: w.long_tk.clone(),
                    min: eth_usd * ONE_USD,
                    max: eth_usd * ONE_USD,
                },
                TokenPrice {
                    token: w.short_tk.clone(),
                    min: ONE_USD,
                    max: ONE_USD,
                },
                TokenPrice {
                    token: w.index_tk.clone(),
                    min: eth_usd * ONE_USD,
                    max: eth_usd * ONE_USD,
                },
            ],
        ),
    );
}

fn order(
    w: &World,
    user: &Address,
    order_type: OrderType,
    size_usd: i128,
    collateral: i128,
) -> CreateOrderParams {
    CreateOrderParams {
        receiver: user.clone(),
        market: w.market_tk.clone(),
        initial_collateral_token: w.short_tk.clone(),
        swap_path: Vec::new(&w.env),
        size_delta_usd: size_usd,
        collateral_delta_amount: collateral,
        trigger_price: 0,
        acceptable_price: match order_type {
            OrderType::MarketIncrease => 2100 * ONE_USD,
            _ => 1900 * ONE_USD,
        },
        execution_fee: 0,
        min_output_amount: 0,
        order_type,
        is_long: true,
        expiry_ledger: None,
        on_behalf_of: None,
    }
}

/// Runs every scenario in a fresh environment and returns metrics keyed by scenario name.
fn run_scenarios() -> BTreeMap<&'static str, Metrics> {
    let mut out = BTreeMap::new();

    // ── Lifecycle: deposit → open long → close long → withdraw ────────────────
    let w = setup();
    let env = &w.env;
    let alice = Address::generate(env);
    let bob = Address::generate(env);
    StellarAssetClient::new(env, &w.long_tk).mint(&alice, &ONE_TOKEN);
    StellarAssetClient::new(env, &w.short_tk).mint(&alice, &(2000 * ONE_TOKEN));
    StellarAssetClient::new(env, &w.short_tk).mint(&bob, &(200 * ONE_TOKEN));
    set_prices(&w, 2000);

    let dh = DHClient::new(env, &w.dep_handler);
    let dep_key = dh.create_deposit(
        &alice,
        &CreateDepositParams {
            receiver: alice.clone(),
            market: w.market_tk.clone(),
            initial_long_token: w.long_tk.clone(),
            initial_short_token: w.short_tk.clone(),
            long_token_amount: ONE_TOKEN,
            short_token_amount: 2000 * ONE_TOKEN,
            min_market_tokens: 1,
            execution_fee: 0,
        },
    );
    out.insert("lifecycle/create_deposit", Metrics::of_last_invocation(env));
    dh.execute_deposit(&w.keeper, &dep_key);
    out.insert(
        "lifecycle/execute_deposit",
        Metrics::of_last_invocation(env),
    );

    let oh = OHClient::new(env, &w.ord_handler);
    StellarAssetClient::new(env, &w.short_tk).transfer(&bob, &w.ord_vault, &(200 * ONE_TOKEN));
    set_prices(&w, 2000);
    let open_key = oh.create_order(
        &bob,
        &order(
            &w,
            &bob,
            OrderType::MarketIncrease,
            2000 * ONE_USD,
            200 * ONE_TOKEN,
        ),
    );
    out.insert(
        "lifecycle/create_order_increase",
        Metrics::of_last_invocation(env),
    );
    oh.execute_order(&w.keeper, &open_key);
    out.insert(
        "lifecycle/execute_order_increase",
        Metrics::of_last_invocation(env),
    );

    set_prices(&w, 2000);
    let close_key = oh.create_order(
        &bob,
        &order(&w, &bob, OrderType::MarketDecrease, 2000 * ONE_USD, 0),
    );
    out.insert(
        "lifecycle/create_order_decrease",
        Metrics::of_last_invocation(env),
    );
    oh.execute_order(&w.keeper, &close_key);
    out.insert(
        "lifecycle/execute_order_decrease",
        Metrics::of_last_invocation(env),
    );

    set_prices(&w, 2000);
    let alice_gm = MtClient::new(env, &w.market_tk).balance(&alice);
    let wh = WHClient::new(env, &w.wth_handler);
    let wth_key = wh.create_withdrawal(
        &alice,
        &CreateWithdrawalParams {
            receiver: alice.clone(),
            market: w.market_tk.clone(),
            market_token_amount: alice_gm,
            min_long_token_amount: 0,
            min_short_token_amount: 0,
            execution_fee: 0,
        },
    );
    out.insert(
        "lifecycle/create_withdrawal",
        Metrics::of_last_invocation(env),
    );
    wh.execute_withdrawal(&w.keeper, &wth_key);
    out.insert(
        "lifecycle/execute_withdrawal",
        Metrics::of_last_invocation(env),
    );

    // ── Bound: create_orders at MAX_ORDER_BATCH_SIZE ───────────────────────────
    // Evidence for the batch cap: raising MAX_ORDER_BATCH_SIZE changes this
    // scenario's size, and the compare script refuses a larger constant until
    // the baseline is regenerated.
    let w = setup();
    let user = Address::generate(&w.env);
    let mut batch = Vec::new(&w.env);
    for _ in 0..MAX_ORDER_BATCH_SIZE {
        batch.push_back(order(
            &w,
            &user,
            OrderType::MarketDecrease,
            100 * ONE_USD,
            0,
        ));
    }
    OHClient::new(&w.env, &w.ord_handler).create_orders(&user, &batch);
    out.insert(
        "bounds/create_orders_max_batch",
        Metrics::of_last_invocation(&w.env),
    );

    // ── Fees: configure trusted recipients, then sweep to them ────────────────
    // Evidence for the constrained-recipient model: claim_fees now resolves the
    // per-market override and the protocol recipient before transferring.
    let w = setup();
    let env = &w.env;
    let fee_keeper = Address::generate(env);
    let rs_c = RsClient::new(env, &w.rs);
    rs_c.grant_role(&w.admin, &fee_keeper, &roles::fee_keeper(env));
    rs_c.grant_role(&w.admin, &w.market_tk, &roles::controller(env));
    let fh_id = env.register(FeeHandler, ());
    let fh = FHClient::new(env, &fh_id);
    fh.initialize(&w.admin, &w.rs, &w.ds);
    rs_c.grant_role(&w.admin, &fh_id, &roles::controller(env));

    fh.set_fee_receiver(&Address::generate(env));
    out.insert("fees/set_fee_receiver", Metrics::of_last_invocation(env));
    fh.set_market_fee_receiver(&w.market_tk, &Some(Address::generate(env)));
    out.insert(
        "fees/set_market_fee_receiver",
        Metrics::of_last_invocation(env),
    );

    let fees: u128 = ONE_TOKEN as u128;
    let ds_c = DsClient::new(env, &w.ds);
    let seed = || {
        ds_c.set_u128(
            &w.admin,
            &gmx_keys::claimable_fee_amount_key(env, &w.market_tk, &w.long_tk),
            &fees,
        );
        ds_c.apply_delta_to_u128(
            &w.admin,
            &gmx_keys::total_claimable_fee_amount_key(env, &w.long_tk),
            &(fees as i128),
        );
        ds_c.set_u128(
            &w.admin,
            &gmx_keys::pool_amount_key(env, &w.market_tk, &w.long_tk),
            &fees,
        );
        StellarAssetClient::new(env, &w.long_tk).mint(&w.market_tk, &(fees as i128));
    };
    seed();
    fh.claim_fees(&fee_keeper, &w.market_tk, &w.long_tk);
    out.insert("fees/claim_fees", Metrics::of_last_invocation(env));

    seed();
    fh.claim_token_fees(
        &fee_keeper,
        &w.long_tk,
        &soroban_sdk::Vec::from_array(env, [w.market_tk.clone()]),
    );
    out.insert("fees/claim_token_fees", Metrics::of_last_invocation(env));

    fh.reconcile_token_fees(
        &w.long_tk,
        &soroban_sdk::Vec::from_array(env, [w.market_tk.clone()]),
    );
    out.insert(
        "fees/reconcile_token_fees",
        Metrics::of_last_invocation(env),
    );

    out
}

fn to_json(results: &BTreeMap<&'static str, Metrics>) -> String {
    let mut s = String::new();
    s.push_str("{\n");
    s.push_str(&format!("  \"schema\": {SCHEMA_VERSION},\n"));
    s.push_str("  \"scenarios\": {\n");
    let n = results.len();
    for (i, (name, m)) in results.iter().enumerate() {
        s.push_str(&format!("    \"{name}\": {{"));
        let fields = m.fields();
        for (j, (k, v)) in fields.iter().enumerate() {
            let sep = if j + 1 < fields.len() { ", " } else { "" };
            s.push_str(&format!("\"{k}\": {v}{sep}"));
        }
        s.push_str(if i + 1 < n { "},\n" } else { "}\n" });
    }
    s.push_str("  }\n}\n");
    s
}

#[test]
fn resource_benchmarks_are_deterministic_and_within_mainnet_limits() {
    let first = run_scenarios();
    let second = run_scenarios();
    assert_eq!(
        first, second,
        "metering must be deterministic so CI can compare against a baseline"
    );
    assert!(!first.is_empty());

    for (name, m) in &first {
        assert!(m.instructions > 0, "{name}: no instructions metered");
        assert!(m.fee_total_stroops > 0, "{name}: no fee estimated");
        assert!(
            m.instructions <= MAINNET_INSTRUCTIONS,
            "{name}: over the mainnet instruction limit"
        );
        assert!(
            m.disk_read_entries <= MAINNET_DISK_READ_ENTRIES,
            "{name}: too many disk reads"
        );
        assert!(
            m.write_entries <= MAINNET_WRITE_ENTRIES,
            "{name}: too many writes"
        );
        assert!(
            m.contract_events_size_bytes <= MAINNET_EVENTS_BYTES,
            "{name}: events too large"
        );
    }

    let json = to_json(&first);
    eprintln!("{json}");
    if let Ok(path) = std::env::var("RESOURCE_BENCH_OUT") {
        std::fs::write(&path, &json).unwrap_or_else(|e| panic!("writing {path}: {e}"));
    }
}

#[test]
fn json_output_lists_every_metric_for_every_scenario() {
    let mut results = BTreeMap::new();
    results.insert(
        "x/y",
        Metrics {
            instructions: 1,
            mem_bytes: 2,
            disk_read_entries: 3,
            memory_read_entries: 4,
            write_entries: 5,
            disk_read_bytes: 6,
            write_bytes: 7,
            contract_events_size_bytes: 8,
            fee_total_stroops: 9,
        },
    );
    let json = to_json(&results);
    assert!(json.contains("\"schema\": 1"));
    for (k, v) in results["x/y"].fields() {
        assert!(
            json.contains(&format!("\"{k}\": {v}")),
            "missing {k} in {json}"
        );
    }
}
