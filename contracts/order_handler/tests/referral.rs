// Integration tests for the referral system — issue #195.
//
// Tests both the referral_storage contract's own logic AND the order_handler's
// integration with it (increment_referrer_volume called after order execution).
//
// NOTE: get_position_fees does NOT apply referral discounts — the discount_bps
// value is computed correctly but is NOT deducted from the trader's fee at
// execution time. These tests verify what IS implemented:
//   - discount_bps calculation (tier config × code ownership)
//   - volume accumulation via execute_order → increment_referrer_volume
//   - Trader with no code gets 0 discount
//   - Referral storage authorization (only registered order_handler may call
//     increment_referrer_volume)

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
use referral_storage::{ReferralStorage, ReferralStorageClient, TierConfig};
use role_store::{RoleStore, RoleStoreClient as RsClient};
use soroban_sdk::{
    testutils::Address as _,
    token::StellarAssetClient,
    Address, Bytes, Env, Vec,
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
    ref_storage: Address,
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

    // Referral storage
    let ref_storage = env.register(ReferralStorage, ());
    ReferralStorageClient::new(&env, &ref_storage).initialize(&admin);

    // Wire referral_storage ↔ ord_handler bidirectionally
    // referral_storage must know which ord_handler is authorized to increment volumes
    ReferralStorageClient::new(&env, &ref_storage).set_order_handler(&admin, &ord_handler);
    // ord_handler must know where to send volume increments
    OrderHandlerClient::new(&env, &ord_handler).set_referral_storage(&admin, &ref_storage);

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
        ref_storage,
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
    set_prices(w, 2000 * FLOAT_PRECISION);
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

/// get_trader_discount_bps returns the correct value:
///   discount = total_rebate_bps × discount_share_bps / 10_000
///
/// Setup: Alice registers "ALICE" code; Bob links to it.
/// Tier 1: total_rebate_bps=2000, discount_share_bps=5000 → 1000 bps discount.
#[test]
fn discount_bps_computed_correctly_for_linked_trader() {
    let w = setup();
    let alice = Address::generate(&w.env);
    let bob = Address::generate(&w.env);
    let ref_c = ReferralStorageClient::new(&w.env, &w.ref_storage);
    let code = Bytes::from_slice(&w.env, b"ALICE");

    // Alice registers the code
    ref_c.register_code(&alice, &code);

    // Configure tier 1: total_rebate=2000 bps, discount_share=5000 bps → 1000 bps net discount
    ref_c.set_tier_config(
        &w.admin,
        &1u32,
        &TierConfig { total_rebate_bps: 2_000, discount_share_bps: 5_000 },
    );

    // Set Alice to tier 1
    ref_c.set_referrer_tier(&w.admin, &alice, &1u32);

    // Bob links to ALICE code
    ref_c.set_trader_referral_code(&bob, &code);

    // Verify discount_bps: 2_000 × 5_000 / 10_000 = 1_000
    let discount = ref_c.get_trader_discount_bps(&bob);
    assert_eq!(discount, 1_000, "Bob's discount must be 1000 bps (10%)");
}

/// A trader with no referral code gets 0 discount.
#[test]
fn trader_with_no_code_gets_zero_discount() {
    let w = setup();
    let trader = Address::generate(&w.env);
    let ref_c = ReferralStorageClient::new(&w.env, &w.ref_storage);

    assert_eq!(ref_c.get_trader_discount_bps(&trader), 0);
}

/// A referrer on tier 0 with no configured TierConfig gets 0 discount.
#[test]
fn unconfigured_tier_returns_zero_discount() {
    let w = setup();
    let alice = Address::generate(&w.env);
    let bob = Address::generate(&w.env);
    let ref_c = ReferralStorageClient::new(&w.env, &w.ref_storage);
    let code = Bytes::from_slice(&w.env, b"ALICE2");

    ref_c.register_code(&alice, &code);
    ref_c.set_trader_referral_code(&bob, &code);
    // No set_tier_config for tier 0 → discount must be 0

    assert_eq!(ref_c.get_trader_discount_bps(&bob), 0);
}

/// After execute_order for a position order (size_delta_usd > 0), the order_handler
/// calls increment_referrer_volume, which accumulates in referral_storage.
#[test]
fn execute_order_increments_referrer_volume() {
    let w = setup();
    let fp = FLOAT_PRECISION;
    let price = 2000 * fp;
    let alice = Address::generate(&w.env);
    let ref_c = ReferralStorageClient::new(&w.env, &w.ref_storage);
    let code = Bytes::from_slice(&w.env, b"ALICE");

    // Alice registers code
    ref_c.register_code(&alice, &code);

    // w.user links to Alice's code
    ref_c.set_trader_referral_code(&w.user, &code);

    set_prices(&w, price);
    seed_pool(&w);

    // Alice starts with 0 cumulative volume
    assert_eq!(ref_c.get_referrer_cumulative_volume(&alice), 0);

    // Open a position: size_delta_usd = 2000 * FP
    let size_delta_usd = 2_000 * fp;
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
            size_delta_usd,
            collateral_delta_amount: collateral,
            trigger_price: 0,
            acceptable_price: 0,
            execution_fee: 0,
            min_output_amount: 0,
            order_type: OrderType::MarketIncrease,
            is_long: true,
        },
    );
    set_prices(&w, price);
    hc.execute_order(&w.keeper, &open_key);

    // Alice's volume must equal the position's size_delta_usd
    let alice_volume = ref_c.get_referrer_cumulative_volume(&alice);
    assert_eq!(
        alice_volume,
        size_delta_usd as u128,
        "Alice's cumulative volume must equal the position's size_delta_usd"
    );
}

