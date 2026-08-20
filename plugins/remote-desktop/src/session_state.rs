//! EasyNet CLI — remote desktop session state machine
//! ===================================================
//!
//! File: plugins/remote-desktop/src/session_state.rs
//! Description: Canonical lifecycle state and terminal outcome for one targeted
//! remote desktop session.
//!
//! Protocol Responsibility:
//! - Enforce the SPEC lifecycle from a resolved binding through one terminal
//!   closure.
//! - Preserve the existing product wire-state projection without using that
//!   projection as the domain state machine.
//!
//! Implementation Approach:
//! - Store the precise domain phase independently from the coarse public view.
//! - Admit only explicit phase transitions; terminal state is absorbing.
//!
//! Usage Contract:
//! - The session aggregate is the sole caller allowed to advance this machine.
//! - Preview and degraded projections never manufacture a domain lifecycle phase.
//!
//! Architectural Position:
//! - Remote-desktop session aggregate component; not an Axon SDK lifecycle.

use super::contract::RemoteDesktopSessionState;

/// Product-owned public lifecycle vocabulary retained for the existing wire API.
pub(in crate::daemon::plugins::remote_desktop) type RemoteDesktopState = RemoteDesktopSessionState;

/// Canonical internal lifecycle required by the targeted-session SPEC.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::daemon::plugins::remote_desktop) enum RemoteDesktopSessionPhase {
    BindingActive,
    MediaStarting,
    MediaActive,
    InputActive,
    Suspended,
    Rebinding,
    Terminating,
    Terminated,
}

/// Explicit proof that the input plane is safe to activate for the current
/// session epoch. The state machine deliberately does not inspect JSON policy
/// or platform permissions; callers must prove those predicates before
/// providing this gate.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::daemon::plugins::remote_desktop) enum InputActivationGate {
    Ready,
}

impl RemoteDesktopSessionPhase {
    pub(in crate::daemon::plugins::remote_desktop) const fn as_str(self) -> &'static str {
        match self {
            Self::BindingActive => "binding_active",
            Self::MediaStarting => "media_starting",
            Self::MediaActive => "media_active",
            Self::InputActive => "input_active",
            Self::Suspended => "suspended",
            Self::Rebinding => "rebinding",
            Self::Terminating => "terminating",
            Self::Terminated => "terminated",
        }
    }

    const fn is_terminal(self) -> bool {
        matches!(self, Self::Terminated)
    }
}

/// Lifecycle state for one remote desktop session.
///
/// `phase` is the source of truth. `public_state` is only the stable product
/// projection used by existing clients and event envelopes.
#[derive(Debug, Clone)]
pub(in crate::daemon::plugins::remote_desktop) struct RemoteDesktopSessionStateMachine {
    phase: RemoteDesktopSessionPhase,
    public_state: RemoteDesktopState,
    end_reason: Option<String>,
}

impl RemoteDesktopSessionStateMachine {
    /// An inserted aggregate always starts with a proven active binding.
    pub(in crate::daemon::plugins::remote_desktop) fn new() -> Self {
        Self {
            phase: RemoteDesktopSessionPhase::BindingActive,
            public_state: RemoteDesktopState::Negotiating,
            end_reason: None,
        }
    }

    pub(in crate::daemon::plugins::remote_desktop) const fn phase(
        &self,
    ) -> RemoteDesktopSessionPhase {
        self.phase
    }

    pub(in crate::daemon::plugins::remote_desktop) const fn state(&self) -> RemoteDesktopState {
        self.public_state
    }

    pub(in crate::daemon::plugins::remote_desktop) const fn is_terminal(&self) -> bool {
        self.phase.is_terminal()
    }

    pub(in crate::daemon::plugins::remote_desktop) fn end_reason(&self) -> Option<&str> {
        self.end_reason.as_deref()
    }

    /// Enter media startup/negotiation from a committed binding or a new media
    /// generation. Restarting media always leaves `InputActive` until the input
    /// plane proves readiness again for the new epoch.
    pub(in crate::daemon::plugins::remote_desktop) fn start_media(&mut self) -> bool {
        if self.is_terminal()
            || !matches!(
                self.phase,
                RemoteDesktopSessionPhase::BindingActive
                    | RemoteDesktopSessionPhase::MediaStarting
                    | RemoteDesktopSessionPhase::MediaActive
                    | RemoteDesktopSessionPhase::InputActive
            )
        {
            return false;
        }
        self.set_active(
            RemoteDesktopSessionPhase::MediaStarting,
            RemoteDesktopState::Negotiating,
        )
    }

    /// Commit an active production media source. The caller may subsequently
    /// promote to `InputActive` only after input policy and target validity pass.
    pub(in crate::daemon::plugins::remote_desktop) fn activate_media(&mut self) -> bool {
        if self.is_terminal()
            || !matches!(
                self.phase,
                RemoteDesktopSessionPhase::MediaStarting
                    | RemoteDesktopSessionPhase::MediaActive
                    | RemoteDesktopSessionPhase::InputActive
            )
        {
            return false;
        }
        self.set_active(
            RemoteDesktopSessionPhase::MediaActive,
            RemoteDesktopState::Connected,
        )
    }

