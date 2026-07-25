//! Mintable test token for SO4.market testnet markets.
//!
//! This is intentionally close to the local `market_token` SEP-41 surface so
//! handlers, vaults, and local tests can use `soroban_sdk::token::Client`.
//! Production collateral should use real Stellar Asset Contracts instead.
#![no_std]
#![allow(deprecated)]

use soroban_sdk::{
    contract, contracterror, contractimpl, contracttype, panic_with_error, symbol_short, Address,
    BytesN, Env, String,
};

/// `network_id` (SHA-256 of the network passphrase) for the Stellar public
/// network. Test tokens must never be initialized here (issue #400).
const MAINNET_NETWORK_ID: [u8; 32] = [
    0x7a, 0xc3, 0x39, 0x97, 0x54, 0x4e, 0x31, 0x75, 0xd2, 0x66, 0xbd, 0x02, 0x24, 0x39, 0xb2, 0x2c,
    0xdb, 0x16, 0x50, 0x8c, 0x01, 0x16, 0x3f, 0x26, 0xe5, 0xcb, 0x2a, 0x3e, 0x10, 0x45, 0xa9, 0x79,
];

#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
#[repr(u32)]
pub enum Error {
    AlreadyInitialized = 1,
    NotInitialized = 2,
    Unauthorized = 3,
    InsufficientBalance = 4,
    InsufficientAllowance = 5,
    NegativeAmount = 6,
    AllowanceExpired = 7,
    Paused = 8,
    MainnetNotAllowed = 9,
}

#[contracttype]
enum InstanceKey {
    Owner,
    Decimals,
    Name,
    Symbol,
    Paused,
}

#[contracttype]
enum DataKey {
    Balance(Address),
    Allowance(Address, Address),
    TotalSupply,
}

#[contracttype]
struct AllowanceData {
    amount: i128,
    expiration_ledger: u32,
}

#[contract]
pub struct TestToken;

#[contractimpl]
impl TestToken {
    pub fn initialize(env: Env, owner: Address, decimal: u32, name: String, symbol: String) {
        require_not_mainnet(&env);
        if env.storage().instance().has(&InstanceKey::Owner) {
            panic_with_error!(&env, Error::AlreadyInitialized);
        }

        env.storage().instance().set(&InstanceKey::Owner, &owner);
        env.storage()
            .instance()
            .set(&InstanceKey::Decimals, &decimal);
        env.storage().instance().set(&InstanceKey::Name, &name);
        env.storage().instance().set(&InstanceKey::Symbol, &symbol);
        env.storage().instance().set(&InstanceKey::Paused, &false);
        env.storage()
            .persistent()
            .set(&DataKey::TotalSupply, &0i128);
    }

    pub fn owner(env: Env) -> Address {
        get_owner(&env)
    }

    pub fn paused(env: Env) -> bool {
        env.storage()
            .instance()
            .get(&InstanceKey::Paused)
            .unwrap_or(false)
    }

    pub fn pause(env: Env, caller: Address) {
        require_owner(&env, &caller);
        env.storage().instance().set(&InstanceKey::Paused, &true);
        env.events().publish((symbol_short!("pause"),), caller);
    }

    pub fn unpause(env: Env, caller: Address) {
        require_owner(&env, &caller);
        env.storage().instance().set(&InstanceKey::Paused, &false);
        env.events().publish((symbol_short!("unpause"),), caller);
    }

    pub fn transfer_owner(env: Env, caller: Address, new_owner: Address) {
        require_owner(&env, &caller);
        env.storage()
            .instance()
            .set(&InstanceKey::Owner, &new_owner);
        env.events()
            .publish((symbol_short!("owner"),), (caller, new_owner));
    }

    pub fn decimals(env: Env) -> u32 {
        env.storage()
            .instance()
            .get(&InstanceKey::Decimals)
            .unwrap_or_else(|| panic_with_error!(&env, Error::NotInitialized))
    }

    pub fn name(env: Env) -> String {
        env.storage()
            .instance()
            .get(&InstanceKey::Name)
            .unwrap_or_else(|| panic_with_error!(&env, Error::NotInitialized))
    }

    pub fn symbol(env: Env) -> String {
        env.storage()
            .instance()
            .get(&InstanceKey::Symbol)
            .unwrap_or_else(|| panic_with_error!(&env, Error::NotInitialized))
    }

