#![no_std]

use soroban_sdk::{
    contract, contractimpl, contracttype, contracterror, panic_with_error, symbol_short,
    BytesN, Env, Address, Vec,
};
use gmx_keys::roles;

// ─── Errors ───────────────────────────────────────────────────────────────────

#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
#[repr(u32)]
pub enum Error {
    NotInitialized    = 1,
    AlreadyInitialized = 2,
    Unauthorized      = 3,
    LastAdmin         = 4, // can't remove the last ROLE_ADMIN holder
}

// ─── Storage keys ─────────────────────────────────────────────────────────────

#[contracttype]
enum RoleKey {
    /// true if account holds role
    HasRole(Address, BytesN<32>),
    /// Vec<Address> — every holder of a given role
    RoleMembers(BytesN<32>),
    /// Vec<BytesN<32>> — all roles an account currently holds
    AccountRoles(Address),
    /// Vec<BytesN<32>> — all distinct roles ever granted
    AllRoles,
    /// Init flag
    Initialized,
}

// ─── Contract ─────────────────────────────────────────────────────────────────

#[contract]
pub struct RoleStore;

#[contractimpl]
impl RoleStore {
    // ── Initializer ──────────────────────────────────────────────────────────

    /// Deploy-time init: grant ROLE_ADMIN to `admin`. Can only be called once.
    pub fn initialize(env: Env, admin: Address) {
        admin.require_auth();
        if env.storage().instance().has(&RoleKey::Initialized) {
            panic_with_error!(&env, Error::AlreadyInitialized);
        }
        env.storage().instance().set(&RoleKey::Initialized, &true);
        let admin_role = roles::role_admin(&env);
        internal_grant_role(&env, &admin, &admin_role);
        env.events().publish((symbol_short!("init"),), (admin, admin_role));
    }

    // ── Public write ─────────────────────────────────────────────────────────

    /// Grant `role` to `account`. Caller must hold ROLE_ADMIN.
    pub fn grant_role(env: Env, caller: Address, account: Address, role: BytesN<32>) {
        caller.require_auth();
        require_init(&env);
        require_admin(&env, &caller);
        internal_grant_role(&env, &account, &role);
        env.events().publish((symbol_short!("grant"),), (account, role));
    }

    /// Revoke `role` from `account`. Caller must hold ROLE_ADMIN.
    /// Prevents removing the last ROLE_ADMIN holder.
    pub fn revoke_role(env: Env, caller: Address, account: Address, role: BytesN<32>) {
        caller.require_auth();
        require_init(&env);
        require_admin(&env, &caller);

        let admin_role = roles::role_admin(&env);
        if role == admin_role {
            let members: Vec<Address> = env
                .storage()
                .persistent()
                .get(&RoleKey::RoleMembers(admin_role.clone()))
                .unwrap_or(Vec::new(&env));
            if members.len() <= 1 {
                panic_with_error!(&env, Error::LastAdmin);
            }
        }

        internal_revoke_role(&env, &account, &role);
        env.events().publish((symbol_short!("revoke"),), (account, role));
    }

    // ── Public reads ─────────────────────────────────────────────────────────

    /// Returns true if `account` currently holds `role`.
    pub fn has_role(env: Env, account: Address, role: BytesN<32>) -> bool {
        env.storage()
            .persistent()
            .get(&RoleKey::HasRole(account, role))
            .unwrap_or(false)
    }

    /// All roles currently held by `account`.
    pub fn get_roles(env: Env, account: Address) -> Vec<BytesN<32>> {
        env.storage()
            .persistent()
            .get(&RoleKey::AccountRoles(account))
            .unwrap_or(Vec::new(&env))
    }

    /// Paginated list of all accounts that hold `role`.
    pub fn get_role_members(env: Env, role: BytesN<32>, start: u32, end: u32) -> Vec<Address> {
        let members: Vec<Address> = env
            .storage()
            .persistent()
            .get(&RoleKey::RoleMembers(role))
            .unwrap_or(Vec::new(&env));
        paginate_addr(&env, &members, start, end)
    }