    /// Commit a live sender while preserving the existing public distinction
    /// between device-sending and client-presenting media.
    pub(in crate::daemon::plugins::remote_desktop) fn activate_media_awaiting_client(
        &mut self,
    ) -> bool {
        if self.is_terminal()
            || !matches!(
                self.phase,
                RemoteDesktopSessionPhase::MediaStarting
                    | RemoteDesktopSessionPhase::MediaActive
                    | RemoteDesktopSessionPhase::InputActive
            )
        {
            return false;
        }
        self.set_active(
            RemoteDesktopSessionPhase::MediaActive,
            RemoteDesktopState::Negotiating,
        )
    }

    pub(in crate::daemon::plugins::remote_desktop) fn activate_input(
        &mut self,
        gate: InputActivationGate,
    ) -> bool {
        let InputActivationGate::Ready = gate;
        if self.phase != RemoteDesktopSessionPhase::MediaActive || self.is_terminal() {
            return false;
        }
        self.set_active(
            RemoteDesktopSessionPhase::InputActive,
            RemoteDesktopState::Connected,
        )
    }

    /// Leave `InputActive` when the committed target snapshot no longer proves
    /// that input is directed at the selected target. Media may continue, but
    /// input must be re-proven before re-entering `InputActive`.
    pub(in crate::daemon::plugins::remote_desktop) fn deactivate_input_for_target_block(
        &mut self,
    ) -> bool {
        if self.phase != RemoteDesktopSessionPhase::InputActive || self.is_terminal() {
            return false;
        }
        self.set_active(
            RemoteDesktopSessionPhase::MediaActive,
            RemoteDesktopState::Connected,
        )
    }

    /// Diagnostic preview changes only the coarse public projection. It is not a
    /// production media lifecycle transition.
    pub(in crate::daemon::plugins::remote_desktop) fn project_preview_connected(&mut self) -> bool {
        if self.is_terminal()
            || !matches!(
                self.phase,
                RemoteDesktopSessionPhase::BindingActive | RemoteDesktopSessionPhase::MediaStarting
            )
        {
            return false;
        }
        self.set_public_state(RemoteDesktopState::ConnectedPreview)
    }

    pub(in crate::daemon::plugins::remote_desktop) fn project_waiting_for_media(&mut self) -> bool {
        if self.is_terminal()
            || !matches!(
                self.phase,
                RemoteDesktopSessionPhase::BindingActive
                    | RemoteDesktopSessionPhase::MediaStarting
                    | RemoteDesktopSessionPhase::MediaActive
            )
        {
            return false;
        }
        self.set_public_state(RemoteDesktopState::Negotiating)
    }

    /// Degradation is a health projection; the precise active phase remains
    /// intact so recovery cannot bypass phase guards.
    pub(in crate::daemon::plugins::remote_desktop) fn project_degraded(&mut self) -> bool {
        if self.is_terminal()
            || matches!(
                self.phase,
                RemoteDesktopSessionPhase::Suspended
                    | RemoteDesktopSessionPhase::Rebinding
                    | RemoteDesktopSessionPhase::Terminating
            )
        {
            return false;
        }
        self.set_public_state(RemoteDesktopState::Degraded)
    }

    pub(in crate::daemon::plugins::remote_desktop) fn suspend(&mut self) -> bool {
        if self.is_terminal()
            || matches!(
                self.phase,
                RemoteDesktopSessionPhase::Suspended | RemoteDesktopSessionPhase::Terminating
            )
        {
            return false;
        }
        self.set_active(
            RemoteDesktopSessionPhase::Suspended,
            RemoteDesktopState::Suspended,
        )
    }

    pub(in crate::daemon::plugins::remote_desktop) fn begin_rebinding(&mut self) -> bool {
        if self.is_terminal()
            || !matches!(
                self.phase,
                RemoteDesktopSessionPhase::MediaStarting
                    | RemoteDesktopSessionPhase::MediaActive
                    | RemoteDesktopSessionPhase::InputActive
                    | RemoteDesktopSessionPhase::Suspended
            )
        {
            return false;
        }
        self.set_active(
            RemoteDesktopSessionPhase::Rebinding,
            RemoteDesktopState::Suspended,
        )
    }

    #[cfg_attr(not(target_os = "macos"), allow(dead_code))]
    pub(in crate::daemon::plugins::remote_desktop) fn complete_rebinding(&mut self) -> bool {
        if self.phase != RemoteDesktopSessionPhase::Rebinding || self.is_terminal() {
            return false;
        }
        self.set_active(
            RemoteDesktopSessionPhase::MediaActive,
            RemoteDesktopState::Connected,
        )
    }

    pub(in crate::daemon::plugins::remote_desktop) fn reject_rebinding(&mut self) -> bool {
        if self.phase != RemoteDesktopSessionPhase::Rebinding || self.is_terminal() {
            return false;
        }
        self.set_active(
            RemoteDesktopSessionPhase::Suspended,
            RemoteDesktopState::Suspended,
        )
    }

