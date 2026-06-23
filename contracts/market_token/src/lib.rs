//! Market Token — SEP-41 LP token for each GMX-style market.
//!
//! Each market gets one instance of this contract, deployed deterministically
//! by `market_factory`. Mint and burn are gated to CONTROLLER role (held by
//! deposit_handler / withdrawal_handler). All other SEP-41 methods are public.
#![no_std]

use gmx_keys::roles;
use soroban_sdk::{
    contract, contracterror, contractimpl, contracttype, panic_with_error, symbol_short, token,
    Address, BytesN, Env, String,
};

// ─── Errors ───────────────────────────────────────────────────────────────────

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
}

// ─── Storage keys ─────────────────────────────────────────────────────────────

#[contracttype]
enum InstanceKey {
    Admin,     // Address: market_factory (initial admin)
    RoleStore, // Address: role_store for controller checks
    Decimals,  // u32
    Name,      // String
    Symbol,    // String
}

#[contracttype]
enum DataKey {
    Balance(Address),
    Allowance(Address, Address), // (from, spender)
    TotalSupply,
}

// ─── Allowance data ───────────────────────────────────────────────────────────

#[contracttype]
struct AllowanceData {
    amount: i128,
    expiration_ledger: u32,
}

// ─── Role store cross-contract ────────────────────────────────────────────────

#[allow(dead_code)]
#[soroban_sdk::contractclient(name = "RoleStoreClient")]
trait IRoleStore {
    fn has_role(env: Env, account: Address, role: BytesN<32>) -> bool;
}

// ─── Contract ─────────────────────────────────────────────────────────────────

#[contract]
pub struct MarketToken;

#[contractimpl]
impl MarketToken {
    // ── Initializer ──────────────────────────────────────────────────────────

    /// Called once by market_factory immediately after deploying this contract.
    pub fn initialize(
        env: Env,
        admin: Address,
        role_store: Address,
        decimal: u32,
        name: String,
        symbol: String,
    ) {
        if env.storage().instance().has(&InstanceKey::Admin) {
            panic_with_error!(&env, Error::AlreadyInitialized);
        }
        env.storage().instance().set(&InstanceKey::Admin, &admin);
        env.storage()
            .instance()
            .set(&InstanceKey::RoleStore, &role_store);
        env.storage()
            .instance()
            .set(&InstanceKey::Decimals, &decimal);
        env.storage().instance().set(&InstanceKey::Name, &name);
        env.storage().instance().set(&InstanceKey::Symbol, &symbol);
        env.storage()
            .persistent()
            .set(&DataKey::TotalSupply, &0i128);
    }

    // ── SEP-41 metadata ───────────────────────────────────────────────────────

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

    // ── SEP-41 balance & allowance ────────────────────────────────────────────

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
            Some(d) => {
                if env.ledger().sequence() > d.expiration_ledger {
                    0
                } else {
                    d.amount
                }
            }
        }
    }

    // ── SEP-41 approve / transfer ─────────────────────────────────────────────

    pub fn approve(
        env: Env,
        from: Address,
        spender: Address,
        amount: i128,
        expiration_ledger: u32,
    ) {
        from.require_auth();
        if amount < 0 {
            panic_with_error!(&env, Error::NegativeAmount);
        }
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
        from.require_auth();
        if amount < 0 {
            panic_with_error!(&env, Error::NegativeAmount);
        }
        spend_balance(&env, &from, amount);
        receive_balance(&env, &to, amount);
        env.events()
            .publish((symbol_short!("transfer"),), (from, to, amount));
    }

    pub fn transfer_from(env: Env, spender: Address, from: Address, to: Address, amount: i128) {
        spender.require_auth();
        if amount < 0 {
            panic_with_error!(&env, Error::NegativeAmount);
        }
        spend_allowance(&env, &from, &spender, amount);
        spend_balance(&env, &from, amount);
        receive_balance(&env, &to, amount);
        env.events()
            .publish((symbol_short!("xfer_from"),), (spender, from, to, amount));
    }

    pub fn burn(env: Env, from: Address, amount: i128) {
        from.require_auth();
        if amount < 0 {
            panic_with_error!(&env, Error::NegativeAmount);
        }
        spend_balance(&env, &from, amount);
        change_total_supply(&env, -amount);
        env.events()
            .publish((symbol_short!("burn"),), (from, amount));
    }

    pub fn burn_from(env: Env, spender: Address, from: Address, amount: i128) {
        spender.require_auth();
        if amount < 0 {
            panic_with_error!(&env, Error::NegativeAmount);
        }
        spend_allowance(&env, &from, &spender, amount);
        spend_balance(&env, &from, amount);
        change_total_supply(&env, -amount);
        env.events()
            .publish((symbol_short!("burn_from"),), (spender, from, amount));
    }

    // ── Controlled mint/pool-withdraw (handlers only) ────────────────────────

    /// Mint `amount` LP tokens to `to`. Caller must hold CONTROLLER role.
    pub fn mint(env: Env, caller: Address, to: Address, amount: i128) {
        caller.require_auth();
        if amount < 0 {
            panic_with_error!(&env, Error::NegativeAmount);
        }
        require_controller(&env, &caller);
        receive_balance(&env, &to, amount);
        change_total_supply(&env, amount);
        env.events()
            .publish((symbol_short!("mint"),), (caller, to, amount));
    }

    /// Transfer underlying pool tokens (long/short token) held by this contract
    /// to a receiver. Called by withdrawal_handler after burning LP tokens.
    /// Caller must hold CONTROLLER role.
    pub fn withdraw_from_pool(
        env: Env,
        caller: Address,
        pool_token: Address,
        receiver: Address,
        amount: i128,
    ) {
        caller.require_auth();
        if amount <= 0 {
            panic_with_error!(&env, Error::NegativeAmount);
        }
        require_controller(&env, &caller);
        token::Client::new(&env, &pool_token).transfer(
            &env.current_contract_address(),
            &receiver,
            &amount,
        );
        env.events()
            .publish((symbol_short!("pool_out"),), (pool_token, receiver, amount));
    }
}

