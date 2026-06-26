// Integration tests for order cancellation — issue #268.
//
// Verifies that cancelling an order (user-initiated or after keeper freeze)
// returns the EXACT deposited token amount to the receiver with no slippage
// or rounding loss, and that storage is cleaned up correctly.
//
// Covers:
//   - User cancels a USDC-collateral order: receives exact USDC back
//   - User cancels an XLM (long-token) collateral order: receives exact XLM back
//   - Keeper freezes the order; user then cancels: exact refund, storage cleared
//   - Keeper freezes the order; non-owner cancel attempt panics

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
    long_tk: Address,  // XLM-like native token
    short_tk: Address, // USDC-like stable token
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
    let fp = FLOAT_PRECISION;
    OClient::new(&w.env, &w.oracle).set_prices_simple(
        &w.keeper,
        &Vec::from_array(
            &w.env,
            [
                TokenPrice { token: w.long_tk.clone(), min: 2_000 * fp, max: 2_000 * fp },
                TokenPrice { token: w.short_tk.clone(), min: fp, max: fp },
                TokenPrice { token: w.index_tk.clone(), min: 2_000 * fp, max: 2_000 * fp },
            ],
        ),
    );
    DepositHandlerClient::new(&w.env, &w.dep_handler).execute_deposit(&w.keeper, &k);
}

fn token_balance(w: &World, token: &Address, account: &Address) -> i128 {
    soroban_sdk::token::Client::new(&w.env, token).balance(account)
}

// ─── Tests ────────────────────────────────────────────────────────────────────

/// Cancelling a short-token (USDC) collateral order returns the exact deposited
/// USDC amount — no slippage, no fee, no rounding loss.
#[test]
fn cancel_usdc_order_refunds_exact_short_token() {
    let w = setup();
    let deposit = 50 * TOKEN_PRECISION; // 50 USDC

    // Fund the vault (user pre-transfers collateral to the vault before create_order)
    StellarAssetClient::new(&w.env, &w.short_tk).mint(&w.ord_vault, &deposit);

    let hc = OrderHandlerClient::new(&w.env, &w.ord_handler);
    let key = hc.create_order(
        &w.user,
        &CreateOrderParams {
            receiver: w.user.clone(),
            market: w.market_tk.clone(),
            initial_collateral_token: w.short_tk.clone(),
            swap_path: Vec::new(&w.env),
            size_delta_usd: 100 * FLOAT_PRECISION,
            collateral_delta_amount: deposit,
            trigger_price: 0,
            acceptable_price: 0,
            execution_fee: 0,
            min_output_amount: 0,
            order_type: OrderType::MarketIncrease,
            is_long: false,
        },
    );

    let before = token_balance(&w, &w.short_tk, &w.user);
    hc.cancel_order(&w.user, &key);
    let after = token_balance(&w, &w.short_tk, &w.user);

    assert_eq!(
        after - before,
        deposit,
        "cancel must refund EXACT USDC deposit: expected {} got {}",
        deposit,
        after - before
    );
    assert!(hc.get_order(&key).is_none(), "order record must be removed after cancel");
    assert_eq!(
        OVClient::new(&w.env, &w.ord_vault).get_recorded_balance(&w.short_tk),
        0,
        "vault recorded USDC balance must be zero after cancel"
    );
}

/// Cancelling an XLM (long-token) collateral order returns the exact deposited
/// XLM amount with no slippage.
#[test]
fn cancel_xlm_order_refunds_exact_long_token() {
    let w = setup();
    let deposit = 3 * TOKEN_PRECISION; // 3 XLM

    StellarAssetClient::new(&w.env, &w.long_tk).mint(&w.ord_vault, &deposit);

    let hc = OrderHandlerClient::new(&w.env, &w.ord_handler);
    let key = hc.create_order(
        &w.user,
        &CreateOrderParams {
            receiver: w.user.clone(),
            market: w.market_tk.clone(),
            initial_collateral_token: w.long_tk.clone(),
            swap_path: Vec::new(&w.env),
            size_delta_usd: 6_000 * FLOAT_PRECISION,
            collateral_delta_amount: deposit,
            trigger_price: 0,
            acceptable_price: 0,
            execution_fee: 0,
            min_output_amount: 0,
            order_type: OrderType::MarketIncrease,
            is_long: true,
        },
    );

    let before = token_balance(&w, &w.long_tk, &w.user);
    hc.cancel_order(&w.user, &key);
    let after = token_balance(&w, &w.long_tk, &w.user);

    assert_eq!(
        after - before,
        deposit,
        "cancel must refund EXACT XLM deposit: expected {} got {}",
        deposit,
        after - before
    );
    assert!(hc.get_order(&key).is_none(), "order record must be removed after cancel");
    assert_eq!(
        OVClient::new(&w.env, &w.ord_vault).get_recorded_balance(&w.long_tk),
        0,
        "vault recorded XLM balance must be zero after cancel"
    );
}