    pub fn total_supply(env: Env) -> i128 {
        env.storage()
            .persistent()
            .get(&DataKey::TotalSupply)
            .unwrap_or(0)
    }

    pub fn balance(env: Env, id: Address) -> i128 {
        env.storage()
            .persistent()
            .get(&DataKey::Balance(id))
            .unwrap_or(0)
    }

    pub fn allowance(env: Env, from: Address, spender: Address) -> i128 {
        let data: Option<AllowanceData> = env
            .storage()
            .temporary()
            .get(&DataKey::Allowance(from, spender));

        match data {
            None => 0,
            Some(d) if env.ledger().sequence() > d.expiration_ledger => 0,
            Some(d) => d.amount,
        }
    }

    pub fn approve(
        env: Env,
        from: Address,
        spender: Address,
        amount: i128,
        expiration_ledger: u32,
    ) {
        when_not_paused(&env);
        from.require_auth();
        require_non_negative(&env, amount);

        let key = DataKey::Allowance(from.clone(), spender.clone());
        if amount == 0 {
            env.storage().temporary().remove(&key);
        } else {
            let ledger_gap = expiration_ledger.saturating_sub(env.ledger().sequence());
            env.storage().temporary().set(
                &key,
                &AllowanceData {
                    amount,
                    expiration_ledger,
                },
            );
            env.storage()
                .temporary()
                .extend_ttl(&key, ledger_gap, ledger_gap);
        }

        env.events().publish(
            (symbol_short!("approve"),),
            (from, spender, amount, expiration_ledger),
        );
    }

    pub fn transfer(env: Env, from: Address, to: Address, amount: i128) {
        when_not_paused(&env);
        from.require_auth();
        require_non_negative(&env, amount);

        spend_balance(&env, &from, amount);
        receive_balance(&env, &to, amount);
        env.events()
            .publish((symbol_short!("transfer"),), (from, to, amount));
    }

    pub fn transfer_from(env: Env, spender: Address, from: Address, to: Address, amount: i128) {
        when_not_paused(&env);
        spender.require_auth();
        require_non_negative(&env, amount);

        spend_allowance(&env, &from, &spender, amount);
        spend_balance(&env, &from, amount);
        receive_balance(&env, &to, amount);
        env.events()
            .publish((symbol_short!("xfer_from"),), (spender, from, to, amount));
    }

    pub fn mint(env: Env, caller: Address, account: Address, amount: i128) {
        when_not_paused(&env);
        require_owner(&env, &caller);
        require_non_negative(&env, amount);

        receive_balance(&env, &account, amount);
        change_total_supply(&env, amount);
        env.events()
            .publish((symbol_short!("mint"),), (caller, account, amount));
    }

    pub fn burn(env: Env, from: Address, amount: i128) {
        when_not_paused(&env);
        from.require_auth();
        require_non_negative(&env, amount);

        spend_balance(&env, &from, amount);
        change_total_supply(&env, -amount);
        env.events()
            .publish((symbol_short!("burn"),), (from, amount));
    }

    pub fn burn_from(env: Env, spender: Address, from: Address, amount: i128) {
        when_not_paused(&env);
        spender.require_auth();
        require_non_negative(&env, amount);

        spend_allowance(&env, &from, &spender, amount);
        spend_balance(&env, &from, amount);
        change_total_supply(&env, -amount);
        env.events()
            .publish((symbol_short!("burn_from"),), (spender, from, amount));
    }
}

fn require_not_mainnet(env: &Env) {
    if env.ledger().network_id() == BytesN::from_array(env, &MAINNET_NETWORK_ID) {
        panic_with_error!(env, Error::MainnetNotAllowed);
    }
}

fn get_owner(env: &Env) -> Address {
    env.storage()
        .instance()
        .get(&InstanceKey::Owner)
        .unwrap_or_else(|| panic_with_error!(env, Error::NotInitialized))
}

fn require_owner(env: &Env, caller: &Address) {
    caller.require_auth();
    if caller != &get_owner(env) {
        panic_with_error!(env, Error::Unauthorized);
    }
}

fn when_not_paused(env: &Env) {
    if env
        .storage()
        .instance()
        .get(&InstanceKey::Paused)
        .unwrap_or(false)
    {
        panic_with_error!(env, Error::Paused);
    }
}

fn require_non_negative(env: &Env, amount: i128) {
    if amount < 0 {
        panic_with_error!(env, Error::NegativeAmount);
    }
}

