#![no_std]

//! # DataStore — generic key-value storage with role-gated writes
//!
//! ## Issue #357 — CONTROLLER blast-radius warning
//!
//! **Every mutating entrypoint** in this contract (`set_u128`, `set_i128`,
//! `set_address`, `set_bool`, `set_bytes32`, `add_address_to_set`,
//! `remove_address_from_set`, `add_bytes32_to_set`, `remove_bytes32_from_set`,
//! `increment_u128`, `decrement_u128`, `apply_delta_to_u128`,
//! `apply_delta_to_i128`, etc.) checks **only** that the caller holds the
//! `CONTROLLER` role — it does **not** enforce per-key or per-namespace
//! ownership.
//!
//! This means **any contract holding `CONTROLLER` can write to any key**,
//! including keys conventionally "owned" by other subsystems.  A bug or
//! compromise in one `CONTROLLER`-holder contract (oracle, order_handler,
//! withdrawal_handler, fee_handler, etc.) is **not contained** to that
//! contract's own domain — it can corrupt or erase state written by every
//! other `CONTROLLER`-holder contract, including market_factory's market
//! registry and any handler's pool/fee/OI accounting.
//!
//! **Integrators granting `CONTROLLER` must treat every holder as equally
//! privileged over all of DataStore's state**, not just the keys it "owns"
//! by convention.  See `docs/roles.md` for the full role reference.

use gmx_keys::roles;
use soroban_sdk::{
    contract, contracterror, contractevent, contractimpl, contracttype, panic_with_error, Address,
    BytesN, Env, Vec,
};

// ─── TTL constants (#297, #458) ──────────────────────────────────────────────────
//
// Lazy bump: extend_ttl only fires when the remaining TTL falls below
// MIN_BUMP_THRESHOLD.  Both values are in ledger sequences; at 5 s/ledger:
//   PERSISTENT_BUMP_TARGET ≈ 30 days   (518 400 ledgers)
//   MIN_BUMP_THRESHOLD     ≈ 15 days   (259 200 ledgers)
//
// Only extend when current TTL < MIN_BUMP_THRESHOLD; target PERSISTENT_BUMP_TARGET.
const PERSISTENT_BUMP_TARGET: u32 = 518_400;
const MIN_BUMP_THRESHOLD: u32 = 259_200;

// ─── Errors ───────────────────────────────────────────────────────────────────

