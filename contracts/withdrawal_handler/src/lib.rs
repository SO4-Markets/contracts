//! Withdrawal Handler — create, execute, and cancel LP token withdrawals.
//!
//! Mirrors GMX's WithdrawalHandler.sol + ExecuteWithdrawalUtils.sol:
//!
//! Flow:
//!   1. User approves LP tokens to withdrawal_handler.
//!   2. User calls `create_withdrawal` → LP tokens pulled to withdrawal_vault.
//!   3. Keeper sets oracle prices, then calls `execute_withdrawal`:
//!      - Computes pro-rata long/short amounts from pool.
//!      - Burns LP tokens from vault.
//!      - Transfers pool tokens from market_token contract → receiver.
//!      - Updates pool amounts.
//!   4. On cancel: LP tokens refunded from vault.
#![no_std]
#![allow(dependency_on_unit_never_type_fallback)]

use soroban_sdk::{
    contract, contractimpl, contracttype, contracterror, panic_with_error,
    symbol_short, token, Address, BytesN, Env,
};
use gmx_types::{WithdrawalProps, MarketProps};
use gmx_math::{mul_div_wide, TOKEN_PRECISION};
use gmx_keys::{
    roles,
    withdrawal_key, withdrawal_list_key, account_withdrawal_list_key,
    market_index_token_key, market_long_token_key, market_short_token_key,
    pool_amount_key,
};
use gmx_market_utils::{
    get_pool_amount, apply_delta_to_pool_amount,
    update_funding_state, update_cumulative_borrowing_factor,
};

// ─── Errors ───────────────────────────────────────────────────────────────────

#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
#[repr(u32)]
pub enum Error {
    AlreadyInitialized   = 1,
    Unauthorized         = 2,
    WithdrawalNotFound   = 3,
    InsufficientLongOut  = 4,
    InsufficientShortOut = 5,
    ZeroWithdrawal       = 6,
}

// ─── Storage ──────────────────────────────────────────────────────────────────

#[contracttype]
enum InstanceKey {
    Initialized,
    Admin,
    RoleStore,
    DataStore,
    Oracle,
    WithdrawalVault,
}

#[contracttype]
enum LocalKey {
    Withdrawal(BytesN<32>),
}

// ─── Cross-contract clients ───────────────────────────────────────────────────

#[soroban_sdk::contractclient(name = "RoleStoreClient")]
trait IRoleStore {
    fn has_role(env: Env, account: Address, role: BytesN<32>) -> bool;
}

#[soroban_sdk::contractclient(name = "DataStoreClient")]
trait IDataStore {
    fn get_u128(env: Env, key: BytesN<32>) -> u128;
    fn set_u128(env: Env, caller: Address, key: BytesN<32>, value: u128) -> u128;
    fn get_i128(env: Env, key: BytesN<32>) -> i128;
    fn set_i128(env: Env, caller: Address, key: BytesN<32>, value: i128) -> i128;
    fn apply_delta_to_u128(env: Env, caller: Address, key: BytesN<32>, delta: i128) -> u128;
    fn apply_delta_to_i128(env: Env, caller: Address, key: BytesN<32>, delta: i128) -> i128;
    fn get_address(env: Env, key: BytesN<32>) -> Option<Address>;
    fn add_bytes32_to_set(env: Env, caller: Address, set_key: BytesN<32>, value: BytesN<32>);
    fn remove_bytes32_from_set(env: Env, caller: Address, set_key: BytesN<32>, value: BytesN<32>);
    fn increment_nonce(env: Env, caller: Address) -> u64;
}

#[soroban_sdk::contractclient(name = "OracleClient")]
trait IOracle {
    fn get_primary_price(env: Env, token: Address) -> gmx_types::PriceProps;
}

#[soroban_sdk::contractclient(name = "WithdrawalVaultClient")]
trait IWithdrawalVault {
    fn transfer_out(env: Env, caller: Address, token: Address, receiver: Address, amount: i128);
}

#[soroban_sdk::contractclient(name = "MarketTokenClient")]
trait IMarketToken {
    fn burn(env: Env, from: Address, amount: i128);
    fn total_supply(env: Env) -> i128;
    fn withdraw_from_pool(env: Env, caller: Address, pool_token: Address, receiver: Address, amount: i128);
}

