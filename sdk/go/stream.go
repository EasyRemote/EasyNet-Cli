package easynet

import (
	"context"
	"encoding/json"
	"errors"
	"fmt"
	"sync"
)

const MaxStreamBufferedEvents = 1024

// StreamState is the Runtime Core server-stream state.
type StreamState string

const (
	StreamOpening           StreamState = "Opening"
	StreamOpen              StreamState = "Open"
	StreamCancelRequested   StreamState = "CancelRequested"
	StreamTerminalFrameSeen StreamState = "TerminalFrameSeen"
	StreamDraining          StreamState = "Draining"
	StreamClosed            StreamState = "Closed"
	StreamCancelled         StreamState = "Cancelled"
	StreamFailed            StreamState = "Failed"
)

// StreamTransport supplies stream event frames behind the SDK facade.
type StreamTransport interface {
	Recv(ctx context.Context) ([]byte, error)
	Cancel(ctx context.Context, reason string) ([]byte, error)
	Close(ctx context.Context) error
}

// StreamTransportFunc adapts functions into a StreamTransport.
type StreamTransportFunc struct {
	RecvFunc   func(ctx context.Context) ([]byte, error)
	CancelFunc func(ctx context.Context, reason string) ([]byte, error)
	CloseFunc  func(ctx context.Context) error
}

func (f StreamTransportFunc) Recv(ctx context.Context) ([]byte, error) {
	if f.RecvFunc == nil {
		return nil, invalidRuntimeClient("stream recv transport function is required")
	}
	return f.RecvFunc(ctx)
}

func (f StreamTransportFunc) Cancel(ctx context.Context, reason string) ([]byte, error) {
	if f.CancelFunc == nil {
		return nil, invalidRuntimeClient("stream cancel transport function is required")
	}
	return f.CancelFunc(ctx, reason)
}

func (f StreamTransportFunc) Close(ctx context.Context) error {
	if f.CloseFunc == nil {
		return nil
	}
	return f.CloseFunc(ctx)
}

// StreamHandle is the public ordered stream event state object.
type StreamHandle struct {
	mu            sync.Mutex
	streamID      string
	transport     StreamTransport
	state         StreamState
	events        []StreamEvent
	lastSequence  uint64
	terminalSeen  bool
	terminalEvent *StreamTerminalEvent
	maxBuffered   int
	receiving     bool
}

// StreamEvent is an SDK stream event projection.
type StreamEvent struct {
	sequence             uint64
	kind                 string
	state                string
	terminal             bool
	transportTerminal    bool
	payloadContentType   string
	payloadBase64        string
	payloadJSON          json.RawMessage
	selectedNodeID       string
	schedulingReason     string
	elapsedMS            int64
	errorJSON            json.RawMessage
	admissionReceiptJSON json.RawMessage
	terminalReceiptJSON  json.RawMessage
}

// StreamTerminalEvent is the schema-shaped Runtime Core stream terminal projection.
type StreamTerminalEvent struct {
	streamID        string
	eventType       string
	seq             uint64
	payload         json.RawMessage
	errorJSON       json.RawMessage
	terminalReceipt json.RawMessage
}

// StreamCancel is the stream cancellation outcome projection.
type StreamCancel struct {
	streamID  string
	cancelled bool
	state     StreamState
	terminal  bool
}

// NewStreamHandleFromJSON decodes stream-open metadata and creates an Opening handle.
func NewStreamHandleFromJSON(transport StreamTransport, raw []byte) (*StreamHandle, error) {
	if transport == nil {
		return nil, invalidRuntimeClient("stream transport is required")
	}
	var dto struct {
		StreamID    string `json:"stream_id"`
		State       string `json:"state"`
		MaxBuffered int    `json:"max_buffered_events"`
	}
	if len(raw) != 0 {
		if err := json.Unmarshal(raw, &dto); err != nil {
			return nil, invalidRuntimePayload(fmt.Sprintf("decode stream open JSON: %v", err), err)
		}
	}
	if dto.StreamID == "" {
		return nil, invalidRuntimePayload("stream_id is required", nil)
	}
	maxBuffered := dto.MaxBuffered
	if maxBuffered == 0 {
		maxBuffered = MaxStreamBufferedEvents
	}
	if maxBuffered < 0 {
		return nil, invalidRuntimePayload("max_buffered_events must be non-negative", nil)
	}
	state := StreamOpening
	if dto.State != "" {
		state = StreamState(dto.State)
	}
	if state != StreamOpening && state != StreamOpen {
		return nil, invalidRuntimePayload("stream open state must be Opening or Open", nil)
	}
	return &StreamHandle{
		streamID:    dto.StreamID,
		transport:   transport,
		state:       state,
		events:      []StreamEvent{},
		maxBuffered: maxBuffered,
	}, nil
}

