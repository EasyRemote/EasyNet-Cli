package easynet

import "fmt"

// InvocationLifecycleState is the canonical runtime lifecycle state projected
// by receipts and invocation observations.
type InvocationLifecycleState string

const (
	InvocationLifecycleUnspecified InvocationLifecycleState = "UNSPECIFIED"
	InvocationLifecycleAccepted    InvocationLifecycleState = "ACCEPTED"
	InvocationLifecycleAdmitted    InvocationLifecycleState = "ADMITTED"
	InvocationLifecycleDispatched  InvocationLifecycleState = "DISPATCHED"
	InvocationLifecycleRunning     InvocationLifecycleState = "RUNNING"
	InvocationLifecycleCompleted   InvocationLifecycleState = "COMPLETED"
	InvocationLifecycleFailed      InvocationLifecycleState = "FAILED"
	InvocationLifecycleTimedOut    InvocationLifecycleState = "TIMED_OUT"
	InvocationLifecycleCancelled   InvocationLifecycleState = "CANCELLED"
)

// ParseInvocationLifecycleState decodes the finite canonical wire vocabulary.
// It rejects numeric values, punctuation folding, and unknown states.
func ParseInvocationLifecycleState(value string) (InvocationLifecycleState, error) {
	switch value {
	case "unspecified", "Unspecified", "UNSPECIFIED":
		return InvocationLifecycleUnspecified, nil
	case "accepted", "Accepted", "ACCEPTED":
		return InvocationLifecycleAccepted, nil
	case "admitted", "Admitted", "ADMITTED":
		return InvocationLifecycleAdmitted, nil
	case "dispatched", "Dispatched", "DISPATCHED":
		return InvocationLifecycleDispatched, nil
	case "running", "Running", "RUNNING":
		return InvocationLifecycleRunning, nil
	case "completed", "Completed", "COMPLETED":
		return InvocationLifecycleCompleted, nil
	case "failed", "Failed", "FAILED":
		return InvocationLifecycleFailed, nil
	case "timed_out", "TimedOut", "TIMED_OUT":
		return InvocationLifecycleTimedOut, nil
	case "cancelled", "Cancelled", "CANCELLED":
		return InvocationLifecycleCancelled, nil
	default:
		return InvocationLifecycleUnspecified, invalidRuntimePayload(
			fmt.Sprintf("unknown invocation lifecycle state %q", value),
			nil,
		)
	}
}

// IsTerminal reports whether this state closes the canonical invocation.
func (s InvocationLifecycleState) IsTerminal() bool {
	switch s {
	case InvocationLifecycleCompleted,
		InvocationLifecycleFailed,
		InvocationLifecycleTimedOut,
		InvocationLifecycleCancelled:
		return true
	default:
		return false
	}
}
