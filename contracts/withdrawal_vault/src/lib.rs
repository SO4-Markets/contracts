//! Withdrawal Vault — holds LP tokens between withdrawal creation and execution.
//!
//! Identical pattern to DepositVault but for the withdrawal flow:
//!   - User transfers LP tokens here before creating a withdrawal.
//!   - withdrawal_handler calls `transfer_out` to burn (or refund) the LP tokens.
#![no_std]
#![allow(dependency_on_unit_never_type_fallback)]

use soroban_sdk::{
    contract, contracterror, contractimpl, contracttype, panic_with_error, token, Address, BytesN,
    Env,
};

#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
#[repr(u32)]
pub enum Error {
    AlreadyInitialized = 1,
    NotInitialized = 2,
    Unauthorized = 3,
    InvalidAmount = 4,
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
        env.storage()
            .instance()
            .set(&InstanceKey::Initialized, &true);
        env.storage()
            .instance()
            .set(&InstanceKey::RoleStore, &role_store);
    }

    pub fn record_transfer_in(env: Env, caller: Address, token: Address) -> i128 {
        caller.require_auth();
        require_controller(&env, &caller);

        let current = token::Client::new(&env, &token).balance(&env.current_contract_address());
        let recorded: i128 = env
            .storage()
            .persistent()
            .get(&DataKey::TokenBalance(token.clone()))
            .unwrap_or(0);
        let delta = current - recorded;
        env.storage()
            .persistent()
            .set(&DataKey::TokenBalance(token), &current);
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

        // Input validation
        if amount <= 0 {
            panic_with_error!(&env, Error::InvalidAmount);
        }

        token::Client::new(&env, &token).transfer(
            &env.current_contract_address(),
            &receiver,
            &amount,
        );
        let new_bal = token::Client::new(&env, &token).balance(&env.current_contract_address());
        env.storage()
            .persistent()
            .set(&DataKey::TokenBalance(token), &new_bal);
    }

    pub fn get_recorded_balance(env: Env, token: Address) -> i128 {
        env.storage()
            .persistent()
            .get(&DataKey::TokenBalance(token))
            .unwrap_or(0)
    }
}