/// Referrer volume accumulates across multiple order executions.
#[test]
fn referrer_volume_accumulates_across_orders() {
    let w = setup();
    let fp = FLOAT_PRECISION;
    let price = 2000 * fp;
    let alice = Address::generate(&w.env);
    let ref_c = ReferralStorageClient::new(&w.env, &w.ref_storage);
    let code = Bytes::from_slice(&w.env, b"ALICE");

    ref_c.register_code(&alice, &code);
    ref_c.set_trader_referral_code(&w.user, &code);

    set_prices(&w, price);
    seed_pool(&w);

    let size1 = 1_000 * fp;
    let size2 = 500 * fp;

    // First position open
    let collateral1 = 5 * TOKEN_PRECISION;
    StellarAssetClient::new(&w.env, &w.long_tk).mint(&w.ord_vault, &collateral1);
    let hc = OrderHandlerClient::new(&w.env, &w.ord_handler);
    let key1 = hc.create_order(
        &w.user,
        &CreateOrderParams {
            receiver: w.user.clone(),
            market: w.market_tk.clone(),
            initial_collateral_token: w.long_tk.clone(),
            swap_path: Vec::new(&w.env),
            size_delta_usd: size1,
            collateral_delta_amount: collateral1,
            trigger_price: 0,
            acceptable_price: 0,
            execution_fee: 0,
            min_output_amount: 0,
            order_type: OrderType::MarketIncrease,
            is_long: true,
        },
    );
    set_prices(&w, price);
    hc.execute_order(&w.keeper, &key1);

    // Second increase to the same position
    let collateral2 = 3 * TOKEN_PRECISION;
    StellarAssetClient::new(&w.env, &w.long_tk).mint(&w.ord_vault, &collateral2);
    let key2 = hc.create_order(
        &w.user,
        &CreateOrderParams {
            receiver: w.user.clone(),
            market: w.market_tk.clone(),
            initial_collateral_token: w.long_tk.clone(),
            swap_path: Vec::new(&w.env),
            size_delta_usd: size2,
            collateral_delta_amount: collateral2,
            trigger_price: 0,
            acceptable_price: 0,
            execution_fee: 0,
            min_output_amount: 0,
            order_type: OrderType::MarketIncrease,
            is_long: true,
        },
    );
    set_prices(&w, price);
    hc.execute_order(&w.keeper, &key2);

    let alice_volume = ref_c.get_referrer_cumulative_volume(&alice);
    assert_eq!(
        alice_volume,
        (size1 + size2) as u128,
        "Alice's volume must sum both position sizes"
    );
}

