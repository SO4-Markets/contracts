//! Model-level CLOB test harness for Phase 8 issues #1041, #1045, #1046 and #1047.
//!
//! This repository does not contain a CLOB contract yet. Nothing in this file is wired to a
//! deployed or in-repo contract. It holds two small in-memory engines that share one set of
//! published arithmetic rules and are tested against each other and against invariants:
//!
//! - `Reference`: a deliberately simple engine (flat `Vec` of orders, linear scans) that is small
//!   enough to review by hand.
//! - `Book`: a storage-shaped engine (sorted price levels with FIFO queues and cached level
//!   totals) that supports bounded-work matching with a continuation, as a Soroban
//!   implementation would.
//!
//! The differential tests are therefore model-versus-model. When a real contract lands, its
//! adapter should implement `BookView` and be driven by `replay` in place of `Book`.
//!
//! Published arithmetic rules (all integer, no floating point):
//! - `notional = price * qty` (price in ticks, qty in lots), computed in `u128`.
//! - taker fee = `ceil(notional * taker_bps / 10_000)`, maker fee = `floor(notional * maker_bps / 10_000)`.
//! - a fill executes at the resting (maker) price; a buying taker escrows its limit price and is
//!   refunded `(limit - fill_price) * qty`.
//!
//! Protocol maximums (lower than the integer maximum, enforced before any arithmetic):
//! - `MAX_PRICE` = 10^12 ticks, `MAX_QTY` = `u64::MAX / 2` lots, fee rate <= 10_000 bps.
//! - The order sequence number may never reach `u64::MAX`.
//! - Aggregate quantity at one price level is a checked `u64`.

#![cfg(test)]

use std::collections::{BTreeMap, VecDeque};