func (s *StreamHandle) StreamID() string {
	if s == nil {
		return ""
	}
	return s.streamID
}

func (s *StreamHandle) State() StreamState {
	if s == nil {
		return StreamFailed
	}
	s.mu.Lock()
	defer s.mu.Unlock()
	return s.state
}

func (s *StreamHandle) Events() []StreamEvent {
	if s == nil {
		return nil
	}
	s.mu.Lock()
	defer s.mu.Unlock()
	return append([]StreamEvent(nil), s.events...)
}

func (s *StreamHandle) MaxBufferedEvents() int {
	if s == nil {
		return 0
	}
	s.mu.Lock()
	defer s.mu.Unlock()
	return s.maxBuffered
}

func (s *StreamHandle) TerminalEvent() (StreamTerminalEvent, error) {
	if s == nil {
		return StreamTerminalEvent{}, invalidRuntimeClient("stream handle is not initialized")
	}
	s.mu.Lock()
	defer s.mu.Unlock()
	if s.terminalEvent == nil {
		return StreamTerminalEvent{}, invalidRuntimePayload("stream terminal event has not been seen", nil)
	}
	return *s.terminalEvent, nil
}

func (s *StreamHandle) Next(ctx context.Context) (StreamEvent, error) {
	if s == nil {
		return StreamEvent{}, invalidRuntimeClient("stream handle is not initialized")
	}
	if ctx == nil {
		return StreamEvent{}, invalidRuntimeClient("context is required")
	}
	s.mu.Lock()
	if s.transport == nil {
		s.mu.Unlock()
		return StreamEvent{}, invalidRuntimeClient("stream handle is not initialized")
	}
	if s.isTerminalLocked() {
		s.mu.Unlock()
		return StreamEvent{}, invalidRuntimePayload("stream is terminal", nil)
	}
	if s.state == StreamTerminalFrameSeen || s.state == StreamDraining {
		s.mu.Unlock()
		return StreamEvent{}, invalidRuntimePayload("stream terminal event already seen", nil)
	}
	if s.receiving {
		s.mu.Unlock()
		return StreamEvent{}, invalidRuntimePayload("stream recv is already in progress", nil)
	}
	s.receiving = true
	transport := s.transport
	s.mu.Unlock()

	raw, err := transport.Recv(ctx)
	if err != nil {
		s.mu.Lock()
		s.receiving = false
		s.state = StreamFailed
		s.mu.Unlock()
		var sdkErr *SDKError
		if errors.As(err, &sdkErr) {
			return StreamEvent{}, sdkErr
		}
		return StreamEvent{}, transportRuntimeError("stream recv transport failed", err)
	}
	event, err := NewStreamEventFromJSON(raw)
	s.mu.Lock()
	s.receiving = false
	if err != nil {
		s.state = StreamFailed
		s.mu.Unlock()
		return StreamEvent{}, err
	}
	if err := s.applyEventLocked(event); err != nil {
		s.mu.Unlock()
		return StreamEvent{}, err
	}
	s.mu.Unlock()
	return event, nil
}

