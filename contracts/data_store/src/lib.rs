#![no_std]

use soroban_sdk::{
    contract, contractevent, contractimpl, contracttype, contracterror, panic_with_error,
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
    Underflow         = 4, // apply_delta would cause underflow
}

// ─── Instance storage keys ────────────────────────────────────────────────────

#[contracttype]
enum InstanceKey {
    Initialized,
    RoleStore,
}

// ─── Typed persistent storage keys ───────────────────────────────────────────
//
// We wrap the user-supplied BytesN<32> key in a discriminant enum so that
// a u128 and an i128 stored under the same bytes32 key cannot collide.

#[contracttype]
enum DataKey {
    U128(BytesN<32>),
    I128(BytesN<32>),
    Addr(BytesN<32>),
    Bool(BytesN<32>),
    B32(BytesN<32>),
    AddrSet(BytesN<32>),
    B32Set(BytesN<32>),
}

// ─── Cross-contract role check interface ─────────────────────────────────────

#[allow(dead_code)]
#[soroban_sdk::contractclient(name = "RoleStoreClient")]
trait IRoleStore {
    fn has_role(env: Env, account: Address, role: BytesN<32>) -> bool;
}

// ─── Events ───────────────────────────────────────────────────────────────────

#[contractevent(topics = ["init"])]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DataStoreInitialized {
    pub role_store: Address,
}

// ─── Contract ─────────────────────────────────────────────────────────────────

#[contract]
pub struct DataStore;

#[contractimpl]
impl DataStore {
    // ── Initializer ──────────────────────────────────────────────────────────

    /// One-time init: link to role_store for CONTROLLER checks.
    pub fn initialize(env: Env, admin: Address, role_store: Address) {
        admin.require_auth();
        if env.storage().instance().has(&InstanceKey::Initialized) {
            panic_with_error!(&env, Error::AlreadyInitialized);
        }
        env.storage().instance().set(&InstanceKey::Initialized, &true);
        env.storage().instance().set(&InstanceKey::RoleStore, &role_store);
        env.events().publish_event(&DataStoreInitialized { role_store });
    }

    // ── u128 operations ──────────────────────────────────────────────────────

    pub fn get_u128(env: Env, key: BytesN<32>) -> u128 {
        env.storage().persistent().get(&DataKey::U128(key)).unwrap_or(0)
    }

    pub fn set_u128(env: Env, caller: Address, key: BytesN<32>, value: u128) -> u128 {
        caller.require_auth();
        require_controller(&env, &caller);
        env.storage().persistent().set(&DataKey::U128(key), &value);
        value
    }

    pub fn remove_u128(env: Env, caller: Address, key: BytesN<32>) {
        caller.require_auth();
        require_controller(&env, &caller);
        env.storage().persistent().remove(&DataKey::U128(key));
    }

    /// Add `delta` (signed) to existing u128 value. Panics on underflow.
    pub fn apply_delta_to_u128(env: Env, caller: Address, key: BytesN<32>, delta: i128) -> u128 {
        caller.require_auth();
        require_controller(&env, &caller);
        let current: u128 = env.storage().persistent().get(&DataKey::U128(key.clone())).unwrap_or(0);
        let next = if delta >= 0 {
            current.saturating_add(delta as u128)
        } else {
            let sub = (-delta) as u128;
            if sub > current {
                panic_with_error!(&env, Error::Underflow);
            }
            current - sub
        };
        env.storage().persistent().set(&DataKey::U128(key), &next);
        next
    }

    pub fn increment_u128(env: Env, caller: Address, key: BytesN<32>, amount: u128) -> u128 {
        caller.require_auth();
        require_controller(&env, &caller);
        let current: u128 = env.storage().persistent().get(&DataKey::U128(key.clone())).unwrap_or(0);
        let next = current.saturating_add(amount);
        env.storage().persistent().set(&DataKey::U128(key), &next);
        next
    }

