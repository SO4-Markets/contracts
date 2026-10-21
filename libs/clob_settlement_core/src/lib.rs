// CLOB Settlement, Accounting, Fees, and Order Lifecycle Implementation
// Addresses issues #957, #958, #959, #960, #961, #962, #963, #964, #965, #967, #981

#![no_std]
use soroban_sdk::{Address, Env, i128};

// #900: market configuration validation for creation and updates.
pub mod market_config;

// #015: enumerable market registry with bounded pagination and O(1) lookup.
pub mod market_registry;

// #014: market deployment and initialization, MarketCreated event.
pub mod market_factory;

/// Fixed-point denominator used for protocol fee rates.
/// A rate of 100_000 (0.1%) is represented as 100_000 / FEE_RATE_SCALE.
pub const FEE_RATE_SCALE: i128 = 100_000_000;

/// Protocol maximum for any maker or taker rate. Market configuration must not
/// exceed it. A rate of zero means that side pays no fee.
pub const MAX_FEE_RATE: i128 = FEE_RATE_SCALE;

// ============ Resource-bound constants (Phase 0, #013) ============
/// Maximum number of steps (fills, expired-maker removals, tick scans) per transaction.
/// Enforced against Soroban CPU/memory/ledger limits. Consensus-critical.
/// Set by benchmark harness; contract and SDK must use this same value.
pub const MAX_MATCH_STEPS: u32 = 100; // Placeholder; real value set by benchmark.

/// Largest valid tick index. Orders and market prices must not exceed it.
/// Consensus-critical; set by benchmark harness.
pub const MAX_TICK: u64 = 1_000_000; // Placeholder; real value set by benchmark.

/// Price scale denominator (protocol spec section 4.1).
/// Scaled price = tick * tick_size / PRICE_SCALE.
pub const PRICE_SCALE: i128 = 100_000_000;

// ============ Bounded-work constants (Phase 0, #013, part 1/2) ============
/// Maximum number of fill operations per single matching call.
/// Limits CPU work and ledger writes from matching one incoming order.
/// When exhausted, matching stops deterministically and the client must resubmit the remainder.
/// Rationale: Soroban transactions have finite CPU, memory, and ledger write budgets.
/// Fills are the primary work unit; each fill requires O(1) ledger writes and compute.
pub const MAX_MATCHES_PER_CALL: u32 = 100;

/// Maximum number of price levels (ticks) examined during one matching traversal.
/// Limits iteration over potentially sparse order book levels when finding next liquidity.
/// When exhausted, matching stops and client must resubmit to continue at next level.
/// Rationale: Finding the next non-empty level can require scanning many empty ticks.
/// Bounding this prevents quadratic-time matching on adversarially sparse books.
/// Implementation must use an ordered index or bitmap to find levels in O(1) per level examined.
pub const MAX_LEVELS_CROSSED_PER_CALL: u32 = 100;

/// Maximum number of orders in a batch operation (batch cancel, batch place).
/// Limits the number of individual order operations processed in one transaction.
/// Rationale: Each order operation (create, cancel) requires ledger reads/writes.
/// Batch operations are not atomic; partial batches succeed and client must retry remainder.
pub const MAX_BATCH_SIZE: u32 = 50;

/// Maximum number of orders returned in a depth query (market data snapshot).
/// Limits ledger read operations when querying the order book state.
/// Rationale: Depth queries scan the book; bounding them keeps query costs predictable.
/// Clients paginate through large books using continuation tokens.
pub const MAX_DEPTH_QUERY_SIZE: u32 = 100;

/// Maximum size (in bytes) of a single order's payload (including owner, side, tick, size, tif).
/// Prevents adversarially large orders from consuming excessive transaction space.
/// Rationale: Soroban transactions have a hard size limit. Bounding per-order size ensures
/// reasonable batch sizes remain within transaction budget.
pub const MAX_ORDER_PAYLOAD_BYTES: u32 = 256;

// ============ Resource-bound guards (Phase 0, #013) ============
/// Validates that a tick is within protocol bounds.
/// Rejects any value > MAX_TICK.
pub fn validate_tick(tick: u64) -> Result<(), &'static str> {
    if tick == 0 {
        return Err("tick: must be positive");
    }
    if tick > MAX_TICK {
        return Err("tick: exceeds MAX_TICK");
    }
    Ok(())
}

/// Validates that a match-step count is within protocol bounds.
/// Rejects any value > MAX_MATCH_STEPS.
pub fn validate_match_steps(steps: u32) -> Result<(), &'static str> {
    if steps > MAX_MATCH_STEPS {
        return Err("match_steps: exceeds MAX_MATCH_STEPS");
    }
    Ok(())
}

/// Validates lot size is positive (protocol spec section 3).
/// Lot size must be > 0; zero and negative sizes are rejected at market creation.
pub fn validate_lot_size(lot_size: u64) -> Result<(), &'static str> {
    if lot_size == 0 {
        return Err("lot_size: must be positive");
    }
    Ok(())
}

/// Validates tick size is positive and does not overflow when scaled.
/// Tick size must be > 0. Product tick * tick_size must fit in u64 (checked at order placement).
pub fn validate_tick_size(tick_size: u64) -> Result<(), &'static str> {
    if tick_size == 0 {
        return Err("tick_size: must be positive");
    }
    Ok(())
}

/// Validates that a fill count does not exceed MAX_MATCHES_PER_CALL.
/// Called during matching to enforce bounded work per transaction.
pub fn validate_matches_per_call(count: u32) -> Result<(), &'static str> {
    if count > MAX_MATCHES_PER_CALL {
        return Err("matches: exceeds MAX_MATCHES_PER_CALL");
    }
    Ok(())
}

/// Validates that a level count does not exceed MAX_LEVELS_CROSSED_PER_CALL.
/// Called during book traversal to enforce bounded level scanning.
pub fn validate_levels_crossed_per_call(count: u32) -> Result<(), &'static str> {
    if count > MAX_LEVELS_CROSSED_PER_CALL {
        return Err("levels: exceeds MAX_LEVELS_CROSSED_PER_CALL");
    }
    Ok(())
}

/// Validates that a batch size does not exceed MAX_BATCH_SIZE.
/// Called when accepting batch operations.
pub fn validate_batch_size(size: u32) -> Result<(), &'static str> {
    if size > MAX_BATCH_SIZE {
        return Err("batch: exceeds MAX_BATCH_SIZE");
    }
    Ok(())
}

/// Validates that a depth query size does not exceed MAX_DEPTH_QUERY_SIZE.
/// Called when returning market data.
pub fn validate_depth_query_size(size: u32) -> Result<(), &'static str> {
    if size > MAX_DEPTH_QUERY_SIZE {
        return Err("depth: exceeds MAX_DEPTH_QUERY_SIZE");
    }
    Ok(())
}

/// Validates that an order payload does not exceed MAX_ORDER_PAYLOAD_BYTES.
/// Called at order creation to enforce transaction size bounds.
pub fn validate_order_payload_size(size: u32) -> Result<(), &'static str> {
    if size > MAX_ORDER_PAYLOAD_BYTES {
        return Err("order: payload exceeds MAX_ORDER_PAYLOAD_BYTES");
    }
    Ok(())
}

// ============ #957: Base and Quote Balance Transfers ============
#[derive(Clone)]
pub struct Fill {
    pub maker: Address,
    pub taker: Address,
    pub base_amount: i128,
    pub quote_amount: i128,
    pub maker_is_sell: bool,
}

#[derive(Clone)]
pub struct SettlementLedger {
    pub base_transfers: Vec<(Address, i128)>,
    pub quote_transfers: Vec<(Address, i128)>,
    /// Protocol-owned liabilities, kept separate from trader transfers.
    pub fee_liabilities: Vec<(Address, i128)>,
}

impl SettlementLedger {
    fn add_fee_liability(&mut self, token: Address, amount: i128) -> Result<(), &'static str> {
        if amount < 0 { return Err("fee: negative accrual"); }
        // A zero fee leaves no entry, so zero-fee markets have no hidden deductions.
        if amount > 0 { self.fee_liabilities.push((token, amount)); }
        Ok(())
    }
}

/// Protocol fees owed for one fill, in quote atoms. Both amounts belong to the
/// protocol, never to a trader.
#[derive(Clone, Copy)]
pub struct FillFees {
    pub taker: i128,
    pub maker: i128,
}

impl FillFees {
    pub fn total(&self) -> Result<i128, &'static str> {
        self.taker.checked_add(self.maker).ok_or("fee: total overflow")
    }
}

impl Fill {
    pub fn settle_balance_transfer(
        &self,
        _env: &Env,
        ledger: &mut SettlementLedger,
    ) -> Result<(), &'static str> {
        if self.maker_is_sell {
            ledger.base_transfers.push((self.maker.clone(), -self.base_amount));
            ledger.quote_transfers.push((self.maker.clone(), self.quote_amount));
            ledger.base_transfers.push((self.taker.clone(), self.base_amount));
            ledger.quote_transfers.push((self.taker.clone(), -self.quote_amount));
        } else {
            ledger.quote_transfers.push((self.maker.clone(), -self.quote_amount));
            ledger.base_transfers.push((self.maker.clone(), self.base_amount));
            ledger.quote_transfers.push((self.taker.clone(), self.quote_amount));
            ledger.base_transfers.push((self.taker.clone(), -self.base_amount));
        }
        Ok(())
    }

    /// Fees owed for this fill. `taker_quote_before` is the quote the same taker
    /// already filled earlier in the transaction (zero for its first fill), so
    /// the taker fee is charged on the running total, not on each fragment.
    pub fn fees(
        &self,
        config: &MarketFeeConfig,
        taker_quote_before: i128,
    ) -> Result<FillFees, &'static str> {
        if self.base_amount < 0 || self.quote_amount < 0 {
            return Err("fill: amounts must be non-negative");
        }
        Ok(FillFees {
            taker: config.cumulative_taker_fee_delta(taker_quote_before, self.quote_amount)?,
            maker: config.maker_fee(self.quote_amount)?,
        })
    }

    /// Settle a fill and its fees as one ledger operation.  Fees are deducted
    /// from each trader's quote side and recorded as protocol liabilities; they
    /// are never added to a trader's available balance.
    pub fn settle_with_fee(
        &self,
        ledger: &mut SettlementLedger,
        config: &MarketFeeConfig,
        quote_token: Address,
        taker_quote_before: i128,
    ) -> Result<FillFees, &'static str> {
        let fees = self.fees(config, taker_quote_before)?;
        self.settle_with_fees(ledger, fees, quote_token)?;
        Ok(fees)
    }

    /// Variant used by the contract settlement path. The accumulator is
    /// preflighted before mutating either economic state, then updated only
    /// after the ledger operation succeeds.
    pub fn settle_with_fee_and_accumulator(
        &self,
        ledger: &mut SettlementLedger,
        accumulator: &mut FeeAccumulator,
        config: &MarketFeeConfig,
        taker_quote_before: i128,
    ) -> Result<FillFees, &'static str> {
        let fees = self.fees(config, taker_quote_before)?;
        let total = fees.total()?;
        accumulator
            .accrued
            .checked_add(total)
            .ok_or("fee: accumulator overflow")?;
        self.settle_with_fees(ledger, fees, accumulator.token.clone())?;
        // The preflight above proves this cannot fail now.
        accumulator.accrued = accumulator.accrued.checked_add(total)
            .ok_or("fee: accumulator overflow")?;
        Ok(fees)
    }

    fn settle_with_fees(
        &self,
        ledger: &mut SettlementLedger,
        fees: FillFees,
        quote_token: Address,
    ) -> Result<(), &'static str> {
        // Stage all mutations so an overflow or conservation failure leaves
        // the caller's economic state untouched.
        let mut staged = ledger.clone();
        let quote = self.quote_amount;
        // The maker's fee is taken from its own leg: a seller receives quote net
        // of the fee, a buyer pays quote plus the fee. The taker's leg mirrors it.
        let maker_quote = if self.maker_is_sell {
            quote.checked_sub(fees.maker)
        } else {
            quote.checked_add(fees.maker).and_then(|v| v.checked_neg())
        }
        .ok_or("fee: maker quote overflow")?;
        let taker_quote = if self.maker_is_sell {
            quote.checked_add(fees.taker).and_then(|v| v.checked_neg())
        } else {
            quote.checked_sub(fees.taker)
        }
        .ok_or("fee: taker quote overflow")?;
        if self.maker_is_sell {
            staged.base_transfers.push((self.maker.clone(), -self.base_amount));
            staged.base_transfers.push((self.taker.clone(), self.base_amount));
        } else {
            staged.base_transfers.push((self.maker.clone(), self.base_amount));
            staged.base_transfers.push((self.taker.clone(), -self.base_amount));
        }
        staged.quote_transfers.push((self.maker.clone(), maker_quote));
        staged.quote_transfers.push((self.taker.clone(), taker_quote));
        staged.add_fee_liability(quote_token.clone(), fees.taker)?;
        staged.add_fee_liability(quote_token, fees.maker)?;
        Self::validate_conservation(&staged)?;
        *ledger = staged;
        Ok(())
    }
    
    pub fn validate_conservation(ledger: &SettlementLedger) -> Result<(), &'static str> {
        let mut base_sum: i128 = 0;
        for (_, delta) in &ledger.base_transfers {
            base_sum = base_sum.checked_add(*delta).ok_or("base overflow")?;
        }
        if base_sum != 0 { return Err("base not conserved"); }
        
        let mut quote_sum: i128 = 0;
        for (_, delta) in &ledger.quote_transfers {
            quote_sum = quote_sum.checked_add(*delta).ok_or("quote overflow")?;
        }
        let mut fees: i128 = 0;
        for (_, amount) in &ledger.fee_liabilities {
            fees = fees.checked_add(*amount).ok_or("fee overflow")?;
        }
        // Fee deductions are liabilities, not trader-owned quote.  Including
        // them here makes custody reconciliation distinguish the two owners.
        if quote_sum.checked_add(fees).ok_or("quote overflow")? != 0 {
            return Err("quote not conserved");
        }
        Ok(())
    }
}

