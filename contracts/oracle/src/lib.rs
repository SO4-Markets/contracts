//! Oracle — keeper-fed price store.
//!
//! Mirrors GMX's Oracle.sol model:
//!   - Authorized keepers submit signed `(token, min_price, max_price, timestamp)` bundles
//!     before each execution call.
//!   - Prices live in **temporary** storage and auto-expire after one ledger.
//!   - Consumers call `get_primary_price(token)` to read the current price.
//!   - Stablecoin prices can be pinned in `data_store` (stable_price_key) and
//!     returned via `get_stable_price`.
//!
//! Signature scheme (ed25519):
//!   message = sha256(network_passphrase ‖ ledger_sequence (u32 BE) ‖ token_strkey
//!                    ‖ min_price (16-byte BE) ‖ max_price (16-byte BE) ‖ timestamp (8-byte BE))
//!   The oracle stores keeper public keys in `data_store` under `keeper_public_key_prefix`.
//!   Keys are stored as: keeper_public_key_prefix ‖ sha256(pubkey_bytes) → BytesN<32> pubkey prefix.
//!   We use a simple approach: keepers are registered by index (u32), stored directly.
#![no_std]
#![allow(dependency_on_unit_never_type_fallback)]

use gmx_keys::{keeper_public_key_prefix, stable_price_key, market_list_key, market_index_token_key, market_long_token_key, market_short_token_key};
use gmx_types::{PriceProps, TokenPrice};
use soroban_sdk::{
    contract, contracterror, contractimpl, contracttype, panic_with_error, symbol_short, Address,
    Bytes, BytesN, Env, Vec,
};

// ─── Errors ───────────────────────────────────────────────────────────────────

#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
#[repr(u32)]
pub enum Error {
    NotInitialized = 1,
    AlreadyInitialized = 2,
    Unauthorized = 3,
    InvalidPrice = 4, // min > max or zero
    StalePrice = 5,   // timestamp too old
    PriceNotFound = 6,
    InvalidSignature = 7,
    NoKeepers = 8,
}

// ─── Storage keys ─────────────────────────────────────────────────────────────

#[contracttype]
enum InstanceKey {
    Initialized,
    Admin,
    RoleStore,
    DataStore,
    NetworkPassphrase,
}

#[contracttype]
enum TempKey {
    Price(Address),
}

/// Ledgers to keep a submitted price readable in temporary storage.
///
/// `set_prices` and `execute_*` run in **separate** transactions, and a keeper
/// may drain a batch of pending orders one-by-one after a single price set.
/// Bumping the temp TTL keeps prices readable across that window so later
/// executions in the batch don't revert with `PriceNotFound`. Kept short so
/// prices remain ephemeral (≈10 min at ~5s/ledger), in line with the 300s /
/// 60-ledger freshness window enforced at submission time.
const PRICE_TTL_LEDGERS: u32 = 120;

// ─── Signed price submitted by keeper ────────────────────────────────────────

/// One signed price attestation from a keeper.
#[contracttype]
pub struct SignedPrice {
    pub token: Address,
    pub min_price: i128,
    pub max_price: i128,
    pub timestamp: u64,
    /// ed25519 signature over the canonical message (64 bytes)
    pub signature: BytesN<64>,
    /// Index of the keeper's public key in data_store (0-based)
    pub keeper_index: u32,
    /// Ledger sequence at which the keeper signed this price.
    /// Must be within LEDGER_SEQ_WINDOW of the current ledger.
    pub ledger_seq: u32,
}

#[contracttype]
pub struct StoredPrice {
    pub min: i128,
    pub max: i128,
    pub ledger_seq: u32,
}

// ─── Cross-contract clients ───────────────────────────────────────────────────

#[allow(dead_code)]
#[soroban_sdk::contractclient(name = "RoleStoreClient")]
trait IRoleStore {
    fn has_role(env: Env, account: Address, role: BytesN<32>) -> bool;
}

