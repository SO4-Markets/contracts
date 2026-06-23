//! Auto-Deleveraging (ADL) handler — partially close profitable positions
//! when the pool's PnL-to-pool-value ratio exceeds the configured threshold.
//! Mirrors GMX's AdlHandler.sol.
//!
//! Delegates actual position closure to order_handler since positions live there.
#![no_std]
#![allow(dependency_on_unit_never_type_fallback)]

use gmx_keys::{
    market_index_token_key, market_long_token_key, market_short_token_key,
    max_pnl_factor_for_adl_key, position_key, roles,
};
use gmx_market_utils::{get_pnl, get_pool_value};
use gmx_math::{mul_div_wide, FLOAT_PRECISION};
use gmx_position_utils::get_position_pnl_usd;
use gmx_types::{MarketProps, PositionProps, PriceProps};
use soroban_sdk::{
    contract, contracterror, contractimpl, contracttype, panic_with_error, symbol_short, Address,
    BytesN, Env,
};

// ─── Storage keys ─────────────────────────────────────────────────────────────

#[contracttype]
enum InstanceKey {
    Initialized,
    Admin,
    RoleStore,
    DataStore,
    Oracle,
    OrderHandler,
}

// ─── Errors ───────────────────────────────────────────────────────────────────

#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
#[repr(u32)]
pub enum Error {
    AlreadyInitialized = 1,
    NotInitialized = 2,
    Unauthorized = 3,
    AdlNotRequired = 4,
    InvalidInput = 5,
    NotProfitable = 6,
    PositionNotFound = 7,
    /// Max PnL factor for ADL is not configured (0) for the requested market/side.
    /// Callers must set a non-zero value via DataStore before ADL can be evaluated.
    MissingMaxPnlConfig = 8,
}

// ─── External clients ─────────────────────────────────────────────────────────

#[allow(dead_code)]
#[soroban_sdk::contractclient(name = "RoleStoreClient")]
trait IRoleStore {
    fn has_role(env: Env, account: Address, role: BytesN<32>) -> bool;
}

#[allow(dead_code)]
#[soroban_sdk::contractclient(name = "DataStoreClient")]
trait IDataStore {
    fn get_u128(env: Env, key: BytesN<32>) -> u128;
    fn get_address(env: Env, key: BytesN<32>) -> Option<Address>;
}

#[allow(dead_code)]
#[soroban_sdk::contractclient(name = "OracleClient")]
trait IOracle {
    fn get_primary_price(env: Env, token: Address) -> PriceProps;
}

#[allow(dead_code)]
#[soroban_sdk::contractclient(name = "OrderHandlerClient")]
trait IOrderHandler {
    fn execute_adl(
        env: Env,
        keeper: Address,
        account: Address,
        market: Address,
        collateral_token: Address,
        is_long: bool,
        size_delta_usd: i128,
    );
    fn get_position(env: Env, key: BytesN<32>) -> Option<PositionProps>;
}

// ─── Contract ─────────────────────────────────────────────────────────────────

#[contract]
pub struct AdlHandler;

#[contractimpl]
impl AdlHandler {
    pub fn initialize(
        env: Env,
        admin: Address,
        role_store: Address,
        data_store: Address,
        oracle: Address,
        order_handler: Address,
    ) {
        admin.require_auth();
        if env.storage().instance().has(&InstanceKey::Initialized) {
            panic_with_error!(&env, Error::AlreadyInitialized);
        }
        env.storage()
            .instance()
            .set(&InstanceKey::Initialized, &true);
        env.storage().instance().set(&InstanceKey::Admin, &admin);
        env.storage()
            .instance()
            .set(&InstanceKey::RoleStore, &role_store);
        env.storage()
            .instance()
            .set(&InstanceKey::DataStore, &data_store);
        env.storage().instance().set(&InstanceKey::Oracle, &oracle);
        env.storage()
            .instance()
            .set(&InstanceKey::OrderHandler, &order_handler);
    }

