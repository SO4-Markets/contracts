//! Order vault — holds collateral and LP tokens during order lifecycle.
//! Mirrors GMX's OrderVault pattern (same balance-snapshot pattern as deposit/withdrawal vaults).
//!
//! Collateral for market/limit increase orders and LP tokens for decrease orders
//! are held here between create_order and execute_order.
#![no_std]
#![allow(dependency_on_unit_never_type_fallback)]

use soroban_sdk::{
    contract, contracterror, contractimpl, contracttype, panic_with_error,
    Address, BytesN, Env, token,
};
use gmx_keys::roles;

// ─── Errors ───────────────────────────────────────────────────────────────────

#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
#[repr(u32)]
pub enum Error {
    AlreadyInitialized = 1,
    NotInitialized     = 2,
    Unauthorized       = 3,
    NegativeAmount     = 4,
}

// ─── Storage keys ─────────────────────────────────────────────────────────────

#[contracttype]
enum InstanceKey {
    Initialized,
    RoleStore,
}

#[contracttype]
enum DataKey {
    TokenBalance(Address),
}

// ─── Role-store client ────────────────────────────────────────────────────────

#[allow(dead_code)]
#[soroban_sdk::contractclient(name = "RoleStoreClient")]
trait IRoleStore {
    fn has_role(env: Env, account: Address, role: BytesN<32>) -> bool;
}

fn require_controller(env: &Env, caller: &Address) {
    let rs: Address = env.storage().instance().get(&InstanceKey::RoleStore)
        .unwrap_or_else(|| panic_with_error!(env, Error::NotInitialized));
    if !RoleStoreClient::new(env, &rs).has_role(caller, &roles::controller(env)) {
        panic_with_error!(env, Error::Unauthorized);
    }
}

// ─── Contract ─────────────────────────────────────────────────────────────────

#[contract]
pub struct OrderVault;

#[contractimpl]
impl OrderVault {
    /// One-time setup: store admin and role_store addresses.
    pub fn initialize(env: Env, admin: Address, role_store: Address) {
        admin.require_auth();
        if env.storage().instance().has(&InstanceKey::Initialized) {
            panic_with_error!(&env, Error::AlreadyInitialized);
        }
        env.storage().instance().set(&InstanceKey::Initialized, &true);
        env.storage().instance().set(&InstanceKey::RoleStore, &role_store);
    }

    /// Snapshot the balance of `token` in this vault and return the received delta.
    ///
    /// # Balance invariant (issue #47)
    ///
    /// The returned delta is `current_on_chain_balance − last_recorded_balance`.
    /// A positive delta means tokens arrived since the last snapshot; the caller
    /// (order_handler.create_order) treats this as the collateral amount and
    /// reverts the transaction if the delta is ≤ 0.
    ///
    /// After every transfer-out the vault re-snapshots in the same call, so the
    /// recorded balance always equals the actual on-chain balance.  This prevents
    /// double-counting: a second `record_transfer_in` before any new deposit
    /// returns 0, which order_handler will reject.
    pub fn record_transfer_in(env: Env, token: Address) -> i128 {
        let current = token::Client::new(&env, &token)
            .balance(&env.current_contract_address());
        let recorded: i128 = env.storage().persistent()
            .get(&DataKey::TokenBalance(token.clone()))
            .unwrap_or(0);
        let delta = current - recorded;
        env.storage().persistent().set(&DataKey::TokenBalance(token), &current);
        delta
    }

    /// Transfer `amount` of `token` out to `receiver`. CONTROLLER-gated.
    pub fn transfer_out(
        env: Env,
        caller: Address,
        token: Address,
        receiver: Address,
        amount: i128,
    ) {
        caller.require_auth();
        if amount <= 0 {
            panic_with_error!(&env, Error::NegativeAmount);
        }
        require_controller(&env, &caller);
        token::Client::new(&env, &token)
            .transfer(&env.current_contract_address(), &receiver, &amount);
        // Sync recorded balance
        let new_bal = token::Client::new(&env, &token)
            .balance(&env.current_contract_address());
        env.storage().persistent().set(&DataKey::TokenBalance(token), &new_bal);
    }

    /// Return the last recorded balance for a token.
    pub fn get_recorded_balance(env: Env, token: Address) -> i128 {
        env.storage().persistent()
            .get(&DataKey::TokenBalance(token))
            .unwrap_or(0)
    }
}