const MAX_PRICE: u64 = 1_000_000_000_000;
const MAX_QTY: u64 = u64::MAX / 2;
const MAX_FEE_BPS: u32 = 10_000;
const BPS: u128 = 10_000;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Side {
    Buy,
    Sell,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Error {
    InvalidPrice,
    InvalidQty,
    InvalidFeeRate,
    Overflow,
    SequenceOverflow,
    LevelOverflow,
    WouldCross,
    UnknownOrder,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Order {
    id: u64,
    owner: u8,
    side: Side,
    price: u64,
    qty: u64,
}

#[derive(Clone, Copy, Debug)]
struct NewOrder {
    owner: u8,
    side: Side,
    price: u64,
    qty: u64,
    post_only: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Fill {
    maker_id: u64,
    taker_id: u64,
    price: u64,
    qty: u64,
}

#[derive(Clone, Copy, Debug)]
struct Params {
    taker_bps: u32,
    maker_bps: u32,
}

/// Per-owner `(base, quote)` balances plus accumulated protocol fees (quote).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct Ledger {
    accounts: BTreeMap<u8, (i128, i128)>,
    fees: i128,
}

impl Ledger {
    fn acct(&mut self, owner: u8) -> &mut (i128, i128) {
        self.accounts.entry(owner).or_insert((0, 0))
    }

    fn quote(&self, owner: u8) -> i128 {
        self.accounts.get(&owner).map_or(0, |a| a.1)
    }

    fn base(&self, owner: u8) -> i128 {
        self.accounts.get(&owner).map_or(0, |a| a.0)
    }
}

// ---------------------------------------------------------------------------
// Shared arithmetic and validation
// ---------------------------------------------------------------------------

fn notional(price: u64, qty: u64) -> Result<u128, Error> {
    (price as u128)
        .checked_mul(qty as u128)
        .ok_or(Error::Overflow)
}

fn to_i128(v: u128) -> Result<i128, Error> {
    i128::try_from(v).map_err(|_| Error::Overflow)
}

fn fee_ceil(n: u128, bps: u32) -> Result<u128, Error> {
    let p = n.checked_mul(bps as u128).ok_or(Error::Overflow)?;
    Ok(p / BPS + u128::from(p % BPS != 0))
}

fn fee_floor(n: u128, bps: u32) -> Result<u128, Error> {
    let p = n.checked_mul(bps as u128).ok_or(Error::Overflow)?;
    Ok(p / BPS)
}

fn check_params(p: &Params) -> Result<(), Error> {
    if p.taker_bps > MAX_FEE_BPS || p.maker_bps > MAX_FEE_BPS {
        return Err(Error::InvalidFeeRate);
    }
    Ok(())
}

fn check_new(o: &NewOrder, next_seq: u64) -> Result<(), Error> {
    if o.price == 0 || o.price > MAX_PRICE {
        return Err(Error::InvalidPrice);
    }
    if o.qty == 0 || o.qty > MAX_QTY {
        return Err(Error::InvalidQty);
    }
    if next_seq == u64::MAX {
        return Err(Error::SequenceOverflow);
    }
    Ok(())
}

fn limit_crosses(side: Side, limit: u64, maker_price: u64) -> bool {
    match side {
        Side::Buy => maker_price <= limit,
        Side::Sell => maker_price >= limit,
    }
}

/// Settles one fill. All fallible arithmetic happens before the ledger is touched.
fn settle(
    ledger: &mut Ledger,
    p: &Params,
    taker: &Order,
    maker: &Order,
    fill: &Fill,
) -> Result<(), Error> {
    let n = notional(fill.price, fill.qty)?;
    let taker_fee = to_i128(fee_ceil(n, p.taker_bps)?)?;
    let maker_fee = to_i128(fee_floor(n, p.maker_bps)?)?;
    let n = to_i128(n)?;
    let qty = fill.qty as i128;
    let (buyer, seller, buyer_fee, seller_fee) = match taker.side {
        Side::Buy => (taker.owner, maker.owner, taker_fee, maker_fee),
        Side::Sell => (maker.owner, taker.owner, maker_fee, taker_fee),
    };
    let (escrow, refund) = if taker.side == Side::Buy {
        let limit = notional(taker.price, fill.qty)?;
        let improvement = notional(
            taker.price.checked_sub(fill.price).ok_or(Error::Overflow)?,
            fill.qty,
        )?;
        (to_i128(limit)?, to_i128(improvement)?)
    } else {
        (n, 0)
    };
    let fees = taker_fee + maker_fee;

    let b = ledger.acct(buyer);
    b.0 += qty;
    b.1 -= escrow + buyer_fee;
    b.1 += refund;
    let s = ledger.acct(seller);
    s.0 -= qty;
    s.1 += n - seller_fee;
    ledger.fees += fees;
    Ok(())
}

// ---------------------------------------------------------------------------
// Common read-only view used by invariants and the differential comparison
// ---------------------------------------------------------------------------

trait BookView {
    fn open_orders(&self) -> Vec<Order>;
    fn best_bid(&self) -> Option<u64>;
    fn best_ask(&self) -> Option<u64>;
    fn level_qty(&self, side: Side, price: u64) -> u64;
    fn ledger(&self) -> &Ledger;
    fn next_seq(&self) -> u64;
}

fn crossed_report<E: BookView>(e: &E) -> Option<String> {
    let (b, a) = (e.best_bid()?, e.best_ask()?);
    if b < a {
        return None;
    }
    let implicated: Vec<Order> = e
        .open_orders()
        .into_iter()
        .filter(|o| (o.side == Side::Buy && o.price == b) || (o.side == Side::Sell && o.price == a))
        .collect();
    Some(format!(
        "crossed book: best_bid={b} best_ask={a} implicated={implicated:?}"
    ))
}

/// Debug helper for integration tests: panics with best bid/ask and the implicated orders.
fn assert_not_crossed<E: BookView>(e: &E, ctx: &str) {
    if let Some(report) = crossed_report(e) {
        panic!("{report} ({ctx})");
    }
}

fn assert_conserved(l: &Ledger) {
    let base: i128 = l.accounts.values().map(|a| a.0).sum();
    let quote: i128 = l.accounts.values().map(|a| a.1).sum::<i128>() + l.fees;
    assert_eq!((base, quote), (0, 0), "base/quote not conserved: {l:?}");
}

/// Best bid and best ask as `(price, quantity)`.
type TopOfBook = (Option<(u64, u64)>, Option<(u64, u64)>);

fn top_of_book<E: BookView>(e: &E) -> TopOfBook {
    (
        e.best_bid().map(|p| (p, e.level_qty(Side::Buy, p))),
        e.best_ask().map(|p| (p, e.level_qty(Side::Sell, p))),
    )
}

fn compare_state<A: BookView, B: BookView>(reference: &A, contract: &B) -> Result<(), String> {
    if reference.open_orders() != contract.open_orders() {
        return Err(format!(
            "open orders differ: reference={:?} model={:?}",
            reference.open_orders(),
            contract.open_orders()
        ));
    }
    if reference.ledger() != contract.ledger() {
        return Err(format!(
            "balances/fees differ: reference={:?} model={:?}",
            reference.ledger(),
            contract.ledger()
        ));
    }
    if reference.next_seq() != contract.next_seq() {
        return Err("sequence numbers differ".to_string());
    }
    let (r, c) = (top_of_book(reference), top_of_book(contract));
    if r != c {
        return Err(format!("top of book differs: reference={r:?} model={c:?}"));
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Reference engine: flat list, linear scans, price-time priority
// ---------------------------------------------------------------------------

#[derive(Clone, Debug)]
struct Reference {
    orders: Vec<Order>,
    next_seq: u64,
    params: Params,
    ledger: Ledger,
}

impl Reference {
    fn new(params: Params) -> Result<Self, Error> {
        check_params(&params)?;
        Ok(Self {
            orders: Vec::new(),
            next_seq: 0,
            params,
            ledger: Ledger::default(),
        })
    }

    fn place(&mut self, o: NewOrder) -> Result<Vec<Fill>, Error> {
        check_new(&o, self.next_seq)?;
        if o.post_only
            && self
                .orders
                .iter()
                .any(|m| m.side != o.side && limit_crosses(o.side, o.price, m.price))
        {
            return Err(Error::WouldCross);
        }
        let mut next = self.clone();
        let mut taker = Order {
            id: next.next_seq,
            owner: o.owner,
            side: o.side,
            price: o.price,
            qty: o.qty,
        };
        next.next_seq += 1;
        let params = next.params;
        let mut fills = Vec::new();
        while taker.qty > 0 {
            let best = next
                .orders
                .iter()
                .enumerate()
                .filter(|(_, m)| {
                    m.side != taker.side && limit_crosses(taker.side, taker.price, m.price)
                })
                .min_by_key(|(_, m)| match taker.side {
                    Side::Buy => (m.price, m.id),
                    Side::Sell => (u64::MAX - m.price, m.id),
                })
                .map(|(i, _)| i);
            let Some(i) = best else { break };
            let maker = next.orders[i];
            let qty = taker.qty.min(maker.qty);
            let fill = Fill {
                maker_id: maker.id,
                taker_id: taker.id,
                price: maker.price,
                qty,
            };
            settle(&mut next.ledger, &params, &taker, &maker, &fill)?;
            next.orders[i].qty -= qty;
            if next.orders[i].qty == 0 {
                next.orders.remove(i);
            }
            taker.qty -= qty;
            fills.push(fill);
        }
        if taker.qty > 0 {
            next.orders.push(taker);
        }
        *self = next;
        Ok(fills)
    }

    fn cancel(&mut self, id: u64, owner: u8) -> Result<(), Error> {
        match self
            .orders
            .iter()
            .position(|o| o.id == id && o.owner == owner)
        {
            Some(i) => {
                self.orders.remove(i);
                Ok(())
            }
            None => Err(Error::UnknownOrder),
        }
    }
}

impl BookView for Reference {
    fn open_orders(&self) -> Vec<Order> {
        let mut v = self.orders.clone();
        v.sort_by_key(|o| o.id);
        v
    }
    fn best_bid(&self) -> Option<u64> {
        self.orders
            .iter()
            .filter(|o| o.side == Side::Buy)
            .map(|o| o.price)
            .max()
    }
    fn best_ask(&self) -> Option<u64> {
        self.orders
            .iter()
            .filter(|o| o.side == Side::Sell)
            .map(|o| o.price)
            .min()
    }
    fn level_qty(&self, side: Side, price: u64) -> u64 {
        self.orders
            .iter()
            .filter(|o| o.side == side && o.price == price)
            .map(|o| o.qty)
            .sum()
    }
    fn ledger(&self) -> &Ledger {
        &self.ledger
    }
    fn next_seq(&self) -> u64 {
        self.next_seq
    }
}

// ---------------------------------------------------------------------------
// Storage-shaped engine: sorted levels, FIFO queues, bounded matching
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, Default)]
struct Level {
    total: u64,
    queue: VecDeque<Order>,
}

#[derive(Clone, Debug)]
struct Book {
    bids: BTreeMap<u64, Level>,
    asks: BTreeMap<u64, Level>,
    next_seq: u64,
    params: Params,
    ledger: Ledger,
}

/// Result of one bounded matching pass. `continuation` is the unfilled, still-crossing
/// remainder when the fill budget ran out; it is never rested, so the book cannot stay crossed.
struct Outcome {
    fills: Vec<Fill>,
    continuation: Option<Order>,
}

fn remove_order(map: &mut BTreeMap<u64, Level>, id: u64, owner: u8) -> bool {
    let found = map.iter().find_map(|(p, l)| {
        l.queue
            .iter()
            .position(|o| o.id == id && o.owner == owner)
            .map(|i| (*p, i))
    });
    let Some((p, i)) = found else { return false };
    let level = map.get_mut(&p).expect("level exists");
    let o = level.queue.remove(i).expect("order exists");
    level.total -= o.qty;
    if level.queue.is_empty() {
        map.remove(&p);
    }
    true
}

impl Book {
    fn new(params: Params) -> Result<Self, Error> {
        check_params(&params)?;
        Ok(Self {
            bids: BTreeMap::new(),
            asks: BTreeMap::new(),
            next_seq: 0,
            params,
            ledger: Ledger::default(),
        })
    }

    fn with_next_seq(mut self, next_seq: u64) -> Self {
        self.next_seq = next_seq;
        self
    }

    fn best_opposite(&self, side: Side) -> Option<u64> {
        match side {
            Side::Buy => self.asks.keys().next().copied(),
            Side::Sell => self.bids.keys().next_back().copied(),
        }
    }

    fn place(&mut self, o: NewOrder, max_fills: usize) -> Result<Outcome, Error> {
        check_new(&o, self.next_seq)?;
        if o.post_only
            && self
                .best_opposite(o.side)
                .is_some_and(|p| limit_crosses(o.side, o.price, p))
        {
            return Err(Error::WouldCross);
        }
        let mut next = self.clone();
        let order = Order {
            id: next.next_seq,
            owner: o.owner,
            side: o.side,
            price: o.price,
            qty: o.qty,
        };
        next.next_seq += 1;
        let out = next.run(order, max_fills.max(1))?;
        *self = next;
        Ok(out)
    }

    /// Continues a taker returned as `Outcome::continuation`, keeping its order id.
    fn resume(&mut self, order: Order, max_fills: usize) -> Result<Outcome, Error> {
        let mut next = self.clone();
        let out = next.run(order, max_fills.max(1))?;
        *self = next;
        Ok(out)
    }

    fn run(&mut self, mut order: Order, max_fills: usize) -> Result<Outcome, Error> {
        let params = self.params;
        let mut fills = Vec::new();
        while order.qty > 0 {
            let price = match self.best_opposite(order.side) {
                Some(p) if limit_crosses(order.side, order.price, p) => p,
                _ => break,
            };
            if fills.len() == max_fills {
                return Ok(Outcome {
                    fills,
                    continuation: Some(order),
                });
            }
            let opposite = match order.side {
                Side::Buy => &mut self.asks,
                Side::Sell => &mut self.bids,
            };
            let level = opposite.get_mut(&price).expect("level exists");
            let maker = *level.queue.front().expect("level is non-empty");
            let qty = order.qty.min(maker.qty);
            let fill = Fill {
                maker_id: maker.id,
                taker_id: order.id,
                price,
                qty,
            };
            settle(&mut self.ledger, &params, &order, &maker, &fill)?;
            level.total -= qty;
            if qty == maker.qty {
                level.queue.pop_front();
            } else {
                level.queue.front_mut().expect("level is non-empty").qty -= qty;
            }
            if level.queue.is_empty() {
                opposite.remove(&price);
            }
            order.qty -= qty;
            fills.push(fill);
        }
        if order.qty > 0 {
            let own = match order.side {
                Side::Buy => &mut self.bids,
                Side::Sell => &mut self.asks,
            };
            let level = own.entry(order.price).or_default();
            level.total = level
                .total
                .checked_add(order.qty)
                .ok_or(Error::LevelOverflow)?;
            level.queue.push_back(order);
        }
        Ok(Outcome {
            fills,
            continuation: None,
        })
    }

    fn cancel(&mut self, id: u64, owner: u8) -> Result<(), Error> {
        if remove_order(&mut self.bids, id, owner) || remove_order(&mut self.asks, id, owner) {
            Ok(())
        } else {
            Err(Error::UnknownOrder)
        }
    }

    fn check_structure(&self) -> Result<(), String> {
        for (side, map) in [(Side::Buy, &self.bids), (Side::Sell, &self.asks)] {
            for (price, level) in map {
                let sum: u64 = level.queue.iter().map(|o| o.qty).sum();
                if level.queue.is_empty() || level.total != sum {
                    return Err(format!(
                        "bad level {side:?}@{price}: total={} queue={:?}",
                        level.total, level.queue
                    ));
                }
                if level
                    .queue
                    .iter()
                    .any(|o| o.side != side || o.price != *price || o.qty == 0)
                {
                    return Err(format!("misfiled order at {side:?}@{price}"));
                }
            }
        }
        Ok(())
    }
}

impl BookView for Book {
    fn open_orders(&self) -> Vec<Order> {
        let mut v: Vec<Order> = self
            .bids
            .values()
            .chain(self.asks.values())
            .flat_map(|l| l.queue.iter().copied())
            .collect();
        v.sort_by_key(|o| o.id);
        v
    }
    fn best_bid(&self) -> Option<u64> {
        self.bids.keys().next_back().copied()
    }
    fn best_ask(&self) -> Option<u64> {
        self.asks.keys().next().copied()
    }
    fn level_qty(&self, side: Side, price: u64) -> u64 {
        let map = match side {
            Side::Buy => &self.bids,
            Side::Sell => &self.asks,
        };
        map.get(&price).map_or(0, |l| l.total)
    }
    fn ledger(&self) -> &Ledger {
        &self.ledger
    }
    fn next_seq(&self) -> u64 {
        self.next_seq
    }
}

/// Places an order and keeps resuming its continuation until matching is complete.
fn place_to_completion(book: &mut Book, o: NewOrder, max_fills: usize) -> Result<Vec<Fill>, Error> {
    let mut out = book.place(o, max_fills)?;
    let mut fills = std::mem::take(&mut out.fills);
    while let Some(c) = out.continuation {
        out = book.resume(c, max_fills)?;
        fills.append(&mut out.fills);
    }
    Ok(fills)
}

// ---------------------------------------------------------------------------
// Random traces and replay
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, Debug)]
enum Action {
    Place(NewOrder),
    Cancel { id: u64, owner: u8 },
}