    /// Count of accounts holding `role`.
    pub fn get_role_member_count(env: Env, role: BytesN<32>) -> u32 {
        let members: Vec<Address> = env
            .storage()
            .persistent()
            .get(&RoleKey::RoleMembers(role))
            .unwrap_or(Vec::new(&env));
        members.len()
    }

    /// All role IDs that have ever been granted.
    pub fn get_all_roles(env: Env) -> Vec<BytesN<32>> {
        env.storage()
            .persistent()
            .get(&RoleKey::AllRoles)
            .unwrap_or(Vec::new(&env))
    }

    /// Count of distinct roles.
    pub fn get_role_count(env: Env) -> u32 {
        let all: Vec<BytesN<32>> = env
            .storage()
            .persistent()
            .get(&RoleKey::AllRoles)
            .unwrap_or(Vec::new(&env));
        all.len()
    }
}

// ─── Internal helpers ─────────────────────────────────────────────────────────

fn require_init(env: &Env) {
    if !env.storage().instance().has(&RoleKey::Initialized) {
        panic_with_error!(env, Error::NotInitialized);
    }
}

fn require_admin(env: &Env, caller: &Address) {
    let admin_role = roles::role_admin(env);
    let has: bool = env
        .storage()
        .persistent()
        .get(&RoleKey::HasRole(caller.clone(), admin_role))
        .unwrap_or(false);
    if !has {
        panic_with_error!(env, Error::Unauthorized);
    }
}

fn internal_grant_role(env: &Env, account: &Address, role: &BytesN<32>) {
    let has_key = RoleKey::HasRole(account.clone(), role.clone());
    if env.storage().persistent().get::<_, bool>(&has_key).unwrap_or(false) {
        return; // idempotent
    }
    env.storage().persistent().set(&has_key, &true);

    // Add to role's member list
    let mut members: Vec<Address> = env
        .storage()
        .persistent()
        .get(&RoleKey::RoleMembers(role.clone()))
        .unwrap_or(Vec::new(env));
    members.push_back(account.clone());
    env.storage().persistent().set(&RoleKey::RoleMembers(role.clone()), &members);

    // Add to account's role list
    let mut acct_roles: Vec<BytesN<32>> = env
        .storage()
        .persistent()
        .get(&RoleKey::AccountRoles(account.clone()))
        .unwrap_or(Vec::new(env));
    acct_roles.push_back(role.clone());
    env.storage().persistent().set(&RoleKey::AccountRoles(account.clone()), &acct_roles);

    // Track in all-roles list (deduplicated)
    let mut all: Vec<BytesN<32>> = env
        .storage()
        .persistent()
        .get(&RoleKey::AllRoles)
        .unwrap_or(Vec::new(env));
    if !vec_contains_b32(&all, role) {
        all.push_back(role.clone());
        env.storage().persistent().set(&RoleKey::AllRoles, &all);
    }
}

fn internal_revoke_role(env: &Env, account: &Address, role: &BytesN<32>) {
    let has_key = RoleKey::HasRole(account.clone(), role.clone());
    if !env.storage().persistent().get::<_, bool>(&has_key).unwrap_or(false) {
        return; // idempotent
    }
    env.storage().persistent().remove(&has_key);

    // Remove from role's member list
    let mut members: Vec<Address> = env
        .storage()
        .persistent()
        .get(&RoleKey::RoleMembers(role.clone()))
        .unwrap_or(Vec::new(env));
    vec_remove_addr(&mut members, account);
    env.storage().persistent().set(&RoleKey::RoleMembers(role.clone()), &members);

    // Remove from account's role list
    let mut acct_roles: Vec<BytesN<32>> = env
        .storage()
        .persistent()
        .get(&RoleKey::AccountRoles(account.clone()))
        .unwrap_or(Vec::new(env));
    vec_remove_b32(&mut acct_roles, role);
    env.storage().persistent().set(&RoleKey::AccountRoles(account.clone()), &acct_roles);
}

// ─── Vec utilities (no_std) ───────────────────────────────────────────────────