    pub fn decrement_u128(env: Env, caller: Address, key: BytesN<32>, amount: u128) -> u128 {
        caller.require_auth();
        require_controller(&env, &caller);
        let current: u128 = env.storage().persistent().get(&DataKey::U128(key.clone())).unwrap_or(0);
        if amount > current {
            panic_with_error!(&env, Error::Underflow);
        }
        let next = current - amount;
        env.storage().persistent().set(&DataKey::U128(key), &next);
        next
    }

    // ── i128 operations ──────────────────────────────────────────────────────

    pub fn get_i128(env: Env, key: BytesN<32>) -> i128 {
        env.storage().persistent().get(&DataKey::I128(key)).unwrap_or(0)
    }

    pub fn set_i128(env: Env, caller: Address, key: BytesN<32>, value: i128) -> i128 {
        caller.require_auth();
        require_controller(&env, &caller);
        env.storage().persistent().set(&DataKey::I128(key), &value);
        value
    }

    pub fn remove_i128(env: Env, caller: Address, key: BytesN<32>) {
        caller.require_auth();
        require_controller(&env, &caller);
        env.storage().persistent().remove(&DataKey::I128(key));
    }

    pub fn apply_delta_to_i128(env: Env, caller: Address, key: BytesN<32>, delta: i128) -> i128 {
        caller.require_auth();
        require_controller(&env, &caller);
        let current: i128 = env.storage().persistent().get(&DataKey::I128(key.clone())).unwrap_or(0);
        let next = current.saturating_add(delta);
        env.storage().persistent().set(&DataKey::I128(key), &next);
        next
    }

    // ── Address operations ────────────────────────────────────────────────────

    pub fn get_address(env: Env, key: BytesN<32>) -> Option<Address> {
        env.storage().persistent().get(&DataKey::Addr(key))
    }

    pub fn set_address(env: Env, caller: Address, key: BytesN<32>, value: Address) -> Address {
        caller.require_auth();
        require_controller(&env, &caller);
        env.storage().persistent().set(&DataKey::Addr(key), &value);
        value
    }

    pub fn remove_address(env: Env, caller: Address, key: BytesN<32>) {
        caller.require_auth();
        require_controller(&env, &caller);
        env.storage().persistent().remove(&DataKey::Addr(key));
    }

    // ── bool operations ───────────────────────────────────────────────────────

    pub fn get_bool(env: Env, key: BytesN<32>) -> bool {
        env.storage().persistent().get(&DataKey::Bool(key)).unwrap_or(false)
    }

    pub fn set_bool(env: Env, caller: Address, key: BytesN<32>, value: bool) -> bool {
        caller.require_auth();
        require_controller(&env, &caller);
        env.storage().persistent().set(&DataKey::Bool(key), &value);
        value
    }

    pub fn remove_bool(env: Env, caller: Address, key: BytesN<32>) {
        caller.require_auth();
        require_controller(&env, &caller);
        env.storage().persistent().remove(&DataKey::Bool(key));
    }

    // ── BytesN<32> operations ─────────────────────────────────────────────────

    pub fn get_bytes32(env: Env, key: BytesN<32>) -> BytesN<32> {
        env.storage()
            .persistent()
            .get(&DataKey::B32(key))
            .unwrap_or(BytesN::from_array(&env, &[0u8; 32]))
    }

    pub fn set_bytes32(env: Env, caller: Address, key: BytesN<32>, value: BytesN<32>) -> BytesN<32> {
        caller.require_auth();
        require_controller(&env, &caller);
        env.storage().persistent().set(&DataKey::B32(key), &value);
        value
    }

    // ── Address set operations ────────────────────────────────────────────────

    pub fn add_address_to_set(env: Env, caller: Address, set_key: BytesN<32>, value: Address) {
        caller.require_auth();
        require_controller(&env, &caller);
        let mut set: Vec<Address> = env
            .storage()
            .persistent()
            .get(&DataKey::AddrSet(set_key.clone()))
            .unwrap_or(Vec::new(&env));
        if !vec_contains_addr(&set, &value) {
            set.push_back(value);
            env.storage().persistent().set(&DataKey::AddrSet(set_key), &set);
        }
    }

    pub fn remove_address_from_set(env: Env, caller: Address, set_key: BytesN<32>, value: Address) {
        caller.require_auth();
        require_controller(&env, &caller);
        let mut set: Vec<Address> = env
            .storage()
            .persistent()
            .get(&DataKey::AddrSet(set_key.clone()))
            .unwrap_or(Vec::new(&env));
        vec_remove_addr(&mut set, &value);
        env.storage().persistent().set(&DataKey::AddrSet(set_key), &set);
    }

    pub fn get_address_set_count(env: Env, set_key: BytesN<32>) -> u32 {
        let set: Vec<Address> = env
            .storage()
            .persistent()
            .get(&DataKey::AddrSet(set_key))
            .unwrap_or(Vec::new(&env));
        set.len()
    }

    pub fn get_address_set_at(env: Env, set_key: BytesN<32>, start: u32, end: u32) -> Vec<Address> {
        let set: Vec<Address> = env
            .storage()
            .persistent()
            .get(&DataKey::AddrSet(set_key))
            .unwrap_or(Vec::new(&env));
        paginate_addr(&env, &set, start, end)
    }

    pub fn contains_address(env: Env, set_key: BytesN<32>, value: Address) -> bool {
        let set: Vec<Address> = env
            .storage()
            .persistent()
            .get(&DataKey::AddrSet(set_key))
            .unwrap_or(Vec::new(&env));
        vec_contains_addr(&set, &value)
    }

    // ── BytesN<32> set operations ─────────────────────────────────────────────

    pub fn add_bytes32_to_set(env: Env, caller: Address, set_key: BytesN<32>, value: BytesN<32>) {
        caller.require_auth();
        require_controller(&env, &caller);
        let mut set: Vec<BytesN<32>> = env
            .storage()
            .persistent()
            .get(&DataKey::B32Set(set_key.clone()))
            .unwrap_or(Vec::new(&env));
        if !vec_contains_b32(&set, &value) {
            set.push_back(value);
            env.storage().persistent().set(&DataKey::B32Set(set_key), &set);
        }
    }

    pub fn remove_bytes32_from_set(env: Env, caller: Address, set_key: BytesN<32>, value: BytesN<32>) {
        caller.require_auth();
        require_controller(&env, &caller);
        let mut set: Vec<BytesN<32>> = env
            .storage()
            .persistent()
            .get(&DataKey::B32Set(set_key.clone()))
            .unwrap_or(Vec::new(&env));
        vec_remove_b32(&mut set, &value);
        env.storage().persistent().set(&DataKey::B32Set(set_key), &set);
    }

    pub fn get_bytes32_set_count(env: Env, set_key: BytesN<32>) -> u32 {
        let set: Vec<BytesN<32>> = env
            .storage()
            .persistent()
            .get(&DataKey::B32Set(set_key))
            .unwrap_or(Vec::new(&env));
        set.len()
    }

    pub fn get_bytes32_set_at(env: Env, set_key: BytesN<32>, start: u32, end: u32) -> Vec<BytesN<32>> {
        let set: Vec<BytesN<32>> = env
            .storage()
            .persistent()
            .get(&DataKey::B32Set(set_key))
            .unwrap_or(Vec::new(&env));
        paginate_b32(&env, &set, start, end)
    }

    pub fn contains_bytes32(env: Env, set_key: BytesN<32>, value: BytesN<32>) -> bool {
        let set: Vec<BytesN<32>> = env
            .storage()
            .persistent()
            .get(&DataKey::B32Set(set_key))
            .unwrap_or(Vec::new(&env));
        vec_contains_b32(&set, &value)
    }

    // ── Nonce (auto-incrementing counter for order/deposit keys) ──────────────

    pub fn get_nonce(env: Env) -> u64 {
        use gmx_keys::nonce_key;
        let key = DataKey::U128(nonce_key(&env));
        env.storage().persistent().get(&key).unwrap_or(0u128) as u64
    }

    pub fn increment_nonce(env: Env, caller: Address) -> u64 {
        caller.require_auth();
        require_controller(&env, &caller);
        use gmx_keys::nonce_key;
        let key = DataKey::U128(nonce_key(&env));
        let current: u128 = env.storage().persistent().get(&key).unwrap_or(0);
        let next = current + 1;
        env.storage().persistent().set(&key, &next);
        next as u64
    }
}

// ─── Internal helpers ─────────────────────────────────────────────────────────

fn require_init(env: &Env) {
    if !env.storage().instance().has(&InstanceKey::Initialized) {
        panic_with_error!(env, Error::NotInitialized);
    }
}

fn require_controller(env: &Env, caller: &Address) {
    require_init(env);
    let role_store: Address = env
        .storage()
        .instance()
        .get(&InstanceKey::RoleStore)
        .unwrap();
    let client = RoleStoreClient::new(env, &role_store);
    let ctrl_role = roles::controller(env);
    if !client.has_role(caller, &ctrl_role) {
        panic_with_error!(env, Error::Unauthorized);
    }
}

// ─── Vec utilities ────────────────────────────────────────────────────────────