struct Rng(u64);

impl Rng {
    fn step(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.0 = x;
        x
    }
    fn below(&mut self, n: u64) -> u64 {
        self.step() % n
    }
}

fn generate_trace(seed: u64, steps: usize) -> Vec<Action> {
    let mut rng = Rng(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1);
    (0..steps)
        .map(|_| {
            if rng.below(5) == 0 {
                Action::Cancel {
                    id: rng.below(60),
                    owner: rng.below(4) as u8,
                }
            } else {
                Action::Place(NewOrder {
                    owner: rng.below(4) as u8,
                    side: if rng.below(2) == 0 {
                        Side::Buy
                    } else {
                        Side::Sell
                    },
                    price: 1 + rng.below(12),
                    qty: 1 + rng.below(8),
                    post_only: rng.below(6) == 0,
                })
            }
        })
        .collect()
}

const PARAMS: Params = Params {
    taker_bps: 30,
    maker_bps: 10,
};

/// Feeds one trace to both engines. On the first mismatch the error holds the failing step and
/// the exact trace prefix, so the failure can be replayed by passing that prefix back in.
fn replay(trace: &[Action], max_fills: usize) -> Result<(Book, Reference), String> {
    let mut reference = Reference::new(PARAMS).expect("valid params");
    let mut book = Book::new(PARAMS).expect("valid params");
    for (step, action) in trace.iter().enumerate() {
        let fail = |what: String| {
            format!(
                "{what} at step {step} action {action:?} (max_fills={max_fills})\nreplay trace: {:?}",
                &trace[..=step]
            )
        };
        match *action {
            Action::Place(o) => {
                let r = reference.place(o);
                let b = place_to_completion(&mut book, o, max_fills);
                if r != b {
                    return Err(fail(format!(
                        "fills/result differ: reference={r:?} model={b:?}"
                    )));
                }
            }
            Action::Cancel { id, owner } => {
                let r = reference.cancel(id, owner);
                let b = book.cancel(id, owner);
                if r != b {
                    return Err(fail(format!("cancel differs: reference={r:?} model={b:?}")));
                }
            }
        }
        compare_state(&reference, &book).map_err(fail)?;
        book.check_structure().map_err(fail)?;
        if let Some(report) = crossed_report(&book) {
            return Err(fail(report));
        }
        let (base, quote) = book
            .ledger()
            .accounts
            .values()
            .fold((0i128, book.ledger().fees), |(b, q), a| (b + a.0, q + a.1));
        if (base, quote) != (0, 0) {
            return Err(fail(format!("ledger not conserved: {:?}", book.ledger())));
        }
    }
    Ok((book, reference))
}