#[allow(dead_code)]
#[soroban_sdk::contractclient(name = "DataStoreClient")]
trait IDataStore {
    fn get_u128(env: Env, key: BytesN<32>) -> u128;
    fn get_bytes32(env: Env, key: BytesN<32>) -> BytesN<32>;
    fn get_address(env: Env, key: BytesN<32>) -> Option<Address>;
    fn get_address_set_count(env: Env, set_key: BytesN<32>) -> u32;
    fn get_address_set_at(env: Env, set_key: BytesN<32>, start: u32, end: u32) -> Vec<Address>;
    fn set_bool(env: Env, caller: Address, key: BytesN<32>, value: bool) -> bool;
    fn set_bytes32(env: Env, caller: Address, key: BytesN<32>, value: BytesN<32>);
}

#[contracttype]
pub struct CircuitBreakerTripped {
    pub market: Address,
    pub token: Address,
    pub old_price: i128,
    pub new_price: i128,
    pub deviation_bps: u128,
}

#[contracttype]
pub struct OracleSignerRotated {
    pub keeper_index: u32,
    pub old_signer: BytesN<32>,
    pub new_signer: BytesN<32>,
}

// ─── Contract ─────────────────────────────────────────────────────────────────

#[contract]
pub struct Oracle;

#[contractimpl]
impl Oracle {
    // ── Init ─────────────────────────────────────────────────────────────────

    pub fn initialize(
        env: Env,
        admin: Address,
        role_store: Address,
        data_store: Address,
        network_passphrase: Bytes,
    ) {
        admin.require_auth();
        if env.storage().instance().has(&InstanceKey::Initialized) {
            panic_with_error!(&env, Error::AlreadyInitialized);
        }
        env.storage()
            .instance()
            .set(&InstanceKey::Initialized, &true);
        env.storage().instance().set(&InstanceKey::Admin, &admin);
        env.storage()
            .instance()
            .set(&InstanceKey::RoleStore, &role_store);
        env.storage()
            .instance()
            .set(&InstanceKey::DataStore, &data_store);
        env.storage()
            .instance()
            .set(&InstanceKey::NetworkPassphrase, &network_passphrase);
    }

    // ── Upgrade ──────────────────────────────────────────────────────────────

    pub fn upgrade(env: Env, new_wasm_hash: BytesN<32>) {
        let admin: Address = env
            .storage()
            .instance()
            .get(&InstanceKey::Admin)
            .unwrap_or_else(|| panic_with_error!(&env, Error::NotInitialized));
        admin.require_auth();
        env.deployer().update_current_contract_wasm(new_wasm_hash);
    }

    // ── Keeper price submission ───────────────────────────────────────────────