fn vec_contains_addr(vec: &Vec<Address>, item: &Address) -> bool {
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

fn vec_contains_b32(vec: &Vec<BytesN<32>>, item: &BytesN<32>) -> bool {
    for i in 0..vec.len() {
        if vec.get_unchecked(i) == *item {
            return true;
        }
    }
    false
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
    let s = start.min(len);
    let e = end.min(len);
    let mut out = Vec::new(env);
    for i in s..e {
        out.push_back(vec.get_unchecked(i));
    }
    out
}

fn paginate_b32(env: &Env, vec: &Vec<BytesN<32>>, start: u32, end: u32) -> Vec<BytesN<32>> {
    let len = vec.len();
    let s = start.min(len);
    let e = end.min(len);
    let mut out = Vec::new(env);
    for i in s..e {
        out.push_back(vec.get_unchecked(i));
    }
    out
}

// ─── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use soroban_sdk::{testutils::Address as _, Env};
    use role_store::{RoleStore, RoleStoreClient as RoleClient};

    fn setup() -> (Env, Address, Address, Address) {
        let env = Env::default();
        env.mock_all_auths();

        let admin = Address::generate(&env);

        // Deploy role_store
        let rs_id = env.register(RoleStore, ());
        let rs_client = RoleClient::new(&env, &rs_id);
        rs_client.initialize(&admin);

        // Grant CONTROLLER role to admin (for test purposes)
        let ctrl_role = roles::controller(&env);
        rs_client.grant_role(&admin, &admin, &ctrl_role);

        // Deploy data_store
        let ds_id = env.register(DataStore, ());
        let ds_client = DataStoreClient::new(&env, &ds_id);
        ds_client.initialize(&admin, &rs_id);

        (env, admin, rs_id, ds_id)
    }

    #[test]
    fn test_u128_crud() {
        let (env, admin, _, ds_id) = setup();
        let client = DataStoreClient::new(&env, &ds_id);
        let key = BytesN::from_array(&env, &[1u8; 32]);

        assert_eq!(client.get_u128(&key), 0);
        client.set_u128(&admin, &key, &42u128);
        assert_eq!(client.get_u128(&key), 42);
        client.remove_u128(&admin, &key);
        assert_eq!(client.get_u128(&key), 0);
    }

    #[test]
    fn test_apply_delta_to_u128() {
        let (env, admin, _, ds_id) = setup();
        let client = DataStoreClient::new(&env, &ds_id);
        let key = BytesN::from_array(&env, &[2u8; 32]);

        client.set_u128(&admin, &key, &100u128);
        let result = client.apply_delta_to_u128(&admin, &key, &50i128);
        assert_eq!(result, 150);
        let result = client.apply_delta_to_u128(&admin, &key, &(-30i128));
        assert_eq!(result, 120);
    }

    #[test]
    #[should_panic]
    fn test_apply_delta_underflow() {
        let (env, admin, _, ds_id) = setup();
        let client = DataStoreClient::new(&env, &ds_id);
        let key = BytesN::from_array(&env, &[3u8; 32]);

        client.set_u128(&admin, &key, &10u128);
        client.apply_delta_to_u128(&admin, &key, &(-20i128)); // underflow
    }

    #[test]
    fn test_i128_crud() {
        let (env, admin, _, ds_id) = setup();
        let client = DataStoreClient::new(&env, &ds_id);
        let key = BytesN::from_array(&env, &[4u8; 32]);

        assert_eq!(client.get_i128(&key), 0);
        client.set_i128(&admin, &key, &-500i128);
        assert_eq!(client.get_i128(&key), -500);
    }

    #[test]
    fn test_bool_crud() {
        let (env, admin, _, ds_id) = setup();
        let client = DataStoreClient::new(&env, &ds_id);
        let key = BytesN::from_array(&env, &[5u8; 32]);

        assert!(!client.get_bool(&key));
        client.set_bool(&admin, &key, &true);
        assert!(client.get_bool(&key));
    }

    #[test]
    fn test_address_set_ops() {
        let (env, admin, _, ds_id) = setup();
        let client = DataStoreClient::new(&env, &ds_id);
        let set_key = BytesN::from_array(&env, &[6u8; 32]);
        let a = Address::generate(&env);
        let b = Address::generate(&env);

        assert_eq!(client.get_address_set_count(&set_key), 0);
        client.add_address_to_set(&admin, &set_key, &a);
        client.add_address_to_set(&admin, &set_key, &b);
        client.add_address_to_set(&admin, &set_key, &a); // duplicate → no-op
        assert_eq!(client.get_address_set_count(&set_key), 2);
        assert!(client.contains_address(&set_key, &a));

        client.remove_address_from_set(&admin, &set_key, &a);
        assert_eq!(client.get_address_set_count(&set_key), 1);
        assert!(!client.contains_address(&set_key, &a));
    }

    #[test]
    fn test_bytes32_set_ops() {
        let (env, admin, _, ds_id) = setup();
        let client = DataStoreClient::new(&env, &ds_id);
        let set_key = BytesN::from_array(&env, &[7u8; 32]);
        let v1 = BytesN::from_array(&env, &[11u8; 32]);
        let v2 = BytesN::from_array(&env, &[22u8; 32]);

        client.add_bytes32_to_set(&admin, &set_key, &v1);
        client.add_bytes32_to_set(&admin, &set_key, &v2);
        assert_eq!(client.get_bytes32_set_count(&set_key), 2);
        assert!(client.contains_bytes32(&set_key, &v1));

        let page = client.get_bytes32_set_at(&set_key, &0, &2);
        assert_eq!(page.len(), 2);

        client.remove_bytes32_from_set(&admin, &set_key, &v1);
        assert_eq!(client.get_bytes32_set_count(&set_key), 1);
    }

    #[test]
    fn test_nonce() {
        let (env, admin, _, ds_id) = setup();
        let client = DataStoreClient::new(&env, &ds_id);

        assert_eq!(client.get_nonce(), 0);
        let n1 = client.increment_nonce(&admin);
        let n2 = client.increment_nonce(&admin);
        assert_eq!(n1, 1);
        assert_eq!(n2, 2);
    }
}