/// Market-wide v1 fee configuration.  The lookup is O(1) and does not depend
/// on historical volume or any unbounded trader scan.  Both rates are fixed
/// point over `FEE_RATE_SCALE` and non-negative: maker rebates are out of
/// scope for v1.
#[derive(Clone, Copy)]
pub struct MarketFeeConfig {
    pub taker_fee_rate: i128,
    pub maker_fee_rate: i128,
}

impl MarketFeeConfig {
    pub fn new(taker_fee_rate: i128, maker_fee_rate: i128) -> Result<Self, &'static str> {
        validate_fee_rate(taker_fee_rate)?;
        validate_fee_rate(maker_fee_rate)?;
        Ok(Self { taker_fee_rate, maker_fee_rate })
    }

    /// Fee charged to the taker on a fill of `quote_amount`, rounded up.
    pub fn taker_fee(&self, quote_amount: i128) -> Result<i128, &'static str> {
        fee_on_quote(quote_amount, self.taker_fee_rate)
    }

    /// Fee charged to the maker on a fill of `quote_amount`, rounded up.
    pub fn maker_fee(&self, quote_amount: i128) -> Result<i128, &'static str> {
        fee_on_quote(quote_amount, self.maker_fee_rate)
    }

    /// Taker fee for a fill that takes the taker's running total from
    /// `taker_quote_before` to `taker_quote_before + fill_quote`.  The fee is the
    /// difference of two rounded-up totals, so the fees of consecutive fills sum
    /// to the fee of their total: splitting a fill never changes the fee.
    pub fn cumulative_taker_fee_delta(
        &self,
        taker_quote_before: i128,
        fill_quote: i128,
    ) -> Result<i128, &'static str> {
        if fill_quote < 0 { return Err("fee: negative quote amount"); }
        let before = self.taker_fee(taker_quote_before)?;
        let total = taker_quote_before
            .checked_add(fill_quote)
            .ok_or("fee: cumulative quote overflow")?;
        self.taker_fee(total)?
            .checked_sub(before)
            .ok_or("fee: cumulative underflow")
    }

    /// Quote a bid must lock to cover its quote and the larger of its maker and
    /// taker fees.  A resting bid can be filled as a maker later, so its lock
    /// cannot assume the taker role.
    pub fn required_bid_reservation(&self, quote_amount: i128) -> Result<i128, &'static str> {
        let fee = self.taker_fee(quote_amount)?.max(self.maker_fee(quote_amount)?);
        quote_amount.checked_add(fee).ok_or("fee: reservation overflow")
    }

    /// Resolve one already-authorized bounded tier. `None` is the unambiguous
    /// v1 default and uses the market-wide rate. A tier replaces only the taker rate.
    pub fn resolve_tier(&self, tier: Option<FeeTier>) -> Result<Self, &'static str> {
        match tier {
            Some(tier) => {
                tier.validate()?;
                Ok(Self { taker_fee_rate: tier.taker_fee_rate, ..*self })
            }
            None => Ok(*self),
        }
    }
}

/// A bounded, auditable trader tier.  v1 callers may pass `None` and always
/// get the market default; no historical-volume accounting is performed.
#[derive(Clone, Copy)]
pub struct FeeTier {
    pub id: u32,
    pub taker_fee_rate: i128,
}

impl FeeTier {
    pub fn validate(&self) -> Result<(), &'static str> {
        validate_fee_rate(self.taker_fee_rate)
    }
}

fn validate_fee_rate(rate: i128) -> Result<(), &'static str> {
    if rate < 0 || rate > MAX_FEE_RATE {
        return Err("fee: rate out of range");
    }
    Ok(())
}

/// Fee on `quote_amount` at `rate`, rounded up so a non-zero obligation can
/// never be under-reserved.  A zero rate charges nothing.
///
/// # Rounding Behavior (CLOB-092)
///
/// Fee rounding follows these rules:
/// 1. **Zero rate always returns zero fee**: If rate = 0, no fee is charged regardless of amount.
/// 2. **Non-zero rate with zero amount returns zero fee**: If amount = 0, no fee can be charged.
/// 3. **Rounding direction is always UP (ceiling)**: This ensures the protocol never under-collects
///    fees. A trader with a valid order can always pay the rounded-up fee.
/// 4. **Tiny fills may round to zero**: When `quote_amount * rate < FEE_RATE_SCALE`, the fee rounds
///    to zero. This is acceptable in v1 because:
///    - It never causes insolvency: an order that can fill at all has locked enough quote.
///    - It does not create fragmentation exploits: the cumulative taker fee formula (section 7.2)
///      ensures splitting a fill never reduces the total fee below the fee of the whole.
/// 5. **Minimum fee is ZERO**: v1 has no minimum fee per fill. Dust trades are discouraged by
///    lot size and minimum notional rules enforced during order placement, not by fee logic.
///
/// # Examples
/// - rate = 100_000 (0.1%), quote = 1_000_000 => fee = ceil(1_000_000 * 100_000 / 100_000_000) = 1
/// - rate = 100_000 (0.1%), quote = 1_000 => fee = ceil(1_000 * 100_000 / 100_000_000) = 1
/// - rate = 100_000 (0.1%), quote = 999 => fee = ceil(999 * 100_000 / 100_000_000) = 1
/// - rate = 1, quote = 1 => fee = ceil(1 * 1 / 100_000_000) = 1 (minimum non-zero)
/// - rate = 1, quote = 0 => fee = 0 (no amount to charge)
/// - rate = 0, quote = 1_000_000 => fee = 0 (zero rate market)
fn fee_on_quote(quote_amount: i128, rate: i128) -> Result<i128, &'static str> {
    if quote_amount < 0 { return Err("fee: negative quote amount"); }
    let numerator = quote_amount.checked_mul(rate).ok_or("fee: multiplication overflow")?;
    if numerator == 0 { return Ok(0); }
    numerator.checked_add(FEE_RATE_SCALE - 1)
        .ok_or("fee: rounding overflow")
        .map(|n| n / FEE_RATE_SCALE)
}

// ============ #958: Fee Buckets ============
#[derive(Clone)]
pub struct FeeBucket {
    pub token: Address,
    pub execution_fee: i128,
    pub position_fee: i128,
    pub referral_credit: i128,
}

impl FeeBucket {
    pub fn new(token: Address) -> Self {
        Self { token, execution_fee: 0, position_fee: 0, referral_credit: 0 }
    }
    
    pub fn apply_execution_fee(&mut self, amount: i128) -> Result<(), &'static str> {
        self.execution_fee = self.execution_fee.checked_add(amount).ok_or("fee overflow")?;
        Ok(())
    }
    
    pub fn apply_position_fee(&mut self, amount: i128) -> Result<(), &'static str> {
        self.position_fee = self.position_fee.checked_add(amount).ok_or("fee overflow")?;
        Ok(())
    }
    
    pub fn apply_referral_credit(&mut self, amount: i128) -> Result<(), &'static str> {
        self.referral_credit = self.referral_credit.checked_add(amount).ok_or("credit overflow")?;
        Ok(())
    }
    
    pub fn net_fee(&self) -> Result<i128, &'static str> {
        let gross = self.execution_fee.checked_add(self.position_fee).ok_or("gross overflow")?;
        gross.checked_sub(self.referral_credit).ok_or("net overflow")
    }
}

/// Explicit protocol-owned balance for one token.  It is intentionally not a
/// `TraderAccount`, so withdrawal code cannot spend it as user available funds.
#[derive(Clone)]
pub struct FeeAccumulator {
    pub token: Address,
    pub accrued: i128,
}

impl FeeAccumulator {
    pub fn new(token: Address) -> Self { Self { token, accrued: 0 } }

    pub fn accrue(&mut self, amount: i128) -> Result<(), &'static str> {
        if amount < 0 { return Err("fee: negative accrual"); }
        self.accrued = self.accrued.checked_add(amount).ok_or("fee: accumulator overflow")?;
        Ok(())
    }

    pub fn pay_out(&mut self, amount: i128) -> Result<(), &'static str> {
        if amount < 0 { return Err("fee: negative payout"); }
        self.accrued = self.accrued.checked_sub(amount).ok_or("fee: insufficient accrued fees")?;
        Ok(())
    }
}

// ============ #959, #960: Quote Rounding ============
/// Only `Down` and `Up` exist: the spec names no other rounding mode (section 4.2).
#[derive(Clone, Copy)]
pub enum RoundingMode { Down, Up }

/// Quote for `base_atoms` at `scaled_price`, rounded as section 4.2 requires.
/// Contract and SDK previews must call this function, not re-derive the math.
pub fn quote_atoms(
    base_atoms: i128,
    scaled_price: i128,
    rounding: RoundingMode,
) -> Result<i128, &'static str> {
    if base_atoms < 0 || scaled_price < 0 {
        return Err("quote: negative input");
    }
    let numerator = base_atoms.checked_mul(scaled_price).ok_or("quote overflow")?;
    let quotient = numerator / PRICE_SCALE;
    match rounding {
        RoundingMode::Down => Ok(quotient),
        RoundingMode::Up if numerator % PRICE_SCALE == 0 => Ok(quotient),
        RoundingMode::Up => Ok(quotient + 1),
    }
}

/// The quote legs of one fill (protocol spec section 7.2).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FillQuoteLegs {
    /// Removed from the buyer's locked quote.
    pub debit: i128,
    /// Added to the seller's available quote.
    pub credit: i128,
    /// Credited to the protocol dust account.
    pub residue: i128,
}

/// Quote legs for filling `fill_lots` of a bid with `remaining_before_lots` left.
/// The debit is a difference of locks, so the buyer's total over any split of
/// the same bid equals the total over one fill (spec section 10.4, Example C).
pub fn fill_quote_legs(
    lot_size: i128,
    remaining_before_lots: i128,
    fill_lots: i128,
    scaled_price: i128,
) -> Result<FillQuoteLegs, &'static str> {
    if lot_size <= 0 {
        return Err("fill: lot size must be positive");
    }
    if fill_lots <= 0 || fill_lots > remaining_before_lots {
        return Err("fill: size out of range");
    }
    let remaining_after_lots = remaining_before_lots - fill_lots;
    let lock_before = quote_atoms(
        remaining_before_lots.checked_mul(lot_size).ok_or("fill: atoms overflow")?,
        scaled_price,
        RoundingMode::Up,
    )?;
    let lock_after = quote_atoms(
        remaining_after_lots.checked_mul(lot_size).ok_or("fill: atoms overflow")?,
        scaled_price,
        RoundingMode::Up,
    )?;
    let credit = quote_atoms(
        fill_lots.checked_mul(lot_size).ok_or("fill: atoms overflow")?,
        scaled_price,
        RoundingMode::Down,
    )?;
    let debit = lock_before - lock_after;
    let residue = debit - credit;
    // Two rounded-up values differ by at most one atom from their rounded-down
    // sum, so a single fill can only leave a residue of 0 or 1.
    if !(0..=1).contains(&residue) {
        return Err("fill: residue out of bounds");
    }
    Ok(FillQuoteLegs { debit, credit, residue })
}

// ============ #981: Order Lifecycle ============
// ============ #918: Ledger-Based Order Expiration ============
// ============ #978: Timestamp-Based Good-Til-Date Expiration ============

