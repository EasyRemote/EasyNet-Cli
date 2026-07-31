package easynet

import (
	"context"
	"encoding/base64"
	"encoding/json"
	"errors"
	"fmt"
	"sync"
)

const MaxBidiBufferedFrames = 1024

// BidiState is the Runtime Core bidirectional session state.
type BidiState string

const (
	BidiCreated          BidiState = "Created"
	BidiOpening          BidiState = "Opening"
	BidiOpen             BidiState = "Open"
	BidiCancelRequested  BidiState = "CancelRequested"
	BidiHalfClosedLocal  BidiState = "HalfClosedLocal"
	BidiHalfClosedRemote BidiState = "HalfClosedRemote"
	BidiTerminal         BidiState = "Terminal"
	BidiClosed           BidiState = "Closed"
	BidiCancelled        BidiState = "Cancelled"
	BidiFailed           BidiState = "Failed"
)

// BidiStreamDescriptor describes a logical bidi stream requested for a session.
type BidiStreamDescriptor struct {
	StreamID    uint64 `json:"stream_id"`
	ContentType string `json:"content_type,omitempty"`
	CodecParams string `json:"codec_params,omitempty"`
	Ordering    string `json:"ordering,omitempty"`
}

// BidiTransport supplies bidirectional frames behind the SDK facade.
type BidiTransport interface {
	Send(ctx context.Context, frameJSON []byte) ([]byte, error)
	Recv(ctx context.Context) ([]byte, error)
	CloseSend(ctx context.Context) ([]byte, error)
	Close(ctx context.Context) error
	Cancel(ctx context.Context, reason string) ([]byte, error)
}

// BidiTransportFunc adapts functions into a BidiTransport.
type BidiTransportFunc struct {
	SendFunc      func(ctx context.Context, frameJSON []byte) ([]byte, error)
	RecvFunc      func(ctx context.Context) ([]byte, error)
	CloseSendFunc func(ctx context.Context) ([]byte, error)
	CloseFunc     func(ctx context.Context) error
	CancelFunc    func(ctx context.Context, reason string) ([]byte, error)
}

type bidiLifecycleEventKind uint8

const (
	bidiLifecycleReceiveFrame bidiLifecycleEventKind = iota + 1
	bidiLifecycleCloseSendOutcome
	bidiLifecycleCancelOutcome
)

type bidiLifecycleEvent struct {
	kind    bidiLifecycleEventKind
	frame   BidiFrame
	outcome BidiOutcome
}

func (f BidiTransportFunc) Send(ctx context.Context, frameJSON []byte) ([]byte, error) {
	if f.SendFunc == nil {
		return nil, invalidRuntimeClient("bidi send transport function is required")
	}
	return f.SendFunc(ctx, frameJSON)
}

func (f BidiTransportFunc) Recv(ctx context.Context) ([]byte, error) {
	if f.RecvFunc == nil {
		return nil, invalidRuntimeClient("bidi recv transport function is required")
	}
	return f.RecvFunc(ctx)
}

func (f BidiTransportFunc) CloseSend(ctx context.Context) ([]byte, error) {
	if f.CloseSendFunc == nil {
		return nil, invalidRuntimeClient("bidi close-send transport function is required")
	}
	return f.CloseSendFunc(ctx)
}

func (f BidiTransportFunc) Close(ctx context.Context) error {
	if f.CloseFunc == nil {
		return nil
	}
	return f.CloseFunc(ctx)
}

func (f BidiTransportFunc) Cancel(ctx context.Context, reason string) ([]byte, error) {
	if f.CancelFunc == nil {
		return nil, invalidRuntimeClient("bidi cancel transport function is required")
	}
	return f.CancelFunc(ctx, reason)
}

// BidiSession is the public bidirectional session lifecycle object.
type BidiSession struct {
	mu              sync.Mutex
	sessionID       string
	transport       BidiTransport
	runtimeState    BidiState
	carrierState    carrierState
	sentFrames      []BidiFrame
	receivedFrames  []BidiFrame
	lastSendSeq     uint64
	lastRecvSeq     uint64
	terminalFrame   *BidiTerminalFrame
	maxBuffered     int
	localHalfClose  bool
	remoteHalfClose bool
	sending         bool
	receiving       bool
}

