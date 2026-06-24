//! Referral storage — on-chain referral code registry and tier management.
//! Mirrors GMX's ReferralStorage.sol.
#![no_std]
#![allow(dependency_on_unit_never_type_fallback)]

use soroban_sdk::{
    contract, contracterror, contractevent, contractimpl, contracttype, panic_with_error, Address,
    Bytes, BytesN, Env,
};

// ─── Constants ────────────────────────────────────────────────────────────────

/// Maximum number of bytes in a referral code.
pub const MAX_REFERRAL_CODE_LENGTH: u32 = 20;

// ─── Storage key types ────────────────────────────────────────────────────────

#[contracttype]
pub enum ReferralKey {
    CodeOwner(Bytes),
    TraderCode(Address),
    ReferrerTier(Address),
    TierConfig(u32),
}

#[contracttype]
enum InstanceKey {
    Initialized,
    Admin,
}

// ─── Config per tier ──────────────────────────────────────────────────────────

#[contracttype]
pub struct TierConfig {
    pub total_rebate_bps: u32, // basis points of position fee paid back to referrer
    pub discount_share_bps: u32, // portion of that rebate forwarded to trader as discount
}

// ─── Events ───────────────────────────────────────────────────────────────────

#[contractevent(topics = ["ref_reg"])]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CodeRegistered {
    pub caller: Address,
    pub code: Bytes,
}

#[contractevent(topics = ["ref_set"])]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TraderCodeSet {
    pub trader: Address,
    pub code: Bytes,
}

#[contractevent(topics = ["ref_xfr"])]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CodeOwnershipTransferred {
    pub code: Bytes,
    pub from: Address,
    pub to: Address,
}

// ─── Errors ───────────────────────────────────────────────────────────────────

#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
#[repr(u32)]
pub enum Error {
    NotInitialized = 1,
    AlreadyInitialized = 2,
    Unauthorized = 3,
    CodeAlreadyTaken = 4,
    CodeNotFound = 5,
    InvalidTier = 6,
    InvalidInput = 7,
    NotCodeOwner = 8,
    CodeTooLong = 9,
    InvalidCodeCharacters = 10,
    EmptyCode = 11,
}

// ─── Internal helpers ─────────────────────────────────────────────────────────

/// Validates that `code` is non-empty, at most `MAX_REFERRAL_CODE_LENGTH` bytes,
/// and consists solely of ASCII alphanumeric characters, hyphens, or underscores
/// (`[A-Za-z0-9_-]`). Panics with the appropriate error on any violation.
fn validate_code(env: &Env, code: &Bytes) {
    let len = code.len();
    if len == 0 {
        panic_with_error!(env, Error::EmptyCode);
    }
    if len > MAX_REFERRAL_CODE_LENGTH {
        panic_with_error!(env, Error::CodeTooLong);
    }
    for i in 0..len {
        if let Some(byte) = code.get(i) {
            match byte {
                b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' => {}
                _ => panic_with_error!(env, Error::InvalidCodeCharacters),
            }
        }
    }
}

// ─── Contract ─────────────────────────────────────────────────────────────────

#[contract]
pub struct ReferralStorage;

