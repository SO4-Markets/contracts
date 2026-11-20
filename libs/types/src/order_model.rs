use soroban_sdk::{contracttype, Address};

/// Order side for CLOB orderbook: Bid (Buy base asset) or Ask (Sell base asset).
#[contracttype]
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
#[repr(u32)]
pub enum OrderSide {
    Bid = 1,
    Ask = 2,
}

impl OrderSide {
    #[inline]
    pub fn is_bid(&self) -> bool {
        matches!(self, OrderSide::Bid)
    }

    #[inline]
    pub fn is_ask(&self) -> bool {
        matches!(self, OrderSide::Ask)
    }

    #[inline]
    pub fn opposite(&self) -> Self {
        match self {
            OrderSide::Bid => OrderSide::Ask,
            OrderSide::Ask => OrderSide::Bid,
        }
    }
}

/// Errors returned by resting order lifecycle operations.
#[contracttype]
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
#[repr(u32)]
pub enum OrderError {
    InvalidOrderId = 1,
    InvalidPriceTicks = 2,
    InvalidBaseLots = 3,
    OrderFullyFilled = 4,
    OrderExpired = 5,
    ReduceAmountExceedsRemaining = 6,
    FillAmountExceedsRemaining = 7,
}

/// Minimal canonical resting Order struct for deterministic matching and FIFO priority.
///
/// Only consensus-critical fields required for matching, cancellation, reduction,
/// and FIFO queue traversal are stored on-chain. Derivable values (market pair,
/// quote notional, filled amounts, fees, and decimals) are computed on the fly.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RestingOrder {
    /// Unique identifier for this order within the market.
    pub order_id: u64,
    /// Owner address who placed the order and deposited collateral.
    pub owner: Address,
    /// Order side: Bid (Buy) or Ask (Sell).
    pub side: OrderSide,
    /// Limit price expressed in integer ticks (multiple of market tick_size).
    pub price_ticks: u64,
    /// Initial size in integer base lots at order placement.
    pub initial_base_lots: u64,
    /// Remaining unfilled size in integer base lots.
    pub remaining_base_lots: u64,
    /// Monotonic sequence number for deterministic FIFO tie-breaking at same price.
    pub sequence_number: u64,
    /// Ledger sequence deadline after which the order is expired (0 if GTC / no expiry).
    pub expiry_ledger: u64,
    /// Client-supplied order identifier (0 if unset) for off-chain trade indexing.
    pub client_order_id: u64,
    /// Pointer to the previous order ID in the price-level FIFO queue (None if head).
    pub queue_prev: Option<u64>,
    /// Pointer to the next order ID in the price-level FIFO queue (None if tail).
    pub queue_next: Option<u64>,
}

