// EasyNet CLI — remote desktop session state machine
// ===================================================
//
// File: src/plugins/builtin/remote_desktop/session_state.rs
// Description: Lifecycle state and terminal reason for one remote desktop
// session.

use easynet_axon::RemoteDesktopSessionState;

/// Axon wire-facing lifecycle enum used by remote desktop session projections.
pub(in crate::plugins::builtin::remote_desktop) type RemoteDesktopState = RemoteDesktopSessionState;

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
pub(in crate::plugins::builtin::remote_desktop) struct RemoteDesktopSessionStateMachine {
    state: RemoteDesktopState,
    end_reason: Option<String>,
}

impl RemoteDesktopSessionStateMachine {
    /// Construct a non-terminal session state machine in negotiating state.
    pub(in crate::plugins::builtin::remote_desktop) fn new() -> Self {
        Self {
            state: RemoteDesktopState::Negotiating,
            end_reason: None,
        }
    }

    /// Current lifecycle state.
    pub(in crate::plugins::builtin::remote_desktop) fn state(&self) -> RemoteDesktopState {
        self.state
    }

    /// Whether the lifecycle has reached a terminal state.
    pub(in crate::plugins::builtin::remote_desktop) fn is_terminal(&self) -> bool {
        self.state.is_terminal()
    }

    /// Operator-visible reason captured by the terminal transition.
    pub(in crate::plugins::builtin::remote_desktop) fn end_reason(&self) -> Option<&str> {
        self.end_reason.as_deref()
    }

    /// Move a non-terminal session back into negotiation.
    pub(in crate::plugins::builtin::remote_desktop) fn mark_negotiating(&mut self) -> bool {
        self.set_non_terminal_state(RemoteDesktopState::Negotiating)
    }

    /// Mark the diagnostic InvokeBidi preview path connected.
    pub(in crate::plugins::builtin::remote_desktop) fn mark_preview_connected(&mut self) -> bool {
        self.set_non_terminal_state(RemoteDesktopState::ConnectedPreview)
    }

    /// Mark the production media plane connected.
    pub(in crate::plugins::builtin::remote_desktop) fn mark_connected(&mut self) -> bool {
        self.set_non_terminal_state(RemoteDesktopState::Connected)
    }

    /// Enter the caller-requested closing phase before final close.
    pub(in crate::plugins::builtin::remote_desktop) fn mark_closing(&mut self) -> bool {
        self.set_non_terminal_state(RemoteDesktopState::Closing)
    }

    /// Finalize a caller-requested close after the closing event was emitted.
    pub(in crate::plugins::builtin::remote_desktop) fn mark_closed(&mut self, reason: &str) {
        self.state = RemoteDesktopState::Closed;
        self.end_reason = Some(reason.to_string());
    }

    /// Close the session through the lease-expiry terminal path.
    pub(in crate::plugins::builtin::remote_desktop) fn expire(&mut self, reason: &str) -> bool {
        if self.is_terminal() {
            return false;
        }
        self.mark_closed(reason);
        true
    }

    /// Fail the session through a transport/backend terminal path.
    pub(in crate::plugins::builtin::remote_desktop) fn fail(&mut self, reason: &str) -> bool {
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
}