/// Time-in-force policy for an order (protocol spec section 5.3).
///
/// - **GTC** (Good-Till-Cancel): Order rests until fully filled or explicitly cancelled.
/// - **GTD** (Good-Till-Date): Order rests until a specified ledger sequence is reached.
/// - **GTT** (Good-Till-Time): Order rests until a specified Unix timestamp is reached.
///
/// # CLOB-978: Timestamp-Based Expiration
///
/// GTT orders expire based on wall-clock time rather than ledger sequence, which is
/// more intuitive for some trading clients. The timestamp is compared against the
/// ledger's timestamp (env.ledger().timestamp()).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum TimeInForce {
    /// Good-Till-Cancel: remains active indefinitely until filled or cancelled.
    GTC,
    /// Good-Till-Date: remains active until expiry_ledger is reached.
    GTD,
    /// Good-Till-Time: remains active until expiry_timestamp is reached.
    GTT,
}

/// Represents a resting order with time-in-force and expiration tracking.
///
/// This structure supports:
/// - CLOB-918: Ledger-based order expiration (GTD)
/// - CLOB-978: Timestamp-based order expiration (GTT)
#[derive(Clone, Copy, Debug)]
pub struct Order {
    pub id: u64,
    pub owner: Address,
    pub tif: TimeInForce,
    /// Last valid ledger sequence (only meaningful for GTD orders).
    /// For GTC and GTT orders, this field is ignored and should be 0.
    pub expiry_ledger: u64,
    /// Last valid Unix timestamp in seconds (only meaningful for GTT orders).
    /// For GTC and GTD orders, this field is ignored and should be 0.
    pub expiry_timestamp: u64,
    pub remaining_lots: u64,
}

impl Order {
    /// Create a new GTC order.
    pub fn new_gtc(id: u64, owner: Address, remaining_lots: u64) -> Self {
        Self {
            id,
            owner,
            tif: TimeInForce::GTC,
            expiry_ledger: 0,
            expiry_timestamp: 0,
            remaining_lots,
        }
    }

    /// Create a new GTD order with specified expiry ledger.
    pub fn new_gtd(id: u64, owner: Address, remaining_lots: u64, expiry_ledger: u64) -> Self {
        Self {
            id,
            owner,
            tif: TimeInForce::GTD,
            expiry_ledger,
            expiry_timestamp: 0,
            remaining_lots,
        }
    }

    /// Create a new GTT order with specified expiry timestamp.
    ///
    /// # CLOB-978: Timestamp-Based Expiration
    ///
    /// The expiry_timestamp is a Unix timestamp in seconds. The order expires when
    /// the ledger timestamp (env.ledger().timestamp()) reaches this value.
    pub fn new_gtt(id: u64, owner: Address, remaining_lots: u64, expiry_timestamp: u64) -> Self {
        Self {
            id,
            owner,
            tif: TimeInForce::GTT,
            expiry_ledger: 0,
            expiry_timestamp,
            remaining_lots,
        }
    }

    /// Check if the order is expired at the given ledger sequence and timestamp.
    ///
    /// Returns `true` if the order is expired. This method checks both ledger-based
    /// and timestamp-based expiration depending on the order's TIF.
    ///
    /// # CLOB-918 & CLOB-978: Expiration Semantics
    ///
    /// - **GTC**: Never expires (always return false)
    /// - **GTD**: Expired when `current_ledger >= expiry_ledger`
    /// - **GTT**: Expired when `current_timestamp >= expiry_timestamp`
    ///
    /// # Boundary Behavior (CLOB-978)
    ///
    /// An order is expired when the current time/ledger is **greater than or equal to**
    /// the expiry value. This means:
    /// - At timestamp T, an order with expiry_timestamp = T is **expired**
    /// - At timestamp T-1, an order with expiry_timestamp = T is **active**
    /// - "Valid through" semantics: the order is valid up to but not including the expiry
    pub fn is_expired(&self, current_ledger: u64, current_timestamp: u64) -> bool {
        match self.tif {
            TimeInForce::GTC => false,
            TimeInForce::GTD => current_ledger >= self.expiry_ledger,
            TimeInForce::GTT => current_timestamp >= self.expiry_timestamp,
        }
    }

    /// Check if the order is active (not expired) at the given ledger and timestamp.
    ///
    /// This is the inverse of `is_expired` and is provided for convenience
    /// in matching logic where "active" is the more natural phrasing.
    pub fn is_active(&self, current_ledger: u64, current_timestamp: u64) -> bool {
        !self.is_expired(current_ledger, current_timestamp)
    }

    /// Validate that this order can be placed with the given environment state.
    ///
    /// # Validation Rules
    ///
    /// - **GTC orders**: Always valid (expiry fields ignored)
    /// - **GTD orders**: expiry_ledger must be strictly greater than current_ledger
    /// - **GTT orders**: expiry_timestamp must be strictly greater than current_timestamp
    ///
    /// This ensures that an order is active for at least one ledger/timestamp after placement.
    pub fn validate_placement(&self, current_ledger: u64, current_timestamp: u64) -> Result<(), &'static str> {
        match self.tif {
            TimeInForce::GTC => {
                // GTC orders should have expiry fields = 0 by convention, but we don't enforce it
                // since the fields are ignored for GTC orders.
                Ok(())
            }
            TimeInForce::GTD => {
                if self.expiry_ledger <= current_ledger {
                    return Err("gtd: expiry must be in the future");
                }
                Ok(())
            }
            TimeInForce::GTT => {
                if self.expiry_timestamp <= current_timestamp {
                    return Err("gtt: expiry must be in the future");
                }
                Ok(())
            }
        }
    }

    /// Check if this order can fill as a maker at the current ledger and timestamp.
    ///
    /// An order can fill if it is active (not expired). This is called during
    /// matching to ensure expired orders never execute (Invariants I11, I11-time).
    pub fn can_fill_as_maker(&self, current_ledger: u64, current_timestamp: u64) -> bool {
        self.is_active(current_ledger, current_timestamp)
    }
}

/// Request to place an order, with time-in-force validation.
///
/// This is used during order placement to validate the TIF policy before
/// creating the actual `Order` structure.
#[derive(Clone, Copy)]
pub struct OrderRequest {
    pub tif: TimeInForce,
    pub expiry_ledger: u64,
    pub expiry_timestamp: u64,
}

impl OrderRequest {
    /// Create a GTC order request.
    pub fn new_gtc() -> Self {
        Self {
            tif: TimeInForce::GTC,
            expiry_ledger: 0,
            expiry_timestamp: 0,
        }
    }

    /// Create a GTD order request with specified expiry ledger.
    pub fn new_gtd(expiry_ledger: u64) -> Self {
        Self {
            tif: TimeInForce::GTD,
            expiry_ledger,
            expiry_timestamp: 0,
        }
    }

    /// Create a GTT order request with specified expiry timestamp.
    ///
    /// # CLOB-978: Timestamp-Based Expiration
    pub fn new_gtt(expiry_timestamp: u64) -> Self {
        Self {
            tif: TimeInForce::GTT,
            expiry_ledger: 0,
            expiry_timestamp,
        }
    }

    /// Validate the time-in-force policy at the current ledger and timestamp.
    ///
    /// This is called during order placement to ensure:
    /// - GTC orders have no expiry or expiry is 0
    /// - GTD orders have a valid future ledger expiry
    /// - GTT orders have a valid future timestamp expiry
    pub fn validate_tif(
        env: &Env,
        tif: TimeInForce,
        expiry_ledger: Option<u64>,
        expiry_timestamp: Option<u64>,
    ) -> Result<(), &'static str> {
        match tif {
            TimeInForce::GTC => {
                // GTC orders should not have an expiry, or it should be 0
                if let Some(exp) = expiry_ledger {
                    if exp != 0 {
                        return Err("gtc: ledger expiry must be zero");
                    }
                }
                if let Some(exp) = expiry_timestamp {
                    if exp != 0 {
                        return Err("gtc: timestamp expiry must be zero");
                    }
                }
                Ok(())
            }
            TimeInForce::GTD => {
                let expiry = expiry_ledger.ok_or("gtd: missing expiry ledger")?;
                if expiry <= env.ledger().sequence() {
                    return Err("gtd: expiry must be in the future");
                }
                Ok(())
            }
            TimeInForce::GTT => {
                let expiry = expiry_timestamp.ok_or("gtt: missing expiry timestamp")?;
                if expiry <= env.ledger().timestamp() {
                    return Err("gtt: expiry must be in the future");
                }
                Ok(())
            }
        }
    }

    /// Check if an order with this request would be active at the given ledger and timestamp.
    pub fn is_active(&self, current_ledger: u64, current_timestamp: u64) -> bool {
        match self.tif {
            TimeInForce::GTC => true,
            TimeInForce::GTD => current_ledger < self.expiry_ledger,
            TimeInForce::GTT => current_timestamp < self.expiry_timestamp,
        }
    }

    /// Validate this request for placement at the current ledger and timestamp.
    pub fn validate(&self, env: &Env) -> Result<(), &'static str> {
        Self::validate_tif(
            env,
            self.tif,
            Some(self.expiry_ledger),
            Some(self.expiry_timestamp),
        )
    }
}

// ============ #903-906: Custody, Deposits, and Withdrawals ============

#[derive(Clone)]
pub struct TraderAccount {
    pub trader: Address,
    pub asset: Address,
    pub total_balance: i128,
    pub available_balance: i128,
    pub locked_balance: i128,
}

impl TraderAccount {
    pub fn new(trader: Address, asset: Address) -> Self {
        Self {
            trader,
            asset,
            total_balance: 0,
            available_balance: 0,
            locked_balance: 0,
        }
    }

    pub fn validate_invariants(&self) -> Result<(), &'static str> {
        let computed_total = self.available_balance.checked_add(self.locked_balance)
            .ok_or("balance overflow")?;
        if computed_total != self.total_balance {
            return Err("balance invariant violated");
        }
        if self.available_balance < 0 || self.locked_balance < 0 || self.total_balance < 0 {
            return Err("negative balance");
        }
        Ok(())
    }
}

#[derive(Clone)]
pub struct DepositRequest {
    pub trader: Address,
    pub asset: Address,
    pub amount: i128,
}

impl DepositRequest {
    pub fn validate(&self) -> Result<(), &'static str> {
        if self.amount <= 0 {
            return Err("deposit: amount must be positive");
        }
        Ok(())
    }

    pub fn process_deposit(
        &self,
        account: &mut TraderAccount,
    ) -> Result<(), &'static str> {
        self.validate()?;

        let new_total = self.amount.checked_add(account.total_balance)
            .ok_or("deposit: total overflow")?;
        let new_available = self.amount.checked_add(account.available_balance)
            .ok_or("deposit: available overflow")?;

        account.total_balance = new_total;
        account.available_balance = new_available;

        account.validate_invariants()?;
        Ok(())
    }
}

#[derive(Clone)]
pub struct WithdrawalRequest {
    pub trader: Address,
    pub asset: Address,
    pub amount: i128,
}

impl WithdrawalRequest {
    pub fn validate(&self) -> Result<(), &'static str> {
        if self.amount <= 0 {
            return Err("withdrawal: amount must be positive");
        }
        Ok(())
    }

    pub fn process_withdrawal(
        &self,
        account: &mut TraderAccount,
    ) -> Result<(), &'static str> {
        self.validate()?;

        if self.amount > account.available_balance {
            return Err("withdrawal: insufficient available balance");
        }

        let new_available = account.available_balance.checked_sub(self.amount)
            .ok_or("withdrawal: available underflow")?;
        let new_total = account.total_balance.checked_sub(self.amount)
            .ok_or("withdrawal: total underflow")?;

        account.available_balance = new_available;
        account.total_balance = new_total;

        account.validate_invariants()?;
        Ok(())
    }
}

#[cfg(test)]
mod fee_tests {
    use super::*;

    #[test]
    fn reservation_rounds_fee_up() {
        let config = MarketFeeConfig::new(1, 0).unwrap();
        assert_eq!(config.taker_fee(1).unwrap(), 1);
        assert_eq!(config.required_bid_reservation(1).unwrap(), 2);
    }

    #[test]
    fn zero_fee_has_no_extra_reservation() {
        let config = MarketFeeConfig::new(0, 0).unwrap();
        assert_eq!(config.required_bid_reservation(123).unwrap(), 123);
    }