impl RestingOrder {
    /// Create a new resting order. Validates non-zero order ID, price, and initial size.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        order_id: u64,
        owner: Address,
        side: OrderSide,
        price_ticks: u64,
        base_lots: u64,
        sequence_number: u64,
        expiry_ledger: u64,
        client_order_id: u64,
    ) -> Result<Self, OrderError> {
        if order_id == 0 {
            return Err(OrderError::InvalidOrderId);
        }
        if price_ticks == 0 {
            return Err(OrderError::InvalidPriceTicks);
        }
        if base_lots == 0 {
            return Err(OrderError::InvalidBaseLots);
        }

        Ok(Self {
            order_id,
            owner,
            side,
            price_ticks,
            initial_base_lots: base_lots,
            remaining_base_lots: base_lots,
            sequence_number,
            expiry_ledger,
            client_order_id,
            queue_prev: None,
            queue_next: None,
        })
    }

    /// Quantity in base lots already executed/filled. Derivable from `initial - remaining`.
    #[inline]
    pub fn filled_base_lots(&self) -> u64 {
        self.initial_base_lots.saturating_sub(self.remaining_base_lots)
    }

    /// Whether the order has been completely filled.
    #[inline]
    pub fn is_filled(&self) -> bool {
        self.remaining_base_lots == 0
    }

    /// Whether the order still has unfilled lots remaining.
    #[inline]
    pub fn is_active(&self) -> bool {
        self.remaining_base_lots > 0
    }

    /// Whether the order has expired given the current ledger sequence.
    /// Orders with `expiry_ledger == 0` do not expire (Good 'Til Cancelled).
    #[inline]
    pub fn is_expired(&self, current_ledger: u64) -> bool {
        self.expiry_ledger > 0 && current_ledger > self.expiry_ledger
    }

    /// Reduce the remaining order size without losing FIFO queue priority.
    /// Returns the new remaining base lots or an error if reduce_lots exceeds remaining.
    pub fn reduce(&mut self, reduce_lots: u64) -> Result<u64, OrderError> {
        if self.is_filled() {
            return Err(OrderError::OrderFullyFilled);
        }
        if reduce_lots > self.remaining_base_lots {
            return Err(OrderError::ReduceAmountExceedsRemaining);
        }
        self.remaining_base_lots -= reduce_lots;
        Ok(self.remaining_base_lots)
    }

    /// Apply an execution fill to the order, decrementing remaining base lots.
    /// Returns the new remaining base lots or an error if fill_lots exceeds remaining.
    pub fn fill(&mut self, fill_lots: u64) -> Result<u64, OrderError> {
        if self.is_filled() {
            return Err(OrderError::OrderFullyFilled);
        }
        if fill_lots > self.remaining_base_lots {
            return Err(OrderError::FillAmountExceedsRemaining);
        }
        self.remaining_base_lots -= fill_lots;
        Ok(self.remaining_base_lots)
    }

    /// Compute the derivable quote notional for the remaining size given market config.
    /// Formula: remaining_base_lots * base_lot_size * price_ticks * tick_size
    pub fn derivable_remaining_quote_notional(
        &self,
        tick_size: u128,
        base_lot_size: u128,
    ) -> Option<u128> {
        let base_amount = (self.remaining_base_lots as u128).checked_mul(base_lot_size)?;
        let price = (self.price_ticks as u128).checked_mul(tick_size)?;
        base_amount.checked_mul(price)
    }

    /// Linkage helper: update FIFO queue pointers.
    pub fn set_queue_links(&mut self, prev: Option<u64>, next: Option<u64>) {
        self.queue_prev = prev;
        self.queue_next = next;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use soroban_sdk::{testutils::Address as _, xdr::ToXdr, Address, Env};

    #[test]
    fn test_order_creation_and_fields() {
        let env = Env::default();
        let owner = Address::generate(&env);

        let order = RestingOrder::new(
            1,
            owner.clone(),
            OrderSide::Bid,
            100,
            50,
            1,
            1000,
            12345,
        )
        .unwrap();

        assert_eq!(order.order_id, 1);
        assert_eq!(order.owner, owner);
        assert_eq!(order.side, OrderSide::Bid);
        assert_eq!(order.price_ticks, 100);
        assert_eq!(order.initial_base_lots, 50);
        assert_eq!(order.remaining_base_lots, 50);
        assert_eq!(order.sequence_number, 1);
        assert_eq!(order.expiry_ledger, 1000);
        assert_eq!(order.client_order_id, 12345);
        assert_eq!(order.queue_prev, None);
        assert_eq!(order.queue_next, None);

        assert_eq!(order.filled_base_lots(), 0);
        assert!(order.is_active());
        assert!(!order.is_filled());
        assert!(!order.is_expired(999));
        assert!(!order.is_expired(1000));
        assert!(order.is_expired(1001));
    }

    #[test]
    fn test_order_validation_rejects_invalid_inputs() {
        let env = Env::default();
        let owner = Address::generate(&env);

        assert_eq!(
            RestingOrder::new(0, owner.clone(), OrderSide::Bid, 100, 50, 1, 0, 0),
            Err(OrderError::InvalidOrderId)
        );
        assert_eq!(
            RestingOrder::new(1, owner.clone(), OrderSide::Bid, 0, 50, 1, 0, 0),
            Err(OrderError::InvalidPriceTicks)
        );
        assert_eq!(
            RestingOrder::new(1, owner.clone(), OrderSide::Bid, 100, 0, 1, 0, 0),
            Err(OrderError::InvalidBaseLots)
        );
    }

    #[test]
    fn test_order_fill_and_reduce_lifecycle() {
        let env = Env::default();
        let owner = Address::generate(&env);

        let mut order = RestingOrder::new(
            10,
            owner.clone(),
            OrderSide::Ask,
            250,
            100,
            42,
            0,
            99,
        )
        .unwrap();

        // Partial fill 30 lots
        let remaining = order.fill(30).unwrap();
        assert_eq!(remaining, 70);
        assert_eq!(order.filled_base_lots(), 30);
        assert!(order.is_active());
        assert!(!order.is_filled());

        // Reduce 20 lots
        let remaining = order.reduce(20).unwrap();
        assert_eq!(remaining, 50);
        assert_eq!(order.remaining_base_lots, 50);

        // Reduce exceeds remaining should fail
        assert_eq!(
            order.reduce(51),
            Err(OrderError::ReduceAmountExceedsRemaining)
        );

        // Fill exceeds remaining should fail
        assert_eq!(
            order.fill(51),
            Err(OrderError::FillAmountExceedsRemaining)
        );

        // Complete fill
        let remaining = order.fill(50).unwrap();
        assert_eq!(remaining, 0);
        assert!(order.is_filled());
        assert!(!order.is_active());
        assert_eq!(order.filled_base_lots(), 100);

        // Further actions fail
        assert_eq!(order.fill(1), Err(OrderError::OrderFullyFilled));
        assert_eq!(order.reduce(1), Err(OrderError::OrderFullyFilled));
    }

    #[test]
    fn test_order_queue_linkage() {
        let env = Env::default();
        let owner = Address::generate(&env);

        let mut order = RestingOrder::new(
            5,
            owner.clone(),
            OrderSide::Bid,
            100,
            10,
            5,
            0,
            0,
        )
        .unwrap();

        assert_eq!(order.queue_prev, None);
        assert_eq!(order.queue_next, None);

        order.set_queue_links(Some(4), Some(6));
        assert_eq!(order.queue_prev, Some(4));
        assert_eq!(order.queue_next, Some(6));

        order.set_queue_links(None, None);
        assert_eq!(order.queue_prev, None);
        assert_eq!(order.queue_next, None);
    }

    #[test]
    fn test_derivable_quote_notional() {
        let env = Env::default();
        let owner = Address::generate(&env);

        let order = RestingOrder::new(
            1,
            owner.clone(),
            OrderSide::Bid,
            200, // 200 ticks
            10,  // 10 base lots
            1,
            0,
            0,
        )
        .unwrap();

        // tick_size = 5, base_lot_size = 100
        // notional = 10 * 100 * 200 * 5 = 1,000,000
        let notional = order.derivable_remaining_quote_notional(5, 100).unwrap();
        assert_eq!(notional, 1_000_000);
    }

    #[test]
    fn test_serialized_xdr_size_measurement() {
        let env = Env::default();
        let owner = Address::generate(&env);

        let order = RestingOrder::new(
            1,
            owner.clone(),
            OrderSide::Bid,
            100,
            50,
            1,
            1000,
            12345,
        )
        .unwrap();

        let xdr_bytes = order.to_xdr(&env);
        let xdr_len = xdr_bytes.len();

        // Exact measured ScVal Map XDR size (including 11 field symbol keys + values) is 384 bytes
        assert_eq!(xdr_len, 384);
        assert!(
            xdr_len <= 512,
            "Resting order XDR size must be compact (<=512 bytes), got {}",
            xdr_len
        );
    }
}
