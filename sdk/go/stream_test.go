package easynet

import (
	"context"
	"encoding/json"
	"strings"
	"sync"
	"testing"
	"time"
)

type memoryStreamTransport struct {
	events       []string
	closed       bool
	cancelReason string
	cancelReply  string
}

type concurrentCancelStreamTransport struct {
	recvStarted  chan struct{}
	terminal     chan []byte
	cancelReason chan string
	startOnce    sync.Once
}

type unsupportedCancelStreamTransport struct {
	memoryStreamTransport
}

type interruptedCancelStreamTransport struct {
	memoryStreamTransport
}

func (*unsupportedCancelStreamTransport) Cancel(context.Context, string) ([]byte, error) {
	return nil, &SDKError{
		Code:      ErrNotImplemented,
		Stage:     "test",
		Retry:     RetryNever,
		Retryable: false,
		Message:   "stream cancellation unsupported",
	}
}

func (*interruptedCancelStreamTransport) Cancel(context.Context, string) ([]byte, error) {
	return nil, context.DeadlineExceeded
}

func newConcurrentCancelStreamTransport() *concurrentCancelStreamTransport {
	return &concurrentCancelStreamTransport{
		recvStarted:  make(chan struct{}),
		terminal:     make(chan []byte, 1),
		cancelReason: make(chan string, 1),
	}
}

func (t *concurrentCancelStreamTransport) Recv(ctx context.Context) ([]byte, error) {
	t.startOnce.Do(func() {
		close(t.recvStarted)
	})
	select {
	case raw := <-t.terminal:
		return raw, nil
	case <-ctx.Done():
		return nil, ctx.Err()
	}
}

func (t *concurrentCancelStreamTransport) Cancel(_ context.Context, reason string) ([]byte, error) {
	t.cancelReason <- reason
	t.terminal <- []byte(`{"sequence":1,"kind":"terminal","state":"Cancelled","terminal":true,"terminal_receipt":{"receipt_id":"cancelled-1"}}`)
	return []byte(`{"stream_id":"stream-1","cancelled":false,"state":"CancelRequested","terminal":false}`), nil
}

func (*concurrentCancelStreamTransport) Close(context.Context) error {
	return nil
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
	if m.cancelReply != "" {
		return []byte(m.cancelReply), nil
	}
	return []byte(`{"stream_id":"stream-1","cancelled":false,"state":"CancelRequested","terminal":false}`), nil
}

func (m *memoryStreamTransport) Close(ctx context.Context) error {
	m.closed = true
	return nil
}