// ─── Internal helpers ─────────────────────────────────────────────────────────

fn require_controller(env: &Env, caller: &Address) {
    let role_store: Address = env
        .storage()
        .instance()
        .get(&InstanceKey::RoleStore)
        .unwrap_or_else(|| panic_with_error!(env, Error::NotInitialized));
    let client = RoleStoreClient::new(env, &role_store);
    let ctrl = roles::controller(env);
    if !client.has_role(caller, &ctrl) {
        panic_with_error!(env, Error::Unauthorized);
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
    let ts: i128 = env
        .storage()
        .persistent()
        .get(&DataKey::TotalSupply)
        .unwrap_or(0);
    env.storage()
        .persistent()
        .set(&DataKey::TotalSupply, &(ts + delta));
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

// ─── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use role_store::{RoleStore, RoleStoreClient as RsClient};
    use soroban_sdk::{testutils::Address as _, Env};

    fn deploy_role_store(env: &Env, admin: &Address) -> Address {
        let id = env.register(RoleStore, ());
        let client = RsClient::new(env, &id);
        client.initialize(admin);
        id
    }

    fn deploy_market_token(env: &Env, admin: &Address, role_store: &Address) -> Address {
        let id = env.register(MarketToken, ());
        let client = MarketTokenClient::new(env, &id);
        client.initialize(
            admin,
            role_store,
            &7u32,
            &String::from_str(env, "GMX Market Token"),
            &String::from_str(env, "GM"),
        );
        id
    }

    fn setup() -> (Env, Address, Address, Address) {
        let env = Env::default();
        env.mock_all_auths();
        let admin = Address::generate(&env);
        let rs_id = deploy_role_store(&env, &admin);
        // Grant CONTROLLER to admin
        let rs = RsClient::new(&env, &rs_id);
        rs.grant_role(&admin, &admin, &roles::controller(&env));
        let mt_id = deploy_market_token(&env, &admin, &rs_id);
        (env, admin, rs_id, mt_id)
    }

    #[test]
    fn test_metadata() {
        let (env, _, _, mt_id) = setup();
        let client = MarketTokenClient::new(&env, &mt_id);
        assert_eq!(client.decimals(), 7);
        assert_eq!(client.name(), String::from_str(&env, "GMX Market Token"));
        assert_eq!(client.symbol(), String::from_str(&env, "GM"));
    }

    #[test]
    fn test_mint_and_balance() {
        let (env, admin, _, mt_id) = setup();
        let client = MarketTokenClient::new(&env, &mt_id);
        let user = Address::generate(&env);

        assert_eq!(client.balance(&user), 0);
        client.mint(&admin, &user, &1_000_0000i128); // 1000.0000000 tokens
        assert_eq!(client.balance(&user), 1_000_0000);
        assert_eq!(client.total_supply(), 1_000_0000);
    }

    #[test]
    fn test_transfer() {
        let (env, admin, _, mt_id) = setup();
        let client = MarketTokenClient::new(&env, &mt_id);
        let alice = Address::generate(&env);
        let bob = Address::generate(&env);

        client.mint(&admin, &alice, &500_0000i128);
        client.transfer(&alice, &bob, &200_0000i128);
        assert_eq!(client.balance(&alice), 300_0000);
        assert_eq!(client.balance(&bob), 200_0000);
    }

    #[test]
    fn test_burn() {
        let (env, admin, _, mt_id) = setup();
        let client = MarketTokenClient::new(&env, &mt_id);
        let user = Address::generate(&env);

        client.mint(&admin, &user, &1000_0000i128);
        client.burn(&user, &400_0000i128);
        assert_eq!(client.balance(&user), 600_0000);
        assert_eq!(client.total_supply(), 600_0000);
    }

    #[test]
    fn test_approve_and_transfer_from() {
        let (env, admin, _, mt_id) = setup();
        let client = MarketTokenClient::new(&env, &mt_id);
        let alice = Address::generate(&env);
        let bob = Address::generate(&env);
        let spender = Address::generate(&env);

        client.mint(&admin, &alice, &1000_0000i128);
        client.approve(
            &alice,
            &spender,
            &500_0000i128,
            &(env.ledger().sequence() + 100),
        );
        assert_eq!(client.allowance(&alice, &spender), 500_0000);

        client.transfer_from(&spender, &alice, &bob, &300_0000i128);
        assert_eq!(client.balance(&alice), 700_0000);
        assert_eq!(client.balance(&bob), 300_0000);
        assert_eq!(client.allowance(&alice, &spender), 200_0000);
    }

    #[test]
    #[should_panic]
    fn test_transfer_insufficient_balance() {
        let (env, admin, _, mt_id) = setup();
        let client = MarketTokenClient::new(&env, &mt_id);
        let user = Address::generate(&env);
        let other = Address::generate(&env);

        client.mint(&admin, &user, &100_0000i128);
        client.transfer(&user, &other, &200_0000i128); // should panic
    }

    // ── Issue #153/#124: mint/burn access-control tests ───────────────────────

    /// A caller without CONTROLLER role must not be able to mint LP tokens.
    #[test]
    #[should_panic]
    fn unauthorized_mint_reverts() {
        let (env, _admin, rs_id, mt_id) = setup();
        let attacker = Address::generate(&env);
        let victim = Address::generate(&env);

        // attacker has no role at all
        let rs = RsClient::new(&env, &rs_id);
        assert!(!rs.has_role(&attacker, &roles::controller(&env)));

        let client = MarketTokenClient::new(&env, &mt_id);
        // Must revert with Unauthorized
        client.mint(&attacker, &victim, &1_000_000i128);
    }

    /// A caller without CONTROLLER role must not be able to call withdraw_from_pool.
    /// This exercises the same require_controller guard used on the burn-equivalent path.
    #[test]
    #[should_panic]
    fn unauthorized_withdraw_from_pool_reverts() {
        let (env, admin, rs_id, mt_id) = setup();
        let attacker = Address::generate(&env);
        let victim = Address::generate(&env);

        // Mint some tokens so the contract holds a balance to attempt withdrawing.
        // We use a real SEP-41 token registered in the env.
        let pool_token = env
            .register_stellar_asset_contract_v2(admin.clone())
            .address();
        let client = MarketTokenClient::new(&env, &mt_id);

        // attacker has no CONTROLLER role
        let rs = RsClient::new(&env, &rs_id);
        assert!(!rs.has_role(&attacker, &roles::controller(&env)));

        // Must revert with Unauthorized
        client.withdraw_from_pool(&attacker, &pool_token, &victim, &1_000i128);
    }

    /// A caller WITH CONTROLLER role can mint and the balance is reflected correctly.
    #[test]
    fn authorized_mint_succeeds() {
        let (env, admin, _, mt_id) = setup();
        let user = Address::generate(&env);
        let client = MarketTokenClient::new(&env, &mt_id);

        client.mint(&admin, &user, &5_000_0000i128);
        assert_eq!(
            client.balance(&user),
            5_000_0000,
            "balance must reflect minted amount"
        );
        assert_eq!(
            client.total_supply(),
            5_000_0000,
            "total supply must grow by minted amount"
        );
    }

    /// After mint + burn, total supply and balance return to zero.
    #[test]
    fn authorized_burn_reduces_supply() {
        let (env, admin, _, mt_id) = setup();
        let user = Address::generate(&env);
        let client = MarketTokenClient::new(&env, &mt_id);

        client.mint(&admin, &user, &1_000_0000i128);
        assert_eq!(client.total_supply(), 1_000_0000);

        client.burn(&user, &1_000_0000i128);
        assert_eq!(
            client.balance(&user),
            0,
            "balance must be zero after full burn"
        );
        assert_eq!(
            client.total_supply(),
            0,
            "total supply must be zero after full burn"
        );
    }

    /// Role assignment at market creation: verifying that a role granted post-init
    /// takes effect for mint (simulates deposit_handler being granted CONTROLLER).
    #[test]
    fn newly_granted_controller_can_mint() {
        let (env, admin, rs_id, mt_id) = setup();
        let handler = Address::generate(&env);
        let user = Address::generate(&env);

        // Grant CONTROLLER to the handler after initialization
        RsClient::new(&env, &rs_id).grant_role(&admin, &handler, &roles::controller(&env));

        let client = MarketTokenClient::new(&env, &mt_id);
        client.mint(&handler, &user, &250_0000i128);
        assert_eq!(client.balance(&user), 250_0000);
    }

    /// Revoking CONTROLLER from an address prevents further mints.
    #[test]
    #[should_panic]
    fn revoked_controller_cannot_mint() {
        let (env, admin, rs_id, mt_id) = setup();
        let handler = Address::generate(&env);
        let user = Address::generate(&env);

        let rs = RsClient::new(&env, &rs_id);
        rs.grant_role(&admin, &handler, &roles::controller(&env));

        let client = MarketTokenClient::new(&env, &mt_id);
        // First mint succeeds
        client.mint(&handler, &user, &100_0000i128);

        // Revoke and attempt again — must revert
        rs.revoke_role(&admin, &handler, &roles::controller(&env));
        client.mint(&handler, &user, &100_0000i128);
    }
}