// BidiFrame is an SDK bidi frame projection.
type BidiFrame struct {
	sequence             uint64
	kind                 string
	streamID             uint64
	terminal             bool
	transportTerminal    bool
	payloadContentType   string
	payloadBase64        string
	payloadJSON          json.RawMessage
	errorJSON            json.RawMessage
	admissionReceiptJSON json.RawMessage
	terminalReceiptJSON  json.RawMessage
}

// BidiTerminalFrame is the schema-shaped Runtime Core bidi terminal projection.
type BidiTerminalFrame struct {
	sessionID       string
	frameType       string
	seq             uint64
	payload         json.RawMessage
	errorJSON       json.RawMessage
	terminalReceipt json.RawMessage
}

// BidiOutcome is a close-send/cancel/terminal outcome projection.
type BidiOutcome struct {
	sessionID string
	state     BidiState
	terminal  bool
	reason    string
}

// NewBidiSessionFromJSON decodes runtime open metadata after frame0 acceptance.
func NewBidiSessionFromJSON(transport BidiTransport, raw []byte) (*BidiSession, error) {
	if transport == nil {
		return nil, invalidRuntimeClient("bidi transport is required")
	}
	var dto struct {
		SessionID   string `json:"session_id"`
		State       string `json:"state"`
		MaxBuffered int    `json:"max_buffered_frames"`
	}
	if err := json.Unmarshal(raw, &dto); err != nil {
		return nil, invalidRuntimePayload(fmt.Sprintf("decode bidi open JSON: %v", err), err)
	}
	if err := rejectUnknownRuntimeProjectionFields(raw, "bidi open", "session_id", "state", "max_buffered_frames"); err != nil {
		return nil, err
	}
	if dto.SessionID == "" {
		return nil, invalidRuntimePayload("session_id is required", nil)
	}
	state := BidiOpening
	if dto.State != "" {
		state = BidiState(dto.State)
	}
	if state != BidiOpening && state != BidiOpen {
		return nil, invalidRuntimePayload("bidi open state must be Opening or Open", nil)
	}
	maxBuffered := dto.MaxBuffered
	if maxBuffered == 0 {
		maxBuffered = MaxBidiBufferedFrames
	}
	if maxBuffered < 0 {
		return nil, invalidRuntimePayload("max_buffered_frames must be non-negative", nil)
	}
	return &BidiSession{
		sessionID:      dto.SessionID,
		transport:      transport,
		runtimeState:   state,
		carrierState:   carrierOpen,
		sentFrames:     []BidiFrame{},
		receivedFrames: []BidiFrame{},
		maxBuffered:    maxBuffered,
	}, nil
}

func (s *BidiSession) SessionID() string {
	if s == nil {
		return ""
	}
	return s.sessionID
}

func (s *BidiSession) State() BidiState {
	if s == nil {
		return BidiFailed
	}
	s.mu.Lock()
	defer s.mu.Unlock()
	switch s.carrierState {
	case carrierClosed:
		return BidiClosed
	case carrierFailed:
		return BidiFailed
	default:
		return s.runtimeState
	}
}

// RuntimeState returns only provider-observed lifecycle state. Local Close
// changes the carrier projection returned by State, not this runtime state.
func (s *BidiSession) RuntimeState() BidiState {
	if s == nil {
		return BidiFailed
	}
	s.mu.Lock()
	defer s.mu.Unlock()
	return s.runtimeState
}

func (s *BidiSession) SentFrames() []BidiFrame {
	if s == nil {
		return nil
	}
	s.mu.Lock()
	defer s.mu.Unlock()
	return append([]BidiFrame(nil), s.sentFrames...)
}

func (s *BidiSession) ReceivedFrames() []BidiFrame {
	if s == nil {
		return nil
	}
	s.mu.Lock()
	defer s.mu.Unlock()
	return append([]BidiFrame(nil), s.receivedFrames...)
}

func (s *BidiSession) MaxBufferedFrames() int {
	if s == nil {
		return 0
	}
	s.mu.Lock()
	defer s.mu.Unlock()
	return s.maxBuffered
}

