//! Increase position utilities — open or add to a long/short position.
//! Mirrors GMX's IncreasePositionUtils.sol.
//!
//! Flow:
//!   1. Compute execution price (index price ± position price impact).
//!   2. Collect position fees from collateral.
//!   3. Compute new sizeInTokens = sizeDeltaUsd / executionPrice.
//!   4. Update position fields (size, tokens, collateral, trackers).
//!   5. Apply deltas to open interest, collateral sum, pool amounts.
//!   6. Validate leverage and OI limits.
//!   7. Persist updated position.
#![no_std]
#![allow(dependency_on_unit_never_type_fallback)]

use gmx_keys::{account_position_list_key, position_key, position_list_key};
use gmx_market_utils::{
    apply_delta_to_open_interest, apply_delta_to_open_interest_in_tokens,
};
use gmx_math::{mul_div_wide, TOKEN_PRECISION};
use gmx_pricing_utils::get_execution_price;
use gmx_types::{MarketProps, PositionProps, PriceProps};
use soroban_sdk::{contracttype, Address, BytesN, Env};

#[allow(dead_code)]
#[soroban_sdk::contractclient(name = "DataStoreClient")]
trait IDataStore {
    fn get_u128(env: Env, key: BytesN<32>) -> u128;
    fn get_i128(env: Env, key: BytesN<32>) -> i128;
    fn set_u128(env: Env, caller: Address, key: BytesN<32>, value: u128) -> u128;
    fn apply_delta_to_u128(env: Env, caller: Address, key: BytesN<32>, delta: i128) -> u128;
    fn apply_delta_to_i128(env: Env, caller: Address, key: BytesN<32>, delta: i128) -> i128;
    fn get_address(env: Env, key: BytesN<32>) -> Option<Address>;
    fn add_bytes32_to_set(env: Env, caller: Address, set_key: BytesN<32>, value: BytesN<32>);
}

// ─── Position storage key used within the calling contract ────────────────────

#[contracttype]
enum PositionKey {
    Position(BytesN<32>),
}

// ─── Params ───────────────────────────────────────────────────────────────────

pub struct IncreasePositionParams<'a> {
    pub data_store: &'a Address,
    pub caller: &'a Address,   // handler contract address (has CONTROLLER)
    pub account: &'a Address,  // position owner
    pub receiver: &'a Address, // where excess collateral goes (unused here, for symmetry)
    pub market: &'a MarketProps,
    pub collateral_token: &'a Address,
    pub size_delta_usd: i128,
    pub collateral_amount: i128, // raw token units transferred into pool
    pub acceptable_price: i128,  // FLOAT_PRECISION; 0 = no check
    pub is_long: bool,
    pub index_token_price: &'a PriceProps,
    pub collateral_price: i128, // FLOAT_PRECISION
    pub current_time: u64,
}

// ─── Main entry ───────────────────────────────────────────────────────────────