fn snapshot(b: &Book) -> (Vec<Order>, Ledger, u64) {
    (b.open_orders(), b.ledger.clone(), b.next_seq)
}

fn order(owner: u8, side: Side, price: u64, qty: u64) -> NewOrder {
    NewOrder {
        owner,
        side,
        price,
        qty,
        post_only: false,
    }
}

// ---------------------------------------------------------------------------
// #1041: the resting book can never remain crossed
// ---------------------------------------------------------------------------

#[test]
fn resting_book_is_never_crossed_after_random_sequences() {
    for seed in 1..=100u64 {
        let trace = generate_trace(seed, 200);
        let mut book = Book::new(PARAMS).unwrap();
        for (step, action) in trace.iter().enumerate() {
            match *action {
                Action::Place(o) => {
                    let _ = book.place(o, usize::MAX);
                }
                Action::Cancel { id, owner } => {
                    let _ = book.cancel(id, owner);
                }
            }
            assert_not_crossed(&book, &format!("seed={seed} step={step}"));
        }
    }
}

#[test]
fn resource_bound_matching_never_rests_crossed_liquidity() {
    for max_fills in 1..=3usize {
        for seed in 1..=100u64 {
            let mut book = Book::new(PARAMS).unwrap();
            for (step, action) in generate_trace(seed, 200).iter().enumerate() {
                match *action {
                    // The continuation is intentionally dropped: the remainder must not rest.
                    Action::Place(o) => {
                        let _ = book.place(o, max_fills);
                    }
                    Action::Cancel { id, owner } => {
                        let _ = book.cancel(id, owner);
                    }
                }
                let ctx = format!("seed={seed} step={step} max_fills={max_fills}");
                assert_not_crossed(&book, &ctx);
                book.check_structure()
                    .unwrap_or_else(|e| panic!("{e} ({ctx})"));
            }
        }
    }
}