func (s *BidiSession) TerminalFrame() (BidiTerminalFrame, error) {
	if s == nil {
		return BidiTerminalFrame{}, invalidRuntimeClient("bidi session is not initialized")
	}
	s.mu.Lock()
	defer s.mu.Unlock()
	if s.terminalFrame == nil {
		return BidiTerminalFrame{}, invalidRuntimePayload("bidi terminal frame has not been seen", nil)
	}
	return *s.terminalFrame, nil
}

func (s *BidiSession) Send(ctx context.Context, frame BidiFrame) (BidiFrame, error) {
	if s == nil {
		return BidiFrame{}, invalidRuntimeClient("bidi session is not initialized")
	}
	if ctx == nil {
		return BidiFrame{}, invalidRuntimeClient("context is required")
	}
	rawFrame, err := json.Marshal(frame)
	if err != nil {
		return BidiFrame{}, invalidRuntimePayload(fmt.Sprintf("encode bidi frame: %v", err), err)
	}

	s.mu.Lock()
	if s.transport == nil {
		s.mu.Unlock()
		return BidiFrame{}, invalidRuntimeClient("bidi session is not initialized")
	}
	if !s.carrierState.open() {
		s.mu.Unlock()
		return BidiFrame{}, invalidRuntimePayload("bidi carrier is closed", nil)
	}
	if s.runtimeState == BidiHalfClosedLocal {
		s.mu.Unlock()
		return BidiFrame{}, &SDKError{
			Code:      ErrCancelled,
			Stage:     "bidi",
			Retry:     RetryNever,
			Retryable: false,
			Message:   "bidi send path is closed",
		}
	}
	if s.runtimeState != BidiOpen && s.runtimeState != BidiHalfClosedRemote {
		s.mu.Unlock()
		return BidiFrame{}, invalidRuntimePayload("bidi send path is closed", nil)
	}
	if s.maxBuffered > 0 && len(s.sentFrames) >= s.maxBuffered {
		s.runtimeState = BidiFailed
		s.mu.Unlock()
		return BidiFrame{}, invalidRuntimePayload("bidi send buffer limit exceeded", nil)
	}
	if s.sending {
		s.mu.Unlock()
		return BidiFrame{}, invalidRuntimePayload("bidi send is already in progress", nil)
	}
	s.sending = true
	transport := s.transport
	s.mu.Unlock()

	rawAck, err := transport.Send(ctx, rawFrame)
	if err != nil {
		s.mu.Lock()
		s.sending = false
		if s.carrierState.open() && !isLocalCarrierInterruption(err) {
			s.runtimeState = BidiFailed
		}
		s.mu.Unlock()
		var sdkErr *SDKError
		if errors.As(err, &sdkErr) {
			return BidiFrame{}, sdkErr
		}
		return BidiFrame{}, transportRuntimeError("bidi send transport failed", err)
	}
	ack, err := NewBidiFrameFromJSON(rawAck)
	s.mu.Lock()
	s.sending = false
	if err != nil {
		s.runtimeState = BidiFailed
		s.mu.Unlock()
		return BidiFrame{}, err
	}
	if s.isRuntimeTerminalLocked() || s.runtimeState == BidiCancelRequested {
		s.mu.Unlock()
		return BidiFrame{}, invalidRuntimePayload("bidi send completed after the send path closed", nil)
	}
	if err := s.recordSentLocked(ack); err != nil {
		s.mu.Unlock()
		return BidiFrame{}, err
	}
	s.mu.Unlock()
	return ack, nil
}

