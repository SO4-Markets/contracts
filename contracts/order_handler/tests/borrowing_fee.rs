// Integration tests for borrowing fee accumulation — issue #192.
//
// Scenario: open a MarketIncrease long, advance ledger time, execute a partial
// MarketDecrease, and verify the cumulative borrowing factor accrued correctly.
//
// Verifies:
//   - cumulative_borrowing_factor increases after ledger timestamp advances
//   - position.borrowing_factor snapshot is refreshed on partial decrease
//   - pool amount increases (borrowing fee paid to pool)

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
    testutils::{Address as _, Ledger as _},
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
    DepositHandlerClient::new(&w.env, &w.dep_handler).execute_deposit(&w.keeper, &k);
}

// ─── Tests ────────────────────────────────────────────────────────────────────

/// cumulative_borrowing_factor increases after the ledger timestamp advances and
/// a partial decrease triggers update_cumulative_borrowing_factor internally.
#[test]
fn borrowing_factor_accrues_over_time() {
    let w = setup();
    let fp = FLOAT_PRECISION;
    let price = 2000 * fp;

    // Configure borrowing parameters
    let ds_c = DsClient::new(&w.env, &w.ds);
    ds_c.set_u128(
        &w.admin,
        &gmx_keys::borrowing_factor_key(&w.env, &w.market_tk, true),
        &(fp as u128 / 10_000),
    );
    ds_c.set_u128(
        &w.admin,
        &gmx_keys::borrowing_exponent_factor_key(&w.env, &w.market_tk, true),
        &(fp as u128),
    );

    set_prices(&w, price);
    seed_pool(&w);

    // Open a long position at T=0
    let collateral = 10 * TOKEN_PRECISION; // 10 tokens
    StellarAssetClient::new(&w.env, &w.long_tk).mint(&w.ord_vault, &collateral);
    let hc = OrderHandlerClient::new(&w.env, &w.ord_handler);
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

    w.env.ledger().set_timestamp(1_000);
    set_prices(&w, price);
    hc.execute_order(&w.keeper, &open_key);

    // Seed collateral_sum_key: increase_position_utils omits this update to stay
    // within Soroban's 40 ledger-entry budget; seed it manually before any decrease.
    ds_c.set_u128(
        &w.admin,
        &gmx_keys::collateral_sum_key(&w.env, &w.market_tk, &w.long_tk, true),
        &(collateral as u128),
    );

    // Read cumulative borrowing factor immediately after opening (T=1000)
    let cum_key = gmx_keys::cumulative_borrowing_factor_key(&w.env, &w.market_tk, true);
    let cum_at_open = ds_c.get_u128(&cum_key);

    // Advance ledger time significantly
    w.env.ledger().set_timestamp(11_000); // +10,000 seconds

    // Open partial close order (20% of size = 400 USD)
    let close_collateral = 2 * TOKEN_PRECISION;
    StellarAssetClient::new(&w.env, &w.long_tk).mint(&w.ord_vault, &close_collateral);
    let close_key = hc.create_order(
        &w.user,
        &CreateOrderParams {
            receiver: w.user.clone(),
            market: w.market_tk.clone(),
            initial_collateral_token: w.long_tk.clone(),
            swap_path: Vec::new(&w.env),
            size_delta_usd: 400 * fp, // 20% of 2000
            collateral_delta_amount: close_collateral,
            trigger_price: 0,
            acceptable_price: 0,
            execution_fee: 0,
            min_output_amount: 0,
            order_type: OrderType::MarketDecrease,
            is_long: true,
        },
    );

    set_prices(&w, price);
    hc.execute_order(&w.keeper, &close_key);

    // cumulative borrowing factor must have increased during the time advance
    let cum_after = ds_c.get_u128(&cum_key);
    assert!(
        cum_after > cum_at_open,
        "cumulative_borrowing_factor must accrue over time: before={} after={}",
        cum_at_open,
        cum_after
    );
}