#[test]
fn post_only_at_equal_price_is_rejected_and_not_rested() {
    let mut book = Book::new(PARAMS).unwrap();
    book.place(order(1, Side::Sell, 10, 5), usize::MAX).unwrap();
    let before = snapshot(&book);
    let mut po = order(2, Side::Buy, 10, 5);
    po.post_only = true;
    assert_eq!(book.place(po, usize::MAX).err(), Some(Error::WouldCross));
    assert_eq!(snapshot(&book), before);
    po.price = 9;
    assert!(book.place(po, usize::MAX).unwrap().fills.is_empty());
    assert_eq!((book.best_bid(), book.best_ask()), (Some(9), Some(10)));
    assert_not_crossed(&book, "post-only below ask");
}

#[test]
#[should_panic(expected = "best_bid=10 best_ask=10")]
fn invariant_failure_prints_best_prices_and_implicated_orders() {
    let mut book = Book::new(PARAMS).unwrap();
    let bid = Order {
        id: 0,
        owner: 1,
        side: Side::Buy,
        price: 10,
        qty: 1,
    };
    let ask = Order {
        id: 1,
        owner: 2,
        side: Side::Sell,
        price: 10,
        qty: 1,
    };
    book.bids.insert(
        10,
        Level {
            total: 1,
            queue: VecDeque::from([bid]),
        },
    );
    book.asks.insert(
        10,
        Level {
            total: 1,
            queue: VecDeque::from([ask]),
        },
    );
    assert_not_crossed(&book, "hand-built crossed book");
}

