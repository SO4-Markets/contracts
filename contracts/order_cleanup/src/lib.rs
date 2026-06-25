#![no_std]

use gmx_types::{OrderProps, OrderType};
use soroban_sdk::{
    contract, contracterror, contractevent, contractimpl, contracttype, panic_with_error, Address,
    Bytes, BytesN, Env,
};

const DEFAULT_ORDER_EXPIRY: u64 = 2_880;

#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
#[repr(u32)]
pub enum Error {
    OrderNotFound = 1,
    NotYetExpired = 2,
}

#[allow(dead_code)]
#[soroban_sdk::contractclient(name = "DataStoreClient")]
trait IDataStore {
    fn get_u128(env: Env, key: BytesN<32>) -> u128;
    fn set_u128(env: Env, caller: Address, key: BytesN<32>, value: u128) -> u128;
}

#[allow(dead_code)]
#[soroban_sdk::contractclient(name = "OrderHandlerClient")]
trait IOrderHandler {
    fn get_order(env: Env, key: BytesN<32>) -> Option<OrderProps>;
    fn cancel_order(env: Env, caller: Address, key: BytesN<32>);
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
        let helper = env.current_contract_address();
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
        order_client.cancel_order(&helper, &key);

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
    fn default_expiry_matches_four_hour_ledger_target() {
        assert_eq!(DEFAULT_ORDER_EXPIRY, 2_880);
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
