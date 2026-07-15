package easynet

import (
	"context"
	"encoding/base64"
	"encoding/json"
	"errors"
	"fmt"
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
	sessionID      string
	transport      BidiTransport
	state          BidiState
	sentFrames     []BidiFrame
	receivedFrames []BidiFrame
	lastSendSeq    uint64
	lastRecvSeq    uint64
	terminalFrame  *BidiTerminalFrame
	maxBuffered    int
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

// NewBidiSessionFromJSON decodes daemon open metadata after frame0 acceptance.
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
		state:          state,
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
	return s.state
}

func (s *BidiSession) SentFrames() []BidiFrame {
	if s == nil {
		return nil
	}
	return append([]BidiFrame(nil), s.sentFrames...)
}

func (s *BidiSession) ReceivedFrames() []BidiFrame {
	if s == nil {
		return nil
	}
	return append([]BidiFrame(nil), s.receivedFrames...)
}

func (s *BidiSession) MaxBufferedFrames() int {
	if s == nil {
		return 0
	}
	return s.maxBuffered
}

func (s *BidiSession) TerminalFrame() (BidiTerminalFrame, error) {
	if s == nil {
		return BidiTerminalFrame{}, invalidRuntimeClient("bidi session is not initialized")
	}
	if s.terminalFrame == nil {
		return BidiTerminalFrame{}, invalidRuntimePayload("bidi terminal frame has not been seen", nil)
	}
	return *s.terminalFrame, nil
}

func (s *BidiSession) Send(ctx context.Context, frame BidiFrame) (BidiFrame, error) {
	if s == nil || s.transport == nil {
		return BidiFrame{}, invalidRuntimeClient("bidi session is not initialized")
	}
	if ctx == nil {
		return BidiFrame{}, invalidRuntimeClient("context is required")
	}
	if s.state == BidiHalfClosedLocal {
		return BidiFrame{}, &SDKError{
			Code:      ErrCancelled,
			Stage:     "bidi",
			Retry:     RetryNever,
			Retryable: false,
			Message:   "bidi send path is closed",
		}
	}
	if s.state != BidiOpen {
		return BidiFrame{}, invalidRuntimePayload("bidi send path is closed", nil)
	}
	if s.maxBuffered > 0 && len(s.sentFrames) >= s.maxBuffered {
		s.state = BidiFailed
		return BidiFrame{}, invalidRuntimePayload("bidi send buffer limit exceeded", nil)
	}
	rawFrame, err := json.Marshal(frame)
	if err != nil {
		return BidiFrame{}, invalidRuntimePayload(fmt.Sprintf("encode bidi frame: %v", err), err)
	}
	rawAck, err := s.transport.Send(ctx, rawFrame)
	if err != nil {
		s.state = BidiFailed
		var sdkErr *SDKError
		if errors.As(err, &sdkErr) {
			return BidiFrame{}, sdkErr
		}
		return BidiFrame{}, transportRuntimeError("bidi send transport failed", err)
	}
	ack, err := NewBidiFrameFromJSON(rawAck)
	if err != nil {
		s.state = BidiFailed
		return BidiFrame{}, err
	}
	if err := s.recordSent(ack); err != nil {
		return BidiFrame{}, err
	}
	return ack, nil
}

func (s *BidiSession) Receive(ctx context.Context) (BidiFrame, error) {
	if s == nil || s.transport == nil {
		return BidiFrame{}, invalidRuntimeClient("bidi session is not initialized")
	}
	if ctx == nil {
		return BidiFrame{}, invalidRuntimeClient("context is required")
	}
	if s.isTerminal() || s.state == BidiTerminal {
		return BidiFrame{}, invalidRuntimePayload("bidi session is terminal", nil)
	}
	raw, err := s.transport.Recv(ctx)
	if err != nil {
		s.state = BidiFailed
		var sdkErr *SDKError
		if errors.As(err, &sdkErr) {
			return BidiFrame{}, sdkErr
		}
		return BidiFrame{}, transportRuntimeError("bidi recv transport failed", err)
	}
	frame, err := NewBidiFrameFromJSON(raw)
	if err != nil {
		s.state = BidiFailed
		return BidiFrame{}, err
	}
	if err := s.recordReceived(frame); err != nil {
		return BidiFrame{}, err
	}
	if err := s.applyReceivedState(frame); err != nil {
		return BidiFrame{}, err
	}
	return frame, nil
}