// ---------------------------------------------------------------------------
// #1045: differential fuzz of the storage-shaped model against the reference
// ---------------------------------------------------------------------------

#[test]
fn differential_fuzz_matches_reference_unbounded() {
    for seed in 1..=300u64 {
        replay(&generate_trace(seed, 200), usize::MAX)
            .unwrap_or_else(|m| panic!("seed {seed}: {m}"));
    }
}

#[test]
fn bounded_continuation_converges_to_reference_result() {
    for max_fills in 1..=3usize {
        for seed in 1..=300u64 {
            replay(&generate_trace(seed, 200), max_fills)
                .unwrap_or_else(|m| panic!("seed {seed}: {m}"));
        }
    }
}

#[test]
fn replay_is_deterministic() {
    let trace = generate_trace(7, 200);
    let (a, ra) = replay(&trace, 2).unwrap();
    let (b, rb) = replay(&trace, 2).unwrap();
    assert_eq!(snapshot(&a), snapshot(&b));
    assert_eq!(ra.open_orders(), rb.open_orders());
}

// ---------------------------------------------------------------------------
// #1046: rounding and dust boundaries
// ---------------------------------------------------------------------------

#[test]
fn fee_rounding_boundary_vectors() {
    // (notional, bps, expected ceil, expected floor)
    let vectors: [(u128, u32, u128, u128); 14] = [
        (0, 30, 0, 0),
        (1, 0, 0, 0),
        (1, 1, 1, 0),
        (9_999, 1, 1, 0),
        (10_000, 1, 1, 1),
        (10_001, 1, 2, 1),
        (3_333, 3, 1, 0),
        (3_334, 3, 2, 1),
        (333, 30, 1, 0),
        (334, 30, 2, 1),
        (1, 10_000, 1, 1),
        (9_999, 10_000, 9_999, 9_999),
        (10_001, 9_999, 10_000, 9_999),
        (u64::MAX as u128, 10_000, u64::MAX as u128, u64::MAX as u128),
    ];
    for (n, bps, ceil, floor) in vectors {
        assert_eq!(fee_ceil(n, bps), Ok(ceil), "ceil n={n} bps={bps}");
        assert_eq!(fee_floor(n, bps), Ok(floor), "floor n={n} bps={bps}");
    }
}

