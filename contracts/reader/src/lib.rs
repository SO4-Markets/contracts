//! Reader — read-only view contract for aggregating protocol state.
//! Mirrors GMX's Reader.sol.
//!
//! Aggregates data across data_store, oracle, and position/market utils
//! into rich structs the frontend consumes without needing multiple calls.
//! All functions are view-only — no writes, no auth.
#![no_std]
#![allow(dependency_on_unit_never_type_fallback)]

use gmx_keys::{
    account_deposit_list_key, account_order_list_key, account_position_list_key,
    account_withdrawal_list_key, deposit_list_key, funding_amount_per_size_key,
    market_index_token_key, market_long_token_key, market_short_token_key, order_list_key,
    position_key, saved_funding_factor_per_second_key, withdrawal_list_key,
};
use gmx_market_utils::{get_open_interest_for_side, get_pool_value};
use gmx_math::{mul_div_wide, TOKEN_PRECISION};
use gmx_position_utils::{get_position_fees, get_position_pnl_usd, is_liquidatable};
use gmx_pricing_utils::{get_execution_price, get_position_price_impact};
use gmx_types::{
    DepositProps, FundingInfo, MarketProps, OrderProps, PoolValueInfo, PositionFees, PositionInfo,
    PositionProps, PriceProps, WithdrawalProps,
};
use soroban_sdk::{
    contract, contracterror, contractimpl, contracttype, panic_with_error, Address, BytesN, Env,
    Vec,
};

// ─── Storage keys ─────────────────────────────────────────────────────────────

#[contracttype]
enum InstanceKey {
    Initialized,
    Admin,
}

// ─── Errors ───────────────────────────────────────────────────────────────────

#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
#[repr(u32)]
pub enum Error {
    NotInitialized = 1,
    AlreadyInitialized = 2,
    Unauthorized = 3,
}

// ─── External clients ─────────────────────────────────────────────────────────

#[allow(dead_code)]
#[soroban_sdk::contractclient(name = "DataStoreClient")]
trait IDataStore {
    fn get_u128(env: Env, key: BytesN<32>) -> u128;
    fn get_i128(env: Env, key: BytesN<32>) -> i128;
    fn get_address(env: Env, key: BytesN<32>) -> Option<Address>;
    fn get_bytes32_set_count(env: Env, key: BytesN<32>) -> u32;
    fn get_bytes32_set_at(env: Env, key: BytesN<32>, start: u32, end: u32) -> Vec<BytesN<32>>;
}

#[allow(dead_code)]
#[soroban_sdk::contractclient(name = "OracleClient")]
trait IOracle {
    fn get_primary_price(env: Env, token: Address) -> PriceProps;
}

#[allow(dead_code)]
#[soroban_sdk::contractclient(name = "OrderHandlerClient")]
trait IOrderHandler {
    fn get_position(env: Env, key: BytesN<32>) -> Option<PositionProps>;
    fn get_order(env: Env, key: BytesN<32>) -> Option<OrderProps>;
}

#[allow(dead_code)]
#[soroban_sdk::contractclient(name = "DepositHandlerClient")]
trait IDepositHandler {
    fn get_deposit(env: Env, key: BytesN<32>) -> Option<DepositProps>;
}

#[allow(dead_code)]
#[soroban_sdk::contractclient(name = "WithdrawalHandlerClient")]
trait IWithdrawalHandler {
    fn get_withdrawal(env: Env, key: BytesN<32>) -> Option<WithdrawalProps>;
}

// ─── Contract ─────────────────────────────────────────────────────────────────

#[contract]
pub struct Reader;