    pub(in crate::daemon::plugins::remote_desktop) fn begin_termination(&mut self) -> bool {
        if self.is_terminal() || self.phase == RemoteDesktopSessionPhase::Terminating {
            return false;
        }
        self.set_active(
            RemoteDesktopSessionPhase::Terminating,
            RemoteDesktopState::Closing,
        )
    }

    pub(in crate::daemon::plugins::remote_desktop) fn terminate_closed(&mut self, reason: &str) {
        assert_eq!(
            self.phase,
            RemoteDesktopSessionPhase::Terminating,
            "remote desktop close must pass through Terminating"
        );
        self.phase = RemoteDesktopSessionPhase::Terminated;
        self.public_state = RemoteDesktopState::Closed;
        self.end_reason = Some(reason.to_string());
    }

    pub(in crate::daemon::plugins::remote_desktop) fn expire(&mut self, reason: &str) -> bool {
        if !self.begin_termination() {
            return false;
        }
        self.terminate_closed(reason);
        true
    }

    pub(in crate::daemon::plugins::remote_desktop) fn fail(&mut self, reason: &str) -> bool {
        if self.is_terminal() {
            return false;
        }
        self.phase = RemoteDesktopSessionPhase::Terminated;
        self.public_state = RemoteDesktopState::Failed;
        self.end_reason = Some(reason.to_string());
        true
    }

    fn set_active(
        &mut self,
        phase: RemoteDesktopSessionPhase,
        public_state: RemoteDesktopState,
    ) -> bool {
        let changed = self.phase != phase || self.public_state != public_state;
        self.phase = phase;
        self.public_state = public_state;
        changed
    }

    fn set_public_state(&mut self, state: RemoteDesktopState) -> bool {
        if self.public_state == state {
            return false;
        }
        self.public_state = state;
        true
    }
}

#[cfg(test)]
mod tests {
    use super::{
        InputActivationGate, RemoteDesktopSessionPhase, RemoteDesktopSessionStateMachine,
        RemoteDesktopState,
    };

    #[test]
    fn inserted_session_starts_binding_active_with_compatible_projection() {
        let state = RemoteDesktopSessionStateMachine::new();
        assert_eq!(state.phase(), RemoteDesktopSessionPhase::BindingActive);
        assert_eq!(state.state(), RemoteDesktopState::Negotiating);
    }

    #[test]
    fn production_lifecycle_follows_binding_media_input_and_terminal_phases() {
        let mut state = RemoteDesktopSessionStateMachine::new();

        assert!(state.start_media());
        assert_eq!(state.phase(), RemoteDesktopSessionPhase::MediaStarting);
        assert!(state.activate_media());
        assert_eq!(state.phase(), RemoteDesktopSessionPhase::MediaActive);
        assert!(state.activate_input(InputActivationGate::Ready));
        assert_eq!(state.phase(), RemoteDesktopSessionPhase::InputActive);
        assert!(state.begin_termination());
        assert_eq!(state.phase(), RemoteDesktopSessionPhase::Terminating);
        state.terminate_closed("caller_ended");

        assert_eq!(state.phase(), RemoteDesktopSessionPhase::Terminated);
        assert_eq!(state.state(), RemoteDesktopState::Closed);
        assert_eq!(state.end_reason(), Some("caller_ended"));
    }

    #[test]
    fn terminal_reason_is_absorbing() {
        let mut state = RemoteDesktopSessionStateMachine::new();

        assert!(state.fail("webrtc_failed"));
        assert!(!state.expire("lease_expired"));
        assert!(!state.start_media());

        assert_eq!(state.phase(), RemoteDesktopSessionPhase::Terminated);
        assert_eq!(state.state(), RemoteDesktopState::Failed);
        assert_eq!(state.end_reason(), Some("webrtc_failed"));
    }

    #[test]
    fn diagnostic_preview_does_not_become_media_active() {
        let mut state = RemoteDesktopSessionStateMachine::new();
        assert!(state.project_preview_connected());
        assert_eq!(state.phase(), RemoteDesktopSessionPhase::BindingActive);
        assert_eq!(state.state(), RemoteDesktopState::ConnectedPreview);
    }

    #[test]
    fn target_loss_uses_explicit_suspended_and_rebinding_phases() {
        let mut state = RemoteDesktopSessionStateMachine::new();

        assert!(state.start_media());
        assert!(state.activate_media());
        assert!(state.suspend());
        assert_eq!(state.phase(), RemoteDesktopSessionPhase::Suspended);
        assert!(!state.start_media());
        assert!(state.begin_rebinding());
        assert_eq!(state.phase(), RemoteDesktopSessionPhase::Rebinding);
        assert!(state.reject_rebinding());
        assert_eq!(state.phase(), RemoteDesktopSessionPhase::Suspended);
        assert!(state.begin_termination());
        state.terminate_closed("caller_ended");
        assert_eq!(state.phase(), RemoteDesktopSessionPhase::Terminated);
    }
}