    /// Check whether ADL is currently required for the given market side.
    ///
    /// Returns true if total trader PnL / pool_value > MAX_PNL_FACTOR_FOR_ADL.
    pub fn is_adl_required(env: Env, market: Address, is_long: bool) -> bool {
        let data_store: Address = env
            .storage()
            .instance()
            .get(&InstanceKey::DataStore)
            .unwrap_or_else(|| panic_with_error!(&env, Error::NotInitialized));
        let oracle: Address = env
            .storage()
            .instance()
            .get(&InstanceKey::Oracle)
            .unwrap_or_else(|| panic_with_error!(&env, Error::NotInitialized));

        let market_props = load_market_props(&env, &data_store, &market);
        let oracle_client = OracleClient::new(&env, &oracle);
        let index_price_props = oracle_client.get_primary_price(&market_props.index_token);
        let long_price = oracle_client
            .get_primary_price(&market_props.long_token)
            .mid_price();
        let short_price = oracle_client
            .get_primary_price(&market_props.short_token)
            .mid_price();
        let index_price = index_price_props.mid_price();

        // Minimize pool value (conservative: harder to trigger ADL)
        let pool_info = get_pool_value(
            &env,
            &data_store,
            &market_props,
            long_price,
            short_price,
            index_price,
            false,
        );
        if pool_info.pool_value <= 0 {
            return false;
        }

        // Maximize trader PnL (worst case for pool)
        let pnl = get_pnl(&env, &data_store, &market_props, index_price, is_long, true);
        if pnl <= 0 {
            return false;
        }

        let pnl_factor = mul_div_wide(&env, pnl, FLOAT_PRECISION, pool_info.pool_value);

        // A zero value means no threshold is configured; ADL is disabled for this market/side.
        let max_pnl_factor = DataStoreClient::new(&env, &data_store)
            .get_u128(&max_pnl_factor_for_adl_key(&env, &market, is_long))
            as i128;

        if max_pnl_factor == 0 {
            return false;
        }
        pnl_factor > max_pnl_factor
    }

    /// Execute ADL on a specific profitable position.
    ///
    /// Validates ADL is required and the position is profitable, then delegates
    /// the partial close to order_handler (where positions are stored).
    pub fn execute_adl(
        env: Env,
        keeper: Address,
        account: Address,
        market: Address,
        collateral_token: Address,
        is_long: bool,
        size_delta_usd: i128,
    ) {
        keeper.require_auth();

        // Input validation
        if size_delta_usd <= 0 {
            panic_with_error!(&env, Error::InvalidInput);
        }

        let role_store: Address = env
            .storage()
            .instance()
            .get(&InstanceKey::RoleStore)
            .unwrap_or_else(|| panic_with_error!(&env, Error::NotInitialized));
        if !RoleStoreClient::new(&env, &role_store).has_role(&keeper, &roles::adl_keeper(&env)) {
            panic_with_error!(&env, Error::Unauthorized);
        }

        // Check ADL is required
        if !AdlHandler::is_adl_required(env.clone(), market.clone(), is_long) {
            panic_with_error!(&env, Error::AdlNotRequired);
        }

        let data_store: Address = env
            .storage()
            .instance()
            .get(&InstanceKey::DataStore)
            .unwrap_or_else(|| panic_with_error!(&env, Error::NotInitialized));
        let oracle: Address = env
            .storage()
            .instance()
            .get(&InstanceKey::Oracle)
            .unwrap_or_else(|| panic_with_error!(&env, Error::NotInitialized));
        let order_handler: Address = env
            .storage()
            .instance()
            .get(&InstanceKey::OrderHandler)
            .unwrap_or_else(|| panic_with_error!(&env, Error::NotInitialized));

        let market_props = load_market_props(&env, &data_store, &market);
        let oracle_client = OracleClient::new(&env, &oracle);
        let index_price = oracle_client.get_primary_price(&market_props.index_token);

        // Verify the target position is profitable (ADL only closes profitable positions)
        let pk = position_key(&env, &account, &market, &collateral_token, is_long);
        let position: PositionProps =
            match OrderHandlerClient::new(&env, &order_handler).get_position(&pk) {
                Some(p) => p,
                None => panic_with_error!(&env, Error::PositionNotFound),
            };

        let (pnl_usd, _) = get_position_pnl_usd(&env, &position, &index_price, size_delta_usd);
        if pnl_usd <= 0 {
            panic_with_error!(&env, Error::NotProfitable);
        }

        // Delegate to order_handler
        OrderHandlerClient::new(&env, &order_handler).execute_adl(
            &keeper,
            &account,
            &market,
            &collateral_token,
            &is_long,
            &size_delta_usd,
        );

        env.events().publish(
            (symbol_short!("adl_req"),),
            (account, market, is_long, size_delta_usd, pnl_usd),
        );
    }
}

