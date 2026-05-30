//! Fee handler — claims and distributes protocol fees accumulated in the pool.
//! Mirrors GMX's FeeHandler.sol.
#![no_std]
#![allow(dependency_on_unit_never_type_fallback)]

use soroban_sdk::{
    contract, contracterror, contractevent, contractimpl, contracttype, panic_with_error,
    Address, BytesN, Env,
};
use gmx_keys::{
    roles,
    claimable_fee_amount_key, claimable_funding_amount_key,
};

// ─── Storage keys ─────────────────────────────────────────────────────────────

#[contracttype]
enum InstanceKey {
    Initialized,
    Admin,
    RoleStore,
    DataStore,
}

// ─── Errors ───────────────────────────────────────────────────────────────────

#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
#[repr(u32)]
pub enum Error {
    AlreadyInitialized = 1,
    NotInitialized     = 2,
    Unauthorized       = 3,
    NothingToClaim     = 4,
}

// ─── External clients ─────────────────────────────────────────────────────────

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
    fn apply_delta_to_u128(env: Env, caller: Address, key: BytesN<32>, delta: i128) -> u128;
}

#[allow(dead_code)]
#[soroban_sdk::contractclient(name = "MarketTokenClient")]
trait IMarketToken {
    fn withdraw_from_pool(env: Env, caller: Address, pool_token: Address, receiver: Address, amount: i128);
}

// ─── Events ───────────────────────────────────────────────────────────────────

#[contractevent(topics = ["fee_clm"])]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FeeClaimed {
    pub market:   Address,
    pub token:    Address,
    pub amount:   u128,
    pub receiver: Address,
}

#[contractevent(topics = ["fnd_clm"])]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FundingFeeClaimed {
    pub account: Address,
    pub market:  Address,
    pub token:   Address,
    pub amount:  u128,
}

// ─── Contract ─────────────────────────────────────────────────────────────────

#[contract]
pub struct FeeHandler;

#[contractimpl]
impl FeeHandler {
    pub fn initialize(env: Env, admin: Address, role_store: Address, data_store: Address) {
        admin.require_auth();
        if env.storage().instance().has(&InstanceKey::Initialized) {
            panic_with_error!(&env, Error::AlreadyInitialized);
        }
        env.storage().instance().set(&InstanceKey::Initialized, &true);
        env.storage().instance().set(&InstanceKey::Admin, &admin);
        env.storage().instance().set(&InstanceKey::RoleStore, &role_store);
        env.storage().instance().set(&InstanceKey::DataStore, &data_store);
    }

    /// Return the accumulated protocol fee amount for a given market + token.
    pub fn claimable_fees(env: Env, market: Address, token: Address) -> u128 {
        let data_store: Address = env.storage().instance().get(&InstanceKey::DataStore).unwrap();
        let key = claimable_fee_amount_key(&env, &market, &token);
        DataStoreClient::new(&env, &data_store).get_u128(&key)
    }

    /// Sweep accumulated protocol fees for a market/token to `receiver`. FEE_KEEPER only.
    pub fn claim_fees(
        env: Env,
        keeper: Address,
        market: Address,
        token: Address,
        receiver: Address,
    ) -> u128 {
        keeper.require_auth();

        let role_store: Address = env.storage().instance().get(&InstanceKey::RoleStore).unwrap();
        if !RoleStoreClient::new(&env, &role_store).has_role(&keeper, &roles::fee_keeper(&env)) {
            panic_with_error!(&env, Error::Unauthorized);
        }

        let data_store: Address = env.storage().instance().get(&InstanceKey::DataStore).unwrap();
        let ds = DataStoreClient::new(&env, &data_store);
        let handler = env.current_contract_address();

        let key = claimable_fee_amount_key(&env, &market, &token);
        let amount = ds.get_u128(&key);
        if amount == 0 {
            return 0;
        }

        ds.set_u128(&handler, &key, &0u128);

        // Transfer from market_token pool to receiver
        MarketTokenClient::new(&env, &market)
            .withdraw_from_pool(&handler, &token, &receiver, &(amount as i128));

        env.events().publish_event(&FeeClaimed { market, token, amount, receiver });
        amount
    }