/// Keeper-initiated failure path: keeper freezes an unexecutable order;
/// user then cancels and receives the exact collateral refund.
#[test]
fn keeper_freeze_then_user_cancel_refunds_exact_collateral() {
    let w = setup();
    let deposit = 5 * TOKEN_PRECISION;

    StellarAssetClient::new(&w.env, &w.long_tk).mint(&w.ord_vault, &deposit);
    let hc = OrderHandlerClient::new(&w.env, &w.ord_handler);
    let key = hc.create_order(
        &w.user,
        &CreateOrderParams {
            receiver: w.user.clone(),
            market: w.market_tk.clone(),
            initial_collateral_token: w.long_tk.clone(),
            swap_path: Vec::new(&w.env),
            size_delta_usd: 10_000 * FLOAT_PRECISION,
            collateral_delta_amount: deposit,
            trigger_price: 0,
            acceptable_price: 0,
            execution_fee: 0,
            min_output_amount: 0,
            order_type: OrderType::MarketIncrease,
            is_long: true,
        },
    );

    // Keeper marks the order frozen (simulates a failed execution attempt)
    hc.freeze_order(&w.keeper, &key);

    // User cancels the frozen order
    let before = token_balance(&w, &w.long_tk, &w.user);
    hc.cancel_order(&w.user, &key);
    let after = token_balance(&w, &w.long_tk, &w.user);

    assert_eq!(
        after - before,
        deposit,
        "frozen-then-cancelled order must refund exact collateral"
    );
    assert!(hc.get_order(&key).is_none(), "frozen order must be removed after user cancel");
}

/// After cancel, both the global order list and the account order list are purged.
#[test]
fn cancel_removes_order_from_all_storage_lists() {
    let w = setup();
    let deposit = 2 * TOKEN_PRECISION;

    StellarAssetClient::new(&w.env, &w.long_tk).mint(&w.ord_vault, &deposit);
    let hc = OrderHandlerClient::new(&w.env, &w.ord_handler);
    let ds_c = DsClient::new(&w.env, &w.ds);
    let key = hc.create_order(
        &w.user,
        &CreateOrderParams {
            receiver: w.user.clone(),
            market: w.market_tk.clone(),
            initial_collateral_token: w.long_tk.clone(),
            swap_path: Vec::new(&w.env),
            size_delta_usd: 4_000 * FLOAT_PRECISION,
            collateral_delta_amount: deposit,
            trigger_price: 0,
            acceptable_price: 0,
            execution_fee: 0,
            min_output_amount: 0,
            order_type: OrderType::MarketIncrease,
            is_long: true,
        },
    );

    assert!(ds_c.contains_bytes32(&gmx_keys::order_list_key(&w.env), &key));
    assert!(ds_c.contains_bytes32(&gmx_keys::account_order_list_key(&w.env, &w.user), &key));

    hc.cancel_order(&w.user, &key);

    assert!(
        !ds_c.contains_bytes32(&gmx_keys::order_list_key(&w.env), &key),
        "global order list must not contain key after cancel"
    );
    assert!(
        !ds_c.contains_bytes32(&gmx_keys::account_order_list_key(&w.env, &w.user), &key),
        "account order list must not contain key after cancel"
    );
}

/// A non-owner cannot cancel another user's order.
#[test]
#[should_panic]
fn non_owner_cannot_cancel_order() {
    let w = setup();
    let deposit = 2 * TOKEN_PRECISION;
    let attacker = Address::generate(&w.env);

    StellarAssetClient::new(&w.env, &w.long_tk).mint(&w.ord_vault, &deposit);
    let hc = OrderHandlerClient::new(&w.env, &w.ord_handler);
    let key = hc.create_order(
        &w.user,
        &CreateOrderParams {
            receiver: w.user.clone(),
            market: w.market_tk.clone(),
            initial_collateral_token: w.long_tk.clone(),
            swap_path: Vec::new(&w.env),
            size_delta_usd: 4_000 * FLOAT_PRECISION,
            collateral_delta_amount: deposit,
            trigger_price: 0,
            acceptable_price: 0,
            execution_fee: 0,
            min_output_amount: 0,
            order_type: OrderType::MarketIncrease,
            is_long: true,
        },
    );

    hc.cancel_order(&attacker, &key); // must panic
}
