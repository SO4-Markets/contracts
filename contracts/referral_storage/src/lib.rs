//! Referral storage — on-chain referral code registry and tier management.
//! Mirrors GMX's ReferralStorage.sol.
#![no_std]
#![allow(dependency_on_unit_never_type_fallback)]

use soroban_sdk::{
    contract, contracterror, contractevent, contractimpl, contracttype, panic_with_error,
    Address, BytesN, Env,
};

// ─── Storage key types ────────────────────────────────────────────────────────

#[contracttype]
pub enum ReferralKey {
    CodeOwner(BytesN<32>),
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
    pub total_rebate_bps: u32,    // basis points of position fee paid back to referrer
    pub discount_share_bps: u32, // portion of that rebate forwarded to trader as discount
}

// ─── Events ───────────────────────────────────────────────────────────────────

#[contractevent(topics = ["ref_reg"])]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CodeRegistered {
    pub caller: Address,
    pub code:   BytesN<32>,
}

#[contractevent(topics = ["ref_set"])]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TraderCodeSet {
    pub trader: Address,
    pub code:   BytesN<32>,
}

// ─── Errors ───────────────────────────────────────────────────────────────────

#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
#[repr(u32)]
pub enum Error {
    AlreadyInitialized = 1,
    Unauthorized       = 2,
    CodeAlreadyTaken   = 3,
    CodeNotFound       = 4,
    InvalidTier        = 5,
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
        env.storage().instance().set(&InstanceKey::Initialized, &true);
        env.storage().instance().set(&InstanceKey::Admin, &admin);
    }

    /// Register a new referral code; caller becomes the owner.
    pub fn register_code(env: Env, caller: Address, code: BytesN<32>) {
        caller.require_auth();
        let key = ReferralKey::CodeOwner(code.clone());
        if env.storage().persistent().has(&key) {
            panic_with_error!(&env, Error::CodeAlreadyTaken);
        }
        env.storage().persistent().set(&key, &caller);
        env.events().publish_event(&CodeRegistered { caller, code });
    }

    /// Set the referral code for a trader (links them to a referrer).
    pub fn set_trader_referral_code(env: Env, trader: Address, code: BytesN<32>) {
        trader.require_auth();
        // Validate code exists
        if !env.storage().persistent().has(&ReferralKey::CodeOwner(code.clone())) {
            panic_with_error!(&env, Error::CodeNotFound);
        }
        env.storage().persistent().set(&ReferralKey::TraderCode(trader.clone()), &code);
        env.events().publish_event(&TraderCodeSet { trader, code });
    }

    /// Look up the referral code for a trader, and return the referrer's address.
    pub fn get_trader_referrer(env: Env, trader: Address) -> Option<Address> {
        let code: BytesN<32> = env.storage().persistent()
            .get(&ReferralKey::TraderCode(trader))?;
        env.storage().persistent().get(&ReferralKey::CodeOwner(code))
    }

    /// Return the referral code for a trader, or None.
    pub fn get_trader_referral_code(env: Env, trader: Address) -> Option<BytesN<32>> {
        env.storage().persistent().get(&ReferralKey::TraderCode(trader))
    }

    /// Set the tier for a referrer (admin only).
    pub fn set_referrer_tier(env: Env, admin: Address, referrer: Address, tier: u32) {
        admin.require_auth();
        let stored_admin: Address = env.storage().instance().get(&InstanceKey::Admin).unwrap();
        if admin != stored_admin {
            panic_with_error!(&env, Error::Unauthorized);
        }
        if tier > 2 {
            panic_with_error!(&env, Error::InvalidTier);
        }
        env.storage().persistent().set(&ReferralKey::ReferrerTier(referrer), &tier);
    }

    /// Configure the rebate/discount parameters for a tier (admin only).
    pub fn set_tier_config(env: Env, admin: Address, tier: u32, config: TierConfig) {
        admin.require_auth();
        let stored_admin: Address = env.storage().instance().get(&InstanceKey::Admin).unwrap();
        if admin != stored_admin {
            panic_with_error!(&env, Error::Unauthorized);
        }
        if tier > 2 {
            panic_with_error!(&env, Error::InvalidTier);
        }
        env.storage().persistent().set(&ReferralKey::TierConfig(tier), &config);
    }

    /// Return the fee discount bps for a trader given their referral code, or 0 if none.
    pub fn get_trader_discount_bps(env: Env, trader: Address) -> u32 {
        let code: BytesN<32> = match env.storage().persistent()
            .get(&ReferralKey::TraderCode(trader))
        {
            Some(c) => c,
            None => return 0,
        };
        let referrer: Address = match env.storage().persistent()
            .get(&ReferralKey::CodeOwner(code))
        {
            Some(r) => r,
            None => return 0,
        };
        let tier: u32 = env.storage().persistent()
            .get(&ReferralKey::ReferrerTier(referrer))
            .unwrap_or(0);
        let config: TierConfig = match env.storage().persistent()
            .get(&ReferralKey::TierConfig(tier))
        {
            Some(c) => c,
            None => return 0,
        };
        // discount = total_rebate * discount_share / 10_000
        config.total_rebate_bps * config.discount_share_bps / 10_000
    }
}
