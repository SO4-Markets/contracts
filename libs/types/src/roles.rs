// Issue #887 [CLOB-011] & #888 [CLOB-012]: Define protocol roles and trusted-address policy
//! Protocol authorization roles and permission boundaries.
//!
//! The CLOB separates permissionless trading from privileged administration.
//! No user trading path requires an admin/keeper role.
//! All role checks are explicit and documented.

use soroban_sdk::{contracttype, Address};

/// Protocol authorization roles. Consensus-critical: do not change without coordination.
#[contracttype]
#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd)]
#[repr(u32)]
pub enum Role {
    /// Super-admin: grant/revoke all roles, emergency pause, contract upgrade authorization.
    /// CRITICAL: this address must be guarded with multi-sig or timelock.
    Admin = 0,

    /// Market administrator: create markets, update market configuration, set emergency states.
    /// Does NOT control matching or custody.
    MarketAdmin = 1,

    /// Fee administrator: set fee rates, update fee recipient address, sweep fees.
    /// Does NOT control user funds.
    FeeAdmin = 2,

    /// Emergency pause authority: transition market to Paused/ReduceOnly/CancelOnly states.
    /// Used to stop trading without full market closure.
    PauseAuthority = 3,

    /// Oracle price setter: trusted price sources for liquidation and liquidation-trigger calculations.
    /// Multiple oracles can hold this role (append-only list).
    OracleSetter = 4,

    /// Contract upgrade authority: submit new WASM hash for upgradeable contracts.
    /// Separate from Admin to allow controlled rollouts.
    UpgradeAuthority = 5,
}

/// Permission matrix: which entrypoints require which roles.
/// Consensus-critical: changes must be validated and tested.
pub struct PermissionMap;

impl PermissionMap {
    /// Check if an entrypoint is permissionless.
    pub fn is_permissionless(entrypoint: &str) -> bool {
        matches!(
            entrypoint,
            // Trading operations
            "place_order"
                | "cancel_order"
                | "close_position"
                | "settle_order"
                // User fund operations
                | "deposit"
                | "withdraw"
                | "claim_liquidation"
                // Queries
                | "get_market_state"
                | "get_order"
                | "get_position"
                | "get_balances"
        )
    }

    /// Get the required role(s) for an entrypoint.
    /// Returns None if entrypoint is permissionless.
    pub fn required_role(entrypoint: &str) -> Option<Role> {
        match entrypoint {
            // Market management
            "create_market" => Some(Role::MarketAdmin),
            "update_market_config" => Some(Role::MarketAdmin),
            "set_market_state" => Some(Role::MarketAdmin),
            "close_market" => Some(Role::MarketAdmin),

            // Emergency control
            "pause_market" => Some(Role::PauseAuthority),
            "set_reduce_only" => Some(Role::PauseAuthority),
            "set_cancel_only" => Some(Role::PauseAuthority),
            "resume_market" => Some(Role::PauseAuthority),

            // Fee management
            "set_fee_rate" => Some(Role::FeeAdmin),
            "set_fee_recipient" => Some(Role::FeeAdmin),
            "sweep_fees" => Some(Role::FeeAdmin),

            // Oracle operations
            "set_oracle_price" => Some(Role::OracleSetter),
            "update_oracle" => Some(Role::OracleSetter),

            // Role management (super-admin only)
            "grant_role" => Some(Role::Admin),
            "revoke_role" => Some(Role::Admin),
            "rotate_role" => Some(Role::Admin),

            // Upgrade (upgrade authority)
            "propose_upgrade" => Some(Role::UpgradeAuthority),
            "confirm_upgrade" => Some(Role::UpgradeAuthority),

            // Fallback: assume restricted
            _ => Some(Role::Admin),
        }
    }
}

/// Trusted address configuration.
/// Immutable addresses reduce attack surface vs. caller-supplied infrastructure.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TrustedAddresses {
    /// Core market admin (can update this struct if needed for migration).
    pub admin: Address,

    /// Oracle provider (may be a contract or multi-sig).
    pub oracle: Address,

    /// Fee recipient address (receives protocol fees).
    pub fee_recipient: Address,

    /// Emergency pause authority (separate from admin).
    pub pause_authority: Address,

    /// Optional upgrade authority contract/account.
    pub upgrade_authority: Option<Address>,
}

impl TrustedAddresses {
    /// Create a new trusted addresses configuration.
    pub fn new(
        admin: Address,
        oracle: Address,
        fee_recipient: Address,
        pause_authority: Address,
        upgrade_authority: Option<Address>,
    ) -> Self {
        TrustedAddresses {
            admin,
            oracle,
            fee_recipient,
            pause_authority,
            upgrade_authority,
        }
    }

    /// Get the address for a given role.
    pub fn address_for_role(&self, role: Role) -> Option<&Address> {
        match role {
            Role::Admin => Some(&self.admin),
            Role::OracleSetter => Some(&self.oracle),
            Role::FeeAdmin => Some(&self.fee_recipient),
            Role::PauseAuthority => Some(&self.pause_authority),
            Role::UpgradeAuthority => self.upgrade_authority.as_ref(),
            _ => None,
        }
    }
}

