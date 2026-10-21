//! Market lifecycle state definitions and operation matrix.
//!
//! Issue #889 [CLOB-013]: Defines market lifecycle states and allowed-operation matrix.
//! Issue #890 [CLOB-014]: Implements emergency-mode exit paths and state-transition events.
//!
//! A market transitions through well-defined states. Emergency modes (Paused, ReduceOnly,
//! CancelOnly) must not trap user funds; cancellations and withdrawals always remain available.

use soroban_sdk::contracttype;

/// Market lifecycle state. Consensus-critical: do not change without protocol coordination.
#[contracttype]
#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd)]
#[repr(u32)]
pub enum MarketState {
    /// Market is registered but not yet operational. No trading allowed.
    Pending = 0,

    /// Market is operational and accepting all order types.
    /// Transitions: → ReduceOnly, CancelOnly, Paused, Closed
    Active = 1,

    /// Emergency mode: only position decreases and order cancellations allowed.
    /// Users can withdraw collateral and settled funds.
    /// Protects LP from fresh leverage; enables emergency de-risking.
    /// Transitions: → Active, Closed
    ReduceOnly = 2,

    /// Emergency mode: only order cancellations allowed, no position changes.
    /// Users can cancel orders and withdraw funds.
    /// Transitions: → Active, Closed
    CancelOnly = 3,

    /// Emergency pause: new orders rejected, but cancellations and withdrawals work.
    /// Existing positions persist; can be liquidated if underwater.
    /// Transitions: → Active, ReduceOnly, Closed
    Paused = 4,

    /// Market is permanently closed. No operations allowed except state queries.
    /// Historical data and positions remain readable.
    /// Terminal state: no transitions out.
    Closed = 5,
}

impl MarketState {
    /// Check if the state allows placing new orders.
    pub fn allows_new_orders(self) -> bool {
        matches!(self, Self::Active)
    }

    /// Check if the state allows position increases.
    pub fn allows_position_increase(self) -> bool {
        matches!(self, Self::Active)
    }

    /// Check if the state allows position decreases.
    pub fn allows_position_decrease(self) -> bool {
        matches!(self, Self::Active | Self::ReduceOnly)
    }

    /// Check if the state allows cancelling orders.
    pub fn allows_cancellation(self) -> bool {
        !matches!(self, Self::Pending | Self::Closed)
    }

    /// Check if the state allows withdrawals (including collateral recovery).
    pub fn allows_withdrawal(self) -> bool {
        !matches!(self, Self::Pending | Self::Closed)
    }

    /// Check if the state allows deposits.
    pub fn allows_deposit(self) -> bool {
        matches!(self, Self::Active)
    }

    /// Check if the state is an emergency mode (paused, reduce-only, or cancel-only).
    pub fn is_emergency_mode(self) -> bool {
        matches!(self, Self::ReduceOnly | Self::CancelOnly | Self::Paused)
    }

    /// Check if the state allows transitions out (i.e., is not a terminal state).
    pub fn allows_transitions(self) -> bool {
        !matches!(self, Self::Closed)
    }

    /// Return a human-readable description for UI/logging.
    pub fn description(self) -> &'static str {
        match self {
            Self::Pending => "Pending (setup in progress)",
            Self::Active => "Active (normal operations)",
            Self::ReduceOnly => "ReduceOnly (emergency: close positions only)",
            Self::CancelOnly => "CancelOnly (emergency: cancel orders only)",
            Self::Paused => "Paused (emergency pause)",
            Self::Closed => "Closed (permanently closed)",
        }
    }
}

/// State transition authorization matrix.
/// Specifies which roles can transition from one state to another.
/// Each role and transition pair encodes the required authorization.
#[contracttype]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TransitionAuthority {
    /// Admin-only state transitions (setup, emergency procedures).
    AdminOnly = 0,
    /// Keeper/bot can trigger under emergency conditions.
    KeeperOrAdmin = 1,
    /// Owner of the market/asset can trigger.
    OwnerOrAdmin = 2,
}