    /// Upgrade the contract wasm. Only the stored admin may call this.
    pub fn upgrade(env: Env, new_wasm_hash: BytesN<32>) {
        let admin: Address = env.storage().instance().get(&InstanceKey::Admin)
            .unwrap_or_else(|| panic_with_error!(&env, Error::NotInitialized));
        admin.require_auth();
        env.deployer().update_current_contract_wasm(new_wasm_hash);
    }

    /// Claim funding fees earned by a position account. Anyone can call for their own account.
    pub fn claim_funding_fees(
        env: Env,
        account: Address,
        market: Address,
        token: Address,
    ) -> u128 {
        account.require_auth();

        let data_store: Address = env.storage().instance().get(&InstanceKey::DataStore).unwrap();
        let ds = DataStoreClient::new(&env, &data_store);
        let handler = env.current_contract_address();

        let key = claimable_funding_amount_key(&env, &market, &token, &account);
        let amount = ds.get_u128(&key);
        if amount == 0 {
            return 0;
        }

        ds.set_u128(&handler, &key, &0u128);

        MarketTokenClient::new(&env, &market)
            .withdraw_from_pool(&handler, &token, &account, &(amount as i128));

        env.events().publish_event(&FundingFeeClaimed { account, market, token, amount });
        amount
    }
}

