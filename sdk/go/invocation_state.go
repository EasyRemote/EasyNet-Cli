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

// ParseInvocationLifecycleState decodes the finite canonical carrier vocabulary.
// It rejects case folding, whitespace trimming, punctuation folding, numeric
// values, and unknown states.
func ParseInvocationLifecycleState(value string) (InvocationLifecycleState, error) {
	switch value {
	case "Unspecified":
		return InvocationLifecycleUnspecified, nil
	case "Accepted":
		return InvocationLifecycleAccepted, nil
	case "Admitted":
		return InvocationLifecycleAdmitted, nil
	case "Dispatched":
		return InvocationLifecycleDispatched, nil
	case "Running":
		return InvocationLifecycleRunning, nil
	case "Completed":
		return InvocationLifecycleCompleted, nil
	case "Failed":
		return InvocationLifecycleFailed, nil
	case "TimedOut":
		return InvocationLifecycleTimedOut, nil
	case "Cancelled":
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
