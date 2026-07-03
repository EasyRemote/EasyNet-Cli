package easynet

import (
	"context"
	"testing"
)

type memoryStreamTransport struct {
	events       []string
	closed       bool
	cancelReason string
}

func (m *memoryStreamTransport) Recv(ctx context.Context) ([]byte, error) {
	if len(m.events) == 0 {
		return nil, invalidRuntimePayload("no event", nil)
	}
	event := m.events[0]
	m.events = m.events[1:]
	return []byte(event), nil
}

func (m *memoryStreamTransport) Cancel(ctx context.Context, reason string) ([]byte, error) {
	m.cancelReason = reason
	return []byte(`{"stream_id":"stream-1","cancelled":true,"state":"Cancelled","terminal":true}`), nil
}

func (m *memoryStreamTransport) Close(ctx context.Context) error {
	m.closed = true
	return nil
}

func TestStreamHandleOrdersEventsAndClosesAfterTerminal(t *testing.T) {
	transport := &memoryStreamTransport{events: []string{
		`{"sequence":1,"event":"chunk","state":"Open","terminal":false,"payload_content_type":"application/json","payload_json":{"step":1}}`,
		`{"sequence":2,"event":"terminal","state":"Completed","terminal":true,"payload_content_type":"application/json","payload_json":{"ok":true}}`,
	}}
	stream, err := NewStreamHandleFromJSON(transport, []byte(`{"stream_id":"stream-1","state":"Opening","max_buffered_events":4}`))
	if err != nil {
		t.Fatalf("NewStreamHandleFromJSON: %v", err)
	}

	first, err := stream.Next(context.Background())
	if err != nil {
		t.Fatalf("Next first: %v", err)
	}
	if first.Sequence() != 1 || stream.State() != StreamOpen {
		t.Fatalf("unexpected first event/state: %#v state=%s", first, stream.State())
	}
	terminal, err := stream.Next(context.Background())
	if err != nil {
		t.Fatalf("Next terminal: %v", err)
	}
	if !terminal.Terminal() || stream.State() != StreamTerminalFrameSeen {
		t.Fatalf("terminal not recorded: %#v state=%s", terminal, stream.State())
	}
	if err := stream.Close(context.Background()); err != nil {
		t.Fatalf("Close: %v", err)
	}
	if !transport.closed || stream.State() != StreamClosed {
		t.Fatalf("stream not closed: transport=%v state=%s", transport.closed, stream.State())
	}
}

func TestStreamHandleRejectsDuplicateTerminal(t *testing.T) {
	transport := &memoryStreamTransport{events: []string{
		`{"sequence":1,"event":"terminal","state":"Completed","terminal":true}`,
		`{"sequence":2,"event":"terminal","state":"Completed","terminal":true}`,
	}}
	stream, err := NewStreamHandleFromJSON(transport, []byte(`{"stream_id":"stream-1","state":"Opening","max_buffered_events":4}`))
	if err != nil {
		t.Fatalf("NewStreamHandleFromJSON: %v", err)
	}
	if _, err := stream.Next(context.Background()); err != nil {
		t.Fatalf("Next terminal: %v", err)
	}
	_, err = stream.Next(context.Background())
	if err == nil {
		t.Fatalf("Next succeeded after terminal")
	}
	if stream.State() != StreamTerminalFrameSeen {
		t.Fatalf("state = %s, want TerminalFrameSeen", stream.State())
	}
}

func TestStreamHandleCancelsNonTerminalStream(t *testing.T) {
	transport := &memoryStreamTransport{}
	stream, err := NewStreamHandleFromJSON(transport, []byte(`{"stream_id":"stream-1","state":"Open","max_buffered_events":4}`))
	if err != nil {
		t.Fatalf("NewStreamHandleFromJSON: %v", err)
	}

	cancel, err := stream.Cancel(context.Background(), "client stop")
	if err != nil {
		t.Fatalf("Cancel: %v", err)
	}
	if !cancel.Cancelled() || !cancel.Terminal() || stream.State() != StreamCancelled {
		t.Fatalf("unexpected cancel: %#v state=%s", cancel, stream.State())
	}
	if transport.cancelReason != "client stop" {
		t.Fatalf("reason = %q", transport.cancelReason)
	}
}

func TestStreamHandleEnforcesBufferBound(t *testing.T) {
	transport := &memoryStreamTransport{events: []string{
		`{"sequence":1,"event":"chunk","state":"Open","terminal":false}`,
		`{"sequence":2,"event":"chunk","state":"Open","terminal":false}`,
	}}
	stream, err := NewStreamHandleFromJSON(transport, []byte(`{"stream_id":"stream-1","state":"Opening","max_buffered_events":1}`))
	if err != nil {
		t.Fatalf("NewStreamHandleFromJSON: %v", err)
	}
	if _, err := stream.Next(context.Background()); err != nil {
		t.Fatalf("Next first: %v", err)
	}
	_, err = stream.Next(context.Background())
	if err == nil {
		t.Fatalf("Next succeeded after buffer limit")
	}
	if stream.State() != StreamFailed {
		t.Fatalf("state = %s, want Failed", stream.State())
	}
}
