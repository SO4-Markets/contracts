// Issue #30: Implement account position list support
// Track active position keys by account for reader and frontend pagination
// Full close must remove the key; partial close must preserve it

use multiversx_sc::prelude::*;

#[derive(TopEncode, TopDecode, NestedEncode, NestedDecode, Clone, Debug, PartialEq, Eq)]
pub struct PositionKey {
    pub account: ManagedAddress,
    pub market: ManagedAddress,
    pub collateral_token: TokenIdentifier,
    pub is_long: bool,
}

#[multiversx_sc::module]
pub trait PositionListModule {
    #[storage_mapper("account_positions")]
    fn account_positions(&self, account: &ManagedAddress) -> UnorderedSetMapper<PositionKey>;

    #[storage_mapper("position_count")]
    fn position_count(&self, account: &ManagedAddress) -> SingleValueMapper<u64>;

    /// Add a position key to the account's position list
    /// Issue #30: Track active position keys by account
    fn add_position_key(&self, account: &ManagedAddress, position_key: PositionKey) {
        let mut positions = self.account_positions(account);
        if positions.insert(position_key.clone()) {
            let count = self.position_count(account).get();
            self.position_count(account).set(count + 1);
        }
    }

    /// Remove a position key from the account's position list
    /// Issue #30: Full close must remove the key from the account list
    fn remove_position_key(&self, account: &ManagedAddress, position_key: &PositionKey) {
        let mut positions = self.account_positions(account);
        if positions.remove(position_key) {
            let count = self.position_count(account).get();
            if count > 0 {
                self.position_count(account).set(count - 1);
            }
        }
    }

    /// Check if a position key exists for an account
    fn has_position_key(&self, account: &ManagedAddress, position_key: &PositionKey) -> bool {
        self.account_positions(account).contains(position_key)
    }

    /// Get all position keys for an account
    fn get_account_positions(&self, account: &ManagedAddress) -> Vec<PositionKey> {
        self.account_positions(account)
            .iter()
            .collect()
    }

    /// Get paginated position keys for an account
    /// Issue #30: Pagination returns stable expected keys
    fn get_account_positions_paginated(
        &self,
        account: &ManagedAddress,
        page: u64,
        page_size: u64,
    ) -> Vec<PositionKey> {
        let positions = self.account_positions(account);
        let total = positions.len() as u64;
        
        if page == 0 || page_size == 0 {
            return Vec::new();
        }

        let start = (page - 1) * page_size;
        if start >= total {
            return Vec::new();
        }

        let end = core::cmp::min(start + page_size, total);
        
        positions
            .iter()
            .skip(start as usize)
            .take((end - start) as usize)
            .collect()
    }

    /// Get the total number of positions for an account
    fn get_account_position_count(&self, account: &ManagedAddress) -> u64 {
        self.position_count(account).get()
    }

    /// Clear all positions for an account
    fn clear_account_positions(&self, account: &ManagedAddress) {
        self.account_positions(account).clear();
        self.position_count(account).set(0);
    }
}