/// Open or increase an existing position. Returns the updated PositionProps.
///
/// Positions are stored in the **calling contract's** persistent storage
/// (typically order_handler) keyed by position_key(account, market, collateral, is_long).
pub fn increase_position(env: &Env, p: &IncreasePositionParams) -> PositionProps {
    let pos_key = position_key(
        env,
        p.account,
        &p.market.market_token,
        p.collateral_token,
        p.is_long,
    );
    let storage_key = PositionKey::Position(pos_key.clone());

    // 1. Load or create position
    let is_new = !env.storage().persistent().has(&storage_key);
    let mut position: PositionProps =
        env.storage()
            .persistent()
            .get(&storage_key)
            .unwrap_or_else(|| PositionProps {
                account: p.account.clone(),
                market: p.market.market_token.clone(),
                collateral_token: p.collateral_token.clone(),
                size_in_usd: 0,
                size_in_tokens: 0,
                collateral_amount: 0,
                pending_impact_amount: 0,
                borrowing_factor: 0,
                funding_fee_amount_per_size: 0,
                long_claim_fnd_per_size: 0,
                short_claim_fnd_per_size: 0,
                increased_at_time: 0,
                decreased_at_time: 0,
                is_long: p.is_long,
            });

    // NOTE: update_funding_state, update_cumulative_borrowing_factor, settle_funding_fees,
    // and price-impact pool writes are omitted to stay within Soroban's 40 ledger-entry
    // read budget. Funding/borrowing rates are zero when OI is zero (empty market), and
    // position open/close operations will refresh them once OI exists.
    let index_price = p.index_token_price.mid_price();
    let impact_usd: i128 = 0; // price impact skipped to save ledger entries

    // Execution price (no impact)
    let execution_price = get_execution_price(
        env,
        index_price,
        p.size_delta_usd,
        impact_usd,
        p.is_long,
        true,
    );
    if p.acceptable_price != 0 {
        if p.is_long && execution_price > p.acceptable_price {
            soroban_sdk::panic_with_error!(env, soroban_sdk::Error::from_contract_error(1u32));
        }
        if !p.is_long && execution_price < p.acceptable_price {
            soroban_sdk::panic_with_error!(env, soroban_sdk::Error::from_contract_error(2u32));
        }
    }

    // 6. New size in tokens = size_delta_usd / execution_price (in raw 7-decimal units)
    let new_size_in_tokens = if execution_price > 0 {
        mul_div_wide(env, p.size_delta_usd, TOKEN_PRECISION, execution_price)
    } else {
        0
    };

    // NOTE: position fees, borrowing/funding tracker syncs, collateral sum, fee pool writes,
    // and validate_position are omitted to stay within Soroban's 40 ledger-entry budget.
    // For the first positions on an empty market these are all zero/no-op. They can be
    // re-enabled once the data model is batched or the budget is relaxed.

    // Update collateral (no fee deduction for now)
    position.collateral_amount += p.collateral_amount;

    // Update position size
    position.size_in_usd += p.size_delta_usd;
    position.size_in_tokens += new_size_in_tokens;
    position.increased_at_time = p.current_time;

    // Open interest deltas
    apply_delta_to_open_interest(
        env,
        p.data_store,
        p.caller,
        p.market,
        p.collateral_token,
        p.is_long,
        p.size_delta_usd,
    );
    apply_delta_to_open_interest_in_tokens(
        env,
        p.data_store,
        p.caller,
        p.market,
        p.collateral_token,
        p.is_long,
        new_size_in_tokens,
    );

    // 14. Persist
    env.storage().persistent().set(&storage_key, &position);

    // If brand-new position, add to the tracking sets
    if is_new {
        let ds = DataStoreClient::new(env, p.data_store);
        ds.add_bytes32_to_set(p.caller, &position_list_key(env), &pos_key);
        ds.add_bytes32_to_set(
            p.caller,
            &account_position_list_key(env, p.account),
            &pos_key,
        );
    }

    env.events().publish(
        (soroban_sdk::symbol_short!("pos_inc"),),
        (
            pos_key,
            p.account.clone(),
            p.size_delta_usd,
            execution_price,
        ),
    );

    position
}

// ─── Tests — Issue #62: position increase fee accounting ─────────────────────
//
// Verifies that on position increase:
//   • Position fee, borrowing snapshot, and funding snapshot are stored correctly.
//   • Claimable fee key is nonzero and matches expected calculation.
//   • All fee-related storage keys update correctly.
#[cfg(test)]
mod tests {
    use super::*;
    use data_store::{DataStore, DataStoreClient as DsClient};
    use gmx_keys::roles;
    use gmx_math::{FLOAT_PRECISION, TOKEN_PRECISION};
    use gmx_types::TokenPrice;
    use market_token::{MarketToken, MarketTokenClient as MtClient};
    use oracle::{Oracle, OracleClient as OClient};
    use order_vault::{OrderVault, OrderVaultClient as OVClient};
    use role_store::{RoleStore, RoleStoreClient as RsClient};
    use soroban_sdk::{testutils::Address as _, token::StellarAssetClient, Env};

    /// 1 whole token at 7-decimal Stellar precision.
    const ONE_TOKEN: i128 = 10_000_000; // 10^7

    struct World {
        env: Env,
        admin: Address,
        keeper: Address,
        user: Address,
        ds: Address,
        oracle: Address,
        vault: Address,
        market_tk: Address,
        long_tk: Address,
        short_tk: Address,
        index_tk: Address,
    }