// ─── Tests — Issue #134: ADL E2E tests through deployed-style clients ─────────
//
// Done: ADL triggers above threshold. Below threshold reverts. Keeper-only
//       access is enforced.
#[cfg(test)]
mod tests {
    use super::*;
    use data_store::{DataStore, DataStoreClient as DsClient};
    use deposit_handler::{DepositHandler, DepositHandlerClient};
    use deposit_vault::{DepositVault, DepositVaultClient as DVClient};
    use gmx_keys::roles;
    use gmx_math::FLOAT_PRECISION;
    use gmx_types::{CreateDepositParams, CreateOrderParams, OrderType, TokenPrice};
    use market_token::{MarketToken, MarketTokenClient as MtClient};
    use oracle::{Oracle, OracleClient as OClient};
    use order_handler::{OrderHandler, OrderHandlerClient as OHClient};
    use order_vault::{OrderVault, OrderVaultClient as OVClient};
    use role_store::{RoleStore, RoleStoreClient as RsClient};
    use soroban_sdk::{testutils::Address as _, token::StellarAssetClient, Env, Vec};

    const ONE_TOKEN: i128 = 10_000_000; // Stellar 7-decimal precision

    struct World {
        env: Env,
        admin: Address,
        keeper: Address,
        adl_keeper: Address,
        rs: Address,
        ds: Address,
        oracle: Address,
        dep_vault: Address,
        ord_vault: Address,
        dep_handler: Address,
        ord_handler: Address,
        adl_handler: Address,
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
        let adl_keeper = Address::generate(&env);

        let rs = env.register(RoleStore, ());
        let rs_c = RsClient::new(&env, &rs);
        rs_c.initialize(&admin);
        rs_c.grant_role(&admin, &admin, &roles::controller(&env));
        rs_c.grant_role(&admin, &keeper, &roles::order_keeper(&env));
        rs_c.grant_role(&admin, &adl_keeper, &roles::adl_keeper(&env));

        let ds = env.register(DataStore, ());
        DsClient::new(&env, &ds).initialize(&admin, &rs);

        let oracle_addr = env.register(Oracle, ());
        let passphrase = soroban_sdk::Bytes::from_slice(&env, b"Test SDF Network ; September 2015");
        OClient::new(&env, &oracle_addr).initialize(&admin, &rs, &ds, &passphrase);

        let dep_vault = env.register(DepositVault, ());
        DVClient::new(&env, &dep_vault).initialize(&admin, &rs);

        let ord_vault = env.register(OrderVault, ());
        OVClient::new(&env, &ord_vault).initialize(&admin, &rs);

        let market_tk = env.register(MarketToken, ());
        MtClient::new(&env, &market_tk).initialize(
            &admin,
            &rs,
            &7u32,
            &soroban_sdk::String::from_str(&env, "ADL Test Market"),
            &soroban_sdk::String::from_str(&env, "GM"),
        );

        let long_tk = env
            .register_stellar_asset_contract_v2(admin.clone())
            .address();
        let short_tk = env
            .register_stellar_asset_contract_v2(admin.clone())
            .address();
        let index_tk = Address::generate(&env);

        let dep_handler = env.register(DepositHandler, ());
        DepositHandlerClient::new(&env, &dep_handler).initialize(
            &admin,
            &rs,
            &ds,
            &oracle_addr,
            &dep_vault,
        );

        let ord_handler = env.register(OrderHandler, ());
        OHClient::new(&env, &ord_handler).initialize(&admin, &rs, &ds, &oracle_addr, &ord_vault);

