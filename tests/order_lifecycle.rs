//! Tests for Issue #917 [CLOB-041]:
//! Define the resting Order struct fields and persistence split.
//!
//! Validates:
//! 1. Order struct contains all required consensus-critical fields and validates inputs.
//! 2. Separation of persistent fields from derivable values (quote notional, filled quantity, status).
//! 3. Deterministic FIFO queue linkage (doubly-linked pointers).
//! 4. Lifecycle state transitions: partial fill, size reduction, cancellation, and expiration.
//! 5. Serialized XDR size measurement and resource budget compliance.

#![cfg(test)]

use gmx_types::order_model::{OrderError, OrderSide, RestingOrder};
use soroban_sdk::{
    testutils::Address as _,
    xdr::ToXdr,
    Address, Env,
};

#[test]
fn test_resting_order_construction_and_field_invariants() {
    let env = Env::default();
    let owner = Address::generate(&env);

    // Valid Bid order
    let bid = RestingOrder::new(
        101,
        owner.clone(),
        OrderSide::Bid,
        500,  // price_ticks
        100,  // base_lots
        1,    // sequence_number
        5000, // expiry_ledger
        42,   // client_order_id
    )
    .expect("Valid order should construct successfully");

    assert_eq!(bid.order_id, 101);
    assert_eq!(bid.owner, owner);
    assert_eq!(bid.side, OrderSide::Bid);
    assert!(bid.side.is_bid());
    assert!(!bid.side.is_ask());
    assert_eq!(bid.price_ticks, 500);
    assert_eq!(bid.initial_base_lots, 100);
    assert_eq!(bid.remaining_base_lots, 100);
    assert_eq!(bid.sequence_number, 1);
    assert_eq!(bid.expiry_ledger, 5000);
    assert_eq!(bid.client_order_id, 42);
    assert_eq!(bid.queue_prev, None);
    assert_eq!(bid.queue_next, None);

    // Side opposite helper
    assert_eq!(bid.side.opposite(), OrderSide::Ask);
    assert_eq!(OrderSide::Ask.opposite(), OrderSide::Bid);

    // Invalid parameters rejected
    assert_eq!(
        RestingOrder::new(0, owner.clone(), OrderSide::Bid, 500, 100, 1, 0, 0),
        Err(OrderError::InvalidOrderId)
    );
    assert_eq!(
        RestingOrder::new(1, owner.clone(), OrderSide::Bid, 0, 100, 1, 0, 0),
        Err(OrderError::InvalidPriceTicks)
    );
    assert_eq!(
        RestingOrder::new(1, owner.clone(), OrderSide::Bid, 500, 0, 1, 0, 0),
        Err(OrderError::InvalidBaseLots)
    );
}

#[test]
fn test_fifo_queue_linkage_and_traversal() {
    let env = Env::default();
    let owner_a = Address::generate(&env);
    let owner_b = Address::generate(&env);
    let owner_c = Address::generate(&env);

    // Three orders at the same price level 100
    let mut order1 = RestingOrder::new(1, owner_a, OrderSide::Bid, 100, 10, 1, 0, 0).unwrap();
    let mut order2 = RestingOrder::new(2, owner_b, OrderSide::Bid, 100, 20, 2, 0, 0).unwrap();
    let mut order3 = RestingOrder::new(3, owner_c, OrderSide::Bid, 100, 30, 3, 0, 0).unwrap();

    // Link: 1 <-> 2 <-> 3
    order1.set_queue_links(None, Some(2));
    order2.set_queue_links(Some(1), Some(3));
    order3.set_queue_links(Some(2), None);

    assert_eq!(order1.queue_prev, None);
    assert_eq!(order1.queue_next, Some(2));
    assert_eq!(order2.queue_prev, Some(1));
    assert_eq!(order2.queue_next, Some(3));
    assert_eq!(order3.queue_prev, Some(2));
    assert_eq!(order3.queue_next, None);

    // Remove order2: relink 1 <-> 3
    order1.set_queue_links(None, Some(3));
    order3.set_queue_links(Some(1), None);

    assert_eq!(order1.queue_next, Some(3));
    assert_eq!(order3.queue_prev, Some(1));
}

#[test]
fn test_lifecycle_fill_reduce_and_derivables() {
    let env = Env::default();
    let owner = Address::generate(&env);

    let mut order = RestingOrder::new(
        200,
        owner,
        OrderSide::Ask,
        250, // price = 250 ticks
        100, // initial = 100 base lots
        1,
        1000, // expiry ledger
        999,
    )
    .unwrap();

    // Initial derivable checks
    assert_eq!(order.filled_base_lots(), 0);
    assert!(order.is_active());
    assert!(!order.is_filled());

    // Partial fill 40 lots
    let remaining = order.fill(40).unwrap();
    assert_eq!(remaining, 60);
    assert_eq!(order.remaining_base_lots, 60);
    assert_eq!(order.filled_base_lots(), 40);
    assert!(order.is_active());
    assert!(!order.is_filled());

    // Reduce size by 20 lots (e.g. trader reduces quote)
    let remaining = order.reduce(20).unwrap();
    assert_eq!(remaining, 40);
    assert_eq!(order.remaining_base_lots, 40);
    assert_eq!(order.filled_base_lots(), 60);

    // Derivable quote notional: remaining (40) * base_lot_size (10) * price_ticks (250) * tick_size (2)
    // 40 * 10 * 250 * 2 = 200,000
    let notional = order.derivable_remaining_quote_notional(2, 10).unwrap();
    assert_eq!(notional, 200_000);

    // Fill remaining 40 lots
    let remaining = order.fill(40).unwrap();
    assert_eq!(remaining, 0);
    assert!(order.is_filled());
    assert!(!order.is_active());
    assert_eq!(order.filled_base_lots(), 100);

    // Further fills or reductions rejected
    assert_eq!(order.fill(1), Err(OrderError::OrderFullyFilled));
    assert_eq!(order.reduce(1), Err(OrderError::OrderFullyFilled));
}

#[test]
fn test_expiry_conditions_across_ledger_sequences() {
    let env = Env::default();
    let owner = Address::generate(&env);

    // Order with explicit deadline: expiry_ledger = 500
    let expiring_order = RestingOrder::new(
        1,
        owner.clone(),
        OrderSide::Bid,
        100,
        10,
        1,
        500,
        0,
    )
    .unwrap();

    assert!(!expiring_order.is_expired(499));
    assert!(!expiring_order.is_expired(500));
    assert!(expiring_order.is_expired(501));

    // Good 'Til Cancelled order: expiry_ledger = 0
    let gtc_order = RestingOrder::new(
        2,
        owner,
        OrderSide::Bid,
        100,
        10,
        2,
        0, // GTC
        0,
    )
    .unwrap();

    assert!(!gtc_order.is_expired(0));
    assert!(!gtc_order.is_expired(1_000_000));
}

#[test]
fn test_serialized_xdr_size_budget_compliance() {
    let env = Env::default();
    let owner = Address::generate(&env);

    let order = RestingOrder::new(
        1,
        owner.clone(),
        OrderSide::Bid,
        1000,
        500,
        1,
        2000,
        999,
    )
    .unwrap();

    let xdr_bytes = order.to_xdr(&env);
    let xdr_len = xdr_bytes.len();

    // Verify compact footprint matches documented 384 bytes
    assert_eq!(xdr_len, 384);
    assert!(
        xdr_len <= 512,
        "Order entry must be bounded well within Soroban limits (<=512 bytes), got {}",
        xdr_len
    );
}