fn require_controller(env: &Env, caller: &Address) {
    let rs: Address = env
        .storage()
        .instance()
        .get(&InstanceKey::RoleStore)
        .unwrap_or_else(|| panic_with_error!(env, Error::NotInitialized));
    if !RoleStoreClient::new(env, &rs).has_role(caller, &gmx_keys::roles::controller(env)) {
        panic_with_error!(env, Error::Unauthorized);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gmx_keys::roles;
    use role_store::{RoleStore, RoleStoreClient as RsClient};
    use soroban_sdk::{testutils::Address as _, Env};
    use test_token::{TestToken, TestTokenClient};

    fn setup(env: &Env) -> (Address, Address, Address) {
        let admin = Address::generate(env);
        let rs = env.register(RoleStore, ());
        RsClient::new(env, &rs).initialize(&admin);
        RsClient::new(env, &rs).grant_role(&admin, &admin, &roles::controller(env));

        let vault = env.register(WithdrawalVault, ());
        WithdrawalVaultClient::new(env, &vault).initialize(&admin, &rs);
        (admin, rs, vault)
    }

    fn register_token(env: &Env) -> (Address, Address) {
        let token = env.register(TestToken, ());
        let owner = Address::generate(env);
        TestTokenClient::new(env, &token).initialize(
            &owner,
            &7u32,
            &soroban_sdk::String::from_str(env, "Test Token"),
            &soroban_sdk::String::from_str(env, "TST"),
        );
        (token, owner)
    }

    #[test]
    fn initialize_works() {
        let env = Env::default();
        env.mock_all_auths();
        let (_, _, vault) = setup(&env);
        let _ = vault;
    }

    #[test]
    fn record_transfer_in_zero_when_nothing_sent() {
        let env = Env::default();
        env.mock_all_auths();
        let (_, _, vault) = setup(&env);
        let token = Address::generate(&env);
        let recorded = WithdrawalVaultClient::new(&env, &vault).get_recorded_balance(&token);
        assert_eq!(recorded, 0);
    }

    #[test]
    fn record_transfer_in_tracks_balance_delta() {
        let env = Env::default();
        env.mock_all_auths();
        let (_, _, vault) = setup(&env);
        let (token, token_owner) = register_token(&env);

        let vault_client = WithdrawalVaultClient::new(&env, &vault);
        let token_client = TestTokenClient::new(&env, &token);

        // Transfer tokens into the vault
        token_client.mint(&token_owner, &vault, &20_000_000_i128);

        // record_transfer_in should return the delta
        let delta = vault_client.record_transfer_in(&token);
        assert_eq!(delta, 20_000_000);

        // get_recorded_balance should now reflect the actual balance
        let recorded = vault_client.get_recorded_balance(&token);
        assert_eq!(recorded, 20_000_000);

        // Second transfer: delta should be only the new amount
        token_client.mint(&token_owner, &vault, &750_0000i128);
        let delta2 = vault_client.record_transfer_in(&token);
        assert_eq!(delta2, 750_0000);

        let recorded2 = vault_client.get_recorded_balance(&token);
        assert_eq!(recorded2, 27_500_000);
    }

    #[test]
    fn transfer_out_by_non_controller_panics() {
        let env = Env::default();
        env.mock_all_auths();
        let (_, _, vault) = setup(&env);
        let (token, token_owner) = register_token(&env);
        let receiver = Address::generate(&env);

        let vault_client = WithdrawalVaultClient::new(&env, &vault);
        let token_client = TestTokenClient::new(&env, &token);
        token_client.mint(&token_owner, &vault, &10_000_000_i128);
        vault_client.record_transfer_in(&token);

        let non_controller = Address::generate(&env);
        let result = vault_client.try_transfer_out(&non_controller, &token, &receiver, &10_000_000);
        assert!(result.is_err());
    }

    #[test]
    fn transfer_out_zero_amount_panics() {
        let env = Env::default();
        env.mock_all_auths();
        let (admin, _, vault) = setup(&env);
        let (token, token_owner) = register_token(&env);
        let receiver = Address::generate(&env);

        let vault_client = WithdrawalVaultClient::new(&env, &vault);
        let token_client = TestTokenClient::new(&env, &token);
        token_client.mint(&token_owner, &vault, &10_000_000_i128);
        vault_client.record_transfer_in(&token);

        let result = vault_client.try_transfer_out(&admin, &token, &receiver, &0);
        assert!(result.is_err());
    }

    #[test]
    fn transfer_out_negative_amount_panics() {
        let env = Env::default();
        env.mock_all_auths();
        let (admin, _, vault) = setup(&env);
        let (token, token_owner) = register_token(&env);
        let receiver = Address::generate(&env);

        let vault_client = WithdrawalVaultClient::new(&env, &vault);
        let token_client = TestTokenClient::new(&env, &token);
        token_client.mint(&token_owner, &vault, &10_000_000_i128);
        vault_client.record_transfer_in(&token);

        let result = vault_client.try_transfer_out(&admin, &token, &receiver, &(-1i128));
        assert!(result.is_err());
    }

    #[test]
    fn transfer_out_controller_can_transfer() {
        let env = Env::default();
        env.mock_all_auths();
        let (admin, _, vault) = setup(&env);
        let (token, token_owner) = register_token(&env);
        let receiver = Address::generate(&env);

        let vault_client = WithdrawalVaultClient::new(&env, &vault);
        let token_client = TestTokenClient::new(&env, &token);
        token_client.mint(&token_owner, &vault, &10_000_000_i128);
        vault_client.record_transfer_in(&token);

        vault_client.transfer_out(&admin, &token, &receiver, &300_0000);

        assert_eq!(token_client.balance(&receiver), 300_0000);
        assert_eq!(token_client.balance(&vault), 700_0000);

        let recorded = vault_client.get_recorded_balance(&token);
        assert_eq!(recorded, 700_0000);
    }
}
