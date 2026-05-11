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

use soroban_sdk::{
    contract, contractimpl, contracttype, contracterror, panic_with_error,
    symbol_short, Address, Bytes, BytesN, Env, Vec,
};
use gmx_types::{PriceProps, TokenPrice};
use gmx_keys::{stable_price_key, keeper_public_key_prefix};

// ─── Errors ───────────────────────────────────────────────────────────────────

#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
#[repr(u32)]
pub enum Error {
    NotInitialized      = 1,
    AlreadyInitialized  = 2,
    Unauthorized        = 3,
    InvalidPrice        = 4,       // min > max or zero
    StalePrice          = 5,       // timestamp too old
    PriceNotFound       = 6,
    InvalidSignature    = 7,
    NoKeepers           = 8,
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

// ─── Signed price submitted by keeper ────────────────────────────────────────

/// One signed price attestation from a keeper.
#[contracttype]
pub struct SignedPrice {
    pub token:     Address,
    pub min_price: i128,
    pub max_price: i128,
    pub timestamp: u64,
    /// ed25519 signature over the canonical message (64 bytes)
    pub signature: BytesN<64>,
    /// Index of the keeper's public key in data_store (0-based)
    pub keeper_index: u32,
}

// ─── Cross-contract clients ───────────────────────────────────────────────────

#[soroban_sdk::contractclient(name = "RoleStoreClient")]
trait IRoleStore {
    fn has_role(env: Env, account: Address, role: BytesN<32>) -> bool;
}

#[soroban_sdk::contractclient(name = "DataStoreClient")]
trait IDataStore {
    fn get_u128(env: Env, key: BytesN<32>) -> u128;
    fn get_bytes32(env: Env, key: BytesN<32>) -> BytesN<32>;
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
        env.storage().instance().set(&InstanceKey::Initialized, &true);
        env.storage().instance().set(&InstanceKey::Admin, &admin);
        env.storage().instance().set(&InstanceKey::RoleStore, &role_store);
        env.storage().instance().set(&InstanceKey::DataStore, &data_store);
        env.storage().instance().set(&InstanceKey::NetworkPassphrase, &network_passphrase);
    }

    // ── Keeper price submission ───────────────────────────────────────────────

    /// Submit a batch of keeper-signed prices.
    ///
    /// Each price is individually signature-verified against the registered
    /// keeper public key at `keeper_index`. The caller must have ORDER_KEEPER role.
    pub fn set_prices(env: Env, caller: Address, prices: Vec<SignedPrice>) {
        caller.require_auth();
        require_order_keeper(&env, &caller);

        let passphrase: Bytes = env.storage().instance().get(&InstanceKey::NetworkPassphrase).unwrap();
        let data_store: Address = env.storage().instance().get(&InstanceKey::DataStore).unwrap();
        let ledger_seq = env.ledger().sequence();

        for i in 0..prices.len() {
            let sp = prices.get(i).unwrap();

            // Basic validation
            if sp.min_price <= 0 || sp.max_price <= 0 || sp.min_price > sp.max_price {
                panic_with_error!(&env, Error::InvalidPrice);
            }

            // Timestamp must be within 5 minutes of current ledger time
            let now = env.ledger().timestamp();
            let age = if now > sp.timestamp { now - sp.timestamp } else { 0 };
            if age > 300 {
                panic_with_error!(&env, Error::StalePrice);
            }

            // Verify ed25519 signature
            let msg = build_price_message(
                &env, &passphrase, ledger_seq,
                &sp.token, sp.min_price, sp.max_price, sp.timestamp,
            );
            let pubkey = get_keeper_pubkey(&env, &data_store, sp.keeper_index);
            // ed25519_verify takes (&BytesN<32> pubkey, &Bytes message, &BytesN<64> sig)
            env.crypto().ed25519_verify(&pubkey, &msg, &sp.signature);

            // Store in temporary storage (expires at end of current ledger TTL)
            let price = PriceProps { min: sp.min_price, max: sp.max_price };
            env.storage().temporary().set(&TempKey::Price(sp.token.clone()), &price);
        }

        env.events().publish(
            (symbol_short!("prices"),),
            (caller, prices.len()),
        );
    }

    /// Submit prices without signature verification.
    ///
    /// Simpler path: caller must have ORDER_KEEPER role, no ed25519 required.
    /// Suitable for local/test environments where keepers are fully trusted.
    pub fn set_prices_simple(env: Env, caller: Address, prices: Vec<TokenPrice>) {
        caller.require_auth();
        require_order_keeper(&env, &caller);

        for i in 0..prices.len() {
            let tp = prices.get(i).unwrap();
            if tp.min <= 0 || tp.max <= 0 || tp.min > tp.max {
                panic_with_error!(&env, Error::InvalidPrice);
            }
            let price = PriceProps { min: tp.min, max: tp.max };
            env.storage().temporary().set(&TempKey::Price(tp.token.clone()), &price);
        }
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
        if price == 0 { None } else { Some(price) }
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
            return PriceProps { min: stable, max: stable };
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

// ─── Internal helpers ─────────────────────────────────────────────────────────

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

// ─── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use soroban_sdk::{testutils::Address as _, Env};
    use role_store::{RoleStore, RoleStoreClient as RsClient};
    use data_store::{DataStore, DataStoreClient as DsClient};
    use gmx_keys::roles;

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

        (admin, rs_id, ds_id, oracle_id)
    }

    #[test]
    fn set_and_get_price_simple() {
        let env = Env::default();
        env.mock_all_auths();
        let (admin, _rs, _ds, oracle_id) = setup(&env);
        let client = OracleClient::new(&env, &oracle_id);

        let token = Address::generate(&env);
        let prices = Vec::from_array(&env, [TokenPrice {
            token: token.clone(),
            min: 2_000_000_000_000_000_000_000_000_000_000_000i128, // $2000 (FLOAT_PRECISION)
            max: 2_001_000_000_000_000_000_000_000_000_000_000i128,
        }]);

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
        let prices = Vec::from_array(&env, [TokenPrice {
            token,
            min: 1_000,
            max: 500,
        }]);
        client.set_prices_simple(&admin, &prices);
    }

    #[test]
    fn clear_price_removes_it() {
        let env = Env::default();
        env.mock_all_auths();
        let (admin, _, _, oracle_id) = setup(&env);
        let client = OracleClient::new(&env, &oracle_id);

        let token = Address::generate(&env);
        let prices = Vec::from_array(&env, [TokenPrice {
            token: token.clone(),
            min: 1_000_000_000_000_000_000_000_000_000_000i128,
            max: 1_001_000_000_000_000_000_000_000_000_000i128,
        }]);

        client.set_prices_simple(&admin, &prices);
        assert!(client.try_get_price(&token).is_some());

        client.clear_price(&admin, &token);
        assert!(client.try_get_price(&token).is_none());
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

        let prices = Vec::from_array(&env, [
            TokenPrice { token: eth.clone(),  min: 2_000 * 10i128.pow(30), max: 2_001 * 10i128.pow(30) },
            TokenPrice { token: btc.clone(),  min: 60_000 * 10i128.pow(30), max: 60_010 * 10i128.pow(30) },
            TokenPrice { token: usdc.clone(), min: 10i128.pow(30), max: 10i128.pow(30) },
        ]);

        client.set_prices_simple(&admin, &prices);

        assert_eq!(client.get_primary_price(&eth).min,  2_000 * 10i128.pow(30));
        assert_eq!(client.get_primary_price(&btc).min,  60_000 * 10i128.pow(30));
        assert_eq!(client.get_primary_price(&usdc).min, 10i128.pow(30));
    }
}