func TestStreamHandleOrdersEventsAndClosesAfterTerminal(t *testing.T) {
	transport := &memoryStreamTransport{events: []string{
		`{"sequence":1,"kind":"data","state":"Open","terminal":false,"payload_content_type":"application/json","payload_json":{"step":1},"elapsed_ms":11}`,
		`{"sequence":2,"kind":"terminal","state":"Completed","terminal":true,"payload_content_type":"application/json","payload_json":{"ok":true}}`,
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
	if first.ElapsedMS() != 11 {
		t.Fatalf("stream elapsed time = %d, want 11", first.ElapsedMS())
	}
	terminal, err := stream.Next(context.Background())
	if err != nil {
		t.Fatalf("Next terminal: %v", err)
	}
	if !terminal.Terminal() || stream.State() != StreamTerminalFrameSeen {
		t.Fatalf("terminal not recorded: %#v state=%s", terminal, stream.State())
	}
	terminalEvent, err := stream.TerminalEvent()
	if err != nil {
		t.Fatalf("TerminalEvent: %v", err)
	}
	if terminalEvent.StreamID() != "stream-1" || terminalEvent.EventType() != "terminal" || terminalEvent.Seq() != 2 {
		t.Fatalf("unexpected terminal event projection: %#v", terminalEvent)
	}
	if string(terminalEvent.PayloadJSON()) != `{"ok":true}` {
		t.Fatalf("terminal payload = %s", terminalEvent.PayloadJSON())
	}
	if err := stream.Close(context.Background()); err != nil {
		t.Fatalf("Close: %v", err)
	}
	if !transport.closed || stream.State() != StreamClosed {
		t.Fatalf("stream not closed: transport=%v state=%s", transport.closed, stream.State())
	}
	if stream.RuntimeState() != StreamTerminalFrameSeen {
		t.Fatalf("runtime state = %s, want TerminalFrameSeen after local close", stream.RuntimeState())
	}
}

func TestStreamEventRejectsLegacyContentTypeAlias(t *testing.T) {
	_, err := NewStreamEventFromJSON([]byte(`{"sequence":1,"kind":"data","content_type":"application/json"}`))
	if err == nil || !strings.Contains(err.Error(), "stream event contains noncanonical field content_type") {
		t.Fatalf("NewStreamEventFromJSON accepted legacy content_type alias: %v", err)
	}
}

func TestStreamProjectionsRejectProductStateCode(t *testing.T) {
	if _, err := NewStreamHandleFromJSON(&memoryStreamTransport{}, []byte(`{"stream_id":"stream-1","state":"Open","max_buffered_events":4,"state_code":"S200"}`)); err == nil || !strings.Contains(err.Error(), "stream open contains noncanonical field state_code") {
		t.Fatalf("NewStreamHandleFromJSON accepted product state_code: %v", err)
	}
	if _, err := NewStreamEventFromJSON([]byte(`{"sequence":1,"kind":"data","state":"Open","terminal":false,"state_code":"S200"}`)); err == nil || !strings.Contains(err.Error(), "stream event contains noncanonical field state_code") {
		t.Fatalf("NewStreamEventFromJSON accepted product state_code: %v", err)
	}
	if _, err := NewStreamCancelFromJSON([]byte(`{"stream_id":"stream-1","cancelled":false,"state":"CancelRequested","terminal":false,"state_code":"S200"}`)); err == nil || !strings.Contains(err.Error(), "stream cancel contains noncanonical field state_code") {
		t.Fatalf("NewStreamCancelFromJSON accepted product state_code: %v", err)
	}
}

func TestStreamEventRejectsLegacyChunkKind(t *testing.T) {
	_, err := NewStreamEventFromJSON([]byte(`{"sequence":1,"kind":"chunk","state":"Open","terminal":false}`))
	if err == nil {
		t.Fatalf("NewStreamEventFromJSON accepted legacy chunk kind")
	}
	if !IsCode(err, ErrInvalidArgument) {
		t.Fatalf("error = %v, want %s", err, ErrInvalidArgument)
	}
}

func TestStreamTerminalEventProjectsTerminalReceipt(t *testing.T) {
	transport := &memoryStreamTransport{events: []string{
		`{"sequence":1,"kind":"terminal","state":"Completed","terminal":true,"payload_content_type":"application/json","payload_json":{"ok":true},"terminal_receipt":{"receipt_ura":"easynet:///r/example/resource/agent.alice.sdk/invocation/r1/receipt"}}`,
	}}
	stream, err := NewStreamHandleFromJSON(transport, []byte(`{"stream_id":"stream-1","state":"Open","max_buffered_events":4}`))
	if err != nil {
		t.Fatalf("NewStreamHandleFromJSON: %v", err)
	}
	if _, err := stream.Next(context.Background()); err != nil {
		t.Fatalf("Next terminal: %v", err)
	}
	terminal, err := stream.TerminalEvent()
	if err != nil {
		t.Fatalf("TerminalEvent: %v", err)
	}
	if string(terminal.TerminalReceiptJSON()) != `{"receipt_ura":"easynet:///r/example/resource/agent.alice.sdk/invocation/r1/receipt"}` {
		t.Fatalf("terminal receipt = %s", terminal.TerminalReceiptJSON())
	}
	raw, err := json.Marshal(terminal)
	if err != nil {
		t.Fatalf("marshal terminal: %v", err)
	}
	var decoded map[string]any
	if err := json.Unmarshal(raw, &decoded); err != nil {
		t.Fatalf("decode terminal: %v", err)
	}
	if decoded["event_type"] != "terminal" || decoded["stream_id"] != "stream-1" || decoded["seq"].(float64) != 1 {
		t.Fatalf("unexpected terminal JSON: %s", raw)
	}
	if _, ok := decoded["receipt"]; ok {
		t.Fatalf("legacy receipt field serialized: %s", raw)
	}
	if _, ok := decoded["terminal_receipt"]; !ok {
		t.Fatalf("terminal receipt omitted: %s", raw)
	}
}

func TestStreamHandleRejectsDuplicateTerminal(t *testing.T) {
	transport := &memoryStreamTransport{events: []string{
		`{"sequence":1,"kind":"terminal","state":"Completed","terminal":true}`,
		`{"sequence":2,"kind":"terminal","state":"Completed","terminal":true}`,
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

func TestStreamTransportTerminalFailsHandleWithoutRuntimeTerminal(t *testing.T) {
	transport := &memoryStreamTransport{events: []string{
		`{"sequence":1,"kind":"error","state":"Failed","terminal":false,"transport_terminal":true,"error":{"code":"ROUTE_UNAVAILABLE"}}`,
	}}
	stream, err := NewStreamHandleFromJSON(transport, []byte(`{"stream_id":"stream-transport","state":"Open","max_buffered_events":4}`))
	if err != nil {
		t.Fatalf("NewStreamHandleFromJSON: %v", err)
	}

	event, err := stream.Next(context.Background())
	if err != nil {
		t.Fatalf("Next transport terminal: %v", err)
	}
	if event.Terminal() || !event.TransportTerminal() {
		t.Fatalf("transport terminal flags = terminal:%v transport:%v", event.Terminal(), event.TransportTerminal())
	}
	if stream.State() != StreamFailed {
		t.Fatalf("state = %s, want Failed", stream.State())
	}
	if _, err := stream.TerminalEvent(); err == nil {
		t.Fatalf("TerminalEvent succeeded for transport terminal")
	}
}

func TestStreamHandleCancelIsNonTerminalRequest(t *testing.T) {
	transport := &memoryStreamTransport{}
	stream, err := NewStreamHandleFromJSON(transport, []byte(`{"stream_id":"stream-1","state":"Open","max_buffered_events":4}`))
	if err != nil {
		t.Fatalf("NewStreamHandleFromJSON: %v", err)
	}

	cancel, err := stream.Cancel(context.Background(), "client stop")
	if err != nil {
		t.Fatalf("Cancel: %v", err)
	}
	if cancel.Cancelled() || cancel.Terminal() || cancel.State() != StreamCancelRequested || stream.State() != StreamCancelRequested {
		t.Fatalf("unexpected cancel: %#v state=%s", cancel, stream.State())
	}
	if transport.cancelReason != "client stop" {
		t.Fatalf("reason = %q", transport.cancelReason)
	}
}

func TestStreamHandleCancelWhileReceivingWaitsForCanonicalTerminal(t *testing.T) {
	transport := newConcurrentCancelStreamTransport()
	stream, err := NewStreamHandleFromJSON(transport, []byte(`{"stream_id":"stream-1","state":"Open","max_buffered_events":4}`))
	if err != nil {
		t.Fatalf("NewStreamHandleFromJSON: %v", err)
	}

	nextResult := make(chan StreamEvent, 1)
	nextErr := make(chan error, 1)
	go func() {
		event, nextError := stream.Next(context.Background())
		nextResult <- event
		nextErr <- nextError
	}()

	select {
	case <-transport.recvStarted:
	case <-time.After(time.Second):
		t.Fatal("stream receive did not start")
	}

	cancel, err := stream.Cancel(context.Background(), "client disconnected")
	if err != nil {
		t.Fatalf("Cancel: %v", err)
	}
	if cancel.State() != StreamCancelRequested || cancel.Terminal() || cancel.Cancelled() {
		t.Fatalf("cancel request = %#v", cancel)
	}

	select {
	case event := <-nextResult:
		if err := <-nextErr; err != nil {
			t.Fatalf("Next terminal: %v", err)
		}
		if !event.Terminal() {
			t.Fatalf("event = %#v, want canonical terminal", event)
		}
	case <-time.After(time.Second):
		t.Fatal("stream did not deliver the canonical terminal")
	}
	if stream.State() != StreamTerminalFrameSeen {
		t.Fatalf("state = %s, want %s", stream.State(), StreamTerminalFrameSeen)
	}
	terminal, err := stream.TerminalEvent()
	if err != nil {
		t.Fatalf("TerminalEvent: %v", err)
	}
	if got := string(terminal.TerminalReceiptJSON()); got != `{"receipt_id":"cancelled-1"}` {
		t.Fatalf("terminal receipt = %s", got)
	}
	if got := <-transport.cancelReason; got != "client disconnected" {
		t.Fatalf("cancel reason = %q", got)
	}
}

func TestStreamHandleUnsupportedCancelPreservesOpenState(t *testing.T) {
	transport := &unsupportedCancelStreamTransport{}
	stream, err := NewStreamHandleFromJSON(transport, []byte(`{"stream_id":"stream-1","state":"Open","max_buffered_events":4}`))
	if err != nil {
		t.Fatalf("NewStreamHandleFromJSON: %v", err)
	}

	if _, err := stream.Cancel(context.Background(), "client stop"); !IsCode(err, ErrNotImplemented) {
		t.Fatalf("Cancel error = %v, want %s", err, ErrNotImplemented)
	}
	if stream.State() != StreamOpen {
		t.Fatalf("state = %s, want %s", stream.State(), StreamOpen)
	}
	if err := stream.Close(context.Background()); err != nil {
		t.Fatalf("Close after unsupported cancellation: %v", err)
	}
	if stream.State() != StreamClosed || stream.RuntimeState() != StreamOpen {
		t.Fatalf(
			"local close changed canonical state: state=%s runtime_state=%s",
			stream.State(),
			stream.RuntimeState(),
		)
	}
}

func TestStreamHandleInterruptedCancelPreservesOpenState(t *testing.T) {
	stream, err := NewStreamHandleFromJSON(
		&interruptedCancelStreamTransport{},
		[]byte(`{"stream_id":"stream-1","state":"Open","max_buffered_events":4}`),
	)
	if err != nil {
		t.Fatalf("NewStreamHandleFromJSON: %v", err)
	}

	if _, err := stream.Cancel(context.Background(), "client stop"); err == nil {
		t.Fatal("Cancel accepted a locally interrupted request")
	}
	if stream.State() != StreamOpen || stream.RuntimeState() != StreamOpen {
		t.Fatalf(
			"local cancellation timeout changed runtime state: state=%s runtime_state=%s",
			stream.State(),
			stream.RuntimeState(),
		)
	}
}

func TestStreamHandleRejectsSecondConcurrentReceiver(t *testing.T) {
	transport := newConcurrentCancelStreamTransport()
	stream, err := NewStreamHandleFromJSON(transport, []byte(`{"stream_id":"stream-1","state":"Open","max_buffered_events":4}`))
	if err != nil {
		t.Fatalf("NewStreamHandleFromJSON: %v", err)
	}

	firstResult := make(chan StreamEvent, 1)
	firstErr := make(chan error, 1)
	go func() {
		event, nextErr := stream.Next(context.Background())
		firstResult <- event
		firstErr <- nextErr
	}()

	select {
	case <-transport.recvStarted:
	case <-time.After(time.Second):
		t.Fatal("first stream receive did not start")
	}

	if _, err := stream.Next(context.Background()); err == nil {
		t.Fatal("second concurrent stream receiver was accepted")
	}
	transport.terminal <- []byte(`{"sequence":1,"kind":"terminal","state":"Completed","terminal":true,"terminal_receipt":{"receipt_id":"completed-1"}}`)

	select {
	case event := <-firstResult:
		if err := <-firstErr; err != nil {
			t.Fatalf("first receiver terminal: %v", err)
		}
		if !event.Terminal() {
			t.Fatalf("first receiver event = %#v, want canonical terminal", event)
		}
	case <-time.After(time.Second):
		t.Fatal("first receiver did not retain stream ownership")
	}
}

func TestStreamHandleRejectsTerminalCancelOutcome(t *testing.T) {
	transport := &memoryStreamTransport{
		cancelReply: `{"stream_id":"stream-1","cancelled":true,"state":"Cancelled","terminal":true}`,
	}
	stream, err := NewStreamHandleFromJSON(transport, []byte(`{"stream_id":"stream-1","state":"Open","max_buffered_events":4}`))
	if err != nil {
		t.Fatalf("NewStreamHandleFromJSON: %v", err)
	}

	if _, err := stream.Cancel(context.Background(), "client stop"); err == nil {
		t.Fatalf("terminal cancel outcome was accepted")
	}
	if stream.State() != StreamFailed {
		t.Fatalf("terminal cancel outcome must fail the stream facade, got %s", stream.State())
	}
}

func TestStreamHandleEnforcesBufferBound(t *testing.T) {
	transport := &memoryStreamTransport{events: []string{
		`{"sequence":1,"kind":"data","state":"Open","terminal":false}`,
		`{"sequence":2,"kind":"data","state":"Open","terminal":false}`,
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

func TestStreamEventRejectsNegativeElapsed(t *testing.T) {
	_, err := NewStreamEventFromJSON([]byte(`{"sequence":1,"kind":"data","state":"Open","terminal":false,"elapsed_ms":-1}`))
	if err == nil {
		t.Fatalf("NewStreamEventFromJSON succeeded with negative elapsed")
	}
	if !IsCode(err, ErrInvalidArgument) {
		t.Fatalf("error code = %v, want %s", err, ErrInvalidArgument)
	}
}

func TestStreamEventRejectsLegacyEventAlias(t *testing.T) {
	_, err := NewStreamEventFromJSON([]byte(`{"sequence":1,"event":"chunk","state":"Open","terminal":false}`))
	if err == nil {
		t.Fatalf("NewStreamEventFromJSON accepted legacy event alias")
	}
	if !IsCode(err, ErrInvalidArgument) {
		t.Fatalf("error code = %v, want %s", err, ErrInvalidArgument)
	}
}

func TestStreamEventPreservesTopLevelCanonicalReceipt(t *testing.T) {
	event, err := NewStreamEventFromJSON([]byte(`{
		"sequence": 9,
		"kind": "terminal",
		"state": "Completed",
		"terminal": true,
		"payload_json": {"receipt":{"receipt_id":"payload-copy"}},
		"admission_receipt": {"receipt_id":"canonical-admission"},
		"terminal_receipt": {"receipt_id":"canonical-terminal"}
	}`))
	if err != nil {
		t.Fatalf("NewStreamEventFromJSON: %v", err)
	}
	if got := string(event.AdmissionReceiptJSON()); got != `{"receipt_id":"canonical-admission"}` {
		t.Fatalf("admission receipt = %s", got)
	}
	terminal, err := NewStreamTerminalEvent("stream-9", event)
	if err != nil {
		t.Fatalf("NewStreamTerminalEvent: %v", err)
	}
	if got := string(terminal.TerminalReceiptJSON()); got != `{"receipt_id":"canonical-terminal"}` {
		t.Fatalf("terminal receipt = %s", got)
	}
}

func TestStreamEventRejectsLegacyReceiptOnlyField(t *testing.T) {
	if _, err := NewStreamEventFromJSON([]byte(`{
		"sequence": 10,
		"kind": "terminal",
		"state": "Completed",
		"terminal": true,
		"receipt": {"receipt_id":"legacy-only"}
	}`)); !IsCode(err, ErrInvalidArgument) {
		t.Fatalf("legacy receipt-only field error = %v, want %s", err, ErrInvalidArgument)
	}
}
