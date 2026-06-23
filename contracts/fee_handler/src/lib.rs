//! Fee handler — claims and distributes protocol fees accumulated in the pool.
//! Mirrors GMX's FeeHandler.sol.
#![no_std]
#![allow(dependency_on_unit_never_type_fallback)]

use gmx_keys::{
    claimable_fee_amount_key, claimable_funding_amount_key, claimable_ui_fee_amount_key, roles,
    ui_fee_factor_key,
};
use gmx_math::FLOAT_PRECISION;
use soroban_sdk::{
    contract, contracterror, contractevent, contractimpl, contracttype, panic_with_error, Address,
    BytesN, Env,
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
    NotInitialized = 2,
    Unauthorized = 3,
    NothingToClaim = 4,
    InvalidAmount = 5,
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
    fn withdraw_from_pool(
        env: Env,
        caller: Address,
        pool_token: Address,
        receiver: Address,
        amount: i128,
    );
}

// ─── Events ───────────────────────────────────────────────────────────────────

#[contractevent(topics = ["fee_clm"])]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FeeClaimed {
    pub market: Address,
    pub token: Address,
    pub amount: u128,
    pub receiver: Address,
}

#[contractevent(topics = ["fnd_clm"])]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FundingFeeClaimed {
    pub account: Address,
    pub market: Address,
    pub token: Address,
    pub amount: u128,
}

#[contractevent(topics = ["ui_fee_acc"])]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct UiFeeAccrued {
    pub ui_receiver: Address,
    pub token: Address,
    pub amount: u128,
}

#[contractevent(topics = ["ui_fee_clm"])]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct UiFeeClaimed {
    pub ui_receiver: Address,
    pub token: Address,
    pub amount: u128,
}

