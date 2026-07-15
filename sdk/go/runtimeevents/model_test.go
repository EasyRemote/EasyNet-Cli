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

func TestRouteCatalogBuildsTokenAndSequenceCursorProjections(t *testing.T) {
	catalog, err := NewRouteCatalog([]Route{
		{
			Topic:            "device",
			Ability:          "events.device.subscribe",
			StreamArgument:   "stream",
			AbilityArgument:  "daemon_ability",
			CursorArgument:   "resume_cursor",
			CursorProjection: CursorProjectionToken,
		},
		{
			Topic:            "session",
			Ability:          "session.attach",
			CursorArgument:   "since_seq",
			CursorProjection: CursorProjectionSequence,
		},
	})
	if err != nil {
		t.Fatalf("NewRouteCatalog: %v", err)
	}

	device, err := catalog.Build(Subscription{
		Topic:               "device",
		Parameters:          map[string]any{"device_ura": "easynet:///r/example/device/laptop"},
		HeartbeatIntervalMS: 30000,
		Metadata:            map[string]any{"request_id": "device:example"},
		ResumeCursor:        &ResumeCursor{Topic: "device", Sequence: 42},
	})
	if err != nil {
		t.Fatalf("Build device: %v", err)
	}
	if device.Ability != "events.device.subscribe" ||
		device.Arguments["stream"] != "device" ||
		device.Arguments["daemon_ability"] != "events.device.subscribe" ||
		device.Arguments["resume_cursor"] != "device:42" ||
		device.Metadata["system_ability"] != "events.device.subscribe" {
		t.Fatalf("unexpected device projection: %#v", device)
	}

	session, err := catalog.Build(Subscription{
		Topic:        "session",
		Parameters:   map[string]any{"session_id": "session-a"},
		ResumeCursor: &ResumeCursor{Topic: "session", Sequence: 7},
	})
	if err != nil {
		t.Fatalf("Build session: %v", err)
	}
	if session.Ability != "session.attach" ||
		session.Arguments["since_seq"] != uint64(7) ||
		session.Arguments["stream"] != nil ||
		session.Arguments["daemon_ability"] != nil {
		t.Fatalf("unexpected session projection: %#v", session)
	}
}

func TestRouteCatalogRejectsDuplicateRoutesAndCursorTopicMismatch(t *testing.T) {
	if _, err := NewRouteCatalog([]Route{
		{Topic: "device", Ability: "events.device.subscribe"},
		{Topic: "device", Ability: "events.device.subscribe"},
	}); err == nil {
		t.Fatal("NewRouteCatalog accepted duplicate topic")
	}
	catalog, err := NewRouteCatalog([]Route{{
		Topic:            "device",
		Ability:          "events.device.subscribe",
		CursorArgument:   "resume_cursor",
		CursorProjection: CursorProjectionToken,
	}})
	if err != nil {
		t.Fatalf("NewRouteCatalog: %v", err)
	}
	if _, err := catalog.Build(Subscription{
		Topic:        "device",
		ResumeCursor: &ResumeCursor{Topic: "session", Sequence: 1},
	}); err == nil {
		t.Fatal("Build accepted mismatched cursor topic")
	}
}
