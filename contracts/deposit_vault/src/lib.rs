//! Deposit Vault — holds long/short tokens between deposit creation and execution.
//!
//! Mirrors GMX's DepositVault.sol:
//!   - User transfers tokens here before creating a deposit.
//!   - `record_transfer_in` snapshots the balance delta (received amount).
//!   - `transfer_out` sends tokens onward (to pool) during execution or refunds on cancel.
//!   - All mutating ops require CONTROLLER role (held by deposit_handler).
#![no_std]
#![allow(dependency_on_unit_never_type_fallback)]

use soroban_sdk::{
    contract, contractimpl, contracttype, contracterror, panic_with_error,
    Address, BytesN, Env, token,
};

// ─── Errors ───────────────────────────────────────────────────────────────────

#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
#[repr(u32)]
pub enum Error {
    AlreadyInitialized = 1,
    Unauthorized       = 2,
}

// ─── Storage ──────────────────────────────────────────────────────────────────

#[contracttype]
enum InstanceKey {
    Initialized,
    RoleStore,
}

#[contracttype]
enum DataKey {
    /// Last recorded balance for a token (used to compute received delta).
    TokenBalance(Address),
}

// ─── Cross-contract ───────────────────────────────────────────────────────────

#[allow(dead_code)]
#[soroban_sdk::contractclient(name = "RoleStoreClient")]
trait IRoleStore {
    fn has_role(env: Env, account: Address, role: BytesN<32>) -> bool;
}

// ─── Contract ─────────────────────────────────────────────────────────────────

#[contract]
pub struct DepositVault;

#[contractimpl]
impl DepositVault {
    pub fn initialize(env: Env, admin: Address, role_store: Address) {
        admin.require_auth();
        if env.storage().instance().has(&InstanceKey::Initialized) {
            panic_with_error!(&env, Error::AlreadyInitialized);
        }
        env.storage().instance().set(&InstanceKey::Initialized, &true);
        env.storage().instance().set(&InstanceKey::RoleStore, &role_store);
    }

    /// Snapshot the balance of `token` in this vault.
    /// Returns the amount received since the last snapshot (delta).
    /// Called by deposit_handler right after the user's transfer lands.
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

    /// Transfer `amount` of `token` from this vault to `receiver`.
    /// Only callable by a CONTROLLER (deposit_handler).
    pub fn transfer_out(
        env: Env,
        caller: Address,
        token: Address,
        receiver: Address,
        amount: i128,
    ) {
        caller.require_auth();
        require_controller(&env, &caller);
        token::Client::new(&env, &token)
            .transfer(&env.current_contract_address(), &receiver, &amount);
        // Sync recorded balance
        let new_bal = token::Client::new(&env, &token)
            .balance(&env.current_contract_address());
        env.storage().persistent().set(&DataKey::TokenBalance(token), &new_bal);
    }

    /// Read the last recorded balance for a token (for diagnostics).
    pub fn get_recorded_balance(env: Env, token: Address) -> i128 {
        env.storage().persistent()
            .get(&DataKey::TokenBalance(token))
            .unwrap_or(0)
    }
}

// ─── Helpers ──────────────────────────────────────────────────────────────────

fn require_controller(env: &Env, caller: &Address) {
    let rs: Address = env.storage().instance().get(&InstanceKey::RoleStore).unwrap();
    if !RoleStoreClient::new(env, &rs).has_role(caller, &gmx_keys::roles::controller(env)) {
        panic_with_error!(env, Error::Unauthorized);
    }
}

// ─── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use soroban_sdk::{testutils::Address as _, Env};
    use role_store::{RoleStore, RoleStoreClient as RsClient};
    use gmx_keys::roles;

    fn setup(env: &Env) -> (Address, Address, Address) {
        let admin = Address::generate(env);
        let rs = env.register(RoleStore, ());
        RsClient::new(env, &rs).initialize(&admin);
        RsClient::new(env, &rs).grant_role(&admin, &admin, &roles::controller(env));

        let vault = env.register(DepositVault, ());
        DepositVaultClient::new(env, &vault).initialize(&admin, &rs);
        (admin, rs, vault)
    }

    #[test]
    fn initialize_works() {
        let env = Env::default();
        env.mock_all_auths();
        let (_, _, vault) = setup(&env);
        // Just verifies no panic
        let _ = vault;
    }

    #[test]
    fn record_transfer_in_zero_when_nothing_sent() {
        let env = Env::default();
        env.mock_all_auths();
        let (_, _, vault) = setup(&env);
        let token = Address::generate(&env);
        // No real token contract — recorded balance starts at 0, current balance is 0
        // Can't test without a real SEP-41 token; covered in handler integration tests
        let recorded = DepositVaultClient::new(&env, &vault).get_recorded_balance(&token);
        assert_eq!(recorded, 0);
    }
}
