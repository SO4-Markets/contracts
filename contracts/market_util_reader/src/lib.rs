//! Market utilisation reader — view-only OI-to-pool-depth ratio.
#![no_std]

use gmx_keys::{
    market_index_token_key, market_long_token_key, market_short_token_key, max_open_interest_key,
};
use gmx_market_utils::{get_open_interest_for_side, get_pool_value};
use gmx_types::{MarketProps, PriceProps};
use soroban_sdk::{contract, contractimpl, contracttype, Address, BytesN, Env};

const BPS_DIVISOR: u128 = 10_000;

#[allow(dead_code)]
#[soroban_sdk::contractclient(name = "DataStoreClient")]
trait IDataStore {
    fn get_u128(env: Env, key: BytesN<32>) -> u128;
    fn get_address(env: Env, key: BytesN<32>) -> Option<Address>;
}

#[allow(dead_code)]
#[soroban_sdk::contractclient(name = "OracleClient")]
trait IOracle {
    fn get_primary_price(env: Env, token: Address) -> PriceProps;
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MarketUtilisation {
    pub pool_value_usd: u128,
    pub long_open_interest_usd: u128,
    pub short_open_interest_usd: u128,
    pub long_utilisation_bps: u32,
    pub short_utilisation_bps: u32,
    pub combined_utilisation_bps: u32,
    pub is_at_long_oi_cap: bool,
    pub is_at_short_oi_cap: bool,
}

#[contract]
pub struct MarketUtilReader;

#[contractimpl]
impl MarketUtilReader {
    pub fn get_market_utilisation(
        env: Env,
        data_store: Address,
        oracle: Address,
        market: Address,
    ) -> MarketUtilisation {
        let market_props = load_market(&env, &data_store, &market);
        let oracle_client = OracleClient::new(&env, &oracle);

        let long_price = oracle_client
            .get_primary_price(&market_props.long_token)
            .mid_price();
        let short_price = oracle_client
            .get_primary_price(&market_props.short_token)
            .mid_price();
        let index_price = oracle_client
            .get_primary_price(&market_props.index_token)
            .mid_price();

        let pool = get_pool_value(
            &env,
            &data_store,
            &market_props,
            long_price,
            short_price,
            index_price,
            false,
        );
        let pool_value_usd = if pool.pool_value <= 0 {
            0
        } else {
            pool.pool_value as u128
        };

        let long_open_interest_usd = get_open_interest_for_side(&env, &data_store, &market_props, true);
        let short_open_interest_usd = get_open_interest_for_side(&env, &data_store, &market_props, false);
        let combined_open_interest_usd = long_open_interest_usd.saturating_add(short_open_interest_usd);

        let ds = DataStoreClient::new(&env, &data_store);
        let long_cap = ds.get_u128(&max_open_interest_key(&env, &market, true));
        let short_cap = ds.get_u128(&max_open_interest_key(&env, &market, false));

        MarketUtilisation {
            pool_value_usd,
            long_open_interest_usd,
            short_open_interest_usd,
            long_utilisation_bps: utilisation_bps(long_open_interest_usd, pool_value_usd),
            short_utilisation_bps: utilisation_bps(short_open_interest_usd, pool_value_usd),
            combined_utilisation_bps: utilisation_bps(combined_open_interest_usd, pool_value_usd),
            is_at_long_oi_cap: long_cap != 0 && long_open_interest_usd >= long_cap,
            is_at_short_oi_cap: short_cap != 0 && short_open_interest_usd >= short_cap,
        }
    }
}

fn load_market(env: &Env, data_store: &Address, market: &Address) -> MarketProps {
    let ds = DataStoreClient::new(env, data_store);
    let index_token = ds
        .get_address(&market_index_token_key(env, market))
        .expect("market index token not found");
    let long_token = ds
        .get_address(&market_long_token_key(env, market))
        .expect("market long token not found");
    let short_token = ds
        .get_address(&market_short_token_key(env, market))
        .expect("market short token not found");

    MarketProps {
        market_token: market.clone(),
        index_token,
        long_token,
        short_token,
    }
}

fn utilisation_bps(open_interest_usd: u128, pool_value_usd: u128) -> u32 {
    if pool_value_usd == 0 {
        return u32::MAX;
    }

    let bps = open_interest_usd.saturating_mul(BPS_DIVISOR) / pool_value_usd;
    bps.min(u32::MAX as u128) as u32
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fifty_percent_utilisation_is_5000_bps() {
        assert_eq!(utilisation_bps(50_000, 100_000), 5_000);
    }

    #[test]
    fn zero_pool_value_returns_max_bps() {
        assert_eq!(utilisation_bps(50_000, 0), u32::MAX);
    }

    #[test]
    fn utilisation_rounds_down_within_one_bps() {
        assert_eq!(utilisation_bps(1, 3), 3_333);
    }

    // ── Issue #363: integration test for get_market_utilisation ─────────────

    use data_store::{DataStore, DataStoreClient as DsClient};
    use deposit_handler::{DepositHandler, DepositHandlerClient};
    use deposit_vault::{DepositVault, DepositVaultClient as DVClient};
    use gmx_keys::roles;
    use gmx_types::{CreateDepositParams, TokenPrice};
    use market_token::{MarketToken, MarketTokenClient as MtClient};
    use oracle::{Oracle, OracleClient as OClient};
    use role_store::{RoleStore, RoleStoreClient as RsClient};
    use soroban_sdk::{testutils::Address as _, token::StellarAssetClient, Env, Vec};

    struct World {
        env: Env,
        admin: Address,
        keeper: Address,
        rs: Address,
        ds: Address,
        oracle: Address,
        vault: Address,
        dep_handler: Address,
        market_tk: Address,
        long_tk: Address,
        short_tk: Address,
        index_tk: Address,
    }

    fn setup() -> World {
        let env = Env::default();
        env.mock_all_auths();
        env.budget().reset_unlimited();

        let admin = Address::generate(&env);
        let keeper = Address::generate(&env);

        let rs = env.register(RoleStore, ());
        let rs_c = RsClient::new(&env, &rs);
        rs_c.initialize(&admin);
        rs_c.grant_role(&admin, &admin, &roles::controller(&env));
        rs_c.grant_role(&admin, &keeper, &roles::order_keeper(&env));

        let ds = env.register(DataStore, ());
        DsClient::new(&env, &ds).initialize(&admin, &rs);

        let oracle_addr = env.register(Oracle, ());
        let passphrase = soroban_sdk::Bytes::from_slice(&env, b"Test SDF Network ; September 2015");
        OClient::new(&env, &oracle_addr).initialize(&admin, &rs, &ds, &passphrase);

        let vault = env.register(DepositVault, ());
        DVClient::new(&env, &vault).initialize(&admin, &rs);

        let long_tk = env
            .register_stellar_asset_contract_v2(admin.clone())
            .address();
        let short_tk = env
            .register_stellar_asset_contract_v2(admin.clone())
            .address();
        let index_tk = Address::generate(&env);

        let market_tk = env.register(MarketToken, ());
        MtClient::new(&env, &market_tk).initialize(
            &admin,
            &rs,
            &7u32,
            &soroban_sdk::String::from_str(&env, "GMX Market Token"),
            &soroban_sdk::String::from_str(&env, "GM"),
            &long_tk,
            &short_tk,
        );

        let dep_handler = env.register(DepositHandler, ());
        DepositHandlerClient::new(&env, &dep_handler).initialize(
            &admin,
            &rs,
            &ds,
            &oracle_addr,
            &vault,
        );
        rs_c.grant_role(&admin, &dep_handler, &roles::controller(&env));

        let ds_c = DsClient::new(&env, &ds);
        ds_c.set_address(
            &dep_handler,
            &gmx_keys::market_index_token_key(&env, &market_tk),
            &index_tk,
        );
        ds_c.set_address(
            &dep_handler,
            &gmx_keys::market_long_token_key(&env, &market_tk),
            &long_tk,
        );
        ds_c.set_address(
            &dep_handler,
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
            dep_handler,
            market_tk,
            long_tk,
            short_tk,
            index_tk,
        }
    }

    fn set_prices(w: &World, long_usd: i128, short_usd: i128, index_usd: i128) {
        OClient::new(&w.env, &w.oracle).set_prices_simple(
            &w.keeper,
            &Vec::from_array(
                &w.env,
                [
                    TokenPrice {
                        token: w.long_tk.clone(),
                        min: long_usd,
                        max: long_usd,
                    },
                    TokenPrice {
                        token: w.short_tk.clone(),
                        min: short_usd,
                        max: short_usd,
                    },
                    TokenPrice {
                        token: w.index_tk.clone(),
                        min: index_usd,
                        max: index_usd,
                    },
                ],
            ),
        );
    }

    /// Helper: deposit long+short tokens and return the minted LP balance.
    fn do_deposit(w: &World, user: &Address, long_amount: i128, short_amount: i128) -> i128 {
        let dep_key = DepositHandlerClient::new(&w.env, &w.dep_handler).create_deposit(
            user,
            &CreateDepositParams {
                receiver: user.clone(),
                market: w.market_tk.clone(),
                initial_long_token: w.long_tk.clone(),
                initial_short_token: w.short_tk.clone(),
                long_token_amount: long_amount,
                short_token_amount: short_amount,
                min_market_tokens: 1,
                execution_fee: 0,
            },
        );
        DepositHandlerClient::new(&w.env, &w.dep_handler).execute_deposit(&w.keeper, &dep_key);
        MtClient::new(&w.env, &w.market_tk).balance(user)
    }

    /// Full integration test: register market, seed pool, set OI caps, and
    /// verify the complete MarketUtilisation struct returned by
    /// get_market_utilisation, including the OI-cap boolean flags.
    #[test]
    fn get_market_utilisation_returns_correct_struct() {
        let w = setup();
        let fp = gmx_math::FLOAT_PRECISION;
        let user = Address::generate(&w.env);

        // Mint tokens for the user and seed the pool
        StellarAssetClient::new(&w.env, &w.long_tk).mint(&user, &10_000_0000i128);
        StellarAssetClient::new(&w.env, &w.short_tk).mint(&user, &5_000_0000i128);
        set_prices(&w, 2000 * fp, fp, 2000 * fp);

        let lp = do_deposit(&w, &user, 10_000_0000, 5_000_0000);
        assert!(lp > 0);

        let ds_c = DsClient::new(&w.env, &w.ds);

        // Set OI caps: long cap = 4_000_0000 (in raw tokens, will be USD later)
        // For the test, set caps in USD-equivalent using FLOAT_PRECISION
        let long_cap: u128 = 40_000 * fp as u128; // $40,000
        let short_cap: u128 = 20_000 * fp as u128; // $20,000
        ds_c.set_u128(
            &w.admin,
            &gmx_keys::max_open_interest_key(&w.env, &w.market_tk, true),
            &long_cap,
        );
        ds_c.set_u128(
            &w.admin,
            &gmx_keys::max_open_interest_key(&w.env, &w.market_tk, false),
            &short_cap,
        );

        // Call get_market_utilisation
        let result = MarketUtilReaderClient::new(&w.env, &w.env.register_contract(None, MarketUtilReader))
            .get_market_utilisation(&w.ds, &w.oracle, &w.market_tk);

        // Pool value: 10_000 long tokens at $2000 + 5_000 short tokens at $1
        // = $20,000,000 + $5,000 = $20,005,000
        // In raw: 10_000_0000 * 2000 * fp / TOKEN_PRECISION + 5_000_0000 * fp / TOKEN_PRECISION
        let long_usd = gmx_math::mul_div_wide(&w.env, 10_000_0000i128, 2000 * fp, gmx_math::TOKEN_PRECISION);
        let short_usd = gmx_math::mul_div_wide(&w.env, 5_000_0000i128, fp, gmx_math::TOKEN_PRECISION);
        let expected_pool = (long_usd + short_usd) as u128;

        assert_eq!(result.pool_value_usd, expected_pool);

        // No OI set → OI values should be 0
        assert_eq!(result.long_open_interest_usd, 0);
        assert_eq!(result.short_open_interest_usd, 0);

        // Utilisation bps: 0 OI / pool = 0
        assert_eq!(result.long_utilisation_bps, 0);
        assert_eq!(result.short_utilisation_bps, 0);
        assert_eq!(result.combined_utilisation_bps, 0);

        // OI caps: caps are set but no OI → not at cap
        assert!(!result.is_at_long_oi_cap);
        assert!(!result.is_at_short_oi_cap);
    }

    /// OI-cap flag becomes true when open interest reaches the cap.
    /// We simulate this by writing open interest directly into data_store
    /// and checking the boolean flag.
    #[test]
    fn oi_cap_flag_true_when_at_cap() {
        let w = setup();
        let fp = gmx_math::FLOAT_PRECISION;
        let user = Address::generate(&w.env);

        StellarAssetClient::new(&w.env, &w.long_tk).mint(&user, &10_000_0000i128);
        StellarAssetClient::new(&w.env, &w.short_tk).mint(&user, &5_000_0000i128);
        set_prices(&w, 2000 * fp, fp, 2000 * fp);

        let _lp = do_deposit(&w, &user, 10_000_0000, 5_000_0000);

        let ds_c = DsClient::new(&w.env, &w.ds);

        // Set long OI cap to exactly what we'll set as OI
        let oi_value: u128 = 10_000 * fp as u128;
        ds_c.set_u128(
            &w.admin,
            &gmx_keys::max_open_interest_key(&w.env, &w.market_tk, true),
            &oi_value,
        );
        // Write open interest directly (simulating a position being opened)
        ds_c.set_u128(
            &w.admin,
            &gmx_keys::open_interest_key(&w.env, &w.market_tk, &w.long_tk, true),
            &oi_value,
        );

        let result = MarketUtilReaderClient::new(&w.env, &w.env.register_contract(None, MarketUtilReader))
            .get_market_utilisation(&w.ds, &w.oracle, &w.market_tk);

        assert!(result.is_at_long_oi_cap, "long OI at cap should be true");
        assert!(!result.is_at_short_oi_cap, "short OI cap not set → not at cap");
    }

    /// Zero pool value (empty pool) returns 0 for pool_value_usd and
    /// u32::MAX for utilisation bps.
    #[test]
    fn empty_pool_returns_zero_pool_value_and_max_bps() {
        let w = setup();
        let fp = gmx_math::FLOAT_PRECISION;
        set_prices(&w, 2000 * fp, fp, 2000 * fp);

        // Write open interest with no pool → utilisation = MAX
        let ds_c = DsClient::new(&w.env, &w.ds);
        ds_c.set_u128(
            &w.admin,
            &gmx_keys::open_interest_key(&w.env, &w.market_tk, &w.long_tk, true),
            &(5_000 * fp as u128),
        );

        let result = MarketUtilReaderClient::new(&w.env, &w.env.register_contract(None, MarketUtilReader))
            .get_market_utilisation(&w.ds, &w.oracle, &w.market_tk);

        assert_eq!(result.pool_value_usd, 0);
        assert_eq!(result.long_utilisation_bps, u32::MAX);
        assert_eq!(result.short_utilisation_bps, u32::MAX);
        assert_eq!(result.combined_utilisation_bps, u32::MAX);
    }
}