fn vec_contains_b32(vec: &Vec<BytesN<32>>, item: &BytesN<32>) -> bool {
    for i in 0..vec.len() {
        if vec.get_unchecked(i) == *item {
            return true;
        }
    }
    false
}

fn vec_remove_addr(vec: &mut Vec<Address>, item: &Address) {
    for i in 0..vec.len() {
        if vec.get_unchecked(i) == *item {
            vec.remove(i);
            return;
        }
    }
}

fn vec_remove_b32(vec: &mut Vec<BytesN<32>>, item: &BytesN<32>) {
    for i in 0..vec.len() {
        if vec.get_unchecked(i) == *item {
            vec.remove(i);
            return;
        }
    }
}

fn paginate_addr(env: &Env, vec: &Vec<Address>, start: u32, end: u32) -> Vec<Address> {
    let len = vec.len();
    let start = start.min(len);
    let end = end.min(len);
    let mut out = Vec::new(env);
    for i in start..end {
        out.push_back(vec.get_unchecked(i));
    }
    out
}

// ─── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use soroban_sdk::{testutils::Address as _, Env};

    fn setup() -> (Env, Address, Address) {
        let env = Env::default();
        env.mock_all_auths();
        let admin = Address::generate(&env);
        let contract_id = env.register(RoleStore, ());
        let client = RoleStoreClient::new(&env, &contract_id);
        client.initialize(&admin);
        (env, admin, contract_id)
    }

    #[test]
    fn test_admin_has_role_after_init() {
        let (env, admin, contract_id) = setup();
        let client = RoleStoreClient::new(&env, &contract_id);
        let admin_role = roles::role_admin(&env);
        assert!(client.has_role(&admin, &admin_role));
    }

    #[test]
    fn test_grant_and_revoke() {
        let (env, admin, contract_id) = setup();
        let client = RoleStoreClient::new(&env, &contract_id);
        let ctrl = roles::controller(&env);
        let keeper = Address::generate(&env);

        assert!(!client.has_role(&keeper, &ctrl));
        client.grant_role(&admin, &keeper, &ctrl);
        assert!(client.has_role(&keeper, &ctrl));

        client.revoke_role(&admin, &keeper, &ctrl);
        assert!(!client.has_role(&keeper, &ctrl));
    }

    #[test]
    fn test_role_member_enumeration() {
        let (env, admin, contract_id) = setup();
        let client = RoleStoreClient::new(&env, &contract_id);
        let ctrl = roles::controller(&env);
        let k1 = Address::generate(&env);
        let k2 = Address::generate(&env);

        client.grant_role(&admin, &k1, &ctrl);
        client.grant_role(&admin, &k2, &ctrl);

        assert_eq!(client.get_role_member_count(&ctrl), 2);
        let members = client.get_role_members(&ctrl, &0, &10);
        assert_eq!(members.len(), 2);
    }

    #[test]
    #[should_panic]
    fn test_cannot_remove_last_admin() {
        let (env, admin, contract_id) = setup();
        let client = RoleStoreClient::new(&env, &contract_id);
        let admin_role = roles::role_admin(&env);
        client.revoke_role(&admin, &admin, &admin_role);
    }

    #[test]
    fn test_idempotent_grant() {
        let (env, admin, contract_id) = setup();
        let client = RoleStoreClient::new(&env, &contract_id);
        let ctrl = roles::controller(&env);
        let keeper = Address::generate(&env);

        client.grant_role(&admin, &keeper, &ctrl);
        client.grant_role(&admin, &keeper, &ctrl); // second is no-op
        assert_eq!(client.get_role_member_count(&ctrl), 1);
    }

    #[test]
    fn test_all_roles_tracked() {
        let (env, admin, contract_id) = setup();
        let client = RoleStoreClient::new(&env, &contract_id);
        // ROLE_ADMIN was granted at init
        assert_eq!(client.get_role_count(), 1);
        let ctrl = roles::controller(&env);
        let keeper = Address::generate(&env);
        client.grant_role(&admin, &keeper, &ctrl);
        assert_eq!(client.get_role_count(), 2);
    }
}
