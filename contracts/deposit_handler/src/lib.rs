//! Deposit Handler — create, execute, and cancel LP token deposits.
//!
//! Mirrors GMX's DepositHandler.sol + ExecuteDepositUtils.sol:
//!
//! Flow:
//!   1. User approves long/short tokens to deposit_handler.
//!   2. User calls `create_deposit` → tokens pulled to deposit_vault, DepositProps stored.
//!   3. Keeper sets oracle prices, then calls `execute_deposit`:
//!      - Reads deposit, computes LP tokens to mint at current pool price.
//!      - Moves tokens from vault → market_token contract (the pool).
//!      - Mints LP tokens to receiver.
//!      - Updates pool amounts, funding state.
//!   4. On failure or timeout: `cancel_deposit` refunds tokens from vault.
#![no_std]
#![allow(dependency_on_unit_never_type_fallback)]
#![allow(deprecated)]

use soroban_sdk::{
    contract, contractimpl, contracttype, contracterror, panic_with_error,
    symbol_short, token, Address, BytesN, Env,
};
use gmx_types::{DepositProps, MarketProps};
pub use gmx_types::CreateDepositParams;
use gmx_math::{mul_div_wide, TOKEN_PRECISION};
use gmx_keys::{
    roles,
    deposit_key, deposit_list_key, account_deposit_list_key,
    market_index_token_key, market_long_token_key, market_short_token_key,
};
use gmx_market_utils::{
    get_market_token_price, apply_delta_to_pool_amount,
    update_funding_state, update_cumulative_borrowing_factor,
};

// ─── Errors ───────────────────────────────────────────────────────────────────

#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
#[repr(u32)]
pub enum Error {
    AlreadyInitialized  = 1,
    NotInitialized      = 2,
    Unauthorized        = 3,
    DepositNotFound     = 4,
    InsufficientLpOut   = 5,
    ZeroDeposit         = 6,
}

// ─── Storage ──────────────────────────────────────────────────────────────────

#[contracttype]
enum InstanceKey {
    Initialized,
    Admin,
    RoleStore,
    DataStore,
    Oracle,
    DepositVault,
}

#[contracttype]
enum LocalKey {
    Deposit(BytesN<32>),
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
    fn set_u128(env: Env, caller: Address, key: BytesN<32>, value: u128) -> u128;
    fn get_i128(env: Env, key: BytesN<32>) -> i128;
    fn set_i128(env: Env, caller: Address, key: BytesN<32>, value: i128) -> i128;
    fn apply_delta_to_u128(env: Env, caller: Address, key: BytesN<32>, delta: i128) -> u128;
    fn apply_delta_to_i128(env: Env, caller: Address, key: BytesN<32>, delta: i128) -> i128;
    fn get_address(env: Env, key: BytesN<32>) -> Option<Address>;
    fn add_address_to_set(env: Env, caller: Address, set_key: BytesN<32>, value: Address);
    fn remove_address_from_set(env: Env, caller: Address, set_key: BytesN<32>, value: Address);
    fn add_bytes32_to_set(env: Env, caller: Address, set_key: BytesN<32>, value: BytesN<32>);
    fn remove_bytes32_from_set(env: Env, caller: Address, set_key: BytesN<32>, value: BytesN<32>);
    fn increment_nonce(env: Env, caller: Address) -> u64;
}

#[allow(dead_code)]
#[soroban_sdk::contractclient(name = "OracleClient")]
trait IOracle {
    fn get_primary_price(env: Env, token: Address) -> gmx_types::PriceProps;
}

#[allow(dead_code)]
#[soroban_sdk::contractclient(name = "DepositVaultClient")]
trait IDepositVault {
    fn transfer_out(env: Env, caller: Address, token: Address, receiver: Address, amount: i128);
}

#[allow(dead_code)]
#[soroban_sdk::contractclient(name = "MarketTokenClient")]
trait IMarketToken {
    fn mint(env: Env, caller: Address, to: Address, amount: i128);
    fn total_supply(env: Env) -> i128;
}

// ─── Contract ─────────────────────────────────────────────────────────────────

#[contract]
pub struct DepositHandler;

#[contractimpl]
impl DepositHandler {
    // ── Init ─────────────────────────────────────────────────────────────────