#[contractimpl]
impl ReferralStorage {
    pub fn initialize(env: Env, admin: Address) {
        admin.require_auth();
        if env.storage().instance().has(&InstanceKey::Initialized) {
            panic_with_error!(&env, Error::AlreadyInitialized);
        }
        env.storage()
            .instance()
            .set(&InstanceKey::Initialized, &true);
        env.storage().instance().set(&InstanceKey::Admin, &admin);
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

    /// Register a new referral code; caller becomes the owner.
    ///
    /// `code` must be 1–20 bytes of `[A-Za-z0-9_-]`. Reverts with:
    /// - `EmptyCode` if `code` is empty
    /// - `CodeTooLong` if `code.len() > MAX_REFERRAL_CODE_LENGTH`
    /// - `InvalidCodeCharacters` if `code` contains disallowed bytes
    /// - `CodeAlreadyTaken` if the code is already registered
    pub fn register_code(env: Env, caller: Address, code: Bytes) {
        caller.require_auth();
        validate_code(&env, &code);
        let key = ReferralKey::CodeOwner(code.clone());
        if env.storage().persistent().has(&key) {
            panic_with_error!(&env, Error::CodeAlreadyTaken);
        }
        env.storage().persistent().set(&key, &caller);
        env.events().publish_event(&CodeRegistered { caller, code });
    }

    /// Set the referral code for a trader (links them to a referrer).
    pub fn set_trader_referral_code(env: Env, trader: Address, code: Bytes) {
        trader.require_auth();
        // Validate code exists
        if !env
            .storage()
            .persistent()
            .has(&ReferralKey::CodeOwner(code.clone()))
        {
            panic_with_error!(&env, Error::CodeNotFound);
        }
        env.storage()
            .persistent()
            .set(&ReferralKey::TraderCode(trader.clone()), &code);
        env.events().publish_event(&TraderCodeSet { trader, code });
    }

    /// Look up the referral code for a trader, and return the referrer's address.
    pub fn get_trader_referrer(env: Env, trader: Address) -> Option<Address> {
        let code: Bytes = env
            .storage()
            .persistent()
            .get(&ReferralKey::TraderCode(trader))?;
        env.storage()
            .persistent()
            .get(&ReferralKey::CodeOwner(code))
    }

    /// Return the referral code for a trader, or None.
    pub fn get_trader_referral_code(env: Env, trader: Address) -> Option<Bytes> {
        env.storage()
            .persistent()
            .get(&ReferralKey::TraderCode(trader))
    }

    /// Set the tier for a referrer (admin only).
    pub fn set_referrer_tier(env: Env, admin: Address, referrer: Address, tier: u32) {
        admin.require_auth();
        let stored_admin: Address = env
            .storage()
            .instance()
            .get(&InstanceKey::Admin)
            .unwrap_or_else(|| panic_with_error!(&env, Error::NotInitialized));
        if admin != stored_admin {
            panic_with_error!(&env, Error::Unauthorized);
        }
        if tier > 2 {
            panic_with_error!(&env, Error::InvalidTier);
        }
        env.storage()
            .persistent()
            .set(&ReferralKey::ReferrerTier(referrer), &tier);
    }

    /// Configure the rebate/discount parameters for a tier (admin only).
    pub fn set_tier_config(env: Env, admin: Address, tier: u32, config: TierConfig) {
        admin.require_auth();
        let stored_admin: Address = env
            .storage()
            .instance()
            .get(&InstanceKey::Admin)
            .unwrap_or_else(|| panic_with_error!(&env, Error::NotInitialized));
        if admin != stored_admin {
            panic_with_error!(&env, Error::Unauthorized);
        }
        if tier > 2 {
            panic_with_error!(&env, Error::InvalidTier);
        }
        // Validate config parameters
        if config.total_rebate_bps > 10000 || config.discount_share_bps > 10000 {
            panic_with_error!(&env, Error::InvalidInput);
        }
        env.storage()
            .persistent()
            .set(&ReferralKey::TierConfig(tier), &config);
    }

    /// Transfer ownership of a registered referral code to a new address.
    ///
    /// Only the current code owner may call this. Requires auth from `from`.
    /// The new owner (`to`) immediately becomes the code's referrer for fee calculations.
    pub fn transfer_code_ownership(env: Env, from: Address, to: Address, code: Bytes) {
        from.require_auth();
        let key = ReferralKey::CodeOwner(code.clone());
        let current_owner: Address = env
            .storage()
            .persistent()
            .get(&key)
            .unwrap_or_else(|| panic_with_error!(&env, Error::CodeNotFound));
        if current_owner != from {
            panic_with_error!(&env, Error::NotCodeOwner);
        }
        env.storage().persistent().set(&key, &to);
        env.events()
            .publish_event(&CodeOwnershipTransferred { code, from, to });
    }

    /// Return the owner address for a given referral code, or None if unregistered.
    pub fn get_code_owner(env: Env, code: Bytes) -> Option<Address> {
        env.storage()
            .persistent()
            .get(&ReferralKey::CodeOwner(code))
    }

    /// Return the fee discount bps for a trader given their referral code, or 0 if none.
    pub fn get_trader_discount_bps(env: Env, trader: Address) -> u32 {
        let code: Bytes = match env
            .storage()
            .persistent()
            .get(&ReferralKey::TraderCode(trader))
        {
            Some(c) => c,
            None => return 0,
        };
        let referrer: Address = match env
            .storage()
            .persistent()
            .get(&ReferralKey::CodeOwner(code))
        {
            Some(r) => r,
            None => return 0,
        };
        let tier: u32 = env
            .storage()
            .persistent()
            .get(&ReferralKey::ReferrerTier(referrer))
            .unwrap_or(0);
        let config: TierConfig = match env
            .storage()
            .persistent()
            .get(&ReferralKey::TierConfig(tier))
        {
            Some(c) => c,
            None => return 0,
        };
        // discount = total_rebate * discount_share / 10_000
        config.total_rebate_bps * config.discount_share_bps / 10_000
    }
}

// ─── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use soroban_sdk::{testutils::Address as _, Bytes, Env};