func (s *BidiSession) Receive(ctx context.Context) (BidiFrame, error) {
	if s == nil {
		return BidiFrame{}, invalidRuntimeClient("bidi session is not initialized")
	}
	if ctx == nil {
		return BidiFrame{}, invalidRuntimeClient("context is required")
	}
	s.mu.Lock()
	if s.transport == nil {
		s.mu.Unlock()
		return BidiFrame{}, invalidRuntimeClient("bidi session is not initialized")
	}
	if !s.carrierState.open() {
		s.mu.Unlock()
		return BidiFrame{}, invalidRuntimePayload("bidi carrier is closed", nil)
	}
	if s.isRuntimeTerminalLocked() {
		s.mu.Unlock()
		return BidiFrame{}, invalidRuntimePayload("bidi session is terminal", nil)
	}
	if s.receiving {
		s.mu.Unlock()
		return BidiFrame{}, invalidRuntimePayload("bidi recv is already in progress", nil)
	}
	s.receiving = true
	transport := s.transport
	s.mu.Unlock()

	raw, err := transport.Recv(ctx)
	if err != nil {
		s.mu.Lock()
		s.receiving = false
		if s.carrierState.open() && !isLocalCarrierInterruption(err) {
			s.runtimeState = BidiFailed
		}
		s.mu.Unlock()
		var sdkErr *SDKError
		if errors.As(err, &sdkErr) {
			return BidiFrame{}, sdkErr
		}
		return BidiFrame{}, transportRuntimeError("bidi recv transport failed", err)
	}
	frame, err := NewBidiFrameFromJSON(raw)
	s.mu.Lock()
	s.receiving = false
	if err != nil {
		s.runtimeState = BidiFailed
		s.mu.Unlock()
		return BidiFrame{}, err
	}
	if err := s.applyLifecycleEventLocked(bidiLifecycleEvent{
		kind:  bidiLifecycleReceiveFrame,
		frame: frame,
	}); err != nil {
		s.mu.Unlock()
		return BidiFrame{}, err
	}
	s.mu.Unlock()
	return frame, nil
}

func (s *BidiSession) CloseSend(ctx context.Context) (BidiOutcome, error) {
	if s == nil {
		return BidiOutcome{}, invalidRuntimeClient("bidi session is not initialized")
	}
	if ctx == nil {
		return BidiOutcome{}, invalidRuntimeClient("context is required")
	}
	s.mu.Lock()
	if s.transport == nil {
		s.mu.Unlock()
		return BidiOutcome{}, invalidRuntimeClient("bidi session is not initialized")
	}
	if !s.carrierState.open() {
		s.mu.Unlock()
		return BidiOutcome{}, invalidRuntimePayload("bidi carrier is closed", nil)
	}
	if s.runtimeState != BidiOpen && s.runtimeState != BidiHalfClosedRemote {
		s.mu.Unlock()
		return BidiOutcome{}, invalidRuntimePayload("bidi send path is closed", nil)
	}
	if s.sending {
		s.mu.Unlock()
		return BidiOutcome{}, invalidRuntimePayload("bidi send is already in progress", nil)
	}
	s.sending = true
	transport := s.transport
	s.mu.Unlock()

	raw, err := transport.CloseSend(ctx)
	if err != nil {
		s.mu.Lock()
		s.sending = false
		if s.carrierState.open() && !isLocalCarrierInterruption(err) {
			s.runtimeState = BidiFailed
		}
		s.mu.Unlock()
		var sdkErr *SDKError
		if errors.As(err, &sdkErr) {
			return BidiOutcome{}, sdkErr
		}
		return BidiOutcome{}, transportRuntimeError("bidi close-send transport failed", err)
	}
	outcome, err := NewBidiOutcomeFromJSON(raw)
	s.mu.Lock()
	s.sending = false
	if err != nil {
		s.runtimeState = BidiFailed
		s.mu.Unlock()
		return BidiOutcome{}, err
	}
	if err := s.applyLifecycleEventLocked(bidiLifecycleEvent{
		kind:    bidiLifecycleCloseSendOutcome,
		outcome: outcome,
	}); err != nil {
		s.mu.Unlock()
		return BidiOutcome{}, err
	}
	s.mu.Unlock()
	return outcome, nil
}

