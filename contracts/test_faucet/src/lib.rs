//! Testnet faucet for SO4.market mintable test tokens.
//!
//! Deploy this contract first, then initialize `test_token` instances with this
//! faucet address as owner. Users claim configured amounts with one call.
#![no_std]
#![allow(deprecated)]

use soroban_sdk::{
    contract, contractclient, contracterror, contractimpl, contracttype, panic_with_error,
    symbol_short, Address, Env, Vec,
};

#[allow(dead_code)]
#[contractclient(name = "TestTokenClient")]
trait ITestToken {
    fn mint(env: Env, caller: Address, account: Address, amount: i128);
}

#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
#[repr(u32)]
pub enum Error {
    AlreadyInitialized = 1,
    NotInitialized = 2,
    Unauthorized = 3,
    TokenNotEnabled = 4,
    InvalidAmount = 5,
    ClaimTooSoon = 6,
}

#[contracttype]
enum InstanceKey {
    Admin,
    CooldownLedgers,
}

#[contracttype]
enum DataKey {
    ClaimAmount(Address),
    LastClaim(Address, Address),
}

#[contract]
pub struct TestFaucet;

#[contractimpl]
impl TestFaucet {
    pub fn initialize(env: Env, admin: Address, cooldown_ledgers: u32) {
        if env.storage().instance().has(&InstanceKey::Admin) {
            panic_with_error!(&env, Error::AlreadyInitialized);
        }

        env.storage().instance().set(&InstanceKey::Admin, &admin);
        env.storage()
            .instance()
            .set(&InstanceKey::CooldownLedgers, &cooldown_ledgers);
    }

    pub fn admin(env: Env) -> Address {
        get_admin(&env)
    }

    pub fn cooldown_ledgers(env: Env) -> u32 {
        env.storage()
            .instance()
            .get(&InstanceKey::CooldownLedgers)
            .unwrap_or_else(|| panic_with_error!(&env, Error::NotInitialized))
    }

    pub fn set_cooldown(env: Env, caller: Address, cooldown_ledgers: u32) {
        require_admin(&env, &caller);
        env.storage()
            .instance()
            .set(&InstanceKey::CooldownLedgers, &cooldown_ledgers);
        env.events()
            .publish((symbol_short!("cooldown"),), cooldown_ledgers);
    }

    pub fn set_token(env: Env, caller: Address, token: Address, claim_amount: i128) {
        require_admin(&env, &caller);
        if claim_amount <= 0 {
            panic_with_error!(&env, Error::InvalidAmount);
        }

        env.storage()
            .persistent()
            .set(&DataKey::ClaimAmount(token.clone()), &claim_amount);
        env.events()
            .publish((symbol_short!("token"),), (token, claim_amount));
    }

    pub fn remove_token(env: Env, caller: Address, token: Address) {
        require_admin(&env, &caller);
        env.storage()
            .persistent()
            .remove(&DataKey::ClaimAmount(token.clone()));
        env.events().publish((symbol_short!("rm_token"),), token);
    }

    pub fn claim_amount(env: Env, token: Address) -> i128 {
        env.storage()
            .persistent()
            .get(&DataKey::ClaimAmount(token))
            .unwrap_or(0)
    }

    pub fn last_claim_ledger(env: Env, account: Address, token: Address) -> u32 {
        env.storage()
            .persistent()
            .get(&DataKey::LastClaim(account, token))
            .unwrap_or(0)
    }

    pub fn claim(env: Env, account: Address, token: Address) -> i128 {
        account.require_auth();
        let amount = Self::claim_amount(env.clone(), token.clone());
        if amount <= 0 {
            panic_with_error!(&env, Error::TokenNotEnabled);
        }

        enforce_cooldown(&env, &account, &token);

        let faucet = env.current_contract_address();
        TestTokenClient::new(&env, &token).mint(&faucet, &account, &amount);

        env.storage().persistent().set(
            &DataKey::LastClaim(account.clone(), token.clone()),
            &env.ledger().sequence(),
        );
        env.events()
            .publish((symbol_short!("claim"),), (account, token, amount));
        amount
    }

    pub fn claim_many(env: Env, account: Address, tokens: Vec<Address>) -> Vec<i128> {
        let mut amounts = Vec::new(&env);
        for token in tokens.iter() {
            amounts.push_back(Self::claim(env.clone(), account.clone(), token));
        }
        amounts
    }
}

fn get_admin(env: &Env) -> Address {
    env.storage()
        .instance()
        .get(&InstanceKey::Admin)
        .unwrap_or_else(|| panic_with_error!(env, Error::NotInitialized))
}

fn require_admin(env: &Env, caller: &Address) {
    caller.require_auth();
    if caller != &get_admin(env) {
        panic_with_error!(env, Error::Unauthorized);
    }
}

fn enforce_cooldown(env: &Env, account: &Address, token: &Address) {
    let cooldown: u32 = env
        .storage()
        .instance()
        .get(&InstanceKey::CooldownLedgers)
        .unwrap_or_else(|| panic_with_error!(env, Error::NotInitialized));

    if cooldown == 0 {
        return;
    }

    let last: u32 = env
        .storage()
        .persistent()
        .get(&DataKey::LastClaim(account.clone(), token.clone()))
        .unwrap_or(0);
    if last != 0 && env.ledger().sequence() < last.saturating_add(cooldown) {
        panic_with_error!(env, Error::ClaimTooSoon);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use soroban_sdk::{
        testutils::{Address as _, Ledger},
        String,
    };
    use test_token::{TestToken, TestTokenClient as TokenClient};

    fn setup() -> (Env, Address, Address, TestFaucetClient<'static>) {
        let env = Env::default();
        env.mock_all_auths();
        env.ledger().set_sequence_number(1);

        let admin = Address::generate(&env);
        let faucet_id = env.register(TestFaucet, ());
        let faucet = TestFaucetClient::new(&env, &faucet_id);
        faucet.initialize(&admin, &10);

        let token_id = env.register(TestToken, ());
        let token = TokenClient::new(&env, &token_id);
        token.initialize(
            &faucet_id,
            &7,
            &String::from_str(&env, "Test USD Coin"),
            &String::from_str(&env, "TUSDC"),
        );
        faucet.set_token(&admin, &token_id, &100_0000000);

        (env, admin, token_id, faucet)
    }

    #[test]
    fn user_can_claim_enabled_token() {
        let (env, _admin, token_id, faucet) = setup();
        let user = Address::generate(&env);
        let token = TokenClient::new(&env, &token_id);

        assert_eq!(faucet.claim(&user, &token_id), 100_0000000);
        assert_eq!(token.balance(&user), 100_0000000);
    }

    #[test]
    fn cooldown_blocks_repeat_claim() {
        let (env, _admin, token_id, faucet) = setup();
        let user = Address::generate(&env);

        faucet.claim(&user, &token_id);
        assert!(faucet.try_claim(&user, &token_id).is_err());

        env.ledger().set_sequence_number(11);
        faucet.claim(&user, &token_id);
    }

    #[test]
    #[should_panic]
    fn admin_must_configure_token() {
        let env = Env::default();
        env.mock_all_auths();
        let admin = Address::generate(&env);
        let user = Address::generate(&env);
        let faucet_id = env.register(TestFaucet, ());
        let faucet = TestFaucetClient::new(&env, &faucet_id);
        faucet.initialize(&admin, &0);

        faucet.claim(&user, &Address::generate(&env));
    }
}