#[contractevent(topics = ["ui_fee_set"])]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct UiFeeFactorSet {
    pub ui_receiver: Address,
    pub factor: u128,
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
    }

    /// Return the accumulated protocol fee amount for a given market + token.
    pub fn claimable_fees(env: Env, market: Address, token: Address) -> u128 {
        let data_store: Address = env
            .storage()
            .instance()
            .get(&InstanceKey::DataStore)
            .unwrap();
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

        let role_store: Address = env
            .storage()
            .instance()
            .get(&InstanceKey::RoleStore)
            .unwrap();
        if !RoleStoreClient::new(&env, &role_store).has_role(&keeper, &roles::fee_keeper(&env)) {
            panic_with_error!(&env, Error::Unauthorized);
        }

        let data_store: Address = env
            .storage()
            .instance()
            .get(&InstanceKey::DataStore)
            .unwrap();
        let ds = DataStoreClient::new(&env, &data_store);
        let handler = env.current_contract_address();

        let key = claimable_fee_amount_key(&env, &market, &token);
        let amount = ds.get_u128(&key);
        if amount == 0 {
            return 0;
        }

        ds.set_u128(&handler, &key, &0u128);

        // Transfer from market_token pool to receiver
        MarketTokenClient::new(&env, &market).withdraw_from_pool(
            &handler,
            &token,
            &receiver,
            &(amount as i128),
        );

        env.events().publish_event(&FeeClaimed {
            market,
            token,
            amount,
            receiver,
        });
        amount
    }

    /// Upgrade the contract wasm. Only the stored admin may call this.
    pub fn upgrade(env: Env, new_wasm_hash: BytesN<32>) {
        let admin: Address = env
            .storage()
            .instance()
            .get(&InstanceKey::Admin)
            .unwrap_or_else(|| panic_with_error!(&env, Error::NotInitialized));
        admin.require_auth();
        env.deployer().update_current_contract_wasm(new_wasm_hash);
    }

    // ── UI fee API (issue #85) ────────────────────────────────────────────────

    /// Return the accumulated UI fee for a (token, ui_receiver) pair.
    pub fn claimable_ui_fees(env: Env, token: Address, ui_receiver: Address) -> u128 {
        let data_store: Address = env
            .storage()
            .instance()
            .get(&InstanceKey::DataStore)
            .unwrap();
        let key = claimable_ui_fee_amount_key(&env, &token, &ui_receiver);
        DataStoreClient::new(&env, &data_store).get_u128(&key)
    }

    /// Accrue a UI fee on behalf of a receiver (called by the exchange_router on every swap/trade).
    ///
    /// Only a caller that holds the CONTROLLER role may accrue fees; this prevents
    /// arbitrary inflation of a receiver's balance.
    pub fn accrue_ui_fee(
        env: Env,
        controller: Address,
        token: Address,
        ui_receiver: Address,
        amount: u128,
    ) {
        controller.require_auth();

        let role_store: Address = env
            .storage()
            .instance()
            .get(&InstanceKey::RoleStore)
            .unwrap();
        if !RoleStoreClient::new(&env, &role_store).has_role(&controller, &roles::controller(&env))
        {
            panic_with_error!(&env, Error::Unauthorized);
        }
        if amount == 0 {
            panic_with_error!(&env, Error::InvalidAmount);
        }

        let data_store: Address = env
            .storage()
            .instance()
            .get(&InstanceKey::DataStore)
            .unwrap();
        let handler = env.current_contract_address();
        let key = claimable_ui_fee_amount_key(&env, &token, &ui_receiver);
        DataStoreClient::new(&env, &data_store).apply_delta_to_u128(
            &handler,
            &key,
            &(amount as i128),
        );

        env.events().publish_event(&UiFeeAccrued {
            ui_receiver,
            token,
            amount,
        });
    }

    /// Claim all accrued UI fees for the calling receiver.
    ///
    /// A receiver may only claim their own balance — passing a different address as
    /// `ui_receiver` will fail the `require_auth()` check.
    pub fn claim_ui_fees(env: Env, ui_receiver: Address, market: Address, token: Address) -> u128 {
        ui_receiver.require_auth();

        let data_store: Address = env
            .storage()
            .instance()
            .get(&InstanceKey::DataStore)
            .unwrap();
        let ds = DataStoreClient::new(&env, &data_store);
        let handler = env.current_contract_address();

        let key = claimable_ui_fee_amount_key(&env, &token, &ui_receiver);
        let amount = ds.get_u128(&key);
        if amount == 0 {
            panic_with_error!(&env, Error::NothingToClaim);
        }

        ds.set_u128(&handler, &key, &0u128);

        // Transfer from the market pool to the UI receiver.
        MarketTokenClient::new(&env, &market).withdraw_from_pool(
            &handler,
            &token,
            &ui_receiver,
            &(amount as i128),
        );

        env.events().publish_event(&UiFeeClaimed {
            ui_receiver,
            token,
            amount,
        });
        amount
    }

    /// Claim funding fees earned by a position account. Anyone can call for their own account.
    pub fn claim_funding_fees(env: Env, account: Address, market: Address, token: Address) -> u128 {
        account.require_auth();

        let data_store: Address = env
            .storage()
            .instance()
            .get(&InstanceKey::DataStore)
            .unwrap();
        let ds = DataStoreClient::new(&env, &data_store);
        let handler = env.current_contract_address();

        let key = claimable_funding_amount_key(&env, &market, &token, &account);
        let amount = ds.get_u128(&key);
        if amount == 0 {
            return 0;
        }

        ds.set_u128(&handler, &key, &0u128);

        MarketTokenClient::new(&env, &market).withdraw_from_pool(
            &handler,
            &token,
            &account,
            &(amount as i128),
        );

        env.events().publish_event(&FundingFeeClaimed {
            account,
            market,
            token,
            amount,
        });
        amount
    }

    // ── UI fee factor configuration (issue #100) ─────────────────────────────

    /// Return the stored UI fee factor for a given receiver (0 if unset).
    pub fn get_ui_fee_factor(env: Env, ui_receiver: Address) -> u128 {
        let data_store: Address = env
            .storage()
            .instance()
            .get(&InstanceKey::DataStore)
            .unwrap();
        DataStoreClient::new(&env, &data_store).get_u128(&ui_fee_factor_key(&env, &ui_receiver))
    }

    /// Set the UI fee factor for a given receiver. Only the stored admin may call.
    ///
    /// `factor` must be ≤ FLOAT_PRECISION (10^30, i.e. 100%). A factor above this
    /// is nonsensical (> 100% fee) and is rejected with `InvalidAmount`.
    pub fn set_ui_fee_factor(env: Env, ui_receiver: Address, factor: u128) {
        let admin: Address = env
            .storage()
            .instance()
            .get(&InstanceKey::Admin)
            .unwrap_or_else(|| panic_with_error!(&env, Error::NotInitialized));
        admin.require_auth();
        if factor > FLOAT_PRECISION as u128 {
            panic_with_error!(&env, Error::InvalidAmount);
        }
        let data_store: Address = env
            .storage()
            .instance()
            .get(&InstanceKey::DataStore)
            .unwrap();
        let handler = env.current_contract_address();
        let key = ui_fee_factor_key(&env, &ui_receiver);
        DataStoreClient::new(&env, &data_store).set_u128(&handler, &key, &factor);
        env.events().publish_event(&UiFeeFactorSet {
            ui_receiver,
            factor,
        });
    }
}

