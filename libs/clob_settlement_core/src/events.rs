//! Canonical CLOB event schema (#885).
//!
//! Indexers and clients rebuild order, fill and balance state from these
//! records alone, without reading storage. Every record carries:
//!
//! - `market`: the [`MarketId`] it belongs to.
//! - `index`: an [`EventIndex`] that is strictly increasing and gap-free within
//!   a market, starting at 1. Ordering is by `index`.
//!
//! Orders are identified by the [`SequenceNumber`] assigned at acceptance. The
//! same id appears on every later event for that order. A fill names both
//! orders, both traders, the taker side and the execution price.
//!
//! Lifecycle invariant: for every order, `accepted = filled + canceled + expired`
//! once the order is closed. A cancel or expiry removes all remaining lots.
//!
//! [`ClobEvent::topic`] strings are stable wire identifiers. Renaming one is a
//! breaking change for indexers.

use soroban_sdk::Address;

use crate::market_id::MarketId;
use crate::units::{BaseLots, QuoteAtoms, SequenceNumber, Tick};

/// Position of a record in its market's event stream. Starts at 1.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd)]
pub struct EventIndex(u64);

impl EventIndex {
    pub const FIRST: Self = Self(1);

    pub const fn new(value: u64) -> Self {
        Self(value)
    }

    pub const fn get(self) -> u64 {
        self.0
    }

    /// Returns `None` at `u64::MAX`.
    pub fn next(self) -> Option<Self> {
        self.0.checked_add(1).map(Self)
    }
}

/// True if `next` is the record immediately after `prev` in one market's stream.
pub fn follows(prev: EventIndex, next: EventIndex) -> bool {
    prev.next() == Some(next)
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Side {
    Bid,
    Ask,
}

/// Fee category, matching the buckets in the fee accounting.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FeeKind {
    Execution,
    Position,
    Referral,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MarketStatus {
    Open,
    Paused,
    Closed,
}

/// Market configuration fields that emit a [`ClobEvent::ConfigChanged`].
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ConfigField {
    TickSize,
    LotSize,
    FeeRate,
}

/// One state transition. Amounts for deposits, withdrawals and fees are token
/// atoms of the named `asset`, so they stay `i128` here.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ClobEvent {
    Deposit {
        trader: Address,
        asset: Address,
        amount: i128,
    },
    Withdrawal {
        trader: Address,
        asset: Address,
        amount: i128,
    },
    /// `client_id` is the caller's own id, echoed back so clients can match
    /// their submissions to the on-chain order.
    OrderAccepted {
        order: SequenceNumber,
        trader: Address,
        side: Side,
        price: Tick,
        lots: BaseLots,
        client_id: Option<u64>,
    },
    OrderRested {
        order: SequenceNumber,
        remaining: BaseLots,
    },
    OrderReduced {
        order: SequenceNumber,
        reduced_by: BaseLots,
        remaining: BaseLots,
    },
    /// Removes all remaining lots. `canceled` is the remaining amount at cancel time.
    OrderCanceled {
        order: SequenceNumber,
        canceled: BaseLots,
    },
    /// Removes all remaining lots. `expired` is the remaining amount at expiry.
    OrderExpired {
        order: SequenceNumber,
        expired: BaseLots,
    },
    Fill {
        maker_order: SequenceNumber,
        taker_order: SequenceNumber,
        maker: Address,
        taker: Address,
        taker_side: Side,
        price: Tick,
        lots: BaseLots,
        quote: QuoteAtoms,
        maker_remaining: BaseLots,
        taker_remaining: BaseLots,
    },
    FeeAccrued {
        asset: Address,
        kind: FeeKind,
        amount: i128,
    },
    FeeWithdrawn {
        asset: Address,
        recipient: Address,
        amount: i128,
    },
    MarketStatusChanged {
        previous: MarketStatus,
        next: MarketStatus,
    },
    ConfigChanged {
        field: ConfigField,
        previous: i128,
        next: i128,
    },
}

impl ClobEvent {
    /// Stable topic name for this event kind.
    pub fn topic(&self) -> &'static str {
        match self {
            Self::Deposit { .. } => "deposit",
            Self::Withdrawal { .. } => "withdrawal",
            Self::OrderAccepted { .. } => "order_accepted",
            Self::OrderRested { .. } => "order_rested",
            Self::OrderReduced { .. } => "order_reduced",
            Self::OrderCanceled { .. } => "order_canceled",
            Self::OrderExpired { .. } => "order_expired",
            Self::Fill { .. } => "fill",
            Self::FeeAccrued { .. } => "fee_accrued",
            Self::FeeWithdrawn { .. } => "fee_withdrawn",
            Self::MarketStatusChanged { .. } => "market_status_changed",
            Self::ConfigChanged { .. } => "config_changed",
        }
    }
}