func (s *StreamHandle) Cancel(ctx context.Context, reason string) (StreamCancel, error) {
	if s == nil {
		return StreamCancel{}, invalidRuntimeClient("stream handle is not initialized")
	}
	if ctx == nil {
		return StreamCancel{}, invalidRuntimeClient("context is required")
	}
	s.mu.Lock()
	if s.transport == nil {
		s.mu.Unlock()
		return StreamCancel{}, invalidRuntimeClient("stream handle is not initialized")
	}
	if s.isTerminalLocked() || s.state == StreamTerminalFrameSeen || s.state == StreamDraining {
		s.mu.Unlock()
		return StreamCancel{}, invalidRuntimePayload("stream is terminal", nil)
	}
	transport := s.transport
	s.mu.Unlock()

	raw, err := transport.Cancel(ctx, reason)
	if err != nil {
		s.mu.Lock()
		s.state = StreamFailed
		s.mu.Unlock()
		var sdkErr *SDKError
		if errors.As(err, &sdkErr) {
			return StreamCancel{}, sdkErr
		}
		return StreamCancel{}, transportRuntimeError("stream cancel transport failed", err)
	}
	cancel, err := NewStreamCancelFromJSON(raw)
	if err != nil {
		s.mu.Lock()
		s.state = StreamFailed
		s.mu.Unlock()
		return StreamCancel{}, err
	}
	if cancel.state != StreamCancelRequested || cancel.terminal || cancel.cancelled {
		s.mu.Lock()
		s.state = StreamFailed
		s.mu.Unlock()
		return StreamCancel{}, invalidRuntimePayload("stream cancel transport must return CancelRequested with terminal=false", nil)
	}
	s.mu.Lock()
	defer s.mu.Unlock()
	if s.state == StreamTerminalFrameSeen || s.state == StreamDraining || s.state == StreamClosed {
		return cancel, nil
	}
	if s.state == StreamFailed {
		return StreamCancel{}, invalidRuntimePayload("stream failed while cancellation was in flight", nil)
	}
	s.state = cancel.state
	return cancel, nil
}

func (s *StreamHandle) Close(ctx context.Context) error {
	if s == nil {
		return invalidRuntimeClient("stream handle is not initialized")
	}
	if ctx == nil {
		return invalidRuntimeClient("context is required")
	}
	s.mu.Lock()
	if s.transport == nil {
		s.mu.Unlock()
		return invalidRuntimeClient("stream handle is not initialized")
	}
	if s.state == StreamClosed {
		s.mu.Unlock()
		return nil
	}
	if s.state == StreamTerminalFrameSeen {
		s.state = StreamDraining
	}
	transport := s.transport
	s.mu.Unlock()

	if err := transport.Close(ctx); err != nil {
		s.mu.Lock()
		s.state = StreamFailed
		s.mu.Unlock()
		var sdkErr *SDKError
		if errors.As(err, &sdkErr) {
			return sdkErr
		}
		return transportRuntimeError("stream close transport failed", err)
	}
	s.mu.Lock()
	s.state = StreamClosed
	s.mu.Unlock()
	return nil
}

func (s *StreamHandle) applyEventLocked(event StreamEvent) error {
	if s.isTerminalLocked() {
		return invalidRuntimePayload("stream is terminal", nil)
	}
	if event.sequence == 0 {
		s.state = StreamFailed
		return invalidRuntimePayload("stream event sequence is required", nil)
	}
	if event.sequence <= s.lastSequence {
		s.state = StreamFailed
		return invalidRuntimePayload("stream events must be strictly ordered", nil)
	}
	if s.terminalSeen {
		s.state = StreamFailed
		return invalidRuntimePayload("stream terminal event already seen", nil)
	}
	if s.maxBuffered > 0 && len(s.events) >= s.maxBuffered {
		s.state = StreamFailed
		return invalidRuntimePayload("stream event buffer limit exceeded", nil)
	}
	if s.state == StreamOpening {
		s.state = StreamOpen
	}
	s.lastSequence = event.sequence
	s.events = append(s.events, event)
	if event.terminal {
		s.terminalSeen = true
		terminal, err := NewStreamTerminalEvent(s.streamID, event)
		if err != nil {
			s.state = StreamFailed
			return err
		}
		s.terminalEvent = &terminal
		s.state = StreamTerminalFrameSeen
	} else if event.transportTerminal {
		s.state = StreamFailed
	}
	return nil
}

func (s *StreamHandle) isTerminalLocked() bool {
	return s.state == StreamClosed || s.state == StreamCancelled || s.state == StreamFailed
}

func (e StreamEvent) Sequence() uint64 {
	return e.sequence
}

func (e StreamEvent) Kind() string {
	return e.kind
}

func (e StreamEvent) State() string {
	return e.state
}

func (e StreamEvent) Terminal() bool {
	return e.terminal
}