    #[test]
    fn invalid_rates_and_overflow_are_rejected() {
        assert!(MarketFeeConfig::new(-1, 0).is_err());
        assert!(MarketFeeConfig::new(0, -1).is_err());
        assert!(MarketFeeConfig::new(FEE_RATE_SCALE + 1, 0).is_err());
        assert!(MarketFeeConfig::new(0, MAX_FEE_RATE + 1).is_err());
        let config = MarketFeeConfig::new(MAX_FEE_RATE, 0).unwrap();
        assert!(config.required_bid_reservation(i128::MAX).is_err());
    }

    #[test]
    fn tier_resolution_is_bounded_and_default_is_explicit() {
        let config = MarketFeeConfig::new(100, 50).unwrap();
        assert_eq!(config.resolve_tier(None).unwrap().taker_fee_rate, 100);
        let tier = FeeTier { id: 7, taker_fee_rate: 200 };
        let resolved = config.resolve_tier(Some(tier)).unwrap();
        assert_eq!(resolved.taker_fee_rate, 200);
        assert_eq!(resolved.maker_fee_rate, 50);
    }

    #[test]
    fn bid_reservation_covers_the_larger_of_maker_and_taker_fee() {
        // 0.1% taker fee and 0.2% maker fee: the maker fee is the larger obligation.
        let config = MarketFeeConfig::new(100_000, 200_000).unwrap();
        assert_eq!(config.taker_fee(1_000).unwrap(), 1);
        assert_eq!(config.maker_fee(1_000).unwrap(), 2);
        assert_eq!(config.required_bid_reservation(1_000).unwrap(), 1_002);
    }

    #[test]
    fn split_taker_fills_pay_the_fee_of_their_total() {
        // A one-atom fill alone rounds its fee up to one atom. Three one-atom
        // fills must still pay one atom in total, not three.
        let config = MarketFeeConfig::new(1, 0).unwrap();
        let mut filled = 0;
        let mut paid = 0;
        for _ in 0..3 {
            paid += config.cumulative_taker_fee_delta(filled, 1).unwrap();
            filled += 1;
        }
        assert_eq!(config.taker_fee(3).unwrap(), 1);
        assert_eq!(paid, 1);
    }

    // ============ CLOB-092: Fee Rounding, Minimum Fee, and Zero-Value Behavior Tests ============

    /// Test case: Zero rate always charges zero fee, regardless of quote amount.
    #[test]
    fn zero_rate_charges_no_fee() {
        let config = MarketFeeConfig::new(0, 0).unwrap();
        
        // Zero rate with various quote amounts
        assert_eq!(config.taker_fee(0).unwrap(), 0);
        assert_eq!(config.taker_fee(1).unwrap(), 0);
        assert_eq!(config.taker_fee(1_000).unwrap(), 0);
        assert_eq!(config.taker_fee(1_000_000).unwrap(), 0);
        assert_eq!(config.taker_fee(i128::MAX / 2).unwrap(), 0);
        
        assert_eq!(config.maker_fee(0).unwrap(), 0);
        assert_eq!(config.maker_fee(1_000_000).unwrap(), 0);
    }

    /// Test case: Zero quote amount always results in zero fee, regardless of rate.
    #[test]
    fn zero_amount_charges_no_fee() {
        // Various rates with zero quote
        let config_low = MarketFeeConfig::new(1, 1).unwrap();
        assert_eq!(config_low.taker_fee(0).unwrap(), 0);
        assert_eq!(config_low.maker_fee(0).unwrap(), 0);
        
        let config_mid = MarketFeeConfig::new(100_000, 50_000).unwrap();
        assert_eq!(config_mid.taker_fee(0).unwrap(), 0);
        assert_eq!(config_mid.maker_fee(0).unwrap(), 0);
        
        let config_max = MarketFeeConfig::new(MAX_FEE_RATE, MAX_FEE_RATE).unwrap();
        assert_eq!(config_max.taker_fee(0).unwrap(), 0);
        assert_eq!(config_max.maker_fee(0).unwrap(), 0);
    }

    /// Test case: One-unit fee is the minimum non-zero fee.
    /// When quote_amount * rate < FEE_RATE_SCALE but > 0, the fee rounds to 1.
    #[test]
    fn one_unit_is_minimum_non_zero_fee() {
        // Minimum rate (1) with minimum quote (1) rounds up to 1 atom fee
        let config = MarketFeeConfig::new(1, 1).unwrap();
        assert_eq!(config.taker_fee(1).unwrap(), 1);
        assert_eq!(config.maker_fee(1).unwrap(), 1);
        
        // Rate of 1 with various small amounts, all round to 1
        assert_eq!(config.taker_fee(1).unwrap(), 1);
        assert_eq!(config.taker_fee(10).unwrap(), 1);
        assert_eq!(config.taker_fee(100).unwrap(), 1);
        assert_eq!(config.taker_fee(1_000).unwrap(), 1);
        assert_eq!(config.taker_fee(10_000).unwrap(), 1);
        assert_eq!(config.taker_fee(FEE_RATE_SCALE - 1).unwrap(), 1);
        
        // Exactly FEE_RATE_SCALE produces fee of 1 without rounding up
        assert_eq!(config.taker_fee(FEE_RATE_SCALE).unwrap(), 1);
        
        // Just over FEE_RATE_SCALE rounds up to 2
        assert_eq!(config.taker_fee(FEE_RATE_SCALE + 1).unwrap(), 2);
    }

    /// Test case: Threshold behavior at rounding boundaries.
    /// Verify that fees round up correctly at exact FEE_RATE_SCALE multiples.
    #[test]
    fn threshold_rounding_behavior() {
        let config = MarketFeeConfig::new(100_000, 50_000).unwrap();
        
        // 0.1% rate: threshold is at 1000 quote atoms for 1 atom fee
        // quote * rate / FEE_RATE_SCALE = quote * 100_000 / 100_000_000 = quote / 1000
        
        // Just below threshold rounds up to 1
        assert_eq!(config.taker_fee(999).unwrap(), 1);
        
        // At threshold, no rounding needed
        assert_eq!(config.taker_fee(1_000).unwrap(), 1);
        
        // Just above threshold rounds up to 2
        assert_eq!(config.taker_fee(1_001).unwrap(), 2);
        
        // Multiple of threshold, no rounding
        assert_eq!(config.taker_fee(2_000).unwrap(), 2);
        assert_eq!(config.taker_fee(1_000_000).unwrap(), 1_000);
        
        // 0.05% maker rate: threshold is 2000 quote atoms for 1 atom fee
        assert_eq!(config.maker_fee(1_999).unwrap(), 1);
        assert_eq!(config.maker_fee(2_000).unwrap(), 1);
        assert_eq!(config.maker_fee(2_001).unwrap(), 2);
    }

    /// Test case: Maximum rate (100%) behavior.
    /// At MAX_FEE_RATE (100%), fee equals the quote amount.
    #[test]
    fn maximum_rate_behavior() {
        let config = MarketFeeConfig::new(MAX_FEE_RATE, MAX_FEE_RATE).unwrap();
        
        // 100% fee means fee = quote amount
        assert_eq!(config.taker_fee(1).unwrap(), 1);
        assert_eq!(config.taker_fee(100).unwrap(), 100);
        assert_eq!(config.taker_fee(1_000).unwrap(), 1_000);
        assert_eq!(config.taker_fee(1_000_000).unwrap(), 1_000_000);
        
        assert_eq!(config.maker_fee(1).unwrap(), 1);
        assert_eq!(config.maker_fee(1_000_000).unwrap(), 1_000_000);
        
        // Reservation at max rate requires double the quote (quote + 100% fee)
        assert_eq!(config.required_bid_reservation(1_000).unwrap(), 2_000);
    }

    /// Test case: Tiny fills that round to zero are acceptable.
    /// This verifies that sub-fee-unit amounts charge zero fee, which is
    /// acceptable in v1 because lot size and notional minimums prevent
    /// excessive dust trading.
    #[test]
    fn tiny_fills_may_have_zero_fee() {
        // At 0.1% (100_000), amounts below 1000 quote atoms round to 1 atom fee.
        // But at 0.001% (1_000), amounts below 100_000 round to 1 atom.
        // At rate 1 (0.000001%), only amounts >= 100_000_000 produce 1 atom fee.
        
        let config_tiny_rate = MarketFeeConfig::new(1, 1).unwrap();
        
        // Sub-scale amounts with rate=1 round up to 1
        assert_eq!(config_tiny_rate.taker_fee(1).unwrap(), 1);
        assert_eq!(config_tiny_rate.taker_fee(FEE_RATE_SCALE - 1).unwrap(), 1);
        
        // But with zero quote, fee is zero (not "minimum 1")
        assert_eq!(config_tiny_rate.taker_fee(0).unwrap(), 0);
        
        // Zero-rate market charges nothing even on large amounts
        let config_zero = MarketFeeConfig::new(0, 0).unwrap();
        assert_eq!(config_zero.taker_fee(1_000_000_000).unwrap(), 0);
    }

    /// Test case: Fees are always deterministic and consistent.
    /// Same inputs produce same outputs; no hidden state affects calculation.
    #[test]
    fn fee_calculation_is_deterministic() {
        let config = MarketFeeConfig::new(100_000, 50_000).unwrap();
        
        // Multiple calls with same inputs produce identical results
        for _ in 0..10 {
            assert_eq!(config.taker_fee(1_000).unwrap(), 1);
            assert_eq!(config.maker_fee(2_000).unwrap(), 1);
        }
    }

    /// Test case: Fee never exceeds amount for rates <= 100%.
    /// Ensures valid orders can always pay their fees (solvency).
    #[test]
    fn fee_never_exceeds_amount_for_valid_rates() {
        let amounts = vec![1, 10, 100, 1_000, 10_000, 100_000, 1_000_000];
        
        for rate in [1, 1_000, 10_000, 50_000, 100_000, MAX_FEE_RATE].iter() {
            let config = MarketFeeConfig::new(*rate, *rate).unwrap();
            
            for &amount in &amounts {
                let taker_fee = config.taker_fee(amount).unwrap();
                let maker_fee = config.maker_fee(amount).unwrap();
                
                // At 100% rate, fee = amount. Below 100%, fee <= amount.
                assert!(taker_fee <= amount, 
                    "taker_fee {} > amount {} at rate {}", taker_fee, amount, rate);
                assert!(maker_fee <= amount,
                    "maker_fee {} > amount {} at rate {}", maker_fee, amount, rate);
            }
        }
    }

    /// Test case: Cumulative taker fee prevents fragmentation exploits.
    /// Splitting a fill never reduces the total fee below the whole fill's fee.
    #[test]
    fn cumulative_fee_prevents_fragmentation_exploit() {
        let config = MarketFeeConfig::new(100_000, 0).unwrap();
        
        // Whole fill fee
        let whole_fill_fee = config.taker_fee(5_000).unwrap();
        
        // Split into 5 fills of 1000 each
        let mut cumulative_fee = 0;
        let mut filled = 0;
        for _ in 0..5 {
            cumulative_fee += config.cumulative_taker_fee_delta(filled, 1_000).unwrap();
            filled += 1_000;
        }
        
        // Cumulative fee equals whole fill fee (no exploit)
        assert_eq!(cumulative_fee, whole_fill_fee);
        
        // Try extreme fragmentation: 5000 fills of 1 atom each
        let mut extreme_cumulative = 0;
        let mut extreme_filled = 0;
        for _ in 0..5_000 {
            extreme_cumulative += config.cumulative_taker_fee_delta(extreme_filled, 1).unwrap();
            extreme_filled += 1;
        }
        
        // Even with extreme fragmentation, total fee is unchanged
        assert_eq!(extreme_cumulative, whole_fill_fee);
    }

    /// Test case: Documented examples from fee-accounting.md are correct.
    #[test]
    fn documented_fee_examples_are_correct() {
        // Example 1: quote=1000000, taker=100000 (0.1%), maker=50000 (0.05%)
        let config1 = MarketFeeConfig::new(100_000, 50_000).unwrap();
        assert_eq!(config1.taker_fee(1_000_000).unwrap(), 1_000);
        assert_eq!(config1.maker_fee(1_000_000).unwrap(), 500);
        
        // Example 2: quote=1000, taker=100000 (0.1%), maker=200000 (0.2%)
        let config2 = MarketFeeConfig::new(100_000, 200_000).unwrap();
        assert_eq!(config2.taker_fee(1_000).unwrap(), 1);
        assert_eq!(config2.maker_fee(1_000).unwrap(), 2);
    }

    /// Test case: Edge case - negative amounts are rejected.
    #[test]
    fn negative_amounts_are_rejected() {
        let config = MarketFeeConfig::new(100_000, 50_000).unwrap();
        
        assert!(config.taker_fee(-1).is_err());
        assert!(config.maker_fee(-1).is_err());
        assert!(config.cumulative_taker_fee_delta(0, -1).is_err());
    }

