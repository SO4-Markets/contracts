//! Fee handler — claims and distributes protocol fees accumulated in the pool.
//! Mirrors GMX's FeeHandler.sol.
//!
//! Fees collected during position open/close and swaps accumulate as pool amounts.
//! The fee handler lets a privileged keeper sweep those fees to a treasury address
//! and optionally distribute a portion to stakers via GLP/GM-style fee sharing.
//!
//! In this minimal implementation:
//!   - claimable_fees(market, token) → how much is claimable
//!   - claim_fees(keeper, market, token, receiver) → sweep to receiver
//!   - claim_funding_fees(account, market, token) → user claims earned funding
#![no_std]
#![allow(dependency_on_unit_never_type_fallback)]

use soroban_sdk::{
    contract, contracterror, contractimpl, Address, BytesN, Env,
};
use gmx_keys::{
    claimable_fee_amount_key, claimable_funding_amount_key,
};

// ─── Storage keys ─────────────────────────────────────────────────────────────

const ADMIN_KEY:      &str = "ADMIN";
const ROLE_STORE_KEY: &str = "ROLE_STORE";
const DATA_STORE_KEY: &str = "DATA_STORE";

// ─── Errors ───────────────────────────────────────────────────────────────────

#[contracterror]
pub enum Error {
    AlreadyInitialized = 1,
    NotInitialized     = 2,
    Unauthorized       = 3,
    NothingToClaim     = 4,
}

// ─── External clients ─────────────────────────────────────────────────────────

#[soroban_sdk::contractclient(name = "RoleStoreClient")]
trait IRoleStore {
    fn has_role(env: Env, account: Address, role: soroban_sdk::Symbol) -> bool;
}

#[soroban_sdk::contractclient(name = "DataStoreClient")]
trait IDataStore {
    fn get_u128(env: Env, key: BytesN<32>) -> u128;
    fn set_u128(env: Env, caller: Address, key: BytesN<32>, value: u128) -> u128;
    fn apply_delta_to_u128(env: Env, caller: Address, key: BytesN<32>, delta: i128) -> u128;
}

// ─── Contract ─────────────────────────────────────────────────────────────────

#[contract]
pub struct FeeHandler;

#[contractimpl]
impl FeeHandler {
    /// One-time setup.
    pub fn initialize(env: Env, admin: Address, role_store: Address, data_store: Address) {
        // TODO: panic if already initialized
        //       Store admin, role_store, data_store in instance storage
        todo!()
    }

    /// Return the accumulated protocol fee amount for a given market + token.
    pub fn claimable_fees(env: Env, market: Address, token: Address) -> u128 {
        // TODO:
        // 1. Load data_store from instance storage
        // 2. key = claimable_fee_amount_key(&env, &market, &token)
        // 3. Return ds.get_u128(key)
        todo!()
    }

    /// Sweep accumulated protocol fees for a market/token to `receiver`.
    ///
    /// Only FEE_KEEPER role may call this.
    pub fn claim_fees(
        env: Env,
        keeper: Address,
        market: Address,
        token: Address,
        receiver: Address,
    ) -> u128 {
        // TODO: (mirrors GMX FeeHandler.claimFees)
        //
        // 1. keeper.require_auth()
        //    Require FEE_KEEPER role
        //
        // 2. Load data_store from instance storage
        //
        // 3. key = claimable_fee_amount_key(&env, &market, &token)
        //    amount = ds.get_u128(key)
        //    if amount == 0 → panic Error::NothingToClaim
        //
        // 4. Reset to zero:
        //    ds.set_u128(&keeper, key, 0)
        //
        // 5. Transfer `amount` of `token` from market_token contract to receiver:
        //    MarketTokenClient::new(&env, &market)
        //        .withdraw_from_pool(&keeper, &token, &receiver, amount as i128)
        //
        // 6. Emit "fees_claimed" event with { market, token, amount, receiver }
        //
        // Returns amount claimed
        todo!()
    }

    /// Claim funding fees earned by a position account for a given market + collateral token.
    ///
    /// Any user can call this to collect their accrued funding credits.
    pub fn claim_funding_fees(
        env: Env,
        account: Address,
        market: Address,
        token: Address,
    ) -> u128 {
        // TODO: (mirrors GMX FeeHandler.claimFundingFees)
        //
        // 1. account.require_auth()
        //
        // 2. Load data_store from instance storage
        //
        // 3. key = claimable_funding_amount_key(&env, &market, &token, &account)
        //    amount = ds.get_u128(key)
        //    if amount == 0 → return 0 (nothing to claim, not an error)
        //
        // 4. Reset to zero:
        //    ds.set_u128(&env.current_contract_address(), key, 0)
        //
        // 5. Transfer `amount` of `token` from market_token contract to account:
        //    MarketTokenClient::new(&env, &market)
        //        .withdraw_from_pool(&env.current_contract_address(), &token, &account, amount as i128)
        //    (fee_handler must hold CONTROLLER role for this to work)
        //
        // 6. Emit "funding_fees_claimed" event with { account, market, token, amount }
        //
        // Returns amount claimed
        todo!()
    }
}
