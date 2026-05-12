//! Order vault — holds collateral and LP tokens during order lifecycle.
//! Mirrors GMX's OrderVault pattern (same balance-snapshot pattern as deposit/withdrawal vaults).
//!
//! Collateral for market/limit increase orders and LP tokens for decrease orders
//! are held here between create_order and execute_order.
#![no_std]
#![allow(dependency_on_unit_never_type_fallback)]

use soroban_sdk::{contract, contracterror, contractimpl, contracttype, Address, Env, symbol_short, token};
// recorded balance is stored under a local key derived from token address

// ─── Storage keys ─────────────────────────────────────────────────────────────

const ADMIN_KEY:      &str = "ADMIN";
const ROLE_STORE_KEY: &str = "ROLE_STORE";

// ─── Errors ───────────────────────────────────────────────────────────────────

#[contracterror]
pub enum Error {
    AlreadyInitialized = 1,
    NotInitialized     = 2,
    Unauthorized       = 3,
    NegativeAmount     = 4,
}

// ─── Role-store client (minimal) ──────────────────────────────────────────────

#[soroban_sdk::contractclient(name = "RoleStoreClient")]
trait IRoleStore {
    fn has_role(env: Env, account: Address, role: soroban_sdk::Symbol) -> bool;
}

fn require_controller(env: &Env, caller: &Address) {
    // TODO: same pattern as deposit_vault — read ROLE_STORE from instance storage,
    //       call role_store_client.has_role(caller, CONTROLLER), panic if false
    todo!()
}

// ─── Contract ─────────────────────────────────────────────────────────────────

#[contract]
pub struct OrderVault;

#[contractimpl]
impl OrderVault {
    /// One-time setup: store admin and role_store addresses.
    pub fn initialize(env: Env, admin: Address, role_store: Address) {
        // TODO: panic if already initialized (ADMIN_KEY exists in instance storage)
        //       env.storage().instance().set(&ADMIN_KEY, &admin)
        //       env.storage().instance().set(&ROLE_STORE_KEY, &role_store)
        todo!()
    }

    /// Record how many tokens of `token` arrived since last snapshot.
    ///
    /// Should be called by the handler immediately after the user's SEP-41 `transfer`
    /// into this vault. Returns the amount received (delta from last recorded balance).
    pub fn record_transfer_in(env: Env, token: Address) -> i128 {
        // TODO: (same pattern as deposit_vault::record_transfer_in)
        //
        // 1. current_balance = token::Client::new(&env, &token)
        //        .balance(&env.current_contract_address())
        //
        // 2. key = recorded_balance_key(&env, &token)
        //    prev_balance = env.storage().persistent().get::<_,i128>(&key).unwrap_or(0)
        //
        // 3. delta = current_balance - prev_balance
        //    if delta < 0 → panic "balance decreased"
        //
        // 4. env.storage().persistent().set(&key, &current_balance)
        //
        // Returns delta (the new tokens received)
        todo!()
    }

    /// Transfer `amount` of `token` out to `receiver`. CONTROLLER-gated.
    pub fn transfer_out(env: Env, caller: Address, token: Address, receiver: Address, amount: i128) {
        // TODO:
        // 1. caller.require_auth()
        // 2. require_controller(&env, &caller)
        // 3. if amount <= 0 → panic "negative amount"
        // 4. token::Client::new(&env, &token).transfer(&env.current_contract_address(), &receiver, &amount)
        // 5. Update recorded balance: subtract amount from stored balance for `token`
        todo!()
    }

    /// Return the last snapshot balance for `token`.
    pub fn get_recorded_balance(env: Env, token: Address) -> i128 {
        // TODO: read recorded_balance_key(&env, &token) from persistent storage, default 0
        todo!()
    }
}