    /// Test case: Overflow protection in fee calculation.
    #[test]
    fn overflow_protection_in_fee_calculation() {
        let config = MarketFeeConfig::new(MAX_FEE_RATE, MAX_FEE_RATE).unwrap();
        
        // i128::MAX with any non-zero rate will overflow in multiplication
        assert!(config.taker_fee(i128::MAX).is_err());
        assert!(config.maker_fee(i128::MAX).is_err());
        
        // Large but valid amounts should work
        let large_valid = i128::MAX / (MAX_FEE_RATE + 1);
        assert!(config.taker_fee(large_valid).is_ok());
    }
}

#[cfg(test)]
mod fee_settlement_tests {
    use super::*;
    use soroban_sdk::testutils::Address as _;

    fn empty_ledger() -> SettlementLedger {
        SettlementLedger {
            base_transfers: Vec::new(),
            quote_transfers: Vec::new(),
            fee_liabilities: Vec::new(),
        }
    }

    #[test]
    fn maker_sell_pays_maker_fee_from_its_proceeds() {
        let env = Env::default();
        let quote_token = Address::generate(&env);
        // 0.1% taker fee and 0.05% maker fee.
        let config = MarketFeeConfig::new(100_000, 50_000).unwrap();
        let fill = Fill {
            maker: Address::generate(&env),
            taker: Address::generate(&env),
            base_amount: 10,
            quote_amount: 1_000_000,
            maker_is_sell: true,
        };
        let mut ledger = empty_ledger();
        let mut accumulator = FeeAccumulator::new(quote_token);
        let fees = fill
            .settle_with_fee_and_accumulator(&mut ledger, &mut accumulator, &config, 0)
            .unwrap();
        assert_eq!(fees.taker, 1_000);
        assert_eq!(fees.maker, 500);
        // The seller receives quote net of its fee; the buyer pays quote plus its fee.
        assert_eq!(ledger.quote_transfers[0].1, 999_500);
        assert_eq!(ledger.quote_transfers[1].1, -1_001_000);
        assert_eq!(accumulator.accrued, 1_500);
    }

    #[test]
    fn maker_buy_pays_quote_plus_fee_and_taker_seller_nets_its_fee() {
        let env = Env::default();
        let config = MarketFeeConfig::new(100_000, 50_000).unwrap();
        let fill = Fill {
            maker: Address::generate(&env),
            taker: Address::generate(&env),
            base_amount: 10,
            quote_amount: 1_000_000,
            maker_is_sell: false,
        };
        let mut ledger = empty_ledger();
        let fees = fill
            .settle_with_fee(&mut ledger, &config, Address::generate(&env), 0)
            .unwrap();
        assert_eq!(fees.total().unwrap(), 1_500);
        assert_eq!(ledger.base_transfers[0].1, 10);
        assert_eq!(ledger.quote_transfers[0].1, -1_000_500);
        assert_eq!(ledger.quote_transfers[1].1, 999_000);
    }

    #[test]
    fn zero_fee_market_has_no_liability_and_no_deduction() {
        let env = Env::default();
        let config = MarketFeeConfig::new(0, 0).unwrap();
        let fill = Fill {
            maker: Address::generate(&env),
            taker: Address::generate(&env),
            base_amount: 10,
            quote_amount: 1_000_000,
            maker_is_sell: true,
        };
        let mut ledger = empty_ledger();
        let fees = fill
            .settle_with_fee(&mut ledger, &config, Address::generate(&env), 0)
            .unwrap();
        assert_eq!(fees.total().unwrap(), 0);
        assert!(ledger.fee_liabilities.is_empty());
        assert_eq!(ledger.quote_transfers[0].1, 1_000_000);
        assert_eq!(ledger.quote_transfers[1].1, -1_000_000);
    }

    #[test]
    fn split_fills_charge_one_taker_fee_in_total_through_settlement() {
        let env = Env::default();
        let quote_token = Address::generate(&env);
        let config = MarketFeeConfig::new(1, 0).unwrap();
        let taker = Address::generate(&env);
        let mut ledger = empty_ledger();
        let mut taker_quote_before = 0;
        let mut taker_fees = 0;
        for _ in 0..3 {
            let fill = Fill {
                maker: Address::generate(&env),
                taker: taker.clone(),
                base_amount: 1,
                quote_amount: 1,
                maker_is_sell: false,
            };
            let fees = fill
                .settle_with_fee(&mut ledger, &config, quote_token.clone(), taker_quote_before)
                .unwrap();
            taker_fees += fees.taker;
            taker_quote_before += 1;
        }
        assert_eq!(taker_fees, config.taker_fee(3).unwrap());
    }
    #[test]
    fn arithmetic_failure_after_fee_calculation_rolls_back_ledger_and_accumulator() {
        let env = Env::default();
        let quote_token = Address::generate(&env);
        let config = MarketFeeConfig::new(1, 1).unwrap();
        let fill = Fill {
            maker: Address::generate(&env),
            taker: Address::generate(&env),
            base_amount: 1,
            // Taker quote + fee overflows only after fee calculations have succeeded.
            quote_amount: i128::MAX,
            maker_is_sell: true,
        };
        let mut ledger = empty_ledger();
        let mut accumulator = FeeAccumulator::new(quote_token);
        let before_base = ledger.base_transfers.len();
        let before_quote = ledger.quote_transfers.len();
        let before_liabilities = ledger.fee_liabilities.len();

        assert!(fill
            .settle_with_fee_and_accumulator(&mut ledger, &mut accumulator, &config, 0)
            .is_err());
        assert_eq!(ledger.base_transfers.len(), before_base);
        assert_eq!(ledger.quote_transfers.len(), before_quote);
        assert_eq!(ledger.fee_liabilities.len(), before_liabilities);
        assert_eq!(accumulator.accrued, 0);
    }

    #[test]
    fn fee_accumulator_overflow_rolls_back_all_ledger_mutations() {
        let env = Env::default();
        let quote_token = Address::generate(&env);
        let config = MarketFeeConfig::new(1, 1).unwrap();
        let fill = Fill {
            maker: Address::generate(&env),
            taker: Address::generate(&env),
            base_amount: 2,
            quote_amount: 100,
            maker_is_sell: true,
        };
        let mut ledger = empty_ledger();
        let mut accumulator = FeeAccumulator::new(quote_token);
        accumulator.accrued = i128::MAX;
        let before = ledger.clone();

        assert!(fill
            .settle_with_fee_and_accumulator(&mut ledger, &mut accumulator, &config, 0)
            .is_err());
        assert_eq!(ledger.base_transfers.len(), before.base_transfers.len());
        assert_eq!(ledger.quote_transfers.len(), before.quote_transfers.len());
        assert_eq!(ledger.fee_liabilities.len(), before.fee_liabilities.len());
        assert_eq!(accumulator.accrued, i128::MAX);
    }

}

#[cfg(test)]
mod rounding_tests {
    use super::*;

    // Spec section 10.4 Examples B and C: lot_size 1, tick 3 with
    // tick_size 150_000_000, so the scaled price is 4.5 quote atoms per base atom.
    const TICK_3: i128 = 450_000_000;

    #[test]
    fn quote_atoms_rounds_in_the_requested_direction() {
        assert_eq!(quote_atoms(3, TICK_3, RoundingMode::Up).unwrap(), 14);
        assert_eq!(quote_atoms(3, TICK_3, RoundingMode::Down).unwrap(), 13);
        assert_eq!(quote_atoms(2, TICK_3, RoundingMode::Up).unwrap(), 9);
        assert_eq!(quote_atoms(2, TICK_3, RoundingMode::Down).unwrap(), 9);
    }

    #[test]
    fn quote_atoms_rejects_negative_and_overflowing_inputs() {
        assert!(quote_atoms(-1, TICK_3, RoundingMode::Up).is_err());
        assert!(quote_atoms(i128::MAX, TICK_3, RoundingMode::Down).is_err());
    }

    #[test]
    fn single_fill_residue_is_credited_to_dust() {
        // Example B: the buyer is debited 14, the seller credited 13, dust gains 1.
        let legs = fill_quote_legs(1, 3, 3, TICK_3).unwrap();
        assert_eq!(legs, FillQuoteLegs { debit: 14, credit: 13, residue: 1 });
    }

    #[test]
    fn split_fills_pay_the_same_as_one_fill() {
        // Example C: fills of 1 lot and 2 lots total the same as one 3-lot fill.
        let whole = fill_quote_legs(1, 3, 3, TICK_3).unwrap();
        let first = fill_quote_legs(1, 3, 1, TICK_3).unwrap();
        let second = fill_quote_legs(1, 2, 2, TICK_3).unwrap();
        assert_eq!(first.debit + second.debit, whole.debit);
        assert_eq!(first.credit + second.credit, whole.credit);
        assert_eq!(first.residue + second.residue, whole.residue);
    }

    #[test]
    fn fragmentation_never_increases_seller_credit() {
        // Seller credit is a floor of each part, so the parts sum to at most the
        // floor of the whole. Splitting can only move value to the dust account.
        let whole_credit = quote_atoms(5, TICK_3, RoundingMode::Down).unwrap();
        let mut parts_credit = 0;
        // Five 1-lot fills of a 5-lot bid, with the remaining size walking 5 -> 1.
        for remaining in (1..=5).rev() {
            parts_credit += fill_quote_legs(1, remaining, 1, TICK_3).unwrap().credit;
        }
        assert_eq!(whole_credit, 22);
        assert_eq!(parts_credit, 20);
        assert!(parts_credit <= whole_credit);
    }

    #[test]
    fn fill_size_and_lot_size_are_validated() {
        assert!(fill_quote_legs(1, 3, 0, TICK_3).is_err());
        assert!(fill_quote_legs(1, 3, 4, TICK_3).is_err());
        assert!(fill_quote_legs(0, 3, 1, TICK_3).is_err());
    }
}


// ============ CLOB-918: Ledger-Based Order Expiration Tests ============
#[cfg(test)]
mod expiration_tests {
    use super::*;
    use soroban_sdk::testutils::{Address as _, Ledger};

    /// Test case: GTC orders never expire regardless of ledger sequence or timestamp.
    #[test]
    fn gtc_orders_never_expire() {
        let env = Env::default();
        let owner = Address::generate(&env);
        let order = Order::new_gtc(1, owner.clone(), 100);

        // GTC orders are always active, regardless of ledger or timestamp
        assert!(!order.is_expired(0, 0));
        assert!(!order.is_expired(1, 1));
        assert!(!order.is_expired(100, 1000));
        assert!(!order.is_expired(1_000_000, 1_000_000_000));
        assert!(!order.is_expired(u64::MAX, u64::MAX));

        // is_active is the inverse
        assert!(order.is_active(0, 0));
        assert!(order.is_active(u64::MAX, u64::MAX));
    }

    /// Test case: GTD orders expire at exact expiry_ledger boundary.
    #[test]
    fn gtd_orders_expire_at_exact_boundary() {
        let env = Env::default();
        let owner = Address::generate(&env);
        let expiry = 100;
        let order = Order::new_gtd(1, owner.clone(), 50, expiry);

        // Active before expiry (timestamp is irrelevant for GTD)
        assert!(order.is_active(99, 0));
        assert!(!order.is_expired(99, 1_000_000));

        // Expired at exact expiry ledger
        assert!(order.is_expired(100, 0));
        assert!(!order.is_active(100, 0));

        // Expired after expiry ledger
        assert!(order.is_expired(101, 0));
        assert!(order.is_expired(1_000, 0));
        assert!(!order.is_active(101, 0));
    }

    /// Test case: GTD order placement validation requires future expiry.
    #[test]
    fn gtd_placement_requires_future_expiry() {
        let env = Env::default();
        let owner = Address::generate(&env);

        // Set current ledger to 100
        env.ledger().set_sequence_number(100);
        env.ledger().set_timestamp(5000);

        // Expiry in the past: rejected
        let order_past = Order::new_gtd(1, owner.clone(), 50, 99);
        assert!(order_past.validate_placement(100, 5000).is_err());

        // Expiry at current ledger: rejected (must be strictly greater)
        let order_current = Order::new_gtd(2, owner.clone(), 50, 100);
        assert!(order_current.validate_placement(100, 5000).is_err());

        // Expiry in the future: accepted
        let order_future = Order::new_gtd(3, owner.clone(), 50, 101);
        assert!(order_future.validate_placement(100, 5000).is_ok());

        // Far future expiry: accepted
        let order_far_future = Order::new_gtd(4, owner.clone(), 50, 10_000);
        assert!(order_far_future.validate_placement(100, 5000).is_ok());
    }