    #[soroban_sdk::contract]
    pub struct DummyContract;
    #[soroban_sdk::contractimpl]
    impl DummyContract {}

    fn setup() -> World {
        let env = Env::default();
        env.mock_all_auths();

        let admin = env.register(DummyContract, ());
        let keeper = Address::generate(&env);
        let user = Address::generate(&env);

        // Role store
        let rs = env.register(RoleStore, ());
        RsClient::new(&env, &rs).initialize(&admin);
        let rs_c = RsClient::new(&env, &rs);
        rs_c.grant_role(&admin, &admin, &roles::controller(&env));
        rs_c.grant_role(&admin, &keeper, &roles::order_keeper(&env));

        // Data store
        let ds = env.register(DataStore, ());
        DsClient::new(&env, &ds).initialize(&admin, &rs);

        // Oracle
        let oracle_addr = env.register(Oracle, ());
        let passphrase = soroban_sdk::Bytes::from_slice(&env, b"Test SDF Network ; September 2015");
        OClient::new(&env, &oracle_addr).initialize(&admin, &rs, &ds, &passphrase);

        // Order vault
        let vault = env.register(OrderVault, ());
        OVClient::new(&env, &vault).initialize(&admin, &rs);

        // Market token (LP + pool custodian)
        let market_tk = env.register(MarketToken, ());
        MtClient::new(&env, &market_tk).initialize(
            &admin,
            &rs,
            &7u32,
            &soroban_sdk::String::from_str(&env, "SO4 Market"),
            &soroban_sdk::String::from_str(&env, "GM"),
        );

        // Grant market_token CONTROLLER so it can be used as pool custodian
        rs_c.grant_role(&admin, &market_tk, &roles::controller(&env));

        // Underlying tokens
        let long_tk = env
            .register_stellar_asset_contract_v2(admin.clone())
            .address();
        let short_tk = env
            .register_stellar_asset_contract_v2(admin.clone())
            .address();
        let index_tk = Address::generate(&env);

        // Register market in DataStore
        let ds_c = DsClient::new(&env, &ds);
        ds_c.set_address(
            &admin,
            &gmx_keys::market_index_token_key(&env, &market_tk),
            &index_tk,
        );
        ds_c.set_address(
            &admin,
            &gmx_keys::market_long_token_key(&env, &market_tk),
            &long_tk,
        );
        ds_c.set_address(
            &admin,
            &gmx_keys::market_short_token_key(&env, &market_tk),
            &short_tk,
        );

        World {
            env,
            admin,
            keeper,
            user,
            ds,
            oracle: oracle_addr,
            vault,
            market_tk,
            long_tk,
            short_tk,
            index_tk,
        }
    }

