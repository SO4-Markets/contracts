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
}
