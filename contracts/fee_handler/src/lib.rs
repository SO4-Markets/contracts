//! Fee handler — claims and distributes protocol fees accumulated in the pool.
//! Mirrors GMX's FeeHandler.sol.
#![no_std]
#![allow(dependency_on_unit_never_type_fallback)]

use soroban_sdk::{
    contract, contracterror, contractevent, contractimpl, contracttype, panic_with_error,
    Address, BytesN, Env,
};
use gmx_keys::{
    roles,
    claimable_fee_amount_key, claimable_funding_amount_key,
};

// ─── Storage keys ─────────────────────────────────────────────────────────────

#[contracttype]
enum InstanceKey {
    Initialized,
    Admin,
    RoleStore,
    DataStore,
}

// ─── Errors ───────────────────────────────────────────────────────────────────

#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
#[repr(u32)]
pub enum Error {
    AlreadyInitialized = 1,
    Unauthorized       = 3,
    NothingToClaim     = 4,
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
    fn set_u128(env: Env, caller: Address, key: BytesN<32>, value: u128) -> u128;
    fn apply_delta_to_u128(env: Env, caller: Address, key: BytesN<32>, delta: i128) -> u128;
}

#[allow(dead_code)]
#[soroban_sdk::contractclient(name = "MarketTokenClient")]
trait IMarketToken {
    fn withdraw_from_pool(env: Env, caller: Address, pool_token: Address, receiver: Address, amount: i128);
}

// ─── Events ───────────────────────────────────────────────────────────────────

#[contractevent(topics = ["fee_clm"])]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FeeClaimed {
    pub market:   Address,
    pub token:    Address,
    pub amount:   u128,
    pub receiver: Address,
}

#[contractevent(topics = ["fnd_clm"])]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FundingFeeClaimed {
    pub account: Address,
    pub market:  Address,
    pub token:   Address,
    pub amount:  u128,
}

// ─── Contract ─────────────────────────────────────────────────────────────────

#[contract]
pub struct FeeHandler;

#[contractimpl]
impl FeeHandler {
    pub fn initialize(env: Env, admin: Address, role_store: Address, data_store: Address) {
        admin.require_auth();
        if env.storage().instance().has(&InstanceKey::Initialized) {
            panic_with_error!(&env, Error::AlreadyInitialized);
        }
        env.storage().instance().set(&InstanceKey::Initialized, &true);
        env.storage().instance().set(&InstanceKey::Admin, &admin);
        env.storage().instance().set(&InstanceKey::RoleStore, &role_store);
        env.storage().instance().set(&InstanceKey::DataStore, &data_store);
    }

    /// Return the accumulated protocol fee amount for a given market + token.
    pub fn claimable_fees(env: Env, market: Address, token: Address) -> u128 {
        let data_store: Address = env.storage().instance().get(&InstanceKey::DataStore).unwrap();
        let key = claimable_fee_amount_key(&env, &market, &token);
        DataStoreClient::new(&env, &data_store).get_u128(&key)
    }

    /// Sweep accumulated protocol fees for a market/token to `receiver`. FEE_KEEPER only.
    pub fn claim_fees(
        env: Env,
        keeper: Address,
        market: Address,
        token: Address,
        receiver: Address,
    ) -> u128 {
        keeper.require_auth();

        let role_store: Address = env.storage().instance().get(&InstanceKey::RoleStore).unwrap();
        if !RoleStoreClient::new(&env, &role_store).has_role(&keeper, &roles::fee_keeper(&env)) {
            panic_with_error!(&env, Error::Unauthorized);
        }

        let data_store: Address = env.storage().instance().get(&InstanceKey::DataStore).unwrap();
        let ds = DataStoreClient::new(&env, &data_store);
        let handler = env.current_contract_address();

        let key = claimable_fee_amount_key(&env, &market, &token);
        let amount = ds.get_u128(&key);
        if amount == 0 {
            panic_with_error!(&env, Error::NothingToClaim);
        }

        ds.set_u128(&handler, &key, &0u128);

        // Transfer from market_token pool to receiver
        MarketTokenClient::new(&env, &market)
            .withdraw_from_pool(&handler, &token, &receiver, &(amount as i128));

        env.events().publish_event(&FeeClaimed { market, token, amount, receiver });
        amount
    }

    /// Claim funding fees earned by a position account. Anyone can call for their own account.
    pub fn claim_funding_fees(
        env: Env,
        account: Address,
        market: Address,
        token: Address,
    ) -> u128 {
        account.require_auth();

        let data_store: Address = env.storage().instance().get(&InstanceKey::DataStore).unwrap();
        let ds = DataStoreClient::new(&env, &data_store);
        let handler = env.current_contract_address();

        let key = claimable_funding_amount_key(&env, &market, &token, &account);
        let amount = ds.get_u128(&key);
        if amount == 0 {
            return 0;
        }

        ds.set_u128(&handler, &key, &0u128);

        MarketTokenClient::new(&env, &market)
            .withdraw_from_pool(&handler, &token, &account, &(amount as i128));

        env.events().publish_event(&FundingFeeClaimed { account, market, token, amount });
        amount
    }
}