/// Role state: maps role to current holder address.
/// Supports role rotation via time-delayed transitions.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RoleState {
    /// Current role holder.
    pub current: Address,

    /// Pending new holder (if role rotation is in progress).
    pub pending: Option<Address>,

    /// Timestamp when pending role becomes active (if set).
    pub rotation_timestamp: Option<u64>,
}

impl RoleState {
    /// Create a new role state with an initial holder.
    pub fn new(initial_holder: Address) -> Self {
        RoleState {
            current: initial_holder,
            pending: None,
            rotation_timestamp: None,
        }
    }

    /// Propose a role rotation to a new address.
    /// Rotation takes effect at `rotation_timestamp`.
    pub fn propose_rotation(&mut self, new_holder: Address, delay_seconds: u64, now: u64) {
        self.pending = Some(new_holder);
        self.rotation_timestamp = Some(now + delay_seconds);
    }

    /// Cancel a pending role rotation.
    pub fn cancel_rotation(&mut self) {
        self.pending = None;
        self.rotation_timestamp = None;
    }

    /// Finalize a pending role rotation if the time has passed.
    /// Returns true if rotation was applied, false otherwise.
    pub fn finalize_rotation(&mut self, now: u64) -> bool {
        if let Some(pending) = self.pending.take() {
            if let Some(ts) = self.rotation_timestamp.take() {
                if now >= ts {
                    self.current = pending;
                    return true;
                }
                // Put it back; not yet time
                self.pending = Some(pending);
                self.rotation_timestamp = Some(ts);
            }
        }
        false
    }

    /// Revoke a role (remove pending rotation).
    pub fn revoke(&mut self) {
        self.pending = None;
        self.rotation_timestamp = None;
    }

    /// Check if a given address holds this role.
    pub fn has_role(&self, addr: &Address, now: u64) -> bool {
        if self.current == *addr {
            return true;
        }
        // Check if pending and time has elapsed
        if let (Some(pending), Some(ts)) = (&self.pending, self.rotation_timestamp) {
            if *pending == *addr && now >= ts {
                return true;
            }
        }
        false
    }
}

/// Events for role transitions.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum RoleEvent {
    /// A role was granted to an address.
    RoleGranted {
        role: Role,
        to: Address,
        by: Address,
    },

    /// A role rotation was proposed.
    RotationProposed {
        role: Role,
        from: Address,
        to: Address,
        effective_at: u64,
        by: Address,
    },

    /// A role rotation was finalized.
    RotationFinalized {
        role: Role,
        from: Address,
        to: Address,
        by: Address,
    },

    /// A role rotation was cancelled.
    RotationCancelled {
        role: Role,
        pending: Address,
        by: Address,
    },

    /// A role was revoked from an address.
    RoleRevoked {
        role: Role,
        from: Address,
        by: Address,
    },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_permission_map() {
        // Permissionless operations
        assert!(PermissionMap::is_permissionless("place_order"));
        assert!(PermissionMap::is_permissionless("withdraw"));
        assert!(PermissionMap::is_permissionless("get_market_state"));

        // Restricted operations
        assert!(!PermissionMap::is_permissionless("create_market"));
        assert!(!PermissionMap::is_permissionless("pause_market"));
        assert!(!PermissionMap::is_permissionless("grant_role"));

        // Role requirements
        assert_eq!(
            PermissionMap::required_role("create_market"),
            Some(Role::MarketAdmin)
        );
        assert_eq!(
            PermissionMap::required_role("pause_market"),
            Some(Role::PauseAuthority)
        );
        assert_eq!(PermissionMap::required_role("place_order"), None);
    }

    #[test]
    fn test_role_state_rotation() {
        let initial = Address::random(&soroban_sdk::Env::default());
        let new_holder = Address::random(&soroban_sdk::Env::default());

        let mut role_state = RoleState::new(initial.clone());

        // Initially, only initial holder has role
        assert!(role_state.has_role(&initial, 0));
        assert!(!role_state.has_role(&new_holder, 0));

        // Propose rotation at time 1000
        role_state.propose_rotation(new_holder.clone(), 500, 0);

        // Before rotation time, old holder still active
        assert!(role_state.has_role(&initial, 100));
        assert!(!role_state.has_role(&new_holder, 100));

        // At rotation time
        assert!(role_state.has_role(&new_holder, 500));

        // Finalize rotation
        assert!(role_state.finalize_rotation(600));
        assert!(!role_state.has_role(&initial, 600));
        assert!(role_state.has_role(&new_holder, 600));
    }

    #[test]
    fn test_role_revocation() {
        let initial = Address::random(&soroban_sdk::Env::default());
        let pending = Address::random(&soroban_sdk::Env::default());

        let mut role_state = RoleState::new(initial.clone());
        role_state.propose_rotation(pending.clone(), 100, 0);

        // Revoke the pending rotation
        role_state.revoke();
        assert!(role_state.has_role(&initial, 0));
        assert!(!role_state.has_role(&pending, 150)); // Never activates
    }
}