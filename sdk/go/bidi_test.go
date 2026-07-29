package easynet

import (
	"context"
	"encoding/base64"
	"encoding/json"
	"strings"
	"testing"
)

type memoryBidiTransport struct {
	recvFrames   []string
	sentFrames   []map[string]any
	closed       bool
	cancelReason string
	cancelReply  string
}

type unsupportedCancelBidiTransport struct {
	memoryBidiTransport
}

type interruptedSendBidiTransport struct {
	memoryBidiTransport
}

type interruptedCloseSendBidiTransport struct {
	memoryBidiTransport
}

type interruptedCancelBidiTransport struct {
	memoryBidiTransport
}

type concurrentCancelBidiTransport struct {
	memoryBidiTransport
	recvStarted chan struct{}
	releaseRecv chan struct{}
}

func (*unsupportedCancelBidiTransport) Cancel(context.Context, string) ([]byte, error) {
	return nil, &SDKError{
		Code:      ErrNotImplemented,
		Stage:     "test",
		Retry:     RetryNever,
		Retryable: false,
		Message:   "bidi cancellation unsupported",
	}
}

func (*interruptedSendBidiTransport) Send(context.Context, []byte) ([]byte, error) {
	return nil, context.DeadlineExceeded
}

func (*interruptedCloseSendBidiTransport) CloseSend(context.Context) ([]byte, error) {
	return nil, &SDKError{
		Code:      ErrTimeout,
		Stage:     "test",
		Retry:     RetrySafe,
		Retryable: true,
		Message:   "close-send deadline elapsed",
	}
}

func (*interruptedCancelBidiTransport) Cancel(context.Context, string) ([]byte, error) {
	return nil, &SDKError{
		Code:      ErrCancelled,
		Stage:     "test",
		Retry:     RetryNever,
		Retryable: false,
		Message:   "cancel request interrupted",
	}
}

func newConcurrentCancelBidiTransport() *concurrentCancelBidiTransport {
	return &concurrentCancelBidiTransport{
		recvStarted: make(chan struct{}),
		releaseRecv: make(chan struct{}),
	}
}

func (t *concurrentCancelBidiTransport) Recv(context.Context) ([]byte, error) {
	close(t.recvStarted)
	<-t.releaseRecv
	return []byte(`{"sequence":1,"kind":"terminal","stream_id":1,"terminal":true,"terminal_receipt":{"receipt_ura":"easynet:///r/example/resource/agent.alice.sdk/invocation/r1/receipt"}}`), nil
}

func (m *memoryBidiTransport) Send(ctx context.Context, frameJSON []byte) ([]byte, error) {
	var frame map[string]any
	if err := json.Unmarshal(frameJSON, &frame); err != nil {
		return nil, err
	}
	m.sentFrames = append(m.sentFrames, frame)
	return frameJSON, nil
}

func (m *memoryBidiTransport) Recv(ctx context.Context) ([]byte, error) {
	if len(m.recvFrames) == 0 {
		return nil, invalidRuntimePayload("no frame", nil)
	}
	frame := m.recvFrames[0]
	m.recvFrames = m.recvFrames[1:]
	return []byte(frame), nil
}

func (m *memoryBidiTransport) CloseSend(ctx context.Context) ([]byte, error) {
	return []byte(`{"session_id":"bidi-1","state":"HalfClosedLocal","terminal":false}`), nil
}

func (m *memoryBidiTransport) Close(ctx context.Context) error {
	m.closed = true
	return nil
}

func (m *memoryBidiTransport) Cancel(ctx context.Context, reason string) ([]byte, error) {
	m.cancelReason = reason
	if m.cancelReply != "" {
		return []byte(m.cancelReply), nil
	}
	return []byte(`{"session_id":"bidi-1","state":"CancelRequested","terminal":false,"reason":"client stop"}`), nil
}

func newTestBidiSession(t *testing.T, transport BidiTransport) *BidiSession {
	t.Helper()
	session, err := NewBidiSessionFromJSON(transport, []byte(`{"session_id":"bidi-1","state":"Open","max_buffered_frames":4}`))
	if err != nil {
		t.Fatalf("NewBidiSessionFromJSON: %v", err)
	}
	return session
}