        let adl_handler_addr = env.register(AdlHandler, ());
        AdlHandlerClient::new(&env, &adl_handler_addr).initialize(
            &admin,
            &rs,
            &ds,
            &oracle_addr,
            &ord_handler,
        );

        // Grant CONTROLLER to all handlers
        rs_c.grant_role(&admin, &dep_handler, &roles::controller(&env));
        rs_c.grant_role(&admin, &ord_handler, &roles::controller(&env));
        rs_c.grant_role(&admin, &adl_handler_addr, &roles::controller(&env));
        rs_c.grant_role(&admin, &market_tk, &roles::controller(&env));

        // Register market tokens in DataStore
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

        // Market config
        let fee_factor = FLOAT_PRECISION / 1_000; // 0.1%
        let min_col_factor = FLOAT_PRECISION / 100; // 1%
        ds_c.set_u128(
            &admin,
            &gmx_keys::position_fee_factor_key(&env, &market_tk, true),
            &(fee_factor as u128),
        );
        ds_c.set_u128(
            &admin,
            &gmx_keys::position_fee_factor_key(&env, &market_tk, false),
            &(fee_factor as u128),
        );
        ds_c.set_u128(
            &admin,
            &gmx_keys::min_collateral_factor_key(&env, &market_tk),
            &(min_col_factor as u128),
        );
        ds_c.set_u128(
            &admin,
            &gmx_keys::max_leverage_key(&env, &market_tk),
            &(100 * FLOAT_PRECISION as u128),
        );

        World {
            env,
            admin,
            keeper,
            adl_keeper,
            rs,
            ds,
            oracle: oracle_addr,
            dep_vault,
            ord_vault,
            dep_handler,
            ord_handler,
            adl_handler: adl_handler_addr,
            market_tk,
            long_tk,
            short_tk,
            index_tk,
        }
    }

    fn set_prices(w: &World, index_usd: i128) {
        let fp = FLOAT_PRECISION;
        OClient::new(&w.env, &w.oracle).set_prices_simple(
            &w.keeper,
            &Vec::from_array(
                &w.env,
                [
                    TokenPrice {
                        token: w.long_tk.clone(),
                        min: index_usd,
                        max: index_usd,
                    },
                    TokenPrice {
                        token: w.short_tk.clone(),
                        min: fp,
                        max: fp,
                    },
                    TokenPrice {
                        token: w.index_tk.clone(),
                        min: index_usd,
                        max: index_usd,
                    },
                ],
            ),
        );
    }

    fn seed_pool(w: &World, long_amt: i128) {
        let lp = Address::generate(&w.env);
        StellarAssetClient::new(&w.env, &w.long_tk).mint(&lp, &long_amt);
        let key = DepositHandlerClient::new(&w.env, &w.dep_handler).create_deposit(
            &lp,
            &CreateDepositParams {
                receiver: lp.clone(),
                market: w.market_tk.clone(),
                initial_long_token: w.long_tk.clone(),
                initial_short_token: w.short_tk.clone(),
                long_token_amount: long_amt,
                short_token_amount: 0,
                min_market_tokens: 1,
                execution_fee: 0,
            },
        );
        DepositHandlerClient::new(&w.env, &w.dep_handler).execute_deposit(&w.keeper, &key);
    }

    /// Open a MarketIncrease long using the canonical send-then-create pattern.
    fn open_long(
        w: &World,
        trader: &Address,
        collateral: i128,
        size_usd: i128,
    ) -> soroban_sdk::BytesN<32> {
        StellarAssetClient::new(&w.env, &w.long_tk).mint(trader, &collateral);
        soroban_sdk::token::Client::new(&w.env, &w.long_tk).transfer(
            trader,
            &w.ord_vault,
            &collateral,
        );
        let key = OHClient::new(&w.env, &w.ord_handler).create_order(
            trader,
            &CreateOrderParams {
                receiver: trader.clone(),
                market: w.market_tk.clone(),
                initial_collateral_token: w.long_tk.clone(),
                swap_path: soroban_sdk::Vec::new(&w.env),
                size_delta_usd: size_usd,
                collateral_delta_amount: collateral,
                trigger_price: 0,
                acceptable_price: 0,
                execution_fee: 0,
                min_output_amount: 0,
                order_type: OrderType::MarketIncrease,
                is_long: true,
            },
        );
        OHClient::new(&w.env, &w.ord_handler).execute_order(&w.keeper, &key);
        key
    }

