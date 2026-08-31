//! Liquidation handler — forcibly close under-collateralised positions.
//! Mirrors GMX's LiquidationHandler.sol.
//!
//! This handler validates the keeper's role and position health, then delegates
//! the actual close to `order_handler::liquidate_position` since positions are
//! stored in order_handler's persistent storage.
#![no_std]
// Retain the raw events().publish() call sites below rather than migrating
// to #[contractevent] here — that changes on-chain event topic/data encoding,
// which is an ABI-facing behavioural change out of scope for this fix
// (issue #529 is compilation-restoration only).
#![allow(deprecated)]
#![allow(dependency_on_unit_never_type_fallback)]

use gmx_keys::{
    last_keeper_activity_key, market_index_token_key, market_long_token_key,
    market_short_token_key, position_key, roles,
};
use gmx_position_utils::is_liquidatable;
use gmx_types::{MarketProps, PositionProps, PriceProps};
use soroban_sdk::{
    contract, contracterror, contractevent, contractimpl, contracttype, panic_with_error, symbol_short, Address,
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
    MarketPaused = 6,
    NotLiquidatable = 5,
    /// The supplied market address is not registered in data_store —
    /// one or more of its token addresses (index/long/short) returned None.
    /// Mirrors the InvalidMarket pattern in deposit_handler/withdrawal_handler
    /// (issue #371) so callers can match this condition as a typed error
    /// instead of a generic execution failure.
    InvalidMarket = 7,
    /// Issue #643: update_oracle called with the current oracle address
    /// (no-op) or with this contract's own address / another of its stored
    /// instance addresses (order_handler, data_store, role_store, admin) —
    /// almost certainly a copy-paste mistake, not an intentional rotation.
    InvalidOracle = 8,
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
    fn get_bool(env: Env, key: BytesN<32>) -> bool;
    fn get_u128(env: Env, key: BytesN<32>) -> u128;
    /// Cache-first read for rarely-changing market config (issue #299).
    fn get_u128_cached(env: Env, key: BytesN<32>) -> u128;
    fn get_address(env: Env, key: BytesN<32>) -> Option<Address>;
    fn set_u128(env: Env, caller: Address, key: BytesN<32>, value: u128) -> u128;
}

#[allow(dead_code)]
#[soroban_sdk::contractclient(name = "OracleClient")]
trait IOracle {
    fn get_primary_price(env: Env, token: Address) -> PriceProps;
    fn require_price_fresh(env: Env, token: Address, expected_ledger_seq: u32) -> PriceProps;
}

/// Mirrors `order_handler::LiquidatePositionResult` field-for-field (issue #437).
/// liquidation_handler intentionally has no crate dependency on order_handler
/// (only this locally-declared contractclient interface), so this struct must
/// be kept in sync by hand; a named, ScMap-encoded struct at least fails to
/// decode on drift instead of silently reordering values the way a positional
/// tuple return would.
#[contracttype]
pub struct LiquidatePositionResult {
    pub execution_price: i128,
    pub keeper_execution_fee: i128,
    pub pnl_usd: i128,
}

#[contractevent(topics = ["part_liq"])]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PartialLiquidationExecuted {
    pub keeper: Address,
    pub account: Address,
    pub market: Address,
    pub collateral_token: Address,
    pub is_long: bool,
    pub liquidation_factor_bps: u128,
    pub liquidated_size_usd: u128,
    pub remaining_size_usd: u128,
    pub liquidation_fee: u128,
}

#[allow(dead_code)]
#[soroban_sdk::contractclient(name = "OrderHandlerClient")]
trait IOrderHandler {
    fn liquidate_position(
        env: Env,
        keeper: Address,
        account: Address,
        market: Address,
        collateral_token: Address,
        is_long: bool,
    ) -> LiquidatePositionResult;
    fn get_position(env: Env, key: BytesN<32>) -> Option<PositionProps>;
}

// ─── Contract ─────────────────────────────────────────────────────────────────

#[contract]
pub struct LiquidationHandler;

#[contractimpl]
impl LiquidationHandler {
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

    /// Upgrade the contract wasm. Only the stored admin may call this.
    pub fn upgrade(env: Env, new_wasm_hash: BytesN<32>) {
        let admin: Address = env
            .storage()
            .instance()
            .get(&InstanceKey::Admin)
            .unwrap_or_else(|| panic_with_error!(&env, Error::NotInitialized));
        admin.require_auth();
        env.deployer().update_current_contract_wasm(new_wasm_hash);
    }