    /// Submit a batch of keeper-signed prices.
    ///
    /// Each price is individually signature-verified against the registered
    /// keeper public key at `keeper_index`. The caller must have ORDER_KEEPER role.
    pub fn set_prices(env: Env, caller: Address, prices: Vec<SignedPrice>) {
        caller.require_auth();
        require_order_keeper(&env, &caller);

        let passphrase: Bytes = env
            .storage()
            .instance()
            .get(&InstanceKey::NetworkPassphrase)
            .unwrap();
        let data_store: Address = env
            .storage()
            .instance()
            .get(&InstanceKey::DataStore)
            .unwrap();
        // Allow prices signed up to ~5 minutes ago (5s/ledger × 60 = 60 ledgers).
        const LEDGER_SEQ_WINDOW: u32 = 60;
        let current_seq = env.ledger().sequence();

        for i in 0..prices.len() {
            let sp = prices.get(i).unwrap();

            // Basic validation
            if sp.min_price <= 0 || sp.max_price <= 0 || sp.min_price > sp.max_price {
                panic_with_error!(&env, Error::InvalidPrice);
            }

            // Timestamp must be within 5 minutes of current ledger time
            let now = env.ledger().timestamp();
            let age = now.saturating_sub(sp.timestamp);
            if age > 300 {
                panic_with_error!(&env, Error::StalePrice);
            }

            // keeper_ledger_seq must be within LEDGER_SEQ_WINDOW of current.
            // is_seq_stale uses u64 arithmetic to safely handle u32 wrap-around.
            if is_seq_stale(current_seq, sp.ledger_seq, LEDGER_SEQ_WINDOW) {
                panic_with_error!(&env, Error::StalePrice);
            }

            // Verify ed25519 signature using the keeper-provided ledger_seq
            let msg = build_price_message(
                &env,
                &passphrase,
                sp.ledger_seq,
                &sp.token,
                sp.min_price,
                sp.max_price,
                sp.timestamp,
            );
            let pubkey = get_keeper_pubkey(&env, &data_store, sp.keeper_index);
            // ed25519_verify takes (&BytesN<32> pubkey, &Bytes message, &BytesN<64> sig)
            env.crypto().ed25519_verify(&pubkey, &msg, &sp.signature);

            // Check circuit breaker before overwriting price
            check_circuit_breaker(&env, &data_store, &sp.token, sp.min_price, sp.max_price);

            // Store in temporary storage and bump its TTL so the price survives
            // the keeper's set_prices → execute_* batch window (see PRICE_TTL_LEDGERS).
            let stored = StoredPrice {
                min: sp.min_price,
                max: sp.max_price,
                ledger_seq: sp.ledger_seq,
            };
            let price_key = TempKey::Price(sp.token.clone());
            env.storage().temporary().set(&price_key, &stored);
            env.storage()
                .temporary()
                .extend_ttl(&price_key, PRICE_TTL_LEDGERS, PRICE_TTL_LEDGERS);
        }

        env.events()
            .publish((symbol_short!("prices"),), (caller, prices.len()));
    }

    // ── Price reads ───────────────────────────────────────────────────────────

    /// Returns the current price for a token. Panics if not set this execution.
    pub fn get_primary_price(env: Env, token: Address) -> PriceProps {
        let stored: StoredPrice = env
            .storage()
            .temporary()
            .get::<TempKey, StoredPrice>(&TempKey::Price(token))
            .unwrap_or_else(|| panic_with_error!(&env, Error::PriceNotFound));
        PriceProps {
            min: stored.min,
            max: stored.max,
        }
    }

    /// Returns the price for a token, or None if not set.
    pub fn try_get_price(env: Env, token: Address) -> Option<PriceProps> {
        env.storage()
            .temporary()
            .get::<TempKey, StoredPrice>(&TempKey::Price(token))
            .map(|s| PriceProps { min: s.min, max: s.max })
    }

    /// Returns pinned stable price from data_store, or None if not configured.
    pub fn get_stable_price(env: Env, token: Address) -> Option<i128> {
        let data_store: Address = env
            .storage()
            .instance()
            .get(&InstanceKey::DataStore)
            .unwrap();
        let key = stable_price_key(&env, &token);
        let price = DataStoreClient::new(&env, &data_store).get_u128(&key) as i128;
        if price == 0 {
            None
        } else {
            Some(price)
        }
    }

    /// Convenience: returns stable price if available, otherwise primary price.
    pub fn get_price_with_stable_fallback(env: Env, token: Address) -> PriceProps {
        let data_store: Address = env
            .storage()
            .instance()
            .get(&InstanceKey::DataStore)
            .unwrap();
        let key = stable_price_key(&env, &token);
        let stable = DataStoreClient::new(&env, &data_store).get_u128(&key) as i128;
        if stable > 0 {
            return PriceProps {
                min: stable,
                max: stable,
            };
        }
        let stored: StoredPrice = env
            .storage()
            .temporary()
            .get::<TempKey, StoredPrice>(&TempKey::Price(token))
            .unwrap_or_else(|| panic_with_error!(&env, Error::PriceNotFound));
        PriceProps {
            min: stored.min,
            max: stored.max,
        }
    }

