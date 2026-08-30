package easynet

import (
	"context"
	"encoding/json"
	"errors"
	"fmt"
	"io"
	"math"
	"sync"
)

// leasedPayloadStorage is the native ownership boundary behind LeasedPayload.
// Implementations must keep the payload address valid until release returns.
type leasedPayloadStorage interface {
	Len() int
	Released() bool
	CopyBytes() ([]byte, error)
	WriteTo(io.Writer) (int64, error)
	Retain() (leasedPayloadStorage, error)
	Release() error
}

// LeasedPayload owns one Runtime ABI v9 payload reference. It never exposes a
// persistent view over native memory. Callers must consume it with ToBytes or
// WriteTo, or explicitly call Release/Close. No finalizer participates in
// correctness; unreleased payloads intentionally apply Runtime backpressure.
type LeasedPayload struct {
	mu         sync.Mutex
	storage    leasedPayloadStorage
	released   bool
	releaseErr error
}

func newLeasedPayload(storage leasedPayloadStorage) (*LeasedPayload, error) {
	if storage == nil {
		return &LeasedPayload{released: true}, nil
	}
	if storage.Len() <= 0 {
		_ = storage.Release()
		return nil, invalidRuntimePayload("leased payload storage must be non-empty", nil)
	}
	return &LeasedPayload{storage: storage}, nil
}

// Len returns the immutable payload length. It remains available after release
// only as zero, so callers cannot mistake a released capability for live data.
func (p *LeasedPayload) Len() int {
	if p == nil {
		return 0
	}
	p.mu.Lock()
	defer p.mu.Unlock()
	if p.released || p.storage == nil {
		return 0
	}
	return p.storage.Len()
}

// Released reports whether this owner has relinquished its native reference.
func (p *LeasedPayload) Released() bool {
	if p == nil {
		return true
	}
	p.mu.Lock()
	defer p.mu.Unlock()
	return p.released || p.storage == nil || p.storage.Released()
}

// Retain creates another explicit owner for the same immutable payload. The
// returned owner must be released independently.
func (p *LeasedPayload) Retain() (*LeasedPayload, error) {
	if p == nil {
		return nil, invalidRuntimeClient("leased payload is not initialized")
	}
	p.mu.Lock()
	defer p.mu.Unlock()
	if p.released || p.storage == nil {
		return nil, invalidRuntimePayload("leased payload is released", nil)
	}
	storage, err := p.storage.Retain()
	if err != nil {
		return nil, err
	}
	return newLeasedPayload(storage)
}

// ToBytes copies the payload into Go-owned memory and releases this owner even
// when the copy fails. It is the convenient, explicitly non-zero-copy path.
func (p *LeasedPayload) ToBytes() (owned []byte, err error) {
	if p == nil {
		return nil, invalidRuntimeClient("leased payload is not initialized")
	}
	p.mu.Lock()
	defer p.mu.Unlock()
	if p.released || p.storage == nil {
		return nil, invalidRuntimePayload("leased payload is released", nil)
	}
	storage := p.storage
	defer func() {
		releaseErr := storage.Release()
		p.storage = nil
		p.released = true
		p.releaseErr = releaseErr
		err = errors.Join(err, releaseErr)
	}()
	return storage.CopyBytes()
}

// WriteTo copies once into Go-owned storage before calling an arbitrary
// io.Writer. This preserves the v9 no-base64 path without allowing a writer to
// retain a slice backed by native memory after the lease is released. The owner
// is released after the write, including short-write and error paths.
func (p *LeasedPayload) WriteTo(writer io.Writer) (written int64, err error) {
	if p == nil {
		return 0, invalidRuntimeClient("leased payload is not initialized")
	}
	p.mu.Lock()
	defer p.mu.Unlock()
	if p.released || p.storage == nil {
		return 0, invalidRuntimePayload("leased payload is released", nil)
	}
	storage := p.storage
	defer func() {
		releaseErr := storage.Release()
		p.storage = nil
		p.released = true
		p.releaseErr = releaseErr
		err = errors.Join(err, releaseErr)
	}()
	if writer == nil {
		return 0, invalidRuntimeClient("leased payload writer is required")
	}
	return storage.WriteTo(writer)
}

