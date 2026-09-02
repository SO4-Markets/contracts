//! Mintable test token for SO4.market testnet markets.
//!
//! This is intentionally close to the local `market_token` SEP-41 surface so
//! handlers, vaults, and local tests can use `soroban_sdk::token::Client`.
//! Production collateral should use real Stellar Asset Contracts instead.
#![no_std]
#![allow(deprecated)]

use soroban_sdk::{
    contract, contracterror, contractimpl, contracttype, panic_with_error, symbol_short, Address,
    Env, String,
};

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
    /// approve() called with amount > 0 and an expiration_ledger already in
    /// the past (issue #616) — matches the standard SEP-41 token contract's
    /// validation, which panics on this input rather than silently accepting it.
    InvalidExpirationLedger = 10,
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
        gmx_keys::require_not_mainnet(&env, Error::MainnetNotAllowed as u32);
        // Issue #612: require the owner's own auth so a front-runner who
        // observes the deploy transaction (or predicts the deterministic
        // contract address) can't call initialize first with themselves as
        // owner, matching every other initialize in the workspace.
        owner.require_auth();
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
            // Issue #616: reject an already-past expiration_ledger, matching
            // the standard SEP-41 token contract's validation. Without this,
            // ledger_gap silently saturates to 0 and the call succeeds while
            // storing an allowance that is already expired.
            if expiration_ledger < env.ledger().sequence() {
                panic_with_error!(&env, Error::InvalidExpirationLedger);
            }
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

        client.mint(&owner, &alice, &10_000_000);
        client.transfer(&alice, &bob, &250_0000);

        assert_eq!(client.balance(&alice), 750_0000);
        assert_eq!(client.balance(&bob), 250_0000);
        assert_eq!(client.total_supply(), 10_000_000);
    }

    /// Issue #612: initialize must require the owner's own auth, not just
    /// guard against re-initialization — otherwise a front-runner who
    /// observes the deploy transaction could call initialize first with
    /// themselves as owner.
    #[test]
    #[should_panic]
    fn initialize_requires_owner_auth() {
        let env = Env::default();
        // No mock_all_auths() — any require_auth() call must panic.
        let owner = Address::generate(&env);
        let id = env.register(TestToken, ());
        TestTokenClient::new(&env, &id).initialize(
            &owner,
            &7,
            &String::from_str(&env, "Test Wrapped Bitcoin"),
            &String::from_str(&env, "TWBTC"),
        );
    }

    /// Issue #616: approve() with amount > 0 and an already-past
    /// expiration_ledger must revert, matching standard SEP-41 behavior,
    /// instead of silently storing an already-expired allowance.
    #[test]
    #[should_panic]
    fn approve_rejects_past_expiration_ledger() {
        let (env, owner, client) = setup();
        let alice = Address::generate(&env);
        let spender = Address::generate(&env);

        client.mint(&owner, &alice, &10_000_000);
        env.ledger().with_mut(|li| li.sequence_number = 2);
        client.approve(&alice, &spender, &500_0000, &1u32);
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
        env.ledger().set_network_id(gmx_keys::MAINNET_NETWORK_ID);
        let owner = Address::generate(&env);
        let id = env.register(TestToken, ());
        TestTokenClient::new(&env, &id).initialize(
            &owner,
            &7,
            &String::from_str(&env, "Test Wrapped Bitcoin"),
            &String::from_str(&env, "TWBTC"),
        );
    }

    // ── Ported from market_token: allowance-expiration coverage (issue #362) ──

    /// Once the ledger sequence passes an approval's expiration_ledger,
    /// allowance() must report 0 even though the underlying temporary entry
    /// (if not yet TTL-evicted) still holds the original amount.
    #[test]
    fn allowance_reads_zero_after_expiration_ledger_passes() {
        let (env, owner, client) = setup();
        let alice = Address::generate(&env);
        let spender = Address::generate(&env);

        client.mint(&owner, &alice, &1000_0000);
        let expiration = env.ledger().sequence() + 100;
        client.approve(&alice, &spender, &500_0000, &expiration);
        assert_eq!(client.allowance(&alice, &spender), 500_0000);

        env.ledger().set_sequence_number(expiration + 1);
        assert_eq!(
            client.allowance(&alice, &spender),
            0,
            "allowance() must return 0 once expiration_ledger has passed"
        );
    }

    #[test]
    fn test_approve_and_transfer_from() {
        let (env, owner, client) = setup();
        let alice = Address::generate(&env);
        let bob = Address::generate(&env);
        let spender = Address::generate(&env);

        client.mint(&owner, &alice, &1000_0000);
        client.approve(
            &alice,
            &spender,
            &500_0000,
            &(env.ledger().sequence() + 100),
        );
        assert_eq!(client.allowance(&alice, &spender), 500_0000);

        client.transfer_from(&spender, &alice, &bob, &300_0000);
        assert_eq!(client.balance(&alice), 700_0000);
        assert_eq!(client.balance(&bob), 300_0000);
        assert_eq!(client.allowance(&alice, &spender), 200_0000);
    }

    /// transfer_from on an expired allowance must revert with AllowanceExpired.
    #[test]
    fn transfer_from_after_expiration_reverts_with_allowance_expired() {
        let (env, owner, client) = setup();
        let alice = Address::generate(&env);
        let bob = Address::generate(&env);
        let spender = Address::generate(&env);

        client.mint(&owner, &alice, &1000_0000);
        let expiration = env.ledger().sequence() + 100;
        client.approve(&alice, &spender, &500_0000, &expiration);

        env.ledger().set_sequence_number(expiration + 1);

        let result = client.try_transfer_from(&spender, &alice, &bob, &1_0000);
        assert_eq!(
            result,
            Err(Ok(soroban_sdk::Error::from_contract_error(
                Error::AllowanceExpired as u32
            )))
        );
    }

    #[test]
    fn test_approve_and_burn_from() {
        let (env, owner, client) = setup();
        let alice = Address::generate(&env);
        let spender = Address::generate(&env);

        client.mint(&owner, &alice, &1000_0000);
        assert_eq!(client.total_supply(), 1000_0000);

        client.approve(
            &alice,
            &spender,
            &500_0000,
            &(env.ledger().sequence() + 100),
        );
        assert_eq!(client.allowance(&alice, &spender), 500_0000);

        client.burn_from(&spender, &alice, &300_0000);
        assert_eq!(client.balance(&alice), 700_0000);
        assert_eq!(client.allowance(&alice, &spender), 200_0000);
        assert_eq!(client.total_supply(), 700_0000);
    }

    /// burn_from on an expired allowance must revert with AllowanceExpired.
    #[test]
    fn burn_from_after_expiration_reverts_with_allowance_expired() {
        let (env, owner, client) = setup();
        let alice = Address::generate(&env);
        let spender = Address::generate(&env);

        client.mint(&owner, &alice, &1000_0000);
        let expiration = env.ledger().sequence() + 100;
        client.approve(&alice, &spender, &500_0000, &expiration);

        env.ledger().set_sequence_number(expiration + 1);

        let result = client.try_burn_from(&spender, &alice, &1_0000);
        assert_eq!(
            result,
            Err(Ok(soroban_sdk::Error::from_contract_error(
                Error::AllowanceExpired as u32
            )))
        );
    }

    // ── transfer_owner and pause/unpause owner gates ────────────────────────

    #[test]
    fn transfer_owner_moves_ownership() {
        let (env, owner, client) = setup();
        let new_owner = Address::generate(&env);
        let alice = Address::generate(&env);

        client.transfer_owner(&owner, &new_owner);
        assert_eq!(client.owner(), new_owner);

        // old owner can no longer mint
        assert!(client.try_mint(&owner, &alice, &1).is_err());
        // new owner can mint
        client.mint(&new_owner, &alice, &1);
        assert_eq!(client.balance(&alice), 1);
    }

    #[test]
    #[should_panic]
    fn non_owner_cannot_transfer_owner() {
        let (env, _owner, client) = setup();
        let attacker = Address::generate(&env);
        let new_owner = Address::generate(&env);
        client.transfer_owner(&attacker, &new_owner);
    }

    #[test]
    #[should_panic]
    fn non_owner_cannot_pause() {
        let (env, _owner, client) = setup();
        let attacker = Address::generate(&env);
        client.pause(&attacker);
    }

    #[test]
    #[should_panic]
    fn non_owner_cannot_unpause() {
        let (env, owner, client) = setup();
        let attacker = Address::generate(&env);
        client.pause(&owner);
        client.unpause(&attacker);
    }

    // ── Negative-amount rejection (require_non_negative guard) ──────────────

    #[test]
    #[should_panic]
    fn transfer_rejects_negative_amount() {
        let (env, owner, client) = setup();
        let alice = Address::generate(&env);
        let bob = Address::generate(&env);
        client.mint(&owner, &alice, &1000);
        client.transfer(&alice, &bob, &-1);
    }

    #[test]
    #[should_panic]
    fn approve_rejects_negative_amount() {
        let (env, _, client) = setup();
        let alice = Address::generate(&env);
        let spender = Address::generate(&env);
        client.approve(&alice, &spender, &-1, &(env.ledger().sequence() + 100));
    }

    #[test]
    #[should_panic]
    fn mint_rejects_negative_amount() {
        let (env, owner, client) = setup();
        let alice = Address::generate(&env);
        client.mint(&owner, &alice, &-1);
    }

    #[test]
    #[should_panic]
    fn burn_rejects_negative_amount() {
        let (env, owner, client) = setup();
        let alice = Address::generate(&env);
        client.mint(&owner, &alice, &1000);
        client.burn(&alice, &-1);
    }

    #[test]
    #[should_panic]
    fn transfer_from_rejects_negative_amount() {
        let (env, owner, client) = setup();
        let alice = Address::generate(&env);
        let bob = Address::generate(&env);
        let spender = Address::generate(&env);
        client.mint(&owner, &alice, &1000);
        client.approve(
            &alice,
            &spender,
            &1000,
            &(env.ledger().sequence() + 100),
        );
        client.transfer_from(&spender, &alice, &bob, &-1);
    }

    #[test]
    #[should_panic]
    fn burn_from_rejects_negative_amount() {
        let (env, owner, client) = setup();
        let alice = Address::generate(&env);
        let spender = Address::generate(&env);
        client.mint(&owner, &alice, &1000);
        client.approve(
            &alice,
            &spender,
            &1000,
            &(env.ledger().sequence() + 100),
        );
        client.burn_from(&spender, &alice, &-1);
    }
}
