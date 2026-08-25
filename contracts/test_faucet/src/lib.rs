//! Testnet faucet for SO4.market mintable test tokens.
//!
//! Deploy this contract first, then initialize `test_token` instances with this
//! faucet address as owner. Users claim configured amounts with one call.
#![no_std]
#![allow(deprecated)]

use soroban_sdk::{
    contract, contractclient, contracterror, contractimpl, contracttype, panic_with_error,
    symbol_short, Address, BytesN, Env, Vec,
};

/// `network_id` (SHA-256 of the network passphrase) for the Stellar public
/// network. Test faucets must never be initialized here (issue #400).
const MAINNET_NETWORK_ID: [u8; 32] = [
    0x7a, 0xc3, 0x39, 0x97, 0x54, 0x4e, 0x31, 0x75, 0xd2, 0x66, 0xbd, 0x02, 0x24, 0x39, 0xb2, 0x2c,
    0xdb, 0x16, 0x50, 0x8c, 0x01, 0x16, 0x3f, 0x26, 0xe5, 0xcb, 0x2a, 0x3e, 0x10, 0x45, 0xa9, 0x79,
];

#[allow(dead_code)]
#[contractclient(name = "TestTokenClient")]
trait ITestToken {
    fn mint(env: Env, caller: Address, account: Address, amount: i128);
}

/// Maximum tokens per `claim_many` call (issue #609), matching the batch-cap
/// pattern used elsewhere in the workspace (fee_batch_sweeper::MAX_BATCH_CLAIM_SIZE).
const MAX_CLAIM_BATCH_SIZE: u32 = 20;

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
    MainnetNotAllowed = 7,
    /// claim_many called with more than MAX_CLAIM_BATCH_SIZE tokens (issue #609).
    BatchSizeLimitExceeded = 8,
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
        require_not_mainnet(&env);
        // Issue #612: require the admin's own auth so a front-runner who
        // observes the deploy transaction (or predicts the deterministic
        // contract address) can't call initialize first with themselves as
        // admin, matching every other initialize in the workspace.
        admin.require_auth();
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
        do_claim(&env, &account, token)
    }

    /// Claim multiple tokens in a single transaction.
    ///
    /// Authorizes `account` once for the whole call rather than once per token
    /// (issue #399) — looping `Self::claim` per token previously called
    /// `account.require_auth()` once per iteration within the same invocation,
    /// which hit a Soroban auth-reuse bug and failed for real signed transactions.
    pub fn claim_many(env: Env, account: Address, tokens: Vec<Address>) -> Vec<i128> {
        account.require_auth();
        // Issue #609: cap batch size, matching every other batch entrypoint
        // in the codebase (fee_batch_sweeper::MAX_BATCH_CLAIM_SIZE,
        // order_handler::create_orders). do_claim performs a storage
        // read/write and a cross-contract mint call per entry.
        if tokens.len() > MAX_CLAIM_BATCH_SIZE {
            panic_with_error!(&env, Error::BatchSizeLimitExceeded);
        }
        let mut amounts = Vec::new(&env);
        for token in tokens.iter() {
            amounts.push_back(do_claim(&env, &account, token));
        }
        amounts
    }
}

fn do_claim(env: &Env, account: &Address, token: Address) -> i128 {
    let amount = TestFaucet::claim_amount(env.clone(), token.clone());
    if amount <= 0 {
        panic_with_error!(env, Error::TokenNotEnabled);
    }

    enforce_cooldown(env, account, &token);

    let faucet = env.current_contract_address();
    TestTokenClient::new(env, &token).mint(&faucet, account, &amount);

    env.storage().persistent().set(
        &DataKey::LastClaim(account.clone(), token.clone()),
        &env.ledger().sequence(),
    );
    env.events()
        .publish((symbol_short!("claim"),), (account.clone(), token, amount));
    amount
}

fn require_not_mainnet(env: &Env) {
    if env.ledger().network_id() == BytesN::from_array(env, &MAINNET_NETWORK_ID) {
        panic_with_error!(env, Error::MainnetNotAllowed);
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

    /// Issue #399: `claim_many` must authorize `account` once and mint every
    /// configured token in a single call, matching what `claim` does per-token.
    #[test]
    fn claim_many_mints_every_configured_token() {
        let (env, admin, token_a_id, faucet) = setup();
        let user = Address::generate(&env);

        let token_b_id = env.register(TestToken, ());
        let token_b = TokenClient::new(&env, &token_b_id);
        token_b.initialize(
            &faucet.address,
            &7,
            &String::from_str(&env, "Test Wrapped Bitcoin"),
            &String::from_str(&env, "TWBTC"),
        );
        faucet.set_token(&admin, &token_b_id, &50_0000000);

        let amounts = faucet.claim_many(
            &user,
            &Vec::from_array(&env, [token_a_id.clone(), token_b_id.clone()]),
        );

        assert_eq!(amounts, Vec::from_array(&env, [100_0000000, 50_0000000]));
        assert_eq!(TokenClient::new(&env, &token_a_id).balance(&user), 100_0000000);
        assert_eq!(token_b.balance(&user), 50_0000000);
    }

    /// Issue #609: claim_many must reject a batch larger than
    /// MAX_CLAIM_BATCH_SIZE, matching the cap pattern used elsewhere in the
    /// workspace.
    #[test]
    #[should_panic]
    fn claim_many_over_max_batch_size_reverts() {
        let (env, _admin, token_id, faucet) = setup();
        let user = Address::generate(&env);

        let mut tokens = Vec::new(&env);
        let mut i = 0u32;
        while i < MAX_CLAIM_BATCH_SIZE + 1 {
            tokens.push_back(token_id.clone());
            i += 1;
        }

        faucet.claim_many(&user, &tokens);
    }

    /// Issue #612: initialize must require the admin's own auth, not just
    /// guard against re-initialization — otherwise a front-runner who
    /// observes the deploy transaction could call initialize first with
    /// themselves as admin.
    #[test]
    #[should_panic]
    fn initialize_requires_admin_auth() {
        let env = Env::default();
        // No mock_all_auths() — any require_auth() call must panic.
        let admin = Address::generate(&env);
        let faucet_id = env.register(TestFaucet, ());
        TestFaucetClient::new(&env, &faucet_id).initialize(&admin, &10);
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

    /// Issue #400: initializing against the mainnet `network_id` must panic —
    /// the faucet must never come up live on mainnet.
    #[test]
    #[should_panic]
    fn initialize_rejects_mainnet_network_id() {
        let env = Env::default();
        env.mock_all_auths();
        env.ledger().set_network_id(MAINNET_NETWORK_ID);
        let admin = Address::generate(&env);
        let faucet_id = env.register(TestFaucet, ());
        TestFaucetClient::new(&env, &faucet_id).initialize(&admin, &10);
    }
}
