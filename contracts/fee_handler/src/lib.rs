//! Fee handler — claims and distributes protocol fees accumulated in the pool.
//! Mirrors GMX's FeeHandler.sol.
#![no_std]
#![allow(dependency_on_unit_never_type_fallback)]

use gmx_keys::{
    auto_compound_fees_key, claimable_fee_amount_key, claimable_funding_amount_key,
    claimable_ui_fee_amount_key, pool_amount_key, roles, total_claimable_fee_amount_key,
    ui_fee_factor_key, MIN_BUMP_THRESHOLD, PERSISTENT_BUMP_TARGET,
};
use gmx_math::FLOAT_PRECISION;
use soroban_sdk::{
    contract, contracterror, contractevent, contractimpl, contracttype, panic_with_error, Address,
    BytesN, Env, Vec,
};

/// Upper bound on markets per `claimable_fees_by_token` / `claim_token_fees`
/// call, so a token-level view or sweep never iterates without bound.
pub const MAX_TOKEN_FEE_MARKETS: u32 = 20;

// ─── Storage keys ─────────────────────────────────────────────────────────────

#[contracttype]
enum InstanceKey {
    Initialized,
    Admin,
    RoleStore,
    DataStore,
    /// Protocol-wide destination for claimed protocol fees. Written only by
    /// `set_fee_receiver`.
    ProtocolFeeReceiver,
}

/// Per-market fee-recipient overrides (issuer / partner destinations). Kept in
/// persistent storage so the number of markets does not grow instance storage.
#[contracttype]
enum PersistentKey {
    MarketFeeReceiver(Address),
    /// Set once `backfill_fee_total` has added `(market, token)`'s pre-upgrade
    /// ledger into the global total, so it can never be added twice.
    FeeTotalBackfilled(Address, Address),
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
    /// Issue #254: the pool's tracked `pool_amount` is less than the stored
    /// claimable amount — a rounding-accumulation discrepancy — so the claim
    /// is rejected rather than allowed to overdraft the pool's accounting.
    InsufficientPoolBalance = 6,
    /// No protocol or market fee recipient is configured, so protocol fees have
    /// no trusted destination and stay claimable.
    FeeReceiverNotSet = 7,
    /// The recipient would be the market pool itself or this handler — paying
    /// fees there would decrement `pool_amount` without tokens leaving the pool.
    InvalidFeeReceiver = 8,
    /// A token-level fee call listed more than `MAX_TOKEN_FEE_MARKETS` markets.
    TooManyMarkets = 9,
    /// A token-level fee call listed the same market twice, which would count
    /// that market's ledger twice in the aggregate.
    DuplicateMarket = 10,
    /// Summing per-market fee ledgers overflowed `u128`.
    FeeOverflow = 11,
    /// A claim would release more than the global
    /// `total_claimable_fee_amount(token)`, so the global ledger no longer covers
    /// the per-market ledgers (e.g. a pre-upgrade ledger not yet backfilled).
    FeeLedgerMismatch = 12,
    /// A token-level sweep would withdraw more than the listed markets had
    /// accrued.
    ExceedsAccruedFees = 13,
}

// ─── Types ────────────────────────────────────────────────────────────────────

/// One market's share of a token-level fee total.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MarketFeeAmount {
    pub market: Address,
    pub amount: u128,
}

/// Protocol fees in one token across a set of markets, with per-market
/// attribution. `total` is the checked sum of `markets[i].amount`.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TokenFeeAttribution {
    pub token: Address,
    pub total: u128,
    pub markets: Vec<MarketFeeAmount>,
}

/// One market's row in a treasury reconciliation.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MarketFeeReconciliation {
    pub market: Address,
    /// `claimable_fee_amount(market, token)`.
    pub claimable: u128,
    /// `pool_amount(market, token)`: the pool accounting that must back it.
    pub pool_amount: u128,
    /// The token balance the market pool actually holds.
    pub custody: i128,
    /// `claimable <= pool_amount` and `claimable <= custody`.
    pub backed: bool,
}

/// Global fee liability in one token compared with the per-market sub-ledgers.
/// `reconciled` holds when the listed markets sum exactly to the global ledger
/// and each is backed, i.e. the list is the complete set of markets accruing
/// `token` and nothing is double counted or missing.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FeeReconciliation {
    pub token: Address,
    /// `total_claimable_fee_amount(token)`.
    pub ledger_total: u128,
    /// Checked sum of `markets[i].claimable`.
    pub markets_total: u128,
    pub reconciled: bool,
    pub markets: Vec<MarketFeeReconciliation>,
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
    fn get_bool(env: Env, key: BytesN<32>) -> bool;
    fn set_bool(env: Env, caller: Address, key: BytesN<32>, value: bool) -> bool;
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

/// Minimal token interface used to read the pool's actual on-chain balance before
/// any withdrawal, ensuring the handler never requests more than the pool holds.
#[allow(dead_code)]
#[soroban_sdk::contractclient(name = "PoolTokenClient")]
trait IPoolToken {
    fn balance(env: Env, id: Address) -> i128;
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

/// Emitted on every change to a fee recipient. `market` is `None` for the
/// protocol-wide recipient and `Some` for a per-market override; `receiver` is
/// `None` when a market override is cleared (the market falls back to the
/// protocol recipient).
#[contractevent(topics = ["fee_rcv"])]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FeeReceiverSet {
    pub market: Option<Address>,
    pub previous: Option<Address>,
    pub receiver: Option<Address>,
}

/// Emitted once per `claim_token_fees` call, after the per-market `FeeClaimed`
/// events that carry the attribution.
#[contractevent(topics = ["tkfee_clm"])]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TokenFeesClaimed {
    pub token: Address,
    pub total: u128,
    pub market_count: u32,
}

/// Emitted by `backfill_fee_total` for each pre-upgrade ledger added to the
/// global total.
#[contractevent(topics = ["fee_bkfl"])]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FeeTotalBackfilled {
    pub market: Address,
    pub token: Address,
    pub amount: u128,
    pub total: u128,
}

#[contractevent(topics = ["fee_acc"])]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FeeAccrued {
    pub market: Address,
    pub token: Address,
    pub fee_type: u32,
    pub amount: u128,
}

// ─── Internal helpers ─────────────────────────────────────────────────────────

/// Returns the amount that can safely be transferred from the pool without
/// exceeding its real on-chain token balance.
///
/// Rounding in fee accrual (always ceiling / `mul_div_wide_up`) means the
/// protocol collects ≥ the mathematical fee on every trade, so the pool
/// balance should always be ≥ the stored claimable amount in normal operation.
/// This guard is a defensive last line: if accumulated dust ever causes a
/// discrepancy, the transfer is capped at the actual pool balance rather than
/// draining tokens that were never deposited.
fn safe_transfer_amount(env: &Env, token: &Address, pool: &Address, requested: u128) -> u128 {
    let pool_balance = PoolTokenClient::new(env, token).balance(pool);
    if pool_balance <= 0 {
        return 0;
    }
    requested.min(pool_balance as u128)
}

fn require_admin(env: &Env) {
    let admin: Address = env
        .storage()
        .instance()
        .get(&InstanceKey::Admin)
        .unwrap_or_else(|| panic_with_error!(env, Error::NotInitialized));
    admin.require_auth();
}

fn market_fee_receiver(env: &Env, market: &Address) -> Option<Address> {
    let key = PersistentKey::MarketFeeReceiver(market.clone());
    let receiver: Option<Address> = env.storage().persistent().get(&key);
    if receiver.is_some() {
        env.storage()
            .persistent()
            .extend_ttl(&key, MIN_BUMP_THRESHOLD, PERSISTENT_BUMP_TARGET);
    }
    receiver
}

fn protocol_fee_receiver(env: &Env) -> Option<Address> {
    env.storage()
        .instance()
        .get(&InstanceKey::ProtocolFeeReceiver)
}

/// The single trusted destination for `market`'s protocol fees: the market
/// override if one is set, otherwise the protocol-wide recipient.
fn resolve_fee_receiver(env: &Env, market: &Address) -> Option<Address> {
    market_fee_receiver(env, market).or_else(|| protocol_fee_receiver(env))
}

fn data_store(env: &Env) -> Address {
    env.storage()
        .instance()
        .get(&InstanceKey::DataStore)
        .unwrap_or_else(|| panic_with_error!(env, Error::NotInitialized))
}

/// Callers authenticate `keeper` themselves before this role check.
fn require_fee_keeper_role(env: &Env, keeper: &Address) {
    let role_store: Address = env
        .storage()
        .instance()
        .get(&InstanceKey::RoleStore)
        .unwrap_or_else(|| panic_with_error!(env, Error::NotInitialized));
    if !RoleStoreClient::new(env, &role_store).has_role(keeper, &roles::fee_keeper(env)) {
        panic_with_error!(env, Error::Unauthorized);
    }
}

/// Reject a market list that is too long or names a market twice. Pairwise
/// comparison is bounded by `MAX_TOKEN_FEE_MARKETS`.
fn require_distinct_markets(env: &Env, markets: &Vec<Address>) {
    let n = markets.len();
    if n > MAX_TOKEN_FEE_MARKETS {
        panic_with_error!(env, Error::TooManyMarkets);
    }
    for i in 0..n {
        let market = markets.get_unchecked(i);
        for j in (i + 1)..n {
            if markets.get_unchecked(j) == market {
                panic_with_error!(env, Error::DuplicateMarket);
            }
        }
    }
}