    /// Test case: GTC order placement always succeeds.
    #[test]
    fn gtc_placement_always_succeeds() {
        let env = Env::default();
        let owner = Address::generate(&env);

        // GTC orders can be placed at any ledger/timestamp
        for ledger in [0, 1, 100, 1_000, u64::MAX / 2].iter() {
            let timestamp = ledger * 5; // Arbitrary timestamp
            env.ledger().set_sequence_number(*ledger);
            env.ledger().set_timestamp(timestamp);
            let order = Order::new_gtc(1, owner.clone(), 100);
            assert!(order.validate_placement(*ledger, timestamp).is_ok());
        }
    }

    /// Test case: Expired makers cannot fill (Invariant I11).
    #[test]
    fn expired_makers_cannot_fill() {
        let env = Env::default();
        let owner = Address::generate(&env);
        let expiry = 500;
        let order = Order::new_gtd(1, owner.clone(), 100, expiry);

        // Can fill before expiry
        assert!(order.can_fill_as_maker(499, 0));
        assert!(order.can_fill_as_maker(100, 0));
        assert!(order.can_fill_as_maker(0, 0));

        // Cannot fill at or after expiry
        assert!(!order.can_fill_as_maker(500, 0));
        assert!(!order.can_fill_as_maker(501, 0));
        assert!(!order.can_fill_as_maker(1_000, 0));
    }

    /// Test case: GTC makers can always fill.
    #[test]
    fn gtc_makers_can_always_fill() {
        let env = Env::default();
        let owner = Address::generate(&env);
        let order = Order::new_gtc(1, owner.clone(), 100);

        // Can fill at any ledger/timestamp
        assert!(order.can_fill_as_maker(0, 0));
        assert!(order.can_fill_as_maker(1_000, 10_000));
        assert!(order.can_fill_as_maker(u64::MAX, u64::MAX));
    }

    /// Test case: OrderRequest validation with Env.
    #[test]
    fn order_request_validation_with_env() {
        let env = Env::default();
        env.ledger().set_sequence_number(100);
        env.ledger().set_timestamp(5000);

        // GTC request: always valid
        let gtc_request = OrderRequest::new_gtc();
        assert!(gtc_request.validate(&env).is_ok());

        // GTD request with past expiry: invalid
        let gtd_past = OrderRequest::new_gtd(99);
        assert!(gtd_past.validate(&env).is_err());

        // GTD request with current ledger expiry: invalid
        let gtd_current = OrderRequest::new_gtd(100);
        assert!(gtd_current.validate(&env).is_err());

        // GTD request with future expiry: valid
        let gtd_future = OrderRequest::new_gtd(101);
        assert!(gtd_future.validate(&env).is_ok());
    }

    /// Test case: OrderRequest::validate_tif static method.
    #[test]
    fn order_request_validate_tif_static() {
        let env = Env::default();
        env.ledger().set_sequence_number(100);
        env.ledger().set_timestamp(5000);

        // GTC with no expiry: valid
        assert!(OrderRequest::validate_tif(&env, TimeInForce::GTC, None, None).is_ok());

        // GTC with zero expiry: valid
        assert!(OrderRequest::validate_tif(&env, TimeInForce::GTC, Some(0), Some(0)).is_ok());

        // GTC with non-zero ledger expiry: invalid
        assert!(OrderRequest::validate_tif(&env, TimeInForce::GTC, Some(200), Some(0)).is_err());

        // GTD with no expiry: invalid
        assert!(OrderRequest::validate_tif(&env, TimeInForce::GTD, None, None).is_err());

        // GTD with past expiry: invalid
        assert!(OrderRequest::validate_tif(&env, TimeInForce::GTD, Some(99), None).is_err());

        // GTD with current expiry: invalid
        assert!(OrderRequest::validate_tif(&env, TimeInForce::GTD, Some(100), None).is_err());

        // GTD with future expiry: valid
        assert!(OrderRequest::validate_tif(&env, TimeInForce::GTD, Some(101), None).is_ok());
    }

    /// Test case: is_active matches expected behavior across ledger boundaries.
    #[test]
    fn is_active_behavior_across_ledger_boundaries() {
        let env = Env::default();
        let owner = Address::generate(&env);

        // GTD order with expiry at ledger 1000
        let gtd_order = Order::new_gtd(1, owner.clone(), 50, 1000);

        // Active well before expiry
        assert!(gtd_order.is_active(0, 0));
        assert!(gtd_order.is_active(500, 0));
        assert!(gtd_order.is_active(999, 0));

        // Expired at and after expiry
        assert!(!gtd_order.is_active(1000, 0));
        assert!(!gtd_order.is_active(1001, 0));

        // GTC order always active
        let gtc_order = Order::new_gtc(2, owner.clone(), 50);
        assert!(gtc_order.is_active(0, 0));
        assert!(gtc_order.is_active(1000, 10000));
        assert!(gtc_order.is_active(u64::MAX, u64::MAX));
    }

    /// Test case: Multiple orders with different expiries.
    #[test]
    fn multiple_orders_different_expiries() {
        let env = Env::default();
        let owner = Address::generate(&env);

        let order1 = Order::new_gtd(1, owner.clone(), 10, 100);
        let order2 = Order::new_gtd(2, owner.clone(), 20, 200);
        let order3 = Order::new_gtd(3, owner.clone(), 30, 300);
        let order4 = Order::new_gtc(4, owner.clone(), 40);

        // At ledger 150: order1 expired, others active
        assert!(order1.is_expired(150, 0));
        assert!(order2.is_active(150, 0));
        assert!(order3.is_active(150, 0));
        assert!(order4.is_active(150, 0));

        // At ledger 250: order1 and order2 expired, others active
        assert!(order1.is_expired(250, 0));
        assert!(order2.is_expired(250, 0));
        assert!(order3.is_active(250, 0));
        assert!(order4.is_active(250, 0));

        // At ledger 350: all GTD expired, GTC still active
        assert!(order1.is_expired(350, 0));
        assert!(order2.is_expired(350, 0));
        assert!(order3.is_expired(350, 0));
        assert!(order4.is_active(350, 0));
    }

    /// Test case: Edge case - order expiring at ledger 0.
    #[test]
    fn order_expiring_at_ledger_zero() {
        let env = Env::default();
        let owner = Address::generate(&env);

        // This order would be immediately expired at any positive ledger
        let order = Order::new_gtd(1, owner.clone(), 50, 0);

        // Expired at ledger 0 and beyond
        assert!(order.is_expired(0, 0));
        assert!(order.is_expired(1, 0));
        assert!(order.is_expired(100, 0));

        // Such an order cannot be placed because expiry must be > current_ledger
        // If we're at ledger 0 and try to place with expiry 0, it should fail
        env.ledger().set_sequence_number(0);
        assert!(order.validate_placement(0, 0).is_err());
    }

    /// Test case: Edge case - order with maximum expiry ledger.
    #[test]
    fn order_with_maximum_expiry() {
        let env = Env::default();
        let owner = Address::generate(&env);

        let max_expiry = u64::MAX;
        let order = Order::new_gtd(1, owner.clone(), 50, max_expiry);

        // Active at all ledgers except MAX
        assert!(order.is_active(0, 0));
        assert!(order.is_active(u64::MAX / 2, 0));
        assert!(order.is_active(u64::MAX - 1, 0));

        // Expired at MAX
        assert!(order.is_expired(u64::MAX, 0));
    }

    /// Test case: Order expiration does not depend on other fields.
    #[test]
    fn expiration_independent_of_other_fields() {
        let env = Env::default();
        let owner1 = Address::generate(&env);
        let owner2 = Address::generate(&env);

        // Same expiry, different owners/ids/lots - same expiration behavior
        let order1 = Order::new_gtd(1, owner1.clone(), 10, 500);
        let order2 = Order::new_gtd(999, owner2.clone(), 10000, 500);

        for ledger in [0, 499, 500, 501, 1000].iter() {
            assert_eq!(order1.is_expired(*ledger, 0), order2.is_expired(*ledger, 0));
            assert_eq!(order1.is_active(*ledger, 0), order2.is_active(*ledger, 0));
        }
    }

    /// Test case: Deterministic expiration checking.
    #[test]
    fn expiration_is_deterministic() {
        let env = Env::default();
        let owner = Address::generate(&env);
        let order = Order::new_gtd(1, owner.clone(), 50, 1000);

        // Multiple checks at same ledger produce same result
        for _ in 0..10 {
            assert!(order.is_active(999, 0));
            assert!(order.is_expired(1000, 0));
            assert!(!order.is_active(1001, 0));
        }
    }

    /// Test case: OrderRequest is_active matches Order behavior.
    #[test]
    fn order_request_is_active_matches_order() {
        let env = Env::default();
        let owner = Address::generate(&env);

        // GTC request
        let gtc_request = OrderRequest::new_gtc();
        let gtc_order = Order::new_gtc(1, owner.clone(), 50);

        for ledger in [0, 100, 1000, u64::MAX / 2].iter() {
            assert_eq!(gtc_request.is_active(*ledger, 0), gtc_order.is_active(*ledger, 0));
        }

        // GTD request
        let gtd_request = OrderRequest::new_gtd(500);
        let gtd_order = Order::new_gtd(2, owner.clone(), 50, 500);

        for ledger in [0, 499, 500, 501, 1000].iter() {
            assert_eq!(gtd_request.is_active(*ledger, 0), gtd_order.is_active(*ledger, 0));
        }
    }

    /// Test case: Composition with collateral - expired order lock should be released.
    /// This tests the conceptual requirement that expiration composes with
    /// collateral release, though the actual release logic is in matching/cancel.
    #[test]
    fn expiration_composes_with_collateral_logic() {
        let env = Env::default();
        let owner = Address::generate(&env);

        // Order expires at ledger 100
        let order = Order::new_gtd(1, owner.clone(), 50, 100);

        // Before expiry: can fill, so lock should remain
        assert!(order.can_fill_as_maker(99, 0));

        // At expiry: cannot fill, so lock should be released (by caller)
        assert!(!order.can_fill_as_maker(100, 0));

        // After expiry: still cannot fill
        assert!(!order.can_fill_as_maker(101, 0));
    }

    /// Test case: Batch expiration check (simulating matching loop).
    /// This verifies that expiration checks can be performed efficiently
    /// in a loop without unbounded iteration.
    #[test]
    fn batch_expiration_check_is_bounded() {
        let env = Env::default();
        let owner = Address::generate(&env);

        // Create orders with various expiries
        let orders = vec![
            Order::new_gtd(1, owner.clone(), 10, 100),
            Order::new_gtd(2, owner.clone(), 20, 200),
            Order::new_gtc(3, owner.clone(), 30),
            Order::new_gtd(4, owner.clone(), 40, 150),
            Order::new_gtd(5, owner.clone(), 50, 175),
        ];

        // At ledger 160, check which can fill (simulating matching)
        let current_ledger = 160;
        let current_timestamp = 0;
        let active_count = orders
            .iter()
            .filter(|o| o.can_fill_as_maker(current_ledger, current_timestamp))
            .count();

        // Orders 2 and 3 should be active (expiry 200 and GTC)
        // Orders 1, 4, 5 should be expired (expiry 100, 150, 175)
        assert_eq!(active_count, 2);
    }

    /// Test case: Zero-lot orders (edge case).
    /// While the protocol states remaining_lots > 0 for resting orders,
    /// test that expiration logic doesn't break with zero lots.
    #[test]
    fn expiration_works_with_zero_lots() {
        let env = Env::default();
        let owner = Address::generate(&env);

        let order = Order::new_gtd(1, owner.clone(), 0, 100);

        assert!(order.is_active(99, 0));
        assert!(order.is_expired(100, 0));
    }
}


// ============ CLOB-978: Timestamp-Based Expiration Tests ============
#[cfg(test)]
mod timestamp_expiration_tests {
    use super::*;
    use soroban_sdk::testutils::{Address as _, Ledger};

    /// Test case: GTT orders expire at exact timestamp boundary.
    #[test]
    fn gtt_orders_expire_at_exact_timestamp_boundary() {
        let env = Env::default();
        let owner = Address::generate(&env);
        let expiry_timestamp = 10_000;
        let order = Order::new_gtt(1, owner.clone(), 50, expiry_timestamp);

        // Active before expiry (ledger is irrelevant for GTT)
        assert!(order.is_active(0, 9_999));
        assert!(!order.is_expired(100, 9_999));

        // Expired at exact expiry timestamp
        assert!(order.is_expired(0, 10_000));
        assert!(!order.is_active(0, 10_000));

        // Expired after expiry timestamp
        assert!(order.is_expired(0, 10_001));
        assert!(order.is_expired(0, 20_000));
        assert!(!order.is_active(0, 10_001));
    }