// ─── Params ───────────────────────────────────────────────────────────────────

#[contracttype]
pub struct CreateWithdrawalParams {
    pub receiver:              Address,
    pub market:                Address,
    pub market_token_amount:   i128,
    pub min_long_token_amount: i128,
    pub min_short_token_amount: i128,
    pub execution_fee:         i128,
}

// ─── Contract ─────────────────────────────────────────────────────────────────

#[contract]
pub struct WithdrawalHandler;

#[contractimpl]
impl WithdrawalHandler {
    // ── Init ─────────────────────────────────────────────────────────────────

    pub fn initialize(
        env: Env,
        admin: Address,
        role_store: Address,
        data_store: Address,
        oracle: Address,
        withdrawal_vault: Address,
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
        env.storage().instance().set(&InstanceKey::WithdrawalVault, &withdrawal_vault);
    }

    // ── Create withdrawal ─────────────────────────────────────────────────────

    /// Pull LP tokens from caller into the withdrawal_vault and record the withdrawal.
    pub fn create_withdrawal(env: Env, caller: Address, params: CreateWithdrawalParams) -> BytesN<32> {
        caller.require_auth();

        if params.market_token_amount == 0 {
            panic_with_error!(&env, Error::ZeroWithdrawal);
        }

        let data_store: Address = env.storage().instance().get(&InstanceKey::DataStore).unwrap();
        let withdrawal_vault: Address = env.storage().instance().get(&InstanceKey::WithdrawalVault).unwrap();
        let handler = env.current_contract_address();
        let ds = DataStoreClient::new(&env, &data_store);

        // Pull LP tokens from caller → withdrawal_vault
        let market_addr = params.market.clone();
        token::Client::new(&env, &params.market)
            .transfer(&caller, &withdrawal_vault, &params.market_token_amount);

        // Allocate withdrawal key from nonce
        let nonce = ds.increment_nonce(&handler);
        let key = withdrawal_key(&env, nonce);

        let withdrawal = WithdrawalProps {
            account:               caller.clone(),
            receiver:              params.receiver,
            market:                params.market,
            market_token_amount:   params.market_token_amount,
            min_long_token_amount: params.min_long_token_amount,
            min_short_token_amount: params.min_short_token_amount,
            execution_fee:         params.execution_fee,
            updated_at_time:       env.ledger().timestamp(),
        };
        env.storage().persistent().set(&LocalKey::Withdrawal(key.clone()), &withdrawal);

        ds.add_bytes32_to_set(&handler, &withdrawal_list_key(&env), &key);
        ds.add_bytes32_to_set(&handler, &account_withdrawal_list_key(&env, &caller), &key);

        env.events().publish((symbol_short!("wth_crt"),), (key.clone(), caller, market_addr));
        key
    }

    // ── Execute withdrawal ────────────────────────────────────────────────────

