package easynet

import (
	"bytes"
	"context"
	"errors"
	"io"
	"sync"
	"testing"
)

type fakeLeaseState struct {
	mu       sync.Mutex
	data     []byte
	refs     int
	releases int
}

type fakeLeaseStorage struct {
	mu       sync.Mutex
	state    *fakeLeaseState
	released bool
}

func newFakeLease(data []byte) (*fakeLeaseStorage, *fakeLeaseState) {
	state := &fakeLeaseState{data: append([]byte(nil), data...), refs: 1}
	return &fakeLeaseStorage{state: state}, state
}

func (s *fakeLeaseStorage) Len() int {
	s.mu.Lock()
	defer s.mu.Unlock()
	if s.released {
		return 0
	}
	return len(s.state.data)
}

func (s *fakeLeaseStorage) Released() bool {
	s.mu.Lock()
	defer s.mu.Unlock()
	return s.released
}

func (s *fakeLeaseStorage) CopyBytes() ([]byte, error) {
	s.mu.Lock()
	defer s.mu.Unlock()
	if s.released {
		return nil, errors.New("released")
	}
	return append([]byte(nil), s.state.data...), nil
}

func (s *fakeLeaseStorage) WriteTo(writer io.Writer) (int64, error) {
	s.mu.Lock()
	defer s.mu.Unlock()
	if s.released {
		return 0, errors.New("released")
	}
	n, err := writer.Write(s.state.data)
	if err == nil && n != len(s.state.data) {
		err = io.ErrShortWrite
	}
	return int64(n), err
}

func (s *fakeLeaseStorage) Retain() (leasedPayloadStorage, error) {
	s.mu.Lock()
	defer s.mu.Unlock()
	if s.released {
		return nil, errors.New("released")
	}
	s.state.mu.Lock()
	s.state.refs++
	s.state.mu.Unlock()
	return &fakeLeaseStorage{state: s.state}, nil
}

func (s *fakeLeaseStorage) Release() error {
	s.mu.Lock()
	defer s.mu.Unlock()
	if s.released {
		return nil
	}
	s.released = true
	s.state.mu.Lock()
	s.state.refs--
	s.state.releases++
	s.state.mu.Unlock()
	return nil
}

func (s *fakeLeaseState) snapshot() (refs, releases int) {
	s.mu.Lock()
	defer s.mu.Unlock()
	return s.refs, s.releases
}

func TestLeasedPayloadToBytesCopiesAndReleasesExactlyOnce(t *testing.T) {
	storage, state := newFakeLease([]byte("remote-frame"))
	payload, err := newLeasedPayload(storage)
	if err != nil {
		t.Fatalf("newLeasedPayload: %v", err)
	}
	owned, err := payload.ToBytes()
	if err != nil {
		t.Fatalf("ToBytes: %v", err)
	}
	owned[0] = 'R'
	if string(state.data) != "remote-frame" {
		t.Fatalf("ToBytes returned an alias: %q", state.data)
	}
	if err := payload.Release(); err != nil {
		t.Fatalf("idempotent Release: %v", err)
	}
	if refs, releases := state.snapshot(); refs != 0 || releases != 1 {
		t.Fatalf("lease state = refs:%d releases:%d, want 0/1", refs, releases)
	}
}

func TestLeasedPayloadRetainCreatesIndependentOwner(t *testing.T) {
	storage, state := newFakeLease([]byte("frame"))
	payload, err := newLeasedPayload(storage)
	if err != nil {
		t.Fatalf("newLeasedPayload: %v", err)
	}
	retained, err := payload.Retain()
	if err != nil {
		t.Fatalf("Retain: %v", err)
	}
	if err := payload.Close(); err != nil {
		t.Fatalf("Close original: %v", err)
	}
	if refs, releases := state.snapshot(); refs != 1 || releases != 1 {
		t.Fatalf("state after original close = refs:%d releases:%d", refs, releases)
	}
	got, err := retained.ToBytes()
	if err != nil || string(got) != "frame" {
		t.Fatalf("retained ToBytes = %q, %v", got, err)
	}
	if refs, releases := state.snapshot(); refs != 0 || releases != 2 {
		t.Fatalf("final lease state = refs:%d releases:%d", refs, releases)
	}
}

type failingLeaseWriter struct{}

func (failingLeaseWriter) Write(p []byte) (int, error) {
	return len(p) / 2, errors.New("sink failed")
}