    pub fn initialize(
        env: Env,
        admin: Address,
        role_store: Address,
        data_store: Address,
        oracle: Address,
        deposit_vault: Address,
    ) {
        admin.require_auth();
        if env.storage().instance().has(&InstanceKey::Initialized) {
            panic_with_error!(&env, Error::AlreadyInitialized);
        }
        env.storage().instance().set(&InstanceKey::Initialized, &true);
        env.storage().instance().set(&InstanceKey::Admin, &admin);
        env.storage().instance().set(&InstanceKey::RoleStore, &role_store);
        env.storage().instance().set(&InstanceKey::DataStore, &data_store);
        env.storage().instance().set(&InstanceKey::Oracle, &oracle);
        env.storage().instance().set(&InstanceKey::DepositVault, &deposit_vault);
    }

    // ── Create deposit ────────────────────────────────────────────────────────

    /// Pull tokens from caller into the deposit_vault and record the deposit.
    /// Returns a unique deposit key (BytesN<32>).
    pub fn create_deposit(env: Env, caller: Address, params: CreateDepositParams) -> BytesN<32> {
        caller.require_auth();

        if params.long_token_amount == 0 && params.short_token_amount == 0 {
            panic_with_error!(&env, Error::ZeroDeposit);
        }

        let data_store: Address = env.storage().instance().get(&InstanceKey::DataStore).unwrap();
        let deposit_vault: Address = env.storage().instance().get(&InstanceKey::DepositVault).unwrap();
        let handler = env.current_contract_address();
        let ds = DataStoreClient::new(&env, &data_store);

        // Pull tokens from caller → deposit_vault
        if params.long_token_amount > 0 {
            token::Client::new(&env, &params.initial_long_token)
                .transfer(&caller, &deposit_vault, &params.long_token_amount);
        }
        if params.short_token_amount > 0 {
            token::Client::new(&env, &params.initial_short_token)
                .transfer(&caller, &deposit_vault, &params.short_token_amount);
        }

        // Allocate deposit key from nonce
        let nonce = ds.increment_nonce(&handler);
        let key = deposit_key(&env, nonce);

        // Build and store DepositProps
        let market_addr = params.market.clone();
        let deposit = DepositProps {
            account:              caller.clone(),
            receiver:             params.receiver,
            market:               params.market,
            initial_long_token:   params.initial_long_token,
            initial_short_token:  params.initial_short_token,
            long_token_amount:    params.long_token_amount,
            short_token_amount:   params.short_token_amount,
            min_market_tokens:    params.min_market_tokens,
            execution_fee:        params.execution_fee,
            updated_at_time:      env.ledger().timestamp(),
        };
        env.storage().persistent().set(&LocalKey::Deposit(key.clone()), &deposit);

        // Index in data_store
        ds.add_bytes32_to_set(&handler, &deposit_list_key(&env), &key);
        ds.add_bytes32_to_set(&handler, &account_deposit_list_key(&env, &caller), &key);

        env.events().publish((symbol_short!("dep_crt"),), (key.clone(), caller, market_addr));
        key
    }

    // ── Execute deposit ───────────────────────────────────────────────────────