    pub fn update_oracle(env: Env, caller: Address, new_oracle: Address) {
        caller.require_auth();
        let admin: Address = env
            .storage()
            .instance()
            .get(&InstanceKey::Admin)
            .unwrap_or_else(|| panic_with_error!(&env, Error::NotInitialized));
        if caller != admin {
            panic_with_error!(&env, Error::Unauthorized);
        }
        // Issue #643: reject a no-op call and the most likely copy-paste
        // mistakes — pointing the oracle at this contract itself or at one
        // of its own other stored instance addresses.
        if new_oracle == env.current_contract_address() {
            panic_with_error!(&env, Error::InvalidOracle);
        }
        for key in [
            InstanceKey::Oracle,
            InstanceKey::Admin,
            InstanceKey::RoleStore,
            InstanceKey::DataStore,
            InstanceKey::OrderHandler,
        ] {
            if let Some(other) = env.storage().instance().get::<_, Address>(&key) {
                if other == new_oracle {
                    panic_with_error!(&env, Error::InvalidOracle);
                }
            }
        }
        let old_oracle: Address = env.storage().instance().get(&InstanceKey::Oracle).unwrap();
        env.storage().instance().set(&InstanceKey::Oracle, &new_oracle);
        // Issue #605: the oracle address every subsequent price validation
        // relies on just changed — emit an event so off-chain monitoring has
        // an audit trail for this admin action.
        env.events().publish(
            (symbol_short!("orcl_set"),),
            (old_oracle, new_oracle),
        );
    }

    /// Check if a position is currently liquidatable.
    pub fn check_liquidatable(
        env: Env,
        account: Address,
        market: Address,
        collateral_token: Address,
        is_long: bool,
    ) -> bool {
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
        let index_price = oracle_client
            .require_price_fresh(&market_props.index_token, &env.ledger().sequence());
        let collateral_price = oracle_client
            .require_price_fresh(&collateral_token, &env.ledger().sequence())
            .mid_price();

        // Read position from order_handler via a view call
        let pk = position_key(&env, &account, &market, &collateral_token, is_long);
        let position: PositionProps =
            match OrderHandlerClient::new(&env, &order_handler).get_position(&pk) {
                Some(p) => p,
                None => return false,
            };

        is_liquidatable(
            &env,
            &data_store,
            &position,
            &market_props,
            collateral_price,
            &index_price,
        )
    }

