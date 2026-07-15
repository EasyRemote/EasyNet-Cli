package easynet

import (
	"context"
	"encoding/base64"
	"encoding/json"
	"testing"
)

type memoryBidiTransport struct {
	recvFrames   []string
	sentFrames   []map[string]any
	closed       bool
	cancelReason string
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
	return []byte(`{"session_id":"bidi-1","state":"Cancelled","terminal":true,"reason":"client stop"}`), nil
}

func newTestBidiSession(t *testing.T, transport *memoryBidiTransport) *BidiSession {
	t.Helper()
	session, err := NewBidiSessionFromJSON(transport, []byte(`{"session_id":"bidi-1","state":"Open","max_buffered_frames":4}`))
	if err != nil {
		t.Fatalf("NewBidiSessionFromJSON: %v", err)
	}
	return session
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

func TestBidiRemoteCloseThenLocalCloseSendReachesTerminal(t *testing.T) {
	transport := &memoryBidiTransport{recvFrames: []string{
		`{"sequence":1,"kind":"remote_close_send","stream_id":1}`,
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
	if outcome.State() != BidiTerminal || !outcome.Terminal() || session.State() != BidiTerminal {
		t.Fatalf("unexpected close-send terminal: outcome=%#v state=%s", outcome, session.State())
	}
	if err := session.Close(context.Background()); err != nil {
		t.Fatalf("Close: %v", err)
	}
	if !transport.closed || session.State() != BidiClosed {
		t.Fatalf("not closed: transport=%v state=%s", transport.closed, session.State())
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

func TestBidiFrameIgnoresLegacyReceiptOnlyField(t *testing.T) {
	frame, err := NewBidiFrameFromJSON([]byte(`{"sequence":2,"kind":"terminal","stream_id":1,"terminal":true,"receipt":{"receipt_id":"legacy-only"}}`))
	if err != nil {
		t.Fatalf("NewBidiFrameFromJSON: %v", err)
	}
	if got := frame.TerminalReceiptJSON(); len(got) != 0 {
		t.Fatalf("legacy receipt-only field must not populate terminal receipt: %s", got)
	}
	terminal, err := NewBidiTerminalFrame("bidi-legacy", frame)
	if err != nil {
		t.Fatalf("NewBidiTerminalFrame: %v", err)
	}
	if got := terminal.TerminalReceiptJSON(); len(got) != 0 {
		t.Fatalf("legacy receipt-only field must not project into terminal schema: %s", got)
	}
}

func TestBidiCancelIsTerminal(t *testing.T) {
	transport := &memoryBidiTransport{}
	session := newTestBidiSession(t, transport)

	outcome, err := session.Cancel(context.Background(), "client stop")
	if err != nil {
		t.Fatalf("Cancel: %v", err)
	}

	if outcome.State() != BidiCancelled || !outcome.Terminal() || transport.cancelReason != "client stop" {
		t.Fatalf("unexpected cancel: %#v reason=%q", outcome, transport.cancelReason)
	}
	if _, err := session.CloseSend(context.Background()); err == nil {
		t.Fatalf("CloseSend succeeded after cancel")
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