/// Allowed state transitions and their required authorization.
/// Returns Some(authority) if transition is allowed, None otherwise.
pub fn validate_state_transition(
    from: MarketState,
    to: MarketState,
) -> Option<TransitionAuthority> {
    use MarketState::*;

    match (from, to) {
        // From Pending
        (Pending, Active) => Some(TransitionAuthority::AdminOnly),
        (Pending, Closed) => Some(TransitionAuthority::AdminOnly),

        // From Active
        (Active, ReduceOnly) => Some(TransitionAuthority::KeeperOrAdmin),
        (Active, CancelOnly) => Some(TransitionAuthority::KeeperOrAdmin),
        (Active, Paused) => Some(TransitionAuthority::KeeperOrAdmin),
        (Active, Closed) => Some(TransitionAuthority::AdminOnly),

        // From ReduceOnly
        (ReduceOnly, Active) => Some(TransitionAuthority::AdminOnly),
        (ReduceOnly, Closed) => Some(TransitionAuthority::AdminOnly),

        // From CancelOnly
        (CancelOnly, Active) => Some(TransitionAuthority::AdminOnly),
        (CancelOnly, Closed) => Some(TransitionAuthority::AdminOnly),

        // From Paused
        (Paused, Active) => Some(TransitionAuthority::AdminOnly),
        (Paused, ReduceOnly) => Some(TransitionAuthority::KeeperOrAdmin),
        (Paused, Closed) => Some(TransitionAuthority::AdminOnly),

        // From Closed
        (Closed, _) => None, // Terminal state: no transitions out

        // All other transitions are invalid
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn market_state_active_allows_all_operations() {
        let state = MarketState::Active;
        assert!(state.allows_new_orders());
        assert!(state.allows_position_increase());
        assert!(state.allows_position_decrease());
        assert!(state.allows_cancellation());
        assert!(state.allows_withdrawal());
        assert!(state.allows_deposit());
        assert!(!state.is_emergency_mode());
    }

    #[test]
    fn market_state_reduce_only_allows_decreases_and_cancellations() {
        let state = MarketState::ReduceOnly;
        assert!(!state.allows_new_orders());
        assert!(!state.allows_position_increase());
        assert!(state.allows_position_decrease());
        assert!(state.allows_cancellation());
        assert!(state.allows_withdrawal());
        assert!(!state.allows_deposit());
        assert!(state.is_emergency_mode());
    }

    #[test]
    fn market_state_cancel_only_allows_cancellations_and_withdrawals() {
        let state = MarketState::CancelOnly;
        assert!(!state.allows_new_orders());
        assert!(!state.allows_position_increase());
        assert!(!state.allows_position_decrease());
        assert!(state.allows_cancellation());
        assert!(state.allows_withdrawal());
        assert!(!state.allows_deposit());
        assert!(state.is_emergency_mode());
    }

    #[test]
    fn market_state_paused_allows_cancellations_and_withdrawals() {
        let state = MarketState::Paused;
        assert!(!state.allows_new_orders());
        assert!(!state.allows_position_increase());
        assert!(!state.allows_position_decrease());
        assert!(state.allows_cancellation());
        assert!(state.allows_withdrawal());
        assert!(!state.allows_deposit());
        assert!(state.is_emergency_mode());
    }

    #[test]
    fn market_state_closed_allows_no_operations() {
        let state = MarketState::Closed;
        assert!(!state.allows_new_orders());
        assert!(!state.allows_position_increase());
        assert!(!state.allows_position_decrease());
        assert!(!state.allows_cancellation());
        assert!(!state.allows_withdrawal());
        assert!(!state.allows_deposit());
        assert!(!state.allows_transitions());
    }

    #[test]
    fn valid_transitions_from_active() {
        assert_eq!(
            validate_state_transition(MarketState::Active, MarketState::ReduceOnly),
            Some(TransitionAuthority::KeeperOrAdmin)
        );
        assert_eq!(
            validate_state_transition(MarketState::Active, MarketState::CancelOnly),
            Some(TransitionAuthority::KeeperOrAdmin)
        );
        assert_eq!(
            validate_state_transition(MarketState::Active, MarketState::Paused),
            Some(TransitionAuthority::KeeperOrAdmin)
        );
        assert_eq!(
            validate_state_transition(MarketState::Active, MarketState::Closed),
            Some(TransitionAuthority::AdminOnly)
        );
    }

    #[test]
    fn closed_state_is_terminal() {
        assert_eq!(
            validate_state_transition(MarketState::Closed, MarketState::Active),
            None
        );
        assert_eq!(
            validate_state_transition(MarketState::Closed, MarketState::ReduceOnly),
            None
        );
    }

    #[test]
    fn reduce_only_to_active_requires_admin() {
        assert_eq!(
            validate_state_transition(MarketState::ReduceOnly, MarketState::Active),
            Some(TransitionAuthority::AdminOnly)
        );
    }
}