// Release relinquishes this owner's native reference exactly once.
func (p *LeasedPayload) Release() error {
	if p == nil {
		return nil
	}
	p.mu.Lock()
	defer p.mu.Unlock()
	if p.released {
		return p.releaseErr
	}
	p.released = true
	if p.storage != nil {
		p.releaseErr = p.storage.Release()
		p.storage = nil
	}
	return p.releaseErr
}

// Close is an idempotent alias for Release.
func (p *LeasedPayload) Close() error { return p.Release() }

type leasedStreamPacket struct {
	sequence             uint64
	kind                 string
	state                string
	terminal             bool
	transportTerminal    bool
	elapsedMS            uint64
	payloadContentType   string
	payload              leasedPayloadStorage
	admissionReceiptJSON []byte
	terminalReceiptJSON  []byte
	errorJSON            []byte
}

func (p *leasedStreamPacket) release() error {
	if p == nil || p.payload == nil {
		return nil
	}
	storage := p.payload
	p.payload = nil
	return storage.Release()
}

type leasedStreamTransport interface {
	RecvLeased(context.Context) (leasedStreamPacket, error)
	Cancel(context.Context, string) ([]byte, error)
	Close(context.Context) error
}

type leasedStreamOpener interface {
	OpenLeasedStream(context.Context, []byte) (leasedStreamTransport, []byte, error)
}

// LeasedStreamEvent projects canonical Runtime stream metadata while keeping
// the ABI v9 payload in a separate explicit owner.
type LeasedStreamEvent struct {
	sequence             uint64
	kind                 string
	state                string
	terminal             bool
	transportTerminal    bool
	payloadContentType   string
	payload              *LeasedPayload
	elapsedMS            int64
	errorJSON            json.RawMessage
	admissionReceiptJSON json.RawMessage
	terminalReceiptJSON  json.RawMessage
}

func newLeasedStreamEvent(packet leasedStreamPacket) (*LeasedStreamEvent, error) {
	if packet.sequence == 0 {
		_ = packet.release()
		return nil, invalidRuntimePayload("leased stream frame sequence must be positive", nil)
	}
	if _, ok := canonicalRuntimeStreamStates[packet.state]; !ok {
		_ = packet.release()
		return nil, invalidRuntimePayload("leased stream frame state is not canonical: "+packet.state, nil)
	}
	if !canonicalBinaryStreamKind(packet.kind) {
		_ = packet.release()
		return nil, invalidRuntimePayload("leased stream frame kind is not canonical: "+packet.kind, nil)
	}
	if packet.elapsedMS > math.MaxInt64 {
		_ = packet.release()
		return nil, invalidRuntimePayload("leased stream frame elapsed_ms exceeds int64", nil)
	}
	admission, err := decodeBinaryStreamSidecar(packet.admissionReceiptJSON, "admission_receipt")
	if err != nil {
		_ = packet.release()
		return nil, err
	}
	terminalReceipt, err := decodeBinaryStreamSidecar(packet.terminalReceiptJSON, "terminal_receipt")
	if err != nil {
		_ = packet.release()
		return nil, err
	}
	errorJSON, err := decodeBinaryStreamSidecar(packet.errorJSON, "error")
	if err != nil {
		_ = packet.release()
		return nil, err
	}
	payload, err := newLeasedPayload(packet.payload)
	if err != nil {
		_ = packet.release()
		return nil, err
	}
	packet.payload = nil
	return &LeasedStreamEvent{
		sequence:             packet.sequence,
		kind:                 packet.kind,
		state:                packet.state,
		terminal:             packet.terminal,
		transportTerminal:    packet.transportTerminal,
		payloadContentType:   packet.payloadContentType,
		payload:              payload,
		elapsedMS:            int64(packet.elapsedMS),
		errorJSON:            errorJSON,
		admissionReceiptJSON: admission,
		terminalReceiptJSON:  terminalReceipt,
	}, nil
}