    // ── Issue #134 Test 1: ADL executes above threshold ───────────────────────

    /// Open a profitable long position, set a very low ADL threshold so ADL is
    /// required, then call execute_adl through the AdlHandler client.
    /// The position size must decrease after ADL.
    #[test]
    fn e2e_adl_executes_when_pnl_factor_exceeds_threshold() {
        let w = setup();
        let fp = FLOAT_PRECISION;
        let trader = Address::generate(&w.env);

        let entry_price = 1_000 * fp;
        set_prices(&w, entry_price);

        // Deep liquidity so position can open and the pool can cover the PnL
        seed_pool(&w, ONE_TOKEN * 200);

        set_prices(&w, entry_price);

        // Open a sizeable long position (2x leverage)
        let collateral = 5 * ONE_TOKEN;
        let size_usd = 10_000 * fp; // $10_000 notional
        open_long(&w, &trader, collateral, size_usd);

        // Price rises sharply → position is now very profitable
        let rally_price = 2_000 * fp;
        set_prices(&w, rally_price);

        // Confirm ADL is not required with no threshold configured (returns false)
        assert!(
            !AdlHandlerClient::new(&w.env, &w.adl_handler).is_adl_required(&w.market_tk, &true),
            "ADL must not be required when max_pnl_factor is 0 (no cap)"
        );

        // Set a very low ADL threshold so the current PnL ratio triggers ADL
        let low_threshold = fp / 1_000_000; // tiny: almost any profit triggers ADL
        DsClient::new(&w.env, &w.ds).set_u128(
            &w.admin,
            &max_pnl_factor_for_adl_key(&w.env, &w.market_tk, true),
            &(low_threshold as u128),
        );

        assert!(
            AdlHandlerClient::new(&w.env, &w.adl_handler).is_adl_required(&w.market_tk, &true),
            "ADL must be required when PnL factor exceeds the threshold"
        );

        // Record position size before ADL
        let pos_key = position_key(&w.env, &trader, &w.market_tk, &w.long_tk, true);
        let pos_before = OHClient::new(&w.env, &w.ord_handler)
            .get_position(&pos_key)
            .expect("position must exist before ADL");
        assert!(pos_before.size_in_usd > 0);

        // Execute ADL via the handler client (keeper-gated)
        let adl_size = size_usd / 4; // Partially close 25% of the position
        AdlHandlerClient::new(&w.env, &w.adl_handler).execute_adl(
            &w.adl_keeper,
            &trader,
            &w.market_tk,
            &w.long_tk,
            &true,
            &adl_size,
        );

        // Position size must have decreased
        let pos_after = OHClient::new(&w.env, &w.ord_handler)
            .get_position(&pos_key)
            .expect("position must still exist after partial ADL");
        assert!(
            pos_after.size_in_usd < pos_before.size_in_usd,
            "ADL must reduce position size: before={}, after={}",
            pos_before.size_in_usd,
            pos_after.size_in_usd
        );
    }

    // ── Issue #134 Test 2: ADL reverts when PnL factor is below threshold ─────

    /// When the ADL threshold is set high enough that the current PnL ratio
    /// does not exceed it, execute_adl must revert with AdlNotRequired.
    #[test]
    #[should_panic]
    fn e2e_adl_reverts_when_pnl_factor_below_threshold() {
        let w = setup();
        let fp = FLOAT_PRECISION;
        let trader = Address::generate(&w.env);

        let entry_price = 1_000 * fp;
        set_prices(&w, entry_price);
        seed_pool(&w, ONE_TOKEN * 200);
        set_prices(&w, entry_price);

        // Open a modest long position
        open_long(&w, &trader, 5 * ONE_TOKEN, 5_000 * fp);

        // Price rises only slightly → small PnL ratio
        let rally_price = 1_100 * fp; // +10%
        set_prices(&w, rally_price);

        // Set a very high ADL threshold — PnL ratio will not exceed it
        let high_threshold = FLOAT_PRECISION; // 100% — essentially unreachable
        DsClient::new(&w.env, &w.ds).set_u128(
            &w.admin,
            &max_pnl_factor_for_adl_key(&w.env, &w.market_tk, true),
            &(high_threshold as u128),
        );

        assert!(
            !AdlHandlerClient::new(&w.env, &w.adl_handler).is_adl_required(&w.market_tk, &true),
            "ADL must not be required with high threshold and modest profit"
        );

        // Must panic with AdlNotRequired
        AdlHandlerClient::new(&w.env, &w.adl_handler).execute_adl(
            &w.adl_keeper,
            &trader,
            &w.market_tk,
            &w.long_tk,
            &true,
            &(500 * fp),
        );
    }

