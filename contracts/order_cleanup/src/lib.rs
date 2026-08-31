#![no_std]
// Retain the raw events().publish() call sites below rather than migrating
// to #[contractevent] here — that changes on-chain event topic/data encoding,
// which is an ABI-facing behavioural change out of scope for this fix
// (issue #529 is compilation-restoration only).
#![allow(deprecated)]

use gmx_keys::roles;
use gmx_types::{OrderProps, OrderType};
use soroban_sdk::{
    contract, contracterror, contractevent, contractimpl, contracttype, panic_with_error, Address,
    Bytes, BytesN, Env,
};

const DEFAULT_ORDER_EXPIRY: u64 = 14_400;

#[contracttype]
enum InstanceKey {
    Initialized,
    RoleStore,
}

#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
#[repr(u32)]
pub enum Error {
    OrderNotFound = 1,
    NotYetExpired = 2,
    AlreadyInitialized = 3,
    NotInitialized = 4,
    Unauthorized = 5,
}

#[allow(dead_code)]
#[soroban_sdk::contractclient(name = "DataStoreClient")]
trait IDataStore {
    fn get_u128(env: Env, key: BytesN<32>) -> u128;
    fn set_u128(env: Env, caller: Address, key: BytesN<32>, value: u128) -> u128;
}

#[allow(dead_code)]
#[soroban_sdk::contractclient(name = "RoleStoreClient")]
trait IRoleStore {
    fn has_role(env: Env, account: Address, role: BytesN<32>) -> bool;
}

#[allow(dead_code)]
#[soroban_sdk::contractclient(name = "OrderHandlerClient")]
trait IOrderHandler {
    fn get_order(env: Env, key: BytesN<32>) -> Option<OrderProps>;
    fn cleanup_expired_order(env: Env, caller: Address, key: BytesN<32>);
}

#[contractevent(topics = ["ord_exp"])]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ExpiredOrderCancelled {
    pub key: BytesN<32>,
    pub account: Address,
    pub caller: Address,
    pub age: u64,
    pub expiry: u64,
    pub cleanup_fee: i128,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ExpiredOrderPreview {
    pub exists: bool,
    pub is_expired: bool,
    pub age: u64,
    pub expiry: u64,
    pub cleanup_fee: i128,
}

#[contract]
pub struct OrderCleanup;

#[contractimpl]
impl OrderCleanup {
    pub fn initialize(env: Env, admin: Address, role_store: Address) {
        admin.require_auth();
        if env.storage().instance().has(&InstanceKey::Initialized) {
            panic_with_error!(&env, Error::AlreadyInitialized);
        }
        env.storage()
            .instance()
            .set(&InstanceKey::Initialized, &true);
        env.storage()
            .instance()
            .set(&InstanceKey::RoleStore, &role_store);
    }

    pub fn set_order_expiry(
        env: Env,
        data_store: Address,
        caller: Address,
        order_type: OrderType,
        expiry: u64,
    ) {
        caller.require_auth();
        DataStoreClient::new(&env, &data_store).set_u128(
            &caller,
            &order_expiry_key(&env, &order_type),
            &(expiry as u128),
        );
    }

    pub fn cancel_expired_order(
        env: Env,
        data_store: Address,
        order_handler: Address,
        caller: Address,
        key: BytesN<32>,
    ) {
        caller.require_auth();
        let order_client = OrderHandlerClient::new(&env, &order_handler);
        let order = order_client
            .get_order(&key)
            .unwrap_or_else(|| panic_with_error!(&env, Error::OrderNotFound));

        let expiry = expiry_for_order(&env, &data_store, &order.order_type);
        let now = env.ledger().timestamp();
        let age = now.saturating_sub(order.updated_at_time);
        if age < expiry {
            panic_with_error!(&env, Error::NotYetExpired);
        }

        let cleanup_fee = cleanup_fee_from_execution_fee(order.execution_fee);
        // Forward the real external caller (issue #536) so order_handler pays the
        // cleanup incentive to whoever actually did the work, not to this helper
        // contract's own address (which has no way to withdraw it).
        order_client.cleanup_expired_order(&caller, &key);

        env.events().publish_event(&ExpiredOrderCancelled {
            key,
            account: order.account,
            caller,
            age,
            expiry,
            cleanup_fee,
        });
    }

    pub fn record_manual_refund(
        env: Env,
        admin: Address,
        token: Address,
        receiver: Address,
        amount: i128,
        reason: BytesN<32>,
    ) {
        admin.require_auth();

        // Issue #537: require_auth only proves `admin` signed this call — anyone
        // can pass their own address as `admin`. Bind the audit event to an actual
        // protocol admin/controller so it can't be spoofed by an arbitrary caller.
        let role_store: Address = env
            .storage()
            .instance()
            .get(&InstanceKey::RoleStore)
            .unwrap_or_else(|| panic_with_error!(&env, Error::NotInitialized));
        if !RoleStoreClient::new(&env, &role_store).has_role(&admin, &roles::controller(&env)) {
            panic_with_error!(&env, Error::Unauthorized);
        }

        env.events().publish(
            (soroban_sdk::symbol_short!("man_ref"),),
            (admin, token, receiver, amount, reason),
        );
    }

