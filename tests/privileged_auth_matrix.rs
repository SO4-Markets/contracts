//! Privileged-entrypoint authorization matrix (issues #1048 and #1049).
//!
//! Caller categories exercised against every privileged entrypoint:
//!   admin (contract admin and ROLE_ADMIN), trader, stranger, and holders of an
//!   unrelated role (fee / order / liquidation / ADL keeper, CONTROLLER).
//!
//! Part 1 (#1048): each role-gated or admin-gated entrypoint rejects unauthorised
//! callers, and a rejected call leaves the guarded state unchanged.
//! Part 2 (#1049): role rotation and revocation take effect immediately, no role
//! grants unrelated authority, and already-initialised contracts cannot be re-pointed
//! at a user-supplied role_store / data_store / oracle.
//!
//! Contract `upgrade` is not covered here: it relies on `admin.require_auth()`, which
//! `mock_all_auths` cannot reject (see tests/contract_upgrade.rs).

#![cfg(test)]

use adl_handler::{AdlHandler, AdlHandlerClient as AHClient};
use data_store::{DataStore, DataStoreClient as DsClient};
use fee_handler::{FeeHandler, FeeHandlerClient as FHClient};
use gmx_keys::roles;
use gmx_math::FLOAT_PRECISION;
use gmx_types::TokenPrice;
use liquidation_handler::{LiquidationHandler, LiquidationHandlerClient as LHClient};
use oracle::{Oracle, OracleClient as OClient};
use order_handler::{OrderHandler, OrderHandlerClient as OHClient};
use order_vault::{OrderVault, OrderVaultClient as OVClient};
use role_store::{RoleStore, RoleStoreClient as RsClient};
use soroban_sdk::{testutils::Address as _, Address, BytesN, Env, Vec};

const ONE_USD: i128 = FLOAT_PRECISION;

struct World {
    env: Env,
    admin: Address,
    trader: Address,
    stranger: Address,
    fee_keeper: Address,
    order_keeper: Address,
    liq_keeper: Address,
    adl_keeper: Address,
    controller: Address,
    rs: Address,
    ds: Address,
    oracle: Address,
    ord_vault: Address,
    ord_handler: Address,
    liq_handler: Address,
    adl_handler: Address,
    fee_handler: Address,
}

fn setup() -> World {
    let env = Env::default();
    env.mock_all_auths();
    env.cost_estimate().budget().reset_unlimited();

    let admin = Address::generate(&env);
    let trader = Address::generate(&env);
    let stranger = Address::generate(&env);
    let fee_keeper = Address::generate(&env);
    let order_keeper = Address::generate(&env);
    let liq_keeper = Address::generate(&env);
    let adl_keeper = Address::generate(&env);
    let controller = Address::generate(&env);

    let rs = env.register(RoleStore, ());
    let rs_c = RsClient::new(&env, &rs);
    rs_c.initialize(&admin);
    rs_c.grant_role(&admin, &fee_keeper, &roles::fee_keeper(&env));
    rs_c.grant_role(&admin, &order_keeper, &roles::order_keeper(&env));
    rs_c.grant_role(&admin, &liq_keeper, &roles::liquidation_keeper(&env));
    rs_c.grant_role(&admin, &adl_keeper, &roles::adl_keeper(&env));
    rs_c.grant_role(&admin, &controller, &roles::controller(&env));

    let ds = env.register(DataStore, ());
    DsClient::new(&env, &ds).initialize(&admin, &rs);

    let oracle = env.register(Oracle, ());
    let passphrase = soroban_sdk::Bytes::from_slice(&env, b"Test SDF Network ; September 2015");
    OClient::new(&env, &oracle).initialize(&admin, &rs, &ds, &passphrase);

    let ord_vault = env.register(OrderVault, ());
    OVClient::new(&env, &ord_vault).initialize(&admin, &rs);

    let ord_handler = env.register(OrderHandler, ());
    OHClient::new(&env, &ord_handler).initialize(&admin, &rs, &ds, &oracle, &ord_vault);

    let liq_handler = env.register(LiquidationHandler, ());
    LHClient::new(&env, &liq_handler).initialize(&admin, &rs, &ds, &oracle, &ord_handler);

    let adl_handler = env.register(AdlHandler, ());
    AHClient::new(&env, &adl_handler).initialize(&admin, &rs, &ds, &oracle, &ord_handler);

    let fee_handler = env.register(FeeHandler, ());
    FHClient::new(&env, &fee_handler).initialize(&admin, &rs, &ds);

    World {
        env,
        admin,
        trader,
        stranger,
        fee_keeper,
        order_keeper,
        liq_keeper,
        adl_keeper,
        controller,
        rs,
        ds,
        oracle,
        ord_vault,
        ord_handler,
        liq_handler,
        adl_handler,
        fee_handler,
    }
}

