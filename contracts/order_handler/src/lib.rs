//! Order handler — create, execute, cancel, update, and freeze orders.
//! Mirrors GMX's OrderHandler.sol.
//!
//! Supported order types (OrderType enum in gmx_types):
//!   MarketSwap, LimitSwap            → routed to swap_utils
//!   MarketIncrease, LimitIncrease    → routed to increase_position_utils
//!   MarketDecrease, LimitDecrease,
//!   StopLossDecrease, Liquidation    → routed to decrease_position_utils
//!
//! Two-step lifecycle (same as deposit/withdrawal):
//!   create_order  → pulls collateral into order_vault, stores OrderProps
//!   execute_order → keeper calls with fresh oracle prices, dispatches by type
//!   cancel_order  → refunds collateral from order_vault to account
//!   update_order  → modify trigger_price / acceptable_price / size before execution
//!   freeze_order  → mark order as frozen (keeper-side circuit breaker)
#![no_std]
#![allow(dependency_on_unit_never_type_fallback)]

use gmx_decrease_position_utils::{decrease_position, DecreasePositionParams};
use gmx_increase_position_utils::{increase_position, IncreasePositionParams};
use gmx_keys::{
    account_order_list_key, keeper_heartbeat_timeout_key, last_keeper_activity_key,
    liquidation_execution_fee_key,
    market_index_token_key, market_long_token_key, market_short_token_key,
    max_leverage_key, order_key, order_list_key, position_fee_factor_key, position_key,
    fee_tier_position_fee_factor_key, fee_tier_volume_threshold_key,
    trader_volume_key, trader_volume_window_start_key,
    roles, DEFAULT_KEEPER_HEARTBEAT_TIMEOUT, is_market_paused_key,
};
use gmx_math::{mul_div_wide, FLOAT_PRECISION};
use gmx_swap_utils::swap_with_path;
pub use gmx_types::CreateOrderParams;
use gmx_types::PositionProps;
use gmx_types::{MarketProps, OrderProps, OrderType, PriceProps};
use soroban_sdk::{
    contract, contracterror, contractevent, contractimpl, contracttype, panic_with_error,
    symbol_short, Address, BytesN, Env,
};

// ─── Storage keys ─────────────────────────────────────────────────────────────

#[contracttype]
enum InstanceKey {
    Initialized,
    Admin,
    RoleStore,
    DataStore,
    Oracle,
    OrderVault,
    ReferralStorage,
}

// ─── Errors ───────────────────────────────────────────────────────────────────

#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
#[repr(u32)]
pub enum Error {
    AlreadyInitialized = 1,
    NotInitialized = 2,
    Unauthorized = 3,
    OrderNotFound = 4,
    InvalidOrderType = 5,
    UnsatisfiedTrigger = 6,
    PriceTooHigh = 7,
    PriceTooLow = 8,
    OrderFrozen = 9,
    /// Increase/swap orders require collateral to have been transferred to
    /// order_vault (via exchange_router SendTokens) before calling create_order.
    /// record_transfer_in returned zero, meaning no collateral arrived.
    ZeroCollateral = 10,
    UnauthorizedPositionManager = 11,
    /// `flag_stale_keeper` was called but the role's last activity is still
    /// within the configured heartbeat timeout (issue #249).
    KeeperNotStale = 12,
    /// Position size/collateral ratio exceeds configured maximum leverage.
    MaxLeverageExceeded = 13,
    /// `create_orders` received more than 5 orders in a single batch.
    BatchSizeLimitExceeded = 14,
    /// The target market is paused due to circuit breaker (issue #203).
    MarketPaused = 15,
    /// swap_path contains a repeated market address — would corrupt pool accounting (issue #232).
    CyclicSwapPath = 16,
}


// ─── Position events (issue #205) ────────────────────────────────────────────

#[contracttype]
pub struct PositionOpenedEvent {
    pub key: BytesN<32>,
    pub account: Address,
    pub market: Address,
    pub size_in_usd: i128,
    pub collateral_amount: i128,
    pub is_long: bool,
    pub avg_entry_price: i128,
}

#[contracttype]
pub struct PositionIncreasedEvent {
    pub key: BytesN<32>,
    pub account: Address,
    pub market: Address,
    pub delta_size_usd: i128,
    pub delta_collateral: i128,
    pub new_size_usd: i128,
    pub avg_entry_price: i128,
}

#[contracttype]
pub struct PositionDecreasedEvent {
    pub key: BytesN<32>,
    pub account: Address,
    pub market: Address,
    pub delta_size_usd: i128,
    pub pnl_usd: i128,
    pub execution_price: i128,
}

#[contracttype]
pub struct PositionClosedEvent {
    pub key: BytesN<32>,
    pub account: Address,
    pub market: Address,
    pub pnl_usd: i128,
    pub execution_price: i128,
}

#[contracttype]
pub struct PositionLiquidatedEvent {
    pub key: BytesN<32>,
    pub account: Address,
    pub market: Address,
    pub execution_price: i128,
    pub remaining_collateral: i128,
}

// ─── External contract clients ────────────────────────────────────────────────

#[allow(dead_code)]
#[soroban_sdk::contractclient(name = "RoleStoreClient")]
trait IRoleStore {
    fn has_role(env: Env, account: Address, role: BytesN<32>) -> bool;
}

#[allow(dead_code)]
#[soroban_sdk::contractclient(name = "DataStoreClient")]
trait IDataStore {
    fn get_u128(env: Env, key: BytesN<32>) -> u128;
    fn set_u128(env: Env, caller: Address, key: BytesN<32>, value: u128) -> u128;
    fn apply_delta_to_u128(env: Env, caller: Address, key: BytesN<32>, delta: i128) -> u128;
    fn get_i128(env: Env, key: BytesN<32>) -> i128;
    fn get_position_manager(env: Env, owner: Address, market: Address) -> Option<Address>;
    fn increment_nonce(env: Env, caller: Address) -> u64;
    fn get_address(env: Env, key: BytesN<32>) -> Option<Address>;
    fn add_bytes32_to_set(env: Env, caller: Address, set_key: BytesN<32>, value: BytesN<32>);
    fn remove_bytes32_from_set(env: Env, caller: Address, set_key: BytesN<32>, value: BytesN<32>);
    fn contains_bytes32(env: Env, set_key: BytesN<32>, value: BytesN<32>) -> bool;
    fn set_address(env: Env, caller: Address, key: BytesN<32>, value: Address) -> Address;
    fn get_bool(env: Env, key: BytesN<32>) -> bool;
}

#[allow(dead_code)]
#[soroban_sdk::contractclient(name = "OracleClient")]
trait IOracle {
    fn get_primary_price(env: Env, token: Address) -> PriceProps;
}

#[allow(dead_code)]
#[soroban_sdk::contractclient(name = "OrderVaultClient")]
trait IOrderVault {
    fn record_transfer_in(env: Env, token: Address) -> i128;
    fn transfer_out(env: Env, caller: Address, token: Address, receiver: Address, amount: i128);
}

#[allow(dead_code)]
#[soroban_sdk::contractclient(name = "ReferralStorageClient")]
trait IReferralStorage {
    fn get_trader_referrer(env: Env, trader: Address) -> Option<Address>;
    fn increment_referrer_volume(env: Env, caller: Address, referrer: Address, volume_usd: u128);
}


// ─── Position storage key (must match increase/decrease position utils) ───────

/// Positions are stored in this contract's persistent storage under this key.
/// The #[contracttype] XDR encoding must match the one in increase/decrease_position_utils.
#[contracttype]
pub enum PositionStorageKey {
    Position(BytesN<32>),
}

// ─── Order-frozen flag (stored alongside OrderProps) ──────────────────────────

#[contracttype]
pub enum OrderStorageKey {
    Order(BytesN<32>),
    OrderFrozen(BytesN<32>),
}
// OrderProps are stored in this contract's own persistent storage (not DataStore)
// because DataStore supports only primitive/set types, not arbitrary structs.
// DataStore holds only the index sets (order_list_key, account_order_list_key)
// for enumeration. This matches the deposit and withdrawal handler patterns (issue #25).

// ─── Events ───────────────────────────────────────────────────────────────────

/// Emitted when a keeper role is found stale (issue #249): the gap between the
/// current ledger and the role's last recorded activity has exceeded the
/// configured heartbeat timeout. Signals the admin that the keeper has gone
/// silent and its role can be revoked.
#[contractevent(topics = ["kpr_stale"])]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct KeeperHeartbeatMissed {
    pub role: BytesN<32>,
    pub keeper: Address,
    pub last_ledger: u64,
    pub current_ledger: u64,
}

// ─── Contract ─────────────────────────────────────────────────────────────────

#[contract]
pub struct OrderHandler;

#[contractimpl]
impl OrderHandler {
    /// One-time setup.
    pub fn initialize(
        env: Env,
        admin: Address,
        role_store: Address,
        data_store: Address,
        oracle: Address,
        order_vault: Address,
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
            .set(&InstanceKey::OrderVault, &order_vault);
    }

