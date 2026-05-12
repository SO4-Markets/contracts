//! Referral storage — on-chain referral code registry and tier management.
//! Mirrors GMX's ReferralStorage.sol.
//!
//! Traders register referral codes; referrers earn a fee rebate share;
//! referred traders get a discount on position fees.
//!
//! Storage layout (all in persistent storage):
//!   code_owner(code: BytesN<32>)          → Address
//!   trader_referral_code(account: Address) → BytesN<32>
//!   referrer_tier(referrer: Address)       → u32  (0, 1, 2)
//!   tier_config(tier: u32)                 → (total_rebate_bps, discount_share_bps)
#![no_std]
#![allow(dependency_on_unit_never_type_fallback)]

use soroban_sdk::{
    contract, contracterror, contractimpl, contracttype, Address, BytesN, Env,
};

// ─── Storage key types ────────────────────────────────────────────────────────

#[contracttype]
pub enum ReferralKey {
    CodeOwner(BytesN<32>),
    TraderCode(Address),
    ReferrerTier(Address),
    TierConfig(u32),
}

// ─── Config per tier ──────────────────────────────────────────────────────────

#[contracttype]
pub struct TierConfig {
    pub total_rebate_bps: u32,    // basis points of position fee paid back to referrer
    pub discount_share_bps: u32, // portion of that rebate forwarded to trader as discount
}

// ─── Storage/admin keys ───────────────────────────────────────────────────────

const ADMIN_KEY: &str = "ADMIN";

// ─── Errors ───────────────────────────────────────────────────────────────────

#[contracterror]
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
    /// One-time setup: store the admin address.
    pub fn initialize(env: Env, admin: Address) {
        // TODO: panic if already initialized (ADMIN_KEY in instance storage)
        //       env.storage().instance().set(&ADMIN_KEY, &admin)
        todo!()
    }

    /// Register a new referral code; caller becomes the owner.
    ///
    /// `code` is an arbitrary BytesN<32> hash of the chosen code string.
    /// Panics if the code is already taken by someone else.
    pub fn register_code(env: Env, caller: Address, code: BytesN<32>) {
        // TODO:
        // 1. caller.require_auth()
        // 2. key = ReferralKey::CodeOwner(code.clone())
        //    if env.storage().persistent().has(&key) → panic Error::CodeAlreadyTaken
        // 3. env.storage().persistent().set(&key, &caller)
        // 4. Emit "referral_code_registered" event
        todo!()
    }

    /// Set the referral code for a trader (links them to a referrer).
    pub fn set_trader_referral_code(env: Env, trader: Address, code: BytesN<32>) {
        // TODO:
        // 1. trader.require_auth()
        // 2. Validate code exists: ReferralKey::CodeOwner(code) must be set
        //    → panic Error::CodeNotFound if not
        // 3. env.storage().persistent().set(&ReferralKey::TraderCode(trader), &code)
        todo!()
    }

    /// Look up the referral code for a trader, and return the referrer's address.
    ///
    /// Returns None if the trader has no referral code, or code has no owner.
    pub fn get_trader_referrer(env: Env, trader: Address) -> Option<Address> {
        // TODO:
        // 1. code = env.storage().persistent().get::<_, BytesN<32>>(&ReferralKey::TraderCode(trader))?
        // 2. owner = env.storage().persistent().get::<_, Address>(&ReferralKey::CodeOwner(code))?
        // 3. Return Some(owner)
        todo!()
    }

    /// Set the tier for a referrer (admin only). Tier 0 = default, higher = better rebates.
    pub fn set_referrer_tier(env: Env, admin: Address, referrer: Address, tier: u32) {
        // TODO:
        // 1. admin.require_auth()
        //    Validate caller is the stored admin
        // 2. if tier > 2 → panic Error::InvalidTier  (support tiers 0, 1, 2)
        // 3. env.storage().persistent().set(&ReferralKey::ReferrerTier(referrer), &tier)
        todo!()
    }

    /// Configure the rebate/discount parameters for a tier (admin only).
    pub fn set_tier_config(env: Env, admin: Address, tier: u32, config: TierConfig) {
        // TODO:
        // 1. admin.require_auth()
        // 2. if tier > 2 → panic Error::InvalidTier
        // 3. env.storage().persistent().set(&ReferralKey::TierConfig(tier), &config)
        todo!()
    }

    /// Return the fee discount bps for a trader given their referral code, or 0 if none.
    ///
    /// Used by position_utils::get_position_fees to apply the discount.
    pub fn get_trader_discount_bps(env: Env, trader: Address) -> u32 {
        // TODO:
        // 1. code = env.storage().persistent().get::<_, BytesN<32>>(&ReferralKey::TraderCode(trader))
        //    → return 0 if None
        // 2. referrer = env.storage().persistent().get::<_, Address>(&ReferralKey::CodeOwner(code))
        //    → return 0 if None
        // 3. tier = env.storage().persistent().get::<_, u32>(&ReferralKey::ReferrerTier(referrer))
        //    .unwrap_or(0)
        // 4. config = env.storage().persistent().get::<_, TierConfig>(&ReferralKey::TierConfig(tier))
        //    → return 0 if not configured
        // 5. discount = config.total_rebate_bps * config.discount_share_bps / 10_000
        //    (discount_share_bps is the portion of the rebate forwarded to the trader)
        // Returns discount in basis points
        todo!()
    }
}
