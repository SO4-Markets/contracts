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
    account_withdrawal_list_key, claimable_fee_amount_key, deposit_list_key,
    funding_amount_per_size_key, funding_updated_at_key, keeper_heartbeat_timeout_key,
    last_keeper_activity_key, market_index_token_key, market_long_token_key,
    market_short_token_key, open_interest_key, order_list_key, position_key, position_list_key,
    saved_funding_factor_per_second_key, withdrawal_list_key, DEFAULT_KEEPER_HEARTBEAT_TIMEOUT,
};
use gmx_market_utils::{get_open_interest_for_side, get_pool_value};
use gmx_math::{mul_div_wide, TOKEN_PRECISION};
use gmx_position_utils::{get_position_fees, get_position_pnl_usd, is_liquidatable};
use gmx_pricing_utils::{get_execution_price, get_position_price_impact};
use gmx_types::{
    AdlCandidate, DepositProps, FundingInfo, FundingRateInfo, KeeperHeartbeatStatus,
    LiquidatablePosition, MarketProps, OrderProps, PoolValueInfo, PositionFees, PositionInfo,
    PositionLeverage, PositionProps, PriceProps, ProtocolStats, SwapEstimate, WithdrawalProps,
    PendingOrder,
};
use soroban_sdk::{
    contract, contracterror, contractimpl, contracttype, panic_with_error, Address, BytesN, Env,
    Vec,
};

// ─── Constants ────────────────────────────────────────────────────────────────

/// Upper bound on the number of markets `get_protocol_stats` will aggregate in a
/// single call (issue #251). Bounds the per-call instruction cost — each market
/// requires several cross-contract reads — so a large `markets` vec cannot push
/// the call past Soroban's budget. Callers with more markets must paginate by
/// invoking the view across multiple subsets and summing client-side.
const MAX_STATS_MARKETS: u32 = 20;

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
    /// `get_protocol_stats` was passed more than `MAX_STATS_MARKETS` markets.
    TooManyMarkets = 4,
}

// ─── External clients ─────────────────────────────────────────────────────────

#[allow(dead_code)]
#[soroban_sdk::contractclient(name = "DataStoreClient")]
trait IDataStore {
    fn get_u128(env: Env, key: BytesN<32>) -> u128;
    fn get_i128(env: Env, key: BytesN<32>) -> i128;
    fn get_address(env: Env, key: BytesN<32>) -> Option<Address>;
    fn get_bytes32_set_count(env: Env, set_key: BytesN<32>) -> u32;
    fn get_bytes32_set_at(env: Env, set_key: BytesN<32>, start: u32, end: u32) -> Vec<BytesN<32>>;
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

    /// Aggregate protocol-wide statistics across the supplied markets (issue #251).
    ///
    /// Returns total pool value (TVL), long/short open interest, and accumulated
    /// (unclaimed) fees — all in USD at the current oracle prices — plus the
    /// market count and the ledger the snapshot was taken at. Lets the frontend
    /// fetch headline numbers in one call instead of N per-market round-trips.
    ///

        /// Issue #207: per-hour funding rate view for the frontend.
        pub fn get_funding_rate_info(
            env: Env,
            data_store: Address,
            market_token: Address,
        ) -> FundingRateInfo {
            let ds = DataStoreClient::new(&env, &data_store);
            const LEDGERS_PER_HOUR: i128 = 720;

            let factor_key = saved_funding_factor_per_second_key(&env, &market_token);
            let funding_factor_per_second = ds.get_i128(&factor_key);

            let long_funding_rate_per_hour = funding_factor_per_second.saturating_mul(LEDGERS_PER_HOUR);
            let short_funding_rate_per_hour = long_funding_rate_per_hour.saturating_neg();

            let long_fnd_key = funding_amount_per_size_key(&env, &market_token, &market_token, true);
            let short_fnd_key = funding_amount_per_size_key(&env, &market_token, &market_token, false);
            let long_funding_amount_per_size = ds.get_i128(&long_fnd_key);
            let short_funding_amount_per_size = ds.get_i128(&short_fnd_key);

            let updated_at_key = funding_updated_at_key(&env, &market_token);
            let funding_updated_at_ledger = ds.get_u128(&updated_at_key) as u64;

            let long_oi_key = open_interest_key(&env, &market_token, &market_token, true);
            let short_oi_key = open_interest_key(&env, &market_token, &market_token, false);
            let long_open_interest_usd = ds.get_u128(&long_oi_key);
            let short_open_interest_usd = ds.get_u128(&short_oi_key);

            FundingRateInfo {
                long_funding_rate_per_hour,
                short_funding_rate_per_hour,
                long_funding_amount_per_size,
                short_funding_amount_per_size,
                funding_updated_at_ledger,
                long_open_interest_usd,
                short_open_interest_usd,
            }
        }