    /// Upgrade the contract wasm. Only the stored admin may call this.
    ///
    /// Storage layout (InstanceKey and PositionStorageKey / OrderStorageKey) must not
    /// change between versions — existing persistent entries remain readable after upgrade.
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
        env.storage().instance().set(&InstanceKey::Oracle, &new_oracle);
    }

    /// Admin-configurable heartbeat timeout for a keeper `role`, in ledgers
    /// (issue #249). When the gap since the role's last activity exceeds this,
    /// the keeper is considered stale. Unset roles use
    /// `DEFAULT_KEEPER_HEARTBEAT_TIMEOUT` (2880 ledgers, ~4h).
    pub fn set_keeper_heartbeat_timeout(
        env: Env,
        caller: Address,
        role: BytesN<32>,
        timeout_ledgers: u64,
    ) {
        caller.require_auth();
        let admin: Address = env
            .storage()
            .instance()
            .get(&InstanceKey::Admin)
            .unwrap_or_else(|| panic_with_error!(&env, Error::NotInitialized));
        if caller != admin {
            panic_with_error!(&env, Error::Unauthorized);
        }
        let data_store: Address = env
            .storage()
            .instance()
            .get(&InstanceKey::DataStore)
            .unwrap_or_else(|| panic_with_error!(&env, Error::NotInitialized));
        let handler = env.current_contract_address();
        DataStoreClient::new(&env, &data_store).set_u128(
            &handler,
            &keeper_heartbeat_timeout_key(&env, &role),
            &(timeout_ledgers as u128),
        );
    }

    /// Read a keeper role's liveness status from data_store (issue #249).
    ///
    /// View-only. Returns the last-active ledger, the gap since then, and whether
    /// that gap has exceeded the configured heartbeat timeout. A role that has
    /// never recorded activity reports `last_active_ledger = 0` and is treated as
    /// stale (its full lifetime exceeds any timeout).
    pub fn check_keeper_heartbeat(
        env: Env,
        data_store: Address,
        role: BytesN<32>,
    ) -> gmx_types::KeeperHeartbeatStatus {
        let last_active_ledger = DataStoreClient::new(&env, &data_store)
            .get_u128(&last_keeper_activity_key(&env, &role))
            as u64;
        let current_ledger = env.ledger().sequence() as u64;
        let ledgers_since_last_activity = current_ledger.saturating_sub(last_active_ledger);
        let timeout = keeper_heartbeat_timeout(&env, &data_store, &role);
        let is_stale = ledgers_since_last_activity > timeout;
        gmx_types::KeeperHeartbeatStatus {
            last_active_ledger,
            ledgers_since_last_activity,
            is_stale,
        }
    }

    /// Flag a keeper as stale (issue #249). Admin-gated.
    ///
    /// Verifies the `role`'s heartbeat has lapsed and emits `KeeperHeartbeatMissed`
    /// so the staleness is recorded on-chain. The admin can then revoke the
    /// keeper's role via `role_store::revoke_role` — which has no timelock, so the
    /// replacement can be wired immediately. Panics with `KeeperNotStale` if the
    /// role is still within its heartbeat window, preventing premature flagging.
    pub fn flag_stale_keeper(env: Env, caller: Address, keeper: Address, role: BytesN<32>) {
        caller.require_auth();
        let admin: Address = env
            .storage()
            .instance()
            .get(&InstanceKey::Admin)
            .unwrap_or_else(|| panic_with_error!(&env, Error::NotInitialized));
        if caller != admin {
            panic_with_error!(&env, Error::Unauthorized);
        }
        let data_store: Address = env
            .storage()
            .instance()
            .get(&InstanceKey::DataStore)
            .unwrap_or_else(|| panic_with_error!(&env, Error::NotInitialized));

        let status = Self::check_keeper_heartbeat(env.clone(), data_store.clone(), role.clone());
        if !status.is_stale {
            panic_with_error!(&env, Error::KeeperNotStale);
        }

        env.events().publish_event(&KeeperHeartbeatMissed {
            role,
            keeper,
            last_ledger: status.last_active_ledger,
            current_ledger: env.ledger().sequence() as u64,
        });
    }

    /// Register the referral_storage contract address (admin only — issue #217).
    /// Once set, execute_order calls increment_referrer_volume after each trade.
    pub fn set_referral_storage(env: Env, caller: Address, referral_storage: Address) {
        caller.require_auth();
        let admin: Address = env
            .storage()
            .instance()
            .get(&InstanceKey::Admin)
            .unwrap_or_else(|| panic_with_error!(&env, Error::NotInitialized));
        if caller != admin {
            panic_with_error!(&env, Error::Unauthorized);
        }
        env.storage()
            .instance()
            .set(&InstanceKey::ReferralStorage, &referral_storage);
    }

    /// Create up to 5 orders atomically in a single call (issue #219).
    ///
    /// All orders are created or none are (Soroban atomicity). For increase/swap orders,
    /// the caller must pre-fund the order_vault via SendTokens before calling this.
    /// `record_transfer_in` is called per increase/swap order to snapshot the delta.
    ///
    /// Returns the list of created order keys in the same order as `requests`.
    pub fn create_orders(
        env: Env,
        caller: Address,
        requests: soroban_sdk::Vec<CreateOrderParams>,
    ) -> soroban_sdk::Vec<BytesN<32>> {
        caller.require_auth();

        if requests.len() > 5 {
            panic_with_error!(&env, Error::BatchSizeLimitExceeded);
        }

        let data_store: Address = env
            .storage()
            .instance()
            .get(&InstanceKey::DataStore)
            .unwrap_or_else(|| panic_with_error!(&env, Error::NotInitialized));
        let order_vault: Address = env
            .storage()
            .instance()
            .get(&InstanceKey::OrderVault)
            .unwrap_or_else(|| panic_with_error!(&env, Error::NotInitialized));
        let handler = env.current_contract_address();
        let ds = DataStoreClient::new(&env, &data_store);
        let vault_client = OrderVaultClient::new(&env, &order_vault);

        let mut keys: soroban_sdk::Vec<BytesN<32>> = soroban_sdk::Vec::new(&env);

        let len = requests.len();
        let mut i = 0u32;
        while i < len {
            let params = requests.get_unchecked(i);

            let is_increase_or_swap = matches!(
                params.order_type,
                OrderType::MarketIncrease
                    | OrderType::LimitIncrease
                    | OrderType::StopIncrease
                    | OrderType::MarketSwap
                    | OrderType::LimitSwap
            );

            let collateral_delta_amount = if is_increase_or_swap {
                let received = vault_client.record_transfer_in(&params.initial_collateral_token);
                if received <= 0 {
                    panic_with_error!(&env, Error::ZeroCollateral);
                }
                received
            } else {
                params.collateral_delta_amount
            };

            let nonce = ds.increment_nonce(&handler);
            let key = order_key(&env, nonce);

            let order = OrderProps {
                account: caller.clone(),
                receiver: params.receiver.clone(),
                market: params.market.clone(),
                initial_collateral_token: params.initial_collateral_token.clone(),
                swap_path: params.swap_path.clone(),
                size_delta_usd: params.size_delta_usd,
                collateral_delta_amount,
                trigger_price: params.trigger_price,
                acceptable_price: params.acceptable_price,
                execution_fee: params.execution_fee,
                min_output_amount: params.min_output_amount,
                order_type: params.order_type.clone(),
                is_long: params.is_long,
                updated_at_time: env.ledger().timestamp(),
            };

            env.storage()
                .persistent()
                .set(&OrderStorageKey::Order(key.clone()), &order);
            ds.add_bytes32_to_set(&handler, &order_list_key(&env), &key);
            ds.add_bytes32_to_set(&handler, &account_order_list_key(&env, &caller), &key);

            env.events().publish(
                (symbol_short!("ord_crt"),),
                (key.clone(), caller.clone(), params.market.clone()),
            );
            keys.push_back(key);

            i += 1;
        }

        keys
    }

    /// Create a new order and record collateral in the order vault.
    ///
    /// # Collateral model (canonical — issue #47)
    ///
    /// **Chosen path:** the exchange_router is responsible for transferring
    /// collateral from the caller to order_vault BEFORE invoking create_order.
    /// order_handler then calls `record_transfer_in` to snapshot the delta.
    ///
    /// **Why this model:**
    /// - The router owns the auth context for the caller's token approval.
    /// - Keeping the pull inside the router makes the vault a passive custodian
    ///   with no token-approval dependencies of its own.
    /// - Handlers never hold approvals, so they cannot silently double-pull.
    ///
    /// **Invariant enforced here:**
    /// - For increase/swap orders: `record_transfer_in` delta MUST be > 0.
    ///   A zero delta means tokens were not pre-sent; the transaction reverts
    ///   with `ZeroCollateral` before any state is written.
    /// - For decrease/stop-loss/liquidation orders: no collateral is deposited;
    ///   `collateral_delta_amount` comes from params (typically the existing position size).
    ///
    /// **Multicall sequence the router enforces for increase/swap orders:**
    /// ```text
    /// multicall([
    ///   SendTokens { token, receiver: order_vault, amount },   // 1. push collateral
    ///   CreateOrder { params },                                 // 2. snapshot + store order
    /// ])
    /// ```
    ///
    /// Returns the order key.
    pub fn create_order(env: Env, caller: Address, params: CreateOrderParams) -> BytesN<32> {
        caller.require_auth();

        let data_store: Address = env
            .storage()
            .instance()
            .get(&InstanceKey::DataStore)
            .unwrap_or_else(|| panic_with_error!(&env, Error::NotInitialized));
        let order_vault: Address = env
            .storage()
            .instance()
            .get(&InstanceKey::OrderVault)
            .unwrap_or_else(|| panic_with_error!(&env, Error::NotInitialized));
        let handler = env.current_contract_address();
        let ds = DataStoreClient::new(&env, &data_store);

        if ds.get_bool(&is_market_paused_key(&env, &params.market)) {
            panic_with_error!(&env, Error::MarketPaused);
        }

        // Issue #232: reject cyclic swap paths at creation time; any repeated market
        // would double-mutate pool state and corrupt price-impact accounting.
        {
            let path = &params.swap_path;
            let path_len = path.len();
            let mut i = 0u32;
            while i < path_len {
                let mut j = i + 1;
                while j < path_len {
                    if path.get(i).unwrap() == path.get(j).unwrap() {
                        panic_with_error!(&env, Error::CyclicSwapPath);
                    }
                    j += 1;
                }
                i += 1;
            }
        }

        // Determine whether this order type requires upfront collateral in the vault.
        // Increase and swap orders pull from the vault; decrease orders do not deposit.
        let is_increase_or_swap = matches!(
            params.order_type,
            OrderType::MarketIncrease
                | OrderType::LimitIncrease
                | OrderType::StopIncrease
                | OrderType::MarketSwap
                | OrderType::LimitSwap
        );
        
        // Determine if this is a position order (increase/decrease)
        let is_position_order = matches!(
            params.order_type,
            OrderType::MarketIncrease | OrderType::LimitIncrease | OrderType::StopIncrease |
            OrderType::MarketDecrease | OrderType::LimitDecrease | OrderType::StopLossDecrease
        );

        // Position manager authorization: 
        // For position orders, verify caller is either the owner OR an authorized manager for this market.
        // If caller is a manager, receiver must be the owner (cannot redirect funds).
        let (actual_owner, actual_receiver) = if is_position_order {
            // Check if caller is an authorized manager for this market
            match ds.get_position_manager(&caller, &params.market) {
                Some(owner) => {
                    // Caller is a manager; position owner is stored in data_store
                    // Receiver must be the owner (cannot redirect)
                    (owner.clone(), owner)
                }
                None => {
                    // Caller is not a manager; must be the owner
                    (caller.clone(), params.receiver)
                }
            }
        } else {
            // For swap orders, no position manager check needed
            (caller.clone(), params.receiver)
        };

        // Snapshot vault balance and derive received amount (canonical model — issue #47).
        // Reverts with ZeroCollateral if caller skipped the SendTokens pre-step.
        let collateral_delta_amount = if is_increase_or_swap {
            let received = OrderVaultClient::new(&env, &order_vault)
                .record_transfer_in(&params.initial_collateral_token);
            if received <= 0 {
                panic_with_error!(&env, Error::ZeroCollateral);
            }
            received
        } else {
            // Decrease/liquidation orders: no collateral deposit required.
            params.collateral_delta_amount
        };

        // Generate unique key
        let nonce = ds.increment_nonce(&handler);
        let key = order_key(&env, nonce);

        let order = OrderProps {
            account:                  actual_owner.clone(),
            receiver:                 actual_receiver,
            market:                   params.market.clone(),
            initial_collateral_token: params.initial_collateral_token,
            swap_path: params.swap_path,
            size_delta_usd: params.size_delta_usd,
            collateral_delta_amount,
            trigger_price: params.trigger_price,
            acceptable_price: params.acceptable_price,
            execution_fee: params.execution_fee,
            min_output_amount: params.min_output_amount,
            order_type: params.order_type,
            is_long: params.is_long,
            updated_at_time: env.ledger().timestamp(),
        };

        env.storage()
            .persistent()
            .set(&OrderStorageKey::Order(key.clone()), &order);

        ds.add_bytes32_to_set(&handler, &order_list_key(&env), &key);
        ds.add_bytes32_to_set(&handler, &account_order_list_key(&env, &actual_owner), &key);

        env.events().publish((symbol_short!("ord_crt"),), (key.clone(), actual_owner, params.market));
        key
    }

    /// Execute a pending order (called by keeper).
    pub fn execute_order(env: Env, keeper: Address, key: BytesN<32>) {
        keeper.require_auth();
        require_order_keeper(&env, &keeper);

        let data_store: Address = env
            .storage()
            .instance()
            .get(&InstanceKey::DataStore)
            .unwrap_or_else(|| panic_with_error!(&env, Error::NotInitialized));
        let order_vault: Address = env
            .storage()
            .instance()
            .get(&InstanceKey::OrderVault)
            .unwrap_or_else(|| panic_with_error!(&env, Error::NotInitialized));
        let oracle: Address = env
            .storage()
            .instance()
            .get(&InstanceKey::Oracle)
            .unwrap_or_else(|| panic_with_error!(&env, Error::NotInitialized));
        let handler = env.current_contract_address();

        // Load order
        let order: OrderProps = env
            .storage()
            .persistent()
            .get(&OrderStorageKey::Order(key.clone()))
            .unwrap_or_else(|| panic_with_error!(&env, Error::OrderNotFound));

        // Check frozen
        let is_frozen: bool = env
            .storage()
            .persistent()
            .get(&OrderStorageKey::OrderFrozen(key.clone()))
            .unwrap_or(false);
        if is_frozen {
            panic_with_error!(&env, Error::OrderFrozen);
        }

        let ds = DataStoreClient::new(&env, &data_store);
        if ds.get_bool(&is_market_paused_key(&env, &order.market)) {
            panic_with_error!(&env, Error::MarketPaused);
        }

        // Load market props
        let market = load_market_props(&env, &data_store, &order.market);

        // Fetch oracle prices
        let oracle_client = OracleClient::new(&env, &oracle);
        let index_price = oracle_client.get_primary_price(&market.index_token);
        let collateral_price = oracle_client
            .get_primary_price(&order.initial_collateral_token)
            .mid_price();

        // Trigger price checks for non-market orders
        match order.order_type {
            OrderType::LimitIncrease if index_price.min > order.trigger_price => {
                panic_with_error!(&env, Error::UnsatisfiedTrigger);
            }
            // StopIncrease fires when price rises to or above the trigger (buy-stop).
            // Reject execution while the index price is still below the trigger.
            OrderType::StopIncrease if index_price.min < order.trigger_price => {
                panic_with_error!(&env, Error::UnsatisfiedTrigger);
            }
            OrderType::LimitDecrease if index_price.max < order.trigger_price => {
                panic_with_error!(&env, Error::UnsatisfiedTrigger);
            }
            OrderType::StopLossDecrease if index_price.min > order.trigger_price => {
                panic_with_error!(&env, Error::UnsatisfiedTrigger);
            }
            // LimitSwap: execute only when the index price is at or below trigger_price.
            // A non-zero trigger_price means the user wants to swap only when the
            // index token is cheap enough (e.g. buy long_token when price <= trigger).
            // When trigger_price == 0 the check is skipped; min_output_amount is the
            // only guard in that case.
            OrderType::LimitSwap
                if order.trigger_price > 0 && index_price.min > order.trigger_price =>
            {
                panic_with_error!(&env, Error::UnsatisfiedTrigger);
            }
            _ => {}
        }

        // Dispatch by order type
        match order.order_type {
            OrderType::MarketSwap | OrderType::LimitSwap => {
                // Transfer collateral from vault to first market in path
                let first_market = order
                    .swap_path
                    .get(0)
                    .unwrap_or_else(|| panic_with_error!(&env, Error::InvalidOrderType));
                OrderVaultClient::new(&env, &order_vault).transfer_out(
                    &handler,
                    &order.initial_collateral_token,
                    &first_market,
                    &order.collateral_delta_amount,
                );
                let (_token_out, amount_out) = swap_with_path(
                    &env,
                    &data_store,
                    &handler,
                    &oracle,
                    &order.initial_collateral_token,
                    order.collateral_delta_amount,
                    &order.swap_path,
                    &order.receiver,
                );
                if amount_out < order.min_output_amount {
                    panic_with_error!(&env, Error::PriceTooLow);
                }
            }

            OrderType::MarketIncrease | OrderType::LimitIncrease | OrderType::StopIncrease => {
                // Transfer collateral from vault into the market pool
                OrderVaultClient::new(&env, &order_vault).transfer_out(
                    &handler,
                    &order.initial_collateral_token,
                    &market.market_token,
                    &order.collateral_delta_amount,
                );

                // Issue #204: apply fee tier based on 30-day rolling volume
                let effective_fee_factor =
                    resolve_fee_tier(&env, &data_store, &handler, &order.account, &market.market_token, order.size_delta_usd);
                let fee_key_pos = position_fee_factor_key(&env, &market.market_token, true);
                let fee_key_neg = position_fee_factor_key(&env, &market.market_token, false);
                let ds_client = DataStoreClient::new(&env, &data_store);
                let original_fee_pos = ds_client.get_u128(&fee_key_pos);
                let original_fee_neg = ds_client.get_u128(&fee_key_neg);
                ds_client.set_u128(&handler, &fee_key_pos, &effective_fee_factor);
                ds_client.set_u128(&handler, &fee_key_neg, &effective_fee_factor);

                // Issue #205: snapshot pre-increase state for event
                let pos_key = position_key(&env, &order.account, &market.market_token, &order.initial_collateral_token, order.is_long);
                let pre_position: Option<PositionProps> = env.storage().persistent().get(&PositionStorageKey::Position(pos_key.clone()));
                let is_new_position = pre_position.is_none();

                let updated = increase_position(
                    &env,
                    &IncreasePositionParams {
                        data_store: &data_store,
                        caller: &handler,
                        account: &order.account,
                        receiver: &order.receiver,
                        market: &market,
                        collateral_token: &order.initial_collateral_token,
                        size_delta_usd: order.size_delta_usd,
                        collateral_amount: order.collateral_delta_amount,
                        acceptable_price: order.acceptable_price,
                        is_long: order.is_long,
                        index_token_price: &index_price,
                        collateral_price,
                        current_time: env.ledger().timestamp(),
                    },
                );

                // Restore original fee factors
                ds_client.set_u128(&handler, &fee_key_pos, &original_fee_pos);
                ds_client.set_u128(&handler, &fee_key_neg, &original_fee_neg);

                // Issue #206: enforce max leverage
                let max_lev_key = max_leverage_key(&env, &market.market_token);
                let max_leverage_bps = ds_client.get_u128(&max_lev_key);
                if max_leverage_bps > 0 && updated.collateral_amount > 0 {
                    let effective_bps = (updated.size_in_usd * 100 / updated.collateral_amount) as u128;
                    if effective_bps > max_leverage_bps {
                        panic_with_error!(&env, Error::MaxLeverageExceeded);
                    }
                }

                // Issue #205: emit position event
                let avg_price = if updated.size_in_tokens > 0 {
                    mul_div_wide(&env, updated.size_in_usd, 1, updated.size_in_tokens)
                } else {
                    0
                };
                if is_new_position {
                    env.events().publish(
                        (symbol_short!("pos_open"),),
                        PositionOpenedEvent {
                            key: pos_key,
                            account: order.account.clone(),
                            market: market.market_token.clone(),
                            size_in_usd: updated.size_in_usd,
                            collateral_amount: updated.collateral_amount,
                            is_long: order.is_long,
                            avg_entry_price: avg_price,
                        },
                    );
                } else {
                    env.events().publish(
                        (symbol_short!("pos_inc"),),
                        PositionIncreasedEvent {
                            key: pos_key,
                            account: order.account.clone(),
                            market: market.market_token.clone(),
                            delta_size_usd: order.size_delta_usd,
                            delta_collateral: order.collateral_delta_amount,
                            new_size_usd: updated.size_in_usd,
                            avg_entry_price: avg_price,
                        },
                    );
                }
            }

            OrderType::MarketDecrease
            | OrderType::LimitDecrease
            | OrderType::StopLossDecrease
            | OrderType::Liquidation => {
                let pos_key = position_key(&env, &order.account, &market.market_token, &order.initial_collateral_token, order.is_long);
                let result = decrease_position(
                    &env,
                    &DecreasePositionParams {
                        data_store: &data_store,
                        caller: &handler,
                        account: &order.account,
                        receiver: &order.receiver,
                        market: &market,
                        collateral_token: &order.initial_collateral_token,
                        size_delta_usd: order.size_delta_usd,
                        acceptable_price: order.acceptable_price,
                        is_long: order.is_long,
                        index_token_price: &index_price,
                        collateral_price,
                        current_time: env.ledger().timestamp(),
                        swap_path: order.swap_path.clone(),
                        oracle: &oracle,
                    },
                );

                // Issue #205: emit decrease or close event
                if result.is_fully_closed {
                    env.events().publish(
                        (symbol_short!("pos_cls"),),
                        PositionClosedEvent {
                            key: pos_key,
                            account: order.account.clone(),
                            market: market.market_token.clone(),
                            pnl_usd: result.pnl_usd,
                            execution_price: result.execution_price,
                        },
                    );
                } else {
                    env.events().publish(
                        (symbol_short!("pos_dec"),),
                        PositionDecreasedEvent {
                            key: pos_key,
                            account: order.account.clone(),
                            market: market.market_token.clone(),
                            delta_size_usd: order.size_delta_usd,
                            pnl_usd: result.pnl_usd,
                            execution_price: result.execution_price,
                        },
                    );
                }
            }
        }

        // Remove order
        remove_order(&env, &data_store, &handler, &key, &order.account);

        // Issue #217: referral tier auto-upgrade — increment volume for the trader's referrer.
        if order.size_delta_usd > 0 {
            if let Some(ref_storage_addr) = env
                .storage()
                .instance()
                .get::<_, Address>(&InstanceKey::ReferralStorage)
            {
                let ref_client = ReferralStorageClient::new(&env, &ref_storage_addr);
                if let Some(referrer) = ref_client.get_trader_referrer(&order.account) {
                    ref_client.increment_referrer_volume(
                        &handler,
                        &referrer,
                        &(order.size_delta_usd as u128),
                    );
                }
            }
        }

        // Issue #249: record keeper liveness — stamp the ORDER_KEEPER role's last
        // activity with the current ledger so the protocol has an on-chain
        // heartbeat. The handler holds CONTROLLER, so it may write to data_store.
        record_keeper_activity(&env, &data_store, &handler, &roles::order_keeper(&env));

        env.events()
            .publish((symbol_short!("ord_exe"),), (key, order.account));
    }

    /// Cancel a pending order and refund collateral to the account.
    pub fn cancel_order(env: Env, caller: Address, key: BytesN<32>) {
        caller.require_auth();

        let data_store: Address = env
            .storage()
            .instance()
            .get(&InstanceKey::DataStore)
            .unwrap_or_else(|| panic_with_error!(&env, Error::NotInitialized));
        let order_vault: Address = env
            .storage()
            .instance()
            .get(&InstanceKey::OrderVault)
            .unwrap_or_else(|| panic_with_error!(&env, Error::NotInitialized));
        let role_store: Address = env
            .storage()
            .instance()
            .get(&InstanceKey::RoleStore)
            .unwrap_or_else(|| panic_with_error!(&env, Error::NotInitialized));
        let handler = env.current_contract_address();

        let order: OrderProps = env
            .storage()
            .persistent()
            .get(&OrderStorageKey::Order(key.clone()))
            .unwrap_or_else(|| panic_with_error!(&env, Error::OrderNotFound));

        let is_keeper =
            RoleStoreClient::new(&env, &role_store).has_role(&caller, &roles::order_keeper(&env));
        if caller != order.account && !is_keeper {
            panic_with_error!(&env, Error::Unauthorized);
        }

        // Refund collateral for increase/swap order types
        let needs_refund = matches!(
            order.order_type,
            OrderType::MarketIncrease
                | OrderType::LimitIncrease
                | OrderType::StopIncrease
                | OrderType::MarketSwap
                | OrderType::LimitSwap
        );
        if needs_refund && order.collateral_delta_amount > 0 {
            OrderVaultClient::new(&env, &order_vault).transfer_out(
                &handler,
                &order.initial_collateral_token,
                &order.account,
                &order.collateral_delta_amount,
            );
        }

        remove_order(&env, &data_store, &handler, &key, &order.account);

        env.events()
            .publish((symbol_short!("ord_can"),), (key, order.account));
    }

    /// Update a pending order's trigger/acceptable price or size delta.
    pub fn update_order(
        env: Env,
        caller: Address,
        key: BytesN<32>,
        size_delta_usd: i128,
        acceptable_price: i128,
        trigger_price: i128,
        min_output_amount: i128,
    ) {
        caller.require_auth();

        let mut order: OrderProps = env
            .storage()
            .persistent()
            .get(&OrderStorageKey::Order(key.clone()))
            .unwrap_or_else(|| panic_with_error!(&env, Error::OrderNotFound));

        if caller != order.account {
            panic_with_error!(&env, Error::Unauthorized);
        }

        order.size_delta_usd = size_delta_usd;
        order.acceptable_price = acceptable_price;
        order.trigger_price = trigger_price;
        order.min_output_amount = min_output_amount;
        order.updated_at_time = env.ledger().timestamp();

        env.storage()
            .persistent()
            .set(&OrderStorageKey::Order(key.clone()), &order);

        // Clear frozen flag if set (order is being updated = re-enabled)
        env.storage()
            .persistent()
            .remove(&OrderStorageKey::OrderFrozen(key.clone()));

        env.events()
            .publish((symbol_short!("ord_upd"),), (key, caller));
    }

    /// Freeze an order that cannot currently be executed.
    pub fn freeze_order(env: Env, keeper: Address, key: BytesN<32>) {
        keeper.require_auth();
        require_order_keeper(&env, &keeper);

        let _order: OrderProps = env
            .storage()
            .persistent()
            .get(&OrderStorageKey::Order(key.clone()))
            .unwrap_or_else(|| panic_with_error!(&env, Error::OrderNotFound));

        env.storage()
            .persistent()
            .set(&OrderStorageKey::OrderFrozen(key.clone()), &true);

        env.events().publish((symbol_short!("ord_frz"),), key);
    }

    /// Return a stored order by key, or None if not found.
    pub fn get_order(env: Env, key: BytesN<32>) -> Option<OrderProps> {
        env.storage().persistent().get(&OrderStorageKey::Order(key))
    }

    /// Return a stored position by its position_key (sha256 hash), or None.
    /// Used by liquidation_handler and adl_handler to check position health.
    pub fn get_position(env: Env, key: BytesN<32>) -> Option<PositionProps> {
        env.storage()
            .persistent()
            .get(&PositionStorageKey::Position(key))
    }

    /// Force-liquidate a position. Called by the liquidation_handler after role/health checks.
    ///
    /// Positions live in order_handler storage, so liquidation must run here.
    pub fn liquidate_position(
        env: Env,
        keeper: Address, // must have LIQUIDATION_KEEPER role
        account: Address,
        market: Address,
        collateral_token: Address,
        is_long: bool,
    ) {
        keeper.require_auth();
        require_liquidation_keeper(&env, &keeper);

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
        let order_vault: Address = env.storage().instance().get(&InstanceKey::OrderVault)
            .unwrap_or_else(|| panic_with_error!(&env, Error::NotInitialized));
        let handler = env.current_contract_address();

        let market_props = load_market_props(&env, &data_store, &market);
        let oracle_client = OracleClient::new(&env, &oracle);
        let index_price = oracle_client.get_primary_price(&market_props.index_token);
        let collateral_price = oracle_client
            .get_primary_price(&collateral_token)
            .mid_price();

        // Load position to get size
        use gmx_keys::position_key;
        let pk = position_key(&env, &account, &market, &collateral_token, is_long);
        let position: PositionProps = env
            .storage()
            .persistent()
            .get(&PositionStorageKey::Position(pk.clone()))
            .unwrap_or_else(|| panic_with_error!(&env, Error::OrderNotFound));

        // Validate liquidatability
        if !gmx_position_utils::is_liquidatable(
            &env,
            &data_store,
            &position,
            &market_props,
            collateral_price,
            &index_price,
        ) {
            panic_with_error!(&env, Error::InvalidOrderType);
        }

        // Handle keeper execution fee (feature #208): deduct from position collateral first
        let ds = DataStoreClient::new(&env, &data_store);
        let keeper_fee_key = liquidation_execution_fee_key(&env, &market);
        let keeper_execution_fee = ds.get_u128(&keeper_fee_key);

        if keeper_execution_fee > 0 {
            let keeper_fee_i128 = keeper_execution_fee as i128;
            let fee_to_transfer = if keeper_fee_i128 <= position.collateral_amount {
                keeper_fee_i128
            } else {
                position.collateral_amount
            };

            if fee_to_transfer > 0 {
                OrderVaultClient::new(&env, &order_vault).transfer_out(
                    &handler,
                    &collateral_token,
                    &keeper,
                    &fee_to_transfer,
                );
            }
        }

        let liq_pos_key = position_key(&env, &account, &market, &collateral_token, is_long);
        let result = decrease_position(
            &env,
            &DecreasePositionParams {
                data_store: &data_store,
                caller: &handler,
                account: &account,
                receiver: &account,
                market: &market_props,
                collateral_token: &collateral_token,
                size_delta_usd: position.size_in_usd,
                acceptable_price: 0,
                is_long,
                index_token_price: &index_price,
                collateral_price,
                current_time: env.ledger().timestamp(),
                swap_path: soroban_sdk::Vec::new(&env),
                oracle: &oracle,
            },
        );

        // Issue #205: structured liquidation event
        env.events().publish(
            (symbol_short!("pos_liq"),),
            PositionLiquidatedEvent {
                key: liq_pos_key,
                account: account.clone(),
                market: market.clone(),
                execution_price: result.execution_price,
                remaining_collateral: result.remaining_collateral,
            },
        );
    }

    /// Partially close a profitable position for ADL. Called by adl_handler after checks.
    pub fn execute_adl(
        env: Env,
        keeper: Address, // must have ADL_KEEPER role
        account: Address,
        market: Address,
        collateral_token: Address,
        is_long: bool,
        size_delta_usd: i128,
    ) {
        keeper.require_auth();
        require_adl_keeper(&env, &keeper);

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
        let handler = env.current_contract_address();

        let market_props = load_market_props(&env, &data_store, &market);
        let oracle_client = OracleClient::new(&env, &oracle);
        let index_price = oracle_client.get_primary_price(&market_props.index_token);
        let collateral_price = oracle_client
            .get_primary_price(&collateral_token)
            .mid_price();

        let result = decrease_position(
            &env,
            &DecreasePositionParams {
                data_store: &data_store,
                caller: &handler,
                account: &account,
                receiver: &account,
                market: &market_props,
                collateral_token: &collateral_token,
                size_delta_usd,
                acceptable_price: 0,
                is_long,
                index_token_price: &index_price,
                collateral_price,
                current_time: env.ledger().timestamp(),
                swap_path: soroban_sdk::Vec::new(&env),
                oracle: &oracle,
            },
        );

        env.events().publish(
            (symbol_short!("adl_exe"),),
            (account, market, size_delta_usd, result.pnl_usd),
        );
    }
}


// ─── Fee tier resolution (issue #204) ────────────────────────────────────────

/// Walk fee tiers 0-4, find the best (lowest) fee factor the trader qualifies for
/// based on their 30-day rolling volume. Returns the qualifying fee factor or 0.
fn resolve_fee_tier(
    env: &Env,
    data_store: &Address,
    caller: &Address,
    account: &Address,
    market: &Address,
    size_delta_usd: i128,
) -> u128 {
    const WINDOW_LEDGERS: u32 = 518_400; // 30 days × 24h × 720 ledgers/h
    let ds = DataStoreClient::new(env, data_store);

    let vol_key = trader_volume_key(env, account, market);
    let win_key = trader_volume_window_start_key(env, account, market);

    let current_ledger: u32 = env.ledger().sequence();
    let window_start = ds.get_u128(&win_key) as u32;

    // Reset window if expired
    let current_volume = if current_ledger.saturating_sub(window_start) > WINDOW_LEDGERS {
        ds.set_u128(caller, &win_key, &(current_ledger as u128));
        ds.set_u128(caller, &vol_key, &(size_delta_usd as u128));
        size_delta_usd as u128
    } else {
        ds.apply_delta_to_u128(caller, &vol_key, &size_delta_usd)
    };

    // Walk tiers 0-4, return best qualifying fee
    let mut best_fee: u128 = 0;
    for tier in 0u32..5u32 {
        let thresh_key = fee_tier_volume_threshold_key(env, market, tier);
        let threshold = ds.get_u128(&thresh_key);
        if threshold == 0 {
            break; // tier not configured
        }
        if current_volume >= threshold {
            let factor_key = fee_tier_position_fee_factor_key(env, market, tier);
            let fee_factor = ds.get_u128(&factor_key);
            if fee_factor > 0 {
                best_fee = fee_factor;
            }
        }
    }
    best_fee
}

// ─── Helpers ──────────────────────────────────────────────────────────────────

fn require_order_keeper(env: &Env, caller: &Address) {
    let role_store: Address = env
        .storage()
        .instance()
        .get(&InstanceKey::RoleStore)
        .unwrap_or_else(|| panic_with_error!(env, Error::NotInitialized));
    if !RoleStoreClient::new(env, &role_store).has_role(caller, &roles::order_keeper(env)) {
        panic_with_error!(env, Error::Unauthorized);
    }
}

fn require_liquidation_keeper(env: &Env, caller: &Address) {
    let role_store: Address = env
        .storage()
        .instance()
        .get(&InstanceKey::RoleStore)
        .unwrap_or_else(|| panic_with_error!(env, Error::NotInitialized));
    if !RoleStoreClient::new(env, &role_store).has_role(caller, &roles::liquidation_keeper(env)) {
        panic_with_error!(env, Error::Unauthorized);
    }
}

fn require_adl_keeper(env: &Env, caller: &Address) {
    let role_store: Address = env
        .storage()
        .instance()
        .get(&InstanceKey::RoleStore)
        .unwrap_or_else(|| panic_with_error!(env, Error::NotInitialized));
    if !RoleStoreClient::new(env, &role_store).has_role(caller, &roles::adl_keeper(env)) {
        panic_with_error!(env, Error::Unauthorized);
    }
}

fn load_market_props(env: &Env, data_store: &Address, market_token: &Address) -> MarketProps {
    let ds = DataStoreClient::new(env, data_store);
    let index_token = ds
        .get_address(&market_index_token_key(env, market_token))
        .expect("market index token not found");
    let long_token = ds
        .get_address(&market_long_token_key(env, market_token))
        .expect("market long token not found");
    let short_token = ds
        .get_address(&market_short_token_key(env, market_token))
        .expect("market short token not found");
    MarketProps {
        market_token: market_token.clone(),
        index_token,
        long_token,
        short_token,
    }
}

fn remove_order(
    env: &Env,
    data_store: &Address,
    caller: &Address,
    key: &BytesN<32>,
    account: &Address,
) {
    env.storage()
        .persistent()
        .remove(&OrderStorageKey::Order(key.clone()));
    env.storage()
        .persistent()
        .remove(&OrderStorageKey::OrderFrozen(key.clone()));
    let ds = DataStoreClient::new(env, data_store);
    ds.remove_bytes32_from_set(caller, &order_list_key(env), key);
    ds.remove_bytes32_from_set(caller, &account_order_list_key(env, account), key);
}

/// Issue #249: stamp the current ledger sequence as `role`'s last activity.
/// `caller` must hold CONTROLLER in data_store (the handler does).
fn record_keeper_activity(env: &Env, data_store: &Address, caller: &Address, role: &BytesN<32>) {
    let ledger = env.ledger().sequence() as u128;
    DataStoreClient::new(env, data_store).set_u128(
        caller,
        &last_keeper_activity_key(env, role),
        &ledger,
    );
}