func (s *BidiSession) CloseSend(ctx context.Context) (BidiOutcome, error) {
	if s == nil || s.transport == nil {
		return BidiOutcome{}, invalidRuntimeClient("bidi session is not initialized")
	}
	if ctx == nil {
		return BidiOutcome{}, invalidRuntimeClient("context is required")
	}
	if s.state != BidiOpen && s.state != BidiHalfClosedRemote {
		return BidiOutcome{}, invalidRuntimePayload("bidi send path is closed", nil)
	}
	raw, err := s.transport.CloseSend(ctx)
	if err != nil {
		s.state = BidiFailed
		var sdkErr *SDKError
		if errors.As(err, &sdkErr) {
			return BidiOutcome{}, sdkErr
		}
		return BidiOutcome{}, transportRuntimeError("bidi close-send transport failed", err)
	}
	outcome, err := NewBidiOutcomeFromJSON(raw)
	if err != nil {
		s.state = BidiFailed
		return BidiOutcome{}, err
	}
	if s.state == BidiHalfClosedRemote {
		s.state = BidiTerminal
		return BidiOutcome{
			sessionID: outcome.sessionID,
			state:     BidiTerminal,
			terminal:  true,
			reason:    outcome.reason,
		}, nil
	} else {
		s.state = outcome.state
	}
	return outcome, nil
}

func (s *BidiSession) Cancel(ctx context.Context, reason string) (BidiOutcome, error) {
	if s == nil || s.transport == nil {
		return BidiOutcome{}, invalidRuntimeClient("bidi session is not initialized")
	}
	if ctx == nil {
		return BidiOutcome{}, invalidRuntimeClient("context is required")
	}
	if s.isTerminal() {
		return BidiOutcome{}, invalidRuntimePayload("bidi session is terminal", nil)
	}
	raw, err := s.transport.Cancel(ctx, reason)
	if err != nil {
		s.state = BidiFailed
		var sdkErr *SDKError
		if errors.As(err, &sdkErr) {
			return BidiOutcome{}, sdkErr
		}
		return BidiOutcome{}, transportRuntimeError("bidi cancel transport failed", err)
	}
	outcome, err := NewBidiOutcomeFromJSON(raw)
	if err != nil {
		s.state = BidiFailed
		return BidiOutcome{}, err
	}
	s.state = outcome.state
	return outcome, nil
}

func (s *BidiSession) Close(ctx context.Context) error {
	if s == nil || s.transport == nil {
		return invalidRuntimeClient("bidi session is not initialized")
	}
	if ctx == nil {
		return invalidRuntimeClient("context is required")
	}
	if s.state == BidiClosed {
		return nil
	}
	if s.state != BidiTerminal && s.state != BidiCancelled && s.state != BidiFailed {
		return invalidRuntimePayload("bidi session must be terminal before close", nil)
	}
	if err := s.transport.Close(ctx); err != nil {
		s.state = BidiFailed
		var sdkErr *SDKError
		if errors.As(err, &sdkErr) {
			return sdkErr
		}
		return transportRuntimeError("bidi close transport failed", err)
	}
	s.state = BidiClosed
	return nil
}

func (s *BidiSession) recordSent(frame BidiFrame) error {
	if frame.sequence == 0 {
		s.state = BidiFailed
		return invalidRuntimePayload("bidi sent frame sequence is required", nil)
	}
	if frame.sequence <= s.lastSendSeq {
		s.state = BidiFailed
		return invalidRuntimePayload("bidi sent frames must be strictly ordered", nil)
	}
	s.lastSendSeq = frame.sequence
	s.sentFrames = append(s.sentFrames, frame)
	return nil
}

func (s *BidiSession) recordReceived(frame BidiFrame) error {
	if frame.sequence == 0 {
		s.state = BidiFailed
		return invalidRuntimePayload("bidi received frame sequence is required", nil)
	}
	if frame.sequence <= s.lastRecvSeq {
		s.state = BidiFailed
		return invalidRuntimePayload("bidi received frames must be strictly ordered", nil)
	}
	if s.maxBuffered > 0 && len(s.receivedFrames) >= s.maxBuffered {
		s.state = BidiFailed
		return invalidRuntimePayload("bidi receive buffer limit exceeded", nil)
	}
	s.lastRecvSeq = frame.sequence
	s.receivedFrames = append(s.receivedFrames, frame)
	return nil
}

func (s *BidiSession) applyReceivedState(frame BidiFrame) error {
	switch {
	case frame.terminal:
		terminal, err := NewBidiTerminalFrame(s.sessionID, frame)
		if err != nil {
			s.state = BidiFailed
			return err
		}
		s.terminalFrame = &terminal
		s.state = BidiTerminal
	case frame.transportTerminal:
		s.state = BidiFailed
	case frame.kind == "remote_close_send":
		if s.state == BidiHalfClosedLocal {
			s.state = BidiTerminal
		} else {
			s.state = BidiHalfClosedRemote
		}
	case s.state == BidiOpening:
		s.state = BidiOpen
	}
	return nil
}

func (s *BidiSession) isTerminal() bool {
	return s.state == BidiTerminal || s.state == BidiClosed || s.state == BidiCancelled || s.state == BidiFailed
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
		Event              string          `json:"event"`
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
	kind := dto.Kind
	if kind == "" {
		kind = dto.Event
	}
	if kind == "" {
		return BidiFrame{}, invalidRuntimePayload("bidi frame kind is required", nil)
	}
	if dto.Sequence == 0 {
		return BidiFrame{}, invalidRuntimePayload("bidi frame sequence is required", nil)
	}
	return BidiFrame{
		sequence:             dto.Sequence,
		kind:                 kind,
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