    /// Require that the stored price for `token` was signed for `expected_ledger_seq`.
    /// Panics with `StalePrice` if the stored ledger sequence differs.
    pub fn require_price_fresh(env: Env, token: Address, expected_ledger_seq: u32) -> PriceProps {
        let stored: StoredPrice = env
            .storage()
            .temporary()
            .get::<TempKey, StoredPrice>(&TempKey::Price(token))
            .unwrap_or_else(|| panic_with_error!(&env, Error::PriceNotFound));
        if stored.ledger_seq != expected_ledger_seq {
            panic_with_error!(&env, Error::StalePrice);
        }
        PriceProps {
            min: stored.min,
            max: stored.max,
        }
    }

    // ── Cleanup ───────────────────────────────────────────────────────────────

    /// Clear a specific token price from temporary storage.
    pub fn clear_price(env: Env, caller: Address, token: Address) {
        caller.require_auth();
        require_order_keeper(&env, &caller);
        env.storage().temporary().remove(&TempKey::Price(token));
    }

    /// Clear multiple token prices at once (called by keeper after execution).
    pub fn clear_prices(env: Env, caller: Address, tokens: Vec<Address>) {
        caller.require_auth();
        require_order_keeper(&env, &caller);
        for i in 0..tokens.len() {
            let token = tokens.get(i).unwrap();
            env.storage().temporary().remove(&TempKey::Price(token));
        }
    }

    // ── Signer rotation ───────────────────────────────────────────────────────

    /// Atomically replace a keeper's ed25519 public key in data_store.
    ///
    /// Reads the current key at `keeper_index`, writes `new_pubkey` in its place,
    /// and emits `OracleSignerRotated`. Any price bundle signed by the old key
    /// that has not yet been submitted will be rejected once the key is replaced.
    /// Requires the caller to be the contract admin.
    pub fn rotate_signer(env: Env, caller: Address, keeper_index: u32, new_pubkey: BytesN<32>) {
        caller.require_auth();
        require_admin(&env, &caller);

        let data_store: Address = env
            .storage()
            .instance()
            .get(&InstanceKey::DataStore)
            .unwrap_or_else(|| panic_with_error!(&env, Error::NotInitialized));

        let key = keeper_pubkey_storage_key(&env, keeper_index);
        let old_pubkey = DataStoreClient::new(&env, &data_store).get_bytes32(&key);
        DataStoreClient::new(&env, &data_store).set_bytes32(&caller, &key, &new_pubkey);

        env.events().publish(
            (symbol_short!("sig_rot"),),
            OracleSignerRotated {
                keeper_index,
                old_signer: old_pubkey,
                new_signer: new_pubkey,
            },
        );
    }
}

// ─── Test-only price submission ────────────────────────────────────────────────
//
// Kept in a separate, feature-gated `#[contractimpl]` block so the generated
// invoke wrapper is also gated. Inlining a `#[cfg(...)]` method in the main impl
// makes the macro emit a wrapper that references the stripped fn in non-test
// builds, which fails to compile under the current SDK.

#[cfg(any(test, feature = "testutils"))]
#[contractimpl]
impl Oracle {
    /// Submit prices without signature verification.
    ///
    /// Simpler path: caller must have ORDER_KEEPER role, no ed25519 required.
    /// Suitable for local/test environments where keepers are fully trusted.
    pub fn set_prices_simple(env: Env, caller: Address, prices: Vec<TokenPrice>) {
        caller.require_auth();
        require_order_keeper(&env, &caller);

        let data_store: Address = env
            .storage()
            .instance()
            .get(&InstanceKey::DataStore)
            .unwrap_or_else(|| panic_with_error!(&env, Error::NotInitialized));

        for i in 0..prices.len() {
            let tp = prices.get(i).unwrap();
            if tp.min <= 0 || tp.max <= 0 || tp.min > tp.max {
                panic_with_error!(&env, Error::InvalidPrice);
            }

            check_circuit_breaker(&env, &data_store, &tp.token, tp.min, tp.max);

            let stored = StoredPrice {
                min: tp.min,
                max: tp.max,
                ledger_seq: env.ledger().sequence(),
            };
            let price_key = TempKey::Price(tp.token.clone());
            env.storage().temporary().set(&price_key, &stored);
            env.storage()
                .temporary()
                .extend_ttl(&price_key, PRICE_TTL_LEDGERS, PRICE_TTL_LEDGERS);
        }
    }
}