func (s *BidiSession) Cancel(ctx context.Context, reason string) (BidiOutcome, error) {
	if s == nil {
		return BidiOutcome{}, invalidRuntimeClient("bidi session is not initialized")
	}
	if ctx == nil {
		return BidiOutcome{}, invalidRuntimeClient("context is required")
	}
	s.mu.Lock()
	if s.transport == nil {
		s.mu.Unlock()
		return BidiOutcome{}, invalidRuntimeClient("bidi session is not initialized")
	}
	if !s.carrierState.open() {
		s.mu.Unlock()
		return BidiOutcome{}, invalidRuntimePayload("bidi carrier is closed", nil)
	}
	if s.isRuntimeTerminalLocked() {
		s.mu.Unlock()
		return BidiOutcome{}, invalidRuntimePayload("bidi session is terminal", nil)
	}
	transport := s.transport
	s.mu.Unlock()

	raw, err := transport.Cancel(ctx, reason)
	if err != nil {
		var sdkErr *SDKError
		if errors.As(err, &sdkErr) {
			if sdkErr.Code != ErrNotImplemented && !isLocalCarrierInterruption(sdkErr) {
				s.mu.Lock()
				s.runtimeState = BidiFailed
				s.mu.Unlock()
			}
			return BidiOutcome{}, sdkErr
		}
		if !isLocalCarrierInterruption(err) {
			s.mu.Lock()
			s.runtimeState = BidiFailed
			s.mu.Unlock()
		}
		return BidiOutcome{}, transportRuntimeError("bidi cancel transport failed", err)
	}
	outcome, err := NewBidiOutcomeFromJSON(raw)
	if err != nil {
		s.mu.Lock()
		s.runtimeState = BidiFailed
		s.mu.Unlock()
		return BidiOutcome{}, err
	}
	if outcome.state != BidiCancelRequested || outcome.terminal {
		s.mu.Lock()
		s.runtimeState = BidiFailed
		s.mu.Unlock()
		return BidiOutcome{}, invalidRuntimePayload("bidi cancel transport must return CancelRequested with terminal=false", nil)
	}
	s.mu.Lock()
	defer s.mu.Unlock()
	if err := s.applyLifecycleEventLocked(bidiLifecycleEvent{
		kind:    bidiLifecycleCancelOutcome,
		outcome: outcome,
	}); err != nil {
		return BidiOutcome{}, err
	}
	return outcome, nil
}

func (s *BidiSession) Close(ctx context.Context) error {
	if s == nil {
		return invalidRuntimeClient("bidi session is not initialized")
	}
	if ctx == nil {
		return invalidRuntimeClient("context is required")
	}
	s.mu.Lock()
	if s.transport == nil {
		s.mu.Unlock()
		return invalidRuntimeClient("bidi session is not initialized")
	}
	if s.carrierState == carrierClosed {
		s.mu.Unlock()
		return nil
	}
	if s.carrierState == carrierClosing {
		s.mu.Unlock()
		return invalidRuntimePayload("bidi carrier close is already in progress", nil)
	}
	s.carrierState = carrierClosing
	transport := s.transport
	s.mu.Unlock()

	if err := transport.Close(ctx); err != nil {
		s.mu.Lock()
		s.carrierState = carrierFailed
		s.mu.Unlock()
		var sdkErr *SDKError
		if errors.As(err, &sdkErr) {
			return sdkErr
		}
		return transportRuntimeError("bidi close transport failed", err)
	}
	s.mu.Lock()
	s.carrierState = carrierClosed
	s.mu.Unlock()
	return nil
}

func (s *BidiSession) recordSentLocked(frame BidiFrame) error {
	if frame.sequence == 0 {
		s.runtimeState = BidiFailed
		return invalidRuntimePayload("bidi sent frame sequence is required", nil)
	}
	if frame.sequence <= s.lastSendSeq {
		s.runtimeState = BidiFailed
		return invalidRuntimePayload("bidi sent frames must be strictly ordered", nil)
	}
	s.lastSendSeq = frame.sequence
	s.sentFrames = append(s.sentFrames, frame)
	return nil
}

func (s *BidiSession) recordReceivedLocked(frame BidiFrame) error {
	if frame.sequence == 0 {
		s.runtimeState = BidiFailed
		return invalidRuntimePayload("bidi received frame sequence is required", nil)
	}
	if frame.sequence <= s.lastRecvSeq {
		s.runtimeState = BidiFailed
		return invalidRuntimePayload("bidi received frames must be strictly ordered", nil)
	}
	if s.maxBuffered > 0 && len(s.receivedFrames) >= s.maxBuffered {
		s.runtimeState = BidiFailed
		return invalidRuntimePayload("bidi receive buffer limit exceeded", nil)
	}
	s.lastRecvSeq = frame.sequence
	s.receivedFrames = append(s.receivedFrames, frame)
	return nil
}

func (s *BidiSession) applyLifecycleEventLocked(event bidiLifecycleEvent) error {
	switch event.kind {
	case bidiLifecycleReceiveFrame:
		return s.applyReceiveFrameLocked(event.frame)
	case bidiLifecycleCloseSendOutcome:
		return s.applyCloseSendOutcomeLocked(event.outcome)
	case bidiLifecycleCancelOutcome:
		return s.applyCancelOutcomeLocked(event.outcome)
	default:
		s.runtimeState = BidiFailed
		return invalidRuntimePayload("unknown bidi lifecycle event", nil)
	}
}

