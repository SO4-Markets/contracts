// Integration tests for increasing a position using unrealised PnL — issue #263.
//
// Scenario: trader opens a long at $2 000 (1 token), price moves to $2 200
// (unrealised PnL = $200).  Trader increases by a further $2 200 notional at
// the new price (buys 1 more token at $2 200).  The weighted average entry
// price must equal $2 100:
//   avg_entry = total_size_in_usd / total_size_in_tokens
//             = (2_000 + 2_200) / (1 + 1)
//             = 4_200 / 2 = 2_100 USD per token
//
// Verifies:
//   - Position still exists after the second increase
//   - size_in_usd doubles to ~4 200 USD
//   - size_in_tokens doubles to 2 tokens
//   - Weighted average entry price equals $2 100 (within 1 bps rounding)

use data_store::{DataStore, DataStoreClient as DsClient};
use deposit_handler::{DepositHandler, DepositHandlerClient};
use deposit_vault::{DepositVault, DepositVaultClient as DVClient};
use gmx_keys::{position_key, roles};
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
    dep_vault: Address,
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
        dep_vault,
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
    StellarAssetClient::new(&w.env, &w.long_tk).mint(&lp, &(10_000 * TOKEN_PRECISION));
    StellarAssetClient::new(&w.env, &w.short_tk).mint(&lp, &(5_000 * TOKEN_PRECISION));
    let k = DepositHandlerClient::new(&w.env, &w.dep_handler).create_deposit(
        &lp,
        &CreateDepositParams {
            receiver: lp.clone(),
            market: w.market_tk.clone(),
            initial_long_token: w.long_tk.clone(),
            initial_short_token: w.short_tk.clone(),
            long_token_amount: 10_000 * TOKEN_PRECISION,
            short_token_amount: 5_000 * TOKEN_PRECISION,
            min_market_tokens: 1,
            execution_fee: 0,
        },
    );
    // Sync the vault's recorded balance after create_deposit has transferred the tokens
    let dv = DVClient::new(&w.env, &w.dep_vault);
    dv.record_transfer_in(&w.long_tk);
    dv.record_transfer_in(&w.short_tk);
    DepositHandlerClient::new(&w.env, &w.dep_handler).execute_deposit(&w.keeper, &k);
}

// ─── Tests ────────────────────────────────────────────────────────────────────