    pub fn preview_expired_order(
        env: Env,
        data_store: Address,
        order_handler: Address,
        key: BytesN<32>,
    ) -> ExpiredOrderPreview {
        let order = OrderHandlerClient::new(&env, &order_handler).get_order(&key);
        if let Some(order) = order {
            let expiry = expiry_for_order(&env, &data_store, &order.order_type);
            let age = env.ledger().timestamp().saturating_sub(order.updated_at_time);
            ExpiredOrderPreview {
                exists: true,
                is_expired: age >= expiry,
                age,
                expiry,
                cleanup_fee: cleanup_fee_from_execution_fee(order.execution_fee),
            }
        } else {
            ExpiredOrderPreview {
                exists: false,
                is_expired: false,
                age: 0,
                expiry: DEFAULT_ORDER_EXPIRY,
                cleanup_fee: 0,
            }
        }
    }
}

fn expiry_for_order(env: &Env, data_store: &Address, order_type: &OrderType) -> u64 {
    let stored = DataStoreClient::new(env, data_store).get_u128(&order_expiry_key(env, order_type));
    if stored == 0 {
        DEFAULT_ORDER_EXPIRY
    } else {
        stored as u64
    }
}

fn cleanup_fee_from_execution_fee(execution_fee: i128) -> i128 {
    if execution_fee <= 0 {
        0
    } else {
        execution_fee / 10
    }
}

fn order_expiry_key(env: &Env, order_type: &OrderType) -> BytesN<32> {
    let mut bytes = Bytes::new(env);
    bytes.append(&Bytes::from_slice(env, b"ORDER_EXPIRY_LEDGERS"));
    bytes.append(&Bytes::from_slice(env, &[order_type_code(order_type)]));
    env.crypto().sha256(&bytes).into()
}

fn order_type_code(order_type: &OrderType) -> u8 {
    match order_type {
        OrderType::MarketSwap => 0,
        OrderType::LimitSwap => 1,
        OrderType::MarketIncrease => 2,
        OrderType::LimitIncrease => 3,
        OrderType::MarketDecrease => 4,
        OrderType::LimitDecrease => 5,
        OrderType::StopLossDecrease => 6,
        OrderType::Liquidation => 7,
        OrderType::StopIncrease => 8,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn expiry_time_validation() {
        use soroban_sdk::testutils::{Address as _, Ledger};
        let env = Env::default();
        env.mock_all_auths();

        let data_store_id = env.register(MockDataStore, ());
        let order_handler_id = env.register(MockOrderHandler, ());
        let cleanup_id = env.register(OrderCleanup, ());
        let client = OrderCleanupClient::new(&env, &cleanup_id);

        let caller = Address::generate(&env);
        let key = BytesN::from_array(&env, &[0; 32]);

        // order.updated_at_time is 100_000 in the mock
        env.ledger().set_timestamp(114_399); // 14,399 seconds later
        let res = client.try_cancel_expired_order(&data_store_id, &order_handler_id, &caller, &key);
        assert_eq!(res.unwrap_err().unwrap(), Error::NotYetExpired);

        env.ledger().set_timestamp(114_400); // 14,400 seconds later
        let res2 = client.try_cancel_expired_order(&data_store_id, &order_handler_id, &caller, &key);
        assert!(res2.is_ok());
    }

    #[contract]
    struct MockDataStore;
    #[contractimpl]
    impl MockDataStore {
        pub fn get_u128(_env: Env, _key: BytesN<32>) -> u128 { 0 }
    }

    #[contract]
    struct MockOrderHandler;
    #[contractimpl]
    impl MockOrderHandler {
        pub fn get_order(env: Env, _key: BytesN<32>) -> Option<OrderProps> {
            Some(OrderProps {
                account: Address::generate(&env),
                receiver: Address::generate(&env),
                market: Address::generate(&env),
                initial_collateral_token: Address::generate(&env),
                swap_path: soroban_sdk::vec![&env, Address::generate(&env)],
                size_delta_usd: 0,
                collateral_delta_amount: 0,
                trigger_price: 0,
                acceptable_price: 0,
                execution_fee: 1000,
                min_output_amount: 0,
                order_type: OrderType::MarketSwap,
                is_long: true,
                updated_at_time: 100_000,
            })
        }
        pub fn cleanup_expired_order(_env: Env, _caller: Address, _key: BytesN<32>) {}
    }

    #[test]
    fn cleanup_fee_is_small_portion_of_execution_fee() {
        assert_eq!(cleanup_fee_from_execution_fee(1_000), 100);
        assert_eq!(cleanup_fee_from_execution_fee(0), 0);
        assert_eq!(cleanup_fee_from_execution_fee(-1), 0);
    }

    #[test]
    fn order_type_codes_are_stable() {
        assert_eq!(order_type_code(&OrderType::MarketSwap), 0);
        assert_eq!(order_type_code(&OrderType::StopIncrease), 8);
    }
}
