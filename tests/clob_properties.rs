//! Self-contained property tests for CLOB accounting and matching rules
//! (backlog items #105-#107: conservation, strict FIFO, best-price execution).
//!
//! The repository has no on-chain order book yet, so nothing here is wired to a
//! contract. The file holds a small in-memory book (`Engine`), an independent
//! naive reference (`reference_fills`), and one seeded action generator shared by
//! all properties. Once a real CLOB exists, `Engine` is the piece to swap for it.
//! Failing sequences are shrunk and printed with their seed so they can be replayed.

#![cfg(test)]

use std::collections::{BTreeMap, VecDeque};

const TRADERS: usize = 4;
const FEE_BPS: u128 = 30;
const NO_EXPIRY: u64 = u64::MAX;

type Balances = [u128; TRADERS];

#[derive(Clone, Copy, Debug, PartialEq)]
enum Side {
    Bid,
    Ask,
}

#[derive(Clone, Debug, PartialEq)]
struct Order {
    id: u64,
    seq: u64,
    trader: usize,
    side: Side,
    price: u128,
    qty: u128,
    expiry: u64,
}

#[derive(Clone, Debug, PartialEq)]
struct Fill {
    maker_id: u64,
    price: u128,
    qty: u128,
}

#[derive(Clone, Debug, PartialEq)]
enum Action {
    Deposit {
        trader: usize,
        base: bool,
        amount: u128,
    },
    Withdraw {
        trader: usize,
        base: bool,
        amount: u128,
    },
    Place {
        trader: usize,
        side: Side,
        price: u128,
        qty: u128,
        ttl: u64,
    },
    Cancel {
        id: u64,
    },
    Reduce {
        id: u64,
        new_qty: u128,
    },
    WithdrawFees,
    Advance {
        secs: u64,
    },
}

#[derive(Debug, PartialEq)]
enum Reject {
    ZeroAmount,
    Insufficient,
    UnknownOrder,
    BadReduce,
}

/// Seeded xorshift generator so every failure is replayable from its seed.
struct Rng(u64);

impl Rng {
    fn new(seed: u64) -> Self {
        Rng(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1)
    }
    fn next_u64(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
    fn below(&mut self, n: u64) -> u64 {
        self.next_u64() % n
    }
    fn range(&mut self, lo: u128, hi: u128) -> u128 {
        if hi <= lo {
            return lo;
        }
        lo + u128::from(self.next_u64()) % (hi - lo + 1)
    }
    fn trader(&mut self) -> usize {
        self.below(TRADERS as u64) as usize
    }
    fn side(&mut self) -> Side {
        if self.below(2) == 0 {
            Side::Bid
        } else {
            Side::Ask
        }
    }
}

/// Shared generator: deposits, placements (which may fill), cancellations,
/// reductions, fee withdrawals, user withdrawals and clock advances.
fn generate_actions(seed: u64, len: usize) -> Vec<Action> {
    let mut rng = Rng::new(seed);
    (0..len)
        .map(|_| match rng.below(100) {
            0..=14 => Action::Deposit {
                trader: rng.trader(),
                base: rng.below(2) == 0,
                amount: rng.range(1, 2_000),
            },
            15..=44 | 85..=99 => Action::Place {
                trader: rng.trader(),
                side: rng.side(),
                price: rng.range(95, 105),
                qty: rng.range(1, 20),
                ttl: rng.below(3) * 5,
            },
            45..=54 => Action::Cancel {
                id: rng.below(40) + 1,
            },
            55..=64 => Action::Reduce {
                id: rng.below(40) + 1,
                new_qty: rng.range(0, 19),
            },
            65..=69 => Action::WithdrawFees,
            70..=79 => Action::Withdraw {
                trader: rng.trader(),
                base: rng.below(2) == 0,
                amount: rng.range(1, 500),
            },
            _ => Action::Advance {
                secs: rng.below(5) + 1,
            },
        })
        .collect()
}

#[derive(Clone, Debug, Default, PartialEq)]
struct Engine {
    now: u64,
    next_id: u64,
    next_seq: u64,
    free_base: Balances,
    free_quote: Balances,
    bids: BTreeMap<u128, VecDeque<Order>>,
    asks: BTreeMap<u128, VecDeque<Order>>,
    fee_bucket: u128,
    dust_bucket: u128,
    base_in: u128,
    base_out: u128,
    quote_in: u128,
    quote_out: u128,
}

impl Engine {
    fn new() -> Self {
        Engine {
            next_id: 1,
            ..Engine::default()
        }
    }

    fn apply(&mut self, action: &Action) -> Result<Vec<Fill>, Reject> {
        match *action {
            Action::Deposit {
                trader,
                base,
                amount,
            } => {
                if amount == 0 {
                    return Err(Reject::ZeroAmount);
                }
                if base {
                    self.free_base[trader] += amount;
                    self.base_in += amount;
                } else {
                    self.free_quote[trader] += amount;
                    self.quote_in += amount;
                }
                Ok(Vec::new())
            }
            Action::Withdraw {
                trader,
                base,
                amount,
            } => {
                let (free, out) = if base {
                    (&mut self.free_base[trader], &mut self.base_out)
                } else {
                    (&mut self.free_quote[trader], &mut self.quote_out)
                };
                if amount == 0 {
                    return Err(Reject::ZeroAmount);
                }
                if *free < amount {
                    return Err(Reject::Insufficient);
                }
                *free -= amount;
                *out += amount;
                Ok(Vec::new())
            }
            Action::Place {
                trader,
                side,
                price,
                qty,
                ttl,
            } => self.place(trader, side, price, qty, ttl),
            Action::Cancel { id } => {
                let order = self.remove_order(id).ok_or(Reject::UnknownOrder)?;
                self.refund(&order, order.qty);
                Ok(Vec::new())
            }
            Action::Reduce { id, new_qty } => {
                let current = self
                    .orders()
                    .into_iter()
                    .find(|o| o.id == id)
                    .ok_or(Reject::UnknownOrder)?;
                if new_qty == 0 || new_qty >= current.qty {
                    return Err(Reject::BadReduce);
                }
                // Reduce in place: the order keeps its queue position.
                let level = match current.side {
                    Side::Bid => self.bids.get_mut(&current.price),
                    Side::Ask => self.asks.get_mut(&current.price),
                }
                .expect("level exists");
                level
                    .iter_mut()
                    .find(|o| o.id == id)
                    .expect("order exists")
                    .qty = new_qty;
                self.refund(&current, current.qty - new_qty);
                Ok(Vec::new())
            }
            Action::WithdrawFees => {
                if self.fee_bucket == 0 {
                    return Err(Reject::ZeroAmount);
                }
                self.quote_out += self.fee_bucket;
                self.fee_bucket = 0;
                Ok(Vec::new())
            }
            Action::Advance { secs } => {
                self.now += secs;
                Ok(Vec::new())
            }
        }
    }