/// Callers that hold no admin authority anywhere: they must be rejected by every
/// admin-gated entrypoint.
fn non_admins(w: &World) -> [Address; 7] {
    [
        w.trader.clone(),
        w.stranger.clone(),
        w.fee_keeper.clone(),
        w.order_keeper.clone(),
        w.liq_keeper.clone(),
        w.adl_keeper.clone(),
        w.controller.clone(),
    ]
}

fn price(w: &World) -> Vec<TokenPrice> {
    Vec::from_array(
        &w.env,
        [TokenPrice {
            token: Address::generate(&w.env),
            min: ONE_USD,
            max: ONE_USD,
        }],
    )
}

// ─── Part 1: every privileged entrypoint, authorised vs unauthorised ─────────

#[test]
fn role_store_admin_entrypoints_reject_non_admins() {
    let w = setup();
    let rs_c = RsClient::new(&w.env, &w.rs);
    let role = roles::order_keeper(&w.env);
    let target = Address::generate(&w.env);

    for caller in non_admins(&w) {
        assert!(rs_c.try_grant_role(&caller, &target, &role).is_err());
        assert!(rs_c
            .try_revoke_role(&caller, &w.order_keeper, &role)
            .is_err());
    }
    // Rejected calls changed nothing.
    assert!(!rs_c.has_role(&target, &role));
    assert!(rs_c.has_role(&w.order_keeper, &role));
    assert_eq!(rs_c.get_role_member_count(&role), 1);

    // Positive path.
    assert!(rs_c.try_grant_role(&w.admin, &target, &role).is_ok());
    assert!(rs_c.has_role(&target, &role));
}

#[test]
fn data_store_writes_reject_callers_without_controller() {
    let w = setup();
    let ds_c = DsClient::new(&w.env, &w.ds);
    let key = BytesN::from_array(&w.env, &[7u8; 32]);
    let addr = Address::generate(&w.env);

    for caller in [
        w.trader.clone(),
        w.stranger.clone(),
        w.fee_keeper.clone(),
        w.order_keeper.clone(),
        w.liq_keeper.clone(),
        w.adl_keeper.clone(),
    ] {
        assert!(ds_c.try_set_u128(&caller, &key, &1u128).is_err());
        assert!(ds_c.try_set_i128(&caller, &key, &1i128).is_err());
        assert!(ds_c.try_set_bool(&caller, &key, &true).is_err());
        assert!(ds_c.try_set_address(&caller, &key, &addr).is_err());
        assert!(ds_c.try_remove_u128(&caller, &key).is_err());
        assert!(ds_c.try_increment_u128(&caller, &key, &1u128).is_err());
    }
    // Rejected calls wrote nothing.
    assert_eq!(ds_c.get_u128(&key), 0);
    assert!(!ds_c.get_bool(&key));

    // Positive path: CONTROLLER may write.
    assert!(ds_c.try_set_u128(&w.controller, &key, &5u128).is_ok());
    assert_eq!(ds_c.get_u128(&key), 5);
}

#[test]
fn order_vault_entrypoints_reject_callers_without_controller() {
    let w = setup();
    let ov_c = OVClient::new(&w.env, &w.ord_vault);
    let token = Address::generate(&w.env);

    for caller in [
        w.trader.clone(),
        w.stranger.clone(),
        w.fee_keeper.clone(),
        w.order_keeper.clone(),
    ] {
        assert!(ov_c.try_record_transfer_in(&caller, &token).is_err());
        assert!(ov_c
            .try_transfer_out(&caller, &token, &caller, &1i128)
            .is_err());
    }
}