    /// View-only: reads `data_store` and `oracle`, writes nothing.
    ///
    /// Panics with `TooManyMarkets` if `markets.len() > MAX_STATS_MARKETS`, which
    /// bounds the per-call compute cost. An empty `markets` vec is valid and
    /// returns all-zero stats (with `market_count = 0`). A market whose pool value
    /// is zero simply contributes zero — no special-casing, no panic.
    pub fn get_protocol_stats(
        env: Env,
        data_store: Address,
        oracle: Address,
        markets: Vec<Address>,
    ) -> ProtocolStats {
        if markets.len() > MAX_STATS_MARKETS {
            panic_with_error!(&env, Error::TooManyMarkets);
        }

        let ds = DataStoreClient::new(&env, &data_store);
        let oracle_client = OracleClient::new(&env, &oracle);

        let mut total_pool_value_usd: i128 = 0;
        let mut total_long_open_interest_usd: i128 = 0;
        let mut total_short_open_interest_usd: i128 = 0;
        let mut total_accumulated_fees_usd: i128 = 0;

        for i in 0..markets.len() {
            let market_token = markets.get_unchecked(i);
            let market = Self::get_market(env.clone(), data_store.clone(), market_token.clone());

            let long_price = oracle_client.get_primary_price(&market.long_token);
            let short_price = oracle_client.get_primary_price(&market.short_token);
            let index_price = oracle_client.get_primary_price(&market.index_token);

            // Pool value (TVL contribution). Use the conservative (minimized) pool
            // value so headline TVL never overstates what LPs could withdraw.
            let pool = get_pool_value(
                &env,
                &data_store,
                &market,
                long_price.mid_price(),
                short_price.mid_price(),
                index_price.mid_price(),
                false,
            );
            total_pool_value_usd += pool.pool_value;

            // Open interest is already tracked in USD (FLOAT_PRECISION).
            total_long_open_interest_usd +=
                get_open_interest_for_side(&env, &data_store, &market, true) as i128;
            total_short_open_interest_usd +=
                get_open_interest_for_side(&env, &data_store, &market, false) as i128;

            // Unclaimed fees are stored as raw token amounts per (market, token);
            // convert each side to USD with that token's oracle price.
            let long_fee = ds.get_u128(&claimable_fee_amount_key(
                &env,
                &market_token,
                &market.long_token,
            )) as i128;
            let short_fee = ds.get_u128(&claimable_fee_amount_key(
                &env,
                &market_token,
                &market.short_token,
            )) as i128;
            total_accumulated_fees_usd +=
                mul_div_wide(&env, long_fee, long_price.mid_price(), TOKEN_PRECISION);
            total_accumulated_fees_usd +=
                mul_div_wide(&env, short_fee, short_price.mid_price(), TOKEN_PRECISION);
        }

        ProtocolStats {
            total_pool_value_usd,
            total_long_open_interest_usd,
            total_short_open_interest_usd,
            total_accumulated_fees_usd,
            market_count: markets.len(),
            computed_at_ledger: env.ledger().sequence() as u64,
        }
    }