func TestBidiLocalOperationInterruptionsPreserveOpenState(t *testing.T) {
	frame, err := NewBidiBinaryFrame(1, 1, []byte("hello"), "text/plain")
	if err != nil {
		t.Fatalf("NewBidiBinaryFrame: %v", err)
	}
	tests := map[string]struct {
		transport BidiTransport
		operation func(*BidiSession) error
	}{
		"send": {
			transport: &interruptedSendBidiTransport{},
			operation: func(session *BidiSession) error {
				_, err := session.Send(context.Background(), frame)
				return err
			},
		},
		"close-send": {
			transport: &interruptedCloseSendBidiTransport{},
			operation: func(session *BidiSession) error {
				_, err := session.CloseSend(context.Background())
				return err
			},
		},
		"cancel": {
			transport: &interruptedCancelBidiTransport{},
			operation: func(session *BidiSession) error {
				_, err := session.Cancel(context.Background(), "client stop")
				return err
			},
		},
	}
	for name, test := range tests {
		t.Run(name, func(t *testing.T) {
			session := newTestBidiSession(t, test.transport)
			if err := test.operation(session); err == nil {
				t.Fatal("operation accepted a local interruption")
			}
			if session.State() != BidiOpen || session.RuntimeState() != BidiOpen {
				t.Fatalf(
					"local interruption changed runtime state: state=%s runtime_state=%s",
					session.State(),
					session.RuntimeState(),
				)
			}
		})
	}
}

func TestBidiSessionSendsAndReceivesOrderedFrames(t *testing.T) {
	transport := &memoryBidiTransport{recvFrames: []string{
		`{"sequence":1,"kind":"data","stream_id":1,"payload_content_type":"text/plain","payload_base64":"cmVhZHk="}`,
	}}
	session := newTestBidiSession(t, transport)
	frame, err := NewBidiBinaryFrame(1, 1, []byte("hello"), "text/plain")
	if err != nil {
		t.Fatalf("NewBidiBinaryFrame: %v", err)
	}

	ack, err := session.Send(context.Background(), frame)
	if err != nil {
		t.Fatalf("Send: %v", err)
	}
	received, err := session.Receive(context.Background())
	if err != nil {
		t.Fatalf("Receive: %v", err)
	}

	if ack.Sequence() != 1 || received.Sequence() != 1 || session.State() != BidiOpen {
		t.Fatalf("unexpected session state: ack=%#v received=%#v state=%s", ack, received, session.State())
	}
	if ack.PayloadBase64() != base64.StdEncoding.EncodeToString([]byte("hello")) || ack.PayloadContentType() != "text/plain" {
		t.Fatalf("ack payload not preserved: content_type=%q payload=%q", ack.PayloadContentType(), ack.PayloadBase64())
	}
	if received.PayloadBase64() != "cmVhZHk=" || received.PayloadContentType() != "text/plain" {
		t.Fatalf("received payload not preserved: content_type=%q payload=%q", received.PayloadContentType(), received.PayloadBase64())
	}
}

func TestBidiFramePreservesFinalizationCheckpoints(t *testing.T) {
	raw := []byte(`{"sequence":9,"kind":"terminal","stream_id":1,"terminal":true,"admission_receipt":{"invocation_id":"inv-1","index":3,"authority_proof":{"proof_type":"signed"}},"terminal_receipt":{"invocation_id":"inv-1","index":8,"output_hash":"abcd"}}`)
	frame, err := NewBidiFrameFromJSON(raw)
	if err != nil {
		t.Fatalf("NewBidiFrameFromJSON: %v", err)
	}
	if string(frame.AdmissionReceiptJSON()) != `{"invocation_id":"inv-1","index":3,"authority_proof":{"proof_type":"signed"}}` {
		t.Fatalf("admission receipt = %s", frame.AdmissionReceiptJSON())
	}
	if string(frame.TerminalReceiptJSON()) != `{"invocation_id":"inv-1","index":8,"output_hash":"abcd"}` {
		t.Fatalf("terminal receipt = %s", frame.TerminalReceiptJSON())
	}
	encoded, err := json.Marshal(frame)
	if err != nil {
		t.Fatalf("marshal frame: %v", err)
	}
	var decoded map[string]json.RawMessage
	if err := json.Unmarshal(encoded, &decoded); err != nil {
		t.Fatalf("decode frame: %v", err)
	}
	if len(decoded["admission_receipt"]) == 0 || len(decoded["terminal_receipt"]) == 0 {
		t.Fatalf("finalization checkpoints omitted: %s", encoded)
	}
}

func TestBidiFrameRejectsLegacyEventAlias(t *testing.T) {
	_, err := NewBidiFrameFromJSON([]byte(`{"sequence":1,"event":"data","stream_id":1}`))
	if err == nil {
		t.Fatalf("NewBidiFrameFromJSON accepted legacy event alias")
	}
	if !IsCode(err, ErrInvalidArgument) {
		t.Fatalf("error = %v, want %s", err, ErrInvalidArgument)
	}
}