#[test]
fn oracle_admin_entrypoints_reject_non_admins() {
    let w = setup();
    let oracle_c = OClient::new(&w.env, &w.oracle);
    let market = Address::generate(&w.env);
    let token = Address::generate(&w.env);
    let pubkey = BytesN::from_array(&w.env, &[3u8; 32]);

    for caller in non_admins(&w) {
        assert!(oracle_c.try_rotate_signer(&caller, &0u32, &pubkey).is_err());
        assert!(oracle_c
            .try_register_market_for_breaker(&caller, &token, &market)
            .is_err());
        assert!(oracle_c
            .try_set_circuit_breaker_factor(&caller, &market, &100u128)
            .is_err());
    }
    // Rejected calls wrote nothing through the oracle's CONTROLLER reference.
    let ds_c = DsClient::new(&w.env, &w.ds);
    assert_eq!(
        ds_c.get_u128(&gmx_keys::circuit_breaker_factor_key(&w.env, &market)),
        0
    );
}

#[test]
fn oracle_price_entrypoints_require_order_keeper() {
    let w = setup();
    let oracle_c = OClient::new(&w.env, &w.oracle);
    let prices = price(&w);
    let token = prices.get(0).unwrap().token;

    for caller in [
        w.trader.clone(),
        w.stranger.clone(),
        w.fee_keeper.clone(),
        w.liq_keeper.clone(),
        w.adl_keeper.clone(),
        w.controller.clone(),
        w.admin.clone(), // ROLE_ADMIN alone does not grant ORDER_KEEPER
    ] {
        assert!(oracle_c.try_set_prices_simple(&caller, &prices).is_err());
        assert!(oracle_c.try_clear_price(&caller, &token).is_err());
    }
    assert!(oracle_c.try_get_price(&token).is_none());

    // Positive path.
    assert!(oracle_c
        .try_set_prices_simple(&w.order_keeper, &prices)
        .is_ok());
}

#[test]
fn handler_admin_entrypoints_reject_non_admins() {
    let w = setup();
    let new_oracle = Address::generate(&w.env);
    let referral = Address::generate(&w.env);
    let oh_c = OHClient::new(&w.env, &w.ord_handler);
    let lh_c = LHClient::new(&w.env, &w.liq_handler);
    let ah_c = AHClient::new(&w.env, &w.adl_handler);

    for caller in non_admins(&w) {
        assert!(oh_c.try_update_oracle(&caller, &new_oracle).is_err());
        assert!(oh_c.try_set_referral_storage(&caller, &referral).is_err());
        assert!(oh_c
            .try_set_keeper_heartbeat_timeout(&caller, &roles::order_keeper(&w.env), &10u64)
            .is_err());
        assert!(lh_c.try_update_oracle(&caller, &new_oracle).is_err());
        assert!(ah_c.try_update_oracle(&caller, &new_oracle).is_err());
    }

    // Positive path: the contract admin may call them.
    assert!(oh_c.try_update_oracle(&w.admin, &new_oracle).is_ok());
    assert!(oh_c.try_set_referral_storage(&w.admin, &referral).is_ok());
    assert!(lh_c.try_update_oracle(&w.admin, &new_oracle).is_ok());
    assert!(ah_c.try_update_oracle(&w.admin, &new_oracle).is_ok());
}

#[test]
fn keeper_entrypoints_reject_callers_without_their_role() {
    let w = setup();
    let oh_c = OHClient::new(&w.env, &w.ord_handler);
    let lh_c = LHClient::new(&w.env, &w.liq_handler);
    let ah_c = AHClient::new(&w.env, &w.adl_handler);
    let fh_c = FHClient::new(&w.env, &w.fee_handler);
    let market = Address::generate(&w.env);
    let token = Address::generate(&w.env);
    let key = BytesN::from_array(&w.env, &[9u8; 32]);

    // Each keeper holds a role, but none may use another keeper's entrypoints.
    assert!(oh_c.try_execute_order(&w.liq_keeper, &key).is_err());
    assert!(oh_c.try_execute_order(&w.fee_keeper, &key).is_err());
    assert!(oh_c.try_freeze_order(&w.fee_keeper, &key).is_err());
    assert!(oh_c.try_execute_order(&w.stranger, &key).is_err());
    assert!(lh_c
        .try_liquidate_position(&w.order_keeper, &w.trader, &market, &token, &true)
        .is_err());
    assert!(lh_c
        .try_liquidate_position(&w.stranger, &w.trader, &market, &token, &true)
        .is_err());
    assert!(ah_c
        .try_execute_adl(&w.order_keeper, &w.trader, &market, &token, &true, &ONE_USD)
        .is_err());
    assert!(fh_c
        .try_claim_fees(&w.order_keeper, &market, &token)
        .is_err());
    assert!(fh_c.try_claim_fees(&w.stranger, &market, &token).is_err());
}

