//! Market Factory — deploys LP token contracts and registers markets.
//!
//! Mirrors GMX's `MarketFactory.sol`:
//!   - Admin calls `set_market_token_wasm_hash` once to store the WASM hash
//!     of the compiled `market_token` contract.
//!   - MARKET_KEEPER calls `create_market` to deploy a fresh LP token via
//!     deterministic addressing and register the market in `data_store`.
//!
//! Deterministic deploy:
//!   salt = sha256("GMX_MARKET" ‖ index_token ‖ long_token ‖ short_token ‖ market_type)
//!   LP token address = env.deployer().with_address(factory, salt).deployed_address()
#![no_std]

use soroban_sdk::{
    contract, contractimpl, contracttype, contracterror, panic_with_error,
    symbol_short, Address, Bytes, BytesN, Env, String, Vec,
};
use gmx_keys::{
    market_list_key, market_key, roles, token_decimals_key,
    market_index_token_key, market_long_token_key, market_short_token_key,
};
use gmx_types::MarketProps;

// ─── Errors ───────────────────────────────────────────────────────────────────

#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
#[repr(u32)]
pub enum Error {
    NotInitialized    = 1,
    AlreadyInitialized = 2,
    Unauthorized      = 3,
    MarketAlreadyExists = 4,
    WasmHashNotSet    = 5,
}

// ─── Storage ──────────────────────────────────────────────────────────────────

#[contracttype]
enum InstanceKey {
    Initialized,
    Admin,
    RoleStore,
    DataStore,
    MarketTokenWasmHash,
}

// ─── Cross-contract client interfaces ────────────────────────────────────────

#[soroban_sdk::contractclient(name = "RoleStoreClient")]
trait IRoleStore {
    fn has_role(env: Env, account: Address, role: BytesN<32>) -> bool;
}

#[soroban_sdk::contractclient(name = "DataStoreClient")]
trait IDataStore {
    fn get_bool(env: Env, key: BytesN<32>) -> bool;
    fn set_bool(env: Env, caller: Address, key: BytesN<32>, value: bool) -> bool;
    fn set_bytes32(env: Env, caller: Address, key: BytesN<32>, value: BytesN<32>) -> BytesN<32>;
    fn set_address(env: Env, caller: Address, key: BytesN<32>, value: Address) -> Address;
    fn add_address_to_set(env: Env, caller: Address, set_key: BytesN<32>, value: Address);
    fn get_address_set_count(env: Env, set_key: BytesN<32>) -> u32;
    fn get_address_set_at(env: Env, set_key: BytesN<32>, start: u32, end: u32) -> Vec<Address>;
    fn set_u128(env: Env, caller: Address, key: BytesN<32>, value: u128) -> u128;
    fn get_u128(env: Env, key: BytesN<32>) -> u128;
}

#[soroban_sdk::contractclient(name = "MarketTokenClient")]
trait IMarketToken {
    fn initialize(
        env: Env,
        admin: Address,
        role_store: Address,
        decimal: u32,
        name: String,
        symbol: String,
    );
    fn decimals(env: Env) -> u32;
}

// ─── Contract ─────────────────────────────────────────────────────────────────

#[contract]
pub struct MarketFactory;

#[contractimpl]
impl MarketFactory {
    // ── Initializer ──────────────────────────────────────────────────────────

    pub fn initialize(
        env: Env,
        admin: Address,
        role_store: Address,
        data_store: Address,
    ) {
        admin.require_auth();
        if env.storage().instance().has(&InstanceKey::Initialized) {
            panic_with_error!(&env, Error::AlreadyInitialized);
        }
        env.storage().instance().set(&InstanceKey::Initialized, &true);
        env.storage().instance().set(&InstanceKey::Admin, &admin);
        env.storage().instance().set(&InstanceKey::RoleStore, &role_store);
        env.storage().instance().set(&InstanceKey::DataStore, &data_store);
    }

    // ── Admin: set the wasm hash for market_token ─────────────────────────────

    /// Must be called once by admin after uploading market_token WASM to ledger.
    pub fn set_market_token_wasm_hash(env: Env, caller: Address, wasm_hash: BytesN<32>) {
        caller.require_auth();
        require_admin(&env, &caller);
        env.storage()
            .instance()
            .set(&InstanceKey::MarketTokenWasmHash, &wasm_hash);
        env.events().publish((symbol_short!("wasm_set"),), (wasm_hash,));
    }