func TestLeasedPayloadWriteErrorStillReleases(t *testing.T) {
	storage, state := newFakeLease([]byte("video-frame"))
	payload, err := newLeasedPayload(storage)
	if err != nil {
		t.Fatalf("newLeasedPayload: %v", err)
	}
	written, err := payload.WriteTo(failingLeaseWriter{})
	if err == nil || written == 0 {
		t.Fatalf("WriteTo = %d, %v; want partial error", written, err)
	}
	if refs, releases := state.snapshot(); refs != 0 || releases != 1 {
		t.Fatalf("lease state = refs:%d releases:%d, want 0/1", refs, releases)
	}
}

func TestLeasedPayloadNilWriterStillReleases(t *testing.T) {
	storage, state := newFakeLease([]byte("video-frame"))
	payload, err := newLeasedPayload(storage)
	if err != nil {
		t.Fatalf("newLeasedPayload: %v", err)
	}
	if _, err := payload.WriteTo(nil); err == nil {
		t.Fatal("WriteTo(nil) succeeded")
	}
	if refs, releases := state.snapshot(); refs != 0 || releases != 1 {
		t.Fatalf("lease state = refs:%d releases:%d, want 0/1", refs, releases)
	}
}

type fakeLeasedStreamTransport struct {
	packets []leasedStreamPacket
	closed  bool
}

func (t *fakeLeasedStreamTransport) RecvLeased(context.Context) (leasedStreamPacket, error) {
	if len(t.packets) == 0 {
		return leasedStreamPacket{}, errors.New("EOF")
	}
	packet := t.packets[0]
	t.packets = t.packets[1:]
	return packet, nil
}

func (t *fakeLeasedStreamTransport) Cancel(context.Context, string) ([]byte, error) {
	return []byte(`{"stream_id":"leased","cancelled":false,"state":"CancelRequested","terminal":false}`), nil
}

func (t *fakeLeasedStreamTransport) Close(context.Context) error {
	if t.closed {
		return nil
	}
	t.closed = true
	for index := range t.packets {
		_ = t.packets[index].release()
	}
	t.packets = nil
	return nil
}

func TestLeasedStreamRejectsOutOfOrderFrameAndReleasesIt(t *testing.T) {
	firstStorage, firstState := newFakeLease([]byte("first"))
	secondStorage, secondState := newFakeLease([]byte("second"))
	transport := &fakeLeasedStreamTransport{packets: []leasedStreamPacket{
		{sequence: 2, kind: "data", state: "Running", payloadContentType: "video/h264", payload: firstStorage},
		{sequence: 1, kind: "data", state: "Running", payloadContentType: "video/h264", payload: secondStorage},
	}}
	stream, err := newLeasedStreamHandleFromJSON(transport, []byte(`{"stream_id":"leased","state":"Open"}`))
	if err != nil {
		t.Fatalf("newLeasedStreamHandleFromJSON: %v", err)
	}
	event, err := stream.Next(context.Background())
	if err != nil {
		t.Fatalf("Next first: %v", err)
	}
	if err := event.Release(); err != nil {
		t.Fatalf("release first: %v", err)
	}
	if _, err := stream.Next(context.Background()); err == nil {
		t.Fatal("out-of-order frame was accepted")
	}
	if refs, _ := firstState.snapshot(); refs != 0 {
		t.Fatalf("first lease refs = %d", refs)
	}
	if refs, releases := secondState.snapshot(); refs != 0 || releases != 1 {
		t.Fatalf("dropped lease state = refs:%d releases:%d", refs, releases)
	}
}

func TestLeasedPayloadWriteToDoesNotRequireMaterializedBytes(t *testing.T) {
	storage, _ := newFakeLease([]byte("encoded-frame"))
	payload, err := newLeasedPayload(storage)
	if err != nil {
		t.Fatalf("newLeasedPayload: %v", err)
	}
	var destination bytes.Buffer
	if _, err := payload.WriteTo(&destination); err != nil {
		t.Fatalf("WriteTo: %v", err)
	}
	if destination.String() != "encoded-frame" {
		t.Fatalf("destination = %q", destination.String())
	}
}

func TestInvokeLeasedStreamFailsClosedWithoutV9Transport(t *testing.T) {
	client, err := NewRuntimeClient(RuntimeTransportFunc{})
	if err != nil {
		t.Fatalf("NewRuntimeClient: %v", err)
	}
	_, err = client.InvokeLeasedStream(context.Background(), completeDraftForRuntimeTest(t))
	if !IsCode(err, ErrNotImplemented) {
		t.Fatalf("InvokeLeasedStream error = %v, want %s", err, ErrNotImplemented)
	}
}