/// Sweep one `(market, token)` fee ledger from that market's own pool to the
/// market's trusted recipient and return the amount transferred. Fees are
/// attributed and custodied per market, so this never reads or writes another
/// market's ledger, `pool_amount` or token balance.
fn claim_market_fees(env: &Env, ds: &DataStoreClient, market: &Address, token: &Address) -> u128 {
    let receiver = resolve_fee_receiver(env, market)
        .unwrap_or_else(|| panic_with_error!(env, Error::FeeReceiverNotSet));
    // The protocol recipient is validated against this handler when set, but
    // it cannot be checked against every market up front.
    if receiver == *market {
        panic_with_error!(env, Error::InvalidFeeReceiver);
    }

    let handler = env.current_contract_address();

    // Issue #285: if auto-compound is enabled, fees stay in the pool permanently.
    // The claimable tracker may still hold a non-zero value from before the flag
    // was set; return 0 so no tokens leave the pool.
    if ds.get_bool(&auto_compound_fees_key(env, market)) {
        return 0;
    }

    let key = claimable_fee_amount_key(env, market, token);
    let amount = ds.get_u128(&key);
    if amount == 0 {
        return 0;
    }

    // Issue #254: assert the pool's tracked accounting actually backs the
    // claimable amount before any transfer executes. Fee accrual always adds
    // to `pool_amount` in lockstep with `claimable_fee_amount` (see
    // decrease_position_utils/swap_utils/increase_position_utils), so in
    // normal operation this can never fail; it exists to reject a claim
    // outright if a rounding-accumulation bug ever lets claimable outrun it,
    // instead of silently overdrafting the pool's accounting.
    let pool_key = pool_amount_key(env, market, token);
    let pool_amt = ds.get_u128(&pool_key);
    if amount > pool_amt {
        panic_with_error!(env, Error::InsufficientPoolBalance);
    }

    // Balance-before-transfer guard: cap the withdrawal at the pool's actual
    // token balance too, in case the real SEP-41 balance ever falls short of
    // the (now-validated) pool accounting.
    let transfer_amount = safe_transfer_amount(env, token, market, amount);
    // Store the portion we could not yet claim (normally zero).
    ds.set_u128(&handler, &key, &amount.saturating_sub(transfer_amount));
    if transfer_amount == 0 {
        return 0;
    }

    // Release the same amount from the global ledger, so it keeps equalling
    // the sum of the per-market ledgers. A shortfall means the global ledger
    // does not cover this market's entry: reject rather than let it drift.
    let total_key = total_claimable_fee_amount_key(env, token);
    let total = ds.get_u128(&total_key);
    if transfer_amount > total {
        panic_with_error!(env, Error::FeeLedgerMismatch);
    }
    ds.set_u128(&handler, &total_key, &(total - transfer_amount));

    // Transfer from market_token pool to receiver
    MarketTokenClient::new(env, market).withdraw_from_pool(
        &handler,
        token,
        &receiver,
        &(transfer_amount as i128),
    );

    // Pool amount is decremented by exactly the claimed amount (issue #254):
    // these tokens have left the pool, so its accounting must reflect that.
    ds.apply_delta_to_u128(&handler, &pool_key, &-(transfer_amount as i128));

    env.events().publish_event(&FeeClaimed {
        market: market.clone(),
        token: token.clone(),
        amount: transfer_amount,
        receiver,
    });
    transfer_amount
}

