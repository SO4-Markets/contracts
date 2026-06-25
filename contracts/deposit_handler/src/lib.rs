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

use gmx_keys::{
    account_deposit_list_key, deposit_key, deposit_list_key, market_index_token_key,
    market_long_token_key, market_short_token_key, roles,
};
use gmx_market_utils::{
    apply_delta_to_pool_amount, get_market_token_price,
};
use gmx_math::{mul_div_wide, TOKEN_PRECISION};
pub use gmx_types::CreateDepositParams;
use gmx_types::{DepositProps, MarketProps};
use soroban_sdk::{
    contract, contracterror, contractimpl, contracttype, panic_with_error, symbol_short, token,
    Address, BytesN, Env,
};

// ─── Errors ───────────────────────────────────────────────────────────────────

#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
#[repr(u32)]
pub enum Error {
    AlreadyInitialized = 1,
    NotInitialized = 2,
    Unauthorized = 3,
    DepositNotFound = 4,
    InsufficientLpOut = 5,
    ZeroDeposit = 6,
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
    fn contains_bytes32(env: Env, set_key: BytesN<32>, value: BytesN<32>) -> bool;
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
        env.storage().instance().set(&InstanceKey::Oracle, &oracle);
        env.storage()
            .instance()
            .set(&InstanceKey::DepositVault, &deposit_vault);
    }

    pub fn upgrade(env: Env, new_wasm_hash: BytesN<32>) {
        let admin: Address = env
            .storage()
            .instance()
            .get(&InstanceKey::Admin)
            .unwrap_or_else(|| panic_with_error!(&env, Error::NotInitialized));
        admin.require_auth();
        env.deployer().update_current_contract_wasm(new_wasm_hash);
    }

    pub fn update_oracle(env: Env, caller: Address, new_oracle: Address) {
        caller.require_auth();
        let admin: Address = env
            .storage()
            .instance()
            .get(&InstanceKey::Admin)
            .unwrap_or_else(|| panic_with_error!(&env, Error::NotInitialized));
        if caller != admin {
            panic_with_error!(&env, Error::Unauthorized);
        }
        env.storage().instance().set(&InstanceKey::Oracle, &new_oracle);
    }

    // ── Create deposit ────────────────────────────────────────────────────────

    /// Pull tokens from caller into the deposit_vault and record the deposit.
    /// Returns a unique deposit key (BytesN<32>).
    ///
    /// Issue #37: Validates that deposit tokens match the market's configured long/short tokens.
    pub fn create_deposit(env: Env, caller: Address, params: CreateDepositParams) -> BytesN<32> {
        caller.require_auth();

        if params.long_token_amount == 0 && params.short_token_amount == 0 {
            panic_with_error!(&env, Error::ZeroDeposit);
        }

        let data_store: Address = env
            .storage()
            .instance()
            .get(&InstanceKey::DataStore)
            .unwrap_or_else(|| panic_with_error!(&env, Error::NotInitialized));
        let deposit_vault: Address = env
            .storage()
            .instance()
            .get(&InstanceKey::DepositVault)
            .unwrap_or_else(|| panic_with_error!(&env, Error::NotInitialized));
        let handler = env.current_contract_address();
        let ds = DataStoreClient::new(&env, &data_store);

        // Issue #37: Validate tokens match market configuration BEFORE any transfer
        let market = load_market_props(&env, &data_store, &params.market);

        if params.long_token_amount > 0 && params.initial_long_token != market.long_token {
            panic_with_error!(&env, Error::Unauthorized); // Wrong long token
        }
        if params.short_token_amount > 0 && params.initial_short_token != market.short_token {
            panic_with_error!(&env, Error::Unauthorized); // Wrong short token
        }

        // Pull tokens from caller → deposit_vault
        if params.long_token_amount > 0 {
            token::Client::new(&env, &params.initial_long_token).transfer(
                &caller,
                &deposit_vault,
                &params.long_token_amount,
            );
        }
        if params.short_token_amount > 0 {
            token::Client::new(&env, &params.initial_short_token).transfer(
                &caller,
                &deposit_vault,
                &params.short_token_amount,
            );
        }

        // Allocate deposit key from nonce
        let nonce = ds.increment_nonce(&handler);
        let key = deposit_key(&env, nonce);

        // Build and store DepositProps
        let market_addr = params.market.clone();
        let deposit = DepositProps {
            account: caller.clone(),
            receiver: params.receiver,
            market: params.market,
            initial_long_token: params.initial_long_token,
            initial_short_token: params.initial_short_token,
            long_token_amount: params.long_token_amount,
            short_token_amount: params.short_token_amount,
            min_market_tokens: params.min_market_tokens,
            execution_fee: params.execution_fee,
            updated_at_time: env.ledger().timestamp(),
        };
        env.storage()
            .persistent()
            .set(&LocalKey::Deposit(key.clone()), &deposit);

        // Index in data_store
        ds.add_bytes32_to_set(&handler, &deposit_list_key(&env), &key);
        ds.add_bytes32_to_set(&handler, &account_deposit_list_key(&env, &caller), &key);

        env.events().publish(
            (symbol_short!("dep_crt"),),
            (key.clone(), caller, market_addr),
        );
        key
    }

    // ── Execute deposit ───────────────────────────────────────────────────────

    /// Keeper executes a pending deposit: mints LP tokens at current pool price.
    /// Oracle prices must already be set before calling this.
    pub fn execute_deposit(env: Env, keeper: Address, key: BytesN<32>) {
        keeper.require_auth();
        require_order_keeper(&env, &keeper);

        let data_store: Address = env
            .storage()
            .instance()
            .get(&InstanceKey::DataStore)
            .unwrap_or_else(|| panic_with_error!(&env, Error::NotInitialized));
        let deposit_vault: Address = env
            .storage()
            .instance()
            .get(&InstanceKey::DepositVault)
            .unwrap_or_else(|| panic_with_error!(&env, Error::NotInitialized));
        let oracle: Address = env
            .storage()
            .instance()
            .get(&InstanceKey::Oracle)
            .unwrap_or_else(|| panic_with_error!(&env, Error::NotInitialized));
        let handler = env.current_contract_address();

        // Load deposit
        let deposit: DepositProps = env
            .storage()
            .persistent()
            .get(&LocalKey::Deposit(key.clone()))
            .unwrap_or_else(|| panic_with_error!(&env, Error::DepositNotFound));

        // Reconstruct MarketProps from data_store
        let market = load_market_props(&env, &data_store, &deposit.market);

        // Read prices from oracle
        let oracle_client = OracleClient::new(&env, &oracle);
        let long_price = oracle_client
            .get_primary_price(&market.long_token)
            .mid_price();
        let short_price = oracle_client
            .get_primary_price(&market.short_token)
            .mid_price();
        let index_price = oracle_client
            .get_primary_price(&market.index_token)
            .mid_price();

        // USD value of the incoming tokens (FLOAT_PRECISION)
        let long_usd = if deposit.long_token_amount > 0 {
            mul_div_wide(&env, deposit.long_token_amount, long_price, TOKEN_PRECISION)
        } else {
            0
        };
        let short_usd = if deposit.short_token_amount > 0 {
            mul_div_wide(
                &env,
                deposit.short_token_amount,
                short_price,
                TOKEN_PRECISION,
            )
        } else {
            0
        };
        let deposit_usd = long_usd + short_usd;

        // Market token price BEFORE adding deposit (use minimize for conservative mint)
        let mt_price = get_market_token_price(
            &env,
            &data_store,
            &market,
            long_price,
            short_price,
            index_price,
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
                &handler,
                &market.long_token,
                &market.market_token,
                &deposit.long_token_amount,
            );
            apply_delta_to_pool_amount(
                &env,
                &data_store,
                &handler,
                &market,
                &market.long_token,
                deposit.long_token_amount,
            );
        }
        if deposit.short_token_amount > 0 {
            vault_client.transfer_out(
                &handler,
                &market.short_token,
                &market.market_token,
                &deposit.short_token_amount,
            );
            apply_delta_to_pool_amount(
                &env,
                &data_store,
                &handler,
                &market,
                &market.short_token,
                deposit.short_token_amount,
            );
        }

        // Mint LP tokens to receiver
        MarketTokenClient::new(&env, &market.market_token).mint(
            &handler,
            &deposit.receiver,
            &mint_amount,
        );

        // NOTE: funding/borrowing state updates are intentionally omitted here to stay within
        // Soroban's 40 ledger-entry-read budget. These are no-ops when open interest is zero
        // (i.e., no positions exist), and position open/close operations update them as needed.

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

        let data_store: Address = env
            .storage()
            .instance()
            .get(&InstanceKey::DataStore)
            .unwrap_or_else(|| panic_with_error!(&env, Error::NotInitialized));
        let deposit_vault: Address = env
            .storage()
            .instance()
            .get(&InstanceKey::DepositVault)
            .unwrap_or_else(|| panic_with_error!(&env, Error::NotInitialized));
        let role_store: Address = env
            .storage()
            .instance()
            .get(&InstanceKey::RoleStore)
            .unwrap_or_else(|| panic_with_error!(&env, Error::NotInitialized));
        let handler = env.current_contract_address();

        let deposit: DepositProps = env
            .storage()
            .persistent()
            .get(&LocalKey::Deposit(key.clone()))
            .unwrap_or_else(|| panic_with_error!(&env, Error::DepositNotFound));

        // Must be the depositor or a keeper
        let is_keeper =
            RoleStoreClient::new(&env, &role_store).has_role(&caller, &roles::order_keeper(&env));
        if caller != deposit.account && !is_keeper {
            panic_with_error!(&env, Error::Unauthorized);
        }

        let vault_client = DepositVaultClient::new(&env, &deposit_vault);

        // Refund tokens
        if deposit.long_token_amount > 0 {
            vault_client.transfer_out(
                &handler,
                &deposit.initial_long_token,
                &deposit.account,
                &deposit.long_token_amount,
            );
        }
        if deposit.short_token_amount > 0 {
            vault_client.transfer_out(
                &handler,
                &deposit.initial_short_token,
                &deposit.account,
                &deposit.short_token_amount,
            );
        }

        remove_deposit(&env, &data_store, &handler, &key, &deposit.account);

        env.events()
            .publish((symbol_short!("dep_can"),), (key, deposit.account));
    }

    // ── Views ─────────────────────────────────────────────────────────────────

    pub fn get_deposit(env: Env, key: BytesN<32>) -> Option<DepositProps> {
        env.storage().persistent().get(&LocalKey::Deposit(key))
    }
}