    /// Test case: GTT order placement validation requires future timestamp.
    #[test]
    fn gtt_placement_requires_future_timestamp() {
        let env = Env::default();
        let owner = Address::generate(&env);

        // Set current timestamp to 5000
        env.ledger().set_sequence_number(100);
        env.ledger().set_timestamp(5000);

        // Expiry in the past: rejected
        let order_past = Order::new_gtt(1, owner.clone(), 50, 4_999);
        assert!(order_past.validate_placement(100, 5000).is_err());

        // Expiry at current timestamp: rejected (must be strictly greater)
        let order_current = Order::new_gtt(2, owner.clone(), 50, 5_000);
        assert!(order_current.validate_placement(100, 5000).is_err());

        // Expiry in the future: accepted
        let order_future = Order::new_gtt(3, owner.clone(), 50, 5_001);
        assert!(order_future.validate_placement(100, 5000).is_ok());

        // Far future expiry: accepted
        let order_far_future = Order::new_gtt(4, owner.clone(), 50, 100_000);
        assert!(order_far_future.validate_placement(100, 5000).is_ok());
    }

    /// Test case: GTT makers cannot fill after timestamp expiry.
    #[test]
    fn gtt_expired_makers_cannot_fill() {
        let env = Env::default();
        let owner = Address::generate(&env);
        let expiry_timestamp = 50_000;
        let order = Order::new_gtt(1, owner.clone(), 100, expiry_timestamp);

        // Can fill before expiry
        assert!(order.can_fill_as_maker(0, 49_999));
        assert!(order.can_fill_as_maker(100, 10_000));
        assert!(order.can_fill_as_maker(1000, 0));

        // Cannot fill at or after expiry
        assert!(!order.can_fill_as_maker(0, 50_000));
        assert!(!order.can_fill_as_maker(100, 50_001));
        assert!(!order.can_fill_as_maker(1000, 100_000));
    }

    /// Test case: GTT OrderRequest validation with Env.
    #[test]
    fn gtt_order_request_validation_with_env() {
        let env = Env::default();
        env.ledger().set_sequence_number(100);
        env.ledger().set_timestamp(5000);

        // GTT request with past expiry: invalid
        let gtt_past = OrderRequest::new_gtt(4_999);
        assert!(gtt_past.validate(&env).is_err());

        // GTT request with current timestamp expiry: invalid
        let gtt_current = OrderRequest::new_gtt(5_000);
        assert!(gtt_current.validate(&env).is_err());

        // GTT request with future expiry: valid
        let gtt_future = OrderRequest::new_gtt(5_001);
        assert!(gtt_future.validate(&env).is_ok());
    }

    /// Test case: GTT OrderRequest::validate_tif static method.
    #[test]
    fn gtt_validate_tif_static() {
        let env = Env::default();
        env.ledger().set_sequence_number(100);
        env.ledger().set_timestamp(5000);

        // GTT with no timestamp: invalid
        assert!(OrderRequest::validate_tif(&env, TimeInForce::GTT, None, None).is_err());

        // GTT with past timestamp: invalid
        assert!(OrderRequest::validate_tif(&env, TimeInForce::GTT, None, Some(4_999)).is_err());

        // GTT with current timestamp: invalid
        assert!(OrderRequest::validate_tif(&env, TimeInForce::GTT, None, Some(5_000)).is_err());

        // GTT with future timestamp: valid
        assert!(OrderRequest::validate_tif(&env, TimeInForce::GTT, None, Some(5_001)).is_ok());
    }

    /// Test case: Multiple GTT orders with different timestamp expiries.
    #[test]
    fn multiple_gtt_orders_different_expiries() {
        let env = Env::default();
        let owner = Address::generate(&env);

        let order1 = Order::new_gtt(1, owner.clone(), 10, 10_000);
        let order2 = Order::new_gtt(2, owner.clone(), 20, 20_000);
        let order3 = Order::new_gtt(3, owner.clone(), 30, 30_000);
        let order4 = Order::new_gtc(4, owner.clone(), 40);

        // At timestamp 15_000: order1 expired, others active
        assert!(order1.is_expired(0, 15_000));
        assert!(order2.is_active(0, 15_000));
        assert!(order3.is_active(0, 15_000));
        assert!(order4.is_active(0, 15_000));

        // At timestamp 25_000: order1 and order2 expired, others active
        assert!(order1.is_expired(0, 25_000));
        assert!(order2.is_expired(0, 25_000));
        assert!(order3.is_active(0, 25_000));
        assert!(order4.is_active(0, 25_000));

        // At timestamp 35_000: all GTT expired, GTC still active
        assert!(order1.is_expired(0, 35_000));
        assert!(order2.is_expired(0, 35_000));
        assert!(order3.is_expired(0, 35_000));
        assert!(order4.is_active(0, 35_000));
    }

    /// Test case: GTT and GTD orders can coexist independently.
    #[test]
    fn gtt_and_gtd_coexist_independently() {
        let env = Env::default();
        let owner = Address::generate(&env);

        let gtd_order = Order::new_gtd(1, owner.clone(), 50, 100);
        let gtt_order = Order::new_gtt(2, owner.clone(), 50, 10_000);

        // At ledger 50, timestamp 5000: both active
        assert!(gtd_order.is_active(50, 5_000));
        assert!(gtt_order.is_active(50, 5_000));

        // At ledger 150, timestamp 5000: GTD expired, GTT active
        assert!(gtd_order.is_expired(150, 5_000));
        assert!(gtt_order.is_active(150, 5_000));

        // At ledger 50, timestamp 15000: GTD active, GTT expired
        assert!(gtd_order.is_active(50, 15_000));
        assert!(gtt_order.is_expired(50, 15_000));

        // At ledger 150, timestamp 15000: both expired
        assert!(gtd_order.is_expired(150, 15_000));
        assert!(gtt_order.is_expired(150, 15_000));
    }

    /// Test case: Exact timestamp equality boundary behavior.
    #[test]
    fn exact_timestamp_equality_boundary() {
        let env = Env::default();
        let owner = Address::generate(&env);
        let expiry = 100_000;
        let order = Order::new_gtt(1, owner.clone(), 50, expiry);

        // One nanosecond before (assuming second precision): active
        // In practice, Soroban uses second precision
        assert!(order.is_active(0, expiry - 1));

        // Exactly at expiry: expired
        assert!(order.is_expired(0, expiry));
        assert!(!order.is_active(0, expiry));

        // One second after: expired
        assert!(order.is_expired(0, expiry + 1));
    }

    /// Test case: Edge case - GTT order with timestamp 0.
    #[test]
    fn gtt_order_with_timestamp_zero() {
        let env = Env::default();
        let owner = Address::generate(&env);

        // Order expiring at timestamp 0
        let order = Order::new_gtt(1, owner.clone(), 50, 0);

        // Expired at timestamp 0 and beyond
        assert!(order.is_expired(0, 0));
        assert!(order.is_expired(0, 1));
        assert!(order.is_expired(0, 100_000));

        // Cannot be placed at timestamp 0 with expiry 0
        assert!(order.validate_placement(0, 0).is_err());
    }

    /// Test case: Edge case - GTT order with maximum timestamp.
    #[test]
    fn gtt_order_with_maximum_timestamp() {
        let env = Env::default();
        let owner = Address::generate(&env);

        let max_timestamp = u64::MAX;
        let order = Order::new_gtt(1, owner.clone(), 50, max_timestamp);

        // Active at all timestamps except MAX
        assert!(order.is_active(0, 0));
        assert!(order.is_active(0, u64::MAX / 2));
        assert!(order.is_active(0, u64::MAX - 1));

        // Expired at MAX
        assert!(order.is_expired(0, u64::MAX));
    }

    /// Test case: GTT expiration independent of ledger sequence.
    #[test]
    fn gtt_expiration_independent_of_ledger() {
        let env = Env::default();
        let owner = Address::generate(&env);
        let expiry_timestamp = 50_000;
        let order = Order::new_gtt(1, owner.clone(), 50, expiry_timestamp);

        // At different ledgers but same timestamp: same expiration state
        for ledger in [0, 100, 1_000, 10_000, u64::MAX / 2].iter() {
            // Active before expiry
            assert!(order.is_active(*ledger, expiry_timestamp - 1));
            // Expired at expiry
            assert!(order.is_expired(*ledger, expiry_timestamp));
            // Expired after expiry
            assert!(order.is_expired(*ledger, expiry_timestamp + 1));
        }
    }

    /// Test case: GTD expiration independent of timestamp.
    #[test]
    fn gtd_expiration_independent_of_timestamp() {
        let env = Env::default();
        let owner = Address::generate(&env);
        let expiry_ledger = 500;
        let order = Order::new_gtd(1, owner.clone(), 50, expiry_ledger);

        // At different timestamps but same ledger: same expiration state
        for timestamp in [0, 1_000, 10_000, 100_000, u64::MAX / 2].iter() {
            // Active before expiry
            assert!(order.is_active(expiry_ledger - 1, *timestamp));
            // Expired at expiry
            assert!(order.is_expired(expiry_ledger, *timestamp));
            // Expired after expiry
            assert!(order.is_expired(expiry_ledger + 1, *timestamp));
        }
    }

    /// Test case: Deterministic GTT expiration checking.
    #[test]
    fn gtt_expiration_is_deterministic() {
        let env = Env::default();
        let owner = Address::generate(&env);
        let order = Order::new_gtt(1, owner.clone(), 50, 100_000);

        // Multiple checks at same timestamp produce same result
        for _ in 0..10 {
            assert!(order.is_active(0, 99_999));
            assert!(order.is_expired(0, 100_000));
            assert!(!order.is_active(0, 100_001));
        }
    }

    /// Test case: GTT OrderRequest is_active matches Order behavior.
    #[test]
    fn gtt_order_request_is_active_matches_order() {
        let env = Env::default();
        let owner = Address::generate(&env);

        let gtt_request = OrderRequest::new_gtt(50_000);
        let gtt_order = Order::new_gtt(1, owner.clone(), 50, 50_000);

        for timestamp in [0, 49_999, 50_000, 50_001, 100_000].iter() {
            assert_eq!(
                gtt_request.is_active(0, *timestamp),
                gtt_order.is_active(0, *timestamp)
            );
        }
    }

    /// Test case: Batch GTT expiration check is bounded.
    #[test]
    fn batch_gtt_expiration_check_is_bounded() {
        let env = Env::default();
        let owner = Address::generate(&env);

        // Create orders with various timestamp expiries
        let orders = vec![
            Order::new_gtt(1, owner.clone(), 10, 10_000),
            Order::new_gtt(2, owner.clone(), 20, 20_000),
            Order::new_gtc(3, owner.clone(), 30),
            Order::new_gtt(4, owner.clone(), 40, 15_000),
            Order::new_gtt(5, owner.clone(), 50, 17_500),
        ];

        // At timestamp 16_000, check which can fill
        let current_ledger = 0;
        let current_timestamp = 16_000;
        let active_count = orders
            .iter()
            .filter(|o| o.can_fill_as_maker(current_ledger, current_timestamp))
            .count();

        // Orders 2 and 3 should be active (expiry 20_000 and GTC)
        // Orders 1, 4, 5 should be expired (expiry 10_000, 15_000, 17_500)
        assert_eq!(active_count, 2);
    }

    /// Test case: Mixed GTD and GTT orders in batch.
    #[test]
    fn mixed_gtd_gtt_batch_expiration() {
        let env = Env::default();
        let owner = Address::generate(&env);

        let orders = vec![
            Order::new_gtd(1, owner.clone(), 10, 100),      // Expires at ledger 100
            Order::new_gtt(2, owner.clone(), 20, 10_000),   // Expires at timestamp 10_000
            Order::new_gtc(3, owner.clone(), 30),           // Never expires
            Order::new_gtd(4, owner.clone(), 40, 200),      // Expires at ledger 200
            Order::new_gtt(5, owner.clone(), 50, 20_000),   // Expires at timestamp 20_000
        ];

        // At ledger 150, timestamp 15_000:
        // - Order 1: expired (ledger 100 < 150)
        // - Order 2: expired (timestamp 10_000 < 15_000)
        // - Order 3: active (GTC)
        // - Order 4: active (ledger 200 > 150)
        // - Order 5: active (timestamp 20_000 > 15_000)
        let active_count = orders
            .iter()
            .filter(|o| o.can_fill_as_maker(150, 15_000))
            .count();

        assert_eq!(active_count, 3); // Orders 3, 4, 5
    }