fn spend_balance(env: &Env, from: &Address, amount: i128) {
    let balance: i128 = env
        .storage()
        .persistent()
        .get(&DataKey::Balance(from.clone()))
        .unwrap_or(0);
    if balance < amount {
        panic_with_error!(env, Error::InsufficientBalance);
    }
    env.storage()
        .persistent()
        .set(&DataKey::Balance(from.clone()), &(balance - amount));
}

fn receive_balance(env: &Env, to: &Address, amount: i128) {
    let balance: i128 = env
        .storage()
        .persistent()
        .get(&DataKey::Balance(to.clone()))
        .unwrap_or(0);
    env.storage()
        .persistent()
        .set(&DataKey::Balance(to.clone()), &(balance + amount));
}

fn change_total_supply(env: &Env, delta: i128) {
    let supply: i128 = env
        .storage()
        .persistent()
        .get(&DataKey::TotalSupply)
        .unwrap_or(0);
    env.storage()
        .persistent()
        .set(&DataKey::TotalSupply, &(supply + delta));
}

fn spend_allowance(env: &Env, from: &Address, spender: &Address, amount: i128) {
    let key = DataKey::Allowance(from.clone(), spender.clone());
    let data: AllowanceData = env
        .storage()
        .temporary()
        .get(&key)
        .unwrap_or(AllowanceData {
            amount: 0,
            expiration_ledger: 0,
        });

    if env.ledger().sequence() > data.expiration_ledger {
        panic_with_error!(env, Error::AllowanceExpired);
    }
    if data.amount < amount {
        panic_with_error!(env, Error::InsufficientAllowance);
    }

    let new_amount = data.amount - amount;
    if new_amount == 0 {
        env.storage().temporary().remove(&key);
    } else {
        let ledger_gap = data
            .expiration_ledger
            .saturating_sub(env.ledger().sequence());
        env.storage().temporary().set(
            &key,
            &AllowanceData {
                amount: new_amount,
                expiration_ledger: data.expiration_ledger,
            },
        );
        env.storage()
            .temporary()
            .extend_ttl(&key, ledger_gap, ledger_gap);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use soroban_sdk::{
        testutils::{Address as _, Ledger},
        Env,
    };

    fn setup() -> (Env, Address, TestTokenClient<'static>) {
        let env = Env::default();
        env.mock_all_auths();
        let owner = Address::generate(&env);
        let id = env.register(TestToken, ());
        let client = TestTokenClient::new(&env, &id);
        client.initialize(
            &owner,
            &7,
            &String::from_str(&env, "Test Wrapped Bitcoin"),
            &String::from_str(&env, "TWBTC"),
        );
        (env, owner, client)
    }

    #[test]
    fn owner_can_mint_and_user_can_transfer() {
        let (env, owner, client) = setup();
        let alice = Address::generate(&env);
        let bob = Address::generate(&env);

        client.mint(&owner, &alice, &1_000_0000);
        client.transfer(&alice, &bob, &250_0000);

        assert_eq!(client.balance(&alice), 750_0000);
        assert_eq!(client.balance(&bob), 250_0000);
        assert_eq!(client.total_supply(), 1_000_0000);
    }

    #[test]
    #[should_panic]
    fn non_owner_cannot_mint() {
        let (env, _owner, client) = setup();
        let attacker = Address::generate(&env);
        let alice = Address::generate(&env);

        client.mint(&attacker, &alice, &1);
    }

    #[test]
    fn pause_blocks_transfers_and_minting() {
        let (env, owner, client) = setup();
        let alice = Address::generate(&env);

        client.pause(&owner);
        assert!(client.try_mint(&owner, &alice, &1).is_err());
        assert!(client.paused());

        client.unpause(&owner);
        client.mint(&owner, &alice, &1);
        assert_eq!(client.balance(&alice), 1);
    }

    /// Issue #400: initializing against the mainnet `network_id` must panic —
    /// test tokens must never come up live on mainnet.
    #[test]
    #[should_panic]
    fn initialize_rejects_mainnet_network_id() {
        let env = Env::default();
        env.mock_all_auths();
        env.ledger().set_network_id(MAINNET_NETWORK_ID);
        let owner = Address::generate(&env);
        let id = env.register(TestToken, ());
        TestTokenClient::new(&env, &id).initialize(
            &owner,
            &7,
            &String::from_str(&env, "Test Wrapped Bitcoin"),
            &String::from_str(&env, "TWBTC"),
        );
    }
}
