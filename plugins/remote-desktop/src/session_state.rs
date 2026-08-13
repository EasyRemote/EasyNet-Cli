// EasyNet CLI — remote desktop session state machine
// ===================================================
//
// File: plugins/remote-desktop/src/session_state.rs
// Description: Lifecycle state and terminal reason for one remote desktop
// session.

use super::contract::RemoteDesktopSessionState;

/// Product-owned lifecycle vocabulary used by remote desktop projections.
pub(in crate::daemon::plugins::remote_desktop) type RemoteDesktopState = RemoteDesktopSessionState;

/// Lifecycle state for one remote desktop session.
///
/// This type owns only the lifecycle enum and terminal reason. It does not own
/// lease timestamps, signaling payloads, transport readiness, or event
/// emission; those stay in their dedicated modules.
///
/// Invariant 1: once the state is terminal, every mutating transition returns
/// `false` and leaves both state and end reason unchanged.
///
/// Invariant 2: terminal reasons are written exactly by terminal transitions:
/// close, expire, or fail.
#[derive(Debug, Clone)]
pub(in crate::daemon::plugins::remote_desktop) struct RemoteDesktopSessionStateMachine {
    state: RemoteDesktopState,
    end_reason: Option<String>,
}

impl RemoteDesktopSessionStateMachine {
    /// Construct a non-terminal session state machine in negotiating state.
    pub(in crate::daemon::plugins::remote_desktop) fn new() -> Self {
        Self {
            state: RemoteDesktopState::Negotiating,
            end_reason: None,
        }
    }

    /// Current lifecycle state.
    pub(in crate::daemon::plugins::remote_desktop) fn state(&self) -> RemoteDesktopState {
        self.state
    }

    /// Whether the lifecycle has reached a terminal state.
    pub(in crate::daemon::plugins::remote_desktop) fn is_terminal(&self) -> bool {
        self.state.is_terminal()
    }

    /// Operator-visible reason captured by the terminal transition.
    pub(in crate::daemon::plugins::remote_desktop) fn end_reason(&self) -> Option<&str> {
        self.end_reason.as_deref()
    }

    /// Move a non-terminal session back into negotiation.
    pub(in crate::daemon::plugins::remote_desktop) fn mark_negotiating(&mut self) -> bool {
        self.set_non_terminal_state(RemoteDesktopState::Negotiating)
    }

    /// Mark the diagnostic InvokeBidi preview path connected.
    pub(in crate::daemon::plugins::remote_desktop) fn mark_preview_connected(&mut self) -> bool {
        self.set_non_terminal_state(RemoteDesktopState::ConnectedPreview)
    }

    /// Mark the production media plane connected.
    pub(in crate::daemon::plugins::remote_desktop) fn mark_connected(&mut self) -> bool {
        self.set_non_terminal_state(RemoteDesktopState::Connected)
    }

    /// Mark a production transport whose sender is live but whose client is not
    /// currently presenting media.
    pub(in crate::daemon::plugins::remote_desktop) fn mark_degraded(&mut self) -> bool {
        self.set_non_terminal_state(RemoteDesktopState::Degraded)
    }

    /// Suspend the session because the selected target is no longer capturable.
    ///
    /// This is distinct from transport degradation: the transport may still be
    /// alive, but capture and input must stop for the bound target.
    pub(in crate::daemon::plugins::remote_desktop) fn mark_suspended(&mut self) -> bool {
        self.set_non_terminal_state(RemoteDesktopState::Suspended)
    }

    /// Enter the caller-requested closing phase before final close.
    pub(in crate::daemon::plugins::remote_desktop) fn mark_closing(&mut self) -> bool {
        self.set_non_terminal_state(RemoteDesktopState::Closing)
    }

    /// Finalize a caller-requested close after the closing event was emitted.
    pub(in crate::daemon::plugins::remote_desktop) fn mark_closed(&mut self, reason: &str) {
        self.state = RemoteDesktopState::Closed;
        self.end_reason = Some(reason.to_string());
    }

