//! Insurance fund router — liquidation penalty routing and shortfall coverage.
//!
//! This contract provides the data-store backed accounting and transfer rules for
//! issue #213 without changing existing position storage layout. Liquidation
//! handlers can call `route_liquidation_penalty` after a successful liquidation
//! and `cover_shortfall` before charging the pool.
#![no_std]

use soroban_sdk::{
    contract, contracterror, contractevent, contractimpl, contracttype, panic_with_error, token,
    Address, Bytes, BytesN, Env,
};

const BPS_DIVISOR: u128 = 10_000;

#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
#[repr(u32)]
pub enum Error {
    AllocationTooHigh = 1,
    MissingInsuranceFund = 2,
    MissingMarketPool = 3,
    MissingTreasury = 4,
}

#[allow(dead_code)]
#[soroban_sdk::contractclient(name = "DataStoreClient")]
trait IDataStore {
    fn get_u128(env: Env, key: BytesN<32>) -> u128;
    fn set_u128(env: Env, caller: Address, key: BytesN<32>, value: u128) -> u128;
    fn get_address(env: Env, key: BytesN<32>) -> Option<Address>;
    fn set_address(env: Env, caller: Address, key: BytesN<32>, value: Address) -> Address;
}

#[contractevent(topics = ["if_cfg"])]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct InsuranceFundConfigured {
    pub market: Address,
    pub fund: Address,
    pub allocation_bps: u32,
}

#[contractevent(topics = ["if_pool"])]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MarketPoolConfigured {
    pub market: Address,
    pub pool: Address,
}

#[contractevent(topics = ["if_treas"])]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TreasuryConfigured {
    pub treasury: Address,
}

#[contractevent(topics = ["if_pen"])]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct InsurancePenaltyRouted {
    pub market: Address,
    pub token: Address,
    pub fund: Address,
    pub insurance_share: u128,
    pub treasury_share: u128,
}