    /// Keeper executes a pending deposit: mints LP tokens at current pool price.
    /// Oracle prices must already be set before calling this.
    pub fn execute_deposit(env: Env, keeper: Address, key: BytesN<32>) {
        keeper.require_auth();
        require_order_keeper(&env, &keeper);

        let data_store: Address = env.storage().instance().get(&InstanceKey::DataStore).unwrap();
        let deposit_vault: Address = env.storage().instance().get(&InstanceKey::DepositVault).unwrap();
        let oracle: Address = env.storage().instance().get(&InstanceKey::Oracle).unwrap();
        let handler = env.current_contract_address();

        // Load deposit
        let deposit: DepositProps = env.storage().persistent()
            .get(&LocalKey::Deposit(key.clone()))
            .unwrap_or_else(|| panic_with_error!(&env, Error::DepositNotFound));

        // Reconstruct MarketProps from data_store
        let market = load_market_props(&env, &data_store, &deposit.market);

        // Read prices from oracle
        let oracle_client = OracleClient::new(&env, &oracle);
        let long_price  = oracle_client.get_primary_price(&market.long_token).mid_price();
        let short_price = oracle_client.get_primary_price(&market.short_token).mid_price();
        let index_price = oracle_client.get_primary_price(&market.index_token).mid_price();

        // USD value of the incoming tokens (FLOAT_PRECISION)
        let long_usd = if deposit.long_token_amount > 0 {
            mul_div_wide(&env, deposit.long_token_amount, long_price, TOKEN_PRECISION)
        } else { 0 };
        let short_usd = if deposit.short_token_amount > 0 {
            mul_div_wide(&env, deposit.short_token_amount, short_price, TOKEN_PRECISION)
        } else { 0 };
        let deposit_usd = long_usd + short_usd;

        // Market token price BEFORE adding deposit (use minimize for conservative mint)
        let mt_price = get_market_token_price(
            &env, &data_store, &market,
            long_price, short_price, index_price,
            false, // minimize → fewer LP tokens (conservative for depositor)
        );

        // LP tokens to mint = deposit_usd × TOKEN_PRECISION / mt_price
        let mint_amount = mul_div_wide(&env, deposit_usd, TOKEN_PRECISION, mt_price);

        if mint_amount < deposit.min_market_tokens {
            panic_with_error!(&env, Error::InsufficientLpOut);
        }

        let vault_client = DepositVaultClient::new(&env, &deposit_vault);

        // Move pool tokens: vault → market_token contract (the pool)
        if deposit.long_token_amount > 0 {
            vault_client.transfer_out(
                &handler, &market.long_token,
                &market.market_token, &deposit.long_token_amount,
            );
            apply_delta_to_pool_amount(
                &env, &data_store, &handler, &market,
                &market.long_token, deposit.long_token_amount,
            );
        }
        if deposit.short_token_amount > 0 {
            vault_client.transfer_out(
                &handler, &market.short_token,
                &market.market_token, &deposit.short_token_amount,
            );
            apply_delta_to_pool_amount(
                &env, &data_store, &handler, &market,
                &market.short_token, deposit.short_token_amount,
            );
        }

        // Mint LP tokens to receiver
        MarketTokenClient::new(&env, &market.market_token)
            .mint(&handler, &deposit.receiver, &mint_amount);

        // Update market state
        let now = env.ledger().timestamp();
        update_funding_state(&env, &data_store, &handler, &market, long_price, short_price, now);
        update_cumulative_borrowing_factor(&env, &data_store, &handler, &market, true, now);
        update_cumulative_borrowing_factor(&env, &data_store, &handler, &market, false, now);

        // Clean up
        remove_deposit(&env, &data_store, &handler, &key, &deposit.account);

        env.events().publish(
            (symbol_short!("dep_exe"),),
            (key, deposit.receiver, mint_amount),
        );
    }

    // ── Cancel deposit ────────────────────────────────────────────────────────

    /// Cancel a pending deposit and refund tokens to the depositor.
    /// Callable by the depositor or any ORDER_KEEPER.
    pub fn cancel_deposit(env: Env, caller: Address, key: BytesN<32>) {
        caller.require_auth();

        let data_store: Address = env.storage().instance().get(&InstanceKey::DataStore).unwrap();
        let deposit_vault: Address = env.storage().instance().get(&InstanceKey::DepositVault).unwrap();
        let role_store: Address = env.storage().instance().get(&InstanceKey::RoleStore).unwrap();
        let handler = env.current_contract_address();

        let deposit: DepositProps = env.storage().persistent()
            .get(&LocalKey::Deposit(key.clone()))
            .unwrap_or_else(|| panic_with_error!(&env, Error::DepositNotFound));

        // Must be the depositor or a keeper
        let is_keeper = RoleStoreClient::new(&env, &role_store)
            .has_role(&caller, &roles::order_keeper(&env));
        if caller != deposit.account && !is_keeper {
            panic_with_error!(&env, Error::Unauthorized);
        }

        let vault_client = DepositVaultClient::new(&env, &deposit_vault);

        // Refund tokens
        if deposit.long_token_amount > 0 {
            vault_client.transfer_out(
                &handler, &deposit.initial_long_token,
                &deposit.account, &deposit.long_token_amount,
            );
        }
        if deposit.short_token_amount > 0 {
            vault_client.transfer_out(
                &handler, &deposit.initial_short_token,
                &deposit.account, &deposit.short_token_amount,
            );
        }

        remove_deposit(&env, &data_store, &handler, &key, &deposit.account);

        env.events().publish((symbol_short!("dep_can"),), (key, deposit.account));
    }