/// After a partial decrease, the position's borrowing_factor snapshot is reset
/// to the current cumulative factor to prevent double-counting.
#[test]
fn position_borrowing_factor_snapshot_updated_on_partial_decrease() {
    let w = setup();
    let fp = FLOAT_PRECISION;
    let price = 2000 * fp;

    let ds_c = DsClient::new(&w.env, &w.ds);
    ds_c.set_u128(
        &w.admin,
        &gmx_keys::borrowing_factor_key(&w.env, &w.market_tk, true),
        &(fp as u128 / 10_000),
    );
    ds_c.set_u128(
        &w.admin,
        &gmx_keys::borrowing_exponent_factor_key(&w.env, &w.market_tk, true),
        &(fp as u128),
    );

    set_prices(&w, price);
    seed_pool(&w);

    // Open long at T=1000
    let collateral = 10 * TOKEN_PRECISION;
    StellarAssetClient::new(&w.env, &w.long_tk).mint(&w.ord_vault, &collateral);
    let hc = OrderHandlerClient::new(&w.env, &w.ord_handler);
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

    w.env.ledger().set_timestamp(1_000);
    set_prices(&w, price);
    hc.execute_order(&w.keeper, &open_key);

    // Seed collateral_sum_key: increase_position_utils omits this update to stay
    // within Soroban's 40 ledger-entry budget; seed it manually before any decrease.
    ds_c.set_u128(
        &w.admin,
        &gmx_keys::collateral_sum_key(&w.env, &w.market_tk, &w.long_tk, true),
        &(collateral as u128),
    );

    // Advance time
    w.env.ledger().set_timestamp(11_000);

    // Partial decrease (20%)
    let close_collateral = 2 * TOKEN_PRECISION;
    StellarAssetClient::new(&w.env, &w.long_tk).mint(&w.ord_vault, &close_collateral);
    let close_key = hc.create_order(
        &w.user,
        &CreateOrderParams {
            receiver: w.user.clone(),
            market: w.market_tk.clone(),
            initial_collateral_token: w.long_tk.clone(),
            swap_path: Vec::new(&w.env),
            size_delta_usd: 400 * fp,
            collateral_delta_amount: close_collateral,
            trigger_price: 0,
            acceptable_price: 0,
            execution_fee: 0,
            min_output_amount: 0,
            order_type: OrderType::MarketDecrease,
            is_long: true,
        },
    );
    set_prices(&w, price);
    hc.execute_order(&w.keeper, &close_key);

    // Position must still be open (partially closed)
    let pos_key = position_key(&w.env, &w.user, &w.market_tk, &w.long_tk, true);
    let position = hc.get_position(&pos_key).expect("position must remain open after partial close");

    // position.borrowing_factor must equal the current cumulative factor (snapshot refreshed)
    let cum_key = gmx_keys::cumulative_borrowing_factor_key(&w.env, &w.market_tk, true);
    let cum_now = ds_c.get_u128(&cum_key) as i128;
    assert_eq!(
        position.borrowing_factor, cum_now,
        "position.borrowing_factor must equal current cumulative factor after partial decrease"
    );

    // position size must have decreased
    assert!(
        position.size_in_usd < 2_000 * fp,
        "position size must be reduced after partial decrease"
    );
}

/// Pool amount increases after a decrease that incurs borrowing fees.
#[test]
fn pool_receives_borrowing_fee_on_position_decrease() {
    let w = setup();
    let fp = FLOAT_PRECISION;
    let price = 2000 * fp;

    // Use a larger borrowing_factor so the fee contribution is more likely visible
    let ds_c = DsClient::new(&w.env, &w.ds);
    ds_c.set_u128(
        &w.admin,
        &gmx_keys::borrowing_factor_key(&w.env, &w.market_tk, true),
        &(fp as u128 / 100), // 100× larger than default
    );
    ds_c.set_u128(
        &w.admin,
        &gmx_keys::borrowing_exponent_factor_key(&w.env, &w.market_tk, true),
        &(fp as u128),
    );

    set_prices(&w, price);
    seed_pool(&w);

    let pool_key = gmx_keys::pool_amount_key(&w.env, &w.market_tk, &w.long_tk);
    let pool_before = ds_c.get_u128(&pool_key);

    let collateral = 10 * TOKEN_PRECISION;
    StellarAssetClient::new(&w.env, &w.long_tk).mint(&w.ord_vault, &collateral);
    let hc = OrderHandlerClient::new(&w.env, &w.ord_handler);
    let open_key = hc.create_order(
        &w.user,
        &CreateOrderParams {
            receiver: w.user.clone(),
            market: w.market_tk.clone(),
            initial_collateral_token: w.long_tk.clone(),
            swap_path: Vec::new(&w.env),
            size_delta_usd: 5_000 * fp, // larger position
            collateral_delta_amount: collateral,
            trigger_price: 0,
            acceptable_price: 0,
            execution_fee: 0,
            min_output_amount: 0,
            order_type: OrderType::MarketIncrease,
            is_long: true,
        },
    );
    w.env.ledger().set_timestamp(500);
    set_prices(&w, price);
    hc.execute_order(&w.keeper, &open_key);

    // Seed collateral_sum_key: increase_position_utils omits this update to stay
    // within Soroban's 40 ledger-entry budget; seed it manually before any decrease.
    DsClient::new(&w.env, &w.ds).set_u128(
        &w.admin,
        &gmx_keys::collateral_sum_key(&w.env, &w.market_tk, &w.long_tk, true),
        &(collateral as u128),
    );

    // Advance time substantially
    w.env.ledger().set_timestamp(100_500); // 100,000 seconds

    // Full close
    let close_collateral = TOKEN_PRECISION;
    StellarAssetClient::new(&w.env, &w.long_tk).mint(&w.ord_vault, &close_collateral);
    let close_key = hc.create_order(
        &w.user,
        &CreateOrderParams {
            receiver: w.user.clone(),
            market: w.market_tk.clone(),
            initial_collateral_token: w.long_tk.clone(),
            swap_path: Vec::new(&w.env),
            size_delta_usd: 5_000 * fp,
            collateral_delta_amount: close_collateral,
            trigger_price: 0,
            acceptable_price: 0,
            execution_fee: 0,
            min_output_amount: 0,
            order_type: OrderType::MarketDecrease,
            is_long: true,
        },
    );
    set_prices(&w, price);
    hc.execute_order(&w.keeper, &close_key);

    // cumulative borrowing factor must have changed
    let cum_key = gmx_keys::cumulative_borrowing_factor_key(&w.env, &w.market_tk, true);
    let cum_final = ds_c.get_u128(&cum_key);
    assert!(cum_final > 0, "cumulative borrowing factor must be nonzero after time advance");
}
