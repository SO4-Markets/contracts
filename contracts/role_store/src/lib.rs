#![no_std]

use gmx_keys::roles;
use soroban_sdk::{
    contract, contracterror, contractevent, contractimpl, contracttype, panic_with_error, Address,
    BytesN, Env, Vec,
};

// ─── Errors ───────────────────────────────────────────────────────────────────

#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
#[repr(u32)]
pub enum Error {
    NotInitialized = 1,
    AlreadyInitialized = 2,
    Unauthorized = 3,
    LastAdmin = 4, // can't remove the last ROLE_ADMIN holder
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
    /// u32 — number of members holding a given role (avoids full Vec read)
    RoleMemberCount(BytesN<32>),
}

// ─── Events ───────────────────────────────────────────────────────────────────

#[contractevent(topics = ["init"])]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RoleStoreInitialized {
    pub admin: Address,
    pub admin_role: BytesN<32>,
}

#[contractevent(topics = ["grant"])]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RoleGranted {
    pub caller: Address,
    pub account: Address,
    pub role: BytesN<32>,
}

#[contractevent(topics = ["revoke"])]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RoleRevoked {
    pub caller: Address,
    pub account: Address,
    pub role: BytesN<32>,
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
        env.events()
            .publish_event(&RoleStoreInitialized { admin, admin_role });
    }

    // ── Public write ─────────────────────────────────────────────────────────

    /// Grant `role` to `account`. Caller must hold ROLE_ADMIN.
    pub fn grant_role(env: Env, caller: Address, account: Address, role: BytesN<32>) {
        caller.require_auth();
        require_init(&env);
        require_admin(&env, &caller);
        internal_grant_role(&env, &account, &role);
        env.events().publish_event(&RoleGranted { caller, account, role });
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
        env.events().publish_event(&RoleRevoked { caller, account, role });
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
        env.storage()
            .persistent()
            .get(&RoleKey::RoleMemberCount(role))
            .unwrap_or(0)
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
    if env
        .storage()
        .persistent()
        .get::<_, bool>(&has_key)
        .unwrap_or(false)
    {
        return; // idempotent
    }
    env.storage().persistent().set(&has_key, &true);

    // Increment member count
    let count_key = RoleKey::RoleMemberCount(role.clone());
    let count: u32 = env
        .storage()
        .persistent()
        .get(&count_key)
        .unwrap_or(0);
    env.storage()
        .persistent()
        .set(&count_key, &(count + 1));

    // Add to role's member list
    let mut members: Vec<Address> = env
        .storage()
        .persistent()
        .get(&RoleKey::RoleMembers(role.clone()))
        .unwrap_or(Vec::new(env));
    members.push_back(account.clone());
    env.storage()
        .persistent()
        .set(&RoleKey::RoleMembers(role.clone()), &members);

    // Add to account's role list
    let mut acct_roles: Vec<BytesN<32>> = env
        .storage()
        .persistent()
        .get(&RoleKey::AccountRoles(account.clone()))
        .unwrap_or(Vec::new(env));
    acct_roles.push_back(role.clone());
    env.storage()
        .persistent()
        .set(&RoleKey::AccountRoles(account.clone()), &acct_roles);

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
    if !env
        .storage()
        .persistent()
        .get::<_, bool>(&has_key)
        .unwrap_or(false)
    {
        return; // idempotent
    }
    env.storage().persistent().remove(&has_key);

    // Decrement member count
    let count_key = RoleKey::RoleMemberCount(role.clone());
    let count: u32 = env
        .storage()
        .persistent()
        .get(&count_key)
        .unwrap_or(0);
    if count > 0 {
        env.storage()
            .persistent()
            .set(&count_key, &(count - 1));
    }

    // Remove from role's member list
    let mut members: Vec<Address> = env
        .storage()
        .persistent()
        .get(&RoleKey::RoleMembers(role.clone()))
        .unwrap_or(Vec::new(env));
    vec_remove_addr(&mut members, account);
    env.storage()
        .persistent()
        .set(&RoleKey::RoleMembers(role.clone()), &members);

    // Remove from account's role list
    let mut acct_roles: Vec<BytesN<32>> = env
        .storage()
        .persistent()
        .get(&RoleKey::AccountRoles(account.clone()))
        .unwrap_or(Vec::new(env));
    vec_remove_b32(&mut acct_roles, role);
    env.storage()
        .persistent()
        .set(&RoleKey::AccountRoles(account.clone()), &acct_roles);
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

    // ── Issue #109: authorization matrix tests ────────────────────────────────

    /// A non-admin address must not be able to grant roles (ROLE_ADMIN check).
    #[test]
    #[should_panic]
    fn grant_role_by_non_admin_panics() {
        let (env, _admin, contract_id) = setup();
        // mock_all_auths lets require_auth() pass; the role check itself must
        // reject an address that does not hold ROLE_ADMIN.
        let client = RoleStoreClient::new(&env, &contract_id);
        let impostor = Address::generate(&env);
        let victim = Address::generate(&env);
        let ctrl = roles::controller(&env);
        // impostor has no role — grant_role must panic with Unauthorized.
        client.grant_role(&impostor, &victim, &ctrl);
    }

    /// A non-admin address must not be able to revoke roles (ROLE_ADMIN check).
    #[test]
    #[should_panic]
    fn revoke_role_by_non_admin_panics() {
        let (env, admin, contract_id) = setup();
        let client = RoleStoreClient::new(&env, &contract_id);
        let ctrl = roles::controller(&env);
        let holder = Address::generate(&env);
        client.grant_role(&admin, &holder, &ctrl);

        let impostor = Address::generate(&env);
        // impostor does not hold ROLE_ADMIN — revoke must panic.
        client.revoke_role(&impostor, &holder, &ctrl);
    }

    // ── Issue #359: get_roles(account) test coverage ────────────────────────

    /// get_roles(account) must reflect grants and revokes on the account-keyed
    /// side of the bookkeeping (AccountRoles), including multiple roles
    /// simultaneously and correct removal after revoke.
    #[test]
    fn get_roles_reflects_grants_and_revokes() {
        let (env, admin, contract_id) = setup();
        let client = RoleStoreClient::new(&env, &contract_id);
        let ctrl = roles::controller(&env);
        let order_keeper = roles::order_keeper(&env);
        let user = Address::generate(&env);

        // Initially empty
        let roles_list = client.get_roles(&user);
        assert_eq!(roles_list.len(), 0);

        // Grant first role
        client.grant_role(&admin, &user, &ctrl);
        let roles_list = client.get_roles(&user);
        assert_eq!(roles_list.len(), 1);
        assert_eq!(roles_list.get_unchecked(0), ctrl);

        // Grant second role — both must be present
        client.grant_role(&admin, &user, &order_keeper);
        let roles_list = client.get_roles(&user);
        assert_eq!(roles_list.len(), 2);
        assert!(vec_contains_b32(&roles_list, &ctrl));
        assert!(vec_contains_b32(&roles_list, &order_keeper));

        // Revoke first role — only second must remain
        client.revoke_role(&admin, &user, &ctrl);
        let roles_list = client.get_roles(&user);
        assert_eq!(roles_list.len(), 1);
        assert_eq!(roles_list.get_unchecked(0), order_keeper);
        assert!(!vec_contains_b32(&roles_list, &ctrl));

        // Revoke second role — empty again
        client.revoke_role(&admin, &user, &order_keeper);
        let roles_list = client.get_roles(&user);
        assert_eq!(roles_list.len(), 0);
    }

    // ── Issue #233: CONTROLLER cannot self-grant ROLE_ADMIN ──────────────────

    /// A CONTROLLER-only address cannot call grant_role to elevate itself to
    /// ROLE_ADMIN. The require_admin guard inside grant_role is gated on
    /// role_store's own storage — not on the CONTROLLER role in data_store.
    #[test]
    #[should_panic]
    fn controller_cannot_grant_self_admin() {
        let (env, admin, contract_id) = setup();
        let client = RoleStoreClient::new(&env, &contract_id);
        let ctrl = roles::controller(&env);
        let admin_role = roles::role_admin(&env);
        let controller_addr = Address::generate(&env);
        // Grant CONTROLLER but NOT ROLE_ADMIN
        client.grant_role(&admin, &controller_addr, &ctrl);
        // CONTROLLER tries to elevate itself to ROLE_ADMIN — must panic Unauthorized
        client.grant_role(&controller_addr, &controller_addr, &admin_role);
    }

    /// A CONTROLLER-only address cannot grant ROLE_ADMIN to any third party either.
    #[test]
    #[should_panic]
    fn controller_cannot_call_grant_role_directly() {
        let (env, admin, contract_id) = setup();
        let client = RoleStoreClient::new(&env, &contract_id);
        let ctrl = roles::controller(&env);
        let admin_role = roles::role_admin(&env);
        let controller_addr = Address::generate(&env);
        let victim = Address::generate(&env);
        client.grant_role(&admin, &controller_addr, &ctrl);
        // CONTROLLER tries to grant ROLE_ADMIN to victim — must panic Unauthorized
        client.grant_role(&controller_addr, &victim, &admin_role);
    }

    #[test]
    fn test_events_include_caller() {
        use soroban_sdk::testutils::Events;
        use soroban_sdk::{symbol_short, IntoVal};
        
        let (env, admin, contract_id) = setup();
        let client = RoleStoreClient::new(&env, &contract_id);
        let ctrl = roles::controller(&env);
        let keeper = Address::generate(&env);

        client.grant_role(&admin, &keeper, &ctrl);
        
        let events = env.events().all();
        assert!(
            events.contains((
                contract_id.clone(),
                (symbol_short!("grant"),).into_val(&env),
                RoleGranted {
                    caller: admin.clone(),
                    account: keeper.clone(),
                    role: ctrl.clone(),
                }.into_val(&env)
            ))
        );

        client.revoke_role(&admin, &keeper, &ctrl);
        
        let events = env.events().all();
        assert!(
            events.contains((
                contract_id.clone(),
                (symbol_short!("revoke"),).into_val(&env),
                RoleRevoked {
                    caller: admin.clone(),
                    account: keeper.clone(),
                    role: ctrl.clone(),
                }.into_val(&env)
            ))
        );
    }

    // ── Issue #564: revoke-side RoleMemberCount/RoleMembers coverage ─────────

    /// Grant a role to two accounts, revoke one, and assert the count
    /// decremented by exactly one and the remaining member list is correct.
    #[test]
    fn test_revoke_decrements_count_and_removes_member() {
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

        client.revoke_role(&admin, &k1, &ctrl);

        assert_eq!(
            client.get_role_member_count(&ctrl),
            1,
            "count must decrement by exactly one after revoke"
        );
        assert!(!client.has_role(&k1, &ctrl));
        assert!(client.has_role(&k2, &ctrl));

        let members = client.get_role_members(&ctrl, &0, &10);
        assert_eq!(members.len(), 1);
        assert_eq!(members.get_unchecked(0), k2);
        // revoked account must no longer appear
        assert!(!vec_contains_b32(&client.get_roles(&k1), &ctrl));
    }

    /// Revoke one member out of three and assert the other two remain in order
    /// and vec_remove_addr removed only the target (middle of the list).
    #[test]
    fn test_revoke_one_of_many_leaves_others() {
        let (env, admin, contract_id) = setup();
        let client = RoleStoreClient::new(&env, &contract_id);
        let ctrl = roles::controller(&env);
        let k1 = Address::generate(&env);
        let k2 = Address::generate(&env);
        let k3 = Address::generate(&env);

        client.grant_role(&admin, &k1, &ctrl);
        client.grant_role(&admin, &k2, &ctrl);
        client.grant_role(&admin, &k3, &ctrl);
        assert_eq!(client.get_role_member_count(&ctrl), 3);

        // revoke the middle member (k2) — exercises vec_remove_addr not just popping tail
        client.revoke_role(&admin, &k2, &ctrl);

        assert_eq!(client.get_role_member_count(&ctrl), 2);
        assert!(!client.has_role(&k2, &ctrl));
        assert!(client.has_role(&k1, &ctrl));
        assert!(client.has_role(&k3, &ctrl));

        let members = client.get_role_members(&ctrl, &0, &10);
        assert_eq!(members.len(), 2);
        // remaining members must be k1 and k3 (k2 removed from middle)
        assert_eq!(members.get_unchecked(0), k1);
        assert_eq!(members.get_unchecked(1), k3);

        // revoke the first member (k1) — exercises remove from start of list
        client.revoke_role(&admin, &k1, &ctrl);
        assert_eq!(client.get_role_member_count(&ctrl), 1);
        let members = client.get_role_members(&ctrl, &0, &10);
        assert_eq!(members.len(), 1);
        assert_eq!(members.get_unchecked(0), k3);
    }

    /// Second revoke of the same account/role must be a harmless no-op
    /// (internal_revoke_role is documented as idempotent). Count must not
    /// go negative or decrement twice.
    #[test]
    fn test_idempotent_revoke() {
        let (env, admin, contract_id) = setup();
        let client = RoleStoreClient::new(&env, &contract_id);
        let ctrl = roles::controller(&env);
        let keeper = Address::generate(&env);

        client.grant_role(&admin, &keeper, &ctrl);
        assert_eq!(client.get_role_member_count(&ctrl), 1);

        client.revoke_role(&admin, &keeper, &ctrl);
        assert_eq!(client.get_role_member_count(&ctrl), 0);
        assert!(!client.has_role(&keeper, &ctrl));

        // second revoke — must not panic, must not decrement below zero
        client.revoke_role(&admin, &keeper, &ctrl);
        assert_eq!(
            client.get_role_member_count(&ctrl),
            0,
            "second revoke must be a no-op and not go negative"
        );
        assert!(!client.has_role(&keeper, &ctrl));
        let members = client.get_role_members(&ctrl, &0, &10);
        assert_eq!(members.len(), 0);

        // revoking an account that never held the role at all must also be a no-op
        let never_holder = Address::generate(&env);
        client.revoke_role(&admin, &never_holder, &ctrl);
        assert_eq!(client.get_role_member_count(&ctrl), 0);
    }

    /// Revoke-side bookkeeping for get_roles: after revoke, the account's
    /// role list must no longer contain the revoked role while other roles
    /// remain — mirroring get_roles_reflects_grants_and_revokes.
    #[test]
    fn test_revoke_updates_account_roles_and_member_list_together() {
        let (env, admin, contract_id) = setup();
        let client = RoleStoreClient::new(&env, &contract_id);
        let ctrl = roles::controller(&env);
        let order_keeper = roles::order_keeper(&env);
        let user = Address::generate(&env);
        let other = Address::generate(&env);

        // give user two roles and other one role
        client.grant_role(&admin, &user, &ctrl);
        client.grant_role(&admin, &user, &order_keeper);
        client.grant_role(&admin, &other, &ctrl);
        assert_eq!(client.get_role_member_count(&ctrl), 2);

        // revoke ctrl from user — user should keep order_keeper, other keeps ctrl
        client.revoke_role(&admin, &user, &ctrl);

        assert_eq!(client.get_role_member_count(&ctrl), 1);
        let members = client.get_role_members(&ctrl, &0, &10);
        assert_eq!(members.len(), 1);
        assert_eq!(members.get_unchecked(0), other);

        let user_roles = client.get_roles(&user);
        assert_eq!(user_roles.len(), 1);
        assert_eq!(user_roles.get_unchecked(0), order_keeper);
        assert!(!vec_contains_b32(&user_roles, &ctrl));
    }
}