func TestBidiProjectionsRejectProductStateCode(t *testing.T) {
	if _, err := NewBidiSessionFromJSON(&memoryBidiTransport{}, []byte(`{"session_id":"bidi-1","state":"Open","max_buffered_frames":4,"state_code":"B200"}`)); err == nil || !strings.Contains(err.Error(), "bidi open contains noncanonical field state_code") {
		t.Fatalf("NewBidiSessionFromJSON accepted product state_code: %v", err)
	}
	if _, err := NewBidiFrameFromJSON([]byte(`{"sequence":1,"kind":"data","stream_id":1,"terminal":false,"state_code":"B200"}`)); err == nil || !strings.Contains(err.Error(), "bidi frame contains noncanonical field state_code") {
		t.Fatalf("NewBidiFrameFromJSON accepted product state_code: %v", err)
	}
	if _, err := NewBidiOutcomeFromJSON([]byte(`{"session_id":"bidi-1","state":"CancelRequested","terminal":false,"reason":"stop","state_code":"B200"}`)); err == nil || !strings.Contains(err.Error(), "bidi outcome contains noncanonical field state_code") {
		t.Fatalf("NewBidiOutcomeFromJSON accepted product state_code: %v", err)
	}
}

func TestBidiTransportTerminalFailsSessionWithoutRuntimeTerminal(t *testing.T) {
	transport := &memoryBidiTransport{recvFrames: []string{
		`{"sequence":1,"kind":"error","stream_id":1,"terminal":false,"transport_terminal":true,"error":{"code":"ROUTE_UNAVAILABLE"}}`,
	}}
	session := newTestBidiSession(t, transport)

	frame, err := session.Receive(context.Background())
	if err != nil {
		t.Fatalf("Receive transport terminal: %v", err)
	}
	if frame.Terminal() || !frame.TransportTerminal() {
		t.Fatalf("transport terminal flags = terminal:%v transport:%v", frame.Terminal(), frame.TransportTerminal())
	}
	if session.State() != BidiFailed {
		t.Fatalf("state = %s, want Failed", session.State())
	}
	if _, err := session.TerminalFrame(); err == nil {
		t.Fatalf("TerminalFrame succeeded for transport terminal")
	}
}

func TestBidiCloseSendDiffersFromCancel(t *testing.T) {
	transport := &memoryBidiTransport{}
	session := newTestBidiSession(t, transport)

	outcome, err := session.CloseSend(context.Background())
	if err != nil {
		t.Fatalf("CloseSend: %v", err)
	}

	if outcome.State() != BidiHalfClosedLocal || outcome.Terminal() {
		t.Fatalf("unexpected close-send outcome: %#v", outcome)
	}
	if session.State() != BidiHalfClosedLocal {
		t.Fatalf("state = %s, want HalfClosedLocal", session.State())
	}
	frame, err := NewBidiFrame(1, "data", 1)
	if err != nil {
		t.Fatalf("NewBidiFrame: %v", err)
	}
	if _, err := session.Send(context.Background(), frame); !IsCode(err, ErrCancelled) {
		t.Fatalf("Send after close-send error = %v, want %s", err, ErrCancelled)
	}
	if session.State() != BidiHalfClosedLocal {
		t.Fatalf("state after rejected send = %s, want HalfClosedLocal", session.State())
	}
}

func TestBidiRemoteAndLocalHalfCloseWaitForTerminalReceipt(t *testing.T) {
	transport := &memoryBidiTransport{recvFrames: []string{
		`{"sequence":1,"kind":"remote_close_send","stream_id":1}`,
		`{"sequence":2,"kind":"terminal","stream_id":1,"terminal":true,"terminal_receipt":{"receipt_ura":"easynet:///r/example/resource/agent.alice.sdk/invocation/r1/receipt"}}`,
	}}
	session := newTestBidiSession(t, transport)

	frame, err := session.Receive(context.Background())
	if err != nil {
		t.Fatalf("Receive: %v", err)
	}
	if frame.Kind() != "remote_close_send" || session.State() != BidiHalfClosedRemote {
		t.Fatalf("unexpected remote close: frame=%#v state=%s", frame, session.State())
	}
	outcome, err := session.CloseSend(context.Background())
	if err != nil {
		t.Fatalf("CloseSend: %v", err)
	}
	if outcome.State() != BidiHalfClosedLocal || outcome.Terminal() || session.State() != BidiHalfClosedLocal {
		t.Fatalf("half-close claimed canonical terminal: outcome=%#v state=%s", outcome, session.State())
	}
	if _, err := session.TerminalFrame(); err == nil {
		t.Fatal("half-closed session exposed terminal frame without receipt")
	}
	terminal, err := session.Receive(context.Background())
	if err != nil {
		t.Fatalf("Receive terminal receipt: %v", err)
	}
	if !terminal.Terminal() || len(terminal.TerminalReceiptJSON()) == 0 || session.State() != BidiTerminal {
		t.Fatalf("receipt-backed terminal not observed: frame=%#v state=%s", terminal, session.State())
	}
	if err := session.Close(context.Background()); err != nil {
		t.Fatalf("Close: %v", err)
	}
	if !transport.closed || session.State() != BidiClosed {
		t.Fatalf("not closed: transport=%v state=%s", transport.closed, session.State())
	}
	if session.RuntimeState() != BidiTerminal {
		t.Fatalf("runtime state = %s, want Terminal after local close", session.RuntimeState())
	}
}