// ─── Part 2: rotation, revocation, trusted-dependency substitution ───────────

#[test]
fn rotating_a_keeper_role_moves_access_to_the_new_holder() {
    let w = setup();
    let rs_c = RsClient::new(&w.env, &w.rs);
    let oracle_c = OClient::new(&w.env, &w.oracle);
    let role = roles::order_keeper(&w.env);
    let prices = price(&w);
    let successor = Address::generate(&w.env);

    assert!(oracle_c.try_set_prices_simple(&successor, &prices).is_err());

    rs_c.grant_role(&w.admin, &successor, &role);
    rs_c.revoke_role(&w.admin, &w.order_keeper, &role);

    assert!(oracle_c.try_set_prices_simple(&successor, &prices).is_ok());
    assert!(oracle_c
        .try_set_prices_simple(&w.order_keeper, &prices)
        .is_err());
    assert!(!rs_c.has_role(&w.order_keeper, &role));
    assert_eq!(rs_c.get_role_member_count(&role), 1);
}

#[test]
fn revoking_controller_blocks_data_store_writes_immediately() {
    let w = setup();
    let rs_c = RsClient::new(&w.env, &w.rs);
    let ds_c = DsClient::new(&w.env, &w.ds);
    let key = BytesN::from_array(&w.env, &[1u8; 32]);

    assert!(ds_c.try_set_u128(&w.controller, &key, &1u128).is_ok());
    rs_c.revoke_role(&w.admin, &w.controller, &roles::controller(&w.env));
    assert!(ds_c.try_set_u128(&w.controller, &key, &2u128).is_err());
    assert_eq!(ds_c.get_u128(&key), 1);
}

#[test]
fn role_admin_rotation_hands_over_grant_authority() {
    let w = setup();
    let rs_c = RsClient::new(&w.env, &w.rs);
    let admin_role = roles::role_admin(&w.env);
    let new_admin = Address::generate(&w.env);
    let target = Address::generate(&w.env);
    let role = roles::fee_keeper(&w.env);

    // The sole ROLE_ADMIN cannot be removed.
    assert!(rs_c
        .try_revoke_role(&w.admin, &w.admin, &admin_role)
        .is_err());
    assert!(rs_c.has_role(&w.admin, &admin_role));

    rs_c.grant_role(&w.admin, &new_admin, &admin_role);
    rs_c.revoke_role(&new_admin, &w.admin, &admin_role);

    assert!(rs_c.try_grant_role(&w.admin, &target, &role).is_err());
    assert!(!rs_c.has_role(&target, &role));
    assert!(rs_c.try_grant_role(&new_admin, &target, &role).is_ok());
    assert!(rs_c.has_role(&target, &role));
    assert_eq!(rs_c.get_role_member_count(&admin_role), 1);
}

#[test]
fn role_admin_grants_no_custody_or_upgrade_authority() {
    let w = setup();
    let rs_c = RsClient::new(&w.env, &w.rs);
    let second_admin = Address::generate(&w.env);
    rs_c.grant_role(&w.admin, &second_admin, &roles::role_admin(&w.env));

    let ds_c = DsClient::new(&w.env, &w.ds);
    let ov_c = OVClient::new(&w.env, &w.ord_vault);
    let oh_c = OHClient::new(&w.env, &w.ord_handler);
    let oracle_c = OClient::new(&w.env, &w.oracle);
    let key = BytesN::from_array(&w.env, &[2u8; 32]);
    let token = Address::generate(&w.env);
    let other = Address::generate(&w.env);

    // ROLE_ADMIN is not CONTROLLER, ORDER_KEEPER, or a contract-level admin.
    assert!(ds_c.try_set_u128(&second_admin, &key, &1u128).is_err());
    assert!(ov_c
        .try_transfer_out(&second_admin, &token, &second_admin, &1i128)
        .is_err());
    assert!(oh_c.try_update_oracle(&second_admin, &other).is_err());
    assert!(oracle_c
        .try_rotate_signer(
            &second_admin,
            &0u32,
            &BytesN::from_array(&w.env, &[4u8; 32])
        )
        .is_err());
    assert!(oracle_c
        .try_set_prices_simple(&second_admin, &price(&w))
        .is_err());
}

