//! Batch fee sweeper — claims protocol fees across many market/token pairs in one call.
//!
//! This contract is intentionally small and delegates each individual claim to the
//! canonical `fee_handler::claim_fees` entry point so existing accounting,
//! zero-balance skipping, pool-balance caps, and FEE_KEEPER role checks remain the
//! single source of truth.
#![no_std]

use fee_handler::FeeHandlerClient;
use soroban_sdk::{
    contract, contracterror, contractevent, contractimpl, contracttype, panic_with_error, Address,
    Env, Vec,
};

pub const MAX_BATCH_CLAIM_SIZE: u32 = 20;

#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
#[repr(u32)]
pub enum Error {
    TooManyEntries = 1,
}

#[contractevent(topics = ["fee_batch"])]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BatchFeesClaimed {
    pub keeper: Address,
    pub receiver: Address,
    pub market_count: u32,
    pub token_count: u32,
    pub total_claimed: u128,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BatchClaimResult {
    pub markets: u32,
    pub tokens: u32,
    pub claims_attempted: u32,
    pub total_claimed: u128,
    /// (market, token) pairs whose `claim_fees` call failed — isolated so one
    /// bad pair does not revert the fees already claimed for every other pair.
    pub failed: Vec<(Address, Address)>,
}

#[contract]
pub struct FeeBatchSweeper;