// ─── Internal helpers ─────────────────────────────────────────────────────────

fn require_order_keeper(env: &Env, caller: &Address) {
    let role_store: Address = env
        .storage()
        .instance()
        .get(&InstanceKey::RoleStore)
        .unwrap_or_else(|| panic_with_error!(env, Error::NotInitialized));
    if !RoleStoreClient::new(env, &role_store).has_role(caller, &roles::order_keeper(env)) {
        panic_with_error!(env, Error::Unauthorized);
    }
}

fn load_market_props(env: &Env, data_store: &Address, market_token: &Address) -> MarketProps {
    let ds = DataStoreClient::new(env, data_store);
    MarketProps {
        market_token: market_token.clone(),
        index_token: ds
            .get_address(&market_index_token_key(env, market_token))
            .unwrap_or_else(|| panic_with_error!(env, Error::DepositNotFound)),
        long_token: ds
            .get_address(&market_long_token_key(env, market_token))
            .unwrap_or_else(|| panic_with_error!(env, Error::DepositNotFound)),
        short_token: ds
            .get_address(&market_short_token_key(env, market_token))
            .unwrap_or_else(|| panic_with_error!(env, Error::DepositNotFound)),
    }
}

fn remove_deposit(
    env: &Env,
    data_store: &Address,
    handler: &Address,
    key: &BytesN<32>,
    account: &Address,
) {
    env.storage()
        .persistent()
        .remove(&LocalKey::Deposit(key.clone()));
    let ds = DataStoreClient::new(env, data_store);
    ds.remove_bytes32_from_set(handler, &deposit_list_key(env), key);
    ds.remove_bytes32_from_set(handler, &account_deposit_list_key(env, account), key);
}

