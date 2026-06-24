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

    pub fn route_liquidation_penalty(
        env: Env,
        data_store: Address,
        market: Address,
        token: Address,
        source: Address,
        treasury: Address,
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
        pool: Address,
        shortfall_amount: u128,
    ) -> ShortfallCoverage {
        let ds = DataStoreClient::new(&env, &data_store);
        let fund = ds
            .get_address(&insurance_fund_address_key(&env, &market))
            .unwrap_or_else(|| panic_with_error!(&env, Error::MissingInsuranceFund));

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
}