    // ── Issue #134 Test 3: Keeper-only access is enforced ────────────────────

    /// A caller without the ADL_KEEPER role must be rejected by execute_adl.
    #[test]
    #[should_panic]
    fn e2e_adl_requires_adl_keeper_role() {
        let w = setup();
        let fp = FLOAT_PRECISION;
        let trader = Address::generate(&w.env);
        let impostor = Address::generate(&w.env); // no ADL_KEEPER role

        let entry_price = 1_000 * fp;
        set_prices(&w, entry_price);
        seed_pool(&w, ONE_TOKEN * 200);
        set_prices(&w, entry_price);

        open_long(&w, &trader, 5 * ONE_TOKEN, 10_000 * fp);

        let rally_price = 2_000 * fp;
        set_prices(&w, rally_price);

        // Set a low threshold so ADL is technically required
        DsClient::new(&w.env, &w.ds).set_u128(
            &w.admin,
            &max_pnl_factor_for_adl_key(&w.env, &w.market_tk, true),
            &1u128,
        );

        // impostor has no ADL_KEEPER role — must panic with Unauthorized
        AdlHandlerClient::new(&w.env, &w.adl_handler).execute_adl(
            &impostor,
            &trader,
            &w.market_tk,
            &w.long_tk,
            &true,
            &(1_000 * fp),
        );
    }

    // ── Issue #134 Test 4: is_adl_required with unprofitable positions ────────

    /// is_adl_required must return false when traders are at a loss
    /// (PnL ≤ 0 → the pool is not at risk, no ADL needed).
    #[test]
    fn e2e_adl_not_required_when_position_unprofitable() {
        let w = setup();
        let fp = FLOAT_PRECISION;
        let trader = Address::generate(&w.env);

        let entry_price = 2_000 * fp;
        set_prices(&w, entry_price);
        seed_pool(&w, ONE_TOKEN * 200);
        set_prices(&w, entry_price);

        open_long(&w, &trader, 5 * ONE_TOKEN, 10_000 * fp);

        // Price drops below entry → long position is at a loss
        let crash_price = 500 * fp;
        set_prices(&w, crash_price);

        // Even with the strictest threshold, unprofitable PnL means no ADL
        DsClient::new(&w.env, &w.ds).set_u128(
            &w.admin,
            &max_pnl_factor_for_adl_key(&w.env, &w.market_tk, true),
            &1u128,
        );

        assert!(
            !AdlHandlerClient::new(&w.env, &w.adl_handler).is_adl_required(&w.market_tk, &true),
            "ADL must not be required when position PnL is negative"
        );
    }
}

// ─── Helpers ──────────────────────────────────────────────────────────────────

fn load_market_props(env: &Env, data_store: &Address, market_token: &Address) -> MarketProps {
    let ds = DataStoreClient::new(env, data_store);
    let index_token = ds
        .get_address(&market_index_token_key(env, market_token))
        .unwrap_or_else(|| panic_with_error!(env, Error::InvalidInput));
    let long_token = ds
        .get_address(&market_long_token_key(env, market_token))
        .unwrap_or_else(|| panic_with_error!(env, Error::InvalidInput));
    let short_token = ds
        .get_address(&market_short_token_key(env, market_token))
        .unwrap_or_else(|| panic_with_error!(env, Error::InvalidInput));
    MarketProps {
        market_token: market_token.clone(),
        index_token,
        long_token,
        short_token,
    }
}
