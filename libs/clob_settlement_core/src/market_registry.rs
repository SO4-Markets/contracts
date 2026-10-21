/// Enumerable market registry with bounded pagination and O(1) lookup
/// 
/// Implements issue #015: Clients need a canonical way to discover markets
/// without scanning contract history. The registry supports bounded enumeration
/// and direct lookup while avoiding unbounded vectors.
/// 
/// Storage patterns:
/// - Market metadata stored individually by ID (O(1) lookup)
/// - Market ID list chunked into pages (bounded pages, no unbounded vector)
/// - Soft-delete via TTL to mark closed/deleted markets

use soroban_sdk::Address;

/// Maximum markets per page. Prevents unbounded reads in a single query.
pub const MARKETS_PER_PAGE: usize = 100;

/// Market metadata needed by SDKs and clients.
/// This is the data returned by enumeration; stored individually for O(1) lookup.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MarketMetadata {
    /// Market ID (contract address)
    pub id: Address,
    
    /// Base asset token
    pub base: Address,
    
    /// Quote asset token
    pub quote: Address,
    
    /// Human-readable pair name (e.g., "USDC/EUR")
    pub pair_name: Option<Vec<u8>>,
    
    /// Ledger at which the market was created
    pub created_ledger: u32,
    
    /// Ledger at which the market was marked closed/deleted (if any)
    /// If Some, the market is soft-deleted and should not be used for trading
    pub deleted_ledger: Option<u32>,
}

impl MarketMetadata {
    /// Check if this market is currently active
    pub fn is_active(&self, current_ledger: u32) -> bool {
        match self.deleted_ledger {
            None => true,
            Some(del_ledger) => current_ledger < del_ledger,
        }
    }
}

/// A single page of market IDs for pagination
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MarketPage {
    /// Market IDs in this page
    pub markets: Vec<Address>,
    
    /// Page index
    pub page: u32,
    
    /// Total number of markets (including deleted)
    pub total_markets: u32,
    
    /// Whether there are more pages after this one
    pub has_next: bool,
}

/// Registry entry key for storage. Each market is stored individually.
/// Storage key: (MarketId) => MarketMetadata
/// 
/// Lookup is O(1): load_market(env, market_id) directly retrieves metadata.
/// Enumeration is bounded: load_page(env, page_num) loads one page of IDs,
/// then caller loads metadata for each ID individually.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RegistryEntry {
    pub market: MarketMetadata,
}

/// Pagination cursor for market enumeration.
/// 
/// Allows clients to iterate through markets without loading all at once.
/// next_page = current_page + 1
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PaginationCursor {
    /// Current page number (0-indexed)
    pub page: u32,
    
    /// Filter by active/all markets
    pub include_deleted: bool,
}

impl Default for PaginationCursor {
    fn default() -> Self {
        Self {
            page: 0,
            include_deleted: false,
        }
    }
}

/// Error codes for market registry operations
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u32)]
pub enum RegistryError {
    /// Market not found in registry
    MarketNotFound = 1,
    /// Page number out of range
    PageOutOfRange = 2,
    /// Market already exists in registry
    MarketAlreadyExists = 3,
    /// Invalid market metadata
    InvalidMetadata = 4,
    /// Storage operation failed
    StorageError = 5,
}

/// Result type for registry operations
pub type RegistryResult<T> = Result<T, RegistryError>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_market_metadata_is_active() {
        use soroban_sdk::testutils::Address as _;
        use soroban_sdk::Env;

        let env = Env::default();
        let market = MarketMetadata {
            id: Address::generate(&env),
            base: Address::generate(&env),
            quote: Address::generate(&env),
            pair_name: None,
            created_ledger: 100,
            deleted_ledger: None,
        };

        // Active market
        assert!(market.is_active(200));
        assert!(market.is_active(100));

        let deleted_market = MarketMetadata {
            deleted_ledger: Some(150),
            ..market.clone()
        };

        // Not active after deletion ledger
        assert!(!deleted_market.is_active(150));
        assert!(!deleted_market.is_active(200));

        // Still active before deletion ledger
        assert!(deleted_market.is_active(149));
    }

    #[test]
    fn test_pagination_cursor_defaults() {
        let cursor = PaginationCursor::default();
        assert_eq!(cursor.page, 0);
        assert!(!cursor.include_deleted);
    }

    #[test]
    fn test_market_page_properties() {
        use soroban_sdk::testutils::Address as _;
        use soroban_sdk::Env;

        let env = Env::default();
        let mut markets = Vec::new(&env);
        for _ in 0..50 {
            markets.push_back(Address::generate(&env));
        }

        let page = MarketPage {
            markets,
            page: 0,
            total_markets: 250,
            has_next: true,
        };

        assert_eq!(page.page, 0);
        assert_eq!(page.markets.len(), 50);
        assert!(page.has_next);
    }

    #[test]
    fn test_registry_entry_struct() {
        use soroban_sdk::testutils::Address as _;
        use soroban_sdk::Env;

        let env = Env::default();
        let metadata = MarketMetadata {
            id: Address::generate(&env),
            base: Address::generate(&env),
            quote: Address::generate(&env),
            pair_name: None,
            created_ledger: 100,
            deleted_ledger: None,
        };

        let entry = RegistryEntry {
            market: metadata.clone(),
        };

        assert_eq!(entry.market, metadata);
    }

    #[test]
    fn test_bounded_page_size() {
        assert_eq!(MARKETS_PER_PAGE, 100);
    }
}
