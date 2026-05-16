//! Withdrawal Vault — holds LP tokens between withdrawal creation and execution.
//!
//! Identical pattern to DepositVault but for the withdrawal flow:
//!   - User transfers LP tokens here before creating a withdrawal.
//!   - withdrawal_handler calls `transfer_out` to burn (or refund) the LP tokens.
#![no_std]
#![allow(dependency_on_unit_never_type_fallback)]

use soroban_sdk::{
    contract, contractimpl, contracttype, contracterror, panic_with_error,
    Address, BytesN, Env, token,
};

#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
#[repr(u32)]
pub enum Error {
    AlreadyInitialized = 1,
    Unauthorized       = 2,
}

#[contracttype]
enum InstanceKey {
    Initialized,
    RoleStore,
}

#[contracttype]
enum DataKey {
    TokenBalance(Address),
}

#[allow(dead_code)]
#[soroban_sdk::contractclient(name = "RoleStoreClient")]
trait IRoleStore {
    fn has_role(env: Env, account: Address, role: BytesN<32>) -> bool;
}

#[contract]
pub struct WithdrawalVault;

#[contractimpl]
impl WithdrawalVault {
    pub fn initialize(env: Env, admin: Address, role_store: Address) {
        admin.require_auth();
        if env.storage().instance().has(&InstanceKey::Initialized) {
            panic_with_error!(&env, Error::AlreadyInitialized);
        }
        env.storage().instance().set(&InstanceKey::Initialized, &true);
        env.storage().instance().set(&InstanceKey::RoleStore, &role_store);
    }

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
        let new_bal = token::Client::new(&env, &token)
            .balance(&env.current_contract_address());
        env.storage().persistent().set(&DataKey::TokenBalance(token), &new_bal);
    }

    pub fn get_recorded_balance(env: Env, token: Address) -> i128 {
        env.storage().persistent()
            .get(&DataKey::TokenBalance(token))
            .unwrap_or(0)
    }
}

fn require_controller(env: &Env, caller: &Address) {
    let rs: Address = env.storage().instance().get(&InstanceKey::RoleStore).unwrap();
    if !RoleStoreClient::new(env, &rs).has_role(caller, &gmx_keys::roles::controller(env)) {
        panic_with_error!(env, Error::Unauthorized);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use soroban_sdk::{testutils::Address as _, Env};
    use role_store::{RoleStore, RoleStoreClient as RsClient};
    use gmx_keys::roles;

    #[test]
    fn initialize_works() {
        let env = Env::default();
        env.mock_all_auths();
        let admin = Address::generate(&env);
        let rs = env.register(RoleStore, ());
        RsClient::new(&env, &rs).initialize(&admin);
        RsClient::new(&env, &rs).grant_role(&admin, &admin, &roles::controller(&env));
        let vault = env.register(WithdrawalVault, ());
        WithdrawalVaultClient::new(&env, &vault).initialize(&admin, &rs);
        let _ = vault;
    }
}