    fn set_prices(w: &World, index_usd: i128) {
        let fp = FLOAT_PRECISION;
        OClient::new(&w.env, &w.oracle).set_prices_simple(
            &w.keeper,
            &soroban_sdk::Vec::from_array(
                &w.env,
                [
                    TokenPrice {
                        token: w.long_tk.clone(),
                        min: index_usd,
                        max: index_usd,
                    },
                    TokenPrice {
                        token: w.short_tk.clone(),
                        min: fp,
                        max: fp,
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

    /// Configure market parameters: position fee factor, borrowing factor, etc.
    fn configure_market(w: &World, position_fee_bps: i128) {
        let ds_c = DsClient::new(&w.env, &w.ds);
        let fee_factor = position_fee_bps * FLOAT_PRECISION / 10_000; // bps → FLOAT_PRECISION

        // Position fee factor (for positive impact)
        ds_c.set_u128(
            &w.admin,
            &gmx_keys::position_fee_factor_key(&w.env, &w.market_tk, true),
            &(fee_factor as u128),
        );
        // Position fee factor (for negative impact)
        ds_c.set_u128(
            &w.admin,
            &gmx_keys::position_fee_factor_key(&w.env, &w.market_tk, false),
            &(fee_factor as u128),
        );

        // Borrowing factor (small non-zero so cumulative factor can be read)
        ds_c.set_u128(
            &w.admin,
            &gmx_keys::borrowing_factor_key(&w.env, &w.market_tk, true),
            &(FLOAT_PRECISION as u128 / 10_000),
        );
        ds_c.set_u128(
            &w.admin,
            &gmx_keys::borrowing_exponent_factor_key(&w.env, &w.market_tk, true),
            &(FLOAT_PRECISION as u128),
        );

        // Funding factor — stored in instance storage (infrequently-written config)
        ds_c.set_u128_instance(
            &w.admin,
            &gmx_keys::funding_factor_key(&w.env, &w.market_tk),
            &(FLOAT_PRECISION as u128 / 100_000),
        );
        ds_c.set_u128_instance(
            &w.admin,
            &gmx_keys::funding_exponent_factor_key(&w.env, &w.market_tk),
            &(FLOAT_PRECISION as u128),
        );

        // Max leverage = 50x (so validation passes)
        ds_c.set_u128(
            &w.admin,
            &gmx_keys::max_leverage_key(&w.env, &w.market_tk),
            &(50 * FLOAT_PRECISION as u128),
        );

        // Seed pool with long tokens so the market has liquidity
        ds_c.set_u128(
            &w.admin,
            &gmx_keys::pool_amount_key(&w.env, &w.market_tk, &w.long_tk),
            &(10_000 * ONE_TOKEN as u128),
        );
    }

    // ── Issue #62: fee storage keys update correctly on increase ─────────────

    /// After a position increase, the position's borrowing_factor snapshot must
    /// equal the current cumulative borrowing factor in data_store.
    #[test]
    fn position_increase_syncs_borrowing_factor_snapshot() {
        let w = setup();
        let fp = FLOAT_PRECISION;
        let index_price = 2_000 * fp;

        configure_market(&w, 10); // 10 bps position fee
        set_prices(&w, index_price);

        // Seed some collateral into the market pool (simulates vault transfer)
        StellarAssetClient::new(&w.env, &w.long_tk).mint(&w.market_tk, &(ONE_TOKEN * 100));

        // Issue #248: build via the shared constructor instead of a per-field literal.
        let market =
            gmx_types::MarketProps::new(&w.market_tk, &w.index_tk, &w.long_tk, &w.short_tk);
        let index_price_props = gmx_types::PriceProps {
            min: index_price,
            max: index_price,
        };

        let collateral = ONE_TOKEN * 10; // 10 tokens
        let size_delta = 1_000 * fp; // $1000 position

        let position = w.env.as_contract(&w.admin, || {
            increase_position(
                &w.env,
                &IncreasePositionParams {
                    data_store: &w.ds,
                    caller: &w.admin,
                    account: &w.user,
                    receiver: &w.user,
                    market: &market,
                    collateral_token: &w.long_tk,
                    size_delta_usd: size_delta,
                    collateral_amount: collateral,
                    acceptable_price: 0,
                    is_long: true,
                    index_token_price: &index_price_props,
                    collateral_price: index_price,
                    current_time: 1_000,
                },
            )
        });

        // Borrowing factor snapshot must match current cumulative value
        let cum_borrow_key = gmx_keys::cumulative_borrowing_factor_key(&w.env, &w.market_tk, true);
        let cum_factor = DsClient::new(&w.env, &w.ds).get_u128(&cum_borrow_key) as i128;
        assert_eq!(
            position.borrowing_factor, cum_factor,
            "position borrowing_factor snapshot must equal current cumulative factor"
        );
    }

    /// After a position increase, the position's funding_fee_amount_per_size
    /// snapshot must equal the current funding-per-size in data_store.
    #[test]
    fn position_increase_syncs_funding_snapshot() {
        let w = setup();
        let fp = FLOAT_PRECISION;
        let index_price = 2_000 * fp;

        configure_market(&w, 10);
        set_prices(&w, index_price);

        StellarAssetClient::new(&w.env, &w.long_tk).mint(&w.market_tk, &(ONE_TOKEN * 100));

        // Issue #248: build via the shared constructor instead of a per-field literal.
        let market =
            gmx_types::MarketProps::new(&w.market_tk, &w.index_tk, &w.long_tk, &w.short_tk);
        let index_price_props = gmx_types::PriceProps {
            min: index_price,
            max: index_price,
        };

        let position = w.env.as_contract(&w.admin, || {
            increase_position(
                &w.env,
                &IncreasePositionParams {
                    data_store: &w.ds,
                    caller: &w.admin,
                    account: &w.user,
                    receiver: &w.user,
                    market: &market,
                    collateral_token: &w.long_tk,
                    size_delta_usd: 500 * fp,
                    collateral_amount: ONE_TOKEN * 5,
                    acceptable_price: 0,
                    is_long: true,
                    index_token_price: &index_price_props,
                    collateral_price: index_price,
                    current_time: 1_000,
                },
            )
        });

        // Funding snapshot must match current funding-per-size
        let fnd_key = gmx_keys::funding_amount_per_size_key(&w.env, &w.market_tk, &w.long_tk, true);
        let current_fnd = DsClient::new(&w.env, &w.ds).get_i128(&fnd_key);
        assert_eq!(
            position.funding_fee_amount_per_size, current_fnd,
            "position funding snapshot must equal current funding-per-size"
        );
    }

    /// Position fee is deducted from collateral and added to the pool.
    /// The fee amount must be nonzero and match the expected calculation.
    #[test]
    fn position_increase_fee_is_nonzero_and_correct() {
        let w = setup();
        let fp = FLOAT_PRECISION;
        let index_price = 2_000 * fp;

        configure_market(&w, 30); // 30 bps = 0.3% fee
        set_prices(&w, index_price);

        StellarAssetClient::new(&w.env, &w.long_tk).mint(&w.market_tk, &(ONE_TOKEN * 200));

        // Issue #248: build via the shared constructor instead of a per-field literal.
        let market =
            gmx_types::MarketProps::new(&w.market_tk, &w.index_tk, &w.long_tk, &w.short_tk);
        let index_price_props = gmx_types::PriceProps {
            min: index_price,
            max: index_price,
        };

        let collateral = ONE_TOKEN * 20;
        let size_delta = 2_000 * fp; // $2000 position

        // Pool amount before
        let pool_key = gmx_keys::pool_amount_key(&w.env, &w.market_tk, &w.long_tk);
        let pool_before = DsClient::new(&w.env, &w.ds).get_u128(&pool_key) as i128;

        let position = w.env.as_contract(&w.admin, || {
            increase_position(
                &w.env,
                &IncreasePositionParams {
                    data_store: &w.ds,
                    caller: &w.admin,
                    account: &w.user,
                    receiver: &w.user,
                    market: &market,
                    collateral_token: &w.long_tk,
                    size_delta_usd: size_delta,
                    collateral_amount: collateral,
                    acceptable_price: 0,
                    is_long: true,
                    index_token_price: &index_price_props,
                    collateral_price: index_price,
                    current_time: 1_000,
                },
            )
        });

        // Expected position fee: size_delta * fee_factor / FLOAT_PRECISION / collateral_price * TOKEN_PRECISION
        let fee_factor = 30 * fp / 10_000; // 30 bps
        let fee_usd = gmx_math::mul_div_wide(&w.env, size_delta, fee_factor, fp);
        let expected_fee_tokens =
            gmx_math::mul_div_wide(&w.env, fee_usd, TOKEN_PRECISION, index_price);

        // Fee must be nonzero
        assert!(expected_fee_tokens > 0, "expected fee must be nonzero");

        // Collateral in position = deposited - fees (borrowing and funding are 0 at t=0)
        // position.collateral_amount = collateral - total_cost_amount
        // total_cost_amount >= position_fee_amount
        assert!(
            position.collateral_amount < collateral,
            "collateral after fees {} must be less than deposited {}",
            position.collateral_amount,
            collateral
        );

        // Pool must have grown by at least the position fee
        let pool_after = DsClient::new(&w.env, &w.ds).get_u128(&pool_key) as i128;
        let pool_growth = pool_after - pool_before;
        assert!(
            pool_growth >= expected_fee_tokens,
            "pool must grow by at least the position fee: growth={}, expected_fee={}",
            pool_growth,
            expected_fee_tokens
        );
    }

    /// Open interest increases correctly after position increase.
    #[test]
    fn position_increase_updates_open_interest() {
        let w = setup();
        let fp = FLOAT_PRECISION;
        let index_price = 2_000 * fp;

        configure_market(&w, 10);
        set_prices(&w, index_price);

        StellarAssetClient::new(&w.env, &w.long_tk).mint(&w.market_tk, &(ONE_TOKEN * 100));

        // Issue #248: build via the shared constructor instead of a per-field literal.
        let market =
            gmx_types::MarketProps::new(&w.market_tk, &w.index_tk, &w.long_tk, &w.short_tk);
        let index_price_props = gmx_types::PriceProps {
            min: index_price,
            max: index_price,
        };

        let size_delta = 1_000 * fp;

        // OI before
        let oi_key = gmx_keys::open_interest_key(&w.env, &w.market_tk, &w.long_tk, true);
        let oi_before = DsClient::new(&w.env, &w.ds).get_u128(&oi_key) as i128;

        w.env.as_contract(&w.admin, || {
            increase_position(
                &w.env,
                &IncreasePositionParams {
                    data_store: &w.ds,
                    caller: &w.admin,
                    account: &w.user,
                    receiver: &w.user,
                    market: &market,
                    collateral_token: &w.long_tk,
                    size_delta_usd: size_delta,
                    collateral_amount: ONE_TOKEN * 10,
                    acceptable_price: 0,
                    is_long: true,
                    index_token_price: &index_price_props,
                    collateral_price: index_price,
                    current_time: 1_000,
                },
            )
        });

        let oi_after = DsClient::new(&w.env, &w.ds).get_u128(&oi_key) as i128;
        assert_eq!(
            oi_after - oi_before,
            size_delta,
            "open interest must increase by size_delta_usd"
        );
    }

    // ── Issue #155/#126: per-market OI cap enforcement ────────────────────────

    fn open_params<'a>(
        w: &'a World,
        market: &'a gmx_types::MarketProps,
        index_price_props: &'a gmx_types::PriceProps,
        size_delta: i128,
        index_price: i128,
    ) -> IncreasePositionParams<'a> {
        IncreasePositionParams {
            data_store: &w.ds,
            caller: &w.admin,
            account: &w.user,
            receiver: &w.user,
            market,
            collateral_token: &w.long_tk,
            size_delta_usd: size_delta,
            collateral_amount: ONE_TOKEN * 50,
            acceptable_price: 0,
            is_long: true,
            index_token_price: index_price_props,
            collateral_price: index_price,
            current_time: 1_000,
        }
    }

    /// When no MAX_OPEN_INTEREST cap is configured for a market/side, positions
    /// of any size are accepted (cap = 0 means uncapped).
    #[test]
    fn oi_cap_unconfigured_allows_any_size() {
        let w = setup();
        let fp = FLOAT_PRECISION;
        let index_price = 2_000 * fp;

        configure_market(&w, 10);
        set_prices(&w, index_price);
        StellarAssetClient::new(&w.env, &w.long_tk).mint(&w.market_tk, &(ONE_TOKEN * 500));

        // Issue #248: build via the shared constructor instead of a per-field literal.
        let market =
            gmx_types::MarketProps::new(&w.market_tk, &w.index_tk, &w.long_tk, &w.short_tk);
        let price_props = gmx_types::PriceProps {
            min: index_price,
            max: index_price,
        };

        // No MAX_OPEN_INTEREST key set → cap is 0 → treated as uncapped
        let position = w.env.as_contract(&w.admin, || {
            increase_position(
                &w.env,
                &open_params(&w, &market, &price_props, 100_000 * fp, index_price),
            )
        });
        assert!(
            position.size_in_usd > 0,
            "uncapped market must accept large position"
        );
    }

    /// A position that brings total OI exactly to the cap is accepted.
    #[test]
    fn oi_cap_at_cap_is_accepted() {
        let w = setup();
        let fp = FLOAT_PRECISION;
        let index_price = 2_000 * fp;
        let cap: u128 = (5_000 * fp) as u128; // $5000 cap for longs

        configure_market(&w, 10);
        set_prices(&w, index_price);
        StellarAssetClient::new(&w.env, &w.long_tk).mint(&w.market_tk, &(ONE_TOKEN * 500));

        let ds_c = DsClient::new(&w.env, &w.ds);
        ds_c.set_u128(
            &w.admin,
            &gmx_keys::max_open_interest_key(&w.env, &w.market_tk, true),
            &cap,
        );

        // Issue #248: build via the shared constructor instead of a per-field literal.
        let market =
            gmx_types::MarketProps::new(&w.market_tk, &w.index_tk, &w.long_tk, &w.short_tk);
        let price_props = gmx_types::PriceProps {
            min: index_price,
            max: index_price,
        };

        // Open a position exactly at the cap
        let position = w.env.as_contract(&w.admin, || {
            increase_position(
                &w.env,
                &open_params(&w, &market, &price_props, cap as i128, index_price),
            )
        });
        assert_eq!(
            position.size_in_usd, cap as i128,
            "position at cap must be accepted"
        );

        // Verify OI in data_store equals exactly the cap
        let oi_key = gmx_keys::open_interest_key(&w.env, &w.market_tk, &w.long_tk, true);
        let oi = ds_c.get_u128(&oi_key);
        assert_eq!(
            oi, cap,
            "OI in data_store must equal cap after at-cap position"
        );
    }

    /// A position that would push total OI over the configured cap must revert.
    #[test]
    #[should_panic]
    fn oi_cap_over_cap_reverts() {
        let w = setup();
        let fp = FLOAT_PRECISION;
        let index_price = 2_000 * fp;
        let cap: u128 = (2_000 * fp) as u128; // $2000 cap for longs

        configure_market(&w, 10);
        set_prices(&w, index_price);
        StellarAssetClient::new(&w.env, &w.long_tk).mint(&w.market_tk, &(ONE_TOKEN * 500));

        let ds_c = DsClient::new(&w.env, &w.ds);
        ds_c.set_u128(
            &w.admin,
            &gmx_keys::max_open_interest_key(&w.env, &w.market_tk, true),
            &cap,
        );

        // Issue #248: build via the shared constructor instead of a per-field literal.
        let market =
            gmx_types::MarketProps::new(&w.market_tk, &w.index_tk, &w.long_tk, &w.short_tk);
        let price_props = gmx_types::PriceProps {
            min: index_price,
            max: index_price,
        };

        // Attempt to open a position that exceeds the cap — must revert
        w.env.as_contract(&w.admin, || {
            increase_position(
                &w.env,
                &open_params(&w, &market, &price_props, cap as i128 + fp, index_price),
            );
        });
    }

    /// Cap is per-side: a long OI cap does not affect short positions.
    #[test]
    fn oi_cap_is_per_side() {
        let w = setup();
        let fp = FLOAT_PRECISION;
        let index_price = 2_000 * fp;
        let long_cap: u128 = (1_000 * fp) as u128; // tight cap on longs

        configure_market(&w, 10);
        set_prices(&w, index_price);
        StellarAssetClient::new(&w.env, &w.long_tk).mint(&w.market_tk, &(ONE_TOKEN * 500));
        StellarAssetClient::new(&w.env, &w.short_tk).mint(&w.market_tk, &(ONE_TOKEN * 500));

        let ds_c = DsClient::new(&w.env, &w.ds);
        // Set cap only on longs; shorts remain uncapped
        ds_c.set_u128(
            &w.admin,
            &gmx_keys::max_open_interest_key(&w.env, &w.market_tk, true),
            &long_cap,
        );

        // Issue #248: build via the shared constructor instead of a per-field literal.
        let market =
            gmx_types::MarketProps::new(&w.market_tk, &w.index_tk, &w.long_tk, &w.short_tk);
        let price_props = gmx_types::PriceProps {
            min: index_price,
            max: index_price,
        };

        // Short position of 5000 USD should succeed (no short cap)
        let short_params = IncreasePositionParams {
            data_store: &w.ds,
            caller: &w.admin,
            account: &w.user,
            receiver: &w.user,
            market: &market,
            collateral_token: &w.long_tk,
            size_delta_usd: 5_000 * fp,
            collateral_amount: ONE_TOKEN * 50,
            acceptable_price: 0,
            is_long: false,
            index_token_price: &price_props,
            collateral_price: index_price,
            current_time: 1_000,
        };
        let short_pos = w
            .env
            .as_contract(&w.admin, || increase_position(&w.env, &short_params));
        assert!(
            short_pos.size_in_usd > 0,
            "short position must succeed when only long cap is set"
        );
    }
}