    /// Close the session through the lease-expiry terminal path.
    pub(in crate::daemon::plugins::remote_desktop) fn expire(&mut self, reason: &str) -> bool {
        if self.is_terminal() {
            return false;
        }
        self.mark_closed(reason);
        true
    }

    /// Fail the session through a transport/backend terminal path.
    pub(in crate::daemon::plugins::remote_desktop) fn fail(&mut self, reason: &str) -> bool {
        if self.is_terminal() {
            return false;
        }
        self.state = RemoteDesktopState::Failed;
        self.end_reason = Some(reason.to_string());
        true
    }

    fn set_non_terminal_state(&mut self, state: RemoteDesktopState) -> bool {
        if self.is_terminal() {
            return false;
        }
        if self.state == state {
            return false;
        }
        let allowed = match self.state {
            RemoteDesktopState::Negotiating => matches!(
                state,
                RemoteDesktopState::Connected
                    | RemoteDesktopState::ConnectedPreview
                    | RemoteDesktopState::Suspended
                    | RemoteDesktopState::Degraded
                    | RemoteDesktopState::Closing
            ),
            RemoteDesktopState::ConnectedPreview => matches!(
                state,
                RemoteDesktopState::Negotiating
                    | RemoteDesktopState::Connected
                    | RemoteDesktopState::Suspended
                    | RemoteDesktopState::Degraded
                    | RemoteDesktopState::Closing
            ),
            RemoteDesktopState::Connected => matches!(
                state,
                RemoteDesktopState::Negotiating
                    | RemoteDesktopState::Suspended
                    | RemoteDesktopState::Degraded
                    | RemoteDesktopState::Closing
            ),
            RemoteDesktopState::Degraded => matches!(
                state,
                RemoteDesktopState::Negotiating
                    | RemoteDesktopState::Connected
                    | RemoteDesktopState::ConnectedPreview
                    | RemoteDesktopState::Suspended
                    | RemoteDesktopState::Closing
            ),
            RemoteDesktopState::Suspended => matches!(state, RemoteDesktopState::Closing),
            RemoteDesktopState::Closing => matches!(state, RemoteDesktopState::Closed),
            RemoteDesktopState::Closed | RemoteDesktopState::Failed => false,
            RemoteDesktopState::Pending | RemoteDesktopState::Unspecified => {
                matches!(state, RemoteDesktopState::Negotiating)
            }
        };
        if !allowed {
            return false;
        }
        self.state = state;
        true
    }
}

#[cfg(test)]
mod tests {
    use super::{RemoteDesktopSessionStateMachine, RemoteDesktopState};

    #[test]
    fn remote_desktop_session_state_keeps_terminal_reason_stable() {
        let mut state = RemoteDesktopSessionStateMachine::new();

        assert!(state.fail("webrtc_failed"));
        assert!(!state.expire("lease_expired"));
        assert!(!state.mark_connected());

        assert_eq!(state.state(), RemoteDesktopState::Failed);
        assert_eq!(state.end_reason(), Some("webrtc_failed"));
    }

    #[test]
    fn remote_desktop_session_state_allows_non_terminal_transitions_before_close() {
        let mut state = RemoteDesktopSessionStateMachine::new();

        assert!(state.mark_preview_connected());
        assert_eq!(state.state(), RemoteDesktopState::ConnectedPreview);
        assert!(state.mark_closing());
        assert_eq!(state.state(), RemoteDesktopState::Closing);
        state.mark_closed("caller_ended");

        assert_eq!(state.state(), RemoteDesktopState::Closed);
        assert_eq!(state.end_reason(), Some("caller_ended"));
    }

    #[test]
    fn remote_desktop_session_state_suspends_target_loss_until_close() {
        let mut state = RemoteDesktopSessionStateMachine::new();

        assert!(state.mark_connected());
        assert!(state.mark_suspended());
        assert_eq!(state.state(), RemoteDesktopState::Suspended);
        assert!(!state.mark_negotiating());
        assert!(!state.mark_connected());
        assert!(state.mark_closing());
        state.mark_closed("caller_ended");

        assert_eq!(state.state(), RemoteDesktopState::Closed);
    }
}