    /// Read a keeper role's liveness status from data_store (issue #249).
    ///
    /// View-only mirror of the heartbeat check so the frontend / monitoring can
    /// surface stale keepers without calling into order_handler. Returns the
    /// last-active ledger, the gap since then, and whether that gap exceeds the
    /// role's configured heartbeat timeout (falling back to the 2880-ledger
    /// default when unset). A role with no recorded activity reports
    /// `last_active_ledger = 0` and is treated as stale.
    pub fn check_keeper_heartbeat(
        env: Env,
        data_store: Address,
        role: BytesN<32>,
    ) -> KeeperHeartbeatStatus {
        let ds = DataStoreClient::new(&env, &data_store);
        let last_active_ledger =
            ds.get_u128(&last_keeper_activity_key(&env, &role)) as u64;
        let current_ledger = env.ledger().sequence() as u64;
        let ledgers_since_last_activity = current_ledger.saturating_sub(last_active_ledger);

        let stored_timeout = ds.get_u128(&keeper_heartbeat_timeout_key(&env, &role));
        let timeout = if stored_timeout == 0 {
            DEFAULT_KEEPER_HEARTBEAT_TIMEOUT
        } else {
            stored_timeout as u64
        };

        KeeperHeartbeatStatus {
            last_active_ledger,
            ledgers_since_last_activity,
            is_stale: ledgers_since_last_activity > timeout,
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

    /// Return paginated pending orders for a given market.
    pub fn get_pending_orders(
        env: Env,
        data_store: Address,
        order_handler: Address,
        market: Address,
        offset: u32,
        limit: u32,
    ) -> Vec<PendingOrder> {
        let ds = DataStoreClient::new(&env, &data_store);
        let set_key = order_list_key(&env);
        let total_count = ds.get_bytes32_set_count(&set_key);
        
        let mut matching_orders = Vec::new(&env);
        let mut skipped = 0;
        
        let keys = ds.get_bytes32_set_at(&set_key, &0, &total_count);
        let oh_client = OrderHandlerClient::new(&env, &order_handler);
        
        for i in 0..keys.len() {
            let k = keys.get_unchecked(i);
            if let Some(order) = oh_client.get_order(&k) {
                if order.market == market {
                    if skipped < offset {
                        skipped += 1;
                        continue;
                    }
                    
                    matching_orders.push_back(PendingOrder {
                        owner: order.account,
                        market: order.market.clone(),
                        order_type: order.order_type,
                        size_delta_usd: order.size_delta_usd,
                        execution_fee: order.execution_fee,
                        updated_at_time: order.updated_at_time,
                        is_long: order.is_long,
                    });
                    
                    if matching_orders.len() >= limit {
                        break;
                    }
                }
            }
        }
        
        matching_orders
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

    /// Compute effective leverage for an open position at current oracle prices (issue #218).
    ///
    /// Net collateral = gross collateral − pending borrowing fee − pending funding fee.
    /// Returns `None` when the position key does not exist.
    /// Returns `effective_leverage_bps = u32::MAX` when net collateral has been fully
    /// consumed by fees (net ≤ 0), signalling imminent liquidation.
    pub fn get_position_leverage(
        env: Env,
        data_store: Address,
        oracle: Address,
        order_handler: Address,
        position_key: BytesN<32>,
    ) -> Option<PositionLeverage> {
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

        // Only borrowing + funding fees reduce net collateral (position fee is paid on open/close).
        let fees: PositionFees = get_position_fees(
            &env,
            &data_store,
            &market_props,
            &position,
            collateral_price,
            position.size_in_usd,
            false,
        );

        let borrowing_fee_usd =
            mul_div_wide(&env, fees.borrowing_fee_amount, collateral_price, TOKEN_PRECISION);
        let funding_fee_usd =
            mul_div_wide(&env, fees.funding_fee_amount, collateral_price, TOKEN_PRECISION);
        let gross_collateral_usd =
            mul_div_wide(&env, position.collateral_amount, collateral_price, TOKEN_PRECISION);

        let net_signed = gross_collateral_usd - borrowing_fee_usd - funding_fee_usd;
        let net_collateral_usd = if net_signed > 0 { net_signed as u128 } else { 0u128 };
        let position_size_usd = if position.size_in_usd > 0 {
            position.size_in_usd as u128
        } else {
            0u128
        };

        let effective_leverage_bps = if net_collateral_usd == 0 {
            u32::MAX
        } else {
            mul_div_wide(&env, position.size_in_usd, 100, net_signed) as u32
        };

        let is_liq = is_liquidatable(
            &env,
            &data_store,
            &position,
            &market_props,
            collateral_price,
            &index_price,
        );

        Some(PositionLeverage {
            effective_leverage_bps,
            net_collateral_usd,
            position_size_usd,
            is_liquidatable: is_liq,
        })
    }

    // ── Issue #283: liquidatable positions batch view ─────────────────────────

    /// Return positions currently eligible for liquidation on a market side, sorted by
    /// health_factor_bps ascending (most under-collateralised first).
    ///
    /// `health_factor_bps = collateral_usd * 10000 / size_usd` — any value below
    /// 10000 means the position is below the minimum-collateral threshold.
    /// Positions where `is_position_liquidatable` returns false are excluded.
    ///
    /// Pagination: the `offset` first matching entries are skipped; at most `limit`
    /// entries are returned. No state is written.
    pub fn get_liquidatable_positions(
        env: Env,
        data_store: Address,
        oracle: Address,
        order_handler: Address,
        market: Address,
        is_long: bool,
        limit: u32,
        offset: u32,
    ) -> Vec<LiquidatablePosition> {
        let ds = DataStoreClient::new(&env, &data_store);
        let oracle_client = OracleClient::new(&env, &oracle);
        let order_client = OrderHandlerClient::new(&env, &order_handler);

        let market_props = Self::get_market(env.clone(), data_store.clone(), market.clone());
        let index_price = oracle_client.get_primary_price(&market_props.index_token);

        let pos_list = position_list_key(&env);
        let total = ds.get_bytes32_set_count(&pos_list);

        // Collect all liquidatable positions for this market/side.
        let mut candidates: Vec<LiquidatablePosition> = Vec::new(&env);
        let mut i = 0u32;
        while i < total {
            let batch_end = if i + 50 > total { total } else { i + 50 };
            let keys = ds.get_bytes32_set_at(&pos_list, &i, &batch_end);
            let keys_len = keys.len();
            let mut j = 0u32;
            while j < keys_len {
                let pk = keys.get_unchecked(j);
                if let Some(position) = order_client.get_position(&pk) {
                    if position.market == market && position.is_long == is_long {
                        let collateral_price = oracle_client
                            .get_primary_price(&position.collateral_token)
                            .mid_price();

                        if is_liquidatable(
                            &env,
                            &data_store,
                            &position,
                            &market_props,
                            collateral_price,
                            &index_price,
                        ) {
                            let collateral_usd = mul_div_wide(
                                &env,
                                position.collateral_amount,
                                collateral_price,
                                TOKEN_PRECISION,
                            ) as u128;
                            let size_usd = if position.size_in_usd > 0 {
                                position.size_in_usd as u128
                            } else {
                                1
                            };
                            let health_factor_bps = (collateral_usd
                                .saturating_mul(10000)
                                / size_usd)
                                as u32;

                            candidates.push_back(LiquidatablePosition {
                                key: pk,
                                owner: position.account,
                                size_usd,
                                collateral_usd,
                                health_factor_bps,
                            });
                        }
                    }
                }
                j += 1;
            }
            i = batch_end;
        }

        // Bubble-sort ascending by health_factor_bps (most undercollateralised first).
        let n = candidates.len();
        if n > 1 {
            let mut a = 0u32;
            while a < n {
                let mut b = 0u32;
                while b + 1 < n - a {
                    let ca = candidates.get_unchecked(b);
                    let cb = candidates.get_unchecked(b + 1);
                    if ca.health_factor_bps > cb.health_factor_bps {
                        candidates.set(b, cb);
                        candidates.set(b + 1, ca);
                    }
                    b += 1;
                }
                a += 1;
            }
        }

        // Apply pagination and return.
        let mut out: Vec<LiquidatablePosition> = Vec::new(&env);
        let mut skipped = 0u32;
        let mut taken = 0u32;
        let mut idx = 0u32;
        while idx < candidates.len() && taken < limit {
            if skipped < offset {
                skipped += 1;
            } else {
                out.push_back(candidates.get_unchecked(idx));
                taken += 1;
            }
            idx += 1;
        }
        out
    }
}

// ─── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use data_store::{DataStore, DataStoreClient as DsClient};
    use gmx_keys::{
        claimable_fee_amount_key, market_index_token_key, market_long_token_key,
        market_short_token_key, open_interest_key, pool_amount_key, position_key, roles,
    };
    use gmx_math::{FLOAT_PRECISION, TOKEN_PRECISION};
    use gmx_types::{PositionProps, TokenPrice};
    use oracle::{Oracle, OracleClient as OClient};
    use order_handler::{OrderHandler, OrderHandlerClient as OHClient, PositionStorageKey};
    use order_vault::{OrderVault, OrderVaultClient as OVClient};
    use role_store::{RoleStore, RoleStoreClient as RsClient};
    use soroban_sdk::{testutils::Address as _, Vec as SdkVec};

    struct World {
        env: Env,
        admin: Address,
        ds: Address,
        oracle: Address,
        reader: Address,
    }

    fn setup() -> World {
        let env = Env::default();
        env.mock_all_auths();
        env.cost_estimate().budget().reset_unlimited();

        let admin = Address::generate(&env);

        let rs = env.register(RoleStore, ());
        let rs_c = RsClient::new(&env, &rs);
        rs_c.initialize(&admin);
        // admin acts as CONTROLLER (writes config) and ORDER_KEEPER (submits prices)
        rs_c.grant_role(&admin, &admin, &roles::controller(&env));
        rs_c.grant_role(&admin, &admin, &roles::order_keeper(&env));

        let ds = env.register(DataStore, ());
        DsClient::new(&env, &ds).initialize(&admin, &rs);

        let oracle = env.register(Oracle, ());
        let passphrase = soroban_sdk::Bytes::from_slice(&env, b"Test SDF Network ; September 2015");
        OClient::new(&env, &oracle).initialize(&admin, &rs, &ds, &passphrase);

        let reader = env.register(Reader, ());
        ReaderClient::new(&env, &reader).initialize(&admin);

        World {
            env,
            admin,
            ds,
            oracle,
            reader,
        }
    }

    /// Register a market in data_store with the given tokens and seed pool amounts,
    /// open interest, and unclaimed fees. Sets oracle prices for all three tokens.
    /// Returns the market_token address.
    #[allow(clippy::too_many_arguments)]
    fn seed_market(
        w: &World,
        long_pool: u128,
        short_pool: u128,
        long_oi_usd: u128,
        short_oi_usd: u128,
        long_fee: u128,
        short_fee: u128,
        price: i128,
    ) -> Address {
        let env = &w.env;
        let market_tk = Address::generate(env);
        let long_tk = Address::generate(env);
        let short_tk = Address::generate(env);
        let index_tk = Address::generate(env);
        let ds_c = DsClient::new(env, &w.ds);

        // Wire the market's token addresses.
        ds_c.set_address(&w.admin, &market_index_token_key(env, &market_tk), &index_tk);
        ds_c.set_address(&w.admin, &market_long_token_key(env, &market_tk), &long_tk);
        ds_c.set_address(&w.admin, &market_short_token_key(env, &market_tk), &short_tk);

        // Pool amounts (raw token units).
        ds_c.set_u128(&w.admin, &pool_amount_key(env, &market_tk, &long_tk), &long_pool);
        ds_c.set_u128(
            &w.admin,
            &pool_amount_key(env, &market_tk, &short_tk),
            &short_pool,
        );

        // Open interest in USD (FLOAT_PRECISION), keyed on the long-side collateral.
        ds_c.set_u128(
            &w.admin,
            &open_interest_key(env, &market_tk, &long_tk, true),
            &long_oi_usd,
        );
        ds_c.set_u128(
            &w.admin,
            &open_interest_key(env, &market_tk, &short_tk, false),
            &short_oi_usd,
        );

        // Unclaimed fees (raw token units).
        ds_c.set_u128(
            &w.admin,
            &claimable_fee_amount_key(env, &market_tk, &long_tk),
            &long_fee,
        );
        ds_c.set_u128(
            &w.admin,
            &claimable_fee_amount_key(env, &market_tk, &short_tk),
            &short_fee,
        );

        // Oracle prices for the three tokens.
        OClient::new(env, &w.oracle).set_prices_simple(
            &w.admin,
            &SdkVec::from_array(
                env,
                [
                    TokenPrice {
                        token: long_tk,
                        min: price,
                        max: price,
                    },
                    TokenPrice {
                        token: short_tk,
                        min: price,
                        max: price,
                    },
                    TokenPrice {
                        token: index_tk,
                        min: price,
                        max: price,
                    },
                ],
            ),
        );

        market_tk
    }

    /// Empty markets vec returns all-zero stats with market_count 0, no panic.
    #[test]
    fn protocol_stats_empty_markets_returns_zeros() {
        let w = setup();
        let stats = ReaderClient::new(&w.env, &w.reader).get_protocol_stats(
            &w.ds,
            &w.oracle,
            &SdkVec::new(&w.env),
        );
        assert_eq!(stats.market_count, 0);
        assert_eq!(stats.total_pool_value_usd, 0);
        assert_eq!(stats.total_long_open_interest_usd, 0);
        assert_eq!(stats.total_short_open_interest_usd, 0);
        assert_eq!(stats.total_accumulated_fees_usd, 0);
    }

    /// A single seeded market reports its OI and accumulated fees in USD.
    #[test]
    fn protocol_stats_single_market_aggregates() {
        let w = setup();
        let fp = FLOAT_PRECISION;
        let long_oi = 1_000 * fp as u128;
        let short_oi = 400 * fp as u128;
        // 5 long-token + 5 short-token fee units at price $2 → $20 total (FLOAT_PRECISION).
        let fee_units = 5 * gmx_math::TOKEN_PRECISION as u128;
        let price = 2 * fp;

        let m = seed_market(&w, 100, 100, long_oi, short_oi, fee_units, fee_units, price);
        let stats = ReaderClient::new(&w.env, &w.reader).get_protocol_stats(
            &w.ds,
            &w.oracle,
            &SdkVec::from_array(&w.env, [m]),
        );

        assert_eq!(stats.market_count, 1);
        assert_eq!(stats.total_long_open_interest_usd, long_oi as i128);
        assert_eq!(stats.total_short_open_interest_usd, short_oi as i128);
        // 10 token-units total at $2 = $20, expressed in FLOAT_PRECISION.
        assert_eq!(stats.total_accumulated_fees_usd, 20 * fp);
        assert!(stats.total_pool_value_usd > 0);
    }

    /// Two markets: totals are the sum of the per-market contributions.
    #[test]
    fn protocol_stats_two_markets_sum() {
        let w = setup();
        let fp = FLOAT_PRECISION;
        let oi = 500 * fp as u128;
        let price = 1 * fp;

        let m1 = seed_market(&w, 50, 50, oi, oi, 0, 0, price);
        let m2 = seed_market(&w, 70, 70, oi, oi, 0, 0, price);
        let stats = ReaderClient::new(&w.env, &w.reader).get_protocol_stats(
            &w.ds,
            &w.oracle,
            &SdkVec::from_array(&w.env, [m1, m2]),
        );

        assert_eq!(stats.market_count, 2);
        assert_eq!(stats.total_long_open_interest_usd, 2 * oi as i128);
        assert_eq!(stats.total_short_open_interest_usd, 2 * oi as i128);
    }

    /// A market with zero pool value contributes zero and does not panic.
    #[test]
    fn protocol_stats_zero_pool_market_contributes_nothing() {
        let w = setup();
        let price = FLOAT_PRECISION;
        let m = seed_market(&w, 0, 0, 0, 0, 0, 0, price);
        let stats = ReaderClient::new(&w.env, &w.reader).get_protocol_stats(
            &w.ds,
            &w.oracle,
            &SdkVec::from_array(&w.env, [m]),
        );
        assert_eq!(stats.market_count, 1);
        assert_eq!(stats.total_pool_value_usd, 0);
        assert_eq!(stats.total_accumulated_fees_usd, 0);
    }

    /// More than MAX_STATS_MARKETS markets must revert with TooManyMarkets.
    #[test]
    #[should_panic]
    fn protocol_stats_too_many_markets_panics() {
        let w = setup();
        let mut markets = SdkVec::new(&w.env);
        for _ in 0..(MAX_STATS_MARKETS + 1) {
            markets.push_back(Address::generate(&w.env));
        }
        ReaderClient::new(&w.env, &w.reader).get_protocol_stats(&w.ds, &w.oracle, &markets);
    }

    // ── Issue #218: get_position_leverage ────────────────────────────────────

    /// A position with 10,000 USD size and 500 USD net collateral (no fees) should
    /// report effective_leverage_bps = 2000 (i.e. 20×).
    #[test]
    fn get_position_leverage_2000_bps() {
        let env = Env::default();
        env.mock_all_auths();
        env.cost_estimate().budget().reset_unlimited();

        let admin = Address::generate(&env);

        let rs = env.register(RoleStore, ());
        RsClient::new(&env, &rs).initialize(&admin);
        RsClient::new(&env, &rs).grant_role(&admin, &admin, &roles::controller(&env));
        RsClient::new(&env, &rs).grant_role(&admin, &admin, &roles::order_keeper(&env));

        let ds = env.register(DataStore, ());
        DsClient::new(&env, &ds).initialize(&admin, &rs);

        let oracle = env.register(Oracle, ());
        let passphrase = soroban_sdk::Bytes::from_slice(&env, b"Test SDF Network ; September 2015");
        OClient::new(&env, &oracle).initialize(&admin, &rs, &ds, &passphrase);

        let reader = env.register(Reader, ());
        ReaderClient::new(&env, &reader).initialize(&admin);

        let index_tk = Address::generate(&env);
        let long_tk = Address::generate(&env);
        let short_tk = Address::generate(&env);
        let market_tk = Address::generate(&env);
        let trader = Address::generate(&env);

        let ds_c = DsClient::new(&env, &ds);
        ds_c.set_address(&admin, &market_index_token_key(&env, &market_tk), &index_tk);
        ds_c.set_address(&admin, &market_long_token_key(&env, &market_tk), &long_tk);
        ds_c.set_address(&admin, &market_short_token_key(&env, &market_tk), &short_tk);

        let fp = FLOAT_PRECISION;
        OClient::new(&env, &oracle).set_prices_simple(
            &admin,
            &SdkVec::from_array(
                &env,
                [
                    TokenPrice { token: index_tk.clone(), min: fp, max: fp },
                    TokenPrice { token: long_tk.clone(), min: fp, max: fp },
                    TokenPrice { token: short_tk.clone(), min: fp, max: fp },
                ],
            ),
        );

        // Register order_handler; use a dummy vault address (not called in this test)
        let dummy_vault = env.register(OrderVault, ());
        OVClient::new(&env, &dummy_vault).initialize(&admin, &rs);
        let ord_handler = env.register(OrderHandler, ());
        OHClient::new(&env, &ord_handler).initialize(&admin, &rs, &ds, &oracle, &dummy_vault);
        RsClient::new(&env, &rs).grant_role(&admin, &ord_handler, &roles::controller(&env));

        // Build a synthetic position:
        //   size_in_usd = 10_000 * FLOAT_PRECISION
        //   collateral_amount = 500 tokens (long_tk at $1 each = $500)
        //   No fees (all fee factors and per-size trackers are zero)
        //   Expected: 10_000 * 100 / 500 = 2000 bps (20×)
        let tk_prec = TOKEN_PRECISION as i128;
        let position = PositionProps {
            account: trader.clone(),
            market: market_tk.clone(),
            collateral_token: long_tk.clone(),
            size_in_usd: 10_000i128 * fp,
            size_in_tokens: 10_000i128 * tk_prec,
            collateral_amount: 500i128 * tk_prec,
            pending_impact_amount: 0,
            borrowing_factor: 0,
            funding_fee_amount_per_size: 0,
            long_claim_fnd_per_size: 0,
            short_claim_fnd_per_size: 0,
            increased_at_time: 0,
            decreased_at_time: 0,
            is_long: true,
        };

        let pk = position_key(&env, &trader, &market_tk, &long_tk, true);
        env.as_contract(&ord_handler, || {
            env.storage()
                .persistent()
                .set(&PositionStorageKey::Position(pk.clone()), &position);
        });

        let result = ReaderClient::new(&env, &reader)
            .get_position_leverage(&ds, &oracle, &ord_handler, &pk);

        let lev = result.unwrap();
        assert_eq!(lev.effective_leverage_bps, 2000);
        assert_eq!(lev.position_size_usd, (10_000u128 * fp as u128));
        assert_eq!(lev.net_collateral_usd, (500u128 * fp as u128));
        assert!(!lev.is_liquidatable);
    }

    /// A non-existent position key must return None.
    #[test]
    fn get_position_leverage_none_for_missing_key() {
        let env = Env::default();
        env.mock_all_auths();
        env.cost_estimate().budget().reset_unlimited();
        let admin = Address::generate(&env);

        let rs = env.register(RoleStore, ());
        RsClient::new(&env, &rs).initialize(&admin);
        RsClient::new(&env, &rs).grant_role(&admin, &admin, &roles::controller(&env));

        let ds = env.register(DataStore, ());
        DsClient::new(&env, &ds).initialize(&admin, &rs);

        let oracle = env.register(Oracle, ());
        let pass = soroban_sdk::Bytes::from_slice(&env, b"Test SDF Network ; September 2015");
        OClient::new(&env, &oracle).initialize(&admin, &rs, &ds, &pass);

        let vault = env.register(OrderVault, ());
        OVClient::new(&env, &vault).initialize(&admin, &rs);

        let ord = env.register(OrderHandler, ());
        OHClient::new(&env, &ord).initialize(&admin, &rs, &ds, &oracle, &vault);

        let reader = env.register(Reader, ());
        ReaderClient::new(&env, &reader).initialize(&admin);

        let missing_key = soroban_sdk::BytesN::from_array(&env, &[0u8; 32]);
        let result =
            ReaderClient::new(&env, &reader).get_position_leverage(&ds, &oracle, &ord, &missing_key);
        assert!(result.is_none());
    }

    // ── ADL (Auto-Deleveraging) views ────────────────────────────────────────

    /// Get all profitable positions eligible for auto-deleveraging on a market side.
    ///
    /// Returns only positions with positive unrealised PnL, sorted by profitability ratio
    /// (highest first). Use `limit` to bound iteration cost; keepers call multiple times
    /// if many positions qualify.
    pub fn get_adl_eligible_positions(
        env: Env,
        data_store: Address,
        oracle: Address,
        order_handler: Address,
        market: Address,
        is_long: bool,
        limit: u32,
    ) -> Vec<AdlCandidate> {
        let ds = DataStoreClient::new(&env, &data_store);
        let oracle_client = OracleClient::new(&env, &oracle);
        let order_client = OrderHandlerClient::new(&env, &order_handler);

        // Get market properties
        let market_props = Self::get_market(env.clone(), data_store.clone(), market.clone());
        let index_price = oracle_client.get_primary_price(&market_props.index_token);

        // Fetch all position keys from global position list
        let pos_list_key = position_list_key(&env);
        let pos_count = ds.get_bytes32_set_count(&pos_list_key);
        
        let mut candidates: Vec<AdlCandidate> = Vec::new(&env);
        
        // Iterate through all positions
        let mut i = 0u32;
        while i < pos_count && (candidates.len() as u32) < limit {
            // Fetch a batch of position keys
            let batch_end = if i + 100 > pos_count { pos_count } else { i + 100 };
            let position_keys = ds.get_bytes32_set_at(&pos_list_key, &i, &batch_end);
            
            let keys_len = position_keys.len();
            let mut j = 0u32;
            while j < keys_len {
                let pos_key = position_keys.get(j).unwrap();
                
                // Get position from order handler
                if let Some(position) = order_client.get_position(&pos_key) {
                    // Filter by market and direction
                    if position.market != market || position.is_long != is_long {
                        j += 1;
                        continue;
                    }

                    // Calculate unrealised PnL
                    let (pnl_usd, _) = get_position_pnl_usd(&env, &position, &index_price, position.size_in_usd);
                    
                    // Only include profitable positions
                    if pnl_usd > 0 {
                        // Calculate PnL to size ratio in basis points
                        let size_usd_abs = if position.size_in_usd > 0 { 
                            position.size_in_usd as u128 
                        } else { 
                            0u128 
                        };
                        
                        let ratio_bps = if size_usd_abs > 0 {
                            mul_div_wide(&env, pnl_usd, 10000i128, position.size_in_usd) as u32
                        } else {
                            0u32
                        };

                        let candidate = AdlCandidate {
                            key: pos_key,
                            owner: position.account.clone(),
                            size_usd: size_usd_abs,
                            unrealised_pnl_usd: pnl_usd as u128,
                            pnl_to_size_ratio_bps: ratio_bps,
                        };
                        
                        candidates.push_back(candidate);
                    }
                }
                
                j += 1;
            }
            
            i = batch_end;
        }

        // Sort candidates by pnl_to_size_ratio_bps descending (bubble sort)
        let candidates_len = candidates.len();
        if candidates_len > 1 {
            let mut k = 0usize;
            while k < candidates_len {
                let mut m = 0usize;
                while m + 1 < candidates_len - k {
                    let cand_m = candidates.get(m).unwrap();
                    let cand_m_next = candidates.get(m + 1).unwrap();
                    
                    // Swap if m+1 has higher ratio (descending sort)
                    if cand_m_next.pnl_to_size_ratio_bps > cand_m.pnl_to_size_ratio_bps {
                        let temp = cand_m.clone();
                        candidates.set(m, cand_m_next.clone());
                        candidates.set(m + 1, temp);
                    }
                    
                    m += 1;
                }
                k += 1;
            }
        }

        // Trim to limit
        let mut result: Vec<AdlCandidate> = Vec::new(&env);
        let take = if candidates_len > (limit as usize) { limit as usize } else { candidates_len };
        let mut idx = 0usize;
        while idx < take {
            result.push_back(candidates.get(idx).unwrap());
            idx += 1;
        }

        result
    }

    // ── Swap estimation (dry-run without state modification) ─────────────────

    /// Estimate the output of a swap without modifying state.
    ///
    /// Returns the estimated output token amount, cumulative price impact, and
    /// whether execution would likely revert due to paused markets or insufficient liquidity.
    ///
    /// This is a read-only view that mirrors swap execution logic for frontend preview.
    pub fn estimate_swap_output(
        env: Env,
        data_store: Address,
        oracle: Address,
        token_in: Address,
        amount_in: u128,
        swap_path: Vec<Address>,
    ) -> SwapEstimate {
        let oracle_client = OracleClient::new(&env, &oracle);
        
        // Validate swap path
        if swap_path.len() == 0 {
            return SwapEstimate {
                token_out: token_in.clone(),
                amount_out: amount_in,
                price_impact_usd: 0i128,
                execution_price: 0u128,
                reverts_if_executed: true,
            };
        }

        let mut current_amount = amount_in;
        let mut current_token = token_in.clone();
        let mut total_impact_usd = 0i128;
        let mut reverts_if_executed = false;

        // Iterate through swap path
        let path_len = swap_path.len();
        let mut i = 0u32;
        while i < path_len {
            let market = swap_path.get(i).unwrap();
            
            // Load market properties
            let market_props = Self::get_market(env.clone(), data_store.clone(), market);
            
            // For now, estimate assumes:
            // - Market is not paused (we don't have pause status check in this version)
            // - Sufficient liquidity exists
            // - Price impact is calculated based on pool state
            
            // Get oracle prices
            let index_price = oracle_client.get_primary_price(&market_props.index_token).mid_price();
            let long_price = oracle_client.get_primary_price(&market_props.long_token).mid_price();
            let short_price = oracle_client.get_primary_price(&market_props.short_token).mid_price();
            
            // Determine which token is input and which is output
            let (input_token, output_token) = if current_token == market_props.long_token {
                (market_props.long_token.clone(), market_props.short_token.clone())
            } else if current_token == market_props.short_token {
                (market_props.short_token.clone(), market_props.long_token.clone())
            } else {
                // Token not in market, swap ends
                reverts_if_executed = true;
                break;
            };

            // Convert amount_in to USD
            let input_price = if input_token == market_props.long_token { 
                long_price 
            } else { 
                short_price 
            };
            
            let input_usd = mul_div_wide(&env, current_amount as i128, input_price, TOKEN_PRECISION);

            // Get swap impact
            let impact_usd = get_position_price_impact(
                &env, &data_store, &market_props,
                false,  // is_long (doesn't matter for swap impact in this context)
                input_usd,
                true,   // is_increase (swap is treated as positive impact)
                index_price,
            );

            total_impact_usd += impact_usd;

            // Apply impact to output
            let output_price = if output_token == market_props.long_token { 
                long_price 
            } else { 
                short_price 
            };
            
            let output_usd = input_usd + impact_usd;
            
            if output_usd <= 0 {
                reverts_if_executed = true;
                break;
            }

            current_amount = mul_div_wide(&env, output_usd, TOKEN_PRECISION, output_price) as u128;
            current_token = output_token;

            i += 1;
        }

        let final_token_out = current_token;
        let final_amount_out = current_amount;

        // Calculate execution price (input / output)
        let execution_price = if final_amount_out > 0 {
            mul_div_wide(&env, amount_in as i128, TOKEN_PRECISION, final_amount_out as i128) as u128
        } else {
            0u128
        };

        SwapEstimate {
            token_out: final_token_out,
            amount_out: final_amount_out,
            price_impact_usd: total_impact_usd,
            execution_price,
            reverts_if_executed,
        }
    }
}