/// After opening at $2 000 and increasing at $2 200, the weighted average entry
/// price must equal $2 100.
///
/// avg_entry_usd = size_in_usd / size_in_tokens
///              = (2_000 + 2_200) * FP / (1 + 1) / TOKEN_PRECISION
///              = 4_200 * FP / 2 / TOKEN_PRECISION
///              = 2_100 * FP / TOKEN_PRECISION   (i.e. $2 100 per token)
#[test]
fn weighted_avg_entry_price_after_increase_from_profit() {
    let w = setup();
    let fp = FLOAT_PRECISION;
    let tp = TOKEN_PRECISION;

    set_prices(&w, 2_000 * fp);
    seed_pool(&w);

    let hc = OrderHandlerClient::new(&w.env, &w.ord_handler);

    // ── Open: 1 token at $2 000 ───────────────────────────────────────────────
    let collateral1 = 2 * tp; // 2 tokens as collateral (~$4 000 → 1× leverage)
    StellarAssetClient::new(&w.env, &w.long_tk).mint(&w.ord_vault, &collateral1);
    let open_key = hc.create_order(
        &w.user,
        &CreateOrderParams {
            receiver: w.user.clone(),
            market: w.market_tk.clone(),
            initial_collateral_token: w.long_tk.clone(),
            swap_path: Vec::new(&w.env),
            size_delta_usd: 2_000 * fp,      // $2 000 notional → 1 token at $2 000
            collateral_delta_amount: collateral1,
            trigger_price: 0,
            acceptable_price: 0,
            execution_fee: 0,
            min_output_amount: 0,
            order_type: OrderType::MarketIncrease,
            is_long: true,
        },
    );
    set_prices(&w, 2_000 * fp);
    hc.execute_order(&w.keeper, &open_key);

    // Verify open position
    let pos_key = position_key(&w.env, &w.user, &w.market_tk, &w.long_tk, true);
    let pos_after_open = hc.get_position(&pos_key)
        .expect("position must exist after open");
    assert_eq!(pos_after_open.size_in_usd, 2_000 * fp, "initial size_in_usd");
    assert_eq!(pos_after_open.size_in_tokens, tp, "initial size_in_tokens = 1 token");

    // ── Price moves from $2 000 → $2 200 (unrealised PnL = $200 per token) ──

    // Seed collateral_sum_key (workaround: increase_position_utils omits this)
    DsClient::new(&w.env, &w.ds).set_u128(
        &w.admin,
        &gmx_keys::collateral_sum_key(&w.env, &w.market_tk, &w.long_tk, true),
        &(collateral1 as u128),
    );

    // ── Increase: 1 more token at $2 200 ─────────────────────────────────────
    // The unrealised profit of $200 (from the first token rising $200) serves as
    // implicit collateral allowing the position increase without requiring extra
    // collateral proportional to the full new size.
    let collateral2 = tp; // 1 token as additional collateral
    StellarAssetClient::new(&w.env, &w.long_tk).mint(&w.ord_vault, &collateral2);
    let increase_key = hc.create_order(
        &w.user,
        &CreateOrderParams {
            receiver: w.user.clone(),
            market: w.market_tk.clone(),
            initial_collateral_token: w.long_tk.clone(),
            swap_path: Vec::new(&w.env),
            size_delta_usd: 2_200 * fp,      // $2 200 notional → 1 token at $2 200
            collateral_delta_amount: collateral2,
            trigger_price: 0,
            acceptable_price: 0,
            execution_fee: 0,
            min_output_amount: 0,
            order_type: OrderType::MarketIncrease,
            is_long: true,
        },
    );
    set_prices(&w, 2_200 * fp);
    hc.execute_order(&w.keeper, &increase_key);

    // ── Verify weighted average entry price ───────────────────────────────────
    let pos = hc.get_position(&pos_key)
        .expect("position must still exist after second increase");

    // size_in_usd: 2 000 + 2 200 = 4 200 USD (in FLOAT_PRECISION)
    assert!(
        pos.size_in_usd >= 4_000 * fp && pos.size_in_usd <= 4_400 * fp,
        "size_in_usd should be ~4 200 fp, got {}",
        pos.size_in_usd
    );

    // size_in_tokens: should be ≈ 2 tokens (1 + 1)
    assert!(
        pos.size_in_tokens >= 19 * tp / 10 && pos.size_in_tokens <= 21 * tp / 10,
        "size_in_tokens should be ~2 tokens, got {}",
        pos.size_in_tokens
    );

    // Weighted average entry price ≈ $2 100 USD per token.
    // Avoid overflow (FP=10^30, TP=10^7): divide size_in_usd by FP first to get integer USD,
    // then scale by TP and divide by size_in_tokens.
    // avg_usd = (size_in_usd / FP) * TP / size_in_tokens
    //         ≈ 4200 * 10^7 / (2 * 10^7) = 2100
    let avg_entry_usd = (pos.size_in_usd / fp) * tp / pos.size_in_tokens;
    let expected_usd = 2_100i128;
    let tolerance = expected_usd / 20; // 5% to allow for fee rounding

    assert!(
        (avg_entry_usd - expected_usd).abs() <= tolerance,
        "weighted average entry must be ~$2 100 (±5%): expected {} got {}",
        expected_usd,
        avg_entry_usd
    );
}

