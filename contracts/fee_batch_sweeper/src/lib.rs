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

        for i in 0..market_count {
            let market = markets.get_unchecked(i);
            for j in 0..token_count {
                let token = tokens.get_unchecked(j);
                let claimed = fee_handler_client.claim_fees(&keeper, &market, &token, &receiver);
                total_claimed = total_claimed.saturating_add(claimed);
                claims_attempted = claims_attempted.saturating_add(1);
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
}