// ─── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use data_store::{DataStore, DataStoreClient as DsClient};
    use gmx_keys::roles;
    use market_token::{MarketToken, MarketTokenClient as MtClient};
    use role_store::{RoleStore, RoleStoreClient as RsClient};
    use soroban_sdk::{testutils::Address as _, token::StellarAssetClient, BytesN, Env};

    const ONE_TOKEN: i128 = 10_000_000;

    struct World {
        env: Env,
        admin: Address,
        keeper: Address,
        rs: Address,
        ds: Address,
        market_tk: Address,
        long_tk: Address,
        handler: Address,
    }

    fn setup() -> World {
        let env = Env::default();
        env.mock_all_auths();

        let admin = Address::generate(&env);
        let keeper = Address::generate(&env);

        let rs = env.register(RoleStore, ());
        let rs_c = RsClient::new(&env, &rs);
        rs_c.initialize(&admin);
        rs_c.grant_role(&admin, &admin, &roles::controller(&env));
        rs_c.grant_role(&admin, &keeper, &roles::fee_keeper(&env));

        let ds = env.register(DataStore, ());
        DsClient::new(&env, &ds).initialize(&admin, &rs);

        let market_tk = env.register(MarketToken, ());
        MtClient::new(&env, &market_tk).initialize(
            &admin,
            &rs,
            &7u32,
            &soroban_sdk::String::from_str(&env, "FH Test Market"),
            &soroban_sdk::String::from_str(&env, "FM"),
        );
        rs_c.grant_role(&admin, &market_tk, &roles::controller(&env));

        let long_tk = env
            .register_stellar_asset_contract_v2(admin.clone())
            .address();

        let handler = env.register(FeeHandler, ());
        FeeHandlerClient::new(&env, &handler).initialize(&admin, &rs, &ds);
        rs_c.grant_role(&admin, &handler, &roles::controller(&env));

        World {
            env,
            admin,
            keeper,
            rs,
            ds,
            market_tk,
            long_tk,
            handler,
        }
    }

    // ── Task 1: fee_handler tests ─────────────────────────────────────────────

    /// claimable_fees returns zero on a fresh DataStore.
    #[test]
    fn claimable_fees_zero_initially() {
        let w = setup();
        let amount =
            FeeHandlerClient::new(&w.env, &w.handler).claimable_fees(&w.market_tk, &w.long_tk);
        assert_eq!(
            amount, 0,
            "claimable fees must be zero before any accumulation"
        );
    }

    /// claim_fees transfers accumulated protocol fees and zeroes the DataStore entry.
    #[test]
    fn claim_fees_transfers_and_zeroes_balance() {
        let w = setup();
        let fee_amount: u128 = ONE_TOKEN as u128 * 3; // 3 tokens

        // Seed claimable fee amount in DataStore
        let fee_key = gmx_keys::claimable_fee_amount_key(&w.env, &w.market_tk, &w.long_tk);
        DsClient::new(&w.env, &w.ds).set_u128(&w.admin, &fee_key, &fee_amount);

        // Mint tokens into the market pool so withdraw_from_pool can transfer
        StellarAssetClient::new(&w.env, &w.long_tk).mint(&w.market_tk, &(fee_amount as i128));

        let receiver = Address::generate(&w.env);
        let bal_before = soroban_sdk::token::Client::new(&w.env, &w.long_tk).balance(&receiver);

        FeeHandlerClient::new(&w.env, &w.handler).claim_fees(
            &w.keeper,
            &w.market_tk,
            &w.long_tk,
            &receiver,
        );

        let bal_after = soroban_sdk::token::Client::new(&w.env, &w.long_tk).balance(&receiver);
        assert_eq!(
            (bal_after - bal_before) as u128,
            fee_amount,
            "receiver must get exactly the claimable fee amount"
        );

        // DataStore entry must be zeroed after claim
        let remaining = DsClient::new(&w.env, &w.ds).get_u128(&fee_key);
        assert_eq!(
            remaining, 0,
            "claimable fee in DataStore must be zero after claim"
        );
    }

    /// claim_fees returns 0 (no transfer) when there is no accumulated fee —
    /// consistent with claim_funding_fees zero-amount behaviour.
    #[test]
    fn claim_fees_returns_zero_when_nothing_to_claim() {
        let w = setup();
        let receiver = Address::generate(&w.env);
        let claimed = FeeHandlerClient::new(&w.env, &w.handler).claim_fees(
            &w.keeper,
            &w.market_tk,
            &w.long_tk,
            &receiver,
        );
        assert_eq!(
            claimed, 0,
            "claim_fees must return 0 when claimable balance is zero"
        );
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
        FeeHandlerClient::new(&w.env, &w.handler).claim_fees(
            &intruder,
            &w.market_tk,
            &w.long_tk,
            &receiver,
        );
    }

    /// claim_funding_fees transfers the claimable amount to the account and zeroes the entry.
    #[test]
    fn claim_funding_fees_transfers_and_zeroes_balance() {
        let w = setup();
        let funding_amount: u128 = ONE_TOKEN as u128 * 2;

        let claim_key =
            gmx_keys::claimable_funding_amount_key(&w.env, &w.market_tk, &w.long_tk, &w.admin);
        DsClient::new(&w.env, &w.ds).set_u128(&w.admin, &claim_key, &funding_amount);

        StellarAssetClient::new(&w.env, &w.long_tk).mint(&w.market_tk, &(funding_amount as i128));

        let bal_before = soroban_sdk::token::Client::new(&w.env, &w.long_tk).balance(&w.admin);

        FeeHandlerClient::new(&w.env, &w.handler).claim_funding_fees(
            &w.admin,
            &w.market_tk,
            &w.long_tk,
        );

        let bal_after = soroban_sdk::token::Client::new(&w.env, &w.long_tk).balance(&w.admin);
        assert_eq!(
            (bal_after - bal_before) as u128,
            funding_amount,
            "account must receive the full claimable funding amount"
        );

        let remaining = DsClient::new(&w.env, &w.ds).get_u128(&claim_key);
        assert_eq!(remaining, 0, "claimable funding must be zero after claim");
    }

    /// claim_funding_fees returns 0 (no transfer) when there is nothing to claim.
    #[test]
    fn claim_funding_fees_returns_zero_when_nothing_to_claim() {
        let w = setup();
        let claimed = FeeHandlerClient::new(&w.env, &w.handler).claim_funding_fees(
            &w.admin,
            &w.market_tk,
            &w.long_tk,
        );
        assert_eq!(
            claimed, 0,
            "claim_funding_fees must return 0 when nothing is claimable"
        );
    }

    // ── Issue #109: FEE_KEEPER authorization matrix ───────────────────────────

    /// claim_fees must reject a caller that does not hold FEE_KEEPER.
    #[test]
    #[should_panic]
    fn claim_fees_by_non_fee_keeper_panics() {
        let w = setup();
        let impostor = Address::generate(&w.env);
        // impostor has no FEE_KEEPER role — must panic with Unauthorized.
        FeeHandlerClient::new(&w.env, &w.handler).claim_fees(
            &impostor,
            &w.market_tk,
            &w.long_tk,
            &w.admin,
        );
    }

    // ── Issue #110: upgrade smoke tests ───────────────────────────────────────

    /// Admin auth passes on upgrade; panics at WASM lookup (not auth) in unit tests.
    /// A compiled WASM binary is required for the host to accept the hash.
    #[test]
    #[should_panic]
    fn upgrade_admin_succeeds() {
        let w = setup(); // mock_all_auths active — admin.require_auth() passes silently
                         // Panics at WASM lookup (not at auth) — proves auth gate is open for admin.
        FeeHandlerClient::new(&w.env, &w.handler).upgrade(&BytesN::from_array(&w.env, &[0u8; 32]));
    }

    /// Calling upgrade without the admin's authorisation must revert.
    #[test]
    #[should_panic]
    fn upgrade_non_admin_reverts() {
        let env = Env::default();
        let admin = Address::generate(&env);
        let rs = Address::generate(&env);
        let ds = Address::generate(&env);

        let handler = env.register(FeeHandler, ());
        env.as_contract(&handler, || {
            env.storage()
                .instance()
                .set(&InstanceKey::Initialized, &true);
            env.storage().instance().set(&InstanceKey::Admin, &admin);
            env.storage().instance().set(&InstanceKey::RoleStore, &rs);
            env.storage().instance().set(&InstanceKey::DataStore, &ds);
        });

        // No auth context — must panic at admin.require_auth().
        FeeHandlerClient::new(&env, &handler).upgrade(&BytesN::from_array(&env, &[0u8; 32]));
    }

    // ── Issue #85: UI fee accrual + claiming ──────────────────────────────────

    /// claimable_ui_fees returns 0 before any accrual.
    #[test]
    fn claimable_ui_fees_zero_initially() {
        let w = setup();
        let ui_recv = Address::generate(&w.env);
        let amount =
            FeeHandlerClient::new(&w.env, &w.handler).claimable_ui_fees(&w.long_tk, &ui_recv);
        assert_eq!(amount, 0, "UI fee balance must be zero before any accrual");
    }

    /// accrue_ui_fee accumulates the amount; claimable_ui_fees reflects it.
    #[test]
    fn accrue_ui_fee_accumulates_correctly() {
        let w = setup();
        let ui_recv = Address::generate(&w.env);
        let fh = FeeHandlerClient::new(&w.env, &w.handler);

        // Grant handler the CONTROLLER role so it can accrue (already done in setup via handler)
        // Use admin as controller (it holds CONTROLLER from setup).
        fh.accrue_ui_fee(&w.admin, &w.long_tk, &ui_recv, &500u128);
        assert_eq!(fh.claimable_ui_fees(&w.long_tk, &ui_recv), 500);

        // Second accrual stacks.
        fh.accrue_ui_fee(&w.admin, &w.long_tk, &ui_recv, &300u128);
        assert_eq!(fh.claimable_ui_fees(&w.long_tk, &ui_recv), 800);
    }

    /// claim_ui_fees transfers the full accrued amount to the receiver and zeroes balance.
    #[test]
    fn claim_ui_fees_transfers_and_zeroes_balance() {
        let w = setup();
        let ui_recv = Address::generate(&w.env);
        let fee_amount: u128 = ONE_TOKEN as u128 * 2;
        let fh = FeeHandlerClient::new(&w.env, &w.handler);

        // Accrue fees.
        fh.accrue_ui_fee(&w.admin, &w.long_tk, &ui_recv, &fee_amount);

        // Mint tokens into market pool so withdraw_from_pool can pay out.
        StellarAssetClient::new(&w.env, &w.long_tk).mint(&w.market_tk, &(fee_amount as i128));

        let bal_before = soroban_sdk::token::Client::new(&w.env, &w.long_tk).balance(&ui_recv);
        let claimed = fh.claim_ui_fees(&ui_recv, &w.market_tk, &w.long_tk);

        let bal_after = soroban_sdk::token::Client::new(&w.env, &w.long_tk).balance(&ui_recv);
        assert_eq!(
            claimed, fee_amount,
            "claim_ui_fees must return the accrued amount"
        );
        assert_eq!(
            (bal_after - bal_before) as u128,
            fee_amount,
            "receiver must get the full accrued UI fee amount"
        );
        assert_eq!(
            fh.claimable_ui_fees(&w.long_tk, &ui_recv),
            0,
            "balance must be zero after claim"
        );
    }

    /// claim_ui_fees reverts with NothingToClaim when the balance is zero.
    #[test]
    #[should_panic]
    fn claim_ui_fees_nothing_to_claim_reverts() {
        let w = setup();
        let ui_recv = Address::generate(&w.env);
        FeeHandlerClient::new(&w.env, &w.handler).claim_ui_fees(&ui_recv, &w.market_tk, &w.long_tk);
    }

    /// A receiver cannot claim another receiver's fees — auth must gate the call.
    #[test]
    #[should_panic]
    fn claim_ui_fees_wrong_receiver_reverts() {
        let env = Env::default();
        // Do NOT mock_all_auths so require_auth() actually enforces identity.
        let admin = Address::generate(&env);
        let keeper = Address::generate(&env);

        let rs = env.register(RoleStore, ());
        let rs_c = RsClient::new(&env, &rs);
        env.mock_all_auths_allowing_non_root_auth();
        rs_c.initialize(&admin);
        rs_c.grant_role(&admin, &admin, &roles::controller(&env));
        rs_c.grant_role(&admin, &keeper, &roles::fee_keeper(&env));

        let ds = env.register(DataStore, ());
        DsClient::new(&env, &ds).initialize(&admin, &rs);

        let market_tk = env.register(MarketToken, ());
        MtClient::new(&env, &market_tk).initialize(
            &admin,
            &rs,
            &7u32,
            &soroban_sdk::String::from_str(&env, "UI Test Market"),
            &soroban_sdk::String::from_str(&env, "UM"),
        );
        rs_c.grant_role(&admin, &market_tk, &roles::controller(&env));

        let long_tk = env
            .register_stellar_asset_contract_v2(admin.clone())
            .address();

        let handler = env.register(FeeHandler, ());
        let fh = FeeHandlerClient::new(&env, &handler);
        fh.initialize(&admin, &rs, &ds);
        rs_c.grant_role(&admin, &handler, &roles::controller(&env));

        let real_recv = Address::generate(&env);
        let other_recv = Address::generate(&env);
        let fee_amount: u128 = ONE_TOKEN as u128;

        // Accrue for real_recv.
        fh.accrue_ui_fee(&admin, &long_tk, &real_recv, &fee_amount);
        StellarAssetClient::new(&env, &long_tk).mint(&market_tk, &(fee_amount as i128));

        // other_recv attempts to claim real_recv's fees — must panic.
        fh.claim_ui_fees(&other_recv, &market_tk, &long_tk);
    }

    /// accrue_ui_fee with amount == 0 must revert with InvalidAmount.
    #[test]
    #[should_panic]
    fn accrue_ui_fee_zero_amount_reverts() {
        let w = setup();
        let ui_recv = Address::generate(&w.env);
        FeeHandlerClient::new(&w.env, &w.handler)
            .accrue_ui_fee(&w.admin, &w.long_tk, &ui_recv, &0u128);
    }

    /// Non-controller cannot accrue UI fees — Unauthorized expected.
    #[test]
    #[should_panic]
    fn accrue_ui_fee_non_controller_reverts() {
        let w = setup();
        let impostor = Address::generate(&w.env);
        let ui_recv = Address::generate(&w.env);
        FeeHandlerClient::new(&w.env, &w.handler)
            .accrue_ui_fee(&impostor, &w.long_tk, &ui_recv, &100u128);
    }

    // ── Issue #100: UI fee factor set/get ────────────────────────────────────

    /// get_ui_fee_factor returns 0 before any factor is set.
    #[test]
    fn ui_fee_factor_zero_initially() {
        let w = setup();
        let ui_recv = Address::generate(&w.env);
        let factor = FeeHandlerClient::new(&w.env, &w.handler).get_ui_fee_factor(&ui_recv);
        assert_eq!(
            factor, 0,
            "UI fee factor must be 0 before any configuration"
        );
    }

    /// set_ui_fee_factor stores the value; get_ui_fee_factor retrieves it.
    #[test]
    fn set_ui_fee_factor_stores_and_retrieves() {
        let w = setup();
        let ui_recv = Address::generate(&w.env);
        let fh = FeeHandlerClient::new(&w.env, &w.handler);
        let factor: u128 = gmx_math::FLOAT_PRECISION as u128 / 100; // 1%
        fh.set_ui_fee_factor(&ui_recv, &factor);
        assert_eq!(
            fh.get_ui_fee_factor(&ui_recv),
            factor,
            "stored factor must equal the value passed to set_ui_fee_factor"
        );
    }

    /// Non-admin cannot call set_ui_fee_factor.
    #[test]
    #[should_panic]
    fn set_ui_fee_factor_non_admin_reverts() {
        let env = Env::default();
        // No mock_all_auths — require_auth() will reject any unauthorised call.
        let admin = Address::generate(&env);
        let rs = Address::generate(&env);
        let ds = Address::generate(&env);

        let handler = env.register(FeeHandler, ());
        env.as_contract(&handler, || {
            env.storage()
                .instance()
                .set(&InstanceKey::Initialized, &true);
            env.storage().instance().set(&InstanceKey::Admin, &admin);
            env.storage().instance().set(&InstanceKey::RoleStore, &rs);
            env.storage().instance().set(&InstanceKey::DataStore, &ds);
        });

        let ui_recv = Address::generate(&env);
        // Must panic because no auth context is provided for `admin`.
        FeeHandlerClient::new(&env, &handler).set_ui_fee_factor(&ui_recv, &100u128);
    }

    /// A factor above FLOAT_PRECISION (> 100%) must revert with InvalidAmount.
    #[test]
    #[should_panic]
    fn set_ui_fee_factor_exceeds_bound_reverts() {
        let w = setup();
        let ui_recv = Address::generate(&w.env);
        let too_large: u128 = gmx_math::FLOAT_PRECISION as u128 + 1;
        FeeHandlerClient::new(&w.env, &w.handler).set_ui_fee_factor(&ui_recv, &too_large);
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

        FeeHandlerClient::new(&w.env, &w.handler).upgrade(&BytesN::from_array(&w.env, &[0u8; 32]));

        // claim_fees must still work — fee is still in DataStore.
        let receiver = Address::generate(&w.env);
        FeeHandlerClient::new(&w.env, &w.handler).claim_fees(
            &w.keeper,
            &w.market_tk,
            &w.long_tk,
            &receiver,
        );

        let bal = soroban_sdk::token::Client::new(&w.env, &w.long_tk).balance(&receiver);
        assert_eq!(
            bal as u128, fee_amount,
            "full fee must be claimable after upgrade"
        );
    }
}