func (s *BidiSession) applyReceiveFrameLocked(frame BidiFrame) error {
	if s.isRuntimeTerminalLocked() && !frame.terminal {
		return invalidRuntimePayload("bidi session became terminal while receive was in progress", nil)
	}
	if err := s.recordReceivedLocked(frame); err != nil {
		return err
	}
	return s.applyReceivedStateLocked(frame)
}

func (s *BidiSession) applyCloseSendOutcomeLocked(outcome BidiOutcome) error {
	if outcome.state != BidiHalfClosedLocal || outcome.terminal {
		s.runtimeState = BidiFailed
		return invalidRuntimePayload("bidi close-send transport must not claim canonical terminality", nil)
	}
	if s.isRuntimeTerminalLocked() || s.runtimeState == BidiCancelRequested {
		return nil
	}
	s.localHalfClose = true
	s.runtimeState = BidiHalfClosedLocal
	return nil
}

func (s *BidiSession) applyCancelOutcomeLocked(outcome BidiOutcome) error {
	if s.runtimeState == BidiTerminal {
		return nil
	}
	if s.runtimeState == BidiFailed {
		return invalidRuntimePayload("bidi session failed while cancellation was in flight", nil)
	}
	s.runtimeState = outcome.state
	return nil
}

func (s *BidiSession) applyReceivedStateLocked(frame BidiFrame) error {
	switch {
	case frame.transportTerminal:
		if frame.terminal {
			terminal, err := NewBidiTerminalFrame(s.sessionID, frame)
			if err != nil {
				s.carrierState = carrierFailed
				return err
			}
			s.terminalFrame = &terminal
		}
		s.carrierState = carrierFailed
	case frame.terminal:
		terminal, err := NewBidiTerminalFrame(s.sessionID, frame)
		if err != nil {
			s.runtimeState = BidiFailed
			return err
		}
		s.terminalFrame = &terminal
		s.runtimeState = BidiTerminal
	case frame.kind == "remote_close_send":
		s.remoteHalfClose = true
		if !s.localHalfClose {
			s.runtimeState = BidiHalfClosedRemote
		}
	case s.runtimeState == BidiOpening:
		s.runtimeState = BidiOpen
	}
	return nil
}

func (s *BidiSession) isRuntimeTerminalLocked() bool {
	return s.runtimeState == BidiTerminal || s.runtimeState == BidiCancelled || s.runtimeState == BidiFailed
}

// NewBidiFrame creates a caller-owned outbound SDK bidi frame.
func NewBidiFrame(sequence uint64, kind string, streamID uint64) (BidiFrame, error) {
	if sequence == 0 {
		return BidiFrame{}, invalidRuntimePayload("sequence is required", nil)
	}
	if kind == "" {
		return BidiFrame{}, invalidRuntimePayload("kind is required", nil)
	}
	return BidiFrame{sequence: sequence, kind: kind, streamID: streamID}, nil
}

func (f BidiFrame) MarshalJSON() ([]byte, error) {
	return json.Marshal(map[string]any{
		"sequence":             f.sequence,
		"kind":                 f.kind,
		"stream_id":            f.streamID,
		"terminal":             f.terminal,
		"transport_terminal":   f.transportTerminal,
		"payload_content_type": f.payloadContentType,
		"payload_base64":       f.payloadBase64,
		"payload_json":         f.payloadJSON,
		"error":                f.errorJSON,
		"admission_receipt":    f.admissionReceiptJSON,
		"terminal_receipt":     f.terminalReceiptJSON,
	})
}

func (f BidiFrame) Sequence() uint64 {
	return f.sequence
}

func (f BidiFrame) Kind() string {
	return f.kind
}

func (f BidiFrame) StreamID() uint64 {
	return f.streamID
}

func (f BidiFrame) Terminal() bool {
	return f.terminal
}

func (f BidiFrame) TransportTerminal() bool {
	return f.transportTerminal
}

func (f BidiFrame) PayloadContentType() string {
	return f.payloadContentType
}

func (f BidiFrame) PayloadBase64() string {
	return f.payloadBase64
}