/// Swap orders (size_delta_usd == 0) do NOT increment referrer volume.
#[test]
fn swap_order_does_not_increment_referrer_volume() {
    let w = setup();
    let fp = FLOAT_PRECISION;
    let alice = Address::generate(&w.env);
    let ref_c = ReferralStorageClient::new(&w.env, &w.ref_storage);
    let code = Bytes::from_slice(&w.env, b"ALICE");

    ref_c.register_code(&alice, &code);
    ref_c.set_trader_referral_code(&w.user, &code);

    set_prices(&w, 2000 * fp);
    seed_pool(&w);

    // Swap 1 WETH → USDC: 1 WETH × $2000 = $2000 worth of USDC.
    // Pool has 5000 USDC so 2000 < 5000 → no liquidity underflow.
    let collateral = TOKEN_PRECISION; // 1 WETH
    StellarAssetClient::new(&w.env, &w.long_tk).mint(&w.ord_vault, &collateral);
    let hc = OrderHandlerClient::new(&w.env, &w.ord_handler);
    let key = hc.create_order(
        &w.user,
        &CreateOrderParams {
            receiver: w.user.clone(),
            market: w.market_tk.clone(),
            initial_collateral_token: w.long_tk.clone(),
            swap_path: Vec::from_array(&w.env, [w.market_tk.clone()]),
            size_delta_usd: 0, // swap has no size_delta
            collateral_delta_amount: collateral,
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

    // No volume should have been recorded (size_delta_usd == 0)
    assert_eq!(
        ref_c.get_referrer_cumulative_volume(&alice),
        0,
        "swap orders (size_delta_usd=0) must not increment referrer volume"
    );
}

/// get_trader_referrer returns the correct referrer address for a linked trader.
#[test]
fn get_trader_referrer_returns_alice_for_bob() {
    let w = setup();
    let alice = Address::generate(&w.env);
    let bob = Address::generate(&w.env);
    let ref_c = ReferralStorageClient::new(&w.env, &w.ref_storage);
    let code = Bytes::from_slice(&w.env, b"ALICE");

    ref_c.register_code(&alice, &code);
    ref_c.set_trader_referral_code(&bob, &code);

    assert_eq!(ref_c.get_trader_referrer(&bob), Some(alice));
}

/// get_trader_referrer returns None for a trader with no code.
#[test]
fn get_trader_referrer_returns_none_for_unlinked_trader() {
    let w = setup();
    let trader = Address::generate(&w.env);
    let ref_c = ReferralStorageClient::new(&w.env, &w.ref_storage);

    assert_eq!(ref_c.get_trader_referrer(&trader), None);
}

/// Volume accumulation triggers an automatic tier upgrade when thresholds are met.
#[test]
fn auto_tier_upgrade_via_execute_order_volume() {
    let w = setup();
    let fp = FLOAT_PRECISION;
    let price = 2000 * fp;
    let alice = Address::generate(&w.env);
    let ref_c = ReferralStorageClient::new(&w.env, &w.ref_storage);
    let code = Bytes::from_slice(&w.env, b"ALICE");

    ref_c.register_code(&alice, &code);
    ref_c.set_trader_referral_code(&w.user, &code);

    // Tier 1 upgrade threshold: 1_000 * FP (1000 USD)
    ref_c.set_tier_upgrade_threshold(&w.admin, &1u32, &(1_000 * fp as u128));
    ref_c.set_tier_config(&w.admin, &1u32, &TierConfig { total_rebate_bps: 500, discount_share_bps: 5_000 });

    set_prices(&w, price);
    seed_pool(&w);

    // Trade with size > threshold → auto-upgrade Alice to tier 1
    let size_delta_usd = 2_000 * fp; // $2000 > threshold of $1000
    let collateral = 10 * TOKEN_PRECISION;
    StellarAssetClient::new(&w.env, &w.long_tk).mint(&w.ord_vault, &collateral);
    let hc = OrderHandlerClient::new(&w.env, &w.ord_handler);
    let key = hc.create_order(
        &w.user,
        &CreateOrderParams {
            receiver: w.user.clone(),
            market: w.market_tk.clone(),
            initial_collateral_token: w.long_tk.clone(),
            swap_path: Vec::new(&w.env),
            size_delta_usd,
            collateral_delta_amount: collateral,
            trigger_price: 0,
            acceptable_price: 0,
            execution_fee: 0,
            min_output_amount: 0,
            order_type: OrderType::MarketIncrease,
            is_long: true,
        },
    );
    set_prices(&w, price);
    hc.execute_order(&w.keeper, &key);

    // Alice should now be on tier 1 → discount_bps = 500 × 5000 / 10000 = 250
    let discount = ref_c.get_trader_discount_bps(&w.user);
    assert_eq!(
        discount, 250,
        "discount must reflect Alice's auto-upgraded tier 1: expected 250 bps, got {}",
        discount
    );
}
