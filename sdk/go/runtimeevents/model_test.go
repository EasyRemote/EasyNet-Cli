package runtimeevents

import "testing"

func TestValidatePageStateRejectsIncoherentTerminalFlag(t *testing.T) {
	if err := ValidatePageState(StreamLive, true); err == nil {
		t.Fatal("ValidatePageState accepted live terminal page")
	}
	if err := ValidatePageState(StreamTerminal, false); err == nil {
		t.Fatal("ValidatePageState accepted terminal non-terminal page")
	}
}