#[test]
fn fragmenting_a_fee_cannot_beat_the_aggregate_by_more_than_one_atom_per_part() {
    for price in [1u64, 3, 7, 333, 9_999] {
        for total in 1..=40u64 {
            let agg = notional(price, total).unwrap();
            // Every two-way split and the all-ones split.
            let mut splits: Vec<Vec<u64>> = (1..total).map(|a| vec![a, total - a]).collect();
            splits.push(vec![1; total as usize]);
            for parts in splits {
                let n = parts.len() as u128;
                let taker: u128 = parts
                    .iter()
                    .map(|q| fee_ceil(notional(price, *q).unwrap(), 30).unwrap())
                    .sum();
                let maker: u128 = parts
                    .iter()
                    .map(|q| fee_floor(notional(price, *q).unwrap(), 10).unwrap())
                    .sum();
                let (agg_taker, agg_maker) =
                    (fee_ceil(agg, 30).unwrap(), fee_floor(agg, 10).unwrap());
                assert!(taker >= agg_taker && taker - agg_taker < n);
                assert!(maker <= agg_maker && agg_maker - maker < n);
            }
        }
    }
}

#[test]
fn fragmented_fills_match_aggregate_fill_within_bound() {
    for price in [7u64, 333, 9_999] {
        let run = |split: bool| {
            let mut b = Book::new(PARAMS).unwrap();
            if split {
                for _ in 0..10 {
                    b.place(order(2, Side::Sell, price, 1), 8).unwrap();
                }
            } else {
                b.place(order(2, Side::Sell, price, 10), 8).unwrap();
            }
            let fills = place_to_completion(&mut b, order(1, Side::Buy, price, 10), 16).unwrap();
            assert_eq!(fills.iter().map(|f| f.qty).sum::<u64>(), 10);
            assert_conserved(b.ledger());
            assert!(b.open_orders().is_empty());
            (
                b.ledger.quote(1),
                b.ledger.quote(2),
                b.ledger.fees,
                fills.len() as i128,
            )
        };
        let (a_taker, a_maker, a_fees, _) = run(false);
        let (s_taker, s_maker, s_fees, n) = run(true);
        // Taker never pays less, makers never receive less, each by under one atom per fill.
        assert!((0..n).contains(&(a_taker - s_taker)), "price {price}");
        assert!((0..n).contains(&(s_maker - a_maker)), "price {price}");
        assert!((s_fees - a_fees).abs() < n, "price {price}");
    }
}

#[test]
fn price_improvement_refund_and_fees_settle_together() {
    let mut book = Book::new(PARAMS).unwrap();
    book.place(order(2, Side::Sell, 100, 5), usize::MAX)
        .unwrap();
    let fills = book
        .place(order(1, Side::Buy, 105, 5), usize::MAX)
        .unwrap()
        .fills;
    assert_eq!(fills.len(), 1);
    assert_eq!(fills[0].price, 100);
    // notional 500: taker fee ceil(1.5)=2, maker fee floor(0.5)=0, refund 25 of the 525 escrow.
    assert_eq!(book.ledger.quote(1), -502);
    assert_eq!(book.ledger.quote(2), 500);
    assert_eq!((book.ledger.base(1), book.ledger.base(2)), (5, -5));
    assert_eq!(book.ledger.fees, 2);
    assert_conserved(book.ledger());
}

#[test]
fn boundary_matrix_conserves_value_and_keeps_proceeds_non_negative() {
    let prices = [
        1u64,
        2,
        3,
        7,
        9_999,
        10_000,
        10_001,
        MAX_PRICE - 1,
        MAX_PRICE,
    ];
    let qtys = [1u64, 2, 3, 9_999, 10_001, MAX_QTY - 1, MAX_QTY];
    let rates = [(0u32, 0u32), (1, 1), (30, 10), (9_999, 1), (10_000, 10_000)];
    for &price in &prices {
        for &qty in &qtys {
            for &(taker_bps, maker_bps) in &rates {
                let mut b = Book::new(Params {
                    taker_bps,
                    maker_bps,
                })
                .unwrap();
                b.place(order(2, Side::Sell, price, qty), usize::MAX)
                    .unwrap();
                let fills = b
                    .place(order(1, Side::Buy, price, qty), usize::MAX)
                    .unwrap()
                    .fills;
                assert_eq!(fills.len(), 1);
                assert_conserved(b.ledger());
                let exact = to_i128(notional(price, qty).unwrap()).unwrap();
                assert!(b.ledger.quote(2) >= 0, "seller proceeds negative");
                assert!(-b.ledger.quote(1) >= exact, "buyer paid less than notional");
                assert_eq!(
                    (b.ledger.base(1), b.ledger.base(2)),
                    (qty as i128, -(qty as i128))
                );
            }
        }
    }
}

// ---------------------------------------------------------------------------
// #1047: extreme sizes, prices, and sequence values
// ---------------------------------------------------------------------------