    fn place(
        &mut self,
        trader: usize,
        side: Side,
        price: u128,
        qty: u128,
        ttl: u64,
    ) -> Result<Vec<Fill>, Reject> {
        if price == 0 || qty == 0 {
            return Err(Reject::ZeroAmount);
        }
        match side {
            Side::Bid => {
                if self.free_quote[trader] < price * qty {
                    return Err(Reject::Insufficient);
                }
                self.free_quote[trader] -= price * qty;
            }
            Side::Ask => {
                if self.free_base[trader] < qty {
                    return Err(Reject::Insufficient);
                }
                self.free_base[trader] -= qty;
            }
        }

        let mut fills = Vec::new();
        let mut remaining = qty;
        while remaining > 0 {
            let best = match side {
                Side::Bid => self.asks.keys().next().copied(),
                Side::Ask => self.bids.keys().next_back().copied(),
            };
            let Some(level_price) = best else { break };
            let crosses = match side {
                Side::Bid => level_price <= price,
                Side::Ask => level_price >= price,
            };
            if !crosses {
                break;
            }
            let book = if side == Side::Bid {
                &mut self.asks
            } else {
                &mut self.bids
            };
            let level = book.get_mut(&level_price).expect("level exists");
            let maker = level.front().cloned().expect("levels are never empty");
            if maker.expiry <= self.now || maker.trader == trader {
                // Expired and self-trade makers are removed and refunded, not filled.
                level.pop_front();
                if level.is_empty() {
                    book.remove(&level_price);
                }
                self.refund(&maker, maker.qty);
                continue;
            }
            let fill_qty = remaining.min(maker.qty);
            if fill_qty == maker.qty {
                level.pop_front();
                if level.is_empty() {
                    book.remove(&level_price);
                }
            } else {
                level.front_mut().expect("front exists").qty -= fill_qty;
            }
            self.settle(trader, side, price, &maker, fill_qty);
            fills.push(Fill {
                maker_id: maker.id,
                price: maker.price,
                qty: fill_qty,
            });
            remaining -= fill_qty;
        }

        if remaining > 0 {
            let order = Order {
                id: self.next_id,
                seq: self.next_seq,
                trader,
                side,
                price,
                qty: remaining,
                expiry: if ttl == 0 {
                    NO_EXPIRY
                } else {
                    self.now.saturating_add(ttl)
                },
            };
            self.next_id += 1;
            self.next_seq += 1;
            let book = if side == Side::Bid {
                &mut self.bids
            } else {
                &mut self.asks
            };
            book.entry(price).or_default().push_back(order);
        }
        Ok(fills)
    }

    /// Settles one fill at the maker price. The seller pays the fee out of quote
    /// proceeds; an odd fee unit is parked in the dust bucket.
    fn settle(
        &mut self,
        taker: usize,
        taker_side: Side,
        taker_limit: u128,
        maker: &Order,
        qty: u128,
    ) {
        let (buyer, seller) = match taker_side {
            Side::Bid => (taker, maker.trader),
            Side::Ask => (maker.trader, taker),
        };
        if taker_side == Side::Bid {
            self.free_quote[buyer] += (taker_limit - maker.price) * qty;
        }
        self.free_base[buyer] += qty;
        let quote = maker.price * qty;
        let fee = quote * FEE_BPS / 10_000;
        let dust = fee & 1;
        self.free_quote[seller] += quote - fee;
        self.fee_bucket += fee - dust;
        self.dust_bucket += dust;
    }

    fn refund(&mut self, order: &Order, qty: u128) {
        match order.side {
            Side::Bid => self.free_quote[order.trader] += order.price * qty,
            Side::Ask => self.free_base[order.trader] += qty,
        }
    }

    fn remove_order(&mut self, id: u64) -> Option<Order> {
        for book in [&mut self.bids, &mut self.asks] {
            let hit = book
                .iter()
                .find_map(|(p, l)| l.iter().position(|o| o.id == id).map(|i| (*p, i)));
            if let Some((price, idx)) = hit {
                let level = book.get_mut(&price).expect("level exists");
                let order = level.remove(idx).expect("index exists");
                if level.is_empty() {
                    book.remove(&price);
                }
                return Some(order);
            }
        }
        None
    }

    fn orders(&self) -> Vec<Order> {
        self.bids
            .values()
            .chain(self.asks.values())
            .flat_map(|l| l.iter().cloned())
            .collect()
    }