    pub fn get_market_token_wasm_hash(env: Env) -> Option<BytesN<32>> {
        env.storage().instance().get(&InstanceKey::MarketTokenWasmHash)
    }

    // ── Create market ─────────────────────────────────────────────────────────

    /// Deploy a fresh market_token and register the market in data_store.
    ///
    /// `market_type`: a BytesN<32> discriminant (e.g. sha256("DEFAULT")) that
    ///   allows multiple markets for the same token pair (different fee configs).
    ///
    /// Returns the newly created `MarketProps`.
    pub fn create_market(
        env: Env,
        caller: Address,
        index_token: Address,
        long_token: Address,
        short_token: Address,
        market_type: BytesN<32>,
    ) -> MarketProps {
        caller.require_auth();
        require_market_keeper(&env, &caller);

        let wasm_hash: BytesN<32> = env
            .storage()
            .instance()
            .get(&InstanceKey::MarketTokenWasmHash)
            .unwrap_or_else(|| panic_with_error!(&env, Error::WasmHashNotSet));

        let data_store: Address = env
            .storage()
            .instance()
            .get(&InstanceKey::DataStore)
            .unwrap();

        let role_store: Address = env
            .storage()
            .instance()
            .get(&InstanceKey::RoleStore)
            .unwrap();

        // Compute deterministic salt = sha256("GMX_MARKET" ‖ index ‖ long ‖ short ‖ type)
        let salt = compute_market_salt(&env, &index_token, &long_token, &short_token, &market_type);

        // Derive the market token address before deploying to check for duplicates
        let factory = env.current_contract_address();
        let deployer = env.deployer().with_address(factory.clone(), salt.clone());
        let market_token_address = deployer.deployed_address();

        // Check market doesn't already exist
        let market_check_key = market_key(&env, &market_token_address);
        let ds_client = DataStoreClient::new(&env, &data_store);
        // If the market already registered, the key will be non-zero
        let existing = ds_client.get_bool(&market_check_key);
        if existing {
            panic_with_error!(&env, Error::MarketAlreadyExists);
        }

        // Build LP token name/symbol
        let name = String::from_str(&env, "GMX Market Token");
        let symbol = String::from_str(&env, "GM");

        // Deploy market_token contract deterministically
        let deployer = env.deployer().with_address(factory, salt);
        let market_token_address = deployer.deploy_v2(
            wasm_hash,
            (
                env.current_contract_address(), // admin = factory
                role_store.clone(),
                7u32,           // Stellar 7-decimal precision
                name,
                symbol,
            ),
        );

        let market = MarketProps {
            market_token: market_token_address.clone(),
            index_token: index_token.clone(),
            long_token: long_token.clone(),
            short_token: short_token.clone(),
        };

        // Register in data_store:
        // 1. Store market existence flag
        ds_client.set_bool(&caller, &market_key(&env, &market_token_address), &true);
        // 2. Store constituent token addresses (so handlers can reconstruct MarketProps)
        ds_client.set_address(&caller, &market_index_token_key(&env, &market_token_address), &index_token);
        ds_client.set_address(&caller, &market_long_token_key(&env, &market_token_address), &long_token);
        ds_client.set_address(&caller, &market_short_token_key(&env, &market_token_address), &short_token);
        // 3. Store token decimals (7 for Stellar standard)
        ds_client.set_u128(&caller, &token_decimals_key(&env, &long_token), &7u128);
        ds_client.set_u128(&caller, &token_decimals_key(&env, &short_token), &7u128);
        ds_client.set_u128(&caller, &token_decimals_key(&env, &index_token), &7u128);
        // 4. Add to market list
        ds_client.add_address_to_set(&caller, &market_list_key(&env), &market_token_address);

        env.events().publish(
            (symbol_short!("mkt_new"),),
            (
                market_token_address.clone(),
                index_token,
                long_token,
                short_token,
            ),
        );

        market
    }

    // ── Views ─────────────────────────────────────────────────────────────────

    pub fn get_market_count(env: Env) -> u32 {
        let data_store: Address = env.storage().instance().get(&InstanceKey::DataStore).unwrap();
        let ds = DataStoreClient::new(&env, &data_store);
        ds.get_address_set_count(&market_list_key(&env))
    }

    pub fn get_markets(env: Env, start: u32, end: u32) -> Vec<Address> {
        let data_store: Address = env.storage().instance().get(&InstanceKey::DataStore).unwrap();
        let ds = DataStoreClient::new(&env, &data_store);
        ds.get_address_set_at(&market_list_key(&env), &start, &end)
    }
}

// ─── Internal helpers ─────────────────────────────────────────────────────────

fn require_admin(env: &Env, caller: &Address) {
    let admin: Address = env.storage().instance().get(&InstanceKey::Admin).unwrap();
    if *caller != admin {
        panic_with_error!(env, Error::Unauthorized);
    }
}

fn require_market_keeper(env: &Env, caller: &Address) {
    let role_store: Address = env
        .storage()
        .instance()
        .get(&InstanceKey::RoleStore)
        .unwrap();
    let client = RoleStoreClient::new(env, &role_store);
    let role = roles::market_keeper(env);
    if !client.has_role(caller, &role) {
        panic_with_error!(env, Error::Unauthorized);
    }
}

/// Deterministic market salt: sha256("GMX_MARKET" ‖ index ‖ long ‖ short ‖ type)
fn compute_market_salt(
    env: &Env,
    index_token: &Address,
    long_token: &Address,
    short_token: &Address,
    market_type: &BytesN<32>,
) -> BytesN<32> {
    let mut buf = Bytes::new(env);

    // tag
    let tag = b"GMX_MARKET";
    let tlen = tag.len() as u16;
    buf.append(&Bytes::from_slice(env, &tlen.to_be_bytes()));
    buf.append(&Bytes::from_slice(env, tag));

    // addresses (strkey encoding → Bytes)
    for addr in [index_token, long_token, short_token] {
        let s: soroban_sdk::String = addr.to_string();
        let b: Bytes = s.into();
        let len = b.len() as u16;
        buf.append(&Bytes::from_slice(env, &len.to_be_bytes()));
        buf.append(&b);
    }

    // market_type discriminant
    buf.extend_from_array(&market_type.to_array());

    env.crypto().sha256(&buf).into()
}

// ─── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use soroban_sdk::{testutils::Address as _, Env};
    use role_store::{RoleStore, RoleStoreClient as RsClient};
    use data_store::{DataStore, DataStoreClient as DsClient};
    #[allow(unused_imports)]
    use market_token::MarketToken;

    fn deploy_role_store(env: &Env, admin: &Address) -> Address {
        let id = env.register(RoleStore, ());
        RsClient::new(env, &id).initialize(admin);
        id
    }

    fn deploy_data_store(env: &Env, admin: &Address, rs: &Address) -> Address {
        let id = env.register(DataStore, ());
        DsClient::new(env, &id).initialize(admin, rs);
        id
    }

    fn setup() -> (Env, Address, Address, Address, Address) {
        let env = Env::default();
        env.mock_all_auths();
        let admin = Address::generate(&env);
        let rs = deploy_role_store(&env, &admin);
        let ds = deploy_data_store(&env, &admin, &rs);

        // Grant roles
        let rs_client = RsClient::new(&env, &rs);
        rs_client.grant_role(&admin, &admin, &roles::controller(&env));
        rs_client.grant_role(&admin, &admin, &roles::market_keeper(&env));

        let factory_id = env.register(MarketFactory, ());
        MarketFactoryClient::new(&env, &factory_id).initialize(&admin, &rs, &ds);

        (env, admin, rs, ds, factory_id)
    }

    #[test]
    fn test_initialize() {
        let (_, _, _, _, factory_id) = setup();
        // Just verifies no panic
        let _ = factory_id;
    }

    #[test]
    fn test_initial_market_count() {
        let (env, _, _, _, factory_id) = setup();
        let client = MarketFactoryClient::new(&env, &factory_id);
        assert_eq!(client.get_market_count(), 0);
        let markets = client.get_markets(&0, &10);
        assert_eq!(markets.len(), 0);
    }
}