func (e StreamEvent) TransportTerminal() bool {
	return e.transportTerminal
}

func (e StreamEvent) PayloadContentType() string {
	return e.payloadContentType
}

func (e StreamEvent) PayloadBase64() string {
	return e.payloadBase64
}

func (e StreamEvent) PayloadJSON() json.RawMessage {
	return append(json.RawMessage(nil), e.payloadJSON...)
}

func (e StreamEvent) SelectedNodeID() string {
	return e.selectedNodeID
}

func (e StreamEvent) SchedulingReason() string {
	return e.schedulingReason
}

func (e StreamEvent) ElapsedMS() int64 {
	return e.elapsedMS
}

func (e StreamEvent) ErrorJSON() json.RawMessage {
	return append(json.RawMessage(nil), e.errorJSON...)
}

func (e StreamEvent) AdmissionReceiptJSON() json.RawMessage {
	if len(e.admissionReceiptJSON) == 0 || string(e.admissionReceiptJSON) == "null" {
		return nil
	}
	return append(json.RawMessage(nil), e.admissionReceiptJSON...)
}

// TerminalReceiptJSON returns the canonical terminal receipt carried by a
// daemon frame. Payload is invocation output and is never interpreted as
// metadata.
func (e StreamEvent) TerminalReceiptJSON() json.RawMessage {
	if len(e.terminalReceiptJSON) != 0 && string(e.terminalReceiptJSON) != "null" {
		return append(json.RawMessage(nil), e.terminalReceiptJSON...)
	}
	return nil
}

func (c StreamCancel) StreamID() string {
	return c.streamID
}

func (c StreamCancel) Cancelled() bool {
	return c.cancelled
}

func (c StreamCancel) State() StreamState {
	return c.state
}

func (c StreamCancel) Terminal() bool {
	return c.terminal
}

// NewStreamTerminalEvent projects a terminal stream event into stream-event.schema.json shape.
func NewStreamTerminalEvent(streamID string, event StreamEvent) (StreamTerminalEvent, error) {
	if streamID == "" {
		return StreamTerminalEvent{}, invalidRuntimePayload("stream_id is required", nil)
	}
	if !event.terminal {
		return StreamTerminalEvent{}, invalidRuntimePayload("stream event is not terminal", nil)
	}
	return StreamTerminalEvent{
		streamID:        streamID,
		eventType:       streamTerminalEventType(event),
		seq:             event.sequence,
		payload:         append(json.RawMessage(nil), event.payloadJSON...),
		errorJSON:       append(json.RawMessage(nil), event.errorJSON...),
		terminalReceipt: event.TerminalReceiptJSON(),
	}, nil
}

func (e StreamTerminalEvent) MarshalJSON() ([]byte, error) {
	value := map[string]any{
		"stream_id":  e.streamID,
		"event_type": e.eventType,
		"seq":        e.seq,
	}
	if len(e.payload) != 0 {
		value["payload"] = json.RawMessage(e.payload)
	}
	if len(e.errorJSON) != 0 {
		value["error"] = json.RawMessage(e.errorJSON)
	}
	if len(e.terminalReceipt) != 0 {
		value["terminal_receipt"] = json.RawMessage(e.terminalReceipt)
	}
	return json.Marshal(value)
}

func (e StreamTerminalEvent) StreamID() string {
	return e.streamID
}

func (e StreamTerminalEvent) EventType() string {
	return e.eventType
}

func (e StreamTerminalEvent) Seq() uint64 {
	return e.seq
}

func (e StreamTerminalEvent) PayloadJSON() json.RawMessage {
	return append(json.RawMessage(nil), e.payload...)
}

func (e StreamTerminalEvent) ErrorJSON() json.RawMessage {
	return append(json.RawMessage(nil), e.errorJSON...)
}

func (e StreamTerminalEvent) TerminalReceiptJSON() json.RawMessage {
	return append(json.RawMessage(nil), e.terminalReceipt...)
}