#[contractimpl]
impl Reader {
    /// One-time setup — store the admin address.
    pub fn initialize(env: Env, admin: Address) {
        admin.require_auth();
        if env.storage().instance().has(&InstanceKey::Initialized) {
            panic_with_error!(&env, Error::AlreadyInitialized);
        }
        env.storage()
            .instance()
            .set(&InstanceKey::Initialized, &true);
        env.storage().instance().set(&InstanceKey::Admin, &admin);
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

    // ── Market views ─────────────────────────────────────────────────────────

    /// Load full MarketProps for a given market_token address from data_store.
    pub fn get_market(env: Env, data_store: Address, market_token: Address) -> MarketProps {
        let ds = DataStoreClient::new(&env, &data_store);
        let index_token = ds
            .get_address(&market_index_token_key(&env, &market_token))
            .expect("market index token not found");
        let long_token = ds
            .get_address(&market_long_token_key(&env, &market_token))
            .expect("market long token not found");
        let short_token = ds
            .get_address(&market_short_token_key(&env, &market_token))
            .expect("market short token not found");
        MarketProps {
            market_token,
            index_token,
            long_token,
            short_token,
        }
    }

    /// Get the full pool value breakdown for a market at current oracle prices.
    pub fn get_market_pool_value_info(
        env: Env,
        data_store: Address,
        oracle: Address,
        market_token: Address,
        maximize: bool,
    ) -> PoolValueInfo {
        let market = Self::get_market(env.clone(), data_store.clone(), market_token);
        let oracle_client = OracleClient::new(&env, &oracle);
        let long_price = oracle_client
            .get_primary_price(&market.long_token)
            .mid_price();
        let short_price = oracle_client
            .get_primary_price(&market.short_token)
            .mid_price();
        let index_price = oracle_client
            .get_primary_price(&market.index_token)
            .mid_price();
        get_pool_value(
            &env,
            &data_store,
            &market,
            long_price,
            short_price,
            index_price,
            maximize,
        )
    }

    /// Get open interest for both sides of a market.
    /// Returns (long_oi_usd, short_oi_usd).
    pub fn get_open_interest(env: Env, data_store: Address, market_token: Address) -> (i128, i128) {
        let market = Self::get_market(env.clone(), data_store.clone(), market_token);
        let long_oi = get_open_interest_for_side(&env, &data_store, &market, true) as i128;
        let short_oi = get_open_interest_for_side(&env, &data_store, &market, false) as i128;
        (long_oi, short_oi)
    }

    /// Get the aggregate funding state for a market.
    pub fn get_funding_info(env: Env, data_store: Address, market_token: Address) -> FundingInfo {
        let market = Self::get_market(env.clone(), data_store.clone(), market_token.clone());
        let ds = DataStoreClient::new(&env, &data_store);

        let funding_factor_per_second =
            ds.get_i128(&saved_funding_factor_per_second_key(&env, &market_token));
        // Long side tracks funding in long_token collateral; short in short_token
        let long_funding_amount_per_size = ds.get_i128(&funding_amount_per_size_key(
            &env,
            &market_token,
            &market.long_token,
            true,
        ));
        let short_funding_amount_per_size = ds.get_i128(&funding_amount_per_size_key(
            &env,
            &market_token,
            &market.short_token,
            false,
        ));

        FundingInfo {
            funding_factor_per_second,
            long_funding_amount_per_size,
            short_funding_amount_per_size,
        }
    }

    // ── Position views ────────────────────────────────────────────────────────

    /// Get a single position enriched with PnL, fees, and liquidation price.
    ///
    /// Reads position from the canonical location (order_handler storage) via cross-contract call.
    /// This ensures all consumers (liquidation_handler, adl_handler, reader) agree on position state.
    pub fn get_position_info(
        env: Env,
        data_store: Address,
        oracle: Address,
        order_handler: Address,
        account: Address,
        market: Address,
        collateral_token: Address,
        is_long: bool,
    ) -> Option<PositionInfo> {
        // Read position from canonical location (order_handler storage)
        let pk = position_key(&env, &account, &market, &collateral_token, is_long);
        let position: PositionProps =
            match OrderHandlerClient::new(&env, &order_handler).get_position(&pk) {
                Some(p) => p,
                None => return None,
            };

        let market_props =
            Self::get_market(env.clone(), data_store.clone(), position.market.clone());
        let oracle_client = OracleClient::new(&env, &oracle);

        let index_price = oracle_client.get_primary_price(&market_props.index_token);
        let collateral_price = oracle_client
            .get_primary_price(&position.collateral_token)
            .mid_price();

        // PnL for the full position size
        let (pnl_usd, uncapped_pnl_usd) =
            get_position_pnl_usd(&env, &position, &index_price, position.size_in_usd);

        // Fees in collateral token units
        let fees: PositionFees = get_position_fees(
            &env,
            &data_store,
            &market_props,
            &position,
            collateral_price,
            position.size_in_usd,
            false,
        );

        // Convert fee amounts (collateral token raw) → USD (FLOAT_PRECISION)
        let borrowing_fee_usd = mul_div_wide(
            &env,
            fees.borrowing_fee_amount,
            collateral_price,
            TOKEN_PRECISION,
        );
        let funding_fee_usd = mul_div_wide(
            &env,
            fees.funding_fee_amount,
            collateral_price,
            TOKEN_PRECISION,
        );
        let position_fee_usd = mul_div_wide(
            &env,
            fees.position_fee_amount,
            collateral_price,
            TOKEN_PRECISION,
        );

        // Approximate liquidation price:
        // For a long:  liq_price = (size_usd - collateral_usd + fees_usd) / size_in_tokens × TOKEN_PRECISION
        // For a short: liq_price = (size_usd + collateral_usd - fees_usd) / size_in_tokens × TOKEN_PRECISION
        let collateral_usd = mul_div_wide(
            &env,
            position.collateral_amount,
            collateral_price,
            TOKEN_PRECISION,
        );
        let total_fees_usd = borrowing_fee_usd + funding_fee_usd + position_fee_usd;

        let liquidation_price = if position.size_in_tokens > 0 {
            let numerator = if position.is_long {
                position.size_in_usd - collateral_usd + total_fees_usd
            } else {
                position.size_in_usd + collateral_usd - total_fees_usd
            };
            if numerator > 0 {
                mul_div_wide(&env, numerator, TOKEN_PRECISION, position.size_in_tokens)
            } else {
                0
            }
        } else {
            0
        };

        Some(PositionInfo {
            position,
            pnl_usd,
            uncapped_pnl_usd,
            borrowing_fee_usd,
            funding_fee_usd,
            position_fee_usd,
            liquidation_price,
        })
    }

    /// Compute the execution price a user would get for a given size and order direction.
    ///
    /// Useful for the UI to preview slippage before placing an order.
    pub fn get_execution_price_preview(
        env: Env,
        data_store: Address,
        oracle: Address,
        market_token: Address,
        is_long: bool,
        is_increase: bool,
        size_delta_usd: i128,
    ) -> i128 {
        let market = Self::get_market(env.clone(), data_store.clone(), market_token);
        let oracle_client = OracleClient::new(&env, &oracle);
        let index_price = oracle_client
            .get_primary_price(&market.index_token)
            .mid_price();

        let impact_usd = get_position_price_impact(
            &env,
            &data_store,
            &market,
            is_long,
            size_delta_usd,
            is_increase,
            index_price,
        );

        get_execution_price(
            &env,
            index_price,
            size_delta_usd,
            impact_usd,
            is_long,
            is_increase,
        )
    }

    /// Return whether a position is currently liquidatable at oracle prices.
    ///
    /// Reads position from the canonical location (order_handler storage) via cross-contract call.
    pub fn is_position_liquidatable(
        env: Env,
        data_store: Address,
        oracle: Address,
        order_handler: Address,
        account: Address,
        market: Address,
        collateral_token: Address,
        is_long: bool,
    ) -> bool {
        // Read position from canonical location (order_handler storage)
        let pk = position_key(&env, &account, &market, &collateral_token, is_long);
        let position: PositionProps =
            match OrderHandlerClient::new(&env, &order_handler).get_position(&pk) {
                Some(p) => p,
                None => return false,
            };

        let market_props =
            Self::get_market(env.clone(), data_store.clone(), position.market.clone());
        let oracle_client = OracleClient::new(&env, &oracle);
        let index_price = oracle_client.get_primary_price(&market_props.index_token);
        let collateral_price = oracle_client
            .get_primary_price(&position.collateral_token)
            .mid_price();
        is_liquidatable(
            &env,
            &data_store,
            &position,
            &market_props,
            collateral_price,
            &index_price,
        )
    }

    /// Return a stored order by key, or None if not found.
    pub fn get_order(env: Env, order_handler: Address, key: BytesN<32>) -> Option<OrderProps> {
        OrderHandlerClient::new(&env, &order_handler).get_order(&key)
    }

    /// Return paginated orders for an account.
    pub fn get_account_orders(
        env: Env,
        data_store: Address,
        order_handler: Address,
        account: Address,
        page: u32,
        page_size: u32,
    ) -> Vec<OrderProps> {
        let ds = DataStoreClient::new(&env, &data_store);
        let set_key = account_order_list_key(&env, &account);
        if page == 0 || page_size == 0 {
            return Vec::new(&env);
        }
        let start = (page - 1).saturating_mul(page_size);
        let end = start.saturating_add(page_size);

        let keys: Vec<BytesN<32>> = ds.get_bytes32_set_at(&set_key, &start, &end);
        let mut out: Vec<OrderProps> = Vec::new(&env);
        for i in 0..keys.len() {
            let k = keys.get_unchecked(i);
            if let Some(o) = OrderHandlerClient::new(&env, &order_handler).get_order(&k) {
                out.push_back(o);
            }
        }
        out
    }

    /// Get position info by canonical position key (BytesN<32>), returning enriched `PositionInfo`.
    pub fn get_position_info_by_key(
        env: Env,
        data_store: Address,
        oracle: Address,
        order_handler: Address,
        position_key: BytesN<32>,
    ) -> Option<PositionInfo> {
        let position: PositionProps =
            match OrderHandlerClient::new(&env, &order_handler).get_position(&position_key) {
                Some(p) => p,
                None => return None,
            };

        let market_props =
            Self::get_market(env.clone(), data_store.clone(), position.market.clone());
        let oracle_client = OracleClient::new(&env, &oracle);

        let index_price = oracle_client.get_primary_price(&market_props.index_token);
        let collateral_price = oracle_client
            .get_primary_price(&position.collateral_token)
            .mid_price();

        let (pnl_usd, uncapped_pnl_usd) =
            get_position_pnl_usd(&env, &position, &index_price, position.size_in_usd);

        let fees: PositionFees = get_position_fees(
            &env,
            &data_store,
            &market_props,
            &position,
            collateral_price,
            position.size_in_usd,
            false,
        );

        let borrowing_fee_usd = mul_div_wide(
            &env,
            fees.borrowing_fee_amount,
            collateral_price,
            TOKEN_PRECISION,
        );
        let funding_fee_usd = mul_div_wide(
            &env,
            fees.funding_fee_amount,
            collateral_price,
            TOKEN_PRECISION,
        );
        let position_fee_usd = mul_div_wide(
            &env,
            fees.position_fee_amount,
            collateral_price,
            TOKEN_PRECISION,
        );

        let collateral_usd = mul_div_wide(
            &env,
            position.collateral_amount,
            collateral_price,
            TOKEN_PRECISION,
        );
        let total_fees_usd = borrowing_fee_usd + funding_fee_usd + position_fee_usd;

        let liquidation_price = if position.size_in_tokens > 0 {
            let numerator = if position.is_long {
                position.size_in_usd - collateral_usd + total_fees_usd
            } else {
                position.size_in_usd + collateral_usd - total_fees_usd
            };
            if numerator > 0 {
                mul_div_wide(&env, numerator, TOKEN_PRECISION, position.size_in_tokens)
            } else {
                0
            }
        } else {
            0
        };

        Some(PositionInfo {
            position,
            pnl_usd,
            uncapped_pnl_usd,
            borrowing_fee_usd,
            funding_fee_usd,
            position_fee_usd,
            liquidation_price,
        })
    }

    /// Get a deposit by key (delegates to deposit_handler).
    pub fn get_deposit(
        env: Env,
        deposit_handler: Address,
        key: BytesN<32>,
    ) -> Option<DepositProps> {
        DepositHandlerClient::new(&env, &deposit_handler).get_deposit(&key)
    }

    /// Get a withdrawal by key (delegates to withdrawal_handler).
    pub fn get_withdrawal(
        env: Env,
        withdrawal_handler: Address,
        key: BytesN<32>,
    ) -> Option<WithdrawalProps> {
        WithdrawalHandlerClient::new(&env, &withdrawal_handler).get_withdrawal(&key)
    }

    // ── Deposit key enumeration (issue #27) ──────────────────────────────────

    /// Count of all pending deposit keys in DataStore.
    pub fn get_deposit_count(env: Env, data_store: Address) -> u32 {
        DataStoreClient::new(&env, &data_store).get_bytes32_set_count(&deposit_list_key(&env))
    }

    /// Paginated list of all deposit keys (raw BytesN<32>).
    pub fn get_deposit_keys(
        env: Env,
        data_store: Address,
        start: u32,
        end: u32,
    ) -> Vec<BytesN<32>> {
        DataStoreClient::new(&env, &data_store).get_bytes32_set_at(
            &deposit_list_key(&env),
            &start,
            &end,
        )
    }

    /// Count of pending deposit keys for a specific account.
    pub fn get_account_deposit_count(env: Env, data_store: Address, account: Address) -> u32 {
        DataStoreClient::new(&env, &data_store)
            .get_bytes32_set_count(&account_deposit_list_key(&env, &account))
    }

    /// Paginated list of deposit keys for a specific account.
    pub fn get_account_deposit_keys(
        env: Env,
        data_store: Address,
        account: Address,
        start: u32,
        end: u32,
    ) -> Vec<BytesN<32>> {
        DataStoreClient::new(&env, &data_store).get_bytes32_set_at(
            &account_deposit_list_key(&env, &account),
            &start,
            &end,
        )
    }

    // ── Withdrawal key enumeration (issue #24) ────────────────────────────────

    /// Count of all pending withdrawal keys in DataStore.
    pub fn get_withdrawal_count(env: Env, data_store: Address) -> u32 {
        DataStoreClient::new(&env, &data_store).get_bytes32_set_count(&withdrawal_list_key(&env))
    }

    /// Paginated list of all withdrawal keys (raw BytesN<32>).
    pub fn get_withdrawal_keys(
        env: Env,
        data_store: Address,
        start: u32,
        end: u32,
    ) -> Vec<BytesN<32>> {
        DataStoreClient::new(&env, &data_store).get_bytes32_set_at(
            &withdrawal_list_key(&env),
            &start,
            &end,
        )
    }

    /// Count of pending withdrawal keys for a specific account.
    pub fn get_account_withdrawal_count(env: Env, data_store: Address, account: Address) -> u32 {
        DataStoreClient::new(&env, &data_store)
            .get_bytes32_set_count(&account_withdrawal_list_key(&env, &account))
    }

    /// Paginated list of withdrawal keys for a specific account.
    pub fn get_account_withdrawal_keys(
        env: Env,
        data_store: Address,
        account: Address,
        start: u32,
        end: u32,
    ) -> Vec<BytesN<32>> {
        DataStoreClient::new(&env, &data_store).get_bytes32_set_at(
            &account_withdrawal_list_key(&env, &account),
            &start,
            &end,
        )
    }

    // ── Order key enumeration (issue #25) ─────────────────────────────────────

    /// Count of all pending order keys in DataStore.
    pub fn get_order_count(env: Env, data_store: Address) -> u32 {
        DataStoreClient::new(&env, &data_store).get_bytes32_set_count(&order_list_key(&env))
    }

    /// Paginated list of all order keys (raw BytesN<32>).
    pub fn get_order_keys(env: Env, data_store: Address, start: u32, end: u32) -> Vec<BytesN<32>> {
        DataStoreClient::new(&env, &data_store).get_bytes32_set_at(
            &order_list_key(&env),
            &start,
            &end,
        )
    }

    /// Count of pending order keys for a specific account.
    pub fn get_account_order_count(env: Env, data_store: Address, account: Address) -> u32 {
        DataStoreClient::new(&env, &data_store)
            .get_bytes32_set_count(&account_order_list_key(&env, &account))
    }

    /// Paginated list of order keys for a specific account.
    pub fn get_account_order_keys(
        env: Env,
        data_store: Address,
        account: Address,
        start: u32,
        end: u32,
    ) -> Vec<BytesN<32>> {
        DataStoreClient::new(&env, &data_store).get_bytes32_set_at(
            &account_order_list_key(&env, &account),
            &start,
            &end,
        )
    }

    /// Return paginated account positions as enriched `PositionInfo` entries.
    pub fn get_account_positions(
        env: Env,
        data_store: Address,
        oracle: Address,
        order_handler: Address,
        account: Address,
        page: u32,
        page_size: u32,
    ) -> Vec<PositionInfo> {
        let ds = DataStoreClient::new(&env, &data_store);
        let set_key = account_position_list_key(&env, &account);
        if page == 0 || page_size == 0 {
            return Vec::new(&env);
        }
        let start = (page - 1).saturating_mul(page_size);
        let end = start.saturating_add(page_size);

        let keys: Vec<BytesN<32>> = ds.get_bytes32_set_at(&set_key, &start, &end);
        let mut out: Vec<PositionInfo> = Vec::new(&env);
        for i in 0..keys.len() {
            let k = keys.get_unchecked(i);
            if let Some(pi) = Self::get_position_info_by_key(
                env.clone(),
                data_store.clone(),
                oracle.clone(),
                order_handler.clone(),
                k,
            ) {
                out.push_back(pi);
            }
        }
        out
    }
}