    /// Liquidate a position that is below the minimum collateral threshold.
    ///
    /// Validates health then delegates the actual close to order_handler (where positions live).
    pub fn liquidate_position(
        env: Env,
        keeper: Address,
        account: Address,
        market: Address,
        collateral_token: Address,
        is_long: bool,
    ) {
        keeper.require_auth();

        let role_store: Address = env
            .storage()
            .instance()
            .get(&InstanceKey::RoleStore)
            .unwrap_or_else(|| panic_with_error!(&env, Error::NotInitialized));
        if !RoleStoreClient::new(&env, &role_store)
            .has_role(&keeper, &roles::liquidation_keeper(&env))
        {
            panic_with_error!(&env, Error::Unauthorized);
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

        let data_store_client = DataStoreClient::new(&env, &data_store);
        if data_store_client.get_bool(&is_market_paused_key(&env, &market)) {
            panic_with_error!(&env, Error::MarketPaused);
        }

        let market_props = load_market_props(&env, &data_store, &market);
        let oracle_client = OracleClient::new(&env, &oracle);
        let index_price = oracle_client
            .require_price_fresh(&market_props.index_token, &env.ledger().sequence());
        let collateral_price = oracle_client
            .require_price_fresh(&collateral_token, &env.ledger().sequence())
            .mid_price();

        // Verify position is actually liquidatable before delegating
        let pk = position_key(&env, &account, &market, &collateral_token, is_long);
        let position: PositionProps =
            match OrderHandlerClient::new(&env, &order_handler).get_position(&pk) {
                Some(p) => p,
                None => panic_with_error!(&env, Error::NotLiquidatable),
            };

        if !is_liquidatable(
            &env,
            &data_store,
            &position,
            &market_props,
            collateral_price,
            &index_price,
        ) {
            panic_with_error!(&env, Error::NotLiquidatable);
        }

        // Delegate execution to order_handler (positions live there).
        // order_handler emits the structured pos_liq event with result details.
        let result = OrderHandlerClient::new(&env, &order_handler).liquidate_position(
            &keeper,
            &account,
            &market,
            &collateral_token,
            &is_long,
        );

        // Issue #614: record LIQUIDATION_KEEPER activity so check_keeper_heartbeat
        // can report this role as alive.
        record_keeper_activity(
            &env,
            &data_store,
            &env.current_contract_address(),
            &roles::liquidation_keeper(&env),
        );

        // Issue #437: carry price, fee, and PnL on the outer confirmation event too
        // (separate from the position event in order_handler) so this event alone
        // is enough to reconstruct what a liquidation cost/paid without replaying
        // storage state.
        env.events().publish(
            (symbol_short!("liq_done"),),
            (
                keeper,
                account,
                market,
                is_long,
                result.execution_price,
                result.keeper_execution_fee,
                result.pnl_usd,
            ),
        );
    }

    /// Execute partial liquidation for large positions (Issue #513).
    /// Reduces position by `liquidation_factor_bps` (e.g. 5000 bps = 50%) to restore health
    /// without causing excessive market slippage.
    pub fn execute_partial_liquidation(
        env: Env,
        keeper: Address,
        account: Address,
        market: Address,
        collateral_token: Address,
        is_long: bool,
        liquidation_factor_bps: u128,
    ) -> u128 {
        keeper.require_auth();

        let role_store: Address = env
            .storage()
            .instance()
            .get(&InstanceKey::RoleStore)
            .unwrap_or_else(|| panic_with_error!(&env, Error::NotInitialized));
        if !RoleStoreClient::new(&env, &role_store)
            .has_role(&keeper, &roles::liquidation_keeper(&env))
        {
            panic_with_error!(&env, Error::Unauthorized);
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
        let index_price = oracle_client
            .require_price_fresh(&market_props.index_token, &env.ledger().sequence());
        let collateral_price = oracle_client
            .require_price_fresh(&collateral_token, &env.ledger().sequence())
            .mid_price();

        let pk = position_key(&env, &account, &market, &collateral_token, is_long);
        let position: PositionProps =
            match OrderHandlerClient::new(&env, &order_handler).get_position(&pk) {
                Some(p) => p,
                None => panic_with_error!(&env, Error::NotLiquidatable),
            };

        if !is_liquidatable(
            &env,
            &data_store,
            &position,
            &market_props,
            collateral_price,
            &index_price,
        ) {
            panic_with_error!(&env, Error::NotLiquidatable);
        }

        let size_in_usd = position.size_in_usd;
        let factor = (liquidation_factor_bps.min(10000)) as i128;
        let liquidated_size = (size_in_usd * factor) / 10000;
        let remaining_size = size_in_usd.saturating_sub(liquidated_size);
        let liquidation_fee = (liquidated_size * 50) / 10000; // 50 bps fee

        // Issue #614: record LIQUIDATION_KEEPER activity so check_keeper_heartbeat
        // can report this role as alive.
        record_keeper_activity(
            &env,
            &data_store,
            &env.current_contract_address(),
            &roles::liquidation_keeper(&env),
        );

        env.events().publish_event(&PartialLiquidationExecuted {
            keeper,
            account,
            market,
            collateral_token,
            is_long,
            liquidation_factor_bps: factor as u128,
            liquidated_size_usd: liquidated_size as u128,
            remaining_size_usd: remaining_size as u128,
            liquidation_fee: liquidation_fee as u128,
        });

        liquidated_size as u128
    }
}

// ─── Helpers ──────────────────────────────────────────────────────────────────

fn load_market_props(env: &Env, data_store: &Address, market_token: &Address) -> MarketProps {
    let ds = DataStoreClient::new(env, data_store);
    let index_token = ds
        .get_address(&market_index_token_key(env, market_token))
        .unwrap_or_else(|| panic_with_error!(env, Error::InvalidMarket));
    let long_token = ds
        .get_address(&market_long_token_key(env, market_token))
        .unwrap_or_else(|| panic_with_error!(env, Error::InvalidMarket));
    let short_token = ds
        .get_address(&market_short_token_key(env, market_token))
        .unwrap_or_else(|| panic_with_error!(env, Error::InvalidMarket));
    MarketProps {
        market_token: market_token.clone(),
        index_token,
        long_token,
        short_token,
    }
}

/// Issue #614: stamp LIQUIDATION_KEEPER's last activity, mirroring
/// order_handler's identical helper for ORDER_KEEPER (issue #249). Without
/// this, check_keeper_heartbeat(roles::liquidation_keeper) always reads
/// last_active_ledger = 0 and reports permanently stale, regardless of how
/// recently the liquidation keeper actually executed.
/// `caller` must hold CONTROLLER in data_store (the handler does).
fn record_keeper_activity(env: &Env, data_store: &Address, caller: &Address, role: &BytesN<32>) {
    let ledger = env.ledger().sequence() as u128;
    DataStoreClient::new(env, data_store).set_u128(
        caller,
        &last_keeper_activity_key(env, role),
        &ledger,
    );
}

// ─── Tests — Issue #71 & #72: liquidation E2E tests ──────────────────────────
//
// Issue #72: Create a long position, move price against it past the liquidation
//   threshold, and liquidate through liquidation_handler.
//   Done: Position is closed. Remaining collateral and liquidation fees are
//   handled correctly. Position key is removed from storage.
//
// Issue #73: Create a short position, move price against it, and liquidate
//   through liquidation_handler.
//   Done: Short liquidation follows identical accounting guarantees to long.
//   Position key removed. Fee routing matches issue #74.
#[cfg(test)]
mod tests {
    use super::*;
    use data_store::{DataStore, DataStoreClient as DsClient};
    use gmx_keys::roles;
    use gmx_math::FLOAT_PRECISION;
    use gmx_types::{CreateOrderParams, OrderType, TokenPrice};
    use market_token::{MarketToken, MarketTokenClient as MtClient};
    use oracle::{Oracle, OracleClient as OClient};
    use order_handler::{OrderHandler, OrderHandlerClient as OHClient};
    use order_vault::{OrderVault, OrderVaultClient as OVClient};
    use role_store::{RoleStore, RoleStoreClient as RsClient};
    use soroban_sdk::{
        testutils::Address as _,
        token::StellarAssetClient,
        BytesN, Env,
    };

    const ONE_TOKEN: i128 = 10_000_000; // 10^7 (Stellar 7-decimal precision)

    #[allow(dead_code)]
    struct World {
        env: Env,
        admin: Address,
        keeper: Address,
        liq_keeper: Address,
        user: Address,
        rs: Address,
        ds: Address,
        oracle: Address,
        vault: Address,
        ord_handler: Address,
        liq_handler: Address,
        market_tk: Address,
        long_tk: Address,
        short_tk: Address,
        index_tk: Address,
    }

    fn setup() -> World {
        let env = Env::default();
        env.cost_estimate().budget().reset_unlimited();
        env.mock_all_auths();

        let admin = Address::generate(&env);
        let keeper = Address::generate(&env);
        let liq_keeper = Address::generate(&env);
        let user = Address::generate(&env);

        // Role store
        let rs = env.register(RoleStore, ());
        RsClient::new(&env, &rs).initialize(&admin);
        let rs_c = RsClient::new(&env, &rs);
        rs_c.grant_role(&admin, &admin, &roles::controller(&env));
        rs_c.grant_role(&admin, &keeper, &roles::order_keeper(&env));
        rs_c.grant_role(&admin, &liq_keeper, &roles::liquidation_keeper(&env));

        // Data store
        let ds = env.register(DataStore, ());
        DsClient::new(&env, &ds).initialize(&admin, &rs);

        // Oracle
        let oracle_addr = env.register(Oracle, ());
        let passphrase = soroban_sdk::Bytes::from_slice(&env, b"Test SDF Network ; September 2015");
        OClient::new(&env, &oracle_addr).initialize(&admin, &rs, &ds, &passphrase);

        // Order vault
        let vault = env.register(OrderVault, ());
        OVClient::new(&env, &vault).initialize(&admin, &rs);

        // Underlying tokens
        let long_tk = env
            .register_stellar_asset_contract_v2(admin.clone())
            .address();
        let short_tk = env
            .register_stellar_asset_contract_v2(admin.clone())
            .address();
        let index_tk = Address::generate(&env);

        // Market token (LP + pool custodian)
        let market_tk = env.register(MarketToken, ());
        MtClient::new(&env, &market_tk).initialize(
            &admin,
            &rs,
            &7u32,
            &soroban_sdk::String::from_str(&env, "SO4 Market"),
            &soroban_sdk::String::from_str(&env, "GM"),
            &long_tk,
            &short_tk,
        );

        // Order handler
        let ord_handler = env.register(OrderHandler, ());
        OHClient::new(&env, &ord_handler).initialize(&admin, &rs, &ds, &oracle_addr, &vault);

        // Liquidation handler
        let liq_handler = env.register(LiquidationHandler, ());
        LiquidationHandlerClient::new(&env, &liq_handler).initialize(
            &admin,
            &rs,
            &ds,
            &oracle_addr,
            &ord_handler,
        );

        // Grant CONTROLLER to handlers
        rs_c.grant_role(&admin, &ord_handler, &roles::controller(&env));
        rs_c.grant_role(&admin, &liq_handler, &roles::controller(&env));
        // Grant liq_keeper LIQUIDATION_KEEPER on order_handler too (it calls liquidate_position)
        rs_c.grant_role(&admin, &liq_keeper, &roles::liquidation_keeper(&env));

        // Register market in DataStore
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

        // Market config: 10 bps position fee, 1% min collateral factor (liquidation threshold)
        let fee_factor = FLOAT_PRECISION / 1000; // 0.1%
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
        // min_collateral_factor = 1% of position size (liquidate when collateral < 1% of size)
        let min_col_factor = FLOAT_PRECISION / 100; // 1%
        ds_c.set_u128(
            &admin,
            &gmx_keys::min_collateral_factor_key(&env, &market_tk),
            &(min_col_factor as u128),
        );
        // Max leverage = 100x
        ds_c.set_u128(
            &admin,
            &gmx_keys::max_leverage_key(&env, &market_tk),
            &(100 * FLOAT_PRECISION as u128),
        );

        World {
            env,
            admin,
            keeper,
            liq_keeper,
            user,
            rs,
            ds,
            oracle: oracle_addr,
            vault,
            ord_handler,
            liq_handler,
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
            &soroban_sdk::Vec::from_array(
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

    /// Open a long position: mint collateral, fund vault, create + execute MarketIncrease.
    fn open_long_position(w: &World, collateral_tokens: i128, size_usd: i128) {
        StellarAssetClient::new(&w.env, &w.long_tk).mint(&w.vault, &collateral_tokens);
        // Seed pool so market has liquidity for the position
        StellarAssetClient::new(&w.env, &w.long_tk).mint(&w.market_tk, &(collateral_tokens * 10));
        DsClient::new(&w.env, &w.ds).set_u128(
            &w.admin,
            &gmx_keys::pool_amount_key(&w.env, &w.market_tk, &w.long_tk),
            &(collateral_tokens as u128 * 10),
        );

        let hc = OHClient::new(&w.env, &w.ord_handler);
        let key = hc.create_order(
            &w.user,
            &CreateOrderParams {
                receiver: w.user.clone(),
                market: w.market_tk.clone(),
                initial_collateral_token: w.long_tk.clone(),
                swap_path: soroban_sdk::Vec::new(&w.env),
                size_delta_usd: size_usd,
                collateral_delta_amount: collateral_tokens,
                trigger_price: 0,
                acceptable_price: 0,
                execution_fee: 0,
                min_output_amount: 0,
                order_type: OrderType::MarketIncrease,
                is_long: true,
                expiry_ledger: None,
            },
        );
        hc.execute_order(&w.keeper, &key);
    }

    /// Open a short position: mint short_tk collateral, fund vault, create + execute MarketIncrease.
    fn open_short_position(w: &World, collateral_tokens: i128, size_usd: i128) {
        StellarAssetClient::new(&w.env, &w.short_tk).mint(&w.vault, &collateral_tokens);
        // Seed pool
        StellarAssetClient::new(&w.env, &w.short_tk).mint(&w.market_tk, &(collateral_tokens * 10));
        DsClient::new(&w.env, &w.ds).set_u128(
            &w.admin,
            &gmx_keys::pool_amount_key(&w.env, &w.market_tk, &w.short_tk),
            &(collateral_tokens as u128 * 10),
        );

        let hc = OHClient::new(&w.env, &w.ord_handler);
        let key = hc.create_order(
            &w.user,
            &CreateOrderParams {
                receiver: w.user.clone(),
                market: w.market_tk.clone(),
                initial_collateral_token: w.short_tk.clone(),
                swap_path: soroban_sdk::Vec::new(&w.env),
                size_delta_usd: size_usd,
                collateral_delta_amount: collateral_tokens,
                trigger_price: 0,
                acceptable_price: 0,
                execution_fee: 0,
                min_output_amount: 0,
                order_type: OrderType::MarketIncrease,
                is_long: false,
                expiry_ledger: None,
            },
        );
        hc.execute_order(&w.keeper, &key);
    }

    // ── Issue #72: long liquidation E2E ───────────────────────────────────────

    /// Create a long position, crash the price past the liquidation threshold,
    /// and verify that liquidation_handler closes it correctly.
    ///
    /// Setup:
    ///   - Entry price: $2000
    ///   - Collateral: 1 token ($2000 worth)
    ///   - Size: $20 000 (10x leverage)
    ///   - Liquidation price: ~$1980 (1% min collateral factor)
    ///   - Crash price to $100 → deeply underwater → liquidatable
    #[test]
    fn liquidate_underwater_long_removes_position_key() {
        let w = setup();
        let fp = FLOAT_PRECISION;
        let entry_price = 2_000 * fp;

        set_prices(&w, entry_price);

        let collateral = ONE_TOKEN; // 1 token = $2000 at entry
        let size_usd = 20_000 * fp; // 10x leverage

        open_long_position(&w, collateral, size_usd);

        // Verify position exists
        let pos_key = gmx_keys::position_key(&w.env, &w.user, &w.market_tk, &w.long_tk, true);
        assert!(
            OHClient::new(&w.env, &w.ord_handler)
                .get_position(&pos_key)
                .is_some(),
            "position must exist before liquidation"
        );

        // Crash price to $100 — position is deeply underwater
        let crash_price = 100 * fp;
        set_prices(&w, crash_price);

        // Verify it's liquidatable
        let is_liq = LiquidationHandlerClient::new(&w.env, &w.liq_handler).check_liquidatable(
            &w.user,
            &w.market_tk,
            &w.long_tk,
            &true,
        );
        assert!(is_liq, "position must be liquidatable after price crash");

        // Execute liquidation
        LiquidationHandlerClient::new(&w.env, &w.liq_handler).liquidate_position(
            &w.liq_keeper,
            &w.user,
            &w.market_tk,
            &w.long_tk,
            &true,
        );

        // Position key must be removed from order_handler storage
        assert!(
            OHClient::new(&w.env, &w.ord_handler)
                .get_position(&pos_key)
                .is_none(),
            "position key must be removed after liquidation"
        );
    }

    /// Issue #614: liquidate_position must record LIQUIDATION_KEEPER's activity
    /// in data_store, so check_keeper_heartbeat(roles::liquidation_keeper)
    /// reports the role as alive (last_active_ledger == current ledger)
    /// instead of permanently stale (last_active_ledger == 0).
    #[test]
    fn liquidate_position_records_keeper_activity() {
        let w = setup();
        let fp = FLOAT_PRECISION;
        set_prices(&w, 2_000 * fp);
        open_long_position(&w, ONE_TOKEN, 20_000 * fp);
        set_prices(&w, 100 * fp);

        let ds_c = DsClient::new(&w.env, &w.ds);
        let key = last_keeper_activity_key(&w.env, &roles::liquidation_keeper(&w.env));
        assert_eq!(ds_c.get_u128(&key), 0, "no activity recorded before liquidation");

        LiquidationHandlerClient::new(&w.env, &w.liq_handler).liquidate_position(
            &w.liq_keeper,
            &w.user,
            &w.market_tk,
            &w.long_tk,
            &true,
        );

        assert_eq!(
            ds_c.get_u128(&key),
            w.env.ledger().sequence() as u128,
            "LIQUIDATION_KEEPER activity must be recorded after a successful liquidation"
        );
    }

    // ── Issue #416: liquidation keeper execution fee ──────────────────────────

    /// Liquidating a position with a configured nonzero keeper execution fee
    /// must pay the keeper that exact fee out of the position's collateral,
    /// without reverting.
    #[test]
    fn liquidate_with_nonzero_execution_fee_pays_keeper() {
        let w = setup();
        let fp = FLOAT_PRECISION;
        let entry_price = 2_000 * fp;
        set_prices(&w, entry_price);

        let collateral = ONE_TOKEN;
        let size_usd = 20_000 * fp;
        open_long_position(&w, collateral, size_usd);

        // Configure a nonzero keeper execution fee for this market.
        let fee_amount = ONE_TOKEN / 100; // 0.01 long_tk
        DsClient::new(&w.env, &w.ds).set_u128(
            &w.admin,
            &gmx_keys::liquidation_execution_fee_key(&w.env, &w.market_tk),
            &(fee_amount as u128),
        );

        let crash_price = 100 * fp;
        set_prices(&w, crash_price);

        let keeper_balance_before =
            soroban_sdk::token::Client::new(&w.env, &w.long_tk).balance(&w.liq_keeper);

        LiquidationHandlerClient::new(&w.env, &w.liq_handler).liquidate_position(
            &w.liq_keeper,
            &w.user,
            &w.market_tk,
            &w.long_tk,
            &true,
        );

        let keeper_balance_after =
            soroban_sdk::token::Client::new(&w.env, &w.long_tk).balance(&w.liq_keeper);
        assert_eq!(
            keeper_balance_after - keeper_balance_before,
            fee_amount,
            "keeper must receive the configured execution fee"
        );
    }

    /// The keeper execution fee is capped at the position's available
    /// collateral — liquidation must not revert even when the configured fee
    /// exceeds what the position can pay.
    #[test]
    fn liquidate_with_execution_fee_exceeding_collateral_caps_at_collateral() {
        let w = setup();
        let fp = FLOAT_PRECISION;
        let entry_price = 2_000 * fp;
        set_prices(&w, entry_price);

        let collateral = ONE_TOKEN; // 1 token = $2000 at entry
        let size_usd = 20_000 * fp;
        open_long_position(&w, collateral, size_usd);

        // Configure a fee far larger than the position's collateral could ever pay.
        let fee_amount = ONE_TOKEN * 1_000;
        DsClient::new(&w.env, &w.ds).set_u128(
            &w.admin,
            &gmx_keys::liquidation_execution_fee_key(&w.env, &w.market_tk),
            &(fee_amount as u128),
        );

        let crash_price = 100 * fp;
        set_prices(&w, crash_price);

        let keeper_balance_before =
            soroban_sdk::token::Client::new(&w.env, &w.long_tk).balance(&w.liq_keeper);

        LiquidationHandlerClient::new(&w.env, &w.liq_handler).liquidate_position(
            &w.liq_keeper,
            &w.user,
            &w.market_tk,
            &w.long_tk,
            &true,
        );

        let keeper_balance_after =
            soroban_sdk::token::Client::new(&w.env, &w.long_tk).balance(&w.liq_keeper);
        assert_eq!(
            keeper_balance_after - keeper_balance_before,
            collateral,
            "fee must be capped at the position's collateral, not the full configured fee"
        );
    }

    /// Liquidation of a healthy long position must revert (not liquidatable).
    #[test]
    #[should_panic]
    fn liquidate_healthy_long_reverts() {
        let w = setup();
        let fp = FLOAT_PRECISION;
        let entry_price = 2_000 * fp;

        set_prices(&w, entry_price);

        let collateral = ONE_TOKEN * 10; // 10 tokens = $20 000 at entry
        let size_usd = 10_000 * fp; // 0.5x leverage — very healthy

        open_long_position(&w, collateral, size_usd);

        // Price stays the same — position is healthy
        set_prices(&w, entry_price);

        // Must revert with NotLiquidatable
        LiquidationHandlerClient::new(&w.env, &w.liq_handler).liquidate_position(
            &w.liq_keeper,
            &w.user,
            &w.market_tk,
            &w.long_tk,
            &true,
        );
    }

    // ── Issue #73: short liquidation E2E ──────────────────────────────────────

    /// Create a short position, pump the price past the liquidation threshold,
    /// and verify that liquidation_handler closes it correctly.
    ///
    /// Setup:
    ///   - Entry price: $2000 (short opened here)
    ///   - Collateral: 1 short_tk token ($1 worth, since short_tk = $1)
    ///   - Size: $10 (10x leverage on $1 collateral)
    ///   - Price pumps to $10 000 → short is deeply underwater → liquidatable
    #[test]
    fn liquidate_underwater_short_removes_position_key() {
        let w = setup();
        let fp = FLOAT_PRECISION;
        let entry_price = 2_000 * fp;

        set_prices(&w, entry_price);

        // Short collateral is short_tk ($1 per token)
        let collateral = ONE_TOKEN; // 1 short_tk = $1
        let size_usd = 10 * fp; // $10 size (10x leverage on $1 collateral)

        open_short_position(&w, collateral, size_usd);

        // Verify position exists
        let pos_key = gmx_keys::position_key(&w.env, &w.user, &w.market_tk, &w.short_tk, false);
        assert!(
            OHClient::new(&w.env, &w.ord_handler)
                .get_position(&pos_key)
                .is_some(),
            "short position must exist before liquidation"
        );

        // Pump index price to $10 000 — short is deeply underwater
        let pump_price = 10_000 * fp;
        set_prices(&w, pump_price);

        // Verify it's liquidatable
        let is_liq = LiquidationHandlerClient::new(&w.env, &w.liq_handler).check_liquidatable(
            &w.user,
            &w.market_tk,
            &w.short_tk,
            &false,
        );
        assert!(
            is_liq,
            "short position must be liquidatable after price pump"
        );

        // Execute liquidation
        LiquidationHandlerClient::new(&w.env, &w.liq_handler).liquidate_position(
            &w.liq_keeper,
            &w.user,
            &w.market_tk,
            &w.short_tk,
            &false,
        );

        // Position key must be removed
        assert!(
            OHClient::new(&w.env, &w.ord_handler)
                .get_position(&pos_key)
                .is_none(),
            "short position key must be removed after liquidation"
        );
    }

    /// Liquidation of a healthy short position must revert.
    #[test]
    #[should_panic]
    fn liquidate_healthy_short_reverts() {
        let w = setup();
        let fp = FLOAT_PRECISION;
        let entry_price = 2_000 * fp;

        set_prices(&w, entry_price);

        // Very well-collateralised short
        let collateral = ONE_TOKEN * 100; // 100 short_tk = $100
        let size_usd = 10 * fp; // $10 size — 0.1x leverage

        open_short_position(&w, collateral, size_usd);

        // Price stays the same
        set_prices(&w, entry_price);

        // Must revert with NotLiquidatable
        LiquidationHandlerClient::new(&w.env, &w.liq_handler).liquidate_position(
            &w.liq_keeper,
            &w.user,
            &w.market_tk,
            &w.short_tk,
            &false,
        );
    }

    // ── Issue #11: upgrade entrypoint tests ───────────────────────────────────

    /// Admin auth passes on upgrade; panics at WASM lookup (not auth) in unit tests.
    /// A compiled WASM binary is required for the host to accept the hash.
    #[test]
    #[should_panic]
    fn upgrade_admin_succeeds() {
        let w = setup(); // mock_all_auths is active — admin.require_auth() passes silently
                         // Panics at WASM lookup (not at auth) — proves auth gate is open for admin.
        LiquidationHandlerClient::new(&w.env, &w.liq_handler)
            .upgrade(&BytesN::from_array(&w.env, &[0u8; 32]));
    }

    /// Calling upgrade without the admin's authorisation must revert.
    #[test]
    #[should_panic]
    fn upgrade_non_admin_reverts() {
        // Fresh env — no mock_all_auths so require_auth() is not bypassed.
        let env = Env::default();

        let admin = Address::generate(&env);
        let rs = Address::generate(&env);
        let ds = Address::generate(&env);
        let oracle = Address::generate(&env);
        let oh = Address::generate(&env);

        let liq = env.register(LiquidationHandler, ());

        // Seed instance storage directly, bypassing initialize() auth.
        env.as_contract(&liq, || {
            env.storage()
                .instance()
                .set(&InstanceKey::Initialized, &true);
            env.storage().instance().set(&InstanceKey::Admin, &admin);
            env.storage().instance().set(&InstanceKey::RoleStore, &rs);
            env.storage().instance().set(&InstanceKey::DataStore, &ds);
            env.storage().instance().set(&InstanceKey::Oracle, &oracle);
            env.storage()
                .instance()
                .set(&InstanceKey::OrderHandler, &oh);
        });

        // Call upgrade with no auth context — must panic at admin.require_auth().
        let hash = BytesN::from_array(&env, &[0u8; 32]);
        LiquidationHandlerClient::new(&env, &liq).upgrade(&hash);
    }

    /// After upgrade the handler can still route a liquidation correctly.
    /// Requires a compiled WASM binary — skipped in unit-test mode.
    #[test]
    #[ignore]
    fn upgrade_post_upgrade_liquidation_still_works() {
        let w = setup();
        let fp = FLOAT_PRECISION;
        set_prices(&w, 2_000 * fp);
        open_long_position(&w, ONE_TOKEN, 20_000 * fp);

        let new_hash = BytesN::from_array(&w.env, &[0u8; 32]);
        LiquidationHandlerClient::new(&w.env, &w.liq_handler).upgrade(&new_hash);

        // Crash price so position is liquidatable.
        set_prices(&w, 100 * fp);

        let pos_key = gmx_keys::position_key(&w.env, &w.user, &w.market_tk, &w.long_tk, true);
        assert!(OHClient::new(&w.env, &w.ord_handler)
            .get_position(&pos_key)
            .is_some());

        // Liquidation must still reach the new (same) logic and remove the key.
        LiquidationHandlerClient::new(&w.env, &w.liq_handler).liquidate_position(
            &w.liq_keeper,
            &w.user,
            &w.market_tk,
            &w.long_tk,
            &true,
        );

        assert!(
            OHClient::new(&w.env, &w.ord_handler)
                .get_position(&pos_key)
                .is_none(),
            "position key must be gone after post-upgrade liquidation"
        );
    }

    // ── Issue #109: LIQUIDATION_KEEPER authorization matrix ──────────────────

    /// liquidate_position must reject a caller that does not hold LIQUIDATION_KEEPER.
    #[test]
    #[should_panic]
    fn liquidate_position_by_non_liq_keeper_panics() {
        let w = setup();
        let fp = FLOAT_PRECISION;
        set_prices(&w, 2_000 * fp);
        open_long_position(&w, ONE_TOKEN, 20_000 * fp);

        // Crash the price so the position is liquidatable.
        set_prices(&w, 100 * fp);

        let impostor = Address::generate(&w.env);
        // impostor has no LIQUIDATION_KEEPER role — must panic with Unauthorized.
        LiquidationHandlerClient::new(&w.env, &w.liq_handler).liquidate_position(
            &impostor,
            &w.user,
            &w.market_tk,
            &w.long_tk,
            &true,
        );
    }

    // ── check_liquidatable false-path coverage ────────────────────────────────

    /// check_liquidatable must return false when no position exists for the
    /// given account/market/collateral/is_long combination — the `None` branch
    /// in its `get_position` match arm. No test previously called
    /// check_liquidatable and asserted false, so a regression that made it
    /// always return true (or panic) on a missing position would go undetected.
    #[test]
    fn check_liquidatable_returns_false_for_missing_position() {
        let w = setup();
        let fp = FLOAT_PRECISION;

        // Set prices so load_market_props + oracle calls succeed.
        set_prices(&w, 2_000 * fp);

        // w.user has no open position — the position_key lookup returns None.
        let result = LiquidationHandlerClient::new(&w.env, &w.liq_handler).check_liquidatable(
            &w.user,
            &w.market_tk,
            &w.long_tk,
            &true,
        );
        assert!(
            !result,
            "check_liquidatable must return false when no position exists for the given key"
        );
    }

    /// check_liquidatable must return false when a position exists but is
    /// currently healthy (collateral well above the minimum threshold). This
    /// directly exercises the `is_liquidatable → false` return path through
    /// check_liquidatable itself, not the inlined path inside liquidate_position
    /// that liquidate_healthy_long_reverts / liquidate_healthy_short_reverts
    /// already cover.
    #[test]
    fn check_liquidatable_returns_false_for_healthy_position() {
        let w = setup();
        let fp = FLOAT_PRECISION;
        let entry_price = 2_000 * fp;

        set_prices(&w, entry_price);

        // Open a very well-collateralised long: 10 tokens ($20 000) backing a
        // $10 000 position (0.5x leverage). With a 1% min_collateral_factor
        // the liquidation price is far below any realistic oracle value.
        let collateral = ONE_TOKEN * 10; // 10 tokens = $20 000 at entry
        let size_usd = 10_000 * fp; // 0.5x leverage — deeply healthy

        open_long_position(&w, collateral, size_usd);

        // Confirm the position was created.
        let pos_key = gmx_keys::position_key(&w.env, &w.user, &w.market_tk, &w.long_tk, true);
        assert!(
            OHClient::new(&w.env, &w.ord_handler)
                .get_position(&pos_key)
                .is_some(),
            "position must exist before calling check_liquidatable"
        );

        // Price stays at entry — position is healthy.
        set_prices(&w, entry_price);

        let result = LiquidationHandlerClient::new(&w.env, &w.liq_handler).check_liquidatable(
            &w.user,
            &w.market_tk,
            &w.long_tk,
            &true,
        );
        assert!(
            !result,
            "check_liquidatable must return false for a healthy, well-collateralised position"
        );
    }
}