// ─── Internal helpers ─────────────────────────────────────────────────────────

/// Returns true if the submitted ledger sequence is too old (or from the future).
///
/// Casts both values to u64 so the subtraction is always safe: when `submitted > current`
/// (which happens after a u32 sequence wrap-around where a pre-wrap sequence numerically
/// exceeds the post-wrap current) the first branch triggers and we treat it as stale.
fn is_seq_stale(current: u32, submitted: u32, window: u32) -> bool {
    let c = current as u64;
    let s = submitted as u64;
    s > c || (c - s) > window as u64
}

fn require_order_keeper(env: &Env, caller: &Address) {
    let role_store: Address = env
        .storage()
        .instance()
        .get(&InstanceKey::RoleStore)
        .unwrap();
    let role = gmx_keys::roles::order_keeper(env);
    if !RoleStoreClient::new(env, &role_store).has_role(caller, &role) {
        panic_with_error!(env, Error::Unauthorized);
    }
}

fn require_admin(env: &Env, caller: &Address) {
    let admin: Address = env
        .storage()
        .instance()
        .get(&InstanceKey::Admin)
        .unwrap_or_else(|| panic_with_error!(env, Error::NotInitialized));
    if *caller != admin {
        panic_with_error!(env, Error::Unauthorized);
    }
}

/// Compute the data_store key for a keeper public key slot by index.
fn keeper_pubkey_storage_key(env: &Env, index: u32) -> BytesN<32> {
    let mut buf = Bytes::new(env);
    let prefix = keeper_public_key_prefix(env);
    buf.extend_from_array(&prefix.to_array());
    buf.extend_from_array(&index.to_be_bytes());
    env.crypto().sha256(&buf).into()
}

/// Retrieve ed25519 public key for a keeper by index from data_store.
///
/// Keys are stored as 32 bytes at key = sha256("KEEPER_PUBLIC_KEY" ‖ index_u32_BE).
/// We pack two consecutive BytesN<32> to form the full 32-byte ed25519 pubkey.
/// For simplicity we store the key at (prefix ‖ index) and read 32 bytes.
fn get_keeper_pubkey(env: &Env, data_store: &Address, index: u32) -> BytesN<32> {
    let key = keeper_pubkey_storage_key(env, index);
    DataStoreClient::new(env, data_store).get_bytes32(&key)
}

/// Build the canonical message that keepers sign.
///
/// message = passphrase ‖ ledger_seq (4 BE) ‖ token_strkey ‖ min (16 BE) ‖ max (16 BE) ‖ ts (8 BE)
///
/// ed25519_verify takes a raw Bytes message (not pre-hashed); the SDK hashes internally.
fn build_price_message(
    env: &Env,
    passphrase: &Bytes,
    ledger_seq: u32,
    token: &Address,
    min_price: i128,
    max_price: i128,
    timestamp: u64,
) -> Bytes {
    let mut buf = Bytes::new(env);

    buf.append(passphrase);
    buf.extend_from_array(&ledger_seq.to_be_bytes());

    let token_str: soroban_sdk::String = token.to_string();
    let token_bytes: Bytes = token_str.into();
    buf.append(&token_bytes);

    buf.extend_from_array(&min_price.to_be_bytes());
    buf.extend_from_array(&max_price.to_be_bytes());
    buf.extend_from_array(&timestamp.to_be_bytes());

    buf
}

