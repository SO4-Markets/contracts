// Issue #31: Add storage TTL extension policy
// Implement automatic storage cleanup and TTL management

use multiversx_sc::prelude::*;

#[derive(TopEncode, TopDecode, NestedEncode, NestedDecode, Clone, Debug)]
pub struct StorageTTLPolicy {
    pub key: ManagedBuffer,
    pub ttl_seconds: u64,
    pub created_at: u64,
    pub last_accessed: u64,
    pub auto_extend: bool,
}

#[multiversx_sc::module]
pub trait StorageTTLModule {
    #[storage_mapper("storage_ttl_policies")]
    fn storage_ttl_policies(&self) -> MapMapper<ManagedBuffer, StorageTTLPolicy>;

    #[storage_mapper("expired_keys")]
    fn expired_keys(&self) -> UnorderedSetMapper<ManagedBuffer>;

    #[storage_mapper("ttl_extension_count")]
    fn ttl_extension_count(&self) -> SingleValueMapper<u64>;

    /// Set a TTL policy for a storage key
    /// Issue #31: Add storage TTL extension policy
    fn set_ttl_policy(
        &self,
        key: ManagedBuffer,
        ttl_seconds: u64,
        auto_extend: bool,
    ) {
        let now = self.blockchain().get_block_timestamp();
        
        let policy = StorageTTLPolicy {
            key: key.clone(),
            ttl_seconds,
            created_at: now,
            last_accessed: now,
            auto_extend,
        };

        self.storage_ttl_policies().insert(key, policy);
    }

    /// Extend the TTL for a storage key
    fn extend_ttl(&self, key: &ManagedBuffer, additional_seconds: u64) -> bool {
        if let Some(mut policy) = self.storage_ttl_policies().get(key) {
            let now = self.blockchain().get_block_timestamp();
            
            // Calculate new expiration
            let current_expiration = policy.created_at + policy.ttl_seconds;
            let new_expiration = current_expiration + additional_seconds;
            
            // Update policy
            policy.ttl_seconds = new_expiration - policy.created_at;
            policy.last_accessed = now;
            
            self.storage_ttl_policies().insert(key.clone(), policy);
            
            let count = self.ttl_extension_count().get();
            self.ttl_extension_count().set(count + 1);
            
            true
        } else {
            false
        }
    }

    /// Check if a storage key has expired
    fn is_expired(&self, key: &ManagedBuffer) -> bool {
        if let Some(policy) = self.storage_ttl_policies().get(key) {
            let now = self.blockchain().get_block_timestamp();
            let expiration = policy.created_at + policy.ttl_seconds;
            now > expiration
        } else {
            false
        }
    }

    /// Get the remaining TTL for a storage key
    fn get_remaining_ttl(&self, key: &ManagedBuffer) -> u64 {
        if let Some(policy) = self.storage_ttl_policies().get(key) {
            let now = self.blockchain().get_block_timestamp();
            let expiration = policy.created_at + policy.ttl_seconds;
            
            if now >= expiration {
                0
            } else {
                expiration - now
            }
        } else {
            0
        }
    }

    /// Clean up expired storage keys
    fn cleanup_expired_keys(&self) -> u64 {
        let mut cleaned_count = 0u64;
        let mut keys_to_remove = Vec::new();

        for (key, _) in self.storage_ttl_policies().iter() {
            if self.is_expired(&key) {
                keys_to_remove.push(key);
            }
        }

        for key in keys_to_remove {
            self.storage_ttl_policies().remove(&key);
            self.expired_keys().insert(key);
            cleaned_count += 1;
        }

        cleaned_count
    }

    /// Auto-extend TTL for keys with auto_extend enabled
    fn auto_extend_ttls(&self) -> u64 {
        let mut extended_count = 0u64;
        let extension_amount = 86400u64; // 24 hours

        for (key, policy) in self.storage_ttl_policies().iter() {
            if policy.auto_extend && self.is_expired(&key) {
                if self.extend_ttl(&key, extension_amount) {
                    extended_count += 1;
                }
            }
        }

        extended_count
    }

    /// Get TTL policy for a key
    fn get_ttl_policy(&self, key: &ManagedBuffer) -> Option<StorageTTLPolicy> {
        self.storage_ttl_policies().get(key)
    }

    /// Remove a TTL policy
    fn remove_ttl_policy(&self, key: &ManagedBuffer) -> bool {
        self.storage_ttl_policies().remove(key)
    }
}