    /// Keeper executes a pending withdrawal: burns LP tokens, returns pool tokens.
    pub fn execute_withdrawal(env: Env, keeper: Address, key: BytesN<32>) {
        keeper.require_auth();
        require_order_keeper(&env, &keeper);

        let data_store: Address = env.storage().instance().get(&InstanceKey::DataStore).unwrap();
        let withdrawal_vault: Address = env.storage().instance().get(&InstanceKey::WithdrawalVault).unwrap();
        let oracle: Address = env.storage().instance().get(&InstanceKey::Oracle).unwrap();
        let handler = env.current_contract_address();

        let withdrawal: WithdrawalProps = env.storage().persistent()
            .get(&LocalKey::Withdrawal(key.clone()))
            .unwrap_or_else(|| panic_with_error!(&env, Error::WithdrawalNotFound));

        let market = load_market_props(&env, &data_store, &withdrawal.market);

        // Read oracle prices
        let oracle_client = OracleClient::new(&env, &oracle);
        let long_price  = oracle_client.get_primary_price(&market.long_token).mid_price();
        let short_price = oracle_client.get_primary_price(&market.short_token).mid_price();
        let index_price = oracle_client.get_primary_price(&market.index_token).mid_price();

        let mt_client = MarketTokenClient::new(&env, &market.market_token);
        let total_supply = mt_client.total_supply();

        // Pro-rata pool amounts:  out = pool_amount × lp_amount / total_supply
        let long_pool  = get_pool_amount(&env, &data_store, &market, &market.long_token) as i128;
        let short_pool = get_pool_amount(&env, &data_store, &market, &market.short_token) as i128;
        let lp_amount  = withdrawal.market_token_amount;

        let long_out  = mul_div_wide(&env, long_pool,  lp_amount, total_supply);
        let short_out = mul_div_wide(&env, short_pool, lp_amount, total_supply);

        if long_out < withdrawal.min_long_token_amount {
            panic_with_error!(&env, Error::InsufficientLongOut);
        }
        if short_out < withdrawal.min_short_token_amount {
            panic_with_error!(&env, Error::InsufficientShortOut);
        }

        // Burn LP tokens from vault
        WithdrawalVaultClient::new(&env, &withdrawal_vault)
            .transfer_out(&handler, &market.market_token, &handler, &lp_amount);
        mt_client.burn(&handler, &lp_amount);

        // Transfer pool tokens from market_token contract → receiver
        if long_out > 0 {
            mt_client.withdraw_from_pool(&handler, &market.long_token, &withdrawal.receiver, &long_out);
            apply_delta_to_pool_amount(&env, &data_store, &handler, &market, &market.long_token, -long_out);
        }
        if short_out > 0 {
            mt_client.withdraw_from_pool(&handler, &market.short_token, &withdrawal.receiver, &short_out);
            apply_delta_to_pool_amount(&env, &data_store, &handler, &market, &market.short_token, -short_out);
        }

        // Update market state
        let now = env.ledger().timestamp();
        update_funding_state(&env, &data_store, &handler, &market, long_price, short_price, now);
        update_cumulative_borrowing_factor(&env, &data_store, &handler, &market, true, now);
        update_cumulative_borrowing_factor(&env, &data_store, &handler, &market, false, now);

        remove_withdrawal(&env, &data_store, &handler, &key, &withdrawal.account);

        env.events().publish(
            (symbol_short!("wth_exe"),),
            (key, withdrawal.receiver, long_out, short_out),
        );
    }

    // ── Cancel withdrawal ─────────────────────────────────────────────────────

    pub fn cancel_withdrawal(env: Env, caller: Address, key: BytesN<32>) {
        caller.require_auth();

        let data_store: Address = env.storage().instance().get(&InstanceKey::DataStore).unwrap();
        let withdrawal_vault: Address = env.storage().instance().get(&InstanceKey::WithdrawalVault).unwrap();
        let role_store: Address = env.storage().instance().get(&InstanceKey::RoleStore).unwrap();
        let handler = env.current_contract_address();

        let withdrawal: WithdrawalProps = env.storage().persistent()
            .get(&LocalKey::Withdrawal(key.clone()))
            .unwrap_or_else(|| panic_with_error!(&env, Error::WithdrawalNotFound));

        let is_keeper = RoleStoreClient::new(&env, &role_store)
            .has_role(&caller, &roles::order_keeper(&env));
        if caller != withdrawal.account && !is_keeper {
            panic_with_error!(&env, Error::Unauthorized);
        }

        // Refund LP tokens
        WithdrawalVaultClient::new(&env, &withdrawal_vault)
            .transfer_out(&handler, &withdrawal.market, &withdrawal.account, &withdrawal.market_token_amount);

        remove_withdrawal(&env, &data_store, &handler, &key, &withdrawal.account);

        env.events().publish((symbol_short!("wth_can"),), (key, withdrawal.account));
    }

    // ── Views ─────────────────────────────────────────────────────────────────

    pub fn get_withdrawal(env: Env, key: BytesN<32>) -> Option<WithdrawalProps> {
        env.storage().persistent().get(&LocalKey::Withdrawal(key))
    }
}

// ─── Helpers ──────────────────────────────────────────────────────────────────

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

fn remove_withdrawal(
    env: &Env,
    data_store: &Address,
    handler: &Address,
    key: &BytesN<32>,
    account: &Address,
) {
    env.storage().persistent().remove(&LocalKey::Withdrawal(key.clone()));
    let ds = DataStoreClient::new(env, data_store);
    ds.remove_bytes32_from_set(handler, &withdrawal_list_key(env), key);
    ds.remove_bytes32_from_set(handler, &account_withdrawal_list_key(env, account), key);
}

