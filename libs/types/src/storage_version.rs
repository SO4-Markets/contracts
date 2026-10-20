// Support for Issue #891: Storage versioning and migration for contract upgrades
//! Storage version tracking and migration helpers for upgradeable contracts.
//!
//! Each upgradeable contract must check its storage version at init and verify
//! compatibility before processing. Major version changes trigger migration.

use soroban_sdk::{contracttype, Env};

/// Storage version triple: major.minor.patch
/// Changes are consensus-critical and must be coordinated.
#[contracttype]
#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd)]
pub struct StorageVersion {
    /// Major version: incompatible changes (migration required).
    pub major: u32,
    /// Minor version: backward-compatible additions.
    pub minor: u32,
    /// Patch version: bug fixes, no schema change.
    pub patch: u32,
}

impl StorageVersion {
    /// Create a new storage version.
    pub const fn new(major: u32, minor: u32, patch: u32) -> Self {
        StorageVersion { major, minor, patch }
    }

    /// Check if this version is compatible with a required version.
    /// Compatible if same major, and at least the minor/patch version.
    pub fn is_compatible_with(&self, required: StorageVersion) -> bool {
        self.major == required.major && (self.minor > required.minor || (self.minor == required.minor && self.patch >= required.patch))
    }

    /// Check if migration is needed (major version changed).
    pub fn migration_needed(&self, new_version: StorageVersion) -> bool {
        self.major != new_version.major
    }
}

/// Data keys for version and upgrade tracking.
#[contracttype]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum VersionDataKey {
    /// Current storage version.
    StorageVersion,
    /// Pending WASM hash for upgrade.
    PendingUpgrade,
    /// Timestamp of pending upgrade proposal.
    PendingUpgradeTimestamp,
    /// Previous WASM hashes (for rollback).
    PreviousVersions,
}

/// Upgrade proposal state.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct UpgradeProposal {
    /// New WASM hash.
    pub wasm_hash: [u8; 32],
    /// When this upgrade was proposed.
    pub proposed_at: u64,
    /// When this upgrade becomes effective (proposed_at + timelock).
    pub effective_at: u64,
}

/// Initialize storage version for a new contract.
/// Must be called once before the contract processes any requests.
pub fn initialize_version(env: &Env, version: StorageVersion) -> Result<(), &'static str> {
    let storage = env.storage().instance();

    // Check if already initialized
    if storage
        .get::<VersionDataKey, StorageVersion>(&VersionDataKey::StorageVersion)
        .is_some()
    {
        return Err("Storage version already initialized");
    }

    storage.set(&VersionDataKey::StorageVersion, &version);
    Ok(())
}

/// Verify the current storage version is compatible.
/// Call this at the start of every upgradeable contract's handler.
pub fn verify_version(env: &Env, required: StorageVersion) -> Result<StorageVersion, &'static str> {
    let storage = env.storage().instance();

    let current: StorageVersion = storage
        .get(&VersionDataKey::StorageVersion)
        .ok_or("Storage not initialized")?;

    if !current.is_compatible_with(required) {
        return Err("Storage version incompatible");
    }

    Ok(current)
}

/// Propose an upgrade to a new WASM hash with a timelock.
pub fn propose_upgrade(
    env: &Env,
    wasm_hash: [u8; 32],
    timelock_seconds: u64,
) -> Result<UpgradeProposal, &'static str> {
    let now = env.ledger().timestamp();
    let effective_at = now.checked_add(timelock_seconds).ok_or("Timelock overflow")?;

    let proposal = UpgradeProposal {
        wasm_hash,
        proposed_at: now,
        effective_at,
    };

    let storage = env.storage().instance();
    storage.set(&VersionDataKey::PendingUpgrade, &proposal);

    Ok(proposal)
}

/// Get the current pending upgrade (if any).
pub fn get_pending_upgrade(env: &Env) -> Option<UpgradeProposal> {
    env.storage().instance().get(&VersionDataKey::PendingUpgrade)
}

/// Check if a pending upgrade has passed its timelock.
pub fn is_upgrade_ready(env: &Env) -> bool {
    if let Some(proposal) = get_pending_upgrade(env) {
        env.ledger().timestamp() >= proposal.effective_at
    } else {
        false
    }
}