    // ── Views ─────────────────────────────────────────────────────────────────

    pub fn get_deposit(env: Env, key: BytesN<32>) -> Option<DepositProps> {
        env.storage().persistent().get(&LocalKey::Deposit(key))
    }
}

// ─── Internal helpers ─────────────────────────────────────────────────────────

fn require_order_keeper(env: &Env, caller: &Address) {
    let role_store: Address = env.storage().instance().get(&InstanceKey::RoleStore).unwrap();
    if !RoleStoreClient::new(env, &role_store).has_role(caller, &roles::order_keeper(env)) {
        panic_with_error!(env, Error::Unauthorized);
    }
}

fn load_market_props(env: &Env, data_store: &Address, market_token: &Address) -> MarketProps {
    let ds = DataStoreClient::new(env, data_store);
    MarketProps {
        market_token: market_token.clone(),
        index_token:  ds.get_address(&market_index_token_key(env, market_token)).unwrap(),
        long_token:   ds.get_address(&market_long_token_key(env, market_token)).unwrap(),
        short_token:  ds.get_address(&market_short_token_key(env, market_token)).unwrap(),
    }
}

fn remove_deposit(
    env: &Env,
    data_store: &Address,
    handler: &Address,
    key: &BytesN<32>,
    account: &Address,
) {
    env.storage().persistent().remove(&LocalKey::Deposit(key.clone()));
    let ds = DataStoreClient::new(env, data_store);
    ds.remove_bytes32_from_set(handler, &deposit_list_key(env), key);
    ds.remove_bytes32_from_set(handler, &account_deposit_list_key(env, account), key);
}