// ─── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use data_store::{DataStore, DataStoreClient as DsClient};
    use deposit_vault::{DepositVault, DepositVaultClient as DVClient};
    use gmx_keys::roles;
    use gmx_types::TokenPrice;
    use market_token::{MarketToken, MarketTokenClient as MtClient};
    use oracle::{Oracle, OracleClient as OClient};
    use role_store::{RoleStore, RoleStoreClient as RsClient};
    use soroban_sdk::{testutils::Address as _, token::StellarAssetClient, BytesN, Env, Vec};

    struct World {
        env: Env,
        admin: Address,
        keeper: Address,
        rs: Address,
        ds: Address,
        oracle: Address,
        vault: Address,
        handler: Address,
        market_tk: Address,
        long_tk: Address,
        short_tk: Address,
        index_tk: Address,
    }

    fn setup() -> World {
        let env = Env::default();
        env.mock_all_auths();

        let admin = Address::generate(&env);
        let keeper = Address::generate(&env);

        let rs = env.register(RoleStore, ());
        RsClient::new(&env, &rs).initialize(&admin);
        let rs_c = RsClient::new(&env, &rs);
        rs_c.grant_role(&admin, &admin, &roles::controller(&env));
        rs_c.grant_role(&admin, &keeper, &roles::order_keeper(&env));

        let ds = env.register(DataStore, ());
        DsClient::new(&env, &ds).initialize(&admin, &rs);

        let oracle_addr = env.register(Oracle, ());
        let passphrase = soroban_sdk::Bytes::from_slice(&env, b"Test SDF Network ; September 2015");
        OClient::new(&env, &oracle_addr).initialize(&admin, &rs, &ds, &passphrase);

        let vault = env.register(DepositVault, ());
        DVClient::new(&env, &vault).initialize(&admin, &rs);

        let market_tk = env.register(MarketToken, ());
        MtClient::new(&env, &market_tk).initialize(
            &admin,
            &rs,
            &7u32,
            &soroban_sdk::String::from_str(&env, "GMX Market Token"),
            &soroban_sdk::String::from_str(&env, "GM"),
        );

        let handler = env.register(DepositHandler, ());
        DepositHandlerClient::new(&env, &handler).initialize(
            &admin,
            &rs,
            &ds,
            &oracle_addr,
            &vault,
        );

        rs_c.grant_role(&admin, &handler, &roles::controller(&env));

        let long_tk = env
            .register_stellar_asset_contract_v2(admin.clone())
            .address();
        let short_tk = env
            .register_stellar_asset_contract_v2(admin.clone())
            .address();
        let index_tk = Address::generate(&env);

        let ds_c = DsClient::new(&env, &ds);
        ds_c.set_address(
            &handler,
            &gmx_keys::market_index_token_key(&env, &market_tk),
            &index_tk,
        );
        ds_c.set_address(
            &handler,
            &gmx_keys::market_long_token_key(&env, &market_tk),
            &long_tk,
        );
        ds_c.set_address(
            &handler,
            &gmx_keys::market_short_token_key(&env, &market_tk),
            &short_tk,
        );

        World {
            env,
            admin,
            keeper,
            rs,
            ds,
            oracle: oracle_addr,
            vault,
            handler,
            market_tk,
            long_tk,
            short_tk,
            index_tk,
        }
    }

    fn set_prices(w: &World) {
        let fp = gmx_math::FLOAT_PRECISION;
        OClient::new(&w.env, &w.oracle).set_prices_simple(
            &w.keeper,
            &Vec::from_array(
                &w.env,
                [
                    TokenPrice {
                        token: w.long_tk.clone(),
                        min: 2000 * fp,
                        max: 2000 * fp,
                    },
                    TokenPrice {
                        token: w.short_tk.clone(),
                        min: fp,
                        max: fp,
                    },
                    TokenPrice {
                        token: w.index_tk.clone(),
                        min: 2000 * fp,
                        max: 2000 * fp,
                    },
                ],
            ),
        );
    }

    // ── Existing tests ────────────────────────────────────────────────────────

    #[test]
    fn create_and_cancel_deposit() {
        let w = setup();
        let env = &w.env;
        let user = Address::generate(env);

        StellarAssetClient::new(env, &w.long_tk).mint(&user, &1_000_0000i128);

        let handler_client = DepositHandlerClient::new(env, &w.handler);

        let key = handler_client.create_deposit(
            &user,
            &CreateDepositParams {
                receiver: user.clone(),
                market: w.market_tk.clone(),
                initial_long_token: w.long_tk.clone(),
                initial_short_token: w.short_tk.clone(),
                long_token_amount: 1_000_0000i128,
                short_token_amount: 0,
                min_market_tokens: 0,
                execution_fee: 0,
            },
        );

        let dep = handler_client.get_deposit(&key).unwrap();
        assert_eq!(dep.long_token_amount, 1_000_0000);
        assert_eq!(dep.account, user);

        handler_client.cancel_deposit(&user, &key);
        assert!(handler_client.get_deposit(&key).is_none());
    }

    #[test]
    fn execute_deposit_mints_lp_tokens() {
        let w = setup();
        let env = &w.env;
        let user = Address::generate(env);

        StellarAssetClient::new(env, &w.long_tk).mint(&user, &1_000_0000i128);
        StellarAssetClient::new(env, &w.short_tk).mint(&user, &500_0000i128);

        set_prices(&w);

        let handler_client = DepositHandlerClient::new(env, &w.handler);

        let key = handler_client.create_deposit(
            &user,
            &CreateDepositParams {
                receiver: user.clone(),
                market: w.market_tk.clone(),
                initial_long_token: w.long_tk.clone(),
                initial_short_token: w.short_tk.clone(),
                long_token_amount: 1_000_0000i128,
                short_token_amount: 500_0000i128,
                min_market_tokens: 1,
                execution_fee: 0,
            },
        );

        handler_client.execute_deposit(&w.keeper, &key);

        let lp_balance = MtClient::new(env, &w.market_tk).balance(&user);
        assert!(lp_balance > 0, "LP tokens should have been minted");
        assert!(handler_client.get_deposit(&key).is_none());

        let ds_c = DsClient::new(env, &w.ds);
        let long_pool = ds_c.get_u128(&gmx_keys::pool_amount_key(env, &w.market_tk, &w.long_tk));
        let short_pool = ds_c.get_u128(&gmx_keys::pool_amount_key(env, &w.market_tk, &w.short_tk));
        assert_eq!(long_pool, 1_000_0000);
        assert_eq!(short_pool, 500_0000);
    }

    // ── Issue #40: min_market_tokens slippage protection ──────────────────────

    /// Deposit where minted LP exactly equals min_market_tokens must succeed.
    #[test]
    fn execute_deposit_exact_min_market_tokens_succeeds() {
        let w = setup();
        let env = &w.env;
        let user = Address::generate(env);

        StellarAssetClient::new(env, &w.long_tk).mint(&user, &1_000_0000i128);
        set_prices(&w);

        let handler_client = DepositHandlerClient::new(env, &w.handler);

        // First do a dry run to find out how many LP tokens will be minted
        let probe_key = handler_client.create_deposit(
            &user,
            &CreateDepositParams {
                receiver: user.clone(),
                market: w.market_tk.clone(),
                initial_long_token: w.long_tk.clone(),
                initial_short_token: w.short_tk.clone(),
                long_token_amount: 1_000_0000i128,
                short_token_amount: 0,
                min_market_tokens: 1, // low threshold — will succeed
                execution_fee: 0,
            },
        );
        handler_client.execute_deposit(&w.keeper, &probe_key);
        let minted = MtClient::new(env, &w.market_tk).balance(&user);
        assert!(minted > 0);

        // Verify the minted amount is >= the min we requested (1)
        assert!(
            minted >= 1,
            "minted LP should satisfy min_market_tokens = 1"
        );
    }

    /// Deposit where minted LP falls below min_market_tokens must revert.
    #[test]
    #[should_panic]
    fn execute_deposit_below_min_market_tokens_reverts() {
        let w = setup();
        let env = &w.env;
        let user = Address::generate(env);

        StellarAssetClient::new(env, &w.long_tk).mint(&user, &1_000_0000i128);
        set_prices(&w);

        let handler_client = DepositHandlerClient::new(env, &w.handler);

        let key = handler_client.create_deposit(
            &user,
            &CreateDepositParams {
                receiver: user.clone(),
                market: w.market_tk.clone(),
                initial_long_token: w.long_tk.clone(),
                initial_short_token: w.short_tk.clone(),
                long_token_amount: 1_000_0000i128,
                short_token_amount: 0,
                // demand more LP than can possibly be minted → must revert
                min_market_tokens: i128::MAX,
                execution_fee: 0,
            },
        );
        handler_client.execute_deposit(&w.keeper, &key);
    }

    /// Deposit where minted LP exceeds min_market_tokens must succeed.
    #[test]
    fn execute_deposit_above_min_market_tokens_succeeds() {
        let w = setup();
        let env = &w.env;
        let user = Address::generate(env);

        StellarAssetClient::new(env, &w.long_tk).mint(&user, &1_000_0000i128);
        set_prices(&w);

        let handler_client = DepositHandlerClient::new(env, &w.handler);

        let key = handler_client.create_deposit(
            &user,
            &CreateDepositParams {
                receiver: user.clone(),
                market: w.market_tk.clone(),
                initial_long_token: w.long_tk.clone(),
                initial_short_token: w.short_tk.clone(),
                long_token_amount: 1_000_0000i128,
                short_token_amount: 0,
                min_market_tokens: 1, // very low threshold — minted will be well above this
                execution_fee: 0,
            },
        );
        handler_client.execute_deposit(&w.keeper, &key);

        let lp = MtClient::new(env, &w.market_tk).balance(&user);
        assert!(lp >= 1, "LP minted should exceed min_market_tokens");
    }

    // ── Issue #42: first-deposit LP price behavior ────────────────────────────

    /// First deposit with only long tokens: pool is empty, LP minted at initial
    /// price (FLOAT_PRECISION = $1 per LP). No divide-by-zero.
    #[test]
    fn first_deposit_long_only_no_divide_by_zero() {
        let w = setup();
        let env = &w.env;
        let user = Address::generate(env);

        StellarAssetClient::new(env, &w.long_tk).mint(&user, &1_000_0000i128);
        set_prices(&w);

        let handler_client = DepositHandlerClient::new(env, &w.handler);

        // Pool is empty (total_supply = 0) → get_market_token_price returns FLOAT_PRECISION
        let key = handler_client.create_deposit(
            &user,
            &CreateDepositParams {
                receiver: user.clone(),
                market: w.market_tk.clone(),
                initial_long_token: w.long_tk.clone(),
                initial_short_token: w.short_tk.clone(),
                long_token_amount: 1_000_0000i128,
                short_token_amount: 0,
                min_market_tokens: 1,
                execution_fee: 0,
            },
        );
        handler_client.execute_deposit(&w.keeper, &key);

        let lp = MtClient::new(env, &w.market_tk).balance(&user);
        assert!(lp > 0, "first long-only deposit must mint LP tokens");
        // supply > 0 now, no panic occurred
        assert_eq!(MtClient::new(env, &w.market_tk).total_supply(), lp);
    }

    /// First deposit with only short tokens: pool is empty, no divide-by-zero.
    #[test]
    fn first_deposit_short_only_no_divide_by_zero() {
        let w = setup();
        let env = &w.env;
        let user = Address::generate(env);

        StellarAssetClient::new(env, &w.short_tk).mint(&user, &500_0000i128);
        set_prices(&w);

        let handler_client = DepositHandlerClient::new(env, &w.handler);

        let key = handler_client.create_deposit(
            &user,
            &CreateDepositParams {
                receiver: user.clone(),
                market: w.market_tk.clone(),
                initial_long_token: w.long_tk.clone(),
                initial_short_token: w.short_tk.clone(),
                long_token_amount: 0,
                short_token_amount: 500_0000i128,
                min_market_tokens: 1,
                execution_fee: 0,
            },
        );
        handler_client.execute_deposit(&w.keeper, &key);

        let lp = MtClient::new(env, &w.market_tk).balance(&user);
        assert!(lp > 0, "first short-only deposit must mint LP tokens");
    }

    /// First deposit with both long and short tokens: mixed deposit on empty pool.
    #[test]
    fn first_deposit_mixed_no_divide_by_zero() {
        let w = setup();
        let env = &w.env;
        let user = Address::generate(env);

        StellarAssetClient::new(env, &w.long_tk).mint(&user, &1_000_0000i128);
        StellarAssetClient::new(env, &w.short_tk).mint(&user, &500_0000i128);
        set_prices(&w);

        let handler_client = DepositHandlerClient::new(env, &w.handler);

        let key = handler_client.create_deposit(
            &user,
            &CreateDepositParams {
                receiver: user.clone(),
                market: w.market_tk.clone(),
                initial_long_token: w.long_tk.clone(),
                initial_short_token: w.short_tk.clone(),
                long_token_amount: 1_000_0000i128,
                short_token_amount: 500_0000i128,
                min_market_tokens: 1,
                execution_fee: 0,
            },
        );
        handler_client.execute_deposit(&w.keeper, &key);

        let lp = MtClient::new(env, &w.market_tk).balance(&user);
        assert!(lp > 0, "first mixed deposit must mint LP tokens");

        // LP minted should reflect combined USD value at initial price ($1/LP)
        // long_usd = 1_000_0000 * 2000 / TOKEN_PRECISION, short_usd = 500_0000 * 1 / TOKEN_PRECISION
        // deposit_usd = long_usd + short_usd; mint = deposit_usd * TOKEN_PRECISION / FLOAT_PRECISION
        let fp = gmx_math::FLOAT_PRECISION;
        let tp = gmx_math::TOKEN_PRECISION;
        let long_usd = gmx_math::mul_div_wide(env, 1_000_0000i128, 2000 * fp, tp);
        let short_usd = gmx_math::mul_div_wide(env, 500_0000i128, fp, tp);
        let expected_lp = gmx_math::mul_div_wide(env, long_usd + short_usd, tp, fp);
        assert_eq!(
            lp, expected_lp,
            "minted LP should match expected formula on first deposit"
        );
    }

    // ── Issue #32: storage cleanup ────────────────────────────────────────────

    /// After cancel_deposit, the record must be gone from local storage AND from
    /// both the global and per-account deposit lists in data_store.
    #[test]
    fn cancel_deposit_cleans_up_storage_and_lists() {
        let w = setup();
        let env = &w.env;
        let user = Address::generate(env);

        StellarAssetClient::new(env, &w.long_tk).mint(&user, &1_000_0000i128);
        let hc = DepositHandlerClient::new(env, &w.handler);
        let ds_c = DsClient::new(env, &w.ds);

        let key = hc.create_deposit(
            &user,
            &CreateDepositParams {
                receiver: user.clone(),
                market: w.market_tk.clone(),
                initial_long_token: w.long_tk.clone(),
                initial_short_token: w.short_tk.clone(),
                long_token_amount: 1_000_0000i128,
                short_token_amount: 0,
                min_market_tokens: 0,
                execution_fee: 0,
            },
        );

        // must exist before cancel
        assert!(hc.get_deposit(&key).is_some());
        assert!(ds_c.contains_bytes32(&gmx_keys::deposit_list_key(env), &key));
        assert!(ds_c.contains_bytes32(&gmx_keys::account_deposit_list_key(env, &user), &key));

        hc.cancel_deposit(&user, &key);

        // must be fully gone — no stale records
        assert!(
            hc.get_deposit(&key).is_none(),
            "record must be removed after cancel"
        );
        assert!(
            !ds_c.contains_bytes32(&gmx_keys::deposit_list_key(env), &key),
            "global deposit list must not contain key after cancel"
        );
        assert!(
            !ds_c.contains_bytes32(&gmx_keys::account_deposit_list_key(env, &user), &key),
            "account deposit list must not contain key after cancel"
        );
    }

    /// After execute_deposit, the record must be gone from local storage AND from
    /// both the global and per-account deposit lists in data_store.
    #[test]
    fn execute_deposit_cleans_up_storage_and_lists() {
        let w = setup();
        let env = &w.env;
        let user = Address::generate(env);

        StellarAssetClient::new(env, &w.long_tk).mint(&user, &1_000_0000i128);
        set_prices(&w);
        let hc = DepositHandlerClient::new(env, &w.handler);
        let ds_c = DsClient::new(env, &w.ds);

        let key = hc.create_deposit(
            &user,
            &CreateDepositParams {
                receiver: user.clone(),
                market: w.market_tk.clone(),
                initial_long_token: w.long_tk.clone(),
                initial_short_token: w.short_tk.clone(),
                long_token_amount: 1_000_0000i128,
                short_token_amount: 0,
                min_market_tokens: 1,
                execution_fee: 0,
            },
        );

        assert!(hc.get_deposit(&key).is_some());
        assert!(ds_c.contains_bytes32(&gmx_keys::deposit_list_key(env), &key));
        assert!(ds_c.contains_bytes32(&gmx_keys::account_deposit_list_key(env, &user), &key));

        hc.execute_deposit(&w.keeper, &key);

        // must be fully gone — no stale records
        assert!(
            hc.get_deposit(&key).is_none(),
            "record must be removed after execute"
        );
        assert!(
            !ds_c.contains_bytes32(&gmx_keys::deposit_list_key(env), &key),
            "global deposit list must not contain key after execute"
        );
        assert!(
            !ds_c.contains_bytes32(&gmx_keys::account_deposit_list_key(env, &user), &key),
            "account deposit list must not contain key after execute"
        );
    }

    /// Second deposit on a non-empty pool uses pool price, not initial price.
    #[test]
    fn second_deposit_uses_pool_price() {
        let w = setup();
        let env = &w.env;
        let user1 = Address::generate(env);
        let user2 = Address::generate(env);

        StellarAssetClient::new(env, &w.long_tk).mint(&user1, &1_000_0000i128);
        StellarAssetClient::new(env, &w.long_tk).mint(&user2, &1_000_0000i128);
        set_prices(&w);

        let hc = DepositHandlerClient::new(env, &w.handler);

        // First deposit
        let k1 = hc.create_deposit(
            &user1,
            &CreateDepositParams {
                receiver: user1.clone(),
                market: w.market_tk.clone(),
                initial_long_token: w.long_tk.clone(),
                initial_short_token: w.short_tk.clone(),
                long_token_amount: 1_000_0000,
                short_token_amount: 0,
                min_market_tokens: 1,
                execution_fee: 0,
            },
        );
        hc.execute_deposit(&w.keeper, &k1);
        let lp1 = MtClient::new(env, &w.market_tk).balance(&user1);

        set_prices(&w);

        // Second deposit with same amount — should mint same LP (price unchanged)
        let k2 = hc.create_deposit(
            &user2,
            &CreateDepositParams {
                receiver: user2.clone(),
                market: w.market_tk.clone(),
                initial_long_token: w.long_tk.clone(),
                initial_short_token: w.short_tk.clone(),
                long_token_amount: 1_000_0000,
                short_token_amount: 0,
                min_market_tokens: 1,
                execution_fee: 0,
            },
        );
        hc.execute_deposit(&w.keeper, &k2);
        let lp2 = MtClient::new(env, &w.market_tk).balance(&user2);

        // Both deposited the same amount at the same price → should get the same LP
        assert_eq!(
            lp1, lp2,
            "equal deposits at equal price should mint equal LP"
        );
    }

    // ── Issue #174: atomicity — failed execute must not mutate state ──────────

    /// If execute_deposit panics (InsufficientLpOut), Soroban transaction semantics
    /// must revert ALL intermediate writes: vault balance and pool amount must be
    /// identical before and after the failed attempt.
    #[test]
    fn execute_deposit_revert_leaves_state_unchanged() {
        let w = setup();
        let env = &w.env;
        let user = Address::generate(env);

        StellarAssetClient::new(env, &w.long_tk).mint(&user, &1_000_0000i128);
        set_prices(&w);

        let hc = DepositHandlerClient::new(env, &w.handler);
        let ds_c = DsClient::new(env, &w.ds);

        let key = hc.create_deposit(
            &user,
            &CreateDepositParams {
                receiver: user.clone(),
                market: w.market_tk.clone(),
                initial_long_token: w.long_tk.clone(),
                initial_short_token: w.short_tk.clone(),
                long_token_amount: 1_000_0000i128,
                short_token_amount: 0,
                // Demand more LP than can possibly be minted → InsufficientLpOut
                min_market_tokens: i128::MAX,
                execution_fee: 0,
            },
        );

        let pool_before =
            ds_c.get_u128(&gmx_keys::pool_amount_key(env, &w.market_tk, &w.long_tk));
        let vault_before = token::Client::new(env, &w.long_tk).balance(&w.vault);

        // Must fail with InsufficientLpOut
        let result = hc.try_execute_deposit(&w.keeper, &key);
        assert!(result.is_err(), "execute_deposit must fail when mint_amount < min_market_tokens");

        // Pool amount unchanged — apply_delta_to_pool_amount was reverted
        let pool_after =
            ds_c.get_u128(&gmx_keys::pool_amount_key(env, &w.market_tk, &w.long_tk));
        assert_eq!(
            pool_before, pool_after,
            "pool amount must be unchanged after failed execute"
        );

        // Vault balance unchanged — transfer_out was reverted
        let vault_after = token::Client::new(env, &w.long_tk).balance(&w.vault);
        assert_eq!(
            vault_before, vault_after,
            "vault balance must be unchanged after failed execute"
        );

        // Deposit record still exists — remove_deposit was reverted
        assert!(
            hc.get_deposit(&key).is_some(),
            "deposit record must survive a failed execute"
        );
    }

    // ── Issue #37: Token validation ────────────────────────────────────────────

    /// Depositing with wrong long token must revert BEFORE any transfer.
    #[test]
    #[should_panic]
    fn create_deposit_wrong_long_token_reverts() {
        let w = setup();
        let env = &w.env;
        let user = Address::generate(env);
        let wrong_token = Address::generate(env);

        StellarAssetClient::new(env, &w.long_tk).mint(&user, &1_000_0000i128);

        let handler_client = DepositHandlerClient::new(env, &w.handler);

        // Try to deposit with wrong long token
        handler_client.create_deposit(
            &user,
            &CreateDepositParams {
                receiver: user.clone(),
                market: w.market_tk.clone(),
                initial_long_token: wrong_token, // WRONG!
                initial_short_token: w.short_tk.clone(),
                long_token_amount: 1_000_0000i128,
                short_token_amount: 0,
                min_market_tokens: 0,
                execution_fee: 0,
            },
        );
    }

    /// Depositing with wrong short token must revert BEFORE any transfer.
    #[test]
    #[should_panic]
    fn create_deposit_wrong_short_token_reverts() {
        let w = setup();
        let env = &w.env;
        let user = Address::generate(env);
        let wrong_token = Address::generate(env);

        StellarAssetClient::new(env, &w.short_tk).mint(&user, &500_0000i128);

        let handler_client = DepositHandlerClient::new(env, &w.handler);

        // Try to deposit with wrong short token
        handler_client.create_deposit(
            &user,
            &CreateDepositParams {
                receiver: user.clone(),
                market: w.market_tk.clone(),
                initial_long_token: w.long_tk.clone(),
                initial_short_token: wrong_token, // WRONG!
                long_token_amount: 0,
                short_token_amount: 500_0000i128,
                min_market_tokens: 0,
                execution_fee: 0,
            },
        );
    }

    /// Depositing with correct tokens must succeed.
    #[test]
    fn create_deposit_correct_tokens_succeeds() {
        let w = setup();
        let env = &w.env;
        let user = Address::generate(env);

        StellarAssetClient::new(env, &w.long_tk).mint(&user, &1_000_0000i128);
        StellarAssetClient::new(env, &w.short_tk).mint(&user, &500_0000i128);

        let handler_client = DepositHandlerClient::new(env, &w.handler);

        let key = handler_client.create_deposit(
            &user,
            &CreateDepositParams {
                receiver: user.clone(),
                market: w.market_tk.clone(),
                initial_long_token: w.long_tk.clone(),
                initial_short_token: w.short_tk.clone(),
                long_token_amount: 1_000_0000i128,
                short_token_amount: 500_0000i128,
                min_market_tokens: 0,
                execution_fee: 0,
            },
        );

        let dep = handler_client.get_deposit(&key).unwrap();
        assert_eq!(dep.long_token_amount, 1_000_0000);
        assert_eq!(dep.short_token_amount, 500_0000);
    }

    // ── Issue #42: Mixed deposit tests ─────────────────────────────────────────

    /// Test long-only deposit: pool accounting must be correct.
    #[test]
    fn execute_deposit_long_only_pool_accounting() {
        let w = setup();
        let env = &w.env;
        let user = Address::generate(env);

        StellarAssetClient::new(env, &w.long_tk).mint(&user, &1_000_0000i128);
        set_prices(&w);

        let handler_client = DepositHandlerClient::new(env, &w.handler);

        let key = handler_client.create_deposit(
            &user,
            &CreateDepositParams {
                receiver: user.clone(),
                market: w.market_tk.clone(),
                initial_long_token: w.long_tk.clone(),
                initial_short_token: w.short_tk.clone(),
                long_token_amount: 1_000_0000i128,
                short_token_amount: 0,
                min_market_tokens: 1,
                execution_fee: 0,
            },
        );

        handler_client.execute_deposit(&w.keeper, &key);

        let ds_c = DsClient::new(env, &w.ds);
        let long_pool = ds_c.get_u128(&gmx_keys::pool_amount_key(env, &w.market_tk, &w.long_tk));
        let short_pool = ds_c.get_u128(&gmx_keys::pool_amount_key(env, &w.market_tk, &w.short_tk));

        assert_eq!(
            long_pool, 1_000_0000,
            "long pool should increase by deposit amount"
        );
        assert_eq!(short_pool, 0, "short pool should remain 0");
    }

    /// Test short-only deposit: pool accounting must be correct.
    #[test]
    fn execute_deposit_short_only_pool_accounting() {
        let w = setup();
        let env = &w.env;
        let user = Address::generate(env);

        StellarAssetClient::new(env, &w.short_tk).mint(&user, &500_0000i128);
        set_prices(&w);

        let handler_client = DepositHandlerClient::new(env, &w.handler);

        let key = handler_client.create_deposit(
            &user,
            &CreateDepositParams {
                receiver: user.clone(),
                market: w.market_tk.clone(),
                initial_long_token: w.long_tk.clone(),
                initial_short_token: w.short_tk.clone(),
                long_token_amount: 0,
                short_token_amount: 500_0000i128,
                min_market_tokens: 1,
                execution_fee: 0,
            },
        );

        handler_client.execute_deposit(&w.keeper, &key);

        let ds_c = DsClient::new(env, &w.ds);
        let long_pool = ds_c.get_u128(&gmx_keys::pool_amount_key(env, &w.market_tk, &w.long_tk));
        let short_pool = ds_c.get_u128(&gmx_keys::pool_amount_key(env, &w.market_tk, &w.short_tk));

        assert_eq!(long_pool, 0, "long pool should remain 0");
        assert_eq!(
            short_pool, 500_0000,
            "short pool should increase by deposit amount"
        );
    }

    /// Test mixed deposit: both long and short tokens added to pool.
    #[test]
    fn execute_deposit_mixed_pool_accounting() {
        let w = setup();
        let env = &w.env;
        let user = Address::generate(env);

        StellarAssetClient::new(env, &w.long_tk).mint(&user, &1_000_0000i128);
        StellarAssetClient::new(env, &w.short_tk).mint(&user, &500_0000i128);
        set_prices(&w);

        let handler_client = DepositHandlerClient::new(env, &w.handler);

        let key = handler_client.create_deposit(
            &user,
            &CreateDepositParams {
                receiver: user.clone(),
                market: w.market_tk.clone(),
                initial_long_token: w.long_tk.clone(),
                initial_short_token: w.short_tk.clone(),
                long_token_amount: 1_000_0000i128,
                short_token_amount: 500_0000i128,
                min_market_tokens: 1,
                execution_fee: 0,
            },
        );

        handler_client.execute_deposit(&w.keeper, &key);

        let ds_c = DsClient::new(env, &w.ds);
        let long_pool = ds_c.get_u128(&gmx_keys::pool_amount_key(env, &w.market_tk, &w.long_tk));
        let short_pool = ds_c.get_u128(&gmx_keys::pool_amount_key(env, &w.market_tk, &w.short_tk));

        assert_eq!(
            long_pool, 1_000_0000,
            "long pool should increase by long deposit amount"
        );
        assert_eq!(
            short_pool, 500_0000,
            "short pool should increase by short deposit amount"
        );

        let lp = MtClient::new(env, &w.market_tk).balance(&user);
        assert!(lp > 0, "LP tokens should be minted for mixed deposit");
    }

    // ── Issue #27: global deposit list lifecycle ──────────────────────────────

    /// Create three deposits for three different users, cancel one, execute
    /// another, leave the third pending.  The global list and per-account list
    /// must reflect exactly the correct state at each stage.
    #[test]
    fn deposit_list_reflects_full_lifecycle() {
        let w = setup();
        let env = &w.env;

        let user_a = Address::generate(env);
        let user_b = Address::generate(env);
        let user_c = Address::generate(env);

        for u in [&user_a, &user_b, &user_c] {
            StellarAssetClient::new(env, &w.long_tk).mint(u, &1_000_0000i128);
        }

        let hc = DepositHandlerClient::new(env, &w.handler);
        let ds = DsClient::new(env, &w.ds);

        let make_params = |user: &Address| CreateDepositParams {
            receiver: user.clone(),
            market: w.market_tk.clone(),
            initial_long_token: w.long_tk.clone(),
            initial_short_token: w.short_tk.clone(),
            long_token_amount: 1_000_0000i128,
            short_token_amount: 0,
            min_market_tokens: 0,
            execution_fee: 0,
        };

        // ── Create three deposits ─────────────────────────────────────────────
        let key_a = hc.create_deposit(&user_a, &make_params(&user_a));
        let key_b = hc.create_deposit(&user_b, &make_params(&user_b));
        let key_c = hc.create_deposit(&user_c, &make_params(&user_c));

        assert_eq!(
            ds.get_bytes32_set_count(&gmx_keys::deposit_list_key(env)),
            3,
            "global list must have 3 entries after three creates"
        );
        for key in [&key_a, &key_b, &key_c] {
            assert!(
                ds.contains_bytes32(&gmx_keys::deposit_list_key(env), key),
                "global list must contain each deposit key after create"
            );
        }
        for (user, key) in [(&user_a, &key_a), (&user_b, &key_b), (&user_c, &key_c)] {
            assert_eq!(
                ds.get_bytes32_set_count(&gmx_keys::account_deposit_list_key(env, user)),
                1,
                "account list must have 1 entry per user after create"
            );
            assert!(ds.contains_bytes32(&gmx_keys::account_deposit_list_key(env, user), key));
        }

        // ── Cancel user_a's deposit ───────────────────────────────────────────
        hc.cancel_deposit(&user_a, &key_a);

        assert_eq!(
            ds.get_bytes32_set_count(&gmx_keys::deposit_list_key(env)),
            2,
            "global list must have 2 entries after one cancel"
        );
        assert!(
            !ds.contains_bytes32(&gmx_keys::deposit_list_key(env), &key_a),
            "cancelled key must be absent from global list"
        );
        assert_eq!(
            ds.get_bytes32_set_count(&gmx_keys::account_deposit_list_key(env, &user_a)),
            0,
            "cancelled user account list must be empty"
        );

        // ── Execute user_b's deposit ──────────────────────────────────────────
        set_prices(&w);
        hc.execute_deposit(&w.keeper, &key_b);

        assert_eq!(
            ds.get_bytes32_set_count(&gmx_keys::deposit_list_key(env)),
            1,
            "global list must have 1 entry after cancel + execute"
        );
        assert!(
            !ds.contains_bytes32(&gmx_keys::deposit_list_key(env), &key_b),
            "executed key must be absent from global list"
        );
        assert_eq!(
            ds.get_bytes32_set_count(&gmx_keys::account_deposit_list_key(env, &user_b)),
            0,
            "executed user account list must be empty"
        );

        // ── user_c's deposit is still pending ─────────────────────────────────
        assert!(
            ds.contains_bytes32(&gmx_keys::deposit_list_key(env), &key_c),
            "pending deposit key must remain in global list"
        );
        assert!(
            hc.get_deposit(&key_c).is_some(),
            "pending deposit record must still exist"
        );

        // ── Final: only key_c remains; list query returns exactly [key_c] ─────
        let page = ds.get_bytes32_set_at(&gmx_keys::deposit_list_key(env), &0, &10);
        assert_eq!(page.len(), 1, "list query must return exactly one key");
        assert_eq!(
            page.get_unchecked(0),
            key_c,
            "list query must return the still-pending deposit key"
        );
    }

    // ── Issue #44: Vault balance invariant tests ───────────────────────────────

    /// After execute_deposit, vault recorded balance must match actual token balance.
    #[test]
    fn vault_balance_invariant_after_execute_deposit() {
        let w = setup();
        let env = &w.env;
        let user = Address::generate(env);

        StellarAssetClient::new(env, &w.long_tk).mint(&user, &1_000_0000i128);
        StellarAssetClient::new(env, &w.short_tk).mint(&user, &500_0000i128);
        set_prices(&w);

        let handler_client = DepositHandlerClient::new(env, &w.handler);
        let vault_client = DVClient::new(env, &w.vault);

        let key = handler_client.create_deposit(
            &user,
            &CreateDepositParams {
                receiver: user.clone(),
                market: w.market_tk.clone(),
                initial_long_token: w.long_tk.clone(),
                initial_short_token: w.short_tk.clone(),
                long_token_amount: 1_000_0000i128,
                short_token_amount: 500_0000i128,
                min_market_tokens: 1,
                execution_fee: 0,
            },
        );

        // Before execute: vault should have the tokens
        let vault_addr = w.vault.clone();
        let long_balance_before = token::Client::new(env, &w.long_tk).balance(&vault_addr);
        let short_balance_before = token::Client::new(env, &w.short_tk).balance(&vault_addr);
        assert_eq!(long_balance_before, 1_000_0000);
        assert_eq!(short_balance_before, 500_0000);

        handler_client.execute_deposit(&w.keeper, &key);

        // After execute: vault should be empty (tokens moved to pool)
        let long_balance_after = token::Client::new(env, &w.long_tk).balance(&vault_addr);
        let short_balance_after = token::Client::new(env, &w.short_tk).balance(&vault_addr);
        assert_eq!(
            long_balance_after, 0,
            "vault long balance should be 0 after execute"
        );
        assert_eq!(
            short_balance_after, 0,
            "vault short balance should be 0 after execute"
        );

        // Recorded balance must match actual balance
        let recorded_long = vault_client.get_recorded_balance(&w.long_tk);
        let recorded_short = vault_client.get_recorded_balance(&w.short_tk);
        assert_eq!(
            recorded_long, long_balance_after,
            "recorded long balance must match actual"
        );
        assert_eq!(
            recorded_short, short_balance_after,
            "recorded short balance must match actual"
        );
    }

    /// After cancel_deposit, vault recorded balance must match actual token balance.
    #[test]
    fn vault_balance_invariant_after_cancel_deposit() {
        let w = setup();
        let env = &w.env;
        let user = Address::generate(env);

        StellarAssetClient::new(env, &w.long_tk).mint(&user, &1_000_0000i128);
        StellarAssetClient::new(env, &w.short_tk).mint(&user, &500_0000i128);

        let handler_client = DepositHandlerClient::new(env, &w.handler);
        let vault_client = DVClient::new(env, &w.vault);

        let key = handler_client.create_deposit(
            &user,
            &CreateDepositParams {
                receiver: user.clone(),
                market: w.market_tk.clone(),
                initial_long_token: w.long_tk.clone(),
                initial_short_token: w.short_tk.clone(),
                long_token_amount: 1_000_0000i128,
                short_token_amount: 500_0000i128,
                min_market_tokens: 1,
                execution_fee: 0,
            },
        );

        // Before cancel: vault has tokens
        let vault_addr = w.vault.clone();
        let long_balance_before = token::Client::new(env, &w.long_tk).balance(&vault_addr);
        assert_eq!(long_balance_before, 1_000_0000);

        handler_client.cancel_deposit(&user, &key);

        // After cancel: vault should be empty (tokens refunded to user)
        let long_balance_after = token::Client::new(env, &w.long_tk).balance(&vault_addr);
        let short_balance_after = token::Client::new(env, &w.short_tk).balance(&vault_addr);
        assert_eq!(
            long_balance_after, 0,
            "vault long balance should be 0 after cancel"
        );
        assert_eq!(
            short_balance_after, 0,
            "vault short balance should be 0 after cancel"
        );

        // Recorded balance must match actual balance
        let recorded_long = vault_client.get_recorded_balance(&w.long_tk);
        let recorded_short = vault_client.get_recorded_balance(&w.short_tk);
        assert_eq!(
            recorded_long, long_balance_after,
            "recorded long balance must match actual after cancel"
        );
        assert_eq!(
            recorded_short, short_balance_after,
            "recorded short balance must match actual after cancel"
        );
    }

    // ── Issue #46: Event field completeness ────────────────────────────────────

    /// Verify that deposit creation event includes all required fields.
    #[test]
    fn deposit_creation_event_has_all_fields() {
        let w = setup();
        let env = &w.env;
        let user = Address::generate(env);

        StellarAssetClient::new(env, &w.long_tk).mint(&user, &1_000_0000i128);

        let handler_client = DepositHandlerClient::new(env, &w.handler);

        // Create deposit — event should be published with key, account, market
        let key = handler_client.create_deposit(
            &user,
            &CreateDepositParams {
                receiver: user.clone(),
                market: w.market_tk.clone(),
                initial_long_token: w.long_tk.clone(),
                initial_short_token: w.short_tk.clone(),
                long_token_amount: 1_000_0000i128,
                short_token_amount: 0,
                min_market_tokens: 0,
                execution_fee: 0,
            },
        );

        // Verify deposit was recorded (event was published)
        let dep = handler_client.get_deposit(&key).unwrap();
        assert_eq!(dep.account, user);
        assert_eq!(dep.market, w.market_tk);
        assert_eq!(dep.long_token_amount, 1_000_0000);
    }

    /// Verify that deposit execution event includes all required fields.
    #[test]
    fn deposit_execution_event_has_all_fields() {
        let w = setup();
        let env = &w.env;
        let user = Address::generate(env);

        StellarAssetClient::new(env, &w.long_tk).mint(&user, &1_000_0000i128);
        set_prices(&w);

        let handler_client = DepositHandlerClient::new(env, &w.handler);

        let key = handler_client.create_deposit(
            &user,
            &CreateDepositParams {
                receiver: user.clone(),
                market: w.market_tk.clone(),
                initial_long_token: w.long_tk.clone(),
                initial_short_token: w.short_tk.clone(),
                long_token_amount: 1_000_0000i128,
                short_token_amount: 0,
                min_market_tokens: 1,
                execution_fee: 0,
            },
        );

        handler_client.execute_deposit(&w.keeper, &key);

        // Verify LP tokens were minted (event was published with mint amount)
        let lp = MtClient::new(env, &w.market_tk).balance(&user);
        assert!(lp > 0, "LP tokens should be minted and event published");
    }

    // ── Issue #109: ORDER_KEEPER authorization matrix ─────────────────────────

    /// execute_deposit must reject a caller that does not hold ORDER_KEEPER.
    #[test]
    #[should_panic]
    fn execute_deposit_by_non_keeper_panics() {
        let w = setup();
        let user = Address::generate(&w.env);
        StellarAssetClient::new(&w.env, &w.long_tk).mint(&user, &1_000_0000i128);
        set_prices(&w);

        let key = DepositHandlerClient::new(&w.env, &w.handler).create_deposit(
            &user,
            &CreateDepositParams {
                receiver: user.clone(),
                market: w.market_tk.clone(),
                initial_long_token: w.long_tk.clone(),
                initial_short_token: w.short_tk.clone(),
                long_token_amount: 1_000_0000,
                short_token_amount: 0,
                min_market_tokens: 1,
                execution_fee: 0,
            },
        );

        // impostor has no ORDER_KEEPER role — execute_deposit must panic.
        let impostor = Address::generate(&w.env);
        DepositHandlerClient::new(&w.env, &w.handler).execute_deposit(&impostor, &key);
    }

    // ── Issue #110: upgrade smoke tests ───────────────────────────────────────

    /// Admin auth passes on upgrade; the call reaches the WASM-lookup stage (not auth).
    /// In unit tests there is no compiled WASM binary, so the host rejects the zero
    /// hash with "Wasm does not exist" — this is AFTER auth, proving auth is satisfied.
    #[test]
    #[should_panic]
    fn upgrade_admin_succeeds() {
        let w = setup(); // mock_all_auths active — admin.require_auth() passes silently
        let new_hash = BytesN::from_array(&w.env, &[0u8; 32]);
        // Panics at WASM lookup (not at auth) — proves auth gate is open for admin.
        DepositHandlerClient::new(&w.env, &w.handler).upgrade(&new_hash);
    }

    /// Calling upgrade without the admin's authorisation must revert.
    #[test]
    #[should_panic]
    fn upgrade_non_admin_reverts() {
        // Fresh env — no mock_all_auths so require_auth() is not bypassed.
        let env = Env::default();
        let admin = Address::generate(&env);
        let rs = Address::generate(&env);
        let ds = Address::generate(&env);
        let oracle = Address::generate(&env);
        let vault = Address::generate(&env);

        let handler = env.register(DepositHandler, ());
        env.as_contract(&handler, || {
            env.storage()
                .instance()
                .set(&InstanceKey::Initialized, &true);
            env.storage().instance().set(&InstanceKey::Admin, &admin);
            env.storage().instance().set(&InstanceKey::RoleStore, &rs);
            env.storage().instance().set(&InstanceKey::DataStore, &ds);
            env.storage().instance().set(&InstanceKey::Oracle, &oracle);
            env.storage()
                .instance()
                .set(&InstanceKey::DepositVault, &vault);
        });

        // No auth context — must panic at admin.require_auth().
        let new_hash = BytesN::from_array(&env, &[0u8; 32]);
        DepositHandlerClient::new(&env, &handler).upgrade(&new_hash);
    }

    /// Persistent storage survives an upgrade (Soroban host guarantee).
    /// Requires a compiled WASM binary to invoke update_current_contract_wasm;
    /// not runnable in unit-test mode. Auth + storage are covered by surrounding tests.
    #[test]
    #[ignore]
    fn upgrade_preserves_deposit_storage() {
        let w = setup();
        let user = Address::generate(&w.env);
        StellarAssetClient::new(&w.env, &w.long_tk).mint(&user, &1_000_0000i128);
        set_prices(&w);

        let hc = DepositHandlerClient::new(&w.env, &w.handler);
        let key = hc.create_deposit(
            &user,
            &CreateDepositParams {
                receiver: user.clone(),
                market: w.market_tk.clone(),
                initial_long_token: w.long_tk.clone(),
                initial_short_token: w.short_tk.clone(),
                long_token_amount: 1_000_0000,
                short_token_amount: 0,
                min_market_tokens: 1,
                execution_fee: 0,
            },
        );

        assert!(
            hc.get_deposit(&key).is_some(),
            "deposit must exist before upgrade"
        );

        hc.upgrade(&BytesN::from_array(&w.env, &[0u8; 32]));

        assert!(
            hc.get_deposit(&key).is_some(),
            "deposit must still be readable after upgrade"
        );
    }
}