/// Confirm and execute a pending upgrade (must be after timelock).
/// Moves the current version to `previous_versions` for rollback eligibility.
pub fn confirm_upgrade(
    env: &Env,
    new_version: StorageVersion,
) -> Result<StorageVersion, &'static str> {
    let storage = env.storage().instance();

    let proposal = storage
        .get::<VersionDataKey, UpgradeProposal>(&VersionDataKey::PendingUpgrade)
        .ok_or("No pending upgrade")?;

    // Check timelock
    if env.ledger().timestamp() < proposal.effective_at {
        return Err("Timelock not elapsed");
    }

    // Get current version
    let current: StorageVersion = storage
        .get(&VersionDataKey::StorageVersion)
        .ok_or("Current version not found")?;

    // Store old version in history
    let mut history: Vec<StorageVersion> = storage
        .get::<VersionDataKey, Vec<StorageVersion>>(&VersionDataKey::PreviousVersions)
        .unwrap_or_else(|| Vec::new(env));

    history.push_back(current);
    storage.set(&VersionDataKey::PreviousVersions, &history);

    // Update to new version
    storage.set(&VersionDataKey::StorageVersion, &new_version);

    // Clear pending upgrade
    storage.remove(&VersionDataKey::PendingUpgrade);

    Ok(new_version)
}

/// Rollback to a previous version (only allowed within rollback window).
/// Only Admin can call this within 7 days of upgrade.
pub fn rollback_to_previous(env: &Env, rollback_window_seconds: u64) -> Result<StorageVersion, &'static str> {
    let storage = env.storage().instance();

    let current: StorageVersion = storage
        .get(&VersionDataKey::StorageVersion)
        .ok_or("Current version not found")?;

    let proposal = storage
        .get::<VersionDataKey, UpgradeProposal>(&VersionDataKey::PendingUpgrade)
        .or_else(|| {
            // If no pending, find the most recent upgrade (from history)
            let history: Vec<StorageVersion> = storage
                .get(&VersionDataKey::PreviousVersions)
                .unwrap_or_else(|| Vec::new(env));
            history.back()
        });

    if let Some(p) = proposal {
        if env.ledger().timestamp() > p.effective_at + rollback_window_seconds {
            return Err("Rollback window expired");
        }
    } else {
        return Err("No upgrade to rollback from");
    }

    // Get previous version
    let mut history: Vec<StorageVersion> = storage
        .get(&VersionDataKey::PreviousVersions)
        .ok_or("No previous versions available")?;

    let previous = history.pop().ok_or("No previous version")?;

    // Update history and current version
    storage.set(&VersionDataKey::PreviousVersions, &history);
    storage.set(&VersionDataKey::StorageVersion, &previous);

    Ok(previous)
}

/// Migration entrypoint: transform data from old schema to new schema.
/// Must be implemented by each contract as a separate function that can be called
/// by the old contract to export data, then called by the new contract to import.
pub trait StorageMigration {
    /// Export data from old contract in a format the new contract can import.
    fn export_state(env: &Env) -> Result<Vec<u8>, &'static str>;

    /// Import data and transform for new schema.
    fn import_state(env: &Env, exported: Vec<u8>) -> Result<(), &'static str>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_version_compatibility() {
        let v1_0_0 = StorageVersion::new(1, 0, 0);
        let v1_1_0 = StorageVersion::new(1, 1, 0);
        let v1_0_5 = StorageVersion::new(1, 0, 5);
        let v2_0_0 = StorageVersion::new(2, 0, 0);

        // Same major, higher minor/patch is compatible
        assert!(v1_1_0.is_compatible_with(v1_0_0));
        assert!(v1_0_5.is_compatible_with(v1_0_0));

        // Different major is not compatible
        assert!(!v2_0_0.is_compatible_with(v1_0_0));

        // Older version is not compatible with newer
        assert!(!v1_0_0.is_compatible_with(v1_1_0));
    }

    #[test]
    fn test_migration_check() {
        let v1 = StorageVersion::new(1, 5, 3);
        let v2 = StorageVersion::new(2, 0, 0);

        assert!(v1.migration_needed(v2));
        assert!(!v1.migration_needed(StorageVersion::new(1, 6, 0)));
    }

    #[test]
    fn test_version_ordering() {
        let v1_0_0 = StorageVersion::new(1, 0, 0);
        let v1_0_1 = StorageVersion::new(1, 0, 1);
        let v1_1_0 = StorageVersion::new(1, 1, 0);
        let v2_0_0 = StorageVersion::new(2, 0, 0);

        assert!(v1_0_0 < v1_0_1);
        assert!(v1_0_1 < v1_1_0);
        assert!(v1_1_0 < v2_0_0);
    }
}