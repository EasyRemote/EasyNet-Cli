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