/// An event as published: the event plus the market and position it belongs to.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ClobEventRecord {
    pub market: MarketId,
    pub index: EventIndex,
    pub event: ClobEvent,
}

#[cfg(test)]
mod tests {
    use super::*;
    use soroban_sdk::{testutils::Address as _, Env};

    fn record(market: &MarketId, index: u64, event: ClobEvent) -> ClobEventRecord {
        ClobEventRecord {
            market: market.clone(),
            index: EventIndex::new(index),
            event,
        }
    }

    #[test]
    fn topics_are_stable_and_unique() {
        let env = Env::default();
        let who = Address::generate(&env);
        let asset = Address::generate(&env);
        let order = SequenceNumber::new(1);
        let lots = BaseLots::new(1);

        let events = [
            ClobEvent::Deposit { trader: who.clone(), asset: asset.clone(), amount: 1 },
            ClobEvent::Withdrawal { trader: who.clone(), asset: asset.clone(), amount: 1 },
            ClobEvent::OrderAccepted {
                order,
                trader: who.clone(),
                side: Side::Bid,
                price: Tick::new(1),
                lots,
                client_id: None,
            },
            ClobEvent::OrderRested { order, remaining: lots },
            ClobEvent::OrderReduced { order, reduced_by: lots, remaining: lots },
            ClobEvent::OrderCanceled { order, canceled: lots },
            ClobEvent::OrderExpired { order, expired: lots },
            ClobEvent::Fill {
                maker_order: order,
                taker_order: order,
                maker: who.clone(),
                taker: who.clone(),
                taker_side: Side::Ask,
                price: Tick::new(1),
                lots,
                quote: QuoteAtoms::ZERO,
                maker_remaining: lots,
                taker_remaining: lots,
            },
            ClobEvent::FeeAccrued { asset: asset.clone(), kind: FeeKind::Execution, amount: 1 },
            ClobEvent::FeeWithdrawn { asset, recipient: who, amount: 1 },
            ClobEvent::MarketStatusChanged { previous: MarketStatus::Open, next: MarketStatus::Paused },
            ClobEvent::ConfigChanged { field: ConfigField::FeeRate, previous: 0, next: 5 },
        ];

        let topics = events.map(|e| e.topic());

        assert_eq!(
            topics,
            [
                "deposit",
                "withdrawal",
                "order_accepted",
                "order_rested",
                "order_reduced",
                "order_canceled",
                "order_expired",
                "fill",
                "fee_accrued",
                "fee_withdrawn",
                "market_status_changed",
                "config_changed",
            ]
        );
    }

    #[test]
    fn fill_carries_both_order_ids_and_values() {
        let env = Env::default();
        let maker = Address::generate(&env);
        let taker = Address::generate(&env);

        let fill = ClobEvent::Fill {
            maker_order: SequenceNumber::new(7),
            taker_order: SequenceNumber::new(9),
            maker: maker.clone(),
            taker: taker.clone(),
            taker_side: Side::Bid,
            price: Tick::new(250),
            lots: BaseLots::new(4),
            quote: QuoteAtoms::new(1_000).unwrap(),
            maker_remaining: BaseLots::new(6),
            taker_remaining: BaseLots::ZERO,
        };

        let ClobEvent::Fill { maker_order, taker_order, taker_side, price, lots, quote, .. } = fill else {
            panic!("expected a fill");
        };
        assert_eq!(maker_order.get(), 7);
        assert_eq!(taker_order.get(), 9);
        assert_eq!(taker_side, Side::Bid);
        assert_eq!(price.get(), 250);
        assert_eq!(lots.get(), 4);
        assert_eq!(quote.get(), 1_000);
    }

    #[test]
    fn event_indices_are_gap_free_and_detect_gaps() {
        assert_eq!(EventIndex::FIRST.get(), 1);
        assert!(follows(EventIndex::new(1), EventIndex::new(2)));
        assert!(!follows(EventIndex::new(2), EventIndex::new(4)), "gap must be detected");
        assert!(!follows(EventIndex::new(2), EventIndex::new(2)), "duplicate must be detected");
        assert_eq!(EventIndex::new(u64::MAX).next(), None);
    }