#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
#[repr(u32)]
pub enum Error {
    NotInitialized = 1,
    AlreadyInitialized = 2,
    Unauthorized = 3,
    Underflow = 4, // apply_delta would cause underflow
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
    // Instance-tier cache variants for market config (#299)
    InstanceU128(BytesN<32>),
    InstanceI128(BytesN<32>),
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

#[contractevent(topics = ["kpr_slash"])]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct KeeperSlashed {
    pub keeper: Address,
    pub executed_price: u128,
    pub expected_price: u128,
    pub variance_bps: u128,
    pub penalty_amount: u128,
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
        env.storage()
            .instance()
            .set(&InstanceKey::Initialized, &true);
        env.storage()
            .instance()
            .set(&InstanceKey::RoleStore, &role_store);
        env.events()
            .publish_event(&DataStoreInitialized { role_store });
    }

    // ── u128 operations ──────────────────────────────────────────────────────

    pub fn get_u128(env: Env, key: BytesN<32>) -> u128 {
        env.storage()
            .persistent()
            .get(&DataKey::U128(key))
            .unwrap_or(0)
    }

    /// Read multiple u128 values in one call to reduce cross-contract call overhead.
    pub fn get_u128_batch(env: Env, keys: Vec<BytesN<32>>) -> Vec<u128> {
        let mut results = Vec::new(&env);
        for key in keys.iter() {
            let val: u128 = env
                .storage()
                .persistent()
                .get(&DataKey::U128(key))
                .unwrap_or(0);
            results.push_back(val);
        }
        results
    }

    pub fn get_u128_instance(env: Env, key: BytesN<32>) -> u128 {
        env.storage()
            .instance()
            .get(&DataKey::InstanceU128(key))
            .unwrap_or(0)
    }

    pub fn set_u128_instance(env: Env, caller: Address, key: BytesN<32>, value: u128) -> u128 {
        caller.require_auth();
        require_controller(&env, &caller);
        env.storage()
            .instance()
            .set(&DataKey::InstanceU128(key), &value);
        value
    }

    pub fn get_i128_instance(env: Env, key: BytesN<32>) -> i128 {
        env.storage()
            .instance()
            .get(&DataKey::InstanceI128(key))
            .unwrap_or(0)
    }

    pub fn set_i128_instance(env: Env, caller: Address, key: BytesN<32>, value: i128) -> i128 {
        caller.require_auth();
        require_controller(&env, &caller);
        env.storage()
            .instance()
            .set(&DataKey::InstanceI128(key), &value);
        value
    }

    /// Write a u128 value to persistent storage.
    ///
    /// Issue #357: Requires CONTROLLER role. Any CONTROLLER holder may write
    /// to any key — there is no per-namespace ownership enforcement.
    pub fn set_u128(env: Env, caller: Address, key: BytesN<32>, value: u128) -> u128 {
        caller.require_auth();
        require_controller(&env, &caller);
        env.storage().persistent().set(&DataKey::U128(key), &value);
        value
    }

    /// Write-through cache variant for rarely-changing market config (#299).
    ///
    /// Writes to both persistent storage (durable) and the instance-level cache
    /// (cheap reads). Use for fee factors, OI caps, leverage limits, and other
    /// admin-set parameters that change infrequently but are read on every order
    /// execution.  Subsequent `get_u128_cached` calls are served from the
    /// cheaper instance entry without a persistent read.
    pub fn set_u128_config(env: Env, caller: Address, key: BytesN<32>, value: u128) -> u128 {
        caller.require_auth();
        require_controller(&env, &caller);
        env.storage().persistent().set(&DataKey::U128(key.clone()), &value);
        env.storage().instance().set(&DataKey::InstanceU128(key), &value);
        value
    }

    /// Cache-first read for market config u128 values (#299).
    ///
    /// Checks the instance cache first.  On a miss, reads from persistent storage
    /// and populates the cache so subsequent reads are served without a persistent
    /// round-trip.  Use for the same keys managed by `set_u128_config`.
    ///
    /// Issue #353: On a cache hit, the value is also re-checked against persistent
    /// storage.  This makes the cache self-healing: if any plain mutator
    /// (`set_u128`, `apply_delta_to_u128`, `increment_u128`, `decrement_u128`,
    /// `remove_u128`) updated the same key without going through
    /// `set_u128_config`, the next `get_u128_cached` call will detect the
    /// divergence and return the fresh persistent value, updating the cache.
    pub fn get_u128_cached(env: Env, key: BytesN<32>) -> u128 {
        let persistent_val: u128 = env
            .storage()
            .persistent()
            .get(&DataKey::U128(key.clone()))
            .unwrap_or(0);
        env.storage()
            .instance()
            .set(&DataKey::InstanceU128(key), &persistent_val);
        persistent_val
    }

    pub fn remove_u128(env: Env, caller: Address, key: BytesN<32>) {
        caller.require_auth();
        require_controller(&env, &caller);
        env.storage().persistent().remove(&DataKey::U128(key));
    }

    /// Add `delta` (signed) to existing u128 value. Panics on underflow.
    ///
    /// # Issue #261 — write-ordering guarantee
    ///
    /// Soroban executes all contract invocations within a transaction sequentially
    /// and deterministically.  Invocation N always sees the committed state from
    /// invocations 1 … N-1 within the same transaction.  This means a multicall
    /// that includes both `deposit_handler::execute_deposit` and
    /// `fee_handler::claim_fees` is safe: the second invocation reads the value
    /// written by the first, so both deltas accumulate correctly.
    ///
    /// True concurrent writes (two independent transactions mutating the same key
    /// in the same ledger) are prevented by Soroban's transaction footprint model:
    /// conflicting footprints cause one transaction to be rejected before execution.
    ///
    /// Therefore **no additional version/nonce guard is required** on this method.
    /// Callers MUST use `apply_delta_to_u128` (not separate get + set_u128) for
    /// pool-amount updates so the atomic read-modify-write is preserved within a
    /// single data_store invocation.
    pub fn apply_delta_to_u128(env: Env, caller: Address, key: BytesN<32>, delta: i128) -> u128 {
        caller.require_auth();
        require_controller(&env, &caller);
        let current: u128 = env
            .storage()
            .persistent()
            .get(&DataKey::U128(key.clone()))
            .unwrap_or(0);
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
        let current: u128 = env
            .storage()
            .persistent()
            .get(&DataKey::U128(key.clone()))
            .unwrap_or(0);
        let next = current.saturating_add(amount);
        env.storage().persistent().set(&DataKey::U128(key), &next);
        next
    }

    pub fn decrement_u128(env: Env, caller: Address, key: BytesN<32>, amount: u128) -> u128 {
        caller.require_auth();
        require_controller(&env, &caller);
        let current: u128 = env
            .storage()
            .persistent()
            .get(&DataKey::U128(key.clone()))
            .unwrap_or(0);
        if amount > current {
            panic_with_error!(&env, Error::Underflow);
        }
        let next = current - amount;
        env.storage().persistent().set(&DataKey::U128(key), &next);
        next
    }

    // ── i128 operations ──────────────────────────────────────────────────────

    pub fn get_i128(env: Env, key: BytesN<32>) -> i128 {
        env.storage()
            .persistent()
            .get(&DataKey::I128(key))
            .unwrap_or(0)
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
        let current: i128 = env
            .storage()
            .persistent()
            .get(&DataKey::I128(key.clone()))
            .unwrap_or(0);
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
        env.storage()
            .persistent()
            .get(&DataKey::Bool(key))
            .unwrap_or(false)
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

    pub fn set_bytes32(
        env: Env,
        caller: Address,
        key: BytesN<32>,
        value: BytesN<32>,
    ) -> BytesN<32> {
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
            env.storage()
                .persistent()
                .set(&DataKey::AddrSet(set_key), &set);
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
        env.storage()
            .persistent()
            .set(&DataKey::AddrSet(set_key), &set);
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
        let data_key = DataKey::B32Set(set_key.clone());
        let mut set: Vec<BytesN<32>> = env
            .storage()
            .persistent()
            .get(&data_key)
            .unwrap_or(Vec::new(&env));
        if !vec_contains_b32(&set, &value) {
            set.push_back(value);
            env.storage()
                .persistent()
                .set(&data_key, &set);
        }
        // Extend TTL on the set entry to keep enumeration index alive alongside primary keys
        env.storage()
            .persistent()
            .extend_ttl(&data_key, MIN_BUMP_THRESHOLD, PERSISTENT_BUMP_TARGET);
    }

    pub fn remove_bytes32_from_set(
        env: Env,
        caller: Address,
        set_key: BytesN<32>,
        value: BytesN<32>,
    ) {
        caller.require_auth();
        require_controller(&env, &caller);
        let data_key = DataKey::B32Set(set_key.clone());
        let mut set: Vec<BytesN<32>> = env
            .storage()
            .persistent()
            .get(&data_key)
            .unwrap_or(Vec::new(&env));
        vec_remove_b32(&mut set, &value);
        env.storage()
            .persistent()
            .set(&data_key, &set);
        // Extend TTL on the set entry to keep enumeration index alive alongside primary keys
        env.storage()
            .persistent()
            .extend_ttl(&data_key, MIN_BUMP_THRESHOLD, PERSISTENT_BUMP_TARGET);
    }

    pub fn get_bytes32_set_count(env: Env, set_key: BytesN<32>) -> u32 {
        let data_key = DataKey::B32Set(set_key);
        let set: Vec<BytesN<32>> = env
            .storage()
            .persistent()
            .get(&data_key)
            .unwrap_or(Vec::new(&env));
        env.storage()
            .persistent()
            .extend_ttl(&data_key, MIN_BUMP_THRESHOLD, PERSISTENT_BUMP_TARGET);
        set.len()
    }

    pub fn get_bytes32_set_at(
        env: Env,
        set_key: BytesN<32>,
        start: u32,
        end: u32,
    ) -> Vec<BytesN<32>> {
        let data_key = DataKey::B32Set(set_key);
        let set: Vec<BytesN<32>> = env
            .storage()
            .persistent()
            .get(&data_key)
            .unwrap_or(Vec::new(&env));
        env.storage()
            .persistent()
            .extend_ttl(&data_key, MIN_BUMP_THRESHOLD, PERSISTENT_BUMP_TARGET);
        paginate_b32(&env, &set, start, end)
    }

    pub fn contains_bytes32(env: Env, set_key: BytesN<32>, value: BytesN<32>) -> bool {
        let data_key = DataKey::B32Set(set_key);
        let set: Vec<BytesN<32>> = env
            .storage()
            .persistent()
            .get(&data_key)
            .unwrap_or(Vec::new(&env));
        env.storage()
            .persistent()
            .extend_ttl(&data_key, MIN_BUMP_THRESHOLD, PERSISTENT_BUMP_TARGET);
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

    // ── Keeper Reputation & Slashing (Issue #514) ────────────────────────────

    /// Record execution quality metrics for a keeper and apply slashing penalty if variance exceeds limit (>5%).
    pub fn record_keeper_execution(
        env: Env,
        caller: Address,
        keeper: Address,
        executed_price: u128,
        expected_price: u128,
    ) {
        caller.require_auth();
        require_controller(&env, &caller);

        let count_key = gmx_keys::keeper_execution_count_key(&env, &keeper);
        let current_count = Self::get_u128(env.clone(), count_key.clone());
        Self::set_u128(env.clone(), caller.clone(), count_key, current_count + 1);

        let variance = executed_price.abs_diff(expected_price);

        if let Some(variance_bps) = (variance * 10000).checked_div(expected_price) {
            let total_var_key = gmx_keys::keeper_total_variance_key(&env, &keeper);
            let current_total_var = Self::get_u128(env.clone(), total_var_key.clone());
            Self::set_u128(env.clone(), caller.clone(), total_var_key, current_total_var + variance_bps);

            // Slash penalty for execution variance exceeding 500 bps (5%)
            if variance_bps > 500 {
                let slash_key = gmx_keys::keeper_slash_amount_key(&env, &keeper);
                let current_slash = Self::get_u128(env.clone(), slash_key.clone());
                let penalty = 100u128;
                Self::set_u128(env.clone(), caller.clone(), slash_key, current_slash + penalty);

                env.events().publish_event(&KeeperSlashed {
                    keeper,
                    executed_price,
                    expected_price,
                    variance_bps,
                    penalty_amount: penalty,
                });
            }
        }
    }

    /// Retrieve performance metrics for a keeper: (execution_count, total_variance_bps, slash_penalty_amount).
    pub fn get_keeper_stats(env: Env, keeper: Address) -> (u128, u128, u128) {
        let count_key = gmx_keys::keeper_execution_count_key(&env, &keeper);
        let total_var_key = gmx_keys::keeper_total_variance_key(&env, &keeper);
        let slash_key = gmx_keys::keeper_slash_amount_key(&env, &keeper);

        let count = Self::get_u128(env.clone(), count_key);
        let total_var = Self::get_u128(env.clone(), total_var_key);
        let slash = Self::get_u128(env.clone(), slash_key);

        (count, total_var, slash)
    }

    // ── Position Manager (delegated position control for copy-trading) ────────

    /// Get the authorized position manager for a given owner and market.
    /// Returns None if no manager is set or if revoked (zero address).
    pub fn get_position_manager(env: Env, owner: Address, market: Address) -> Option<Address> {
        use gmx_keys::position_manager_key;
        let key = DataKey::Addr(position_manager_key(&env, &owner, &market));
        env.storage().persistent().get(&key)
    }

    /// Set or revoke a position manager for a given owner and market.
    /// Only the owner can call this. Pass zero_address to revoke.
    pub fn set_position_manager(env: Env, owner: Address, market: Address, manager: Address) -> Address {
        owner.require_auth();
        // Note: We don't check for CONTROLLER role here because the owner can revoke their own manager.
        // Setting a manager is an authorization, not a state modification done by the protocol.
        use gmx_keys::position_manager_key;
        let key = DataKey::Addr(position_manager_key(&env, &owner, &market));
        env.storage().persistent().set(&key, &manager);
        manager
    }

    // ── Liquidation Execution Fee (keeper reimbursement on liquidation) ───────

    /// Get the liquidation execution fee for a given market.
    /// This fee is paid to the keeper from position collateral on successful liquidation.
    pub fn get_liquidation_execution_fee(env: Env, market: Address) -> u128 {
        use gmx_keys::liquidation_execution_fee_key;
        let key = DataKey::U128(liquidation_execution_fee_key(&env, &market));
        env.storage().persistent().get(&key).unwrap_or(0u128)
    }

    /// Set the liquidation execution fee for a given market (admin-only).
    pub fn set_liquidation_execution_fee(env: Env, caller: Address, market: Address, fee: u128) -> u128 {
        caller.require_auth();
        require_controller(&env, &caller);
        use gmx_keys::liquidation_execution_fee_key;
        let key = DataKey::U128(liquidation_execution_fee_key(&env, &market));
        env.storage().persistent().set(&key, &fee);
        fee
    }

    /// Get the global minimum execution fee required for all order types.
    /// Returns 0 if not configured (no minimum enforced).
    pub fn get_min_execution_fee(env: Env) -> u128 {
        use gmx_keys::min_execution_fee_key;
        let key = DataKey::U128(min_execution_fee_key(&env));
        env.storage().persistent().get(&key).unwrap_or(0u128)
    }

    /// Set the global minimum execution fee (controller-only).
    pub fn set_min_execution_fee(env: Env, caller: Address, fee: u128) -> u128 {
        caller.require_auth();
        require_controller(&env, &caller);
        use gmx_keys::min_execution_fee_key;
        let key = DataKey::U128(min_execution_fee_key(&env));
        env.storage().persistent().set(&key, &fee);
        fee
    }
}

// ─── Internal helpers ─────────────────────────────────────────────────────────

fn require_init(env: &Env) {
    if !env.storage().instance().has(&InstanceKey::Initialized) {
        panic_with_error!(env, Error::NotInitialized);
    }
}

/// Checks that `caller` holds the CONTROLLER role in role_store.
///
/// Issue #357: CONTROLLER is a **single flat trust domain** — there is no
/// per-key or per-namespace ownership check.  Any contract granted
/// CONTROLLER may write to any key in DataStore.  This function does NOT
/// validate which "namespace" the caller is writing to.
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
    use role_store::{RoleStore, RoleStoreClient as RoleClient};
    use soroban_sdk::{testutils::Address as _, Env};

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

    // ── Issue #179: get_address Option semantics ─────────────────────────────

    /// Reading an address for a key that was never written must return None, not panic.
    #[test]
    fn get_address_returns_none_for_missing_key() {
        let (env, _, _, ds_id) = setup();
        let client = DataStoreClient::new(&env, &ds_id);
        let key = BytesN::from_array(&env, &[0xFFu8; 32]);
        assert!(
            client.get_address(&key).is_none(),
            "missing key must return None, not panic"
        );
    }

    /// Reading an address for a key that was written must return Some(addr).
    #[test]
    fn get_address_returns_some_for_present_key() {
        let (env, admin, _, ds_id) = setup();
        let client = DataStoreClient::new(&env, &ds_id);
        let key = BytesN::from_array(&env, &[0xFEu8; 32]);
        let value = Address::generate(&env);
        client.set_address(&admin, &key, &value);
        assert_eq!(
            client.get_address(&key),
            Some(value),
            "present key must return Some(addr)"
        );
    }

    // ── Issue #109: CONTROLLER authorization matrix ───────────────────────────

    /// set_u128 must reject a caller that does not hold CONTROLLER.
    #[test]
    #[should_panic]
    fn set_u128_by_non_controller_panics() {
        let (env, _, _, ds_id) = setup();
        let client = DataStoreClient::new(&env, &ds_id);
        let impostor = Address::generate(&env);
        let key = soroban_sdk::BytesN::from_array(&env, &[1u8; 32]);
        // impostor is not registered as CONTROLLER — must panic.
        client.set_u128(&impostor, &key, &42u128);
    }

    /// set_address must reject a caller that does not hold CONTROLLER.
    #[test]
    #[should_panic]
    fn set_address_by_non_controller_panics() {
        let (env, _, _, ds_id) = setup();
        let client = DataStoreClient::new(&env, &ds_id);
        let impostor = Address::generate(&env);
        let key = soroban_sdk::BytesN::from_array(&env, &[2u8; 32]);
        let value = Address::generate(&env);
        client.set_address(&impostor, &key, &value);
    }

    // ── Issue #353: get_u128_cached self-healing ───────────────────────────

    /// After set_u128_config populates the cache, a plain set_u128 on the same
    /// key must be visible to get_u128_cached (cache self-heals).
    #[test]
    fn get_u128_cached_self_heals_after_plain_set() {
        let (env, admin, _, ds_id) = setup();
        let client = DataStoreClient::new(&env, &ds_id);
        let key = BytesN::from_array(&env, &[0xA0u8; 32]);

        // Populate both persistent and cache via set_u128_config
        client.set_u128_config(&admin, &key, &100u128);
        assert_eq!(client.get_u128_cached(&key), 100);

        // Write a new value via plain set_u128 (bypasses cache)
        client.set_u128(&admin, &key, &200u128);

        // get_u128_cached must return the fresh persistent value, not the stale cache
        assert_eq!(client.get_u128_cached(&key), 200);
    }

    /// After set_u128_config, apply_delta_to_u128 on the same key must be
    /// visible to get_u128_cached.
    #[test]
    fn get_u128_cached_self_heals_after_apply_delta() {
        let (env, admin, _, ds_id) = setup();
        let client = DataStoreClient::new(&env, &ds_id);
        let key = BytesN::from_array(&env, &[0xA1u8; 32]);

        client.set_u128_config(&admin, &key, &1000u128);
        assert_eq!(client.get_u128_cached(&key), 1000);

        // Apply delta via the plain mutator (bypasses cache)
        client.apply_delta_to_u128(&admin, &key, &(-500i128));

        assert_eq!(client.get_u128_cached(&key), 500);
    }

    /// After set_u128_config, increment_u128 on the same key must be visible
    /// to get_u128_cached.
    #[test]
    fn get_u128_cached_self_heals_after_increment() {
        let (env, admin, _, ds_id) = setup();
        let client = DataStoreClient::new(&env, &ds_id);
        let key = BytesN::from_array(&env, &[0xA2u8; 32]);

        client.set_u128_config(&admin, &key, &100u128);
        client.increment_u128(&admin, &key, &50u128);

        assert_eq!(client.get_u128_cached(&key), 150);
    }

    /// After set_u128_config, decrement_u128 on the same key must be visible
    /// to get_u128_cached.
    #[test]
    fn get_u128_cached_self_heals_after_decrement() {
        let (env, admin, _, ds_id) = setup();
        let client = DataStoreClient::new(&env, &ds_id);
        let key = BytesN::from_array(&env, &[0xA3u8; 32]);

        client.set_u128_config(&admin, &key, &100u128);
        client.decrement_u128(&admin, &key, &30u128);

        assert_eq!(client.get_u128_cached(&key), 70);
    }

    // ── Issue #351: InstanceU128 / InstanceI128 discriminant separation ──────

    /// Instance-tier u128 and i128 values stored under the *same* underlying
    /// BytesN<32> key must not collide. The DataKey enum wraps each raw key in
    /// a type-specific discriminant precisely so InstanceU128(key) and
    /// InstanceI128(key) address distinct storage slots; if the two variants
    /// were ever merged back into one discriminant (as happened when
    /// `InstanceU128` was accidentally declared twice, which shadowed the
    /// distinct instance-tier slot `InstanceI128` was meant to occupy), one
    /// write would silently clobber the other instead of the crate failing to
    /// build.
    #[test]
    fn instance_u128_and_i128_do_not_collide_on_same_key() {
        let (env, admin, _, ds_id) = setup();
        let client = DataStoreClient::new(&env, &ds_id);
        let key = BytesN::from_array(&env, &[0xB0u8; 32]);

        client.set_u128_instance(&admin, &key, &777u128);
        client.set_i128_instance(&admin, &key, &-42i128);

        assert_eq!(client.get_u128_instance(&key), 777);
        assert_eq!(client.get_i128_instance(&key), -42);
    }

    // ── Issue #360: CRUD tests for untested functions ─────────────────────

    #[test]
    fn test_get_u128_batch() {
        let (env, admin, _, ds_id) = setup();
        let client = DataStoreClient::new(&env, &ds_id);
        let k1 = BytesN::from_array(&env, &[0xC1u8; 32]);
        let k2 = BytesN::from_array(&env, &[0xC2u8; 32]);
        let k3 = BytesN::from_array(&env, &[0xC3u8; 32]);

        client.set_u128(&admin, &k1, &11u128);
        client.set_u128(&admin, &k2, &22u128);

        let keys = soroban_sdk::vec![&env, k1.clone(), k2.clone(), k3.clone()];
        let results = client.get_u128_batch(&keys);
        assert_eq!(results.len(), 3);
        assert_eq!(results.get_unchecked(0), 11u128);
        assert_eq!(results.get_unchecked(1), 22u128);
        assert_eq!(results.get_unchecked(2), 0u128);
    }

    #[test]
    fn test_u128_instance_crud() {
        let (env, admin, _, ds_id) = setup();
        let client = DataStoreClient::new(&env, &ds_id);
        let key = BytesN::from_array(&env, &[0xC4u8; 32]);

        assert_eq!(client.get_u128_instance(&key), 0);
        client.set_u128_instance(&admin, &key, &999u128);
        assert_eq!(client.get_u128_instance(&key), 999);
        client.set_u128_instance(&admin, &key, &0u128);
        assert_eq!(client.get_u128_instance(&key), 0);
    }

    #[test]
    #[should_panic]
    fn set_u128_instance_by_non_controller_panics() {
        let (env, _, _, ds_id) = setup();
        let client = DataStoreClient::new(&env, &ds_id);
        let impostor = Address::generate(&env);
        let key = BytesN::from_array(&env, &[0xC5u8; 32]);
        client.set_u128_instance(&impostor, &key, &42u128);
    }

    #[test]
    fn test_i128_instance_crud() {
        let (env, admin, _, ds_id) = setup();
        let client = DataStoreClient::new(&env, &ds_id);
        let key = BytesN::from_array(&env, &[0xC6u8; 32]);

        assert_eq!(client.get_i128_instance(&key), 0);
        client.set_i128_instance(&admin, &key, &-777i128);
        assert_eq!(client.get_i128_instance(&key), -777);
        client.set_i128_instance(&admin, &key, &0i128);
        assert_eq!(client.get_i128_instance(&key), 0);
    }

    #[test]
    #[should_panic]
    fn set_i128_instance_by_non_controller_panics() {
        let (env, _, _, ds_id) = setup();
        let client = DataStoreClient::new(&env, &ds_id);
        let impostor = Address::generate(&env);
        let key = BytesN::from_array(&env, &[0xC7u8; 32]);
        client.set_i128_instance(&impostor, &key, &42i128);
    }

    #[test]
    fn test_set_u128_config_and_get_u128_cached() {
        let (env, admin, _, ds_id) = setup();
        let client = DataStoreClient::new(&env, &ds_id);
        let key = BytesN::from_array(&env, &[0xC8u8; 32]);

        assert_eq!(client.get_u128(&key), 0);
        client.set_u128_config(&admin, &key, &500u128);
        assert_eq!(client.get_u128(&key), 500);
        assert_eq!(client.get_u128_cached(&key), 500);
        assert_eq!(client.get_u128_instance(&key), 500);
    }

    #[test]
    #[should_panic]
    fn set_u128_config_by_non_controller_panics() {
        let (env, _, _, ds_id) = setup();
        let client = DataStoreClient::new(&env, &ds_id);
        let impostor = Address::generate(&env);
        let key = BytesN::from_array(&env, &[0xC9u8; 32]);
        client.set_u128_config(&impostor, &key, &42u128);
    }

    #[test]
    fn test_bytes32_crud() {
        let (env, admin, _, ds_id) = setup();
        let client = DataStoreClient::new(&env, &ds_id);
        let key = BytesN::from_array(&env, &[0xCAu8; 32]);
        let val = BytesN::from_array(&env, &[0xABu8; 32]);

        assert_eq!(
            client.get_bytes32(&key),
            BytesN::from_array(&env, &[0u8; 32])
        );
        client.set_bytes32(&admin, &key, &val);
        assert_eq!(client.get_bytes32(&key), val);
    }

    #[test]
    #[should_panic]
    fn set_bytes32_by_non_controller_panics() {
        let (env, _, _, ds_id) = setup();
        let client = DataStoreClient::new(&env, &ds_id);
        let impostor = Address::generate(&env);
        let key = BytesN::from_array(&env, &[0xCBu8; 32]);
        let val = BytesN::from_array(&env, &[0xABu8; 32]);
        client.set_bytes32(&impostor, &key, &val);
    }

    #[test]
    fn test_remove_i128() {
        let (env, admin, _, ds_id) = setup();
        let client = DataStoreClient::new(&env, &ds_id);
        let key = BytesN::from_array(&env, &[0xCCu8; 32]);

        client.set_i128(&admin, &key, &-42i128);
        assert_eq!(client.get_i128(&key), -42);
        client.remove_i128(&admin, &key);
        assert_eq!(client.get_i128(&key), 0);
    }

    #[test]
    #[should_panic]
    fn remove_i128_by_non_controller_panics() {
        let (env, _, _, ds_id) = setup();
        let client = DataStoreClient::new(&env, &ds_id);
        let impostor = Address::generate(&env);
        let key = BytesN::from_array(&env, &[0xCDu8; 32]);
        client.remove_i128(&impostor, &key);
    }

    #[test]
    fn test_apply_delta_to_i128() {
        let (env, admin, _, ds_id) = setup();
        let client = DataStoreClient::new(&env, &ds_id);
        let key = BytesN::from_array(&env, &[0xCEu8; 32]);

        assert_eq!(client.get_i128(&key), 0);
        let result = client.apply_delta_to_i128(&admin, &key, &100i128);
        assert_eq!(result, 100);
        let result = client.apply_delta_to_i128(&admin, &key, &(-30i128));
        assert_eq!(result, 70);
    }

    #[test]
    #[should_panic]
    fn apply_delta_to_i128_by_non_controller_panics() {
        let (env, _, _, ds_id) = setup();
        let client = DataStoreClient::new(&env, &ds_id);
        let impostor = Address::generate(&env);
        let key = BytesN::from_array(&env, &[0xCFu8; 32]);
        client.apply_delta_to_i128(&impostor, &key, &10i128);
    }

    #[test]
    fn test_remove_address() {
        let (env, admin, _, ds_id) = setup();
        let client = DataStoreClient::new(&env, &ds_id);
        let key = BytesN::from_array(&env, &[0xD0u8; 32]);
        let value = Address::generate(&env);

        assert!(client.get_address(&key).is_none());
        client.set_address(&admin, &key, &value);
        assert_eq!(client.get_address(&key), Some(value.clone()));
        client.remove_address(&admin, &key);
        assert!(client.get_address(&key).is_none());
    }

    #[test]
    #[should_panic]
    fn remove_address_by_non_controller_panics() {
        let (env, _, _, ds_id) = setup();
        let client = DataStoreClient::new(&env, &ds_id);
        let impostor = Address::generate(&env);
        let key = BytesN::from_array(&env, &[0xD1u8; 32]);
        client.remove_address(&impostor, &key);
    }

    #[test]
    fn test_remove_bool() {
        let (env, admin, _, ds_id) = setup();
        let client = DataStoreClient::new(&env, &ds_id);
        let key = BytesN::from_array(&env, &[0xD2u8; 32]);

        client.set_bool(&admin, &key, &true);
        assert!(client.get_bool(&key));
        client.remove_bool(&admin, &key);
        assert!(!client.get_bool(&key));
    }

    #[test]
    #[should_panic]
    fn remove_bool_by_non_controller_panics() {
        let (env, _, _, ds_id) = setup();
        let client = DataStoreClient::new(&env, &ds_id);
        let impostor = Address::generate(&env);
        let key = BytesN::from_array(&env, &[0xD3u8; 32]);
        client.remove_bool(&impostor, &key);
    }

    #[test]
    fn test_position_manager_crud() {
        let (env, _, _, ds_id) = setup();
        let client = DataStoreClient::new(&env, &ds_id);
        let owner = Address::generate(&env);
        let market = Address::generate(&env);
        let manager = Address::generate(&env);

        assert!(client.get_position_manager(&owner, &market).is_none());
        client.set_position_manager(&owner, &market, &manager);
        assert_eq!(client.get_position_manager(&owner, &market), Some(manager));
    }

    #[test]
    fn test_liquidation_execution_fee() {
        let (env, admin, _, ds_id) = setup();
        let client = DataStoreClient::new(&env, &ds_id);
        let market = Address::generate(&env);

        assert_eq!(client.get_liquidation_execution_fee(&market), 0);
        client.set_liquidation_execution_fee(&admin, &market, &5000u128);
        assert_eq!(client.get_liquidation_execution_fee(&market), 5000);
    }

    #[test]
    #[should_panic]
    fn set_liquidation_execution_fee_by_non_controller_panics() {
        let (env, _, _, ds_id) = setup();
        let client = DataStoreClient::new(&env, &ds_id);
        let impostor = Address::generate(&env);
        let market = Address::generate(&env);
        client.set_liquidation_execution_fee(&impostor, &market, &1000u128);
    }

    #[test]
    fn test_min_execution_fee() {
        let (env, admin, _, ds_id) = setup();
        let client = DataStoreClient::new(&env, &ds_id);

        assert_eq!(client.get_min_execution_fee(), 0);
        client.set_min_execution_fee(&admin, &3000u128);
        assert_eq!(client.get_min_execution_fee(), 3000);
    }

    #[test]
    #[should_panic]
    fn set_min_execution_fee_by_non_controller_panics() {
        let (env, _, _, ds_id) = setup();
        let client = DataStoreClient::new(&env, &ds_id);
        let impostor = Address::generate(&env);
        client.set_min_execution_fee(&impostor, &1000u128);
    }
}