// ─── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use soroban_sdk::{testutils::Address as _, token::StellarAssetClient, Env, Vec};
    use role_store::{RoleStore, RoleStoreClient as RsClient};
    use data_store::{DataStore, DataStoreClient as DsClient};
    use oracle::{Oracle, OracleClient as OClient};
    use withdrawal_vault::{WithdrawalVault, WithdrawalVaultClient as WVClient};
    use deposit_vault::{DepositVault, DepositVaultClient as DVClient};
    use market_token::{MarketToken, MarketTokenClient as MtClient};
    use deposit_handler::{DepositHandler, DepositHandlerClient, CreateDepositParams};
    use gmx_keys::roles;
    use gmx_types::TokenPrice;

    struct World {
        env:       Env,
        admin:     Address,
        keeper:    Address,
        rs:        Address,
        ds:        Address,
        oracle:    Address,
        dep_vault: Address,
        wth_vault: Address,
        dep_handler: Address,
        wth_handler: Address,
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

        let rs = env.register(RoleStore, ());
        RsClient::new(&env, &rs).initialize(&admin);
        let rs_c = RsClient::new(&env, &rs);
        rs_c.grant_role(&admin, &admin,  &roles::controller(&env));
        rs_c.grant_role(&admin, &keeper, &roles::order_keeper(&env));

        let ds = env.register(DataStore, ());
        DsClient::new(&env, &ds).initialize(&admin, &rs);

        let oracle_addr = env.register(Oracle, ());
        let passphrase = soroban_sdk::Bytes::from_slice(&env, b"Test SDF Network ; September 2015");
        OClient::new(&env, &oracle_addr).initialize(&admin, &rs, &ds, &passphrase);

        let dep_vault = env.register(DepositVault, ());
        DVClient::new(&env, &dep_vault).initialize(&admin, &rs);

        let wth_vault = env.register(WithdrawalVault, ());
        WVClient::new(&env, &wth_vault).initialize(&admin, &rs);

        let market_tk = env.register(MarketToken, ());
        MtClient::new(&env, &market_tk).initialize(
            &admin, &rs, &7u32,
            &soroban_sdk::String::from_str(&env, "GMX Market Token"),
            &soroban_sdk::String::from_str(&env, "GM"),
        );

        let dep_handler = env.register(DepositHandler, ());
        DepositHandlerClient::new(&env, &dep_handler)
            .initialize(&admin, &rs, &ds, &oracle_addr, &dep_vault);

        let wth_handler = env.register(WithdrawalHandler, ());
        WithdrawalHandlerClient::new(&env, &wth_handler)
            .initialize(&admin, &rs, &ds, &oracle_addr, &wth_vault);

        // Grant both handlers CONTROLLER
        rs_c.grant_role(&admin, &dep_handler, &roles::controller(&env));
        rs_c.grant_role(&admin, &wth_handler, &roles::controller(&env));

        let long_tk  = env.register_stellar_asset_contract_v2(admin.clone()).address();
        let short_tk = env.register_stellar_asset_contract_v2(admin.clone()).address();
        let index_tk = Address::generate(&env);

        // Register market props in data_store
        let ds_c = DsClient::new(&env, &ds);
        ds_c.set_address(&dep_handler, &gmx_keys::market_index_token_key(&env, &market_tk), &index_tk);
        ds_c.set_address(&dep_handler, &gmx_keys::market_long_token_key(&env, &market_tk), &long_tk);
        ds_c.set_address(&dep_handler, &gmx_keys::market_short_token_key(&env, &market_tk), &short_tk);

        World { env, admin, keeper, rs, ds, oracle: oracle_addr, dep_vault, wth_vault, dep_handler, wth_handler, market_tk, long_tk, short_tk, index_tk }
    }

    fn set_prices(w: &World) {
        let fp = gmx_math::FLOAT_PRECISION;
        OClient::new(&w.env, &w.oracle).set_prices_simple(&w.keeper, &Vec::from_array(&w.env, [
            TokenPrice { token: w.long_tk.clone(),  min: 2000 * fp, max: 2000 * fp },
            TokenPrice { token: w.short_tk.clone(), min: fp,        max: fp },
            TokenPrice { token: w.index_tk.clone(), min: 2000 * fp, max: 2000 * fp },
        ]));
    }

    #[test]
    fn full_deposit_then_withdrawal() {
        let w = setup();
        let env = &w.env;
        let user = Address::generate(env);

        // Give user pool tokens
        StellarAssetClient::new(env, &w.long_tk).mint(&user,  &1_000_0000i128);
        StellarAssetClient::new(env, &w.short_tk).mint(&user, &500_0000i128);

        set_prices(&w);

        // Deposit
        let dep_key = DepositHandlerClient::new(env, &w.dep_handler).create_deposit(&user, &CreateDepositParams {
            receiver:            user.clone(),
            market:              w.market_tk.clone(),
            initial_long_token:  w.long_tk.clone(),
            initial_short_token: w.short_tk.clone(),
            long_token_amount:   1_000_0000i128,
            short_token_amount:  500_0000i128,
            min_market_tokens:   1,
            execution_fee:       0,
        });
        DepositHandlerClient::new(env, &w.dep_handler).execute_deposit(&w.keeper, &dep_key);

        let lp_balance = MtClient::new(env, &w.market_tk).balance(&user);
        assert!(lp_balance > 0);

        set_prices(&w); // re-set prices for withdrawal

        // Withdraw half
        let half_lp = lp_balance / 2;
        let wth_key = WithdrawalHandlerClient::new(env, &w.wth_handler).create_withdrawal(&user, &CreateWithdrawalParams {
            receiver:              user.clone(),
            market:                w.market_tk.clone(),
            market_token_amount:   half_lp,
            min_long_token_amount:  0,
            min_short_token_amount: 0,
            execution_fee:         0,
        });
        WithdrawalHandlerClient::new(env, &w.wth_handler).execute_withdrawal(&w.keeper, &wth_key);

        // User should have received ~half of pool back
        let long_back  = StellarAssetClient::new(env, &w.long_tk).balance(&user);
        let short_back = StellarAssetClient::new(env, &w.short_tk).balance(&user);
        assert!(long_back > 0,  "should receive long tokens back");
        assert!(short_back > 0, "should receive short tokens back");
        // LP balance should be halved
        let lp_after = MtClient::new(env, &w.market_tk).balance(&user);
        assert_eq!(lp_after, lp_balance - half_lp);
    }

    #[test]
    fn cancel_withdrawal_refunds_lp() {
        let w = setup();
        let env = &w.env;
        let user = Address::generate(env);

        StellarAssetClient::new(env, &w.long_tk).mint(&user, &1_000_0000i128);
        set_prices(&w);

        // Deposit first to get LP tokens
        let dep_key = DepositHandlerClient::new(env, &w.dep_handler).create_deposit(&user, &CreateDepositParams {
            receiver:            user.clone(),
            market:              w.market_tk.clone(),
            initial_long_token:  w.long_tk.clone(),
            initial_short_token: w.short_tk.clone(),
            long_token_amount:   1_000_0000i128,
            short_token_amount:  0,
            min_market_tokens:   1,
            execution_fee:       0,
        });
        DepositHandlerClient::new(env, &w.dep_handler).execute_deposit(&w.keeper, &dep_key);

        let lp_balance = MtClient::new(env, &w.market_tk).balance(&user);

        // Create and cancel withdrawal
        let wth_key = WithdrawalHandlerClient::new(env, &w.wth_handler).create_withdrawal(&user, &CreateWithdrawalParams {
            receiver:              user.clone(),
            market:                w.market_tk.clone(),
            market_token_amount:   lp_balance,
            min_long_token_amount:  0,
            min_short_token_amount: 0,
            execution_fee:         0,
        });

        // LP should be in vault now
        assert_eq!(MtClient::new(env, &w.market_tk).balance(&user), 0);

        WithdrawalHandlerClient::new(env, &w.wth_handler).cancel_withdrawal(&user, &wth_key);

        // LP should be returned
        assert_eq!(MtClient::new(env, &w.market_tk).balance(&user), lp_balance);
        assert!(WithdrawalHandlerClient::new(env, &w.wth_handler).get_withdrawal(&wth_key).is_none());
    }
}