#[contractimpl]
impl FeeBatchSweeper {
    /// Claim protocol fees across all market/token combinations in one call.
    ///
    /// `fee_handler` remains responsible for the actual transfer and the
    /// FEE_KEEPER authorization check. Zero balances are skipped because
    /// `fee_handler::claim_fees` returns `0` for them.
    pub fn claim_all_fees(
        env: Env,
        fee_handler: Address,
        keeper: Address,
        receiver: Address,
        markets: Vec<Address>,
        tokens: Vec<Address>,
    ) -> BatchClaimResult {
        keeper.require_auth();

        let market_count = markets.len();
        let token_count = tokens.len();
        let combinations = market_count.saturating_mul(token_count);
        if market_count > MAX_BATCH_CLAIM_SIZE
            || token_count > MAX_BATCH_CLAIM_SIZE
            || combinations > MAX_BATCH_CLAIM_SIZE
        {
            panic_with_error!(&env, Error::TooManyEntries);
        }

        let fee_handler_client = FeeHandlerClient::new(&env, &fee_handler);
        let mut total_claimed: u128 = 0;
        let mut claims_attempted: u32 = 0;
        let mut failed: Vec<(Address, Address)> = Vec::new(&env);

        for i in 0..market_count {
            let market = markets.get_unchecked(i);
            for j in 0..token_count {
                let token = tokens.get_unchecked(j);
                claims_attempted = claims_attempted.saturating_add(1);
                // Use try_claim_fees so a panic on any single pair (e.g. issue #254's
                // InsufficientPoolBalance guard, a paused token, or a market missing the
                // sweeper's controller role) is isolated to that pair instead of
                // reverting the whole batch and every fee already claimed in this call.
                match fee_handler_client.try_claim_fees(&keeper, &market, &token, &receiver) {
                    Ok(Ok(claimed)) => {
                        total_claimed = total_claimed.saturating_add(claimed);
                    }
                    _ => {
                        failed.push_back((market.clone(), token));
                    }
                }
            }
        }

        env.events().publish_event(&BatchFeesClaimed {
            keeper,
            receiver,
            market_count,
            token_count,
            total_claimed,
        });

        BatchClaimResult {
            markets: market_count,
            tokens: token_count,
            claims_attempted,
            total_claimed,
            failed,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use soroban_sdk::{testutils::Address as _, Env};

    #[test]
    fn max_batch_constant_matches_issue_bound() {
        assert_eq!(MAX_BATCH_CLAIM_SIZE, 20);
    }

    #[test]
    fn empty_batches_do_not_attempt_claims() {
        let env = Env::default();
        env.mock_all_auths();
        let contract_id = env.register(FeeBatchSweeper, ());
        let client = FeeBatchSweeperClient::new(&env, &contract_id);

        let result = client.claim_all_fees(
            &Address::generate(&env),
            &Address::generate(&env),
            &Address::generate(&env),
            &Vec::new(&env),
            &Vec::new(&env),
        );

        assert_eq!(result.claims_attempted, 0);
        assert_eq!(result.total_claimed, 0);
    }

    #[test]
    fn too_many_markets_panics() {
        let env = Env::default();
        env.mock_all_auths();
        let contract_id = env.register(FeeBatchSweeper, ());
        let client = FeeBatchSweeperClient::new(&env, &contract_id);

        let mut markets: Vec<Address> = Vec::new(&env);
        for _ in 0..=MAX_BATCH_CLAIM_SIZE {
            markets.push_back(Address::generate(&env));
        }
        let tokens = soroban_sdk::vec![&env, Address::generate(&env)];

        let result = client.try_claim_all_fees(
            &Address::generate(&env),
            &Address::generate(&env),
            &Address::generate(&env),
            &markets,
            &tokens,
        );
        assert!(result.is_err());
    }

    #[test]
    fn too_many_tokens_panics() {
        let env = Env::default();
        env.mock_all_auths();
        let contract_id = env.register(FeeBatchSweeper, ());
        let client = FeeBatchSweeperClient::new(&env, &contract_id);

        let markets = soroban_sdk::vec![&env, Address::generate(&env)];
        let mut tokens: Vec<Address> = Vec::new(&env);
        for _ in 0..=MAX_BATCH_CLAIM_SIZE {
            tokens.push_back(Address::generate(&env));
        }

        let result = client.try_claim_all_fees(
            &Address::generate(&env),
            &Address::generate(&env),
            &Address::generate(&env),
            &markets,
            &tokens,
        );
        assert!(result.is_err());
    }

    #[test]
    fn product_exceeds_limit_panics() {
        let env = Env::default();
        env.mock_all_auths();
        let contract_id = env.register(FeeBatchSweeper, ());
        let client = FeeBatchSweeperClient::new(&env, &contract_id);

        // 5 markets × 5 tokens = 25 > 20
        let mut markets: Vec<Address> = Vec::new(&env);
        let mut tokens: Vec<Address> = Vec::new(&env);
        for _ in 0..5 {
            markets.push_back(Address::generate(&env));
            tokens.push_back(Address::generate(&env));
        }

        let result = client.try_claim_all_fees(
            &Address::generate(&env),
            &Address::generate(&env),
            &Address::generate(&env),
            &markets,
            &tokens,
        );
        assert!(result.is_err());
    }

    /// Issue #539: one bad (market, token) pair — here, an InsufficientPoolBalance
    /// panic in fee_handler — must not revert the whole batch. The healthy pair's
    /// fees are still claimed and reflected in the result, and the failing pair is
    /// reported in `failed` rather than silently dropped.
    #[test]
    fn one_panicking_pair_does_not_revert_the_rest_of_the_batch() {
        use data_store::{DataStore, DataStoreClient as DsClient};
        use fee_handler::{FeeHandler, FeeHandlerClient};
        use gmx_keys::roles;
        use market_token::{MarketToken, MarketTokenClient as MtClient};
        use role_store::{RoleStore, RoleStoreClient as RsClient};
        use soroban_sdk::token::StellarAssetClient;

        const ONE_TOKEN: i128 = 10_000_000;

        let env = Env::default();
        env.mock_all_auths();

        let admin = Address::generate(&env);
        let keeper = Address::generate(&env);
        let receiver = Address::generate(&env);

        let rs = env.register(RoleStore, ());
        let rs_c = RsClient::new(&env, &rs);
        rs_c.initialize(&admin);
        rs_c.grant_role(&admin, &admin, &roles::controller(&env));
        rs_c.grant_role(&admin, &keeper, &roles::fee_keeper(&env));

        let ds = env.register(DataStore, ());
        DsClient::new(&env, &ds).initialize(&admin, &rs);

        let token = env
            .register_stellar_asset_contract_v2(admin.clone())
            .address();

        let make_market = |name: &str| {
            let market_tk = env.register(MarketToken, ());
            MtClient::new(&env, &market_tk).initialize(
                &admin,
                &rs,
                &7u32,
                &soroban_sdk::String::from_str(&env, name),
                &soroban_sdk::String::from_str(&env, "GM"),
                &token,
                &token,
            );
            rs_c.grant_role(&admin, &market_tk, &roles::controller(&env));
            market_tk
        };

        // Bad pair: claimable exceeds pool_amount accounting, so claim_fees panics
        // with InsufficientPoolBalance (issue #254's guard).
        let bad_market = make_market("Bad Market");
        // Healthy pair: claimable is fully backed and the pool actually holds the tokens.
        let good_market = make_market("Good Market");

        let handler = env.register(FeeHandler, ());
        FeeHandlerClient::new(&env, &handler).initialize(&admin, &rs, &ds);
        rs_c.grant_role(&admin, &handler, &roles::controller(&env));

        let ds_c = DsClient::new(&env, &ds);

        let bad_claimable: u128 = ONE_TOKEN as u128 * 5;
        let bad_pool_amount: u128 = ONE_TOKEN as u128 * 3; // less than claimable -> panics
        ds_c.set_u128(
            &admin,
            &gmx_keys::claimable_fee_amount_key(&env, &bad_market, &token),
            &bad_claimable,
        );
        ds_c.set_u128(
            &admin,
            &gmx_keys::pool_amount_key(&env, &bad_market, &token),
            &bad_pool_amount,
        );
        StellarAssetClient::new(&env, &token).mint(&bad_market, &(bad_claimable as i128));

        let good_claimable: u128 = ONE_TOKEN as u128 * 2;
        ds_c.set_u128(
            &admin,
            &gmx_keys::claimable_fee_amount_key(&env, &good_market, &token),
            &good_claimable,
        );
        ds_c.set_u128(
            &admin,
            &gmx_keys::pool_amount_key(&env, &good_market, &token),
            &good_claimable,
        );
        StellarAssetClient::new(&env, &token).mint(&good_market, &(good_claimable as i128));

        let contract_id = env.register(FeeBatchSweeper, ());
        let client = FeeBatchSweeperClient::new(&env, &contract_id);

        let markets = soroban_sdk::vec![&env, bad_market.clone(), good_market.clone()];
        let tokens = soroban_sdk::vec![&env, token.clone()];

        // Must NOT panic even though the bad pair would panic in fee_handler directly.
        let result = client.claim_all_fees(&handler, &keeper, &receiver, &markets, &tokens);

        assert_eq!(
            result.total_claimed, good_claimable,
            "the healthy pair's fees must still be claimed despite the other pair failing"
        );
        assert_eq!(result.claims_attempted, 2);
        assert_eq!(
            result.failed,
            soroban_sdk::vec![&env, (bad_market, token.clone())],
            "the failing pair must be reported rather than silently dropped"
        );

        let recv_bal = soroban_sdk::token::Client::new(&env, &token).balance(&receiver);
        assert_eq!(recv_bal as u128, good_claimable);
    }
}