func (e *LeasedStreamEvent) Sequence() uint64 {
	if e == nil {
		return 0
	}
	return e.sequence
}
func (e *LeasedStreamEvent) Kind() string {
	if e == nil {
		return ""
	}
	return e.kind
}
func (e *LeasedStreamEvent) State() string {
	if e == nil {
		return ""
	}
	return e.state
}
func (e *LeasedStreamEvent) Terminal() bool          { return e != nil && e.terminal }
func (e *LeasedStreamEvent) TransportTerminal() bool { return e != nil && e.transportTerminal }
func (e *LeasedStreamEvent) PayloadContentType() string {
	if e == nil {
		return ""
	}
	return e.payloadContentType
}
func (e *LeasedStreamEvent) ElapsedMS() int64 {
	if e == nil {
		return 0
	}
	return e.elapsedMS
}
func (e *LeasedStreamEvent) Payload() *LeasedPayload {
	if e == nil {
		return nil
	}
	return e.payload
}
func (e *LeasedStreamEvent) ErrorJSON() json.RawMessage {
	if e == nil {
		return nil
	}
	return append(json.RawMessage(nil), e.errorJSON...)
}
func (e *LeasedStreamEvent) AdmissionReceiptJSON() json.RawMessage {
	if e == nil {
		return nil
	}
	return append(json.RawMessage(nil), e.admissionReceiptJSON...)
}
func (e *LeasedStreamEvent) TerminalReceiptJSON() json.RawMessage {
	if e == nil {
		return nil
	}
	return append(json.RawMessage(nil), e.terminalReceiptJSON...)
}

// Release releases the event's payload owner. Metadata remains readable.
func (e *LeasedStreamEvent) Release() error {
	if e == nil || e.payload == nil {
		return nil
	}
	return e.payload.Release()
}

// Close is an idempotent alias for Release.
func (e *LeasedStreamEvent) Close() error { return e.Release() }

// LeasedStreamHandle owns an ABI v9 server stream. Unlike StreamHandle, it
// intentionally keeps no payload-bearing event history.
type LeasedStreamHandle struct {
	mu           sync.Mutex
	streamID     string
	transport    leasedStreamTransport
	runtimeState StreamState
	carrierState carrierState
	lastSequence uint64
	terminalSeen bool
	receiving    bool
}

func newLeasedStreamHandleFromJSON(transport leasedStreamTransport, raw []byte) (*LeasedStreamHandle, error) {
	if transport == nil {
		return nil, invalidRuntimeClient("leased stream transport is required")
	}
	var dto struct {
		StreamID string `json:"stream_id"`
		State    string `json:"state"`
	}
	if err := json.Unmarshal(raw, &dto); err != nil {
		_ = transport.Close(context.Background())
		return nil, invalidRuntimePayload(fmt.Sprintf("decode leased stream open JSON: %v", err), err)
	}
	if err := rejectUnknownRuntimeProjectionFields(raw, "leased stream open", "stream_id", "state", "max_buffered_events"); err != nil {
		_ = transport.Close(context.Background())
		return nil, err
	}
	if dto.StreamID == "" {
		_ = transport.Close(context.Background())
		return nil, invalidRuntimePayload("stream_id is required", nil)
	}
	state := StreamState(dto.State)
	if state != StreamOpening && state != StreamOpen {
		_ = transport.Close(context.Background())
		return nil, invalidRuntimePayload("leased stream open state must be Opening or Open", nil)
	}
	return &LeasedStreamHandle{streamID: dto.StreamID, transport: transport, runtimeState: state, carrierState: carrierOpen}, nil
}

func (s *LeasedStreamHandle) StreamID() string {
	if s == nil {
		return ""
	}
	return s.streamID
}

func (s *LeasedStreamHandle) State() StreamState {
	if s == nil {
		return StreamFailed
	}
	s.mu.Lock()
	defer s.mu.Unlock()
	if s.carrierState == carrierClosed {
		return StreamClosed
	}
	if s.carrierState == carrierFailed {
		return StreamFailed
	}
	return s.runtimeState
}

// RuntimeState returns only the provider-observed lifecycle state.
func (s *LeasedStreamHandle) RuntimeState() StreamState {
	if s == nil {
		return StreamFailed
	}
	s.mu.Lock()
	defer s.mu.Unlock()
	return s.runtimeState
}

