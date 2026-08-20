// Package runtimeevents defines the provider-neutral runtime event lifecycle.
package runtimeevents

import (
	"fmt"
)

type StreamState string

const (
	StreamLive     StreamState = "Live"
	StreamTerminal StreamState = "Terminal"
	StreamFailed   StreamState = "Failed"
)

// TransitionStream enforces the shared event-feed lifecycle. Terminal and
// failed states are absorbing; a live feed may only remain live or terminate.
func TransitionStream(current, next StreamState) error {
	if current == "" {
		current = StreamLive
	}
	switch current {
	case StreamLive:
		if next == StreamLive || next == StreamTerminal || next == StreamFailed {
			return nil
		}
	case StreamTerminal, StreamFailed:
		if next == current {
			return nil
		}
	}
	return fmt.Errorf("runtime event stream cannot transition from %q to %q", current, next)
}

func ValidatePageState(state StreamState, terminal bool) error {
	if state == "" {
		return fmt.Errorf("runtime event stream state is required")
	}
	wantTerminal := state == StreamTerminal || state == StreamFailed
	if terminal != wantTerminal {
		return fmt.Errorf("runtime event terminal flag does not match state %q", state)
	}
	return TransitionStream(StreamLive, state)
}