func (f BidiFrame) PayloadJSON() json.RawMessage {
	return append(json.RawMessage(nil), f.payloadJSON...)
}

func (f BidiFrame) ErrorJSON() json.RawMessage {
	return append(json.RawMessage(nil), f.errorJSON...)
}

func (f BidiFrame) AdmissionReceiptJSON() json.RawMessage {
	return append(json.RawMessage(nil), f.admissionReceiptJSON...)
}

func (f BidiFrame) TerminalReceiptJSON() json.RawMessage {
	return append(json.RawMessage(nil), f.terminalReceiptJSON...)
}

// NewBidiTerminalFrame projects a terminal bidi frame into bidi-frame.schema.json shape.
func NewBidiTerminalFrame(sessionID string, frame BidiFrame) (BidiTerminalFrame, error) {
	if sessionID == "" {
		return BidiTerminalFrame{}, invalidRuntimePayload("session_id is required", nil)
	}
	if !frame.terminal {
		return BidiTerminalFrame{}, invalidRuntimePayload("bidi frame is not terminal", nil)
	}
	return BidiTerminalFrame{
		sessionID:       sessionID,
		frameType:       bidiTerminalFrameType(frame),
		seq:             frame.sequence,
		payload:         append(json.RawMessage(nil), frame.payloadJSON...),
		errorJSON:       append(json.RawMessage(nil), frame.errorJSON...),
		terminalReceipt: frame.TerminalReceiptJSON(),
	}, nil
}

func (f BidiTerminalFrame) MarshalJSON() ([]byte, error) {
	value := map[string]any{
		"session_id": f.sessionID,
		"frame_type": f.frameType,
		"seq":        f.seq,
	}
	if len(f.payload) != 0 {
		value["payload"] = json.RawMessage(f.payload)
	}
	if len(f.errorJSON) != 0 {
		value["error"] = json.RawMessage(f.errorJSON)
	}
	if len(f.terminalReceipt) != 0 {
		value["terminal_receipt"] = json.RawMessage(f.terminalReceipt)
	}
	return json.Marshal(value)
}

func (f BidiTerminalFrame) SessionID() string {
	return f.sessionID
}

func (f BidiTerminalFrame) FrameType() string {
	return f.frameType
}

func (f BidiTerminalFrame) Seq() uint64 {
	return f.seq
}

func (f BidiTerminalFrame) PayloadJSON() json.RawMessage {
	return append(json.RawMessage(nil), f.payload...)
}

func (f BidiTerminalFrame) ErrorJSON() json.RawMessage {
	return append(json.RawMessage(nil), f.errorJSON...)
}

func (f BidiTerminalFrame) TerminalReceiptJSON() json.RawMessage {
	return append(json.RawMessage(nil), f.terminalReceipt...)
}

// NewBidiBinaryFrame creates an outbound binary data frame.
func NewBidiBinaryFrame(sequence uint64, streamID uint64, payload []byte, contentType string) (BidiFrame, error) {
	frame, err := NewBidiFrame(sequence, "data", streamID)
	if err != nil {
		return BidiFrame{}, err
	}
	frame.payloadBase64 = base64.StdEncoding.EncodeToString(payload)
	frame.payloadContentType = contentType
	return frame, nil
}

// NewBidiJSONFrame creates an outbound JSON-shaped frame.
func NewBidiJSONFrame(sequence uint64, kind string, streamID uint64, payload json.RawMessage) (BidiFrame, error) {
	frame, err := NewBidiFrame(sequence, kind, streamID)
	if err != nil {
		return BidiFrame{}, err
	}
	frame.payloadJSON = append(json.RawMessage(nil), payload...)
	frame.payloadContentType = "application/json"
	return frame, nil
}

func (o BidiOutcome) SessionID() string {
	return o.sessionID
}

func (o BidiOutcome) State() BidiState {
	return o.state
}

func (o BidiOutcome) Terminal() bool {
	return o.terminal
}

func (o BidiOutcome) Reason() string {
	return o.reason
}