// ─── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use soroban_sdk::{testutils::Address as _, token::StellarAssetClient, Env};
    use role_store::{RoleStore, RoleStoreClient as RsClient};
    use data_store::{DataStore, DataStoreClient as DsClient};
    use oracle::{Oracle, OracleClient as OClient};
    use deposit_vault::{DepositVault, DepositVaultClient as DVClient};
    use market_token::{MarketToken, MarketTokenClient as MtClient};
    use gmx_keys::roles;

    struct World {
        env:       Env,
        admin:     Address,
        keeper:    Address,
        rs:        Address,
        ds:        Address,
        oracle:    Address,
        vault:     Address,
        handler:   Address,
        market_tk: Address,
        long_tk:   Address,
        short_tk:  Address,
        index_tk:  Address,
    }

    fn setup() -> World {
        let env = Env::default();
        env.mock_all_auths();

        let admin  = Address::generate(&env);
        let keeper = Address::generate(&env);

        // role_store
        let rs = env.register(RoleStore, ());
        RsClient::new(&env, &rs).initialize(&admin);
        let rs_c = RsClient::new(&env, &rs);
        rs_c.grant_role(&admin, &admin,  &roles::controller(&env));
        rs_c.grant_role(&admin, &keeper, &roles::order_keeper(&env));

        // data_store
        let ds = env.register(DataStore, ());
        DsClient::new(&env, &ds).initialize(&admin, &rs);
        // grant handler controller role later (after registering)

        // oracle
        let oracle_addr = env.register(Oracle, ());
        let passphrase = soroban_sdk::Bytes::from_slice(&env, b"Test SDF Network ; September 2015");
        OClient::new(&env, &oracle_addr).initialize(&admin, &rs, &ds, &passphrase);

        // deposit_vault
        let vault = env.register(DepositVault, ());
        DVClient::new(&env, &vault).initialize(&admin, &rs);

        // market_token (acts as pool + LP token)
        let market_tk = env.register(MarketToken, ());
        MtClient::new(&env, &market_tk).initialize(
            &admin, &rs, &7u32,
            &soroban_sdk::String::from_str(&env, "GMX Market Token"),
            &soroban_sdk::String::from_str(&env, "GM"),
        );

        // deposit_handler
        let handler = env.register(DepositHandler, ());
        DepositHandlerClient::new(&env, &handler).initialize(
            &admin, &rs, &ds, &oracle_addr, &vault,
        );

        // grant deposit_handler CONTROLLER role so it can write to data_store + mint
        rs_c.grant_role(&admin, &handler, &roles::controller(&env));

        // Fake token addresses (using generate — in a real test we'd use StellarAssetClient)
        let long_tk  = env.register_stellar_asset_contract_v2(admin.clone()).address();
        let short_tk = env.register_stellar_asset_contract_v2(admin.clone()).address();
        let index_tk = Address::generate(&env);

        // Store market props in data_store (normally done by market_factory)
        let ds_c = DsClient::new(&env, &ds);
        ds_c.set_address(&handler, &gmx_keys::market_index_token_key(&env, &market_tk), &index_tk);
        ds_c.set_address(&handler, &gmx_keys::market_long_token_key(&env, &market_tk), &long_tk);
        ds_c.set_address(&handler, &gmx_keys::market_short_token_key(&env, &market_tk), &short_tk);

        World { env, admin, keeper, rs, ds, oracle: oracle_addr, vault, handler, market_tk, long_tk, short_tk, index_tk }
    }

    #[test]
    fn create_and_cancel_deposit() {
        let w = setup();
        let env = &w.env;
        let user = Address::generate(env);

        // Mint long tokens to user
        StellarAssetClient::new(env, &w.long_tk).mint(&user, &1_000_0000i128);

        let handler_client = DepositHandlerClient::new(env, &w.handler);

        let key = handler_client.create_deposit(&user, &CreateDepositParams {
            receiver:            user.clone(),
            market:              w.market_tk.clone(),
            initial_long_token:  w.long_tk.clone(),
            initial_short_token: w.short_tk.clone(),
            long_token_amount:   1_000_0000i128,
            short_token_amount:  0,
            min_market_tokens:   0,
            execution_fee:       0,
        });

        // Deposit should be stored
        let dep = handler_client.get_deposit(&key).unwrap();
        assert_eq!(dep.long_token_amount, 1_000_0000);
        assert_eq!(dep.account, user);

        // Cancel — tokens returned to user
        handler_client.cancel_deposit(&user, &key);
        assert!(handler_client.get_deposit(&key).is_none());
    }

    #[test]
    fn execute_deposit_mints_lp_tokens() {
        let w = setup();
        let env = &w.env;
        let user = Address::generate(env);

        // Mint pool tokens to user
        StellarAssetClient::new(env, &w.long_tk).mint(&user, &1_000_0000i128);
        StellarAssetClient::new(env, &w.short_tk).mint(&user, &500_0000i128);

        // Set oracle prices: $2000 for long (ETH-like), $1 for short (USDC), $2000 for index
        let fp = gmx_math::FLOAT_PRECISION;
        OClient::new(env, &w.oracle).set_prices_simple(&w.keeper, &soroban_sdk::Vec::from_array(env, [
            TokenPrice { token: w.long_tk.clone(),  min: 2000 * fp, max: 2000 * fp },
            TokenPrice { token: w.short_tk.clone(), min: fp,        max: fp },
            TokenPrice { token: w.index_tk.clone(), min: 2000 * fp, max: 2000 * fp },
        ]));

        let handler_client = DepositHandlerClient::new(env, &w.handler);

        let key = handler_client.create_deposit(&user, &CreateDepositParams {
            receiver:            user.clone(),
            market:              w.market_tk.clone(),
            initial_long_token:  w.long_tk.clone(),
            initial_short_token: w.short_tk.clone(),
            long_token_amount:   1_000_0000i128,  // 1000 long tokens
            short_token_amount:  500_0000i128,    // 500 short tokens
            min_market_tokens:   1,
            execution_fee:       0,
        });

        handler_client.execute_deposit(&w.keeper, &key);

        // Check LP tokens were minted
        let lp_balance = MtClient::new(env, &w.market_tk).balance(&user);
        assert!(lp_balance > 0, "LP tokens should have been minted");

        // Deposit should be cleaned up
        assert!(handler_client.get_deposit(&key).is_none());

        // Pool amounts should be updated in data_store
        let ds_c = DsClient::new(env, &w.ds);
        let long_pool = ds_c.get_u128(&gmx_keys::pool_amount_key(env, &w.market_tk, &w.long_tk));
        let short_pool = ds_c.get_u128(&gmx_keys::pool_amount_key(env, &w.market_tk, &w.short_tk));
        assert_eq!(long_pool,  1_000_0000);
        assert_eq!(short_pool, 500_0000);
    }
}