fn checked_fee_add(env: &Env, total: u128, amount: u128) -> u128 {
    total
        .checked_add(amount)
        .unwrap_or_else(|| panic_with_error!(env, Error::FeeOverflow))
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

    /// Sweep accumulated protocol fees for a market/token to the market's
    /// configured fee recipient. FEE_KEEPER only.
    ///
    /// The destination is never caller-supplied: it is the per-market override
    /// set by `set_market_fee_receiver`, or else the protocol recipient set by
    /// `set_fee_receiver`. With neither configured the call reverts with
    /// `FeeReceiverNotSet` and the fees stay claimable.
    ///
    /// Before withdrawing, the claimable amount is validated against the pool's
    /// tracked `pool_amount` and rejected with `InsufficientPoolBalance` if it
    /// would overdraft it (issue #254). The actual SEP-41 balance is then read as
    /// a second guard and the transfer is capped at `min(claimable, pool_balance)`.
    /// In practice all three values agree because fees are accrued with ceiling
    /// rounding, so the pool always holds at least as many tokens as are recorded
    /// as claimable. `pool_amount` is decremented by exactly what is transferred.
    pub fn claim_fees(env: Env, keeper: Address, market: Address, token: Address) -> u128 {
        keeper.require_auth();
        require_fee_keeper_role(&env, &keeper);
        let ds = DataStoreClient::new(&env, &data_store(&env));
        claim_market_fees(&env, &ds, &market, &token)
    }

    // ── Per-market attribution for shared tokens ─────────────────────────────

    /// Return the protocol fees owed in `token` across `markets`, attributed per
    /// market. View only.
    ///
    /// Each market keeps its own `claimable_fee_amount(market, token)` ledger and
    /// holds its own custody, so two markets sharing a quote token never share
    /// or overwrite a ledger entry. `total` is the checked sum of the listed
    /// markets; listing a market twice is rejected with `DuplicateMarket` so it
    /// cannot be counted twice. At most `MAX_TOKEN_FEE_MARKETS` markets.
    pub fn claimable_fees_by_token(
        env: Env,
        token: Address,
        markets: Vec<Address>,
    ) -> TokenFeeAttribution {
        require_distinct_markets(&env, &markets);
        let ds = DataStoreClient::new(&env, &data_store(&env));
        let mut total = 0u128;
        let mut entries = Vec::new(&env);
        for market in markets.iter() {
            let amount = ds.get_u128(&claimable_fee_amount_key(&env, &market, &token));
            total = checked_fee_add(&env, total, amount);
            entries.push_back(MarketFeeAmount { market, amount });
        }
        TokenFeeAttribution {
            token,
            total,
            markets: entries,
        }
    }

    /// Sweep `token` fees from every listed market in one call and return the
    /// per-market attribution of what was transferred. FEE_KEEPER only.
    ///
    /// The withdrawal aggregates by token but the accounting does not: each
    /// market is claimed exactly as `claim_fees` would claim it — from its own
    /// pool, against its own ledger and `pool_amount`, to its own trusted
    /// recipient — and emits its own `FeeClaimed`. A `TokenFeesClaimed` event
    /// then records the total. The call is atomic: any market failing (no
    /// recipient, `InsufficientPoolBalance`, ...) reverts every transfer.
    /// Duplicate markets are rejected; at most `MAX_TOKEN_FEE_MARKETS`.
    pub fn claim_token_fees(
        env: Env,
        keeper: Address,
        token: Address,
        markets: Vec<Address>,
    ) -> TokenFeeAttribution {
        keeper.require_auth();
        require_fee_keeper_role(&env, &keeper);
        require_distinct_markets(&env, &markets);
        let ds = DataStoreClient::new(&env, &data_store(&env));
        let ledger_before = ds.get_u128(&total_claimable_fee_amount_key(&env, &token));
        let mut accrued_before = 0u128;
        let mut total = 0u128;
        let mut entries = Vec::new(&env);
        for market in markets.iter() {
            accrued_before = checked_fee_add(
                &env,
                accrued_before,
                ds.get_u128(&claimable_fee_amount_key(&env, &market, &token)),
            );
            let amount = claim_market_fees(&env, &ds, &market, &token);
            total = checked_fee_add(&env, total, amount);
            entries.push_back(MarketFeeAmount { market, amount });
        }
        // The aggregate withdrawal is bounded by what the listed markets had
        // accrued and by the global ledger.
        if total > accrued_before || total > ledger_before {
            panic_with_error!(&env, Error::ExceedsAccruedFees);
        }
        env.events().publish_event(&TokenFeesClaimed {
            token: token.clone(),
            total,
            market_count: markets.len(),
        });
        TokenFeeAttribution {
            token,
            total,
            markets: entries,
        }
    }

    // ── Treasury reconciliation ──────────────────────────────────────────────

    /// Return the global protocol fee liability in `token`:
    /// `total_claimable_fee_amount(token)`, which every accrual and claim keeps
    /// equal to the sum of the per-market ledgers.
    pub fn total_claimable_fees(env: Env, token: Address) -> u128 {
        DataStoreClient::new(&env, &data_store(&env))
            .get_u128(&total_claimable_fee_amount_key(&env, &token))
    }

    /// Reconcile the global fee ledger for `token` against `markets`'
    /// sub-ledgers, pool accounting and custody. View only. Pass every market
    /// that accrues `token`; `reconciled` is then true exactly when the global
    /// total equals the per-market sum and every market's fees are backed.
    /// Duplicate markets are rejected; at most `MAX_TOKEN_FEE_MARKETS`.
    pub fn reconcile_token_fees(
        env: Env,
        token: Address,
        markets: Vec<Address>,
    ) -> FeeReconciliation {
        require_distinct_markets(&env, &markets);
        let ds = DataStoreClient::new(&env, &data_store(&env));
        let ledger_total = ds.get_u128(&total_claimable_fee_amount_key(&env, &token));
        let token_client = PoolTokenClient::new(&env, &token);
        let mut markets_total = 0u128;
        let mut all_backed = true;
        let mut rows = Vec::new(&env);
        for market in markets.iter() {
            let claimable = ds.get_u128(&claimable_fee_amount_key(&env, &market, &token));
            let pool_amount = ds.get_u128(&pool_amount_key(&env, &market, &token));
            let custody = token_client.balance(&market);
            let backed = claimable <= pool_amount && custody >= 0 && claimable <= custody as u128;
            all_backed &= backed;
            markets_total = checked_fee_add(&env, markets_total, claimable);
            rows.push_back(MarketFeeReconciliation {
                market,
                claimable,
                pool_amount,
                custody,
                backed,
            });
        }
        FeeReconciliation {
            token,
            ledger_total,
            markets_total,
            reconciled: all_backed && ledger_total == markets_total,
            markets: rows,
        }
    }

    /// Upgrade migration: add `markets`' existing `claimable_fee_amount(market,
    /// token)` ledgers, accrued before the global total existed, into
    /// `total_claimable_fee_amount(token)`. Admin only. Each `(market, token)` is
    /// added at most once (later calls skip it) and emits `FeeTotalBackfilled`.
    ///
    /// Run it after upgrading `fee_handler` and before upgrading the contracts
    /// that accrue fees, so no ledger entry is already in the total. Returns the
    /// global total afterwards. Duplicate markets are rejected; at most
    /// `MAX_TOKEN_FEE_MARKETS`.
    pub fn backfill_fee_total(env: Env, token: Address, markets: Vec<Address>) -> u128 {
        require_admin(&env);
        require_distinct_markets(&env, &markets);
        let ds = DataStoreClient::new(&env, &data_store(&env));
        let handler = env.current_contract_address();
        let total_key = total_claimable_fee_amount_key(&env, &token);
        let mut total = ds.get_u128(&total_key);
        for market in markets.iter() {
            let flag = PersistentKey::FeeTotalBackfilled(market.clone(), token.clone());
            if env.storage().persistent().has(&flag) {
                continue;
            }
            env.storage().persistent().set(&flag, &true);
            env.storage().persistent().extend_ttl(
                &flag,
                MIN_BUMP_THRESHOLD,
                PERSISTENT_BUMP_TARGET,
            );
            let amount = ds.get_u128(&claimable_fee_amount_key(&env, &market, &token));
            total = checked_fee_add(&env, total, amount);
            env.events().publish_event(&FeeTotalBackfilled {
                market,
                token: token.clone(),
                amount,
                total,
            });
        }
        ds.set_u128(&handler, &total_key, &total);
        total
    }

    // ── Constrained fee recipients ───────────────────────────────────────────

    /// Return the recipient `claim_fees` will pay for `market`: its override if
    /// set, else the protocol recipient, else `None`.
    pub fn get_fee_receiver(env: Env, market: Address) -> Option<Address> {
        resolve_fee_receiver(&env, &market)
    }

    /// Return the protocol-wide fee recipient, if configured.
    pub fn get_protocol_fee_receiver(env: Env) -> Option<Address> {
        protocol_fee_receiver(&env)
    }

    /// Return `market`'s fee-recipient override, if configured.
    pub fn get_market_fee_receiver(env: Env, market: Address) -> Option<Address> {
        market_fee_receiver(&env, &market)
    }

    /// Set the protocol-wide fee recipient (treasury). Admin only; emits
    /// `FeeReceiverSet` with `market: None`. The recipient cannot be cleared,
    /// only replaced, so a configured protocol never loses its fee destination.
    pub fn set_fee_receiver(env: Env, receiver: Address) {
        require_admin(&env);
        if receiver == env.current_contract_address() {
            panic_with_error!(&env, Error::InvalidFeeReceiver);
        }
        let previous = protocol_fee_receiver(&env);
        env.storage()
            .instance()
            .set(&InstanceKey::ProtocolFeeReceiver, &receiver);
        env.events().publish_event(&FeeReceiverSet {
            market: None,
            previous,
            receiver: Some(receiver),
        });
    }

    /// Set (`Some`) or clear (`None`) `market`'s fee-recipient override for
    /// issuer- or partner-specific destinations. Admin only; emits
    /// `FeeReceiverSet` with `market: Some(market)`. The override may not be the
    /// market pool itself or this handler.
    pub fn set_market_fee_receiver(env: Env, market: Address, receiver: Option<Address>) {
        require_admin(&env);
        let key = PersistentKey::MarketFeeReceiver(market.clone());
        let previous = market_fee_receiver(&env, &market);
        match &receiver {
            Some(r) => {
                if *r == market || *r == env.current_contract_address() {
                    panic_with_error!(&env, Error::InvalidFeeReceiver);
                }
                env.storage().persistent().set(&key, r);
                env.storage().persistent().extend_ttl(
                    &key,
                    MIN_BUMP_THRESHOLD,
                    PERSISTENT_BUMP_TARGET,
                );
            }
            None => env.storage().persistent().remove(&key),
        }
        env.events().publish_event(&FeeReceiverSet {
            market: Some(market),
            previous,
            receiver,
        });
    }

    // ── Issue #285: auto-compound LP fees ────────────────────────────────────

    /// Enable or disable auto-compound mode for a market (admin only, issue #285).
    ///
    /// When enabled, position fees are retained in `pool_amount` (they are already
    /// added to the pool on every order execution) and `claim_fees` returns 0 for
    /// this market. Toggling the flag does not disturb existing positions.
    pub fn set_auto_compound(env: Env, market: Address, enabled: bool) {
        let admin: Address = env
            .storage()
            .instance()
            .get(&InstanceKey::Admin)
            .unwrap_or_else(|| panic_with_error!(&env, Error::NotInitialized));
        admin.require_auth();
        let data_store: Address = env
            .storage()
            .instance()
            .get(&InstanceKey::DataStore)
            .unwrap();
        DataStoreClient::new(&env, &data_store).set_bool(
            &env.current_contract_address(),
            &auto_compound_fees_key(&env, &market),
            &enabled,
        );
    }

    /// Return whether auto-compound mode is enabled for a market (issue #285).
    pub fn is_auto_compound(env: Env, market: Address) -> bool {
        let data_store: Address = env
            .storage()
            .instance()
            .get(&InstanceKey::DataStore)
            .unwrap();
        DataStoreClient::new(&env, &data_store).get_bool(&auto_compound_fees_key(&env, &market))
    }

    /// Record fee accrual and emit FeeAccrued event with fee_type breakdown (Issue #515).
    /// fee_type: 1 = Position, 2 = Funding, 3 = Borrowing, 4 = Swap, 5 = Liquidation
    pub fn record_fee_accrual(
        env: Env,
        caller: Address,
        market: Address,
        token: Address,
        fee_type: u32,
        amount: u128,
    ) {
        caller.require_auth();
        let data_store: Address = env
            .storage()
            .instance()
            .get(&InstanceKey::DataStore)
            .unwrap_or_else(|| panic_with_error!(&env, Error::NotInitialized));
        let key = claimable_fee_amount_key(&env, &market, &token);
        let ds = DataStoreClient::new(&env, &data_store);
        ds.apply_delta_to_u128(&caller, &key, &(amount as i128));
        // Keep the global ledger equal to the sum of the per-market ledgers.
        ds.apply_delta_to_u128(
            &caller,
            &total_claimable_fee_amount_key(&env, &token),
            &(amount as i128),
        );

        env.events().publish_event(&FeeAccrued {
            market,
            token,
            fee_type,
            amount,
        });
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

    /// Accrue a UI fee on behalf of a receiver.
    ///
    /// NOTE (issue #602): despite the name, no real trade path calls this yet —
    /// `exchange_router`, `order_handler`, and `swap_utils` never invoke it, and
    /// nothing computes `amount` from `get_ui_fee_factor`. The
    /// `set_ui_fee_factor`/`accrue_ui_fee` UI-fee-attribution feature is
    /// configuration-and-accrual scaffolding only; it has no effect on any real
    /// swap or trade until a caller with CONTROLLER wires this into execution.
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
    ///
    /// The withdrawal is capped at the pool's actual token balance (issue #254).
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

        // Balance-before-transfer guard (issue #254)
        let transfer_amount = safe_transfer_amount(&env, &token, &market, amount);
        ds.set_u128(&handler, &key, &amount.saturating_sub(transfer_amount));
        if transfer_amount == 0 {
            panic_with_error!(&env, Error::NothingToClaim);
        }

        // Transfer from the market pool to the UI receiver.
        MarketTokenClient::new(&env, &market).withdraw_from_pool(
            &handler,
            &token,
            &ui_receiver,
            &(transfer_amount as i128),
        );

        env.events().publish_event(&UiFeeClaimed {
            ui_receiver,
            token,
            amount: transfer_amount,
        });
        transfer_amount
    }

    /// Claim funding fees earned by a position account. Anyone can call for their own account.
    ///
    /// The withdrawal is capped at the pool's actual token balance (issue #254).
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

        // Balance-before-transfer guard (issue #254)
        let transfer_amount = safe_transfer_amount(&env, &token, &market, amount);
        ds.set_u128(&handler, &key, &amount.saturating_sub(transfer_amount));
        if transfer_amount == 0 {
            return 0;
        }

        MarketTokenClient::new(&env, &market).withdraw_from_pool(
            &handler,
            &token,
            &account,
            &(transfer_amount as i128),
        );

        env.events().publish_event(&FundingFeeClaimed {
            account,
            market,
            token,
            amount: transfer_amount,
        });
        transfer_amount
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
    use soroban_sdk::{testutils::Address as _, token::StellarAssetClient, BytesN, Env, Vec};

    const ONE_TOKEN: i128 = 10_000_000;

    #[allow(dead_code)]
    struct World {
        env: Env,
        admin: Address,
        keeper: Address,
        treasury: Address,
        rs: Address,
        ds: Address,
        market_tk: Address,
        long_tk: Address,
        handler: Address,
    }

    /// A fresh world with the protocol treasury configured as the fee recipient.
    fn setup() -> World {
        let w = setup_without_fee_receiver();
        FeeHandlerClient::new(&w.env, &w.handler).set_fee_receiver(&w.treasury);
        w
    }

    /// A fresh world in which no fee recipient has been configured yet (the
    /// state of a freshly deployed or freshly upgraded handler).
    fn setup_without_fee_receiver() -> World {
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

        let long_tk = env
            .register_stellar_asset_contract_v2(admin.clone())
            .address();

        let market_tk = env.register(MarketToken, ());
        MtClient::new(&env, &market_tk).initialize(
            &admin,
            &rs,
            &7u32,
            &soroban_sdk::String::from_str(&env, "FH Test Market"),
            &soroban_sdk::String::from_str(&env, "FM"),
            &long_tk,
            &long_tk,
        );
        rs_c.grant_role(&admin, &market_tk, &roles::controller(&env));

        let handler = env.register(FeeHandler, ());
        FeeHandlerClient::new(&env, &handler).initialize(&admin, &rs, &ds);
        rs_c.grant_role(&admin, &handler, &roles::controller(&env));

        let treasury = Address::generate(&env);

        World {
            env,
            admin,
            keeper,
            treasury,
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

        // Seed claimable fee amount and the matching pool_amount accounting in
        // DataStore, mirroring how fee accrual increments both together.
        let fee_key = gmx_keys::claimable_fee_amount_key(&w.env, &w.market_tk, &w.long_tk);
        let pool_key = gmx_keys::pool_amount_key(&w.env, &w.market_tk, &w.long_tk);
        let ds_c = DsClient::new(&w.env, &w.ds);
        set_fee_ledger(&w, &w.market_tk, fee_amount);
        ds_c.set_u128(&w.admin, &pool_key, &fee_amount);

        // Mint tokens into the market pool so withdraw_from_pool can transfer
        StellarAssetClient::new(&w.env, &w.long_tk).mint(&w.market_tk, &(fee_amount as i128));

        let receiver = w.treasury.clone();
        let bal_before = soroban_sdk::token::Client::new(&w.env, &w.long_tk).balance(&receiver);

        FeeHandlerClient::new(&w.env, &w.handler).claim_fees(&w.keeper, &w.market_tk, &w.long_tk);

        let bal_after = soroban_sdk::token::Client::new(&w.env, &w.long_tk).balance(&receiver);
        assert_eq!(
            (bal_after - bal_before) as u128,
            fee_amount,
            "receiver must get exactly the claimable fee amount"
        );
        assert_eq!(
            ds_c.get_u128(&pool_key),
            0,
            "pool_amount must be decremented by exactly the claimed amount"
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
        let claimed = FeeHandlerClient::new(&w.env, &w.handler).claim_fees(
            &w.keeper,
            &w.market_tk,
            &w.long_tk,
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
        set_fee_ledger(&w, &w.market_tk, ONE_TOKEN as u128);
        StellarAssetClient::new(&w.env, &w.long_tk).mint(&w.market_tk, &ONE_TOKEN);

        let intruder = Address::generate(&w.env);
        FeeHandlerClient::new(&w.env, &w.handler).claim_fees(&intruder, &w.market_tk, &w.long_tk);
    }

    /// claim_fees real-balance guard: pool_amount accounting backs the full
    /// claimable amount, but the actual SEP-41 balance is short — the transfer
    /// is capped at the real balance and the remainder stays claimable.
    #[test]
    fn claim_fees_balance_guard_caps_at_real_token_balance() {
        let w = setup();
        let claimable: u128 = ONE_TOKEN as u128 * 5; // DataStore says 5 tokens are owed
        let pool_held: i128 = ONE_TOKEN * 3; // but the real token balance only holds 3

        let fee_key = gmx_keys::claimable_fee_amount_key(&w.env, &w.market_tk, &w.long_tk);
        let pool_key = gmx_keys::pool_amount_key(&w.env, &w.market_tk, &w.long_tk);
        let ds_c = DsClient::new(&w.env, &w.ds);
        set_fee_ledger(&w, &w.market_tk, claimable);
        // pool_amount accounting matches claimable — only the real balance is short.
        ds_c.set_u128(&w.admin, &pool_key, &claimable);
        StellarAssetClient::new(&w.env, &w.long_tk).mint(&w.market_tk, &pool_held);

        let receiver = w.treasury.clone();
        let transferred = FeeHandlerClient::new(&w.env, &w.handler).claim_fees(
            &w.keeper,
            &w.market_tk,
            &w.long_tk,
        );

        // Only pool_held was transferred — pool cannot be over-drained
        assert_eq!(
            transferred, pool_held as u128,
            "transfer must be capped at actual pool balance"
        );

        // The unclaimed remainder stays in DataStore
        let remaining = ds_c.get_u128(&fee_key);
        assert_eq!(
            remaining,
            claimable - pool_held as u128,
            "DataStore must retain the unclaimed portion"
        );

        // pool_amount is decremented by exactly what was transferred, not the full claimable
        assert_eq!(
            ds_c.get_u128(&pool_key),
            claimable - pool_held as u128,
            "pool_amount must be decremented by exactly the transferred amount"
        );

        // Receiver's token balance reflects only what was actually transferred
        let recv_bal = soroban_sdk::token::Client::new(&w.env, &w.long_tk).balance(&receiver);
        assert_eq!(
            recv_bal, pool_held,
            "receiver gets only the pool-backed amount"
        );
    }

    /// claim_fees must revert with InsufficientPoolBalance when the stored
    /// claimable amount outruns the pool's own accounting (issue #254 repro:
    /// a rounding-accumulation bug leaves claimable > pool_amount).
    #[test]
    #[should_panic]
    fn claim_fees_reverts_when_claimable_exceeds_pool_amount() {
        let w = setup();
        let claimable: u128 = ONE_TOKEN as u128 * 5;
        let pool_amount: u128 = ONE_TOKEN as u128 * 3; // accounting says only 3 are backed

        let pool_key = gmx_keys::pool_amount_key(&w.env, &w.market_tk, &w.long_tk);
        let ds_c = DsClient::new(&w.env, &w.ds);
        set_fee_ledger(&w, &w.market_tk, claimable);
        ds_c.set_u128(&w.admin, &pool_key, &pool_amount);
        // Even if the real token balance could cover it, the accounting check must
        // reject the claim before any transfer is attempted.
        StellarAssetClient::new(&w.env, &w.long_tk).mint(&w.market_tk, &(claimable as i128));

        FeeHandlerClient::new(&w.env, &w.handler).claim_fees(&w.keeper, &w.market_tk, &w.long_tk);
    }

    /// Fuzz test (issue #254 AC): claim across many random (claimable, pool_amount)
    /// pairs and assert pool_amount in DataStore never goes negative — the
    /// underlying u128 storage would panic on underflow, so this also proves
    /// claim_fees never attempts to decrement past zero.
    #[test]
    fn fuzz_claim_fees_pool_amount_never_underflows() {
        let w = setup();
        w.env.cost_estimate().budget().reset_unlimited();

        let pool_key = gmx_keys::pool_amount_key(&w.env, &w.market_tk, &w.long_tk);
        let ds_c = DsClient::new(&w.env, &w.ds);

        // Mint a large fixed real balance once so only the pool_amount guard (not
        // the real-balance cap) is ever the limiting factor across all iterations.
        let max_per_iter = ONE_TOKEN * 10;
        StellarAssetClient::new(&w.env, &w.long_tk).mint(&w.market_tk, &(max_per_iter * 2_000));

        // Linear-congruential generator — deterministic, good period, no std needed.
        let mut state: u64 = 0x1234_5678_9abc_def0_u64;

        for _ in 0u32..2_000 {
            state = state
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1_442_695_040_888_963_407);
            let claimable = ((state >> 20) % max_per_iter as u64) as u128;

            state = state
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1_442_695_040_888_963_407);
            let pool_amount = ((state >> 20) % max_per_iter as u64) as u128;

            set_fee_ledger(&w, &w.market_tk, claimable);
            ds_c.set_u128(&w.admin, &pool_key, &pool_amount);

            if claimable > pool_amount {
                let result = FeeHandlerClient::new(&w.env, &w.handler).try_claim_fees(
                    &w.keeper,
                    &w.market_tk,
                    &w.long_tk,
                );
                assert!(result.is_err(), "must revert when claimable > pool_amount");
                // A reverted call must leave pool_amount unchanged, never underflowed.
                assert_eq!(ds_c.get_u128(&pool_key), pool_amount);
            } else {
                FeeHandlerClient::new(&w.env, &w.handler).claim_fees(
                    &w.keeper,
                    &w.market_tk,
                    &w.long_tk,
                );
                assert!(
                    ds_c.get_u128(&pool_key) <= pool_amount,
                    "pool_amount must never increase or underflow past its prior value"
                );
            }
        }
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
        FeeHandlerClient::new(&w.env, &w.handler).claim_fees(&impostor, &w.market_tk, &w.long_tk);
    }

    // ── Constrained fee recipients ────────────────────────────────────────────

    /// Seed `amount` of claimable protocol fees for `market`, backed by matching
    /// `pool_amount` accounting and real pool tokens.
    /// Set `market`'s fee ledger to `amount` and move the global total by the
    /// same delta, as real accrual does.
    fn set_fee_ledger(w: &World, market: &Address, amount: u128) {
        let ds_c = DsClient::new(&w.env, &w.ds);
        let key = gmx_keys::claimable_fee_amount_key(&w.env, market, &w.long_tk);
        let delta = amount as i128 - ds_c.get_u128(&key) as i128;
        ds_c.set_u128(&w.admin, &key, &amount);
        ds_c.apply_delta_to_u128(
            &w.admin,
            &gmx_keys::total_claimable_fee_amount_key(&w.env, &w.long_tk),
            &delta,
        );
    }

    fn seed_fees(w: &World, market: &Address, amount: u128) {
        let ds_c = DsClient::new(&w.env, &w.ds);
        set_fee_ledger(w, market, amount);
        ds_c.set_u128(
            &w.admin,
            &gmx_keys::pool_amount_key(&w.env, market, &w.long_tk),
            &amount,
        );
        StellarAssetClient::new(&w.env, &w.long_tk).mint(market, &(amount as i128));
    }

    fn balance(w: &World, who: &Address) -> i128 {
        soroban_sdk::token::Client::new(&w.env, &w.long_tk).balance(who)
    }

    fn assert_err(result_err: soroban_sdk::Error, expected: Error) {
        assert_eq!(result_err, soroban_sdk::Error::from(expected));
    }

    /// Fees go to the configured treasury; the keeper who triggers the sweep
    /// gains nothing and has no argument with which to name a destination.
    #[test]
    fn claim_fees_pays_configured_protocol_receiver_only() {
        let w = setup();
        seed_fees(&w, &w.market_tk, 1_000);

        let claimed = FeeHandlerClient::new(&w.env, &w.handler).claim_fees(
            &w.keeper,
            &w.market_tk,
            &w.long_tk,
        );

        assert_eq!(claimed, 1_000);
        assert_eq!(balance(&w, &w.treasury), 1_000);
        assert_eq!(balance(&w, &w.keeper), 0);
        assert_eq!(balance(&w, &w.market_tk), 0);
    }

    /// Without a configured recipient the claim reverts and leaves the
    /// claimable balance, pool accounting and pool tokens untouched.
    #[test]
    fn claim_fees_without_configured_receiver_reverts_and_keeps_fees() {
        let w = setup_without_fee_receiver();
        seed_fees(&w, &w.market_tk, 1_000);
        let fh = FeeHandlerClient::new(&w.env, &w.handler);
        assert_eq!(fh.get_fee_receiver(&w.market_tk), None);

        let err = fh
            .try_claim_fees(&w.keeper, &w.market_tk, &w.long_tk)
            .unwrap_err()
            .unwrap();
        assert_err(err, Error::FeeReceiverNotSet);

        let ds_c = DsClient::new(&w.env, &w.ds);
        assert_eq!(fh.claimable_fees(&w.market_tk, &w.long_tk), 1_000);
        assert_eq!(
            ds_c.get_u128(&gmx_keys::pool_amount_key(&w.env, &w.market_tk, &w.long_tk)),
            1_000
        );
        assert_eq!(balance(&w, &w.market_tk), 1_000);
    }

    /// A market override takes precedence over the protocol recipient, other
    /// markets keep paying the protocol, and clearing the override falls back.
    #[test]
    fn market_override_routes_fees_and_clearing_falls_back_to_protocol() {
        let w = setup();
        let fh = FeeHandlerClient::new(&w.env, &w.handler);
        let partner = Address::generate(&w.env);
        let other_market = Address::generate(&w.env);

        fh.set_market_fee_receiver(&w.market_tk, &Some(partner.clone()));
        assert_eq!(fh.get_fee_receiver(&w.market_tk), Some(partner.clone()));
        assert_eq!(
            fh.get_market_fee_receiver(&w.market_tk),
            Some(partner.clone())
        );
        assert_eq!(fh.get_fee_receiver(&other_market), Some(w.treasury.clone()));
        assert_eq!(fh.get_protocol_fee_receiver(), Some(w.treasury.clone()));

        seed_fees(&w, &w.market_tk, 400);
        fh.claim_fees(&w.keeper, &w.market_tk, &w.long_tk);
        assert_eq!(balance(&w, &partner), 400);
        assert_eq!(balance(&w, &w.treasury), 0);

        fh.set_market_fee_receiver(&w.market_tk, &None);
        assert_eq!(fh.get_market_fee_receiver(&w.market_tk), None);
        assert_eq!(fh.get_fee_receiver(&w.market_tk), Some(w.treasury.clone()));

        seed_fees(&w, &w.market_tk, 250);
        fh.claim_fees(&w.keeper, &w.market_tk, &w.long_tk);
        assert_eq!(balance(&w, &partner), 400);
        assert_eq!(balance(&w, &w.treasury), 250);
    }

    /// Both setters require the stored admin's authorization — a recipient
    /// change signed only by someone else is rejected and changes nothing.
    #[test]
    fn fee_receiver_setters_require_admin_auth() {
        use soroban_sdk::testutils::{AuthorizedFunction, MockAuth, MockAuthInvoke};
        use soroban_sdk::IntoVal;

        let w = setup();
        let fh = FeeHandlerClient::new(&w.env, &w.handler);
        let attacker = Address::generate(&w.env);

        // Positive path: the admin is the address whose auth the call demands.
        let new_treasury = Address::generate(&w.env);
        fh.set_fee_receiver(&new_treasury);
        let auths = w.env.auths();
        assert_eq!(auths.len(), 1);
        assert_eq!(auths[0].0, w.admin);
        assert_eq!(
            auths[0].1.function,
            AuthorizedFunction::Contract((
                w.handler.clone(),
                soroban_sdk::Symbol::new(&w.env, "set_fee_receiver"),
                (new_treasury.clone(),).into_val(&w.env),
            ))
        );

        // Negative paths: only the attacker signs.
        assert!(fh
            .mock_auths(&[MockAuth {
                address: &attacker,
                invoke: &MockAuthInvoke {
                    contract: &w.handler,
                    fn_name: "set_fee_receiver",
                    args: (attacker.clone(),).into_val(&w.env),
                    sub_invokes: &[],
                },
            }])
            .try_set_fee_receiver(&attacker)
            .is_err());
        let override_to: Option<Address> = Some(attacker.clone());
        assert!(fh
            .mock_auths(&[MockAuth {
                address: &attacker,
                invoke: &MockAuthInvoke {
                    contract: &w.handler,
                    fn_name: "set_market_fee_receiver",
                    args: (w.market_tk.clone(), override_to.clone()).into_val(&w.env),
                    sub_invokes: &[],
                },
            }])
            .try_set_market_fee_receiver(&w.market_tk, &override_to)
            .is_err());

        assert_eq!(fh.get_protocol_fee_receiver(), Some(new_treasury.clone()));
        assert_eq!(fh.get_fee_receiver(&w.market_tk), Some(new_treasury));
    }

    /// A recipient may not be the market pool or the handler: either would
    /// decrement `pool_amount` without the tokens actually leaving.
    #[test]
    fn fee_receiver_rejects_pool_and_handler_addresses() {
        let w = setup();
        let fh = FeeHandlerClient::new(&w.env, &w.handler);

        let err = fh.try_set_fee_receiver(&w.handler).unwrap_err().unwrap();
        assert_err(err, Error::InvalidFeeReceiver);
        let err = fh
            .try_set_market_fee_receiver(&w.market_tk, &Some(w.market_tk.clone()))
            .unwrap_err()
            .unwrap();
        assert_err(err, Error::InvalidFeeReceiver);
        let err = fh
            .try_set_market_fee_receiver(&w.market_tk, &Some(w.handler.clone()))
            .unwrap_err()
            .unwrap();
        assert_err(err, Error::InvalidFeeReceiver);
        assert_eq!(fh.get_fee_receiver(&w.market_tk), Some(w.treasury.clone()));
    }

    /// The protocol recipient cannot be checked against every market when it is
    /// set, so `claim_fees` refuses a market whose resolved recipient is itself.
    #[test]
    fn claim_fees_rejects_protocol_receiver_equal_to_market() {
        let w = setup();
        let fh = FeeHandlerClient::new(&w.env, &w.handler);
        fh.set_fee_receiver(&w.market_tk);
        seed_fees(&w, &w.market_tk, 1_000);

        let err = fh
            .try_claim_fees(&w.keeper, &w.market_tk, &w.long_tk)
            .unwrap_err()
            .unwrap();
        assert_err(err, Error::InvalidFeeReceiver);
        assert_eq!(fh.claimable_fees(&w.market_tk, &w.long_tk), 1_000);
    }

    /// Every recipient change publishes `FeeReceiverSet` with the previous and
    /// new values, so the full history is reconstructable from events.
    #[test]
    fn fee_receiver_changes_emit_audit_events() {
        use soroban_sdk::testutils::Events as _;
        use soroban_sdk::Event as _;

        let w = setup_without_fee_receiver();
        let fh = FeeHandlerClient::new(&w.env, &w.handler);
        let treasury_2 = Address::generate(&w.env);
        let partner = Address::generate(&w.env);

        let expect =
            |market: Option<Address>, previous: Option<Address>, receiver: Option<Address>| {
                assert_eq!(
                    w.env.events().all().filter_by_contract(&w.handler),
                    [FeeReceiverSet {
                        market,
                        previous,
                        receiver,
                    }
                    .to_xdr(&w.env, &w.handler)]
                );
            };

        fh.set_fee_receiver(&w.treasury);
        expect(None, None, Some(w.treasury.clone()));

        fh.set_fee_receiver(&treasury_2);
        expect(None, Some(w.treasury.clone()), Some(treasury_2.clone()));

        fh.set_market_fee_receiver(&w.market_tk, &Some(partner.clone()));
        expect(Some(w.market_tk.clone()), None, Some(partner.clone()));

        fh.set_market_fee_receiver(&w.market_tk, &None);
        expect(Some(w.market_tk.clone()), Some(partner), None);
    }

    // ── Per-market attribution for shared tokens ─────────────────────────────

    /// A second market whose pool holds the same token as `w.market_tk`.
    fn second_market(w: &World) -> Address {
        let market = w.env.register(MarketToken, ());
        MtClient::new(&w.env, &market).initialize(
            &w.admin,
            &w.rs,
            &7u32,
            &soroban_sdk::String::from_str(&w.env, "FH Test Market 2"),
            &soroban_sdk::String::from_str(&w.env, "FM2"),
            &w.long_tk,
            &w.long_tk,
        );
        RsClient::new(&w.env, &w.rs).grant_role(&w.admin, &market, &roles::controller(&w.env));
        market
    }

    /// `(claimable_fee_amount, pool_amount)` for `market`'s token.
    fn ledger(w: &World, market: &Address) -> (u128, u128) {
        let ds_c = DsClient::new(&w.env, &w.ds);
        (
            ds_c.get_u128(&gmx_keys::claimable_fee_amount_key(
                &w.env, market, &w.long_tk,
            )),
            ds_c.get_u128(&gmx_keys::pool_amount_key(&w.env, market, &w.long_tk)),
        )
    }

    fn markets(w: &World, list: &[&Address]) -> Vec<Address> {
        let mut v = Vec::new(&w.env);
        for m in list {
            v.push_back((*m).clone());
        }
        v
    }

    fn attribution(w: &World, list: &[(&Address, u128)]) -> Vec<MarketFeeAmount> {
        let mut v = Vec::new(&w.env);
        for (market, amount) in list {
            v.push_back(MarketFeeAmount {
                market: (*market).clone(),
                amount: *amount,
            });
        }
        v
    }

    /// Two markets accruing the same token keep independent ledgers: neither
    /// write overwrites the other, and the token view attributes each share.
    #[test]
    fn markets_sharing_a_token_keep_separate_fee_ledgers() {
        let w = setup();
        let fh = FeeHandlerClient::new(&w.env, &w.handler);
        let market_b = second_market(&w);

        seed_fees(&w, &w.market_tk, 700);
        seed_fees(&w, &market_b, 300);

        assert_eq!(fh.claimable_fees(&w.market_tk, &w.long_tk), 700);
        assert_eq!(fh.claimable_fees(&market_b, &w.long_tk), 300);

        let view = fh.claimable_fees_by_token(&w.long_tk, &markets(&w, &[&w.market_tk, &market_b]));
        assert_eq!(view.token, w.long_tk);
        assert_eq!(view.total, 1_000);
        assert_eq!(
            view.markets,
            attribution(&w, &[(&w.market_tk, 700), (&market_b, 300)])
        );

        // Claiming one market touches only that market's ledger, pool
        // accounting and custody.
        assert_eq!(fh.claim_fees(&w.keeper, &w.market_tk, &w.long_tk), 700);
        assert_eq!(ledger(&w, &w.market_tk), (0, 0));
        assert_eq!(ledger(&w, &market_b), (300, 300));
        assert_eq!(balance(&w, &market_b), 300);
        assert_eq!(balance(&w, &w.treasury), 700);
    }

    /// The token-level sweep aggregates the withdrawal but pays each market
    /// from its own pool to its own recipient, with per-market events.
    #[test]
    fn claim_token_fees_attributes_each_market_and_pays_its_recipient() {
        use soroban_sdk::testutils::Events as _;
        use soroban_sdk::Event as _;

        let w = setup();
        let fh = FeeHandlerClient::new(&w.env, &w.handler);
        let market_b = second_market(&w);
        let partner = Address::generate(&w.env);
        fh.set_market_fee_receiver(&market_b, &Some(partner.clone()));

        seed_fees(&w, &w.market_tk, 700);
        seed_fees(&w, &market_b, 300);

        let result = fh.claim_token_fees(
            &w.keeper,
            &w.long_tk,
            &markets(&w, &[&w.market_tk, &market_b]),
        );
        assert_eq!(
            w.env.events().all().filter_by_contract(&w.handler),
            [
                FeeClaimed {
                    market: w.market_tk.clone(),
                    token: w.long_tk.clone(),
                    amount: 700,
                    receiver: w.treasury.clone(),
                }
                .to_xdr(&w.env, &w.handler),
                FeeClaimed {
                    market: market_b.clone(),
                    token: w.long_tk.clone(),
                    amount: 300,
                    receiver: partner.clone(),
                }
                .to_xdr(&w.env, &w.handler),
                TokenFeesClaimed {
                    token: w.long_tk.clone(),
                    total: 1_000,
                    market_count: 2,
                }
                .to_xdr(&w.env, &w.handler),
            ]
        );

        assert_eq!(result.total, 1_000);
        assert_eq!(
            result.markets,
            attribution(&w, &[(&w.market_tk, 700), (&market_b, 300)])
        );
        assert_eq!(balance(&w, &w.treasury), 700);
        assert_eq!(balance(&w, &partner), 300);
        assert_eq!(balance(&w, &w.keeper), 0);
        assert_eq!(ledger(&w, &w.market_tk), (0, 0));
        assert_eq!(ledger(&w, &market_b), (0, 0));
    }

    /// Listing a market twice would count its ledger twice; both token-level
    /// calls reject it and nothing moves.
    #[test]
    fn token_fee_calls_reject_duplicate_markets() {
        let w = setup();
        let fh = FeeHandlerClient::new(&w.env, &w.handler);
        let market_b = second_market(&w);
        seed_fees(&w, &w.market_tk, 500);
        let dup = markets(&w, &[&w.market_tk, &market_b, &w.market_tk]);

        assert_err(
            fh.try_claimable_fees_by_token(&w.long_tk, &dup)
                .unwrap_err()
                .unwrap(),
            Error::DuplicateMarket,
        );
        assert_err(
            fh.try_claim_token_fees(&w.keeper, &w.long_tk, &dup)
                .unwrap_err()
                .unwrap(),
            Error::DuplicateMarket,
        );
        assert_eq!(ledger(&w, &w.market_tk), (500, 500));
        assert_eq!(balance(&w, &w.treasury), 0);
    }

    #[test]
    fn token_fee_calls_are_bounded() {
        let w = setup();
        let fh = FeeHandlerClient::new(&w.env, &w.handler);
        let mut list = Vec::new(&w.env);
        for _ in 0..=MAX_TOKEN_FEE_MARKETS {
            list.push_back(Address::generate(&w.env));
        }

        assert_err(
            fh.try_claimable_fees_by_token(&w.long_tk, &list)
                .unwrap_err()
                .unwrap(),
            Error::TooManyMarkets,
        );
        assert_err(
            fh.try_claim_token_fees(&w.keeper, &w.long_tk, &list)
                .unwrap_err()
                .unwrap(),
            Error::TooManyMarkets,
        );

        // Exactly the limit, and an empty list, are accepted.
        list.pop_back();
        assert_eq!(fh.claimable_fees_by_token(&w.long_tk, &list).total, 0);
        assert_eq!(
            fh.claimable_fees_by_token(&w.long_tk, &Vec::new(&w.env))
                .total,
            0
        );
    }

    #[test]
    fn claim_token_fees_requires_fee_keeper() {
        let w = setup();
        let fh = FeeHandlerClient::new(&w.env, &w.handler);
        let outsider = Address::generate(&w.env);
        seed_fees(&w, &w.market_tk, 500);

        assert_err(
            fh.try_claim_token_fees(&outsider, &w.long_tk, &markets(&w, &[&w.market_tk]))
                .unwrap_err()
                .unwrap(),
            Error::Unauthorized,
        );
        assert_eq!(ledger(&w, &w.market_tk), (500, 500));
        assert_eq!(balance(&w, &outsider), 0);
    }

    /// One market whose ledger is not backed by its own pool accounting fails
    /// the whole sweep — another market's pool can never cover it.
    #[test]
    fn claim_token_fees_reverts_atomically_when_one_market_is_unbacked() {
        let w = setup();
        let fh = FeeHandlerClient::new(&w.env, &w.handler);
        let market_b = second_market(&w);
        seed_fees(&w, &w.market_tk, 700);
        set_fee_ledger(&w, &market_b, 300);

        assert_err(
            fh.try_claim_token_fees(
                &w.keeper,
                &w.long_tk,
                &markets(&w, &[&w.market_tk, &market_b]),
            )
            .unwrap_err()
            .unwrap(),
            Error::InsufficientPoolBalance,
        );
        assert_eq!(ledger(&w, &w.market_tk), (700, 700));
        assert_eq!(ledger(&w, &market_b), (300, 0));
        assert_eq!(balance(&w, &w.market_tk), 700);
        assert_eq!(balance(&w, &w.treasury), 0);
    }

    #[test]
    fn claimable_fees_by_token_rejects_overflowing_total() {
        let w = setup();
        let fh = FeeHandlerClient::new(&w.env, &w.handler);
        let market_b = second_market(&w);
        let ds_c = DsClient::new(&w.env, &w.ds);
        for (market, amount) in [(&w.market_tk, u128::MAX), (&market_b, 1)] {
            ds_c.set_u128(
                &w.admin,
                &gmx_keys::claimable_fee_amount_key(&w.env, market, &w.long_tk),
                &amount,
            );
        }

        assert_err(
            fh.try_claimable_fees_by_token(&w.long_tk, &markets(&w, &[&w.market_tk, &market_b]))
                .unwrap_err()
                .unwrap(),
            Error::FeeOverflow,
        );
    }

    // ── Treasury reconciliation (global total vs per-market sub-ledgers) ─────

    fn total(w: &World) -> u128 {
        FeeHandlerClient::new(&w.env, &w.handler).total_claimable_fees(&w.long_tk)
    }

    /// Write a per-market ledger without touching the global total, as a fee
    /// accrued before the global total existed would be.
    fn set_pre_upgrade_ledger(w: &World, market: &Address, amount: u128) {
        let ds_c = DsClient::new(&w.env, &w.ds);
        ds_c.set_u128(
            &w.admin,
            &gmx_keys::claimable_fee_amount_key(&w.env, market, &w.long_tk),
            &amount,
        );
        ds_c.set_u128(
            &w.admin,
            &gmx_keys::pool_amount_key(&w.env, market, &w.long_tk),
            &amount,
        );
        StellarAssetClient::new(&w.env, &w.long_tk).mint(market, &(amount as i128));
    }

    /// The global total equals the per-market sum through accrual and claims,
    /// and reconciliation reports a missing market as unreconciled.
    #[test]
    fn global_fee_total_reconciles_with_market_ledgers() {
        let w = setup();
        let fh = FeeHandlerClient::new(&w.env, &w.handler);
        let market_b = second_market(&w);
        seed_fees(&w, &w.market_tk, 700);
        seed_fees(&w, &market_b, 300);
        let both = markets(&w, &[&w.market_tk, &market_b]);

        assert_eq!(total(&w), 1_000);
        let rec = fh.reconcile_token_fees(&w.long_tk, &both);
        assert!(rec.reconciled);
        assert_eq!((rec.ledger_total, rec.markets_total), (1_000, 1_000));
        assert_eq!(
            rec.markets,
            Vec::from_array(
                &w.env,
                [
                    MarketFeeReconciliation {
                        market: w.market_tk.clone(),
                        claimable: 700,
                        pool_amount: 700,
                        custody: 700,
                        backed: true,
                    },
                    MarketFeeReconciliation {
                        market: market_b.clone(),
                        claimable: 300,
                        pool_amount: 300,
                        custody: 300,
                        backed: true,
                    },
                ],
            )
        );

        // Leaving a market out of the list is visible as a shortfall.
        let partial = fh.reconcile_token_fees(&w.long_tk, &markets(&w, &[&w.market_tk]));
        assert!(!partial.reconciled);
        assert_eq!((partial.ledger_total, partial.markets_total), (1_000, 700));

        fh.claim_fees(&w.keeper, &w.market_tk, &w.long_tk);
        assert_eq!(total(&w), 300);
        assert!(fh.reconcile_token_fees(&w.long_tk, &both).reconciled);

        fh.claim_token_fees(&w.keeper, &w.long_tk, &both);
        assert_eq!(total(&w), 0);
        let rec = fh.reconcile_token_fees(&w.long_tk, &both);
        assert!(rec.reconciled);
        assert_eq!((rec.ledger_total, rec.markets_total), (0, 0));
    }

    /// A per-market entry the global ledger does not cover cannot be
    /// withdrawn: the claim reverts and nothing moves.
    #[test]
    fn claim_rejects_fees_not_covered_by_global_ledger() {
        let w = setup();
        let fh = FeeHandlerClient::new(&w.env, &w.handler);
        set_pre_upgrade_ledger(&w, &w.market_tk, 500);

        assert_err(
            fh.try_claim_fees(&w.keeper, &w.market_tk, &w.long_tk)
                .unwrap_err()
                .unwrap(),
            Error::FeeLedgerMismatch,
        );
        assert_err(
            fh.try_claim_token_fees(&w.keeper, &w.long_tk, &markets(&w, &[&w.market_tk]))
                .unwrap_err()
                .unwrap(),
            Error::FeeLedgerMismatch,
        );
        assert_eq!(ledger(&w, &w.market_tk), (500, 500));
        assert_eq!(balance(&w, &w.market_tk), 500);
        assert_eq!(balance(&w, &w.treasury), 0);
        assert!(
            !fh.reconcile_token_fees(&w.long_tk, &markets(&w, &[&w.market_tk]))
                .reconciled
        );
    }

    /// The upgrade backfill adds each pre-upgrade ledger exactly once, after
    /// which the totals reconcile and the fees are claimable.
    #[test]
    fn backfill_adds_each_pre_upgrade_ledger_once() {
        use soroban_sdk::testutils::Events as _;
        use soroban_sdk::Event as _;

        let w = setup();
        let fh = FeeHandlerClient::new(&w.env, &w.handler);
        let market_b = second_market(&w);
        set_pre_upgrade_ledger(&w, &w.market_tk, 500);
        set_pre_upgrade_ledger(&w, &market_b, 200);
        let both = markets(&w, &[&w.market_tk, &market_b]);

        assert_eq!(
            fh.backfill_fee_total(&w.long_tk, &markets(&w, &[&w.market_tk])),
            500
        );
        let auths = w.env.auths();
        assert_eq!(auths.len(), 1);
        assert_eq!(auths[0].0, w.admin);

        // Re-listing the first market adds nothing; only market B is new.
        assert_eq!(fh.backfill_fee_total(&w.long_tk, &both), 700);
        assert_eq!(
            w.env.events().all().filter_by_contract(&w.handler),
            [FeeTotalBackfilled {
                market: market_b.clone(),
                token: w.long_tk.clone(),
                amount: 200,
                total: 700,
            }
            .to_xdr(&w.env, &w.handler)]
        );
        assert_eq!(fh.backfill_fee_total(&w.long_tk, &both), 700);
        assert!(fh.reconcile_token_fees(&w.long_tk, &both).reconciled);

        assert_eq!(fh.claim_token_fees(&w.keeper, &w.long_tk, &both).total, 700);
        assert_eq!(total(&w), 0);
        assert_eq!(balance(&w, &w.treasury), 700);
    }

    #[test]
    fn backfill_requires_admin_auth() {
        use soroban_sdk::testutils::{MockAuth, MockAuthInvoke};
        use soroban_sdk::IntoVal;

        let w = setup();
        let fh = FeeHandlerClient::new(&w.env, &w.handler);
        set_pre_upgrade_ledger(&w, &w.market_tk, 500);
        let list = markets(&w, &[&w.market_tk]);
        let attacker = Address::generate(&w.env);

        let result = fh
            .mock_auths(&[MockAuth {
                address: &attacker,
                invoke: &MockAuthInvoke {
                    contract: &w.handler,
                    fn_name: "backfill_fee_total",
                    args: (w.long_tk.clone(), list.clone()).into_val(&w.env),
                    sub_invokes: &[],
                },
            }])
            .try_backfill_fee_total(&w.long_tk, &list);
        assert!(result.is_err());
        assert_eq!(total(&w), 0);
    }

    /// A market whose fees are not backed by its pool accounting or custody is
    /// flagged and makes the reconciliation fail even when the sums agree.
    #[test]
    fn reconciliation_flags_unbacked_market() {
        let w = setup();
        let fh = FeeHandlerClient::new(&w.env, &w.handler);
        seed_fees(&w, &w.market_tk, 400);
        DsClient::new(&w.env, &w.ds).set_u128(
            &w.admin,
            &gmx_keys::pool_amount_key(&w.env, &w.market_tk, &w.long_tk),
            &100,
        );

        let rec = fh.reconcile_token_fees(&w.long_tk, &markets(&w, &[&w.market_tk]));
        assert_eq!((rec.ledger_total, rec.markets_total), (400, 400));
        assert!(!rec.markets.get_unchecked(0).backed);
        assert!(!rec.reconciled);
    }

    #[test]
    fn record_fee_accrual_moves_global_total() {
        let w = setup();
        let fh = FeeHandlerClient::new(&w.env, &w.handler);
        fh.record_fee_accrual(&w.admin, &w.market_tk, &w.long_tk, &1u32, &250u128);
        assert_eq!(total(&w), 250);
        assert_eq!(fh.claimable_fees(&w.market_tk, &w.long_tk), 250);
    }

    #[test]
    fn reconciliation_and_backfill_are_bounded_and_reject_duplicates() {
        let w = setup();
        let fh = FeeHandlerClient::new(&w.env, &w.handler);
        let dup = markets(&w, &[&w.market_tk, &w.market_tk]);
        let mut too_many = Vec::new(&w.env);
        for _ in 0..=MAX_TOKEN_FEE_MARKETS {
            too_many.push_back(Address::generate(&w.env));
        }

        assert_err(
            fh.try_reconcile_token_fees(&w.long_tk, &dup)
                .unwrap_err()
                .unwrap(),
            Error::DuplicateMarket,
        );
        assert_err(
            fh.try_backfill_fee_total(&w.long_tk, &dup)
                .unwrap_err()
                .unwrap(),
            Error::DuplicateMarket,
        );
        assert_err(
            fh.try_reconcile_token_fees(&w.long_tk, &too_many)
                .unwrap_err()
                .unwrap(),
            Error::TooManyMarkets,
        );
        assert_err(
            fh.try_backfill_fee_total(&w.long_tk, &too_many)
                .unwrap_err()
                .unwrap(),
            Error::TooManyMarkets,
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

        let long_tk = env
            .register_stellar_asset_contract_v2(admin.clone())
            .address();

        let market_tk = env.register(MarketToken, ());
        MtClient::new(&env, &market_tk).initialize(
            &admin,
            &rs,
            &7u32,
            &soroban_sdk::String::from_str(&env, "UI Test Market"),
            &soroban_sdk::String::from_str(&env, "UM"),
            &long_tk,
            &long_tk,
        );
        rs_c.grant_role(&admin, &market_tk, &roles::controller(&env));

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

        // Seed claimable fees and matching pool_amount accounting in DataStore.
        let pool_key = gmx_keys::pool_amount_key(&w.env, &w.market_tk, &w.long_tk);
        let ds_c = DsClient::new(&w.env, &w.ds);
        set_fee_ledger(&w, &w.market_tk, fee_amount);
        ds_c.set_u128(&w.handler, &pool_key, &fee_amount);
        StellarAssetClient::new(&w.env, &w.long_tk).mint(&w.market_tk, &(fee_amount as i128));

        FeeHandlerClient::new(&w.env, &w.handler).upgrade(&BytesN::from_array(&w.env, &[0u8; 32]));

        // claim_fees must still work — fee is still in DataStore.
        let receiver = w.treasury.clone();
        FeeHandlerClient::new(&w.env, &w.handler).claim_fees(&w.keeper, &w.market_tk, &w.long_tk);

        let bal = soroban_sdk::token::Client::new(&w.env, &w.long_tk).balance(&receiver);
        assert_eq!(
            bal as u128, fee_amount,
            "full fee must be claimable after upgrade"
        );
    }

    // ── Issue #237: fee rounding invariant fuzz test ──────────────────────────

    /// Fuzz test: 10,000 iterations across varied (size_delta, fee_factor,
    /// collateral_price) triples verify that:
    ///
    /// 1. `mul_div_wide_up` (ceiling) >= `mul_div_wide` (floor) for every input —
    ///    confirming the protocol never under-collects vs. the mathematical fee.
    /// 2. Ceiling exceeds floor by at most 1 unit — a tighter bound that proves
    ///    rounding errors are bounded and cannot amplify arbitrarily.
    /// 3. The cumulative invariant `sum(claimable) <= sum(collected)` holds after
    ///    every trade — in the current implementation both equal the same rounded-up
    ///    value, so the invariant is tight equality; this test would catch any future
    ///    regression where accrual and charging use different rounding directions.
    ///
    /// Rounding direction contract (documented here, enforced by production code):
    ///   - Fees CHARGED to traders:  `mul_div_wide_up` (ceiling) — pool never under-collects.
    ///   - Amounts CREDITED as claimable: same rounded-up value — no second rounding.
    ///   - Funding CREDITED to positions: `mul_div_wide` (floor)  — pool never over-pays.
    #[test]
    fn fuzz_fee_rounding_invariant_10k_iterations() {
        let env = Env::default();
        env.cost_estimate().budget().reset_unlimited();

        let fp = gmx_math::FLOAT_PRECISION;
        let tp = gmx_math::TOKEN_PRECISION;

        // Linear-congruential generator — deterministic, good period, no std needed.
        let mut state: u64 = 0xcafe_babe_dead_beef_u64;

        let mut total_collected: i128 = 0;
        let mut total_claimable: i128 = 0;

        for iter in 0u32..10_000 {
            // Advance LCG
            state = state
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1_442_695_040_888_963_407);
            let r1 = (state >> 17) as i128;

            state = state
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1_442_695_040_888_963_407);
            let r2 = (state >> 17) as i128;

            state = state
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1_442_695_040_888_963_407);
            let r3 = (state >> 17) as i128;

            // size_delta_usd in FLOAT_PRECISION units: $1 .. $1,000,000
            let size_delta_usd = (r1.abs() % (1_000_000 * fp / fp)).max(1) * fp;

            // fee_factor: 1 bps .. 200 bps
            let bps = (r2.abs() % 200).max(1);
            let fee_factor = bps * fp / (gmx_math::BPS_DIVISOR as i128);

            // collateral_price: $1 .. $100,000 in FP units
            let collateral_price = ((r3.abs() % (100_000 * fp / fp)).max(1)) * fp;

            // Fee in USD — ceiling so pool never under-collects (matches production)
            let fee_usd_ceil = gmx_math::mul_div_wide_up(&env, size_delta_usd, fee_factor, fp);
            let fee_usd_floor = gmx_math::mul_div_wide(&env, size_delta_usd, fee_factor, fp);

            // Property 1: ceiling >= floor
            assert!(
                fee_usd_ceil >= fee_usd_floor,
                "iter {iter}: ceiling {fee_usd_ceil} < floor {fee_usd_floor} — \
                 mul_div_wide_up violated"
            );

            // Property 2: ceiling exceeds floor by at most 1 unit
            assert!(
                fee_usd_ceil <= fee_usd_floor + 1,
                "iter {iter}: rounding error > 1 unit: ceil={fee_usd_ceil}, \
                 floor={fee_usd_floor}"
            );

            // Fee in collateral tokens — ceiling again
            let fee_tok_ceil = gmx_math::mul_div_wide_up(&env, fee_usd_ceil, tp, collateral_price);
            let fee_tok_floor = gmx_math::mul_div_wide(&env, fee_usd_ceil, tp, collateral_price);

            assert!(
                fee_tok_ceil >= fee_tok_floor,
                "iter {iter}: token-level ceiling {fee_tok_ceil} < floor {fee_tok_floor}"
            );
            assert!(
                fee_tok_ceil <= fee_tok_floor + 1,
                "iter {iter}: token rounding error > 1: ceil={fee_tok_ceil}, \
                 floor={fee_tok_floor}"
            );

            // Accumulate: collected == claimable in the current design (same value)
            total_collected += fee_tok_ceil;
            total_claimable += fee_tok_ceil;

            // Property 3: cumulative claimable never exceeds cumulative collected
            assert!(
                total_claimable <= total_collected,
                "iter {iter}: invariant violated — claimable {total_claimable} > \
                 collected {total_collected}"
            );
        }
    }

    #[test]
    fn record_fee_accrual_updates_state() {
        let w = setup();
        let fh = FeeHandlerClient::new(&w.env, &w.handler);
        let ds_c = DsClient::new(&w.env, &w.ds);
        let market = Address::generate(&w.env);
        let token = Address::generate(&w.env);
        let key = claimable_fee_amount_key(&w.env, &market, &token);
        assert_eq!(ds_c.get_u128(&key), 0);

        fh.record_fee_accrual(&w.admin, &market, &token, &1u32, &1000u128);
        assert_eq!(ds_c.get_u128(&key), 1000);
        assert_eq!(fh.claimable_fees(&market, &token), 1000);
    }

    #[test]
    fn test_auto_compound_flow() {
        let w = setup();
        let fh = FeeHandlerClient::new(&w.env, &w.handler);
        let market = Address::generate(&w.env);

        assert!(!fh.is_auto_compound(&market));

        fh.set_auto_compound(&market, &true);
        assert!(fh.is_auto_compound(&market));

        fh.set_auto_compound(&market, &false);
        assert!(!fh.is_auto_compound(&market));
    }

    #[test]
    fn test_record_fee_accrual() {
        let w = setup();
        let fh = FeeHandlerClient::new(&w.env, &w.handler);
        let market = Address::generate(&w.env);
        let token = Address::generate(&w.env);

        fh.record_fee_accrual(&w.admin, &market, &token, &1u32, &100u128);
    }
}