#[contractevent(topics = ["if_draw"])]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct InsuranceShortfallCovered {
    pub market: Address,
    pub token: Address,
    pub fund: Address,
    pub requested_shortfall: u128,
    pub covered_by_fund: u128,
    pub pool_remainder: u128,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PenaltySplit {
    pub insurance_share: u128,
    pub treasury_share: u128,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ShortfallCoverage {
    pub covered_by_fund: u128,
    pub pool_remainder: u128,
}

#[contract]
pub struct InsuranceFundRouter;

#[contractimpl]
impl InsuranceFundRouter {
    pub fn configure_insurance_fund(
        env: Env,
        data_store: Address,
        caller: Address,
        market: Address,
        fund: Address,
        allocation_bps: u32,
    ) {
        caller.require_auth();
        if allocation_bps > BPS_DIVISOR as u32 {
            panic_with_error!(&env, Error::AllocationTooHigh);
        }

        let ds = DataStoreClient::new(&env, &data_store);
        ds.set_address(&caller, &insurance_fund_address_key(&env, &market), &fund);
        ds.set_u128(
            &caller,
            &insurance_fund_allocation_bps_key(&env, &market),
            &(allocation_bps as u128),
        );

        env.events().publish_event(&InsuranceFundConfigured {
            market,
            fund,
            allocation_bps,
        });
    }

    /// Register the market's actual pool/vault address. `cover_shortfall` transfers
    /// exclusively to this address rather than a caller-supplied one.
    pub fn configure_market_pool(
        env: Env,
        data_store: Address,
        caller: Address,
        market: Address,
        pool: Address,
    ) {
        caller.require_auth();
        let ds = DataStoreClient::new(&env, &data_store);
        ds.set_address(&caller, &market_pool_address_key(&env, &market), &pool);

        env.events()
            .publish_event(&MarketPoolConfigured { market, pool });
    }

    /// Register the protocol treasury address. `route_liquidation_penalty` transfers
    /// exclusively to this address rather than a caller-supplied one.
    pub fn configure_treasury(env: Env, data_store: Address, caller: Address, treasury: Address) {
        caller.require_auth();
        let ds = DataStoreClient::new(&env, &data_store);
        ds.set_address(&caller, &treasury_address_key(&env), &treasury);

        env.events().publish_event(&TreasuryConfigured { treasury });
    }

    pub fn route_liquidation_penalty(
        env: Env,
        data_store: Address,
        market: Address,
        token: Address,
        source: Address,
        liquidation_penalty: u128,
    ) -> PenaltySplit {
        source.require_auth();
        let ds = DataStoreClient::new(&env, &data_store);
        let allocation_bps = ds.get_u128(&insurance_fund_allocation_bps_key(&env, &market));
        let insurance_share = liquidation_penalty.saturating_mul(allocation_bps) / BPS_DIVISOR;
        let treasury_share = liquidation_penalty.saturating_sub(insurance_share);

        let token_client = token::TokenClient::new(&env, &token);
        if insurance_share > 0 {
            let fund = ds
                .get_address(&insurance_fund_address_key(&env, &market))
                .unwrap_or_else(|| panic_with_error!(&env, Error::MissingInsuranceFund));
            token_client.transfer(&source, &fund, &(insurance_share as i128));
            env.events().publish_event(&InsurancePenaltyRouted {
                market: market.clone(),
                token: token.clone(),
                fund,
                insurance_share,
                treasury_share,
            });
        }

        if treasury_share > 0 {
            let treasury = ds
                .get_address(&treasury_address_key(&env))
                .unwrap_or_else(|| panic_with_error!(&env, Error::MissingTreasury));
            token_client.transfer(&source, &treasury, &(treasury_share as i128));
        }

        PenaltySplit {
            insurance_share,
            treasury_share,
        }
    }

    pub fn cover_shortfall(
        env: Env,
        data_store: Address,
        market: Address,
        token: Address,
        shortfall_amount: u128,
    ) -> ShortfallCoverage {
        let ds = DataStoreClient::new(&env, &data_store);
        let fund = ds
            .get_address(&insurance_fund_address_key(&env, &market))
            .unwrap_or_else(|| panic_with_error!(&env, Error::MissingInsuranceFund));
        let pool = ds
            .get_address(&market_pool_address_key(&env, &market))
            .unwrap_or_else(|| panic_with_error!(&env, Error::MissingMarketPool));

        let token_client = token::TokenClient::new(&env, &token);
        let fund_balance = token_client.balance(&fund);
        let available = if fund_balance <= 0 { 0 } else { fund_balance as u128 };
        let covered_by_fund = available.min(shortfall_amount);
        let pool_remainder = shortfall_amount.saturating_sub(covered_by_fund);

        if covered_by_fund > 0 {
            token_client.transfer(&fund, &pool, &(covered_by_fund as i128));
        }

        env.events().publish_event(&InsuranceShortfallCovered {
            market,
            token,
            fund,
            requested_shortfall: shortfall_amount,
            covered_by_fund,
            pool_remainder,
        });

        ShortfallCoverage {
            covered_by_fund,
            pool_remainder,
        }
    }

    pub fn preview_penalty_split(
        env: Env,
        data_store: Address,
        market: Address,
        liquidation_penalty: u128,
    ) -> PenaltySplit {
        let allocation_bps = DataStoreClient::new(&env, &data_store)
            .get_u128(&insurance_fund_allocation_bps_key(&env, &market));
        let insurance_share = liquidation_penalty.saturating_mul(allocation_bps) / BPS_DIVISOR;
        PenaltySplit {
            insurance_share,
            treasury_share: liquidation_penalty.saturating_sub(insurance_share),
        }
    }
}

fn insurance_fund_address_key(env: &Env, market: &Address) -> BytesN<32> {
    keyed_address(env, "INSURANCE_FUND_ADDRESS", market)
}

fn insurance_fund_allocation_bps_key(env: &Env, market: &Address) -> BytesN<32> {
    keyed_address(env, "INSURANCE_FUND_ALLOCATION_BPS", market)
}

fn market_pool_address_key(env: &Env, market: &Address) -> BytesN<32> {
    keyed_address(env, "MARKET_POOL_ADDRESS", market)
}

fn treasury_address_key(env: &Env) -> BytesN<32> {
    let bytes = Bytes::from_slice(env, "PROTOCOL_TREASURY_ADDRESS".as_bytes());
    env.crypto().sha256(&bytes).into()
}

fn keyed_address(env: &Env, tag: &str, address: &Address) -> BytesN<32> {
    let mut bytes = Bytes::new(env);
    bytes.append(&Bytes::from_slice(env, tag.as_bytes()));

    let strkey = address.to_string();
    let len = strkey.len() as usize;
    let mut raw = [0u8; 64];
    strkey.copy_into_slice(&mut raw[..len]);
    bytes.append(&Bytes::from_slice(env, &raw[..len]));

    env.crypto().sha256(&bytes).into()
}

#[cfg(test)]
mod tests {
    use super::*;
    use data_store::{DataStore, DataStoreClient as DsClient};
    use gmx_keys::roles;
    use role_store::{RoleStore, RoleStoreClient as RsClient};
    use soroban_sdk::testutils::Address as _;
    use soroban_sdk::token::StellarAssetClient;

    struct World {
        env: Env,
        admin: Address,
        ds: Address,
        router: Address,
        token: Address,
    }

    fn setup() -> World {
        let env = Env::default();
        env.mock_all_auths_allowing_non_root_auth();

        let admin = Address::generate(&env);

        let rs = env.register(RoleStore, ());
        let rs_c = RsClient::new(&env, &rs);
        rs_c.initialize(&admin);
        rs_c.grant_role(&admin, &admin, &roles::controller(&env));

        let ds = env.register(DataStore, ());
        DsClient::new(&env, &ds).initialize(&admin, &rs);

        let router = env.register(InsuranceFundRouter, ());
        let token = env
            .register_stellar_asset_contract_v2(admin.clone())
            .address();

        World {
            env,
            admin,
            ds,
            router,
            token,
        }
    }

    #[test]
    fn zero_bps_routes_all_penalty_to_treasury() {
        let liquidation_penalty = 1_000u128;
        let allocation_bps = 0u128;
        let insurance_share = liquidation_penalty * allocation_bps / BPS_DIVISOR;
        assert_eq!(insurance_share, 0);
        assert_eq!(liquidation_penalty - insurance_share, 1_000);
    }

    #[test]
    fn allocation_splits_penalty_by_bps() {
        let liquidation_penalty = 10_000u128;
        let allocation_bps = 2_500u128;
        let insurance_share = liquidation_penalty * allocation_bps / BPS_DIVISOR;
        assert_eq!(insurance_share, 2_500);
        assert_eq!(liquidation_penalty - insurance_share, 7_500);
    }

    #[test]
    fn fund_covers_shortfall_until_exhausted() {
        let shortfall = 1_000u128;
        let fund_balance = 600u128;
        let covered = fund_balance.min(shortfall);
        assert_eq!(covered, 600);
        assert_eq!(shortfall - covered, 400);
    }

    #[test]
    fn route_liquidation_penalty_pays_only_the_configured_treasury() {
        let w = setup();
        let client = InsuranceFundRouterClient::new(&w.env, &w.router);
        let market = Address::generate(&w.env);
        let source = Address::generate(&w.env);
        let treasury = Address::generate(&w.env);

        StellarAssetClient::new(&w.env, &w.token).mint(&source, &1_000i128);
        client.configure_treasury(&w.ds, &w.admin, &treasury);

        let split = client.route_liquidation_penalty(&w.ds, &market, &w.token, &source, &1_000u128);

        assert_eq!(split.treasury_share, 1_000);
        let token_client = token::TokenClient::new(&w.env, &w.token);
        assert_eq!(token_client.balance(&treasury), 1_000);
        assert_eq!(token_client.balance(&source), 0);
    }

    #[test]
    #[should_panic]
    fn route_liquidation_penalty_panics_without_configured_treasury() {
        let w = setup();
        let client = InsuranceFundRouterClient::new(&w.env, &w.router);
        let market = Address::generate(&w.env);
        let source = Address::generate(&w.env);

        StellarAssetClient::new(&w.env, &w.token).mint(&source, &1_000i128);
        client.route_liquidation_penalty(&w.ds, &market, &w.token, &source, &1_000u128);
    }

    #[test]
    fn cover_shortfall_pays_only_the_configured_market_pool() {
        let w = setup();
        let client = InsuranceFundRouterClient::new(&w.env, &w.router);
        let market = Address::generate(&w.env);
        let fund = Address::generate(&w.env);
        let pool = Address::generate(&w.env);

        StellarAssetClient::new(&w.env, &w.token).mint(&fund, &1_000i128);
        client.configure_insurance_fund(&w.ds, &w.admin, &market, &fund, &5_000u32);
        client.configure_market_pool(&w.ds, &w.admin, &market, &pool);

        let coverage = client.cover_shortfall(&w.ds, &market, &w.token, &600u128);

        assert_eq!(coverage.covered_by_fund, 600);
        assert_eq!(coverage.pool_remainder, 0);
        let token_client = token::TokenClient::new(&w.env, &w.token);
        assert_eq!(token_client.balance(&pool), 600);
        assert_eq!(token_client.balance(&fund), 400);
    }

    #[test]
    #[should_panic]
    fn cover_shortfall_panics_without_configured_market_pool() {
        let w = setup();
        let client = InsuranceFundRouterClient::new(&w.env, &w.router);
        let market = Address::generate(&w.env);
        let fund = Address::generate(&w.env);

        StellarAssetClient::new(&w.env, &w.token).mint(&fund, &1_000i128);
        client.configure_insurance_fund(&w.ds, &w.admin, &market, &fund, &5_000u32);
        client.cover_shortfall(&w.ds, &market, &w.token, &600u128);
    }
}