/// Read `role`'s configured heartbeat timeout, falling back to the default when
/// unset (issue #249).
fn keeper_heartbeat_timeout(env: &Env, data_store: &Address, role: &BytesN<32>) -> u64 {
    let stored =
        DataStoreClient::new(env, data_store).get_u128(&keeper_heartbeat_timeout_key(env, role));
    if stored == 0 {
        DEFAULT_KEEPER_HEARTBEAT_TIMEOUT
    } else {
        stored as u64
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use data_store::{DataStore, DataStoreClient as DsClient};
    use deposit_handler::{CreateDepositParams, DepositHandler, DepositHandlerClient};
    use deposit_vault::{DepositVault, DepositVaultClient as DVClient};
    use gmx_keys::{position_key, roles};
    use gmx_types::TokenPrice;
    use soroban_sdk::testutils::Ledger as _;
    use market_token::{MarketToken, MarketTokenClient as MtClient};
    use oracle::{Oracle, OracleClient as OClient};
    use order_vault::{OrderVault, OrderVaultClient as OVClient};
    use role_store::{RoleStore, RoleStoreClient as RsClient};
    use soroban_sdk::{
        testutils::{Address as _, BytesN as _, Ledger as _},
        token::StellarAssetClient,
        BytesN, Env, Vec,
    };

    const COLLATERAL: i128 = 1_000_0000;

    struct World {
        env: Env,
        admin: Address,
        keeper: Address,
        user: Address,
        rs: Address,
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
        rs_c.grant_role(&admin, &keeper, &roles::liquidation_keeper(&env));

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
        DepositHandlerClient::new(&env, &dep_handler).initialize(
            &admin,
            &rs,
            &ds,
            &oracle_addr,
            &dep_vault,
        );
        rs_c.grant_role(&admin, &dep_handler, &roles::controller(&env));

        let ord_handler = env.register(OrderHandler, ());
        OrderHandlerClient::new(&env, &ord_handler).initialize(
            &admin,
            &rs,
            &ds,
            &oracle_addr,
            &ord_vault,
        );
        rs_c.grant_role(&admin, &ord_handler, &roles::controller(&env));

        let long_tk = env
            .register_stellar_asset_contract_v2(admin.clone())
            .address();
        let short_tk = env
            .register_stellar_asset_contract_v2(admin.clone())
            .address();
        let index_tk = Address::generate(&env);

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

        World {
            env,
            admin,
            keeper,
            user,
            rs,
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
        let fp = gmx_math::FLOAT_PRECISION;
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

    fn seed_pool(w: &World) {
        let lp = Address::generate(&w.env);
        StellarAssetClient::new(&w.env, &w.long_tk).mint(&lp, &10_000_0000i128);
        StellarAssetClient::new(&w.env, &w.short_tk).mint(&lp, &5_000_0000i128);
        set_prices(w, 2000 * gmx_math::FLOAT_PRECISION);
        let k = DepositHandlerClient::new(&w.env, &w.dep_handler).create_deposit(
            &lp,
            &CreateDepositParams {
                receiver: lp.clone(),
                market: w.market_tk.clone(),
                initial_long_token: w.long_tk.clone(),
                initial_short_token: w.short_tk.clone(),
                long_token_amount: 10_000_0000,
                short_token_amount: 5_000_0000,
                min_market_tokens: 1,
                execution_fee: 0,
            },
        );
        DepositHandlerClient::new(&w.env, &w.dep_handler).execute_deposit(&w.keeper, &k);
    }

    fn create_increase_order(
        w: &World,
        order_type: OrderType,
        trigger_price: i128,
    ) -> (OrderHandlerClient<'_>, BytesN<32>) {
        StellarAssetClient::new(&w.env, &w.long_tk).mint(&w.ord_vault, &COLLATERAL);
        let hc = OrderHandlerClient::new(&w.env, &w.ord_handler);
        let key = hc.create_order(
            &w.user,
            &CreateOrderParams {
                receiver: w.user.clone(),
                market: w.market_tk.clone(),
                initial_collateral_token: w.long_tk.clone(),
                swap_path: Vec::new(&w.env),
                size_delta_usd: 2000 * gmx_math::FLOAT_PRECISION,
                collateral_delta_amount: COLLATERAL,
                trigger_price,
                acceptable_price: 0,
                execution_fee: 0,
                min_output_amount: 0,
                order_type,
                is_long: true,
            },
        );
        (hc, key)
    }

    fn create_stop_increase(
        w: &World,
        user: &Address,
        collateral: i128,
        trigger_price: i128,
        acceptable_price: i128,
    ) -> BytesN<32> {
        soroban_sdk::token::Client::new(&w.env, &w.long_tk).transfer(
            user,
            &w.ord_vault,
            &collateral,
        );
        OrderHandlerClient::new(&w.env, &w.ord_handler).create_order(
            user,
            &CreateOrderParams {
                receiver: user.clone(),
                market: w.market_tk.clone(),
                initial_collateral_token: w.long_tk.clone(),
                swap_path: Vec::new(&w.env),
                size_delta_usd: collateral,
                collateral_delta_amount: collateral,
                trigger_price,
                acceptable_price,
                execution_fee: 0,
                min_output_amount: 0,
                order_type: OrderType::StopIncrease,
                is_long: true,
            },
        )
    }

    // ── Issue #50: StopIncrease trigger-boundary tests ────────────────────────

    #[test]
    fn stop_increase_at_trigger_price_executes() {
        let w = setup();
        let fp = gmx_math::FLOAT_PRECISION;
        let user = Address::generate(&w.env);
        let collateral = 1_000_0000i128;
        StellarAssetClient::new(&w.env, &w.long_tk).mint(&user, &collateral);
        let trigger = 2000 * fp;
        set_prices(&w, trigger);
        let key = create_stop_increase(&w, &user, collateral, trigger, 0);
        set_prices(&w, trigger);
        OrderHandlerClient::new(&w.env, &w.ord_handler).execute_order(&w.keeper, &key);
        assert!(
            OrderHandlerClient::new(&w.env, &w.ord_handler)
                .get_order(&key)
                .is_none(),
            "order must be removed after successful execution"
        );
    }

    #[test]
    fn stop_increase_above_trigger_executes() {
        let w = setup();
        let fp = gmx_math::FLOAT_PRECISION;
        let user = Address::generate(&w.env);
        let collateral = 500_0000i128;
        StellarAssetClient::new(&w.env, &w.long_tk).mint(&user, &collateral);
        let trigger = 1800 * fp;
        set_prices(&w, 2000 * fp);
        let key = create_stop_increase(&w, &user, collateral, trigger, 0);
        set_prices(&w, 2000 * fp);
        OrderHandlerClient::new(&w.env, &w.ord_handler).execute_order(&w.keeper, &key);
        assert!(
            OrderHandlerClient::new(&w.env, &w.ord_handler)
                .get_order(&key)
                .is_none(),
            "order must be removed after successful execution above trigger"
        );
    }

    #[test]
    #[should_panic]
    fn stop_increase_below_trigger_reverts() {
        let w = setup();
        let fp = gmx_math::FLOAT_PRECISION;
        let user = Address::generate(&w.env);
        let collateral = 1_000_0000i128;
        StellarAssetClient::new(&w.env, &w.long_tk).mint(&user, &collateral);
        let trigger = 2500 * fp;
        set_prices(&w, 2000 * fp);
        let key = create_stop_increase(&w, &user, collateral, trigger, 0);
        set_prices(&w, 2000 * fp);
        OrderHandlerClient::new(&w.env, &w.ord_handler).execute_order(&w.keeper, &key);
    }

    #[test]
    fn stop_increase_cancel_refunds_collateral() {
        let w = setup();
        let fp = gmx_math::FLOAT_PRECISION;
        let user = Address::generate(&w.env);
        let collateral = 800_0000i128;
        StellarAssetClient::new(&w.env, &w.long_tk).mint(&user, &collateral);
        let trigger = 2500 * fp;
        set_prices(&w, 2000 * fp);
        let key = create_stop_increase(&w, &user, collateral, trigger, 0);
        let bal_before = soroban_sdk::token::Client::new(&w.env, &w.long_tk).balance(&user);
        OrderHandlerClient::new(&w.env, &w.ord_handler).cancel_order(&user, &key);
        let bal_after = soroban_sdk::token::Client::new(&w.env, &w.long_tk).balance(&user);
        assert!(
            OrderHandlerClient::new(&w.env, &w.ord_handler)
                .get_order(&key)
                .is_none(),
            "order removed after cancel"
        );
        assert_eq!(
            bal_after - bal_before,
            collateral,
            "full collateral must be refunded on cancel"
        );
    }

    // ── Issue #47: collateral model guard ────────────────────────────────────

    #[test]
    #[should_panic]
    fn create_order_without_collateral_reverts() {
        let w = setup();
        OrderHandlerClient::new(&w.env, &w.ord_handler).create_order(
            &w.user,
            &CreateOrderParams {
                receiver: w.user.clone(),
                market: w.market_tk.clone(),
                initial_collateral_token: w.long_tk.clone(),
                swap_path: Vec::new(&w.env),
                size_delta_usd: 2000 * gmx_math::FLOAT_PRECISION,
                collateral_delta_amount: COLLATERAL,
                trigger_price: 0,
                acceptable_price: 0,
                execution_fee: 0,
                min_output_amount: 0,
                order_type: OrderType::MarketIncrease,
                is_long: true,
            },
        );
    }

    #[test]
    fn vault_balance_invariant_holds_after_create() {
        let w = setup();
        let (_hc, _key) = create_increase_order(&w, OrderType::MarketIncrease, 0);
        let ov = OVClient::new(&w.env, &w.ord_vault);
        let recorded = ov.get_recorded_balance(&w.long_tk);
        let on_chain = soroban_sdk::token::Client::new(&w.env, &w.long_tk).balance(&w.ord_vault);
        assert_eq!(recorded, on_chain, "vault recorded ≠ on-chain balance");
        assert_eq!(
            recorded, COLLATERAL,
            "vault should hold exactly the deposited collateral"
        );
    }

    #[test]
    fn cancel_order_refunds_collateral_to_user() {
        let w = setup();
        let (hc, key) = create_increase_order(&w, OrderType::MarketIncrease, 0);
        let before = soroban_sdk::token::Client::new(&w.env, &w.long_tk).balance(&w.user);
        hc.cancel_order(&w.user, &key);
        let after = soroban_sdk::token::Client::new(&w.env, &w.long_tk).balance(&w.user);
        assert_eq!(
            after - before,
            COLLATERAL,
            "user should receive full collateral refund"
        );
        assert!(
            hc.get_order(&key).is_none(),
            "order must be removed after cancel"
        );
        assert_eq!(
            OVClient::new(&w.env, &w.ord_vault).get_recorded_balance(&w.long_tk),
            0,
            "vault recorded balance must be zero after refund"
        );
    }

    #[test]
    #[should_panic]
    fn double_create_order_without_new_deposit_reverts() {
        let w = setup();
        let _ = create_increase_order(&w, OrderType::MarketIncrease, 0);
        OrderHandlerClient::new(&w.env, &w.ord_handler).create_order(
            &w.user,
            &CreateOrderParams {
                receiver: w.user.clone(),
                market: w.market_tk.clone(),
                initial_collateral_token: w.long_tk.clone(),
                swap_path: Vec::new(&w.env),
                size_delta_usd: 2000 * gmx_math::FLOAT_PRECISION,
                collateral_delta_amount: COLLATERAL,
                trigger_price: 0,
                acceptable_price: 0,
                execution_fee: 0,
                min_output_amount: 0,
                order_type: OrderType::MarketIncrease,
                is_long: true,
            },
        );
    }

    // ── Issue #49: limit increase order lifecycle ─────────────────────────────

    #[test]
    #[should_panic]
    fn limit_increase_unsatisfied_trigger_reverts() {
        let w = setup();
        let fp = gmx_math::FLOAT_PRECISION;
        let (hc, key) = create_increase_order(&w, OrderType::LimitIncrease, 1000 * fp);
        set_prices(&w, 2000 * fp);
        hc.execute_order(&w.keeper, &key);
    }

    #[test]
    fn limit_increase_satisfied_trigger_creates_position() {
        let w = setup();
        let fp = gmx_math::FLOAT_PRECISION;
        let (hc, key) = create_increase_order(&w, OrderType::LimitIncrease, 3000 * fp);
        set_prices(&w, 2000 * fp);
        hc.execute_order(&w.keeper, &key);
        assert!(
            hc.get_order(&key).is_none(),
            "order should be removed after execution"
        );
        let pk = position_key(&w.env, &w.user, &w.market_tk, &w.long_tk, true);
        let position = hc
            .get_position(&pk)
            .expect("position must exist after limit increase execution");
        assert!(
            position.size_in_usd > 0,
            "position size_in_usd must be positive"
        );
        assert_eq!(
            position.market, w.market_tk,
            "position market must match order market"
        );
        assert!(position.is_long, "position must be long");
        assert_eq!(
            position.collateral_token, w.long_tk,
            "collateral token must match"
        );
        let long_oi = DsClient::new(&w.env, &w.ds).get_u128(&gmx_keys::open_interest_key(
            &w.env,
            &w.market_tk,
            &w.long_tk,
            true,
        ));
        assert!(
            long_oi > 0,
            "long open interest must increase after limit increase execution"
        );
    }

    // ── Issue #32: order storage cleanup tests ────────────────────────────────

    #[test]
    fn cancel_order_cleans_up_storage_and_lists() {
        let w = setup();
        let env = &w.env;
        let user = Address::generate(env);
        StellarAssetClient::new(env, &w.long_tk).mint(&user, &1_000_0000i128);
        soroban_sdk::token::Client::new(env, &w.long_tk).transfer(
            &user,
            &w.ord_vault,
            &1_000_0000i128,
        );
        let hc = OrderHandlerClient::new(env, &w.ord_handler);
        let ds_c = DsClient::new(env, &w.ds);
        let key = hc.create_order(
            &user,
            &CreateOrderParams {
                receiver: user.clone(),
                market: w.market_tk.clone(),
                initial_collateral_token: w.long_tk.clone(),
                swap_path: Vec::new(env),
                size_delta_usd: 500_000_0000i128,
                collateral_delta_amount: 1_000_0000i128,
                trigger_price: 0,
                acceptable_price: i128::MAX,
                execution_fee: 0,
                min_output_amount: 0,
                order_type: OrderType::MarketIncrease,
                is_long: true,
            },
        );
        assert!(hc.get_order(&key).is_some());
        assert!(ds_c.contains_bytes32(&gmx_keys::order_list_key(env), &key));
        assert!(ds_c.contains_bytes32(&gmx_keys::account_order_list_key(env, &user), &key));
        hc.cancel_order(&user, &key);
        assert!(
            hc.get_order(&key).is_none(),
            "record must be removed after cancel"
        );
        assert!(
            !ds_c.contains_bytes32(&gmx_keys::order_list_key(env), &key),
            "global order list must not contain key after cancel"
        );
        assert!(
            !ds_c.contains_bytes32(&gmx_keys::account_order_list_key(env, &user), &key),
            "account order list must not contain key after cancel"
        );
    }

    #[test]
    fn execute_order_cleans_up_storage_and_lists() {
        let w = setup();
        let env = &w.env;
        seed_pool(&w);
        set_prices(&w, 2000 * gmx_math::FLOAT_PRECISION);
        let user = Address::generate(env);
        StellarAssetClient::new(env, &w.long_tk).mint(&user, &1_000_0000i128);
        soroban_sdk::token::Client::new(env, &w.long_tk).transfer(
            &user,
            &w.ord_vault,
            &1_000_0000i128,
        );
        let hc = OrderHandlerClient::new(env, &w.ord_handler);
        let ds_c = DsClient::new(env, &w.ds);
        let key = hc.create_order(
            &user,
            &CreateOrderParams {
                receiver: user.clone(),
                market: w.market_tk.clone(),
                initial_collateral_token: w.long_tk.clone(),
                swap_path: Vec::new(env),
                size_delta_usd: 500_000_0000i128,
                collateral_delta_amount: 1_000_0000i128,
                trigger_price: 0,
                acceptable_price: i128::MAX,
                execution_fee: 0,
                min_output_amount: 0,
                order_type: OrderType::MarketIncrease,
                is_long: true,
            },
        );
        assert!(hc.get_order(&key).is_some());
        assert!(ds_c.contains_bytes32(&gmx_keys::order_list_key(env), &key));
        assert!(ds_c.contains_bytes32(&gmx_keys::account_order_list_key(env, &user), &key));
        hc.execute_order(&w.keeper, &key);
        assert!(
            hc.get_order(&key).is_none(),
            "record must be removed after execute"
        );
        assert!(
            !ds_c.contains_bytes32(&gmx_keys::order_list_key(env), &key),
            "global order list must not contain key after execute"
        );
        assert!(
            !ds_c.contains_bytes32(&gmx_keys::account_order_list_key(env, &user), &key),
            "account order list must not contain key after execute"
        );
    }

    // ── Frozen order tests (Task 2) ───────────────────────────────────────────

    /// Frozen order cannot be executed — execute_order must panic with OrderFrozen.
    #[test]
    #[should_panic]
    fn execute_frozen_order_reverts() {
        let w = setup();
        let (_hc, key) = create_increase_order(&w, OrderType::MarketIncrease, 0);
        let hc = OrderHandlerClient::new(&w.env, &w.ord_handler);
        hc.freeze_order(&w.keeper, &key);
        // Panic expected at the OrderFrozen check (before oracle is consulted)
        hc.execute_order(&w.keeper, &key);
    }

    /// update_order on a frozen order succeeds and clears the frozen flag,
    /// allowing the order to be re-executed afterward.
    #[test]
    fn update_frozen_order_clears_frozen_and_allows_execution() {
        let w = setup();
        seed_pool(&w);
        let (_hc, key) = create_increase_order(&w, OrderType::MarketIncrease, 0);
        let hc = OrderHandlerClient::new(&w.env, &w.ord_handler);
        hc.freeze_order(&w.keeper, &key);
        // Update while frozen: size_delta_usd, acceptable_price, trigger_price, min_output_amount
        hc.update_order(
            &w.user,
            &key,
            &(2000 * gmx_math::FLOAT_PRECISION),
            &0i128,
            &0i128,
            &0i128,
        );
        // After update the freeze is cleared; execution must succeed
        set_prices(&w, 2000 * gmx_math::FLOAT_PRECISION);
        hc.execute_order(&w.keeper, &key);
        assert!(
            hc.get_order(&key).is_none(),
            "order must be consumed after re-execution"
        );
    }

    /// cancel_order on a frozen order succeeds and refunds collateral.
    #[test]
    fn cancel_frozen_order_refunds_collateral() {
        let w = setup();
        let (hc, key) = create_increase_order(&w, OrderType::MarketIncrease, 0);
        hc.freeze_order(&w.keeper, &key);
        let bal_before = soroban_sdk::token::Client::new(&w.env, &w.long_tk).balance(&w.user);
        hc.cancel_order(&w.user, &key);
        let bal_after = soroban_sdk::token::Client::new(&w.env, &w.long_tk).balance(&w.user);
        assert_eq!(
            bal_after - bal_before,
            COLLATERAL,
            "frozen order cancel must refund collateral"
        );
        assert!(
            hc.get_order(&key).is_none(),
            "frozen order must be removed after cancel"
        );
    }

    /// Non-keeper cannot freeze an order — Unauthorized error expected.
    #[test]
    #[should_panic]
    fn freeze_order_by_non_keeper_reverts() {
        let w = setup();
        let (_hc, key) = create_increase_order(&w, OrderType::MarketIncrease, 0);
        let intruder = Address::generate(&w.env);
        OrderHandlerClient::new(&w.env, &w.ord_handler).freeze_order(&intruder, &key);
    }

    // ── Issue #10: upgrade entrypoint tests ───────────────────────────────────

    /// Admin auth passes on upgrade; panics at WASM lookup (not auth) in unit tests.
    /// A compiled WASM binary is required for the host to accept the hash.
    #[test]
    #[should_panic]
    fn upgrade_admin_succeeds() {
        let w = setup(); // mock_all_auths active — admin.require_auth() passes silently
                         // Panics at WASM lookup (not at auth) — proves auth gate is open for admin.
        OrderHandlerClient::new(&w.env, &w.ord_handler).upgrade(&BytesN::random(&w.env));
    }

    /// Calling upgrade without the admin's authorisation must revert.
    #[test]
    #[should_panic]
    fn upgrade_non_admin_reverts() {
        // Fresh env — require_auth() is not mocked.
        let env = Env::default();

        let admin = Address::generate(&env);
        let rs = Address::generate(&env);
        let ds = Address::generate(&env);
        let oracle = Address::generate(&env);
        let ord_vault = Address::generate(&env);

        let ord = env.register(OrderHandler, ());

        // Seed instance storage directly to skip initialize() auth.
        env.as_contract(&ord, || {
            env.storage()
                .instance()
                .set(&InstanceKey::Initialized, &true);
            env.storage().instance().set(&InstanceKey::Admin, &admin);
            env.storage().instance().set(&InstanceKey::RoleStore, &rs);
            env.storage().instance().set(&InstanceKey::DataStore, &ds);
            env.storage().instance().set(&InstanceKey::Oracle, &oracle);
            env.storage()
                .instance()
                .set(&InstanceKey::OrderVault, &ord_vault);
        });

        // No auth context provided — must panic at admin.require_auth().
        let hash = BytesN::from_array(&env, &[0u8; 32]);
        OrderHandlerClient::new(&env, &ord).upgrade(&hash);
    }

    /// Orders and positions written before upgrade remain accessible after.
    /// Requires a compiled WASM binary — skipped in unit-test mode.
    #[test]
    #[ignore]
    fn upgrade_preserves_order_and_position_storage() {
        let w = setup();
        let fp = gmx_math::FLOAT_PRECISION;
        set_prices(&w, 2_000 * fp);
        seed_pool(&w);

        // Create and execute an order so a position is written to persistent storage.
        set_prices(&w, 2_000 * fp);
        let (hc, key) = create_increase_order(&w, OrderType::MarketIncrease, 0);
        hc.execute_order(&w.keeper, &key);

        let pos_key = position_key(&w.env, &w.user, &w.market_tk, &w.long_tk, true);
        assert!(
            hc.get_position(&pos_key).is_some(),
            "position must exist before upgrade"
        );

        // Upgrade.
        OrderHandlerClient::new(&w.env, &w.ord_handler).upgrade(&BytesN::random(&w.env));

        // Position in persistent storage must survive the upgrade.
        assert!(
            hc.get_position(&pos_key).is_some(),
            "position must still be readable after upgrade"
        );
    }

    // ── Issue #56: limit swap order lifecycle tests ───────────────────────────
    //
    // LimitSwap trigger semantics:
    //   trigger_price > 0 → execute only when index_price.min <= trigger_price
    //   trigger_price = 0 → no price gate; min_output_amount is the only guard
    //
    // "Stale or unfavorable price" = index_price.min > trigger_price → reverts.
    // "Favorable price"            = index_price.min <= trigger_price → executes.

    fn create_limit_swap_order(
        w: &World,
        user: &Address,
        collateral: i128,
        token_in: &Address,
        trigger_price: i128,
        min_output: i128,
    ) -> BytesN<32> {
        soroban_sdk::token::Client::new(&w.env, token_in).transfer(user, &w.ord_vault, &collateral);
        OrderHandlerClient::new(&w.env, &w.ord_handler).create_order(
            user,
            &CreateOrderParams {
                receiver: user.clone(),
                market: w.market_tk.clone(),
                initial_collateral_token: token_in.clone(),
                swap_path: Vec::from_array(&w.env, [w.market_tk.clone()]),
                size_delta_usd: 0,
                collateral_delta_amount: collateral,
                trigger_price,
                acceptable_price: 0,
                execution_fee: 0,
                min_output_amount: min_output,
                order_type: OrderType::LimitSwap,
                is_long: false,
            },
        )
    }

    /// LimitSwap with index_price.min > trigger_price must revert (unfavorable price).
    #[test]
    #[should_panic]
    fn limit_swap_above_trigger_price_reverts() {
        let w = setup();
        let fp = gmx_math::FLOAT_PRECISION;
        seed_pool(&w);

        let user = Address::generate(&w.env);
        StellarAssetClient::new(&w.env, &w.short_tk).mint(&user, &1_000_0000i128);

        // Current price = 2000; trigger = 1500 → 2000 > 1500 → should revert
        let trigger = 1500 * fp;
        set_prices(&w, 2000 * fp);
        let key = create_limit_swap_order(&w, &user, 1_000_0000, &w.short_tk, trigger, 0);

        // Price is still 2000, above the 1500 trigger → UnsatisfiedTrigger
        set_prices(&w, 2000 * fp);
        OrderHandlerClient::new(&w.env, &w.ord_handler).execute_order(&w.keeper, &key);
    }

    /// LimitSwap with index_price.min == trigger_price must execute.
    #[test]
    fn limit_swap_at_trigger_price_executes() {
        let w = setup();
        let fp = gmx_math::FLOAT_PRECISION;
        seed_pool(&w);

        let user = Address::generate(&w.env);
        StellarAssetClient::new(&w.env, &w.short_tk).mint(&user, &1_000_0000i128);

        // trigger = 2000; price = 2000 → condition: 2000 <= 2000 → must execute
        let trigger = 2000 * fp;
        set_prices(&w, trigger);
        let key = create_limit_swap_order(&w, &user, 1_000_0000, &w.short_tk, trigger, 0);

        set_prices(&w, trigger);
        OrderHandlerClient::new(&w.env, &w.ord_handler).execute_order(&w.keeper, &key);

        assert!(
            OrderHandlerClient::new(&w.env, &w.ord_handler)
                .get_order(&key)
                .is_none(),
            "order must be consumed after execution at trigger price"
        );
        assert!(
            soroban_sdk::token::Client::new(&w.env, &w.long_tk).balance(&user) > 0,
            "user should receive long_tk output"
        );
    }

    /// LimitSwap with index_price.min < trigger_price must execute (favorable price).
    #[test]
    fn limit_swap_below_trigger_price_executes() {
        let w = setup();
        let fp = gmx_math::FLOAT_PRECISION;
        seed_pool(&w);

        let user = Address::generate(&w.env);
        StellarAssetClient::new(&w.env, &w.short_tk).mint(&user, &1_000_0000i128);

        // trigger = 2500; current price = 2000 → 2000 <= 2500 → favorable
        let trigger = 2500 * fp;
        set_prices(&w, 2000 * fp);
        let key = create_limit_swap_order(&w, &user, 1_000_0000, &w.short_tk, trigger, 0);

        set_prices(&w, 2000 * fp);
        OrderHandlerClient::new(&w.env, &w.ord_handler).execute_order(&w.keeper, &key);

        assert!(
            OrderHandlerClient::new(&w.env, &w.ord_handler)
                .get_order(&key)
                .is_none(),
            "order must be consumed after favorable execution"
        );
        assert!(
            soroban_sdk::token::Client::new(&w.env, &w.long_tk).balance(&user) > 0,
            "user should receive long_tk when limit swap executes at favorable price"
        );
    }

    /// LimitSwap with trigger_price = 0 (no price gate) must always execute.
    #[test]
    fn limit_swap_no_trigger_always_executes() {
        let w = setup();
        let fp = gmx_math::FLOAT_PRECISION;
        seed_pool(&w);

        let user = Address::generate(&w.env);
        StellarAssetClient::new(&w.env, &w.short_tk).mint(&user, &1_000_0000i128);

        set_prices(&w, 2000 * fp);
        let key = create_limit_swap_order(&w, &user, 1_000_0000, &w.short_tk, 0, 0);

        set_prices(&w, 2000 * fp);
        OrderHandlerClient::new(&w.env, &w.ord_handler).execute_order(&w.keeper, &key);

        assert!(
            OrderHandlerClient::new(&w.env, &w.ord_handler)
                .get_order(&key)
                .is_none(),
            "limit swap with trigger_price=0 must always execute"
        );
    }

    /// LimitSwap output below min_output_amount must revert.
    #[test]
    #[should_panic]
    fn limit_swap_min_output_not_met_reverts() {
        let w = setup();
        let fp = gmx_math::FLOAT_PRECISION;
        seed_pool(&w);

        let user = Address::generate(&w.env);
        StellarAssetClient::new(&w.env, &w.short_tk).mint(&user, &1_000_0000i128);

        set_prices(&w, 2000 * fp);
        // min_output_amount = i128::MAX → impossible to satisfy
        let key = create_limit_swap_order(&w, &user, 1_000_0000, &w.short_tk, 0, i128::MAX);

        set_prices(&w, 2000 * fp);
        // Must revert with PriceTooLow because output < min_output_amount
        OrderHandlerClient::new(&w.env, &w.ord_handler).execute_order(&w.keeper, &key);
    }

    /// LimitSwap output meeting min_output_amount must succeed and deliver tokens.
    #[test]
    fn limit_swap_min_output_met_succeeds() {
        let w = setup();
        let fp = gmx_math::FLOAT_PRECISION;
        seed_pool(&w);

        let user = Address::generate(&w.env);
        StellarAssetClient::new(&w.env, &w.short_tk).mint(&user, &1_000_0000i128);

        set_prices(&w, 2000 * fp);
        // min_output_amount = 1 → easy to satisfy
        let key = create_limit_swap_order(&w, &user, 1_000_0000, &w.short_tk, 0, 1);

        set_prices(&w, 2000 * fp);
        OrderHandlerClient::new(&w.env, &w.ord_handler).execute_order(&w.keeper, &key);

        let received = soroban_sdk::token::Client::new(&w.env, &w.long_tk).balance(&user);
        assert!(
            received >= 1,
            "user must receive at least min_output_amount of long_tk"
        );
        assert!(
            OrderHandlerClient::new(&w.env, &w.ord_handler)
                .get_order(&key)
                .is_none(),
            "order must be consumed after successful limit swap"
        );
    }

    // ── Issue #59/#60: configured max swap path length enforced ───────────────

    #[test]
    #[should_panic]
    fn swap_path_over_configured_max_reverts() {
        let w = setup();
        let env = &w.env;
        seed_pool(&w);
        set_prices(&w, 2000 * gmx_math::FLOAT_PRECISION);

        DsClient::new(env, &w.ds).set_u128(
            &w.admin,
            &gmx_keys::max_swap_path_length_key(env),
            &1u128,
        );

        let user = Address::generate(env);
        StellarAssetClient::new(env, &w.long_tk).mint(&user, &1_000_0000i128);
        soroban_sdk::token::Client::new(env, &w.long_tk).transfer(
            &user,
            &w.ord_vault,
            &1_000_0000i128,
        );

        let fake_market = Address::generate(env);
        let hc = OrderHandlerClient::new(env, &w.ord_handler);
        let key = hc.create_order(
            &user,
            &CreateOrderParams {
                receiver: user.clone(),
                market: w.market_tk.clone(),
                initial_collateral_token: w.long_tk.clone(),
                swap_path: Vec::from_array(env, [w.market_tk.clone(), fake_market]),
                size_delta_usd: 0,
                collateral_delta_amount: 1_000_0000i128,
                trigger_price: 0,
                acceptable_price: 0,
                execution_fee: 0,
                min_output_amount: 0,
                order_type: OrderType::MarketSwap,
                is_long: false,
            },
        );
        hc.execute_order(&w.keeper, &key);
    }

    #[test]
    fn swap_path_at_configured_max_succeeds() {
        let w = setup();
        let env = &w.env;
        seed_pool(&w);
        set_prices(&w, 2000 * gmx_math::FLOAT_PRECISION);

        DsClient::new(env, &w.ds).set_u128(
            &w.admin,
            &gmx_keys::max_swap_path_length_key(env),
            &1u128,
        );

        let user = Address::generate(env);
        StellarAssetClient::new(env, &w.short_tk).mint(&user, &1_000_0000i128);
        soroban_sdk::token::Client::new(env, &w.short_tk).transfer(
            &user,
            &w.ord_vault,
            &1_000_0000i128,
        );

        let hc = OrderHandlerClient::new(env, &w.ord_handler);
        let key = hc.create_order(
            &user,
            &CreateOrderParams {
                receiver: user.clone(),
                market: w.market_tk.clone(),
                initial_collateral_token: w.short_tk.clone(),
                swap_path: Vec::from_array(env, [w.market_tk.clone()]),
                size_delta_usd: 0,
                collateral_delta_amount: 1_000_0000i128,
                trigger_price: 0,
                acceptable_price: 0,
                execution_fee: 0,
                min_output_amount: 0,
                order_type: OrderType::MarketSwap,
                is_long: false,
            },
        );
        hc.execute_order(&w.keeper, &key);
        assert!(
            hc.get_order(&key).is_none(),
            "order consumed after single-hop swap"
        );
        assert!(
            soroban_sdk::token::Client::new(env, &w.long_tk).balance(&user) > 0,
            "user should receive long_tk from short->long swap"
        );
    }

    // ── Issue #58: multi-hop swap invariant tests ─────────────────────────────

    #[test]
    fn two_hop_swap_pool_balances_preserved() {
        let w = setup();
        let env = &w.env;

        let mid_tk = env
            .register_stellar_asset_contract_v2(w.admin.clone())
            .address();

        let market_tk1 = env.register(MarketToken, ());
        MtClient::new(env, &market_tk1).initialize(
            &w.admin,
            &w.rs,
            &7u32,
            &soroban_sdk::String::from_str(env, "Market1"),
            &soroban_sdk::String::from_str(env, "M1"),
        );
        let market_tk2 = env.register(MarketToken, ());
        MtClient::new(env, &market_tk2).initialize(
            &w.admin,
            &w.rs,
            &7u32,
            &soroban_sdk::String::from_str(env, "Market2"),
            &soroban_sdk::String::from_str(env, "M2"),
        );

        let ds_c = DsClient::new(env, &w.ds);
        ds_c.set_address(
            &w.admin,
            &gmx_keys::market_index_token_key(env, &market_tk1),
            &w.index_tk,
        );
        ds_c.set_address(
            &w.admin,
            &gmx_keys::market_long_token_key(env, &market_tk1),
            &w.long_tk,
        );
        ds_c.set_address(
            &w.admin,
            &gmx_keys::market_short_token_key(env, &market_tk1),
            &mid_tk,
        );
        ds_c.set_address(
            &w.admin,
            &gmx_keys::market_index_token_key(env, &market_tk2),
            &w.index_tk,
        );
        ds_c.set_address(
            &w.admin,
            &gmx_keys::market_long_token_key(env, &market_tk2),
            &mid_tk,
        );
        ds_c.set_address(
            &w.admin,
            &gmx_keys::market_short_token_key(env, &market_tk2),
            &w.short_tk,
        );

        let fp = gmx_math::FLOAT_PRECISION;
        OClient::new(env, &w.oracle).set_prices_simple(
            &w.keeper,
            &Vec::from_array(
                env,
                [
                    TokenPrice {
                        token: w.long_tk.clone(),
                        min: 2000 * fp,
                        max: 2000 * fp,
                    },
                    TokenPrice {
                        token: mid_tk.clone(),
                        min: 1000 * fp,
                        max: 1000 * fp,
                    },
                    TokenPrice {
                        token: w.short_tk.clone(),
                        min: fp,
                        max: fp,
                    },
                    TokenPrice {
                        token: w.index_tk.clone(),
                        min: 2000 * fp,
                        max: 2000 * fp,
                    },
                ],
            ),
        );

        let pool1_long: u128 = 10_000_0000;
        let pool1_mid: u128 = 5_000_0000;
        let pool2_mid: u128 = 10_000_0000;
        let pool2_short: u128 = 5_000_0000;

        StellarAssetClient::new(env, &w.long_tk).mint(&market_tk1, &(pool1_long as i128));
        StellarAssetClient::new(env, &mid_tk).mint(&market_tk1, &(pool1_mid as i128));
        StellarAssetClient::new(env, &mid_tk).mint(&market_tk2, &(pool2_mid as i128));
        StellarAssetClient::new(env, &w.short_tk).mint(&market_tk2, &(pool2_short as i128));

        ds_c.set_u128(
            &w.admin,
            &gmx_keys::pool_amount_key(env, &market_tk1, &w.long_tk),
            &pool1_long,
        );
        ds_c.set_u128(
            &w.admin,
            &gmx_keys::pool_amount_key(env, &market_tk1, &mid_tk),
            &pool1_mid,
        );
        ds_c.set_u128(
            &w.admin,
            &gmx_keys::pool_amount_key(env, &market_tk2, &mid_tk),
            &pool2_mid,
        );
        ds_c.set_u128(
            &w.admin,
            &gmx_keys::pool_amount_key(env, &market_tk2, &w.short_tk),
            &pool2_short,
        );

        let amount_in: i128 = 1_000;
        let user = Address::generate(env);
        StellarAssetClient::new(env, &w.long_tk).mint(&user, &amount_in);
        soroban_sdk::token::Client::new(env, &w.long_tk).transfer(&user, &w.ord_vault, &amount_in);

        let hc = OrderHandlerClient::new(env, &w.ord_handler);
        let key = hc.create_order(
            &user,
            &CreateOrderParams {
                receiver: user.clone(),
                market: market_tk1.clone(),
                initial_collateral_token: w.long_tk.clone(),
                swap_path: Vec::from_array(env, [market_tk1.clone(), market_tk2.clone()]),
                size_delta_usd: 0,
                collateral_delta_amount: amount_in,
                trigger_price: 0,
                acceptable_price: 0,
                execution_fee: 0,
                min_output_amount: 0,
                order_type: OrderType::MarketSwap,
                is_long: false,
            },
        );
        hc.execute_order(&w.keeper, &key);

        let tk_client_long = soroban_sdk::token::Client::new(env, &w.long_tk);
        let tk_client_mid = soroban_sdk::token::Client::new(env, &mid_tk);
        let tk_client_short = soroban_sdk::token::Client::new(env, &w.short_tk);

        let ds_pool1_long =
            ds_c.get_u128(&gmx_keys::pool_amount_key(env, &market_tk1, &w.long_tk)) as i128;
        let ds_pool1_mid =
            ds_c.get_u128(&gmx_keys::pool_amount_key(env, &market_tk1, &mid_tk)) as i128;
        let ds_pool2_mid =
            ds_c.get_u128(&gmx_keys::pool_amount_key(env, &market_tk2, &mid_tk)) as i128;
        let ds_pool2_short =
            ds_c.get_u128(&gmx_keys::pool_amount_key(env, &market_tk2, &w.short_tk)) as i128;

        assert_eq!(
            tk_client_long.balance(&market_tk1),
            ds_pool1_long,
            "market_tk1 on-chain long_tk balance must match DataStore pool record"
        );
        assert_eq!(
            tk_client_mid.balance(&market_tk1),
            ds_pool1_mid,
            "market_tk1 on-chain mid_tk balance must match DataStore pool record"
        );
        assert_eq!(
            tk_client_mid.balance(&market_tk2),
            ds_pool2_mid,
            "market_tk2 on-chain mid_tk balance must match DataStore pool record"
        );
        assert_eq!(
            tk_client_short.balance(&market_tk2),
            ds_pool2_short,
            "market_tk2 on-chain short_tk balance must match DataStore pool record"
        );
    }

    #[test]
    fn two_hop_swap_output_reaches_receiver() {
        let w = setup();
        let env = &w.env;

        let mid_tk = env
            .register_stellar_asset_contract_v2(w.admin.clone())
            .address();

        let market_tk1 = env.register(MarketToken, ());
        MtClient::new(env, &market_tk1).initialize(
            &w.admin,
            &w.rs,
            &7u32,
            &soroban_sdk::String::from_str(env, "Market1"),
            &soroban_sdk::String::from_str(env, "M1"),
        );
        let market_tk2 = env.register(MarketToken, ());
        MtClient::new(env, &market_tk2).initialize(
            &w.admin,
            &w.rs,
            &7u32,
            &soroban_sdk::String::from_str(env, "Market2"),
            &soroban_sdk::String::from_str(env, "M2"),
        );

        let ds_c = DsClient::new(env, &w.ds);
        ds_c.set_address(
            &w.admin,
            &gmx_keys::market_index_token_key(env, &market_tk1),
            &w.index_tk,
        );
        ds_c.set_address(
            &w.admin,
            &gmx_keys::market_long_token_key(env, &market_tk1),
            &w.long_tk,
        );
        ds_c.set_address(
            &w.admin,
            &gmx_keys::market_short_token_key(env, &market_tk1),
            &mid_tk,
        );
        ds_c.set_address(
            &w.admin,
            &gmx_keys::market_index_token_key(env, &market_tk2),
            &w.index_tk,
        );
        ds_c.set_address(
            &w.admin,
            &gmx_keys::market_long_token_key(env, &market_tk2),
            &mid_tk,
        );
        ds_c.set_address(
            &w.admin,
            &gmx_keys::market_short_token_key(env, &market_tk2),
            &w.short_tk,
        );

        let fp = gmx_math::FLOAT_PRECISION;
        OClient::new(env, &w.oracle).set_prices_simple(
            &w.keeper,
            &Vec::from_array(
                env,
                [
                    TokenPrice {
                        token: w.long_tk.clone(),
                        min: 2000 * fp,
                        max: 2000 * fp,
                    },
                    TokenPrice {
                        token: mid_tk.clone(),
                        min: 1000 * fp,
                        max: 1000 * fp,
                    },
                    TokenPrice {
                        token: w.short_tk.clone(),
                        min: fp,
                        max: fp,
                    },
                    TokenPrice {
                        token: w.index_tk.clone(),
                        min: 2000 * fp,
                        max: 2000 * fp,
                    },
                ],
            ),
        );

        StellarAssetClient::new(env, &w.long_tk).mint(&market_tk1, &10_000_0000i128);
        StellarAssetClient::new(env, &mid_tk).mint(&market_tk1, &5_000_0000i128);
        StellarAssetClient::new(env, &mid_tk).mint(&market_tk2, &10_000_0000i128);
        StellarAssetClient::new(env, &w.short_tk).mint(&market_tk2, &5_000_0000i128);

        ds_c.set_u128(
            &w.admin,
            &gmx_keys::pool_amount_key(env, &market_tk1, &w.long_tk),
            &10_000_0000u128,
        );
        ds_c.set_u128(
            &w.admin,
            &gmx_keys::pool_amount_key(env, &market_tk1, &mid_tk),
            &5_000_0000u128,
        );
        ds_c.set_u128(
            &w.admin,
            &gmx_keys::pool_amount_key(env, &market_tk2, &mid_tk),
            &10_000_0000u128,
        );
        ds_c.set_u128(
            &w.admin,
            &gmx_keys::pool_amount_key(env, &market_tk2, &w.short_tk),
            &5_000_0000u128,
        );

        let amount_in: i128 = 1_000;
        let user = Address::generate(env);
        StellarAssetClient::new(env, &w.long_tk).mint(&user, &amount_in);
        soroban_sdk::token::Client::new(env, &w.long_tk).transfer(&user, &w.ord_vault, &amount_in);

        let hc = OrderHandlerClient::new(env, &w.ord_handler);
        let key = hc.create_order(
            &user,
            &CreateOrderParams {
                receiver: user.clone(),
                market: market_tk1.clone(),
                initial_collateral_token: w.long_tk.clone(),
                swap_path: Vec::from_array(env, [market_tk1.clone(), market_tk2.clone()]),
                size_delta_usd: 0,
                collateral_delta_amount: amount_in,
                trigger_price: 0,
                acceptable_price: 0,
                execution_fee: 0,
                min_output_amount: 0,
                order_type: OrderType::MarketSwap,
                is_long: false,
            },
        );
        hc.execute_order(&w.keeper, &key);

        let short_received = soroban_sdk::token::Client::new(env, &w.short_tk).balance(&user);
        assert!(
            short_received > 0,
            "user must receive short_tk at end of two-hop swap (long_tk -> mid_tk -> short_tk)"
        );
        assert!(
            hc.get_order(&key).is_none(),
            "order consumed after two-hop swap"
        );
    }

    // ── Issue #109: authorization matrix tests ────────────────────────────────

    /// execute_order must reject a caller that does not hold ORDER_KEEPER.
    #[test]
    #[should_panic]
    fn execute_order_by_non_order_keeper_panics() {
        let w = setup();
        let fp = gmx_math::FLOAT_PRECISION;
        set_prices(&w, 2_000 * fp);
        seed_pool(&w);
        set_prices(&w, 2_000 * fp);

        let (hc, key) = create_increase_order(&w, OrderType::MarketIncrease, 0);
        let impostor = Address::generate(&w.env);
        // impostor has no ORDER_KEEPER role — execute_order must panic.
        hc.execute_order(&impostor, &key);
    }

    /// liquidate_position must reject a caller that does not hold LIQUIDATION_KEEPER.
    #[test]
    #[should_panic]
    fn liquidate_position_by_non_liq_keeper_panics() {
        let w = setup();
        let fp = gmx_math::FLOAT_PRECISION;
        set_prices(&w, 2_000 * fp);
        seed_pool(&w);
        set_prices(&w, 2_000 * fp);

        // Open a position first.
        let (hc, key) = create_increase_order(&w, OrderType::MarketIncrease, 0);
        hc.execute_order(&w.keeper, &key);

        // Crash price so position is liquidatable.
        set_prices(&w, 100 * fp);

        let impostor = Address::generate(&w.env);
        // impostor has no LIQUIDATION_KEEPER role — must panic with Unauthorized.
        OrderHandlerClient::new(&w.env, &w.ord_handler).liquidate_position(
            &impostor,
            &w.user,
            &w.market_tk,
            &w.long_tk,
            &true,
        );
    }

    /// execute_adl must reject a caller that does not hold ADL_KEEPER.
    #[test]
    #[should_panic]
    fn execute_adl_by_non_adl_keeper_panics() {
        let w = setup();
        let fp = gmx_math::FLOAT_PRECISION;
        set_prices(&w, 2_000 * fp);
        seed_pool(&w);
        set_prices(&w, 2_000 * fp);

        // Open a position to have something to ADL.
        let (hc, key) = create_increase_order(&w, OrderType::MarketIncrease, 0);
        hc.execute_order(&w.keeper, &key);

        let impostor = Address::generate(&w.env);
        // impostor has no ADL_KEEPER role — must panic with Unauthorized.
        OrderHandlerClient::new(&w.env, &w.ord_handler).execute_adl(
            &impostor,
            &w.user,
            &w.market_tk,
            &w.long_tk,
            &true,
            &0i128,
        );
    }

    // ── Issue #249: keeper heartbeat ──────────────────────────────────────────

    /// Executing an order stamps the ORDER_KEEPER role's last activity at the
    /// current ledger, and the heartbeat reads back as not-stale immediately after.
    #[test]
    fn execute_order_records_keeper_heartbeat() {
        let w = setup();
        let fp = gmx_math::FLOAT_PRECISION;
        set_prices(&w, 2_000 * fp);
        seed_pool(&w);
        set_prices(&w, 2_000 * fp);

        w.env.ledger().set_sequence_number(500);
        let (hc, key) = create_increase_order(&w, OrderType::MarketIncrease, 0);
        hc.execute_order(&w.keeper, &key);

        let status = hc.check_keeper_heartbeat(&w.ds, &roles::order_keeper(&w.env));
        assert_eq!(status.last_active_ledger, 500);
        assert_eq!(status.ledgers_since_last_activity, 0);
        assert!(!status.is_stale, "keeper must be live right after executing");
    }

    /// Full lifecycle: keeper executes, time advances past the timeout, the
    /// heartbeat reports stale, the admin flags it, and the role is revocable
    /// immediately (no timelock).
    #[test]
    fn keeper_goes_stale_after_timeout_and_role_is_revocable() {
        let w = setup();
        let fp = gmx_math::FLOAT_PRECISION;
        set_prices(&w, 2_000 * fp);
        seed_pool(&w);
        set_prices(&w, 2_000 * fp);

        let order_keeper_role = roles::order_keeper(&w.env);
        let hc = OrderHandlerClient::new(&w.env, &w.ord_handler);

        // Keeper executes at ledger 1000 → activity recorded.
        w.env.ledger().set_sequence_number(1000);
        let (_, key) = create_increase_order(&w, OrderType::MarketIncrease, 0);
        hc.execute_order(&w.keeper, &key);
        assert!(!hc
            .check_keeper_heartbeat(&w.ds, &order_keeper_role)
            .is_stale);

        // Advance past the default 2880-ledger timeout.
        w.env.ledger().set_sequence_number(1000 + 2880 + 1);
        let status = hc.check_keeper_heartbeat(&w.ds, &order_keeper_role);
        assert_eq!(status.last_active_ledger, 1000);
        assert!(status.is_stale, "keeper must be stale past the timeout");

        // Admin flags the stale keeper (emits KeeperHeartbeatMissed).
        hc.flag_stale_keeper(&w.admin, &w.keeper, &order_keeper_role);

        // Role is revocable immediately — no timelock on revoke.
        let rs_c = RsClient::new(&w.env, &w.rs);
        assert!(rs_c.has_role(&w.keeper, &order_keeper_role));
        rs_c.revoke_role(&w.admin, &w.keeper, &order_keeper_role);
        assert!(
            !rs_c.has_role(&w.keeper, &order_keeper_role),
            "stale keeper's role must be revocable without waiting"
        );
    }

    /// Admin-configured timeout overrides the default: a shorter window makes a
    /// keeper stale sooner.
    #[test]
    fn custom_heartbeat_timeout_is_respected() {
        let w = setup();
        let fp = gmx_math::FLOAT_PRECISION;
        set_prices(&w, 2_000 * fp);
        seed_pool(&w);
        set_prices(&w, 2_000 * fp);

        let order_keeper_role = roles::order_keeper(&w.env);
        let hc = OrderHandlerClient::new(&w.env, &w.ord_handler);

        // Tighten the timeout to 100 ledgers.
        hc.set_keeper_heartbeat_timeout(&w.admin, &order_keeper_role, &100u64);

        w.env.ledger().set_sequence_number(2000);
        let (_, key) = create_increase_order(&w, OrderType::MarketIncrease, 0);
        hc.execute_order(&w.keeper, &key);

        // 50 ledgers later — within the window.
        w.env.ledger().set_sequence_number(2050);
        assert!(!hc
            .check_keeper_heartbeat(&w.ds, &order_keeper_role)
            .is_stale);

        // 101 ledgers later — past the window.
        w.env.ledger().set_sequence_number(2101);
        assert!(hc
            .check_keeper_heartbeat(&w.ds, &order_keeper_role)
            .is_stale);
    }

    /// `flag_stale_keeper` must revert if the keeper is still within its window.
    #[test]
    #[should_panic]
    fn flag_stale_keeper_reverts_when_keeper_is_live() {
        let w = setup();
        let fp = gmx_math::FLOAT_PRECISION;
        set_prices(&w, 2_000 * fp);
        seed_pool(&w);
        set_prices(&w, 2_000 * fp);

        let order_keeper_role = roles::order_keeper(&w.env);
        let hc = OrderHandlerClient::new(&w.env, &w.ord_handler);

        w.env.ledger().set_sequence_number(3000);
        let (_, key) = create_increase_order(&w, OrderType::MarketIncrease, 0);
        hc.execute_order(&w.keeper, &key);

        // Still live → flagging must panic with KeeperNotStale.
        hc.flag_stale_keeper(&w.admin, &w.keeper, &order_keeper_role);
    }

    /// Only the admin may flag a stale keeper.
    #[test]
    #[should_panic]
    fn flag_stale_keeper_by_non_admin_panics() {
        let w = setup();
        let order_keeper_role = roles::order_keeper(&w.env);
        let hc = OrderHandlerClient::new(&w.env, &w.ord_handler);
        // Never recorded activity → stale, but caller is not admin.
        w.env.ledger().set_sequence_number(5000);
        let impostor = Address::generate(&w.env);
        hc.flag_stale_keeper(&impostor, &w.keeper, &order_keeper_role);
    }

    // ── Issue #219: create_orders (batch) ────────────────────────────────────

    /// Batch with more than 5 orders must revert.
    #[test]
    #[should_panic]
    fn create_orders_more_than_5_reverts() {
        let w = setup();
        let hc = OrderHandlerClient::new(&w.env, &w.ord_handler);
        let mut requests = Vec::new(&w.env);
        // Build 6 decrease orders (no collateral deposit needed)
        for _ in 0..6 {
            requests.push_back(CreateOrderParams {
                receiver: w.user.clone(),
                market: w.market_tk.clone(),
                initial_collateral_token: w.long_tk.clone(),
                swap_path: Vec::new(&w.env),
                size_delta_usd: 100 * gmx_math::FLOAT_PRECISION,
                collateral_delta_amount: 0,
                trigger_price: 0,
                acceptable_price: 0,
                execution_fee: 0,
                min_output_amount: 0,
                order_type: OrderType::MarketDecrease,
                is_long: true,
            });
        }
        hc.create_orders(&w.user, &requests);
    }

    /// Bracket order: 1 MarketIncrease (entry) + 1 StopLossDecrease (SL) + 1 LimitDecrease (TP)
    /// created atomically. Only the increase order requires a vault deposit.
    #[test]
    fn create_orders_bracket_order_created_atomically() {
        let w = setup();
        let fp = gmx_math::FLOAT_PRECISION;

        // Pre-fund the vault for the increase leg
        StellarAssetClient::new(&w.env, &w.long_tk).mint(&w.ord_vault, &COLLATERAL);

        let hc = OrderHandlerClient::new(&w.env, &w.ord_handler);
        let ds_c = DsClient::new(&w.env, &w.ds);

        let requests = Vec::from_array(
            &w.env,
            [
                // Entry: MarketIncrease
                CreateOrderParams {
                    receiver: w.user.clone(),
                    market: w.market_tk.clone(),
                    initial_collateral_token: w.long_tk.clone(),
                    swap_path: Vec::new(&w.env),
                    size_delta_usd: 2000 * fp,
                    collateral_delta_amount: COLLATERAL,
                    trigger_price: 0,
                    acceptable_price: 0,
                    execution_fee: 0,
                    min_output_amount: 0,
                    order_type: OrderType::MarketIncrease,
                    is_long: true,
                },
                // Stop-loss: StopLossDecrease
                CreateOrderParams {
                    receiver: w.user.clone(),
                    market: w.market_tk.clone(),
                    initial_collateral_token: w.long_tk.clone(),
                    swap_path: Vec::new(&w.env),
                    size_delta_usd: 2000 * fp,
                    collateral_delta_amount: 0,
                    trigger_price: 1800 * fp,
                    acceptable_price: 0,
                    execution_fee: 0,
                    min_output_amount: 0,
                    order_type: OrderType::StopLossDecrease,
                    is_long: true,
                },
                // Take-profit: LimitDecrease
                CreateOrderParams {
                    receiver: w.user.clone(),
                    market: w.market_tk.clone(),
                    initial_collateral_token: w.long_tk.clone(),
                    swap_path: Vec::new(&w.env),
                    size_delta_usd: 2000 * fp,
                    collateral_delta_amount: 0,
                    trigger_price: 2500 * fp,
                    acceptable_price: 0,
                    execution_fee: 0,
                    min_output_amount: 0,
                    order_type: OrderType::LimitDecrease,
                    is_long: true,
                },
            ],
        );

        let keys = hc.create_orders(&w.user, &requests);

        // All three orders must be created
        assert_eq!(keys.len(), 3);
        assert!(hc.get_order(&keys.get_unchecked(0)).is_some());
        assert!(hc.get_order(&keys.get_unchecked(1)).is_some());
        assert!(hc.get_order(&keys.get_unchecked(2)).is_some());

        // All three keys must appear in the global order list
        let order_list = gmx_keys::order_list_key(&w.env);
        assert!(ds_c.contains_bytes32(&order_list, &keys.get_unchecked(0)));
        assert!(ds_c.contains_bytes32(&order_list, &keys.get_unchecked(1)));
        assert!(ds_c.contains_bytes32(&order_list, &keys.get_unchecked(2)));

        // Verify order types were stored correctly
        let entry = hc.get_order(&keys.get_unchecked(0)).unwrap();
        let sl = hc.get_order(&keys.get_unchecked(1)).unwrap();
        let tp = hc.get_order(&keys.get_unchecked(2)).unwrap();
        assert!(matches!(entry.order_type, OrderType::MarketIncrease));
        assert!(matches!(sl.order_type, OrderType::StopLossDecrease));
        assert!(matches!(tp.order_type, OrderType::LimitDecrease));
        assert_eq!(sl.trigger_price, 1800 * fp);
        assert_eq!(tp.trigger_price, 2500 * fp);
    }

    // ── Issue #232: cyclic swap_path rejected at create_order time ────────────

    /// [A, A] — immediate repeat must revert with CyclicSwapPath at creation.
    #[test]
    #[should_panic]
    fn create_order_swap_path_aa_reverts() {
        let w = setup();
        let hc = OrderHandlerClient::new(&w.env, &w.ord_handler);
        hc.create_order(
            &w.user,
            &CreateOrderParams {
                receiver: w.user.clone(),
                market: w.market_tk.clone(),
                initial_collateral_token: w.long_tk.clone(),
                swap_path: Vec::from_array(&w.env, [w.market_tk.clone(), w.market_tk.clone()]),
                size_delta_usd: 100 * gmx_math::FLOAT_PRECISION,
                collateral_delta_amount: 0,
                trigger_price: 0,
                acceptable_price: 0,
                execution_fee: 0,
                min_output_amount: 0,
                order_type: OrderType::MarketDecrease,
                is_long: true,
            },
        );
    }

    /// [A, B, A] — indirect repeat must revert with CyclicSwapPath at creation.
    #[test]
    #[should_panic]
    fn create_order_swap_path_aba_reverts() {
        let w = setup();
        let env = &w.env;
        let market_tk2 = env.register(MarketToken, ());
        MtClient::new(env, &market_tk2).initialize(
            &w.admin,
            &w.rs,
            &7u32,
            &soroban_sdk::String::from_str(env, "Market2"),
            &soroban_sdk::String::from_str(env, "M2"),
        );
        let hc = OrderHandlerClient::new(env, &w.ord_handler);
        hc.create_order(
            &w.user,
            &CreateOrderParams {
                receiver: w.user.clone(),
                market: w.market_tk.clone(),
                initial_collateral_token: w.long_tk.clone(),
                swap_path: Vec::from_array(
                    env,
                    [w.market_tk.clone(), market_tk2, w.market_tk.clone()],
                ),
                size_delta_usd: 100 * gmx_math::FLOAT_PRECISION,
                collateral_delta_amount: 0,
                trigger_price: 0,
                acceptable_price: 0,
                execution_fee: 0,
                min_output_amount: 0,
                order_type: OrderType::MarketDecrease,
                is_long: true,
            },
        );
    }

    /// [A, B, C] — three distinct markets must be accepted at creation.
    #[test]
    fn create_order_swap_path_abc_accepted() {
        let w = setup();
        let env = &w.env;
        let market_tk2 = env.register(MarketToken, ());
        MtClient::new(env, &market_tk2).initialize(
            &w.admin,
            &w.rs,
            &7u32,
            &soroban_sdk::String::from_str(env, "Market2"),
            &soroban_sdk::String::from_str(env, "M2"),
        );
        let market_tk3 = env.register(MarketToken, ());
        MtClient::new(env, &market_tk3).initialize(
            &w.admin,
            &w.rs,
            &7u32,
            &soroban_sdk::String::from_str(env, "Market3"),
            &soroban_sdk::String::from_str(env, "M3"),
        );
        // MarketSwap requires vault collateral (is_increase_or_swap = true)
        StellarAssetClient::new(env, &w.long_tk).mint(&w.ord_vault, &COLLATERAL);
        let hc = OrderHandlerClient::new(env, &w.ord_handler);
        let key = hc.create_order(
            &w.user,
            &CreateOrderParams {
                receiver: w.user.clone(),
                market: w.market_tk.clone(),
                initial_collateral_token: w.long_tk.clone(),
                swap_path: Vec::from_array(env, [w.market_tk.clone(), market_tk2, market_tk3]),
                size_delta_usd: 0,
                collateral_delta_amount: COLLATERAL,
                trigger_price: 0,
                acceptable_price: 0,
                execution_fee: 0,
                min_output_amount: 0,
                order_type: OrderType::MarketSwap,
                is_long: false,
            },
        );
        assert!(
            hc.get_order(&key).is_some(),
            "order with distinct 3-market path must be stored"
        );
    }

    // ── Issue #234: attacker cannot pre-occupy victim's position slot ─────────

    /// An attacker who opens their own position cannot occupy the victim's slot.
    /// Position keys encode the actual caller (enforced by require_auth), so
    /// attacker's key and victim's key are always distinct.
    #[test]
    fn third_party_cannot_precreate_victim_position() {
        let w = setup();
        set_prices(&w, 2000 * gmx_math::FLOAT_PRECISION);
        seed_pool(&w);

        let attacker = Address::generate(&w.env);
        let victim = w.user.clone();

        // Attacker funds and opens their own long position
        StellarAssetClient::new(&w.env, &w.long_tk).mint(&w.ord_vault, &COLLATERAL);
        let hc = OrderHandlerClient::new(&w.env, &w.ord_handler);
        let a_key = hc.create_order(
            &attacker,
            &CreateOrderParams {
                receiver: attacker.clone(),
                market: w.market_tk.clone(),
                initial_collateral_token: w.long_tk.clone(),
                swap_path: Vec::new(&w.env),
                size_delta_usd: 2000 * gmx_math::FLOAT_PRECISION,
                collateral_delta_amount: COLLATERAL,
                trigger_price: 0,
                acceptable_price: 0,
                execution_fee: 0,
                min_output_amount: 0,
                order_type: OrderType::MarketIncrease,
                is_long: true,
            },
        );
        set_prices(&w, 2000 * gmx_math::FLOAT_PRECISION);
        hc.execute_order(&w.keeper, &a_key);

        let attacker_pos_key =
            position_key(&w.env, &attacker, &w.market_tk, &w.long_tk, true);
        let victim_pos_key =
            position_key(&w.env, &victim, &w.market_tk, &w.long_tk, true);

        assert_ne!(attacker_pos_key, victim_pos_key, "position keys must differ");
        assert!(
            hc.get_position(&attacker_pos_key).is_some(),
            "attacker has their own position"
        );
        assert!(
            hc.get_position(&victim_pos_key).is_none(),
            "victim slot must be untouched"
        );
    }
}