fn check_circuit_breaker(env: &Env, data_store: &Address, token: &Address, new_min: i128, new_max: i128) {
    let price_key = TempKey::Price(token.clone());
    let prev_price_opt = env.storage().temporary().get::<TempKey, StoredPrice>(&price_key);

    if let Some(prev_price) = prev_price_opt {
        let last_price = (prev_price.min + prev_price.max) / 2;
        let new_price = (new_min + new_max) / 2;
        if last_price > 0 {
            let deviation_val = (new_price - last_price).abs();
            let deviation_bps = ((deviation_val as u128) * 10000) / (last_price as u128);

            let ds = DataStoreClient::new(env, data_store);
            let market_list_k = gmx_keys::market_list_key(env);
            let market_count = ds.get_address_set_count(&market_list_k);
            let markets = ds.get_address_set_at(&market_list_k, &0, &market_count);

            for i in 0..markets.len() {
                let market = markets.get(i).unwrap();
                let index_token = ds.get_address(&gmx_keys::market_index_token_key(env, &market));
                let long_token = ds.get_address(&gmx_keys::market_long_token_key(env, &market));
                let short_token = ds.get_address(&gmx_keys::market_short_token_key(env, &market));

                let matches_market = (index_token.is_some() && index_token.unwrap() == *token)
                    || (long_token.is_some() && long_token.unwrap() == *token)
                    || (short_token.is_some() && short_token.unwrap() == *token);

                if matches_market {
                    let threshold = ds.get_u128(&gmx_keys::circuit_breaker_factor_key(env, &market));
                    if threshold > 0 && deviation_bps > threshold {
                        // Set market pause flag to true
                        ds.set_bool(&env.current_contract_address(), &gmx_keys::is_market_paused_key(env, &market), &true);

                        // Emit event
                        env.events().publish(
                            (soroban_sdk::symbol_short!("cb_trip"),),
                            CircuitBreakerTripped {
                                market: market.clone(),
                                token: token.clone(),
                                old_price: last_price,
                                new_price,
                                deviation_bps,
                            }
                        );
                    }
                }
            }
        }
    }
}