// NewStreamEventFromJSON decodes one daemon stream event projection.
func NewStreamEventFromJSON(raw []byte) (StreamEvent, error) {
	var dto struct {
		Sequence           uint64          `json:"sequence"`
		Kind               string          `json:"kind"`
		State              string          `json:"state"`
		Terminal           bool            `json:"terminal"`
		TransportTerminal  bool            `json:"transport_terminal"`
		PayloadContentType string          `json:"payload_content_type"`
		PayloadBase64      string          `json:"payload_base64"`
		PayloadJSON        json.RawMessage `json:"payload_json"`
		SelectedNodeID     string          `json:"selected_node_id"`
		SchedulingReason   string          `json:"scheduling_reason"`
		ElapsedMS          int64           `json:"elapsed_ms"`
		Error              json.RawMessage `json:"error"`
		AdmissionReceipt   json.RawMessage `json:"admission_receipt"`
		TerminalReceipt    json.RawMessage `json:"terminal_receipt"`
	}
	if err := json.Unmarshal(raw, &dto); err != nil {
		return StreamEvent{}, invalidRuntimePayload(fmt.Sprintf("decode stream event JSON: %v", err), err)
	}
	if err := rejectRetiredTopLevelReceiptAlias(raw, "stream event"); err != nil {
		return StreamEvent{}, err
	}
	if dto.ElapsedMS < 0 {
		return StreamEvent{}, invalidRuntimePayload("elapsed_ms must be non-negative", nil)
	}
	if dto.Kind == "" {
		return StreamEvent{}, invalidRuntimePayload("stream event kind is required", nil)
	}
	if !isValidStreamEventKind(dto.Kind) {
		return StreamEvent{}, invalidRuntimePayload(fmt.Sprintf("unsupported stream event kind: %s", dto.Kind), nil)
	}
	return StreamEvent{
		sequence:             dto.Sequence,
		kind:                 dto.Kind,
		state:                dto.State,
		terminal:             dto.Terminal,
		transportTerminal:    dto.TransportTerminal,
		payloadContentType:   dto.PayloadContentType,
		payloadBase64:        dto.PayloadBase64,
		payloadJSON:          append(json.RawMessage(nil), dto.PayloadJSON...),
		selectedNodeID:       dto.SelectedNodeID,
		schedulingReason:     dto.SchedulingReason,
		elapsedMS:            dto.ElapsedMS,
		errorJSON:            append(json.RawMessage(nil), dto.Error...),
		admissionReceiptJSON: append(json.RawMessage(nil), dto.AdmissionReceipt...),
		terminalReceiptJSON:  append(json.RawMessage(nil), dto.TerminalReceipt...),
	}, nil
}

func isValidStreamEventKind(kind string) bool {
	switch kind {
	case "data", "terminal", "error", "cancelled", "timeout":
		return true
	default:
		return false
	}
}

func streamTerminalEventType(event StreamEvent) string {
	switch event.kind {
	case "terminal", "error", "cancelled", "timeout":
		return event.kind
	}
	if len(event.errorJSON) != 0 {
		return "error"
	}
	return "terminal"
}

// NewStreamCancelFromJSON decodes stream cancellation outcome JSON.
func NewStreamCancelFromJSON(raw []byte) (StreamCancel, error) {
	var dto struct {
		StreamID  string `json:"stream_id"`
		Cancelled bool   `json:"cancelled"`
		State     string `json:"state"`
		Terminal  bool   `json:"terminal"`
	}
	if err := json.Unmarshal(raw, &dto); err != nil {
		return StreamCancel{}, invalidRuntimePayload(fmt.Sprintf("decode stream cancel JSON: %v", err), err)
	}
	if dto.StreamID == "" {
		return StreamCancel{}, invalidRuntimePayload("stream_id is required", nil)
	}
	if dto.State == "" {
		return StreamCancel{}, invalidRuntimePayload("state is required", nil)
	}
	state := StreamState(dto.State)
	if state != StreamCancelRequested && state != StreamCancelled && state != StreamClosed && state != StreamFailed {
		return StreamCancel{}, invalidRuntimePayload("stream cancel state must be CancelRequested, Cancelled, Closed, or Failed", nil)
	}
	if state == StreamCancelRequested && dto.Terminal {
		return StreamCancel{}, invalidRuntimePayload("stream cancel request must not be terminal", nil)
	}
	return StreamCancel{
		streamID:  dto.StreamID,
		cancelled: dto.Cancelled,
		state:     state,
		terminal:  dto.Terminal,
	}, nil
}