    /// Accepted, filled, canceled and remaining lots for one order, rebuilt from events only.
    struct Lifecycle {
        accepted: u64,
        filled: u64,
        closed: u64,
        remaining: u64,
    }

    fn lifecycle(records: &[ClobEventRecord], order: SequenceNumber) -> Lifecycle {
        let mut life = Lifecycle { accepted: 0, filled: 0, closed: 0, remaining: 0 };
        for record in records {
            match &record.event {
                ClobEvent::OrderAccepted { order: o, lots, .. } if *o == order => {
                    life.accepted = lots.get();
                    life.remaining = lots.get();
                }
                ClobEvent::OrderRested { order: o, remaining } if *o == order => {
                    life.remaining = remaining.get();
                }
                ClobEvent::Fill { maker_order, maker_remaining, .. } if *maker_order == order => {
                    life.remaining = maker_remaining.get();
                }
                ClobEvent::Fill { taker_order, taker_remaining, .. } if *taker_order == order => {
                    life.remaining = taker_remaining.get();
                }
                ClobEvent::OrderCanceled { order: o, canceled } if *o == order => {
                    life.closed = canceled.get();
                    life.remaining = 0;
                }
                ClobEvent::OrderExpired { order: o, expired } if *o == order => {
                    life.closed = expired.get();
                    life.remaining = 0;
                }
                _ => {}
            }
            if let ClobEvent::Fill { maker_order, taker_order, lots, .. } = &record.event {
                if *maker_order == order || *taker_order == order {
                    life.filled += lots.get();
                }
            }
        }
        life
    }

    #[test]
    fn replay_reconstructs_order_lifecycle_from_events_alone() {
        let env = Env::default();
        let base = Address::generate(&env);
        let quote = Address::generate(&env);
        let market = MarketId::derive(&env, &base, &quote, 1).unwrap();
        let trader_a = Address::generate(&env);
        let trader_b = Address::generate(&env);

        let maker = SequenceNumber::new(1);
        let taker = SequenceNumber::new(2);

        let records = [
            record(&market, 1, ClobEvent::OrderAccepted {
                order: maker,
                trader: trader_a.clone(),
                side: Side::Bid,
                price: Tick::new(100),
                lots: BaseLots::new(10),
                client_id: Some(42),
            }),
            record(&market, 2, ClobEvent::OrderRested { order: maker, remaining: BaseLots::new(10) }),
            record(&market, 3, ClobEvent::OrderAccepted {
                order: taker,
                trader: trader_b.clone(),
                side: Side::Ask,
                price: Tick::new(100),
                lots: BaseLots::new(4),
                client_id: None,
            }),
            record(&market, 4, ClobEvent::Fill {
                maker_order: maker,
                taker_order: taker,
                maker: trader_a,
                taker: trader_b,
                taker_side: Side::Ask,
                price: Tick::new(100),
                lots: BaseLots::new(4),
                quote: QuoteAtoms::new(400).unwrap(),
                maker_remaining: BaseLots::new(6),
                taker_remaining: BaseLots::ZERO,
            }),
            record(&market, 5, ClobEvent::OrderCanceled { order: maker, canceled: BaseLots::new(6) }),
        ];

        let maker_life = lifecycle(&records, maker);
        let taker_life = lifecycle(&records, taker);

        assert_eq!(maker_life.accepted, 10);
        assert_eq!(maker_life.filled, 4);
        assert_eq!(maker_life.closed, 6);
        assert_eq!(maker_life.remaining, 0);
        assert_eq!(maker_life.accepted, maker_life.filled + maker_life.closed);

        assert_eq!(taker_life.accepted, 4);
        assert_eq!(taker_life.filled, 4);
        assert_eq!(taker_life.remaining, 0);
    }

    #[test]
    fn records_are_strictly_ordered_within_a_market() {
        let env = Env::default();
        let base = Address::generate(&env);
        let quote = Address::generate(&env);
        let market = MarketId::derive(&env, &base, &quote, 1).unwrap();
        let who = Address::generate(&env);

        let records = [
            record(&market, 1, ClobEvent::Deposit { trader: who.clone(), asset: base.clone(), amount: 100 }),
            record(&market, 2, ClobEvent::Withdrawal { trader: who, asset: base, amount: 40 }),
        ];

        assert_eq!(records[0].index, EventIndex::FIRST);
        assert!(follows(records[0].index, records[1].index));
        assert_eq!(records[0].market, records[1].market);
    }
}