// ─── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use data_store::{DataStore, DataStoreClient as DsClient};
    use gmx_keys::roles;
    use role_store::{RoleStore, RoleStoreClient as RsClient};
    use soroban_sdk::{testutils::Address as _, Env};

    fn setup(env: &Env) -> (Address, Address, Address, Address) {
        let admin = Address::generate(env);

        let rs_id = env.register(RoleStore, ());
        RsClient::new(env, &rs_id).initialize(&admin);

        let ds_id = env.register(DataStore, ());
        DsClient::new(env, &ds_id).initialize(&admin, &rs_id);

        let rs_client = RsClient::new(env, &rs_id);
        rs_client.grant_role(&admin, &admin, &roles::controller(env));
        rs_client.grant_role(&admin, &admin, &roles::order_keeper(env));

        let oracle_id = env.register(Oracle, ());
        let passphrase = Bytes::from_slice(env, b"Test SDF Network ; September 2015");
        OracleClient::new(env, &oracle_id).initialize(&admin, &rs_id, &ds_id, &passphrase);

        rs_client.grant_role(&admin, &oracle_id, &roles::controller(env));

        (admin, rs_id, ds_id, oracle_id)
    }

    #[test]
    fn set_and_get_price_simple() {
        let env = Env::default();
        env.mock_all_auths();
        let (admin, _rs, _ds, oracle_id) = setup(&env);
        let client = OracleClient::new(&env, &oracle_id);

        let token = Address::generate(&env);
        let prices = Vec::from_array(
            &env,
            [TokenPrice {
                token: token.clone(),
                min: 2_000_000_000_000_000_000_000_000_000_000_000i128, // $2000 (FLOAT_PRECISION)
                max: 2_001_000_000_000_000_000_000_000_000_000_000i128,
            }],
        );

        client.set_prices_simple(&admin, &prices);

        let price = client.get_primary_price(&token);
        assert_eq!(price.min, 2_000_000_000_000_000_000_000_000_000_000_000i128);
        assert_eq!(price.max, 2_001_000_000_000_000_000_000_000_000_000_000i128);
    }

    #[test]
    fn try_get_price_returns_none_when_not_set() {
        let env = Env::default();
        env.mock_all_auths();
        let (_, _, _, oracle_id) = setup(&env);
        let client = OracleClient::new(&env, &oracle_id);

        let token = Address::generate(&env);
        assert!(client.try_get_price(&token).is_none());
    }

    #[test]
    #[should_panic]
    fn get_primary_price_panics_when_not_set() {
        let env = Env::default();
        env.mock_all_auths();
        let (_, _, _, oracle_id) = setup(&env);
        let client = OracleClient::new(&env, &oracle_id);

        let token = Address::generate(&env);
        client.get_primary_price(&token); // should panic
    }

    #[test]
    #[should_panic]
    fn invalid_price_rejected() {
        let env = Env::default();
        env.mock_all_auths();
        let (admin, _, _, oracle_id) = setup(&env);
        let client = OracleClient::new(&env, &oracle_id);

        let token = Address::generate(&env);
        // min > max → invalid
        let prices = Vec::from_array(
            &env,
            [TokenPrice {
                token,
                min: 1_000,
                max: 500,
            }],
        );
        client.set_prices_simple(&admin, &prices);
    }

    #[test]
    fn clear_price_removes_it() {
        let env = Env::default();
        env.mock_all_auths();
        let (admin, _, _, oracle_id) = setup(&env);
        let client = OracleClient::new(&env, &oracle_id);

        let token = Address::generate(&env);
        let prices = Vec::from_array(
            &env,
            [TokenPrice {
                token: token.clone(),
                min: 1_000_000_000_000_000_000_000_000_000_000i128,
                max: 1_001_000_000_000_000_000_000_000_000_000i128,
            }],
        );

        client.set_prices_simple(&admin, &prices);
        assert!(client.try_get_price(&token).is_some());

        client.clear_price(&admin, &token);
        assert!(client.try_get_price(&token).is_none());
    }

    // ── Issue #172: ledger sequence wrap-around staleness ─────────────────────

    /// Post-wrap scenario: current_seq = 1, submitted = u32::MAX.
    /// The submitted sequence numerically exceeds current (pre-wrap artifact) and
    /// must be rejected as stale regardless of the apparent numeric difference.
    #[test]
    fn seq_staleness_rejects_pre_wrap_sequence_as_stale() {
        assert!(is_seq_stale(1, u32::MAX, 60), "u32::MAX > 1 must be stale");
        assert!(is_seq_stale(1, u32::MAX - 1, 60), "u32::MAX-1 > 1 must be stale");
        // Edge: submitted exactly equals current — fresh (age = 0)
        assert!(!is_seq_stale(1, 1, 60), "same-ledger submission must be fresh");
    }

    /// Normal (non-wrap) staleness check must still work after the refactor.
    #[test]
    fn seq_staleness_normal_case_works() {
        // On boundary (60 ledgers ago) → fresh
        assert!(!is_seq_stale(100, 40, 60));
        // One beyond boundary (61 ledgers ago) → stale
        assert!(is_seq_stale(100, 39, 60));
        // Just submitted (same ledger) → fresh
        assert!(!is_seq_stale(100, 100, 60));
        // One ledger ago → fresh
        assert!(!is_seq_stale(100, 99, 60));
    }

    #[test]
    fn multiple_tokens_set_and_read() {
        let env = Env::default();
        env.mock_all_auths();
        let (admin, _, _, oracle_id) = setup(&env);
        let client = OracleClient::new(&env, &oracle_id);

        let eth = Address::generate(&env);
        let btc = Address::generate(&env);
        let usdc = Address::generate(&env);

        let prices = Vec::from_array(
            &env,
            [
                TokenPrice {
                    token: eth.clone(),
                    min: 2_000 * 10i128.pow(30),
                    max: 2_001 * 10i128.pow(30),
                },
                TokenPrice {
                    token: btc.clone(),
                    min: 60_000 * 10i128.pow(30),
                    max: 60_010 * 10i128.pow(30),
                },
                TokenPrice {
                    token: usdc.clone(),
                    min: 10i128.pow(30),
                    max: 10i128.pow(30),
                },
            ],
        );

        client.set_prices_simple(&admin, &prices);

        assert_eq!(client.get_primary_price(&eth).min, 2_000 * 10i128.pow(30));
        assert_eq!(client.get_primary_price(&btc).min, 60_000 * 10i128.pow(30));
        assert_eq!(client.get_primary_price(&usdc).min, 10i128.pow(30));
    }

    // ── Issue #292: signer rotation ───────────────────────────────────────────

    fn register_keeper_pubkey(env: &Env, ds: &Address, admin: &Address, index: u32, pubkey: &BytesN<32>) {
        let mut buf = Bytes::new(env);
        let prefix = keeper_public_key_prefix(env);
        buf.extend_from_array(&prefix.to_array());
        buf.extend_from_array(&index.to_be_bytes());
        let key: BytesN<32> = env.crypto().sha256(&buf).into();
        DsClient::new(env, ds).set_bytes32(admin, &key, pubkey);
    }

    /// rotate_signer replaces the stored pubkey so get_keeper_pubkey returns the new key.
    #[test]
    fn rotate_signer_updates_stored_pubkey() {
        let env = Env::default();
        env.mock_all_auths();
        let (admin, _rs, ds, oracle_id) = setup(&env);
        let client = OracleClient::new(&env, &oracle_id);

        let pubkey_a = BytesN::from_array(&env, &[0xAA; 32]);
        let pubkey_b = BytesN::from_array(&env, &[0xBB; 32]);

        // Register keeper A at index 0
        register_keeper_pubkey(&env, &ds, &admin, 0, &pubkey_a);
        assert_eq!(get_keeper_pubkey(&env, &ds, 0), pubkey_a);

        // Rotate to keeper B
        client.rotate_signer(&admin, &0u32, &pubkey_b);

        // data_store must now return B
        assert_eq!(get_keeper_pubkey(&env, &ds, 0), pubkey_b);
    }

    /// rotate_signer emits OracleSignerRotated with correct old and new signer fields.
    #[test]
    fn rotate_signer_emits_event_with_old_and_new_signer() {
        let env = Env::default();
        env.mock_all_auths();
        let (admin, _rs, ds, oracle_id) = setup(&env);
        let client = OracleClient::new(&env, &oracle_id);

        let pubkey_a = BytesN::from_array(&env, &[0x11; 32]);
        let pubkey_b = BytesN::from_array(&env, &[0x22; 32]);
        register_keeper_pubkey(&env, &ds, &admin, 1, &pubkey_a);

        client.rotate_signer(&admin, &1u32, &pubkey_b);

        let events = env.events().all();
        let rotation_event = events
            .iter()
            .find(|(_, topics, _)| topics.contains(&soroban_sdk::Val::from(symbol_short!("sig_rot"))))
            .expect("sig_rot event must be emitted");
        let payload: OracleSignerRotated = soroban_sdk::FromVal::from_val(&env, &rotation_event.2);
        assert_eq!(payload.keeper_index, 1);
        assert_eq!(payload.old_signer, pubkey_a);
        assert_eq!(payload.new_signer, pubkey_b);
    }

    /// rotate_signer called by a non-admin must panic with Unauthorized.
    #[test]
    #[should_panic]
    fn rotate_signer_rejects_non_admin() {
        let env = Env::default();
        env.mock_all_auths();
        let (admin, _rs, ds, oracle_id) = setup(&env);
        let client = OracleClient::new(&env, &oracle_id);

        let pubkey_a = BytesN::from_array(&env, &[0xAA; 32]);
        register_keeper_pubkey(&env, &ds, &admin, 0, &pubkey_a);

        // Use a different address as caller — must be rejected
        let non_admin = Address::generate(&env);
        client.rotate_signer(&non_admin, &0u32, &BytesN::from_array(&env, &[0xBB; 32]));
    }
}