// Next transfers ownership of exactly one frame payload to the caller.
func (s *LeasedStreamHandle) Next(ctx context.Context) (*LeasedStreamEvent, error) {
	if s == nil || s.transport == nil {
		return nil, invalidRuntimeClient("leased stream handle is not initialized")
	}
	if ctx == nil {
		return nil, invalidRuntimeClient("context is required")
	}
	s.mu.Lock()
	if !s.carrierState.open() {
		s.mu.Unlock()
		return nil, invalidRuntimePayload("leased stream carrier is closed", nil)
	}
	if s.terminalSeen {
		s.mu.Unlock()
		return nil, invalidRuntimePayload("leased stream is terminal", nil)
	}
	if s.receiving {
		s.mu.Unlock()
		return nil, invalidRuntimePayload("leased stream recv is already in progress", nil)
	}
	s.receiving = true
	transport := s.transport
	s.mu.Unlock()

	packet, err := transport.RecvLeased(ctx)
	if err != nil {
		s.mu.Lock()
		s.receiving = false
		if s.carrierState.open() && !isLocalCarrierInterruption(err) {
			s.runtimeState = StreamFailed
		}
		s.mu.Unlock()
		var sdkErr *SDKError
		if errors.As(err, &sdkErr) {
			return nil, sdkErr
		}
		return nil, transportRuntimeError("leased stream recv transport failed", err)
	}
	event, err := newLeasedStreamEvent(packet)
	if err != nil {
		closeErr := transport.Close(context.Background())
		s.mu.Lock()
		s.receiving = false
		s.runtimeState = StreamFailed
		s.carrierState = carrierFailed
		s.mu.Unlock()
		return nil, errors.Join(err, closeErr)
	}
	s.mu.Lock()
	s.receiving = false
	if event.sequence <= s.lastSequence {
		s.runtimeState = StreamFailed
		s.carrierState = carrierFailed
		s.mu.Unlock()
		_ = event.Release()
		return nil, errors.Join(
			invalidRuntimePayload("leased stream events must be strictly ordered", nil),
			transport.Close(context.Background()),
		)
	}
	if s.runtimeState == StreamOpening {
		s.runtimeState = StreamOpen
	}
	s.lastSequence = event.sequence
	if event.terminal || event.transportTerminal {
		s.terminalSeen = true
		if event.transportTerminal {
			s.carrierState = carrierFailed
		} else {
			s.runtimeState = StreamTerminalFrameSeen
		}
	}
	s.mu.Unlock()
	return event, nil
}

func (s *LeasedStreamHandle) Cancel(ctx context.Context, reason string) (StreamCancel, error) {
	if s == nil || s.transport == nil {
		return StreamCancel{}, invalidRuntimeClient("leased stream handle is not initialized")
	}
	if ctx == nil {
		return StreamCancel{}, invalidRuntimeClient("context is required")
	}
	s.mu.Lock()
	if !s.carrierState.open() {
		s.mu.Unlock()
		return StreamCancel{}, invalidRuntimePayload("leased stream carrier is closed", nil)
	}
	if s.terminalSeen {
		s.mu.Unlock()
		return StreamCancel{}, invalidRuntimePayload("leased stream is terminal", nil)
	}
	transport := s.transport
	s.mu.Unlock()
	raw, err := transport.Cancel(ctx, reason)
	if err != nil {
		return StreamCancel{}, err
	}
	cancel, err := NewStreamCancelFromJSON(raw)
	if err != nil {
		return StreamCancel{}, err
	}
	if cancel.state != StreamCancelRequested || cancel.terminal || cancel.cancelled {
		return StreamCancel{}, invalidRuntimePayload("leased stream cancel transport must return CancelRequested with terminal=false", nil)
	}
	s.mu.Lock()
	if !s.terminalSeen {
		s.runtimeState = StreamCancelRequested
	}
	s.mu.Unlock()
	return cancel, nil
}

// Close stops callback delivery and releases every outstanding native lease,
// including frames received by the caller but not explicitly released yet.
func (s *LeasedStreamHandle) Close(ctx context.Context) error {
	if s == nil {
		return nil
	}
	if ctx == nil {
		return invalidRuntimeClient("context is required")
	}
	s.mu.Lock()
	if s.carrierState == carrierClosed {
		s.mu.Unlock()
		return nil
	}
	if s.carrierState == carrierClosing {
		s.mu.Unlock()
		return invalidRuntimePayload("leased stream carrier close is already in progress", nil)
	}
	s.carrierState = carrierClosing
	transport := s.transport
	s.mu.Unlock()
	err := transport.Close(ctx)
	s.mu.Lock()
	if err != nil {
		s.carrierState = carrierFailed
	} else {
		s.carrierState = carrierClosed
	}
	s.mu.Unlock()
	return err
}
