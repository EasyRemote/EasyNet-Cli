//! Lifecycle policy for submitted Invocation observer handles.
//!
//! The C ABI owns process-local handles and cancellation request delivery. Axon
//! remains the authority for canonical terminal outcomes. This machine keeps
//! those two facts separate: cancellation is a request state, while only an
//! observed Axon outcome may enter a terminal phase.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum InvocationHandlePhase {
    Submitted,
    CancelRequested,
    Completed,
    Failed,
    TimedOut,
    Cancelled,
}

impl InvocationHandlePhase {
    pub(super) fn as_str(self) -> &'static str {
        match self {
            Self::Submitted => "Submitted",
            Self::CancelRequested => "CancelRequested",
            Self::Completed => "Completed",
            Self::Failed => "Failed",
            Self::TimedOut => "TimedOut",
            Self::Cancelled => "Cancelled",
        }
    }

    pub(super) fn event_kind(self) -> &'static str {
        match self {
            Self::Submitted => "submitted",
            Self::CancelRequested => "cancel_requested",
            Self::Completed => "completed",
            Self::Failed => "failed",
            Self::TimedOut => "timed_out",
            Self::Cancelled => "cancelled",
        }
    }

    pub(super) fn is_terminal(self) -> bool {
        matches!(
            self,
            Self::Completed | Self::Failed | Self::TimedOut | Self::Cancelled
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct InvocationHandleCancelOutcome {
    pub(super) request_accepted: bool,
    pub(super) deduplicated: bool,
    pub(super) dispatch_request: bool,
    pub(super) cancelled: bool,
    pub(super) state: InvocationHandlePhase,
    pub(super) terminal: bool,
    pub(super) rejection: Option<String>,
}

pub(super) struct InvocationHandleMachine {
    phase: InvocationHandlePhase,
    cancel_request_in_flight: bool,
}

impl InvocationHandleMachine {
    pub(super) fn submitted() -> Self {
        Self {
            phase: InvocationHandlePhase::Submitted,
            cancel_request_in_flight: false,
        }
    }

    pub(super) fn phase(&self) -> InvocationHandlePhase {
        self.phase
    }

    pub(super) fn request_cancel(&mut self) -> InvocationHandleCancelOutcome {
        if self.phase.is_terminal() {
            return InvocationHandleCancelOutcome {
                request_accepted: false,
                deduplicated: true,
                dispatch_request: false,
                cancelled: self.phase == InvocationHandlePhase::Cancelled,
                state: self.phase,
                terminal: true,
                rejection: None,
            };
        }

        let deduplicated = self.phase == InvocationHandlePhase::CancelRequested;
        let dispatch_request = !self.cancel_request_in_flight;
        self.cancel_request_in_flight = true;
        self.phase = InvocationHandlePhase::CancelRequested;
        InvocationHandleCancelOutcome {
            request_accepted: true,
            deduplicated,
            dispatch_request,
            cancelled: false,
            state: self.phase,
            terminal: false,
            rejection: None,
        }
    }

    /// Accept the only terminal authority this ABI recognizes: an observed
    /// canonical Axon outcome. Returns false when a prior outcome already won.
    pub(super) fn observe_terminal(&mut self, phase: InvocationHandlePhase) -> bool {
        debug_assert!(phase.is_terminal());
        if self.phase.is_terminal() {
            return false;
        }
        self.phase = phase;
        self.cancel_request_in_flight = false;
        true
    }

    /// Complete a cancellation command without claiming that the original
    /// Invocation is cancelled. The original Invocation still needs its own
    /// canonical terminal observation.
    pub(super) fn complete_cancel_request(&mut self) -> bool {
        if self.phase.is_terminal() || !self.cancel_request_in_flight {
            return false;
        }
        self.cancel_request_in_flight = false;
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cancellation_request_is_non_terminal_and_deduplicates_in_flight_delivery() {
        let mut machine = InvocationHandleMachine::submitted();

        let first = machine.request_cancel();
        assert!(first.request_accepted);
        assert!(!first.deduplicated);
        assert!(first.dispatch_request);
        assert!(!first.terminal);

        let duplicate = machine.request_cancel();
        assert!(duplicate.request_accepted);
        assert!(duplicate.deduplicated);
        assert!(!duplicate.dispatch_request);
        assert_eq!(duplicate.state, InvocationHandlePhase::CancelRequested);
    }

    #[test]
    fn completed_cancel_command_allows_retry_without_faking_terminal() {
        let mut machine = InvocationHandleMachine::submitted();
        machine.request_cancel();

        assert!(machine.complete_cancel_request());
        let retry = machine.request_cancel();
        assert!(retry.deduplicated);
        assert!(retry.dispatch_request);
        assert!(!retry.terminal);
    }

    #[test]
    fn cancel_completion_without_an_in_flight_request_is_rejected() {
        let mut machine = InvocationHandleMachine::submitted();
        assert!(!machine.complete_cancel_request());
        assert_eq!(machine.phase(), InvocationHandlePhase::Submitted);
    }

    #[test]
    fn first_canonical_terminal_observation_wins() {
        let mut machine = InvocationHandleMachine::submitted();
        machine.request_cancel();

        assert!(machine.observe_terminal(InvocationHandlePhase::Cancelled));
        assert!(!machine.observe_terminal(InvocationHandlePhase::Completed));
        assert_eq!(machine.phase(), InvocationHandlePhase::Cancelled);

        let late_cancel = machine.request_cancel();
        assert!(!late_cancel.request_accepted);
        assert!(late_cancel.cancelled);
        assert!(late_cancel.terminal);
    }
}
