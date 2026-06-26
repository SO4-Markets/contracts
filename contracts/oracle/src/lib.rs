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

use gmx_keys::{
    keeper_public_key_prefix, market_index_token_key, market_list_key, market_long_token_key,
    market_short_token_key, stable_price_key,
};
use gmx_types::PriceProps;

#[cfg(any(test, feature = "testutils"))]
use gmx_types::TokenPrice;
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
    QuorumNotMet = 9,
    DuplicateSigner = 10,
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
enum PersistentKey {
    TokenSigner(Address, u32),
    TokenSignerCount(Address),
    TokenQuorum(Address),
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
}

#[contracttype]
pub struct CircuitBreakerTripped {
    pub market: Address,
    pub token: Address,
    pub old_price: i128,
    pub new_price: i128,
    pub deviation_bps: u128,
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

    /// Configure the trusted signer set and quorum for a token.
    ///
    /// `signer_indices` refer to keeper public key indices stored in data_store.
    /// `quorum` is the minimum number of valid submitted signer prices required.
    pub fn set_token_signers(
        env: Env,
        caller: Address,
        token: Address,
        signer_indices: Vec<u32>,
        quorum: u32,
    ) {
        caller.require_auth();
        require_admin(&env, &caller);

        if signer_indices.len() == 0 {
            panic_with_error!(&env, Error::NoKeepers);
        }

        if quorum == 0 || quorum > signer_indices.len() {
            panic_with_error!(&env, Error::QuorumNotMet);
        }

        // Reject duplicate signer indices in the new configuration.
        for i in 0..signer_indices.len() {
            let current = signer_indices.get(i).unwrap();

            for j in (i + 1)..signer_indices.len() {
                if signer_indices.get(j).unwrap() == current {
                    panic_with_error!(&env, Error::DuplicateSigner);
                }
            }
        }

        // Remove the previous signer configuration for this token.
        let old_count: u32 = env
            .storage()
            .persistent()
            .get(&PersistentKey::TokenSignerCount(token.clone()))
            .unwrap_or(0);

        for i in 0..old_count {
            env.storage()
                .persistent()
                .remove(&PersistentKey::TokenSigner(token.clone(), i));
        }

        // Store the new signer configuration.
        for i in 0..signer_indices.len() {
            let signer_index = signer_indices.get(i).unwrap();

            env.storage()
                .persistent()
                .set(&PersistentKey::TokenSigner(token.clone(), i), &signer_index);
        }

        let signer_count = signer_indices.len();

        env.storage().persistent().set(
            &PersistentKey::TokenSignerCount(token.clone()),
            &signer_count,
        );

        env.storage()
            .persistent()
            .set(&PersistentKey::TokenQuorum(token), &quorum);
    }

    // ── Keeper price submission ───────────────────────────────────────────────