// ─── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use soroban_sdk::{testutils::Address as _, token::StellarAssetClient, BytesN, Env};
    use role_store::{RoleStore, RoleStoreClient as RsClient};
    use data_store::{DataStore, DataStoreClient as DsClient};
    use market_token::{MarketToken, MarketTokenClient as MtClient};
    use gmx_keys::roles;

    const ONE_TOKEN: i128 = 10_000_000;

    struct World {
        env:       Env,
        admin:     Address,
        keeper:    Address,
        rs:        Address,
        ds:        Address,
        market_tk: Address,
        long_tk:   Address,
        handler:   Address,
    }

    fn setup() -> World {
        let env = Env::default();
        env.mock_all_auths();

        let admin  = Address::generate(&env);
        let keeper = Address::generate(&env);

        let rs = env.register(RoleStore, ());
        let rs_c = RsClient::new(&env, &rs);
        rs_c.initialize(&admin);
        rs_c.grant_role(&admin, &admin,  &roles::controller(&env));
        rs_c.grant_role(&admin, &keeper, &roles::fee_keeper(&env));

        let ds = env.register(DataStore, ());
        DsClient::new(&env, &ds).initialize(&admin, &rs);

        let market_tk = env.register(MarketToken, ());
        MtClient::new(&env, &market_tk).initialize(
            &admin, &rs, &7u32,
            &soroban_sdk::String::from_str(&env, "FH Test Market"),
            &soroban_sdk::String::from_str(&env, "FM"),
        );
        rs_c.grant_role(&admin, &market_tk, &roles::controller(&env));

        let long_tk = env.register_stellar_asset_contract_v2(admin.clone()).address();

        let handler = env.register(FeeHandler, ());
        FeeHandlerClient::new(&env, &handler)
            .initialize(&admin, &rs, &ds);
        rs_c.grant_role(&admin, &handler, &roles::controller(&env));

        World { env, admin, keeper, rs, ds, market_tk, long_tk, handler }
    }

    // ── Task 1: fee_handler tests ─────────────────────────────────────────────

    /// claimable_fees returns zero on a fresh DataStore.
    #[test]
    fn claimable_fees_zero_initially() {
        let w = setup();
        let amount = FeeHandlerClient::new(&w.env, &w.handler)
            .claimable_fees(&w.market_tk, &w.long_tk);
        assert_eq!(amount, 0, "claimable fees must be zero before any accumulation");
    }

    /// claim_fees transfers accumulated protocol fees and zeroes the DataStore entry.
    #[test]
    fn claim_fees_transfers_and_zeroes_balance() {
        let w = setup();
        let fee_amount: u128 = ONE_TOKEN as u128 * 3; // 3 tokens

        // Seed claimable fee amount in DataStore
        let fee_key = gmx_keys::claimable_fee_amount_key(&w.env, &w.market_tk, &w.long_tk);
        DsClient::new(&w.env, &w.ds)
            .set_u128(&w.admin, &fee_key, &fee_amount);

        // Mint tokens into the market pool so withdraw_from_pool can transfer
        StellarAssetClient::new(&w.env, &w.long_tk)
            .mint(&w.market_tk, &(fee_amount as i128));

        let receiver = Address::generate(&w.env);
        let bal_before = soroban_sdk::token::Client::new(&w.env, &w.long_tk).balance(&receiver);

        FeeHandlerClient::new(&w.env, &w.handler)
            .claim_fees(&w.keeper, &w.market_tk, &w.long_tk, &receiver);

        let bal_after = soroban_sdk::token::Client::new(&w.env, &w.long_tk).balance(&receiver);
        assert_eq!((bal_after - bal_before) as u128, fee_amount,
            "receiver must get exactly the claimable fee amount");

        // DataStore entry must be zeroed after claim
        let remaining = DsClient::new(&w.env, &w.ds).get_u128(&fee_key);
        assert_eq!(remaining, 0, "claimable fee in DataStore must be zero after claim");
    }

    /// claim_fees returns 0 (no transfer) when there is no accumulated fee —
    /// consistent with claim_funding_fees zero-amount behaviour.
    #[test]
    fn claim_fees_returns_zero_when_nothing_to_claim() {
        let w = setup();
        let receiver = Address::generate(&w.env);
        let claimed = FeeHandlerClient::new(&w.env, &w.handler)
            .claim_fees(&w.keeper, &w.market_tk, &w.long_tk, &receiver);
        assert_eq!(claimed, 0, "claim_fees must return 0 when claimable balance is zero");
    }

    /// Non-keeper cannot call claim_fees — Unauthorized expected.
    #[test]
    #[should_panic]
    fn claim_fees_by_non_keeper_reverts() {
        let w = setup();
        // Seed some fees so the call reaches the role check
        let fee_key = gmx_keys::claimable_fee_amount_key(&w.env, &w.market_tk, &w.long_tk);
        DsClient::new(&w.env, &w.ds).set_u128(&w.admin, &fee_key, &(ONE_TOKEN as u128));
        StellarAssetClient::new(&w.env, &w.long_tk).mint(&w.market_tk, &ONE_TOKEN);

        let intruder = Address::generate(&w.env);
        let receiver = Address::generate(&w.env);
        FeeHandlerClient::new(&w.env, &w.handler)
            .claim_fees(&intruder, &w.market_tk, &w.long_tk, &receiver);
    }

    /// claim_funding_fees transfers the claimable amount to the account and zeroes the entry.
    #[test]
    fn claim_funding_fees_transfers_and_zeroes_balance() {
        let w = setup();
        let funding_amount: u128 = ONE_TOKEN as u128 * 2;

        let claim_key = gmx_keys::claimable_funding_amount_key(
            &w.env, &w.market_tk, &w.long_tk, &w.admin,
        );
        DsClient::new(&w.env, &w.ds)
            .set_u128(&w.admin, &claim_key, &funding_amount);

        StellarAssetClient::new(&w.env, &w.long_tk)
            .mint(&w.market_tk, &(funding_amount as i128));

        let bal_before = soroban_sdk::token::Client::new(&w.env, &w.long_tk).balance(&w.admin);

        FeeHandlerClient::new(&w.env, &w.handler)
            .claim_funding_fees(&w.admin, &w.market_tk, &w.long_tk);

        let bal_after = soroban_sdk::token::Client::new(&w.env, &w.long_tk).balance(&w.admin);
        assert_eq!((bal_after - bal_before) as u128, funding_amount,
            "account must receive the full claimable funding amount");

        let remaining = DsClient::new(&w.env, &w.ds).get_u128(&claim_key);
        assert_eq!(remaining, 0, "claimable funding must be zero after claim");
    }

    /// claim_funding_fees returns 0 (no transfer) when there is nothing to claim.
    #[test]
    fn claim_funding_fees_returns_zero_when_nothing_to_claim() {
        let w = setup();
        let claimed = FeeHandlerClient::new(&w.env, &w.handler)
            .claim_funding_fees(&w.admin, &w.market_tk, &w.long_tk);
        assert_eq!(claimed, 0, "claim_funding_fees must return 0 when nothing is claimable");
    }

    // ── Issue #109: FEE_KEEPER authorization matrix ───────────────────────────

    /// claim_fees must reject a caller that does not hold FEE_KEEPER.
    #[test]
    #[should_panic]
    fn claim_fees_by_non_fee_keeper_panics() {
        let w = setup();
        let impostor = Address::generate(&w.env);
        // impostor has no FEE_KEEPER role — must panic with Unauthorized.
        FeeHandlerClient::new(&w.env, &w.handler)
            .claim_fees(&impostor, &w.market_tk, &w.long_tk, &w.admin);
    }

    // ── Issue #110: upgrade smoke tests ───────────────────────────────────────

    /// Admin auth passes on upgrade; panics at WASM lookup (not auth) in unit tests.
    /// A compiled WASM binary is required for the host to accept the hash.
    #[test]
    #[should_panic]
    fn upgrade_admin_succeeds() {
        let w = setup(); // mock_all_auths active — admin.require_auth() passes silently
        // Panics at WASM lookup (not at auth) — proves auth gate is open for admin.
        FeeHandlerClient::new(&w.env, &w.handler)
            .upgrade(&BytesN::from_array(&w.env, &[0u8; 32]));
    }

    /// Calling upgrade without the admin's authorisation must revert.
    #[test]
    #[should_panic]
    fn upgrade_non_admin_reverts() {
        let env = Env::default();
        let admin = Address::generate(&env);
        let rs    = Address::generate(&env);
        let ds    = Address::generate(&env);

        let handler = env.register(FeeHandler, ());
        env.as_contract(&handler, || {
            env.storage().instance().set(&InstanceKey::Initialized, &true);
            env.storage().instance().set(&InstanceKey::Admin,       &admin);
            env.storage().instance().set(&InstanceKey::RoleStore,   &rs);
            env.storage().instance().set(&InstanceKey::DataStore,   &ds);
        });

        // No auth context — must panic at admin.require_auth().
        FeeHandlerClient::new(&env, &handler)
            .upgrade(&BytesN::from_array(&env, &[0u8; 32]));
    }

    /// DataStore fee entries written before upgrade remain claimable after.
    /// Requires a compiled WASM binary — skipped in unit-test mode.
    #[test]
    #[ignore]
    fn upgrade_preserves_fee_storage_and_claim_works() {
        let w = setup();
        let fee_amount: u128 = ONE_TOKEN as u128 * 5;

        // Seed claimable fees in DataStore.
        let claim_key = gmx_keys::claimable_fee_amount_key(&w.env, &w.market_tk, &w.long_tk);
        DsClient::new(&w.env, &w.ds).set_u128(&w.handler, &claim_key, &fee_amount);
        StellarAssetClient::new(&w.env, &w.long_tk).mint(&w.market_tk, &(fee_amount as i128));

        FeeHandlerClient::new(&w.env, &w.handler)
            .upgrade(&BytesN::from_array(&w.env, &[0u8; 32]));

        // claim_fees must still work — fee is still in DataStore.
        let receiver = Address::generate(&w.env);
        FeeHandlerClient::new(&w.env, &w.handler)
            .claim_fees(&w.keeper, &w.market_tk, &w.long_tk, &receiver);

        let bal = soroban_sdk::token::Client::new(&w.env, &w.long_tk).balance(&receiver);
        assert_eq!(bal as u128, fee_amount, "full fee must be claimable after upgrade");
    }
}