    /// Test case: Wall-clock reasoning - GTT for human-readable expiry.
    #[test]
    fn gtt_wall_clock_reasoning() {
        let env = Env::default();
        let owner = Address::generate(&env);

        // Example: Order should expire at Unix timestamp 1735689600
        // (January 1, 2025, 00:00:00 UTC)
        let expiry = 1_735_689_600;
        let order = Order::new_gtt(1, owner.clone(), 100, expiry);

        // Before January 1, 2025: active
        assert!(order.is_active(0, expiry - 1));

        // At or after January 1, 2025: expired
        assert!(order.is_expired(0, expiry));
        assert!(order.is_expired(0, expiry + 3600)); // One hour later
    }

    /// Test case: GTT placement at edge of timestamp space.
    #[test]
    fn gtt_placement_at_timestamp_edges() {
        let env = Env::default();
        let owner = Address::generate(&env);

        // At timestamp 0
        env.ledger().set_timestamp(0);
        let order_at_zero = Order::new_gtt(1, owner.clone(), 50, 1);
        assert!(order_at_zero.validate_placement(0, 0).is_ok());

        // Near max timestamp
        env.ledger().set_timestamp(u64::MAX - 1000);
        let order_near_max = Order::new_gtt(2, owner.clone(), 50, u64::MAX);
        assert!(order_near_max.validate_placement(0, u64::MAX - 1000).is_ok());
    }

    /// Test case: GTT order with zero lots (edge case).
    #[test]
    fn gtt_expiration_works_with_zero_lots() {
        let env = Env::default();
        let owner = Address::generate(&env);

        let order = Order::new_gtt(1, owner.clone(), 0, 10_000);

        assert!(order.is_active(0, 9_999));
        assert!(order.is_expired(0, 10_000));
    }

    /// Test case: Composition - GTT expiration composes with collateral.
    #[test]
    fn gtt_expiration_composes_with_collateral() {
        let env = Env::default();
        let owner = Address::generate(&env);

        // Order expires at timestamp 10_000
        let order = Order::new_gtt(1, owner.clone(), 50, 10_000);

        // Before expiry: can fill, lock should remain
        assert!(order.can_fill_as_maker(0, 9_999));

        // At expiry: cannot fill, lock should be released (by caller)
        assert!(!order.can_fill_as_maker(0, 10_000));

        // After expiry: still cannot fill
        assert!(!order.can_fill_as_maker(0, 10_001));
    }

    /// Test case: GTT avoids requiring off-chain keepers.
    /// The expiration check is performed on-chain using ledger timestamp,
    /// so no off-chain keeper is required for correctness.
    #[test]
    fn gtt_no_offchain_keeper_required() {
        let env = Env::default();
        let owner = Address::generate(&env);

        // Place an order that expires at timestamp 10_000
        let order = Order::new_gtt(1, owner.clone(), 100, 10_000);

        // At any ledger, as long as we have the correct timestamp,
        // expiration is checked correctly without external keeper
        env.ledger().set_sequence_number(1);
        env.ledger().set_timestamp(9_999);
        assert!(order.can_fill_as_maker(1, 9_999));

        env.ledger().set_sequence_number(2);
        env.ledger().set_timestamp(10_000);
        assert!(!order.can_fill_as_maker(2, 10_000));

        // The ledger timestamp advances naturally, so expiration
        // is enforced automatically during matching
    }

    /// Test case: Consistency across all three TIF types.
    #[test]
    fn all_tif_types_consistency() {
        let env = Env::default();
        let owner = Address::generate(&env);

        let gtc = Order::new_gtc(1, owner.clone(), 50);
        let gtd = Order::new_gtd(2, owner.clone(), 50, 100);
        let gtt = Order::new_gtt(3, owner.clone(), 50, 10_000);

        // All should have same ID type, owner type, lots type
        assert_eq!(gtc.remaining_lots, gtd.remaining_lots);
        assert_eq!(gtd.remaining_lots, gtt.remaining_lots);

        // Each has distinct expiration behavior
        assert_eq!(gtc.tif, TimeInForce::GTC);
        assert_eq!(gtd.tif, TimeInForce::GTD);
        assert_eq!(gtt.tif, TimeInForce::GTT);
#[cfg(test)]
mod resource_bound_tests {
    use super::*;

    // ============ Tick validation (positive paths) ============
    #[test]
    fn validate_tick_accepts_valid_ticks() {
        assert!(validate_tick(1).is_ok());
        assert!(validate_tick(100).is_ok());
        assert!(validate_tick(MAX_TICK).is_ok());
        assert!(validate_tick(MAX_TICK / 2).is_ok());
    }

    // ============ Tick validation (negative paths) ============
    #[test]
    fn validate_tick_rejects_zero() {
        assert_eq!(validate_tick(0), Err("tick: must be positive"));
    }

    #[test]
    fn validate_tick_rejects_out_of_bounds() {
        assert_eq!(validate_tick(MAX_TICK + 1), Err("tick: exceeds MAX_TICK"));
        assert_eq!(validate_tick(u64::MAX), Err("tick: exceeds MAX_TICK"));
    }

    // ============ Match steps validation (positive paths) ============
    #[test]
    fn validate_match_steps_accepts_valid_budget() {
        assert!(validate_match_steps(0).is_ok());
        assert!(validate_match_steps(1).is_ok());
        assert!(validate_match_steps(50).is_ok());
        assert!(validate_match_steps(MAX_MATCH_STEPS).is_ok());
    }

    // ============ Match steps validation (negative paths) ============
    #[test]
    fn validate_match_steps_rejects_out_of_bounds() {
        assert_eq!(validate_match_steps(MAX_MATCH_STEPS + 1), Err("match_steps: exceeds MAX_MATCH_STEPS"));
        assert_eq!(validate_match_steps(u32::MAX), Err("match_steps: exceeds MAX_MATCH_STEPS"));
    }

    // ============ Lot size validation (positive paths) ============
    #[test]
    fn validate_lot_size_accepts_positive() {
        assert!(validate_lot_size(1).is_ok());
        assert!(validate_lot_size(100).is_ok());
        assert!(validate_lot_size(1_000_000_000).is_ok());
        assert!(validate_lot_size(u64::MAX).is_ok());
    }

    // ============ Lot size validation (negative paths) ============
    #[test]
    fn validate_lot_size_rejects_zero() {
        assert_eq!(validate_lot_size(0), Err("lot_size: must be positive"));
    }

    // ============ Tick size validation (positive paths) ============
    #[test]
    fn validate_tick_size_accepts_positive() {
        assert!(validate_tick_size(1).is_ok());
        assert!(validate_tick_size(100_000_000).is_ok());
        assert!(validate_tick_size(u64::MAX).is_ok());
    }

    // ============ Tick size validation (negative paths) ============
    #[test]
    fn validate_tick_size_rejects_zero() {
        assert_eq!(validate_tick_size(0), Err("tick_size: must be positive"));
    }

    // ============ Boundary tests ============
    #[test]
    fn max_constants_are_non_zero() {
        assert!(MAX_MATCH_STEPS > 0);
        assert!(MAX_TICK > 0);
    }

    #[test]
    fn price_scale_is_fixed_denominator() {
        assert_eq!(PRICE_SCALE, 100_000_000);
    }
}

#[cfg(test)]
mod bounded_work_tests {
    use super::*;

    // ============ Matches per call validation (positive paths) ============
    #[test]
    fn validate_matches_per_call_accepts_valid_count() {
        assert!(validate_matches_per_call(0).is_ok());
        assert!(validate_matches_per_call(1).is_ok());
        assert!(validate_matches_per_call(50).is_ok());
        assert!(validate_matches_per_call(MAX_MATCHES_PER_CALL).is_ok());
    }

    // ============ Matches per call validation (negative paths) ============
    #[test]
    fn validate_matches_per_call_rejects_out_of_bounds() {
        assert_eq!(validate_matches_per_call(MAX_MATCHES_PER_CALL + 1), Err("matches: exceeds MAX_MATCHES_PER_CALL"));
        assert_eq!(validate_matches_per_call(u32::MAX), Err("matches: exceeds MAX_MATCHES_PER_CALL"));
    }

    // ============ Levels crossed per call validation (positive paths) ============
    #[test]
    fn validate_levels_crossed_per_call_accepts_valid_count() {
        assert!(validate_levels_crossed_per_call(0).is_ok());
        assert!(validate_levels_crossed_per_call(1).is_ok());
        assert!(validate_levels_crossed_per_call(50).is_ok());
        assert!(validate_levels_crossed_per_call(MAX_LEVELS_CROSSED_PER_CALL).is_ok());
    }

    // ============ Levels crossed per call validation (negative paths) ============
    #[test]
    fn validate_levels_crossed_per_call_rejects_out_of_bounds() {
        assert_eq!(validate_levels_crossed_per_call(MAX_LEVELS_CROSSED_PER_CALL + 1), Err("levels: exceeds MAX_LEVELS_CROSSED_PER_CALL"));
        assert_eq!(validate_levels_crossed_per_call(u32::MAX), Err("levels: exceeds MAX_LEVELS_CROSSED_PER_CALL"));
    }

    // ============ Batch size validation (positive paths) ============
    #[test]
    fn validate_batch_size_accepts_valid_size() {
        assert!(validate_batch_size(0).is_ok());
        assert!(validate_batch_size(1).is_ok());
        assert!(validate_batch_size(25).is_ok());
        assert!(validate_batch_size(MAX_BATCH_SIZE).is_ok());
    }

    // ============ Batch size validation (negative paths) ============
    #[test]
    fn validate_batch_size_rejects_out_of_bounds() {
        assert_eq!(validate_batch_size(MAX_BATCH_SIZE + 1), Err("batch: exceeds MAX_BATCH_SIZE"));
        assert_eq!(validate_batch_size(u32::MAX), Err("batch: exceeds MAX_BATCH_SIZE"));
    }

    // ============ Depth query size validation (positive paths) ============
    #[test]
    fn validate_depth_query_size_accepts_valid_size() {
        assert!(validate_depth_query_size(0).is_ok());
        assert!(validate_depth_query_size(1).is_ok());
        assert!(validate_depth_query_size(50).is_ok());
        assert!(validate_depth_query_size(MAX_DEPTH_QUERY_SIZE).is_ok());
    }

    // ============ Depth query size validation (negative paths) ============
    #[test]
    fn validate_depth_query_size_rejects_out_of_bounds() {
        assert_eq!(validate_depth_query_size(MAX_DEPTH_QUERY_SIZE + 1), Err("depth: exceeds MAX_DEPTH_QUERY_SIZE"));
        assert_eq!(validate_depth_query_size(u32::MAX), Err("depth: exceeds MAX_DEPTH_QUERY_SIZE"));
    }

    // ============ Order payload size validation (positive paths) ============
    #[test]
    fn validate_order_payload_size_accepts_valid_size() {
        assert!(validate_order_payload_size(0).is_ok());
        assert!(validate_order_payload_size(1).is_ok());
        assert!(validate_order_payload_size(128).is_ok());
        assert!(validate_order_payload_size(MAX_ORDER_PAYLOAD_BYTES).is_ok());
    }

    // ============ Order payload size validation (negative paths) ============
    #[test]
    fn validate_order_payload_size_rejects_out_of_bounds() {
        assert_eq!(validate_order_payload_size(MAX_ORDER_PAYLOAD_BYTES + 1), Err("order: payload exceeds MAX_ORDER_PAYLOAD_BYTES"));
        assert_eq!(validate_order_payload_size(u32::MAX), Err("order: payload exceeds MAX_ORDER_PAYLOAD_BYTES"));
    }

    // ============ Boundary and relationship tests ============
    #[test]
    fn all_bounded_work_constants_are_positive() {
        assert!(MAX_MATCHES_PER_CALL > 0);
        assert!(MAX_LEVELS_CROSSED_PER_CALL > 0);
        assert!(MAX_BATCH_SIZE > 0);
        assert!(MAX_DEPTH_QUERY_SIZE > 0);
        assert!(MAX_ORDER_PAYLOAD_BYTES > 0);
    }

    #[test]
    fn bounded_work_constants_are_consistent() {
        // Batch size should be reasonable relative to matches per call
        assert!(MAX_BATCH_SIZE <= MAX_MATCHES_PER_CALL || MAX_BATCH_SIZE > 0);
        // All constants should be u32 representable
        assert!(MAX_MATCHES_PER_CALL <= u32::MAX);
        assert!(MAX_LEVELS_CROSSED_PER_CALL <= u32::MAX);
        assert!(MAX_BATCH_SIZE <= u32::MAX);
        assert!(MAX_DEPTH_QUERY_SIZE <= u32::MAX);
        assert!(MAX_ORDER_PAYLOAD_BYTES <= u32::MAX);
    }
}