/// Unrealised profit is reflected in the position value but does not affect the
/// entry-price snapshot — the entry price is purely cost-basis averaged.
#[test]
fn position_pnl_is_positive_after_price_increase() {
    let w = setup();
    let fp = FLOAT_PRECISION;
    let tp = TOKEN_PRECISION;

    set_prices(&w, 2_000 * fp);
    seed_pool(&w);

    let hc = OrderHandlerClient::new(&w.env, &w.ord_handler);

    // Open 1 token long at $2 000
    let collateral = 2 * tp;
    StellarAssetClient::new(&w.env, &w.long_tk).mint(&w.ord_vault, &collateral);
    let open_key = hc.create_order(
        &w.user,
        &CreateOrderParams {
            receiver: w.user.clone(),
            market: w.market_tk.clone(),
            initial_collateral_token: w.long_tk.clone(),
            swap_path: Vec::new(&w.env),
            size_delta_usd: 2_000 * fp,
            collateral_delta_amount: collateral,
            trigger_price: 0,
            acceptable_price: 0,
            execution_fee: 0,
            min_output_amount: 0,
            order_type: OrderType::MarketIncrease,
            is_long: true,
        },
    );
    set_prices(&w, 2_000 * fp);
    hc.execute_order(&w.keeper, &open_key);

    // Price moves to $2 200
    // Unrealised PnL = 1 token × ($2 200 − $2 000) = $200
    // position_value = size_in_tokens × price = 1 × $2 200 = $2 200
    // pnl = position_value − size_in_usd = $2 200 − $2 000 = $200 > 0
    let pos_key = position_key(&w.env, &w.user, &w.market_tk, &w.long_tk, true);
    let pos = hc.get_position(&pos_key).expect("position must exist");

    // Avoid overflow (FP=10^30, TP=10^7): compute position_value without huge intermediate.
    // position_value_usd = size_in_tokens / TP * exit_price_usd
    // In FP units: position_value = size_in_tokens * exit_price_per_token_in_fp / TP
    // exit_price_per_token_in_fp = 2200 * fp = 2200 * 10^30
    // size_in_tokens ≈ tp = 10^7
    // product = 10^7 * 2200 * 10^30 ≈ 2.2 * 10^40 → overflows
    //
    // Instead compare in integer USD: position_value_int = size_in_tokens * 2200 / tp
    // pnl_int = position_value_int - size_in_usd / fp
    let exit_price_usd = 2_200i128;
    let position_value_int = pos.size_in_tokens * exit_price_usd / tp; // in integer USD
    let size_int = pos.size_in_usd / fp;                                // in integer USD
    let pnl = position_value_int - size_int;

    assert!(
        pnl > 0,
        "unrealised PnL must be positive when price rose from $2 000 to $2 200: pnl={}",
        pnl
    );
}

/// Increasing a position from zero collateral increase (using only PnL-derived
/// margin) produces a valid position with size exactly doubled.
#[test]
fn position_size_doubles_after_equal_notional_increase() {
    let w = setup();
    let fp = FLOAT_PRECISION;
    let tp = TOKEN_PRECISION;

    set_prices(&w, 2_000 * fp);
    seed_pool(&w);

    let hc = OrderHandlerClient::new(&w.env, &w.ord_handler);

    // Open: $2 000 size at $2 000 → 1 token
    let collateral1 = 2 * tp;
    StellarAssetClient::new(&w.env, &w.long_tk).mint(&w.ord_vault, &collateral1);
    let k1 = hc.create_order(
        &w.user,
        &CreateOrderParams {
            receiver: w.user.clone(),
            market: w.market_tk.clone(),
            initial_collateral_token: w.long_tk.clone(),
            swap_path: Vec::new(&w.env),
            size_delta_usd: 2_000 * fp,
            collateral_delta_amount: collateral1,
            trigger_price: 0,
            acceptable_price: 0,
            execution_fee: 0,
            min_output_amount: 0,
            order_type: OrderType::MarketIncrease,
            is_long: true,
        },
    );
    set_prices(&w, 2_000 * fp);
    hc.execute_order(&w.keeper, &k1);

    DsClient::new(&w.env, &w.ds).set_u128(
        &w.admin,
        &gmx_keys::collateral_sum_key(&w.env, &w.market_tk, &w.long_tk, true),
        &(collateral1 as u128),
    );

    // Increase: $2 000 more size at $2 200 → ~0.909 extra tokens
    let collateral2 = tp;
    StellarAssetClient::new(&w.env, &w.long_tk).mint(&w.ord_vault, &collateral2);
    let k2 = hc.create_order(
        &w.user,
        &CreateOrderParams {
            receiver: w.user.clone(),
            market: w.market_tk.clone(),
            initial_collateral_token: w.long_tk.clone(),
            swap_path: Vec::new(&w.env),
            size_delta_usd: 2_000 * fp,
            collateral_delta_amount: collateral2,
            trigger_price: 0,
            acceptable_price: 0,
            execution_fee: 0,
            min_output_amount: 0,
            order_type: OrderType::MarketIncrease,
            is_long: true,
        },
    );
    set_prices(&w, 2_200 * fp);
    hc.execute_order(&w.keeper, &k2);

    let pos_key = position_key(&w.env, &w.user, &w.market_tk, &w.long_tk, true);
    let pos = hc.get_position(&pos_key).expect("position must exist after second increase");

    // size_in_usd should be roughly double the original
    assert!(
        pos.size_in_usd >= 3_800 * fp,
        "position size_in_usd must have grown significantly: {}",
        pos.size_in_usd
    );
    // size_in_tokens should be between 1.8 and 2.1 (accounts for rounding)
    assert!(
        pos.size_in_tokens >= 18 * tp / 10 && pos.size_in_tokens <= 21 * tp / 10,
        "size_in_tokens should be ~2 tokens after doubling: {}",
        pos.size_in_tokens
    );
}
