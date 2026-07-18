package easynet

import "testing"

func TestInvocationLifecycleStateParsesFiniteWireVocabulary(t *testing.T) {
	tests := map[string]InvocationLifecycleState{
		"accepted":  InvocationLifecycleAccepted,
		"Admitted":  InvocationLifecycleAdmitted,
		"RUNNING":   InvocationLifecycleRunning,
		"TimedOut":  InvocationLifecycleTimedOut,
		"cancelled": InvocationLifecycleCancelled,
	}
	for raw, expected := range tests {
		actual, err := ParseInvocationLifecycleState(raw)
		if err != nil {
			t.Fatalf("ParseInvocationLifecycleState(%q): %v", raw, err)
		}
		if actual != expected {
			t.Fatalf("ParseInvocationLifecycleState(%q) = %q, want %q", raw, actual, expected)
		}
	}
}

func TestInvocationLifecycleStateRejectsInventedNormalization(t *testing.T) {
	for _, raw := range []string{"", "8", " timed_out ", "timed-out", "invented"} {
		if _, err := ParseInvocationLifecycleState(raw); !IsCode(err, ErrInvalidArgument) {
			t.Fatalf("ParseInvocationLifecycleState(%q) = %v, want %s", raw, err, ErrInvalidArgument)
		}
	}
}

func TestInvocationLifecycleStateTerminality(t *testing.T) {
	if !InvocationLifecycleCompleted.IsTerminal() ||
		!InvocationLifecycleFailed.IsTerminal() ||
		!InvocationLifecycleTimedOut.IsTerminal() ||
		!InvocationLifecycleCancelled.IsTerminal() {
		t.Fatal("canonical terminal state was not terminal")
	}
	if InvocationLifecycleUnspecified.IsTerminal() ||
		InvocationLifecycleAccepted.IsTerminal() ||
		InvocationLifecycleRunning.IsTerminal() {
		t.Fatal("canonical non-terminal state was terminal")
	}
}