func TestBidiTerminalFrameProjectsSchemaShape(t *testing.T) {
	transport := &memoryBidiTransport{recvFrames: []string{
		`{"sequence":1,"kind":"terminal","stream_id":1,"terminal":true,"payload_json":{"ok":true},"terminal_receipt":{"receipt_ura":"easynet:///r/example/resource/agent.alice.sdk/invocation/r1/receipt"}}`,
	}}
	session := newTestBidiSession(t, transport)

	if _, err := session.Receive(context.Background()); err != nil {
		t.Fatalf("Receive terminal: %v", err)
	}
	terminal, err := session.TerminalFrame()
	if err != nil {
		t.Fatalf("TerminalFrame: %v", err)
	}
	if terminal.SessionID() != "bidi-1" || terminal.FrameType() != "terminal" || terminal.Seq() != 1 {
		t.Fatalf("unexpected terminal frame projection: %#v", terminal)
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
	if decoded["frame_type"] != "terminal" || decoded["session_id"] != "bidi-1" || decoded["seq"].(float64) != 1 {
		t.Fatalf("unexpected terminal JSON: %s", raw)
	}
	if _, ok := decoded["receipt"]; ok {
		t.Fatalf("legacy receipt field serialized: %s", raw)
	}
	if _, ok := decoded["terminal_receipt"]; !ok {
		t.Fatalf("terminal receipt omitted: %s", raw)
	}
}

func TestBidiFrameRejectsLegacyReceiptOnlyField(t *testing.T) {
	if _, err := NewBidiFrameFromJSON([]byte(`{"sequence":2,"kind":"terminal","stream_id":1,"terminal":true,"receipt":{"receipt_id":"legacy-only"}}`)); !IsCode(err, ErrInvalidArgument) {
		t.Fatalf("legacy receipt-only field error = %v, want %s", err, ErrInvalidArgument)
	}
}

func TestBidiCancelIsNonTerminalRequest(t *testing.T) {
	transport := &memoryBidiTransport{}
	session := newTestBidiSession(t, transport)

	outcome, err := session.Cancel(context.Background(), "client stop")
	if err != nil {
		t.Fatalf("Cancel: %v", err)
	}

	if outcome.State() != BidiCancelRequested || outcome.Terminal() || transport.cancelReason != "client stop" {
		t.Fatalf("unexpected cancel: %#v reason=%q", outcome, transport.cancelReason)
	}
	if _, err := session.CloseSend(context.Background()); err == nil {
		t.Fatalf("CloseSend succeeded after cancel")
	}
	if err := session.Close(context.Background()); err != nil {
		t.Fatalf("Close after cancel request: %v", err)
	}
	if !transport.closed || session.State() != BidiClosed {
		t.Fatalf("bidi close after cancel did not release transport: closed=%v state=%s", transport.closed, session.State())
	}
	if session.RuntimeState() != BidiCancelRequested {
		t.Fatalf("runtime state = %s, want CancelRequested after local close", session.RuntimeState())
	}
}

func TestBidiCancelRejectsTerminalOutcome(t *testing.T) {
	transport := &memoryBidiTransport{
		cancelReply: `{"session_id":"bidi-1","state":"Cancelled","terminal":true,"reason":"client stop"}`,
	}
	session := newTestBidiSession(t, transport)

	if _, err := session.Cancel(context.Background(), "client stop"); err == nil {
		t.Fatalf("terminal cancel outcome was accepted")
	}
	if session.State() != BidiFailed {
		t.Fatalf("terminal cancel outcome must fail the bidi facade, got %s", session.State())
	}
}

func TestBidiUnsupportedCancelPreservesOpenState(t *testing.T) {
	transport := &unsupportedCancelBidiTransport{}
	session, err := NewBidiSessionFromJSON(transport, []byte(`{"session_id":"bidi-1","state":"Open","max_buffered_frames":4}`))
	if err != nil {
		t.Fatalf("NewBidiSessionFromJSON: %v", err)
	}

	if _, err := session.Cancel(context.Background(), "client stop"); !IsCode(err, ErrNotImplemented) {
		t.Fatalf("Cancel error = %v, want %s", err, ErrNotImplemented)
	}
	if session.State() != BidiOpen {
		t.Fatalf("state = %s, want %s", session.State(), BidiOpen)
	}
	if err := session.Close(context.Background()); err != nil {
		t.Fatalf("Close after unsupported cancellation: %v", err)
	}
	if !transport.closed || session.State() != BidiClosed {
		t.Fatalf("local release failed: closed=%v state=%s", transport.closed, session.State())
	}
	if session.RuntimeState() != BidiOpen {
		t.Fatalf("runtime state = %s, want Open after local close", session.RuntimeState())
	}
}

func TestBidiCancelWhileReceivingWaitsForCanonicalTerminal(t *testing.T) {
	transport := newConcurrentCancelBidiTransport()
	session, err := NewBidiSessionFromJSON(transport, []byte(`{"session_id":"bidi-1","state":"Open","max_buffered_frames":4}`))
	if err != nil {
		t.Fatalf("NewBidiSessionFromJSON: %v", err)
	}

	received := make(chan BidiFrame, 1)
	receiveErrors := make(chan error, 1)
	go func() {
		frame, receiveErr := session.Receive(context.Background())
		if receiveErr != nil {
			receiveErrors <- receiveErr
			return
		}
		received <- frame
	}()
	<-transport.recvStarted

	outcome, err := session.Cancel(context.Background(), "client stop")
	if err != nil {
		t.Fatalf("Cancel: %v", err)
	}
	if outcome.State() != BidiCancelRequested || outcome.Terminal() {
		t.Fatalf("cancel outcome = %#v", outcome)
	}
	close(transport.releaseRecv)

	select {
	case receiveErr := <-receiveErrors:
		t.Fatalf("Receive after cancel request: %v", receiveErr)
	case frame := <-received:
		if !frame.Terminal() {
			t.Fatalf("received frame is not terminal: %#v", frame)
		}
	}
	if session.State() != BidiTerminal {
		t.Fatalf("state = %s, want %s", session.State(), BidiTerminal)
	}
	if _, err := session.TerminalFrame(); err != nil {
		t.Fatalf("TerminalFrame after cancel drain: %v", err)
	}
}

func TestBidiRejectsSecondConcurrentReceiver(t *testing.T) {
	transport := newConcurrentCancelBidiTransport()
	session, err := NewBidiSessionFromJSON(transport, []byte(`{"session_id":"bidi-1","state":"Open","max_buffered_frames":4}`))
	if err != nil {
		t.Fatalf("NewBidiSessionFromJSON: %v", err)
	}

	firstDone := make(chan error, 1)
	go func() {
		_, receiveErr := session.Receive(context.Background())
		firstDone <- receiveErr
	}()
	<-transport.recvStarted

	if _, err := session.Receive(context.Background()); !IsCode(err, ErrInvalidArgument) {
		t.Fatalf("second Receive error = %v, want %s", err, ErrInvalidArgument)
	}
	close(transport.releaseRecv)
	if err := <-firstDone; err != nil {
		t.Fatalf("first Receive: %v", err)
	}
}

func TestBidiReceiveBufferBound(t *testing.T) {
	transport := &memoryBidiTransport{recvFrames: []string{
		`{"sequence":1,"kind":"data","stream_id":1}`,
		`{"sequence":2,"kind":"data","stream_id":1}`,
	}}
	session, err := NewBidiSessionFromJSON(transport, []byte(`{"session_id":"bidi-1","state":"Open","max_buffered_frames":1}`))
	if err != nil {
		t.Fatalf("NewBidiSessionFromJSON: %v", err)
	}
	if _, err := session.Receive(context.Background()); err != nil {
		t.Fatalf("Receive first: %v", err)
	}
	if _, err := session.Receive(context.Background()); err == nil {
		t.Fatalf("Receive succeeded after buffer limit")
	}
	if session.State() != BidiFailed {
		t.Fatalf("state = %s, want Failed", session.State())
	}
}