    /// Base and quote are checked independently: external inflow minus outflow
    /// must equal free balances plus escrow plus protocol buckets.
    fn check_invariants(&self) -> Result<(), String> {
        let locked_base: u128 = self.asks.values().flatten().map(|o| o.qty).sum();
        let locked_quote: u128 = self.bids.values().flatten().map(|o| o.price * o.qty).sum();
        let base_held = self.free_base.iter().sum::<u128>() + locked_base;
        let quote_held = self.free_quote.iter().sum::<u128>()
            + locked_quote
            + self.fee_bucket
            + self.dust_bucket;
        if self.base_in - self.base_out != base_held {
            return Err(format!(
                "base drift: net external {} vs held {}",
                self.base_in - self.base_out,
                base_held
            ));
        }
        if self.quote_in - self.quote_out != quote_held {
            return Err(format!(
                "quote drift: net external {} vs held {}",
                self.quote_in - self.quote_out,
                quote_held
            ));
        }
        for level in self.bids.values().chain(self.asks.values()) {
            if level.is_empty() || level.iter().any(|o| o.qty == 0) {
                return Err("empty level or zero-quantity order".into());
            }
            if level
                .iter()
                .zip(level.iter().skip(1))
                .any(|(a, b)| a.seq >= b.seq)
            {
                return Err("level not in strict arrival order".into());
            }
        }
        Ok(())
    }
}

/// Runs a sequence, checking invariants after every success and that every
/// rejected action left the whole state untouched.
fn run_sequence(actions: &[Action]) -> Result<(), String> {
    let mut engine = Engine::new();
    for (i, action) in actions.iter().enumerate() {
        let before = engine.clone();
        match engine.apply(action) {
            Ok(_) => engine
                .check_invariants()
                .map_err(|e| format!("step {i} {action:?}: {e}"))?,
            Err(reject) => {
                if engine != before {
                    return Err(format!(
                        "step {i} {action:?}: rejected ({reject:?}) but state changed"
                    ));
                }
            }
        }
    }
    Ok(())
}

/// Greedy shrink: drop actions while the sequence still fails. Rejected actions
/// are no-ops, so any subsequence stays executable.
fn shrink(mut actions: Vec<Action>) -> Vec<Action> {
    let mut i = 0;
    while i < actions.len() {
        let mut candidate = actions.clone();
        candidate.remove(i);
        if run_sequence(&candidate).is_err() {
            actions = candidate;
        } else {
            i += 1;
        }
    }
    actions
}

/// Independent reference: a flat, sorted scan over the eligible makers. It never
/// looks at price levels, queues or removal logic used by `Engine`.
fn reference_fills(
    book: &[Order],
    taker: usize,
    side: Side,
    limit: u128,
    qty: u128,
    now: u64,
) -> Vec<Fill> {
    let mut eligible: Vec<Order> = book
        .iter()
        .filter(|o| o.side != side && o.trader != taker && o.expiry > now)
        .filter(|o| match side {
            Side::Bid => o.price <= limit,
            Side::Ask => o.price >= limit,
        })
        .cloned()
        .collect();
    match side {
        Side::Bid => eligible.sort_by_key(|o| (o.price, o.seq)),
        Side::Ask => eligible.sort_by_key(|o| (std::cmp::Reverse(o.price), o.seq)),
    }
    let mut remaining = qty;
    let mut fills = Vec::new();
    for maker in eligible {
        if remaining == 0 {
            break;
        }
        let take = remaining.min(maker.qty);
        fills.push(Fill {
            maker_id: maker.id,
            price: maker.price,
            qty: take,
        });
        remaining -= take;
    }
    fills
}

fn funded_engine() -> Engine {
    let mut engine = Engine::new();
    for trader in 0..TRADERS {
        for base in [true, false] {
            engine
                .apply(&Action::Deposit {
                    trader,
                    base,
                    amount: 1_000_000,
                })
                .expect("deposit");
        }
    }
    engine
}

#[test]
fn conservation_holds_after_every_action() {
    for seed in 0..3_000 {
        let actions = generate_actions(seed, 80);
        if let Err(err) = run_sequence(&actions) {
            let minimal = shrink(actions);
            panic!("seed {seed}: {err}\nshrunk sequence: {minimal:#?}");
        }
    }
}

#[test]
fn rejected_actions_change_nothing() {
    let mut engine = Engine::new();
    let before = engine.clone();
    for action in [
        Action::Withdraw {
            trader: 0,
            base: true,
            amount: 1,
        },
        Action::Place {
            trader: 0,
            side: Side::Bid,
            price: 100,
            qty: 1,
            ttl: 0,
        },
        Action::Cancel { id: 7 },
        Action::Reduce { id: 7, new_qty: 1 },
        Action::WithdrawFees,
        Action::Deposit {
            trader: 0,
            base: true,
            amount: 0,
        },
    ] {
        assert!(
            engine.apply(&action).is_err(),
            "{action:?} should be rejected"
        );
        assert_eq!(engine, before, "{action:?} changed state");
    }
}

#[test]
fn equal_price_fills_follow_fifo_with_cancels_and_reductions() {
    for seed in 0..1_000 {
        let mut rng = Rng::new(seed);
        let mut engine = funded_engine();
        // Reference queue of (order id, live qty), oldest first.
        let mut queue: Vec<(u64, u128)> = Vec::new();
        for _ in 0..rng.range(2, 12) {
            let trader = rng.below(3) as usize;
            let qty = rng.range(2, 15);
            engine
                .apply(&Action::Place {
                    trader,
                    side: Side::Ask,
                    price: 100,
                    qty,
                    ttl: 0,
                })
                .expect("rest ask");
            queue.push((engine.next_id - 1, qty));
        }
        let mut cancelled = Vec::new();
        for _ in 0..rng.below(6) {
            if queue.is_empty() {
                break;
            }
            let idx = rng.below(queue.len() as u64) as usize;
            let (id, qty) = queue[idx];
            if rng.below(2) == 0 || qty <= 1 {
                engine.apply(&Action::Cancel { id }).expect("cancel");
                queue.remove(idx);
                cancelled.push(id);
            } else {
                let new_qty = rng.range(1, qty - 1);
                engine
                    .apply(&Action::Reduce { id, new_qty })
                    .expect("reduce");
                queue[idx].1 = new_qty;
            }
        }
        let total: u128 = queue.iter().map(|(_, q)| q).sum();
        let want = rng.range(1, total + 5);
        let fills = engine
            .apply(&Action::Place {
                trader: 3,
                side: Side::Bid,
                price: 100,
                qty: want,
                ttl: 0,
            })
            .expect("taker");

        let mut expected = Vec::new();
        let mut left = want;
        for (id, qty) in &queue {
            if left == 0 {
                break;
            }
            let take = left.min(*qty);
            expected.push((*id, take));
            left -= take;
        }
        let actual: Vec<(u64, u128)> = fills.iter().map(|f| (f.maker_id, f.qty)).collect();
        assert_eq!(
            actual, expected,
            "seed {seed}: fill order differs from reference FIFO"
        );
        assert!(
            fills.iter().all(|f| !cancelled.contains(&f.maker_id)),
            "seed {seed}: cancelled order was filled"
        );
    }
}

#[test]
fn fills_always_take_the_best_eligible_price() {
    for seed in 0..2_000 {
        let mut rng = Rng::new(seed);
        let mut engine = funded_engine();
        // Non-crossing book: bids 90..=100, asks 101..=110.
        for _ in 0..rng.range(2, 16) {
            let trader = rng.below(3) as usize;
            let (side, price) = if rng.below(2) == 0 {
                (Side::Bid, rng.range(90, 100))
            } else {
                (Side::Ask, rng.range(101, 110))
            };
            let ttl = rng.below(3) * 4;
            engine
                .apply(&Action::Place {
                    trader,
                    side,
                    price,
                    qty: rng.range(1, 10),
                    ttl,
                })
                .expect("rest order");
        }
        engine
            .apply(&Action::Advance {
                secs: rng.below(8) + 1,
            })
            .expect("advance");

        // Taker 3 has no resting orders; taker 0 exercises self-trade removal.
        let taker = if rng.below(3) == 0 { 0 } else { 3 };
        let side = rng.side();
        let limit = rng.range(88, 112);
        let qty = rng.range(1, 60);
        let book = engine.orders();
        let expected = reference_fills(&book, taker, side, limit, qty, engine.now);
        let fills = engine
            .apply(&Action::Place {
                trader: taker,
                side,
                price: limit,
                qty,
                ttl: 0,
            })
            .expect("taker");
        assert_eq!(
            fills, expected,
            "seed {seed}: fills differ from sorted reference\nstarting book: {book:#?}\nrequest: taker {taker} {side:?} limit {limit} qty {qty} at t={}",
            engine.now
        );
        engine.check_invariants().expect("invariants after taker");
    }
}