func NewBidiFrameFromJSON(raw []byte) (BidiFrame, error) {
	var dto struct {
		Sequence           uint64          `json:"sequence"`
		Kind               string          `json:"kind"`
		StreamID           uint64          `json:"stream_id"`
		Terminal           bool            `json:"terminal"`
		TransportTerminal  bool            `json:"transport_terminal"`
		PayloadContentType string          `json:"payload_content_type"`
		PayloadBase64      string          `json:"payload_base64"`
		PayloadJSON        json.RawMessage `json:"payload_json"`
		Error              json.RawMessage `json:"error"`
		AdmissionReceipt   json.RawMessage `json:"admission_receipt"`
		TerminalReceipt    json.RawMessage `json:"terminal_receipt"`
	}
	if err := json.Unmarshal(raw, &dto); err != nil {
		return BidiFrame{}, invalidRuntimePayload(fmt.Sprintf("decode bidi frame JSON: %v", err), err)
	}
	if err := rejectUnknownRuntimeProjectionFields(
		raw,
		"bidi frame",
		"sequence",
		"kind",
		"stream_id",
		"terminal",
		"transport_terminal",
		"payload_content_type",
		"payload_base64",
		"payload_json",
		"error",
		"admission_receipt",
		"terminal_receipt",
	); err != nil {
		return BidiFrame{}, err
	}
	if err := rejectRetiredTopLevelReceiptAlias(raw, "bidi frame"); err != nil {
		return BidiFrame{}, err
	}
	if dto.Kind == "" {
		return BidiFrame{}, invalidRuntimePayload("bidi frame kind is required", nil)
	}
	if dto.Sequence == 0 {
		return BidiFrame{}, invalidRuntimePayload("bidi frame sequence is required", nil)
	}
	return BidiFrame{
		sequence:             dto.Sequence,
		kind:                 dto.Kind,
		streamID:             dto.StreamID,
		terminal:             dto.Terminal,
		transportTerminal:    dto.TransportTerminal,
		payloadContentType:   dto.PayloadContentType,
		payloadBase64:        dto.PayloadBase64,
		payloadJSON:          append(json.RawMessage(nil), dto.PayloadJSON...),
		errorJSON:            append(json.RawMessage(nil), dto.Error...),
		admissionReceiptJSON: append(json.RawMessage(nil), dto.AdmissionReceipt...),
		terminalReceiptJSON:  append(json.RawMessage(nil), dto.TerminalReceipt...),
	}, nil
}

func bidiTerminalFrameType(frame BidiFrame) string {
	switch frame.kind {
	case "terminal", "error", "cancelled":
		return frame.kind
	}
	if len(frame.errorJSON) != 0 {
		return "error"
	}
	return "terminal"
}

func NewBidiOutcomeFromJSON(raw []byte) (BidiOutcome, error) {
	var dto struct {
		SessionID string `json:"session_id"`
		State     string `json:"state"`
		Terminal  bool   `json:"terminal"`
		Reason    string `json:"reason"`
	}
	if err := json.Unmarshal(raw, &dto); err != nil {
		return BidiOutcome{}, invalidRuntimePayload(fmt.Sprintf("decode bidi outcome JSON: %v", err), err)
	}
	if err := rejectUnknownRuntimeProjectionFields(raw, "bidi outcome", "session_id", "state", "terminal", "reason"); err != nil {
		return BidiOutcome{}, err
	}
	if dto.SessionID == "" {
		return BidiOutcome{}, invalidRuntimePayload("session_id is required", nil)
	}
	if dto.State == "" {
		return BidiOutcome{}, invalidRuntimePayload("state is required", nil)
	}
	state := BidiState(dto.State)
	if state != BidiCancelRequested && state != BidiHalfClosedLocal && state != BidiHalfClosedRemote && state != BidiTerminal && state != BidiCancelled && state != BidiClosed && state != BidiFailed {
		return BidiOutcome{}, invalidRuntimePayload("invalid bidi outcome state", nil)
	}
	if state == BidiCancelRequested && dto.Terminal {
		return BidiOutcome{}, invalidRuntimePayload("bidi cancel request must not be terminal", nil)
	}
	if (state == BidiTerminal || state == BidiCancelled || state == BidiClosed || state == BidiFailed) && !dto.Terminal {
		return BidiOutcome{}, invalidRuntimePayload("terminal bidi outcome must set terminal", nil)
	}
	return BidiOutcome{
		sessionID: dto.SessionID,
		state:     state,
		terminal:  dto.Terminal,
		reason:    dto.Reason,
	}, nil
}