    /// Submit a batch of keeper-signed prices.
    ///
    /// Each submitted price is signature-verified against the registered keeper
    /// public key at `keeper_index`. Prices are then grouped by token and the
    /// oracle stores the median min price and median max price for each token.
    pub fn set_prices(env: Env, caller: Address, prices: Vec<SignedPrice>) {
        caller.require_auth();
        require_order_keeper(&env, &caller);

        if prices.len() == 0 {
            panic_with_error!(&env, Error::NoKeepers);
        }

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

        // First pass: validate every submitted price and verify its signature.
        for i in 0..prices.len() {
            let sp = prices.get(i).unwrap();

            if sp.min_price <= 0 || sp.max_price <= 0 || sp.min_price > sp.max_price {
                panic_with_error!(&env, Error::InvalidPrice);
            }

            // Timestamp must be within 5 minutes of current ledger time.
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

            // If a per-token signer set is configured, this keeper index must be allowed.
            if !is_token_signer(&env, &sp.token, sp.keeper_index) {
                panic_with_error!(&env, Error::Unauthorized);
            }

            // A single signer must not be counted more than once for the same token.
            if has_duplicate_keeper_for_token(&prices, &sp.token, sp.keeper_index) {
                panic_with_error!(&env, Error::DuplicateSigner);
            }

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

            env.crypto().ed25519_verify(&pubkey, &msg, &sp.signature);
        }

        // Second pass: process each unique token once.
        for i in 0..prices.len() {
            let current = prices.get(i).unwrap();

            // Skip this token if it was already processed earlier.
            let mut already_processed = false;

            for j in 0..i {
                let previous = prices.get(j).unwrap();

                if previous.token == current.token {
                    already_processed = true;
                    break;
                }
            }

            if already_processed {
                continue;
            }

            let mut mins = Vec::new(&env);
            let mut maxs = Vec::new(&env);
            let mut submitted_count = 0u32;

            // Collect all valid submitted prices for this token.
            for j in 0..prices.len() {
                let sp = prices.get(j).unwrap();

                if sp.token == current.token {
                    mins.push_back(sp.min_price);
                    maxs.push_back(sp.max_price);
                    submitted_count += 1;
                }
            }

            let quorum = get_token_quorum(&env, &current.token, submitted_count);

            if submitted_count < quorum {
                panic_with_error!(&env, Error::QuorumNotMet);
            }

            let median_min = median_i128(&env, mins);
            let median_max = median_i128(&env, maxs);

            check_circuit_breaker(&env, &data_store, &current.token, median_min, median_max);

            let price = PriceProps {
                min: median_min,
                max: median_max,
            };

            if price.min <= 0 || price.max <= 0 || price.min > price.max {
                panic_with_error!(&env, Error::InvalidPrice);
            }

            let price_key = TempKey::Price(current.token.clone());

            env.storage().temporary().set(&price_key, &price);

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
        env.storage()
            .temporary()
            .get::<TempKey, PriceProps>(&TempKey::Price(token))
            .unwrap_or_else(|| panic_with_error!(&env, Error::PriceNotFound))
    }

    /// Returns the price for a token, or None if not set.
    pub fn try_get_price(env: Env, token: Address) -> Option<PriceProps> {
        env.storage()
            .temporary()
            .get::<TempKey, PriceProps>(&TempKey::Price(token))
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
        env.storage()
            .temporary()
            .get::<TempKey, PriceProps>(&TempKey::Price(token))
            .unwrap_or_else(|| panic_with_error!(&env, Error::PriceNotFound))
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
            let price = PriceProps {
                min: tp.min,
                max: tp.max,
            };

            if price.min <= 0 || price.max <= 0 || price.min > price.max {
                panic_with_error!(&env, Error::InvalidPrice);
            }

            let price_key = TempKey::Price(tp.token.clone());

            env.storage().temporary().set(&price_key, &price);

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

#[allow(dead_code)]
fn median_i128(env: &Env, values: Vec<i128>) -> i128 {
    let len = values.len();

    if len == 0 {
        panic_with_error!(env, Error::InvalidPrice);
    }

    let mut sorted = values.clone();

    // Insertion sort is enough here because the signer set is expected to be small.
    // Typical quorum configurations are 1-of-1, 2-of-3, or similar.
    let mut i = 1;
    while i < len {
        let key = sorted.get(i).unwrap();
        let mut j = i;

        // Shift larger values one position to the right.
        while j > 0 {
            let prev = sorted.get(j - 1).unwrap();

            if prev <= key {
                break;
            }

            sorted.set(j, prev);
            j -= 1;
        }

        // Insert the current value in its sorted position.
        sorted.set(j, key);
        i += 1;
    }

    let mid = len / 2;

    if len % 2 == 1 {
        // Odd number of values: return the middle value.
        sorted.get(mid).unwrap()
    } else {
        // Even number of values: return the average of the two middle values.
        // This form avoids potential overflow from `(a + b) / 2`.
        let a = sorted.get(mid - 1).unwrap();
        let b = sorted.get(mid).unwrap();

        a + ((b - a) / 2)
    }
}

fn require_admin(env: &Env, caller: &Address) {
    let admin: Address = env
        .storage()
        .instance()
        .get(&InstanceKey::Admin)
        .unwrap_or_else(|| panic_with_error!(env, Error::NotInitialized));

    if &admin != caller {
        panic_with_error!(env, Error::Unauthorized);
    }
}

fn is_token_signer(env: &Env, token: &Address, keeper_index: u32) -> bool {
    let count: u32 = env
        .storage()
        .persistent()
        .get(&PersistentKey::TokenSignerCount(token.clone()))
        .unwrap_or(0);

    // Backward compatibility:
    // If no per-token signer set is configured, preserve the previous behavior
    // and allow any registered keeper public key index.
    if count == 0 {
        return true;
    }

    for i in 0..count {
        let configured: u32 = env
            .storage()
            .persistent()
            .get(&PersistentKey::TokenSigner(token.clone(), i))
            .unwrap();

        if configured == keeper_index {
            return true;
        }
    }

    false
}

fn get_token_quorum(env: &Env, token: &Address, submitted_count: u32) -> u32 {
    let configured_quorum: u32 = env
        .storage()
        .persistent()
        .get(&PersistentKey::TokenQuorum(token.clone()))
        .unwrap_or(0);

    // Backward compatibility:
    // If no quorum is configured, one signed price is enough, matching the old
    // single-signer behavior.
    if configured_quorum == 0 {
        return 1;
    }

    // Never require more signatures than the configured signer set size.
    // Invalid configs are rejected by set_token_signers, so this is defensive.
    if configured_quorum > submitted_count {
        return configured_quorum;
    }

    configured_quorum
}

fn has_duplicate_keeper_for_token(
    prices: &Vec<SignedPrice>,
    token: &Address,
    keeper_index: u32,
) -> bool {
    let mut count = 0u32;

    for i in 0..prices.len() {
        let sp = prices.get(i).unwrap();

        if sp.token == *token && sp.keeper_index == keeper_index {
            count += 1;

            if count > 1 {
                return true;
            }
        }
    }

    false
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

/// Retrieve ed25519 public key for a keeper by index from data_store.
///
/// Keys are stored as 32 bytes at key = sha256("KEEPER_PUBLIC_KEY" ‖ index_u32_BE).
/// We pack two consecutive BytesN<32> to form the full 32-byte ed25519 pubkey.
/// For simplicity we store the key at (prefix ‖ index) and read 32 bytes.
fn get_keeper_pubkey(env: &Env, data_store: &Address, index: u32) -> BytesN<32> {
    let mut buf = Bytes::new(env);
    let prefix = keeper_public_key_prefix(env);
    buf.extend_from_array(&prefix.to_array());
    buf.extend_from_array(&index.to_be_bytes());
    let key = env.crypto().sha256(&buf).into();
    let client = DataStoreClient::new(env, data_store);
    client.get_bytes32(&key)
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

fn check_circuit_breaker(
    env: &Env,
    data_store: &Address,
    token: &Address,
    new_min: i128,
    new_max: i128,
) {
    let price_key = TempKey::Price(token.clone());
    let prev_price_opt = env
        .storage()
        .temporary()
        .get::<TempKey, PriceProps>(&price_key);

    if let Some(prev_price) = prev_price_opt {
        let last_price = prev_price.mid_price();
        let new_price = (new_min + new_max) / 2;
        if last_price > 0 {
            let deviation_val = (new_price - last_price).abs();
            let deviation_bps = ((deviation_val as u128) * 10000) / (last_price as u128);

            let ds = DataStoreClient::new(env, data_store);
            let market_list_k = market_list_key(env);
            let market_count = ds.get_address_set_count(&market_list_k);
            let markets = ds.get_address_set_at(&market_list_k, &0, &market_count);

            for i in 0..markets.len() {
                let market = markets.get(i).unwrap();
                let index_token = ds.get_address(&market_index_token_key(env, &market));
                let long_token = ds.get_address(&market_long_token_key(env, &market));
                let short_token = ds.get_address(&market_short_token_key(env, &market));

                let matches_market = (index_token.is_some() && index_token.unwrap() == *token)
                    || (long_token.is_some() && long_token.unwrap() == *token)
                    || (short_token.is_some() && short_token.unwrap() == *token);

                if matches_market {
                    let threshold =
                        ds.get_u128(&gmx_keys::circuit_breaker_factor_key(env, &market));
                    if threshold > 0 && deviation_bps > threshold {
                        // Set market pause flag to true
                        ds.set_bool(
                            &env.current_contract_address(),
                            &gmx_keys::is_market_paused_key(env, &market),
                            &true,
                        );

                        // Emit event
                        env.events().publish(
                            (soroban_sdk::symbol_short!("cb_trip"),),
                            CircuitBreakerTripped {
                                market: market.clone(),
                                token: token.clone(),
                                old_price: last_price,
                                new_price,
                                deviation_bps,
                            },
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
                min: 2_000_000_000_000_000_000_000_000_000_000_000i128,
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
        client.get_primary_price(&token);
    }

    #[test]
    #[should_panic]
    fn invalid_price_rejected() {
        let env = Env::default();
        env.mock_all_auths();
        let (admin, _, _, oracle_id) = setup(&env);
        let client = OracleClient::new(&env, &oracle_id);

        let token = Address::generate(&env);
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
        assert!(
            is_seq_stale(1, u32::MAX - 1, 60),
            "u32::MAX-1 > 1 must be stale"
        );
        // Edge: submitted exactly equals current — fresh (age = 0)
        assert!(
            !is_seq_stale(1, 1, 60),
            "same-ledger submission must be fresh"
        );
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

    #[test]
    fn median_works_with_three_values() {
        let env = Env::default();

        let values = Vec::from_array(&env, [100i128, 300i128, 200i128]);

        assert_eq!(median_i128(&env, values), 200);
    }

    #[test]
    fn median_works_with_five_values() {
        let env = Env::default();

        let values = Vec::from_array(&env, [500i128, 100i128, 300i128, 200i128, 400i128]);

        assert_eq!(median_i128(&env, values), 300);
    }

    #[test]
    fn median_works_with_two_values() {
        let env = Env::default();

        let values = Vec::from_array(&env, [100i128, 300i128]);

        assert_eq!(median_i128(&env, values), 200);
    }

    #[test]
    fn set_prices_simple_keeps_last_submitted_price() {
        let env = Env::default();
        env.mock_all_auths();

        let (admin, _, _, oracle_id) = setup(&env);
        let client = OracleClient::new(&env, &oracle_id);

        let token = Address::generate(&env);

        let prices = Vec::from_array(
            &env,
            [
                TokenPrice {
                    token: token.clone(),
                    min: 100,
                    max: 110,
                },
                TokenPrice {
                    token: token.clone(),
                    min: 101,
                    max: 111,
                },
                TokenPrice {
                    token: token.clone(),
                    min: 10_000,
                    max: 10_010,
                },
            ],
        );

        client.set_prices_simple(&admin, &prices);

        let price = client.get_primary_price(&token);

        assert_eq!(price.min, 10_000);
        assert_eq!(price.max, 10_010);
    }

    #[test]
    fn set_token_signers_stores_configured_quorum() {
        let env = Env::default();
        env.mock_all_auths();

        let (admin, _, _, oracle_id) = setup(&env);
        let client = OracleClient::new(&env, &oracle_id);

        let token = Address::generate(&env);
        let signer_indices = Vec::from_array(&env, [0u32, 1u32, 2u32]);

        client.set_token_signers(&admin, &token, &signer_indices, &2u32);

        env.as_contract(&oracle_id, || {
            assert_eq!(get_token_quorum(&env, &token, 3), 2);
            assert!(is_token_signer(&env, &token, 0));
            assert!(is_token_signer(&env, &token, 1));
            assert!(is_token_signer(&env, &token, 2));
            assert!(!is_token_signer(&env, &token, 3));
        });
    }

    #[test]
    #[should_panic]
    fn set_token_signers_rejects_duplicate_signers() {
        let env = Env::default();
        env.mock_all_auths();

        let (admin, _, _, oracle_id) = setup(&env);
        let client = OracleClient::new(&env, &oracle_id);

        let token = Address::generate(&env);
        let signer_indices = Vec::from_array(&env, [0u32, 1u32, 1u32]);

        client.set_token_signers(&admin, &token, &signer_indices, &2u32);
    }

    #[test]
    #[should_panic]
    fn set_token_signers_rejects_quorum_above_signer_count() {
        let env = Env::default();
        env.mock_all_auths();

        let (admin, _, _, oracle_id) = setup(&env);
        let client = OracleClient::new(&env, &oracle_id);

        let token = Address::generate(&env);
        let signer_indices = Vec::from_array(&env, [0u32, 1u32, 2u32]);

        client.set_token_signers(&admin, &token, &signer_indices, &4u32);
    }

    #[test]
    fn unset_token_signer_config_preserves_single_signer_behavior() {
        let env = Env::default();
        env.mock_all_auths();

        let (_, _, _, oracle_id) = setup(&env);

        let token = Address::generate(&env);

        env.as_contract(&oracle_id, || {
            assert_eq!(get_token_quorum(&env, &token, 1), 1);
            assert!(is_token_signer(&env, &token, 999));
        });
    }
}