#[test]
fn keeper_roles_grant_no_role_management_authority() {
    let w = setup();
    let rs_c = RsClient::new(&w.env, &w.rs);
    let role = roles::controller(&w.env);

    for keeper in [
        w.fee_keeper.clone(),
        w.order_keeper.clone(),
        w.liq_keeper.clone(),
        w.adl_keeper.clone(),
        w.controller.clone(),
    ] {
        assert!(rs_c.try_grant_role(&keeper, &keeper, &role).is_err());
    }
    assert_eq!(rs_c.get_role_member_count(&role), 1);
    assert!(!rs_c.has_role(&w.fee_keeper, &role));
}

#[test]
fn trusted_dependencies_cannot_be_replaced_by_user_supplied_addresses() {
    let w = setup();
    let fake_rs = Address::generate(&w.env);
    let fake_oracle = Address::generate(&w.env);

    // Re-initialising an already-initialised contract with attacker-chosen
    // dependencies is rejected.
    assert!(DsClient::new(&w.env, &w.ds)
        .try_initialize(&w.stranger, &fake_rs)
        .is_err());
    assert!(OVClient::new(&w.env, &w.ord_vault)
        .try_initialize(&w.stranger, &fake_rs)
        .is_err());
    assert!(FHClient::new(&w.env, &w.fee_handler)
        .try_initialize(&w.stranger, &fake_rs, &w.ds)
        .is_err());
    assert!(RsClient::new(&w.env, &w.rs)
        .try_initialize(&w.stranger)
        .is_err());
    assert!(OHClient::new(&w.env, &w.ord_handler)
        .try_initialize(&w.stranger, &fake_rs, &w.ds, &fake_oracle, &w.ord_vault)
        .is_err());
    assert!(LHClient::new(&w.env, &w.liq_handler)
        .try_initialize(&w.stranger, &fake_rs, &w.ds, &fake_oracle, &w.ord_handler)
        .is_err());
    assert!(AHClient::new(&w.env, &w.adl_handler)
        .try_initialize(&w.stranger, &fake_rs, &w.ds, &fake_oracle, &w.ord_handler)
        .is_err());
    assert!(OClient::new(&w.env, &w.oracle)
        .try_initialize(
            &w.stranger,
            &fake_rs,
            &w.ds,
            &soroban_sdk::Bytes::from_slice(&w.env, b"x")
        )
        .is_err());

    // The original role_store is still the authority: the stranger gained nothing.
    let rs_c = RsClient::new(&w.env, &w.rs);
    assert!(!rs_c.has_role(&w.stranger, &roles::role_admin(&w.env)));
    assert!(rs_c.has_role(&w.admin, &roles::role_admin(&w.env)));
}

#[test]
fn admin_cannot_point_oracle_at_the_handlers_own_dependencies() {
    let w = setup();
    let oh_c = OHClient::new(&w.env, &w.ord_handler);
    let lh_c = LHClient::new(&w.env, &w.liq_handler);
    let ah_c = AHClient::new(&w.env, &w.adl_handler);

    for bad in [&w.rs, &w.ds, &w.admin, &w.ord_handler] {
        assert!(oh_c.try_update_oracle(&w.admin, bad).is_err());
    }
    for bad in [&w.rs, &w.ds, &w.admin, &w.liq_handler] {
        assert!(lh_c.try_update_oracle(&w.admin, bad).is_err());
    }
    for bad in [&w.rs, &w.ds, &w.admin, &w.adl_handler] {
        assert!(ah_c.try_update_oracle(&w.admin, bad).is_err());
    }
}