    // ─── Helpers ─────────────────────────────────────────────────────────────

    struct World {
        env: Env,
        admin: Address,
        handler: Address,
    }

    fn setup() -> World {
        let env = Env::default();
        env.mock_all_auths();
        let admin = Address::generate(&env);
        let handler = env.register(ReferralStorage, ());
        ReferralStorageClient::new(&env, &handler).initialize(&admin);
        World {
            env,
            admin,
            handler,
        }
    }

    fn client(w: &World) -> ReferralStorageClient<'_> {
        ReferralStorageClient::new(&w.env, &w.handler)
    }

    /// Produces a short, distinct, valid referral code for each seed value.
    fn make_code(env: &Env, seed: u8) -> Bytes {
        let suffix = b'A' + (seed % 26);
        Bytes::from_slice(env, &[b'C', b'O', b'D', b'E', b'_', suffix])
    }

    // ─── Issue #89: tier number bounds ───────────────────────────────────────

    /// Tier 0, 1, 2 are all valid; no panic expected.
    #[test]
    fn set_referrer_tier_valid_tiers_accepted() {
        let w = setup();
        let referrer = Address::generate(&w.env);
        for t in 0u32..=2 {
            client(&w).set_referrer_tier(&w.admin, &referrer, &t);
        }
    }

    /// Tier 3 is out-of-range and must revert with InvalidTier.
    #[test]
    #[should_panic]
    fn set_referrer_tier_tier_3_reverts() {
        let w = setup();
        let referrer = Address::generate(&w.env);
        client(&w).set_referrer_tier(&w.admin, &referrer, &3u32);
    }

    /// Tier 100 is far out-of-range and must revert.
    #[test]
    #[should_panic]
    fn set_referrer_tier_tier_100_reverts() {
        let w = setup();
        let referrer = Address::generate(&w.env);
        client(&w).set_referrer_tier(&w.admin, &referrer, &100u32);
    }

    /// set_tier_config with tier > 2 must revert.
    #[test]
    #[should_panic]
    fn set_tier_config_invalid_tier_reverts() {
        let w = setup();
        let cfg = TierConfig {
            total_rebate_bps: 500,
            discount_share_bps: 5000,
        };
        client(&w).set_tier_config(&w.admin, &3u32, &cfg);
    }

    // ─── Issue #89: rebate bps bounds ────────────────────────────────────────

    /// total_rebate_bps == 10_000 is the maximum; must be accepted.
    #[test]
    fn set_tier_config_max_rebate_bps_accepted() {
        let w = setup();
        let cfg = TierConfig {
            total_rebate_bps: 10_000,
            discount_share_bps: 0,
        };
        client(&w).set_tier_config(&w.admin, &0u32, &cfg);
    }

    /// total_rebate_bps > 10_000 must revert with InvalidInput.
    #[test]
    #[should_panic]
    fn set_tier_config_rebate_bps_overflow_reverts() {
        let w = setup();
        let cfg = TierConfig {
            total_rebate_bps: 10_001,
            discount_share_bps: 0,
        };
        client(&w).set_tier_config(&w.admin, &0u32, &cfg);
    }

    /// discount_share_bps == 10_000 is the maximum; must be accepted.
    #[test]
    fn set_tier_config_max_discount_share_bps_accepted() {
        let w = setup();
        let cfg = TierConfig {
            total_rebate_bps: 0,
            discount_share_bps: 10_000,
        };
        client(&w).set_tier_config(&w.admin, &0u32, &cfg);
    }

    /// discount_share_bps > 10_000 must revert with InvalidInput.
    #[test]
    #[should_panic]
    fn set_tier_config_discount_share_bps_overflow_reverts() {
        let w = setup();
        let cfg = TierConfig {
            total_rebate_bps: 0,
            discount_share_bps: 10_001,
        };
        client(&w).set_tier_config(&w.admin, &0u32, &cfg);
    }

    /// Both fields at maximum must be accepted (10_000, 10_000).
    #[test]
    fn set_tier_config_both_at_max_accepted() {
        let w = setup();
        let cfg = TierConfig {
            total_rebate_bps: 10_000,
            discount_share_bps: 10_000,
        };
        client(&w).set_tier_config(&w.admin, &1u32, &cfg);
    }

    // ─── Issue #89: valid configs persist and are readable ───────────────────

    /// A written tier config is readable back with identical values.
    #[test]
    fn set_tier_config_persists_and_is_readable_via_discount_bps() {
        let w = setup();
        // Set tier 1: 20% total_rebate, 50% discount_share → 10% net discount.
        let cfg = TierConfig {
            total_rebate_bps: 2_000,
            discount_share_bps: 5_000,
        };
        client(&w).set_tier_config(&w.admin, &1u32, &cfg);

        // Wire up a code → referrer → tier 1 path so get_trader_discount_bps resolves it.
        let referrer = Address::generate(&w.env);
        let code = Bytes::from_slice(&w.env, b"REF_007");
        let trader = Address::generate(&w.env);
        client(&w).register_code(&referrer, &code);
        client(&w).set_referrer_tier(&w.admin, &referrer, &1u32);
        client(&w).set_trader_referral_code(&trader, &code);

        let discount = client(&w).get_trader_discount_bps(&trader);
        // Expected: 2_000 * 5_000 / 10_000 = 1_000 bps
        assert_eq!(
            discount, 1_000,
            "net discount must equal total_rebate * discount_share / 10_000"
        );
    }

    /// get_trader_discount_bps returns 0 when the tier has no configured TierConfig.
    #[test]
    fn get_trader_discount_bps_returns_zero_for_unconfigured_tier() {
        let w = setup();
        let referrer = Address::generate(&w.env);
        let code = Bytes::from_slice(&w.env, b"REF_009");
        let trader = Address::generate(&w.env);
        client(&w).register_code(&referrer, &code);
        // Assign tier 2 but do NOT configure TierConfig for tier 2.
        client(&w).set_referrer_tier(&w.admin, &referrer, &2u32);
        client(&w).set_trader_referral_code(&trader, &code);

        let discount = client(&w).get_trader_discount_bps(&trader);
        assert_eq!(discount, 0, "discount must be 0 when TierConfig is absent");
    }

    /// get_trader_discount_bps returns 0 when the trader has no referral code.
    #[test]
    fn get_trader_discount_bps_no_code_returns_zero() {
        let w = setup();
        let trader = Address::generate(&w.env);
        assert_eq!(client(&w).get_trader_discount_bps(&trader), 0);
    }

    /// Tier 0 with zero bps config returns 0 discount (not a panic).
    #[test]
    fn set_tier_config_zero_bps_valid_returns_zero_discount() {
        let w = setup();
        let cfg = TierConfig {
            total_rebate_bps: 0,
            discount_share_bps: 0,
        };
        client(&w).set_tier_config(&w.admin, &0u32, &cfg);

        let referrer = Address::generate(&w.env);
        let code = Bytes::from_slice(&w.env, b"REF_005");
        let trader = Address::generate(&w.env);
        client(&w).register_code(&referrer, &code);
        client(&w).set_referrer_tier(&w.admin, &referrer, &0u32);
        client(&w).set_trader_referral_code(&trader, &code);

        assert_eq!(client(&w).get_trader_discount_bps(&trader), 0);
    }

    // ─── Issue #89: non-admin cannot mutate tier state ───────────────────────

    /// Only the stored admin can call set_tier_config — impostor must revert.
    #[test]
    #[should_panic]
    fn set_tier_config_non_admin_reverts() {
        let w = setup();
        let impostor = Address::generate(&w.env);
        let cfg = TierConfig {
            total_rebate_bps: 100,
            discount_share_bps: 100,
        };
        // Bypass mock_all_auths by not passing the real admin.
        ReferralStorageClient::new(&w.env, &w.handler).set_tier_config(&impostor, &0u32, &cfg);
    }

    // ─── Issue #88: code ownership transfer ──────────────────────────────────

    /// Successful transfer: new owner is stored, old owner removed.
    #[test]
    fn test_transfer_code_ownership_success() {
        let w = setup();
        let alice = Address::generate(&w.env);
        let bob = Address::generate(&w.env);
        let code = make_code(&w.env, 0x01);

        client(&w).register_code(&alice, &code);
        assert_eq!(client(&w).get_code_owner(&code), Some(alice.clone()));

        client(&w).transfer_code_ownership(&alice, &bob, &code);
        assert_eq!(client(&w).get_code_owner(&code), Some(bob));
    }

    /// Non-owner attempting transfer must revert with NotCodeOwner.
    #[test]
    fn test_transfer_code_ownership_non_owner_rejected() {
        let w = setup();
        let alice = Address::generate(&w.env);
        let charlie = Address::generate(&w.env);
        let code = make_code(&w.env, 0x02);

        client(&w).register_code(&alice, &code);

        let result = client(&w).try_transfer_code_ownership(&charlie, &alice, &code);
        assert_eq!(
            result,
            Err(Ok(soroban_sdk::Error::from_contract_error(
                Error::NotCodeOwner as u32
            )))
        );
    }

    /// Transfer on an unregistered code must revert with CodeNotFound.
    #[test]
    fn test_transfer_code_ownership_missing_code_rejected() {
        let w = setup();
        let alice = Address::generate(&w.env);
        let bob = Address::generate(&w.env);
        let code = make_code(&w.env, 0x03);

        let result = client(&w).try_transfer_code_ownership(&alice, &bob, &code);
        assert_eq!(
            result,
            Err(Ok(soroban_sdk::Error::from_contract_error(
                Error::CodeNotFound as u32
            )))
        );
    }

    /// get_code_owner returns None for an unregistered code.
    #[test]
    fn test_get_code_owner_returns_none_for_unregistered() {
        let w = setup();
        let code = make_code(&w.env, 0x04);
        assert_eq!(client(&w).get_code_owner(&code), None);
    }

    /// After a transfer, discount calculation uses the new owner's tier.
    #[test]
    fn test_trader_discount_follows_new_owner_tier() {
        let w = setup();
        let alice = Address::generate(&w.env);
        let bob = Address::generate(&w.env);
        let trader = Address::generate(&w.env);
        let code = make_code(&w.env, 0x05);

        client(&w).set_tier_config(
            &w.admin,
            &0,
            &TierConfig {
                total_rebate_bps: 1000,
                discount_share_bps: 5000,
            },
        );
        client(&w).set_tier_config(
            &w.admin,
            &1,
            &TierConfig {
                total_rebate_bps: 2000,
                discount_share_bps: 5000,
            },
        );

        client(&w).register_code(&alice, &code);
        client(&w).set_trader_referral_code(&trader, &code);

        // After transfer, discount should reflect bob's tier (default 0)
        client(&w).transfer_code_ownership(&alice, &bob, &code);
        let discount = client(&w).get_trader_discount_bps(&trader);
        // tier 0 for bob: 1000 * 5000 / 10_000 = 500
        assert_eq!(discount, 500);
    }

    // ─── Issue #236: referral code length and character set validation ────────

    /// Empty code must revert with EmptyCode.
    #[test]
    fn register_code_empty_reverts() {
        let w = setup();
        let caller = Address::generate(&w.env);
        let code = Bytes::from_slice(&w.env, b"");
        let result = client(&w).try_register_code(&caller, &code);
        assert_eq!(
            result,
            Err(Ok(soroban_sdk::Error::from_contract_error(
                Error::EmptyCode as u32
            )))
        );
    }

    /// Code longer than MAX_REFERRAL_CODE_LENGTH must revert with CodeTooLong.
    #[test]
    fn register_code_too_long_reverts() {
        let w = setup();
        let caller = Address::generate(&w.env);
        // 21 valid ASCII chars — one over the limit
        let code = Bytes::from_slice(&w.env, b"ABCDEFGHIJKLMNOPQRSTU");
        let result = client(&w).try_register_code(&caller, &code);
        assert_eq!(
            result,
            Err(Ok(soroban_sdk::Error::from_contract_error(
                Error::CodeTooLong as u32
            )))
        );
    }

    /// Code with disallowed characters must revert with InvalidCodeCharacters.
    #[test]
    fn register_code_invalid_chars_reverts() {
        let w = setup();
        let caller = Address::generate(&w.env);
        // '@' is not in [A-Za-z0-9_-]
        let code = Bytes::from_slice(&w.env, b"CODE@2024");
        let result = client(&w).try_register_code(&caller, &code);
        assert_eq!(
            result,
            Err(Ok(soroban_sdk::Error::from_contract_error(
                Error::InvalidCodeCharacters as u32
            )))
        );
    }

    /// Code of exactly MAX_REFERRAL_CODE_LENGTH characters must succeed.
    #[test]
    fn register_code_exactly_max_length_succeeds() {
        let w = setup();
        let caller = Address::generate(&w.env);
        // Exactly 20 valid ASCII chars
        let code = Bytes::from_slice(&w.env, b"ABCDEFGHIJKLMNOPQRST");
        client(&w).register_code(&caller, &code);
        assert_eq!(client(&w).get_code_owner(&code), Some(caller));
    }
}