#[test]
fn maximum_valid_vector_settles_exactly() {
    let mut b = Book::new(PARAMS).unwrap();
    b.place(order(2, Side::Sell, MAX_PRICE, MAX_QTY), usize::MAX)
        .unwrap();
    let fills = b
        .place(order(1, Side::Buy, MAX_PRICE, MAX_QTY), usize::MAX)
        .unwrap()
        .fills;
    assert_eq!(fills.len(), 1);
    let n = MAX_PRICE as u128 * MAX_QTY as u128;
    let taker_fee = (n * 30).div_ceil(10_000);
    let maker_fee = n * 10 / 10_000;
    assert_eq!(b.ledger.quote(1), -((n + taker_fee) as i128));
    assert_eq!(b.ledger.quote(2), (n - maker_fee) as i128);
    assert_eq!(b.ledger.fees, (taker_fee + maker_fee) as i128);
    assert_conserved(b.ledger());
    assert!(b.open_orders().is_empty());
    b.check_structure().unwrap();
}

#[test]
fn out_of_domain_inputs_return_typed_errors_and_leave_state_unchanged() {
    let mut b = Book::new(PARAMS).unwrap();
    b.place(order(2, Side::Sell, 10, 5), usize::MAX).unwrap();
    let before = snapshot(&b);
    let cases = [
        (order(1, Side::Buy, 0, 1), Error::InvalidPrice),
        (order(1, Side::Buy, MAX_PRICE + 1, 1), Error::InvalidPrice),
        (order(1, Side::Buy, u64::MAX, 1), Error::InvalidPrice),
        (order(1, Side::Buy, 10, 0), Error::InvalidQty),
        (order(1, Side::Buy, 10, MAX_QTY + 1), Error::InvalidQty),
        (order(1, Side::Buy, 10, u64::MAX), Error::InvalidQty),
    ];
    for (o, expected) in cases {
        assert_eq!(b.place(o, usize::MAX).err(), Some(expected), "{o:?}");
        assert_eq!(snapshot(&b), before);
    }
    assert_eq!(b.cancel(99, 1).err(), Some(Error::UnknownOrder));
    assert_eq!(b.cancel(0, 1).err(), Some(Error::UnknownOrder));
    assert_eq!(snapshot(&b), before);
    assert_eq!(
        Book::new(Params {
            taker_bps: MAX_FEE_BPS + 1,
            maker_bps: 0
        })
        .err(),
        Some(Error::InvalidFeeRate)
    );
    assert_eq!(
        Reference::new(Params {
            taker_bps: 0,
            maker_bps: u32::MAX
        })
        .err(),
        Some(Error::InvalidFeeRate)
    );
}

#[test]
fn sequence_number_near_overflow_is_rejected_without_wraparound() {
    let mut b = Book::new(PARAMS).unwrap().with_next_seq(u64::MAX - 1);
    b.place(order(1, Side::Sell, 10, 1), usize::MAX).unwrap();
    assert_eq!(b.open_orders()[0].id, u64::MAX - 1);
    assert_eq!(b.next_seq, u64::MAX);
    let before = snapshot(&b);
    assert_eq!(
        b.place(order(1, Side::Sell, 11, 1), usize::MAX).err(),
        Some(Error::SequenceOverflow)
    );
    assert_eq!(snapshot(&b), before);
}

#[test]
fn aggregate_level_quantity_overflow_is_rejected_atomically() {
    let mut b = Book::new(PARAMS).unwrap();
    b.place(order(1, Side::Sell, 10, MAX_QTY), usize::MAX)
        .unwrap();
    b.place(order(1, Side::Sell, 10, MAX_QTY), usize::MAX)
        .unwrap();
    assert_eq!(b.level_qty(Side::Sell, 10), u64::MAX - 1);
    let before = snapshot(&b);
    assert_eq!(
        b.place(order(1, Side::Sell, 10, 2), usize::MAX).err(),
        Some(Error::LevelOverflow)
    );
    assert_eq!(snapshot(&b), before);
    b.check_structure().unwrap();
}

#[test]
fn intermediate_multiplication_overflow_is_typed_and_leaves_ledger_unchanged() {
    let n = notional(u64::MAX, u64::MAX).unwrap();
    assert_eq!(fee_ceil(n, 30), Err(Error::Overflow));
    assert_eq!(fee_floor(n, 30), Err(Error::Overflow));
    assert_eq!(to_i128(n), Err(Error::Overflow));

    let maker = Order {
        id: 0,
        owner: 2,
        side: Side::Sell,
        price: u64::MAX,
        qty: u64::MAX,
    };
    let taker = Order {
        id: 1,
        owner: 1,
        side: Side::Buy,
        ..maker
    };
    let fill = Fill {
        maker_id: 0,
        taker_id: 1,
        price: u64::MAX,
        qty: u64::MAX,
    };
    let mut ledger = Ledger::default();
    assert_eq!(
        settle(&mut ledger, &PARAMS, &taker, &maker, &fill),
        Err(Error::Overflow)
    );
    assert_eq!(ledger, Ledger::default());
}
