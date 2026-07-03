package easynet

import (
	"context"
	"encoding/json"
	"errors"
	"fmt"
	"strings"
)

const eventsProfile = "events"
const directoryEventStream = "directory"

const (
	MinEventHeartbeatIntervalMS = 1000
	MaxEventHeartbeatIntervalMS = 300000
)

// EventsCarrierBase is the complete carrier context shared by Events operations.
type EventsCarrierBase struct {
	CallerURA         string         `json:"caller_ura"`
	CalleeURA         string         `json:"callee_ura"`
	SubjectURA        string         `json:"subject_ura"`
	DescriptorVersion string         `json:"descriptor_version"`
	NonceBase64       string         `json:"nonce_base64"`
	CausalContext     map[string]any `json:"causal_context"`
	Metadata          map[string]any `json:"metadata,omitempty"`
}

// EventCursor is the explicit stream cursor used for resume and frame ordering.
type EventCursor struct {
	Stream   string `json:"stream"`
	Sequence uint64 `json:"sequence"`
	Token    string `json:"token,omitempty"`
}

func NewEventCursor(stream string, sequence uint64) (EventCursor, error) {
	if stream == "" {
		return EventCursor{}, invalidRuntimePayload("event cursor stream is required", nil)
	}
	if stream != directoryEventStream {
		return EventCursor{}, invalidRuntimePayload("unsupported event stream", nil)
	}
	return EventCursor{Stream: stream, Sequence: sequence}, nil
}

func (c EventCursor) ResumeToken() string {
	if c.Token != "" {
		return c.Token
	}
	if c.Stream == "" {
		return ""
	}
	return fmt.Sprintf("%s:%d", c.Stream, c.Sequence)
}

// EventsDirectorySubscriptionRequest builds a daemon directory stream carrier.
type EventsDirectorySubscriptionRequest struct {
	EventsCarrierBase
	Realm               string       `json:"realm,omitempty"`
	OwnerURA            string       `json:"owner_ura,omitempty"`
	DeviceURA           string       `json:"device_ura,omitempty"`
	AgentURA            string       `json:"agent_ura,omitempty"`
	ResumeCursor        *EventCursor `json:"resume_cursor,omitempty"`
	HeartbeatIntervalMS int          `json:"heartbeat_interval_ms,omitempty"`
}

type DirectoryEventQuery = EventsDirectorySubscriptionRequest

// EventProjectionInput asks the daemon contract to project one raw directory event.
type EventProjectionInput struct {
	Cursor      EventCursor    `json:"cursor"`
	Event       map[string]any `json:"event"`
	EventID     string         `json:"event_id,omitempty"`
	ResumeToken string         `json:"resume_token,omitempty"`
	TenantRef   any            `json:"tenant_ref,omitempty"`
}

// EventDropReportInput asks the daemon contract to project a first-class drop report.
type EventDropReportInput struct {
	Cursor           EventCursor `json:"cursor"`
	OccurredUnixMS   int64       `json:"occurred_unix_ms"`
	DroppedCount     int         `json:"dropped_count"`
	ReconnectAfterMS *int        `json:"reconnect_after_ms,omitempty"`
	Reason           string      `json:"reason,omitempty"`
	EventID          string      `json:"event_id,omitempty"`
	ResumeToken      string      `json:"resume_token,omitempty"`
	TenantRef        any         `json:"tenant_ref,omitempty"`
}

// EventTerminalInput asks the daemon contract to project an explicit terminal frame.
type EventTerminalInput struct {
	Cursor           EventCursor `json:"cursor"`
	OccurredUnixMS   int64       `json:"occurred_unix_ms"`
	ReconnectAfterMS *int        `json:"reconnect_after_ms,omitempty"`
	Reason           string      `json:"reason,omitempty"`
	EventID          string      `json:"event_id,omitempty"`
	ResumeToken      string      `json:"resume_token,omitempty"`
	TenantRef        any         `json:"tenant_ref,omitempty"`
}

// EventStream is the Events profile subscription state seam.
type EventStream struct {
	Stream      string         `json:"stream"`
	StreamID    string         `json:"stream_id,omitempty"`
	State       string         `json:"state"`
	ResumeToken string         `json:"resume_token,omitempty"`
	Metadata    map[string]any `json:"metadata"`
}

type EventFrame struct {
	Profile          string         `json:"profile"`
	Stream           string         `json:"stream"`
	Kind             string         `json:"kind"`
	EventID          string         `json:"event_id"`
	Cursor           EventCursor    `json:"cursor"`
	ResumeToken      string         `json:"resume_token"`
	OccurredUnixMS   int64          `json:"occurred_unix_ms"`
	OccurredAt       string         `json:"occurred_at"`
	SubjectRef       any            `json:"subject_ref"`
	TenantRef        any            `json:"tenant_ref"`
	Payload          any            `json:"payload"`
	DroppedCount     int            `json:"dropped_count"`
	ReconnectAfterMS *int           `json:"reconnect_after_ms"`
	Terminal         bool           `json:"terminal"`
	Metadata         map[string]any `json:"metadata"`
}

type DirectoryEvent = EventFrame
type EventDropReport = EventFrame

// EventTransport supplies daemon Events operations behind the facade.
type EventTransport interface {
	BuildDirectorySubscriptionInvocation(ctx context.Context, requestJSON []byte) ([]byte, error)
	SubscribeDirectory(ctx context.Context, requestJSON []byte) ([]byte, error)
	ProjectDirectoryEvent(ctx context.Context, eventJSON []byte) ([]byte, error)
	ProjectDropReport(ctx context.Context, dropJSON []byte) ([]byte, error)
	ProjectTerminal(ctx context.Context, terminalJSON []byte) ([]byte, error)
}

// EventTransportFunc adapts functions into an EventTransport.
type EventTransportFunc struct {
	BuildDirectorySubscriptionInvocationFunc func(ctx context.Context, requestJSON []byte) ([]byte, error)
	SubscribeDirectoryFunc                   func(ctx context.Context, requestJSON []byte) ([]byte, error)
	ProjectDirectoryEventFunc                func(ctx context.Context, eventJSON []byte) ([]byte, error)
	ProjectDropReportFunc                    func(ctx context.Context, dropJSON []byte) ([]byte, error)
	ProjectTerminalFunc                      func(ctx context.Context, terminalJSON []byte) ([]byte, error)
}

func (f EventTransportFunc) BuildDirectorySubscriptionInvocation(ctx context.Context, requestJSON []byte) ([]byte, error) {
	if f.BuildDirectorySubscriptionInvocationFunc == nil {
		return nil, invalidRuntimeClient("events directory-subscription invocation transport function is required")
	}
	return f.BuildDirectorySubscriptionInvocationFunc(ctx, requestJSON)
}

func (f EventTransportFunc) SubscribeDirectory(ctx context.Context, requestJSON []byte) ([]byte, error) {
	if f.SubscribeDirectoryFunc == nil {
		return nil, invalidRuntimeClient("events subscribe-directory transport function is required")
	}
	return f.SubscribeDirectoryFunc(ctx, requestJSON)
}

func (f EventTransportFunc) ProjectDirectoryEvent(ctx context.Context, eventJSON []byte) ([]byte, error) {
	if f.ProjectDirectoryEventFunc == nil {
		return nil, invalidRuntimeClient("events project-directory-event transport function is required")
	}
	return f.ProjectDirectoryEventFunc(ctx, eventJSON)
}

func (f EventTransportFunc) ProjectDropReport(ctx context.Context, dropJSON []byte) ([]byte, error) {
	if f.ProjectDropReportFunc == nil {
		return nil, invalidRuntimeClient("events project-drop-report transport function is required")
	}
	return f.ProjectDropReportFunc(ctx, dropJSON)
}

func (f EventTransportFunc) ProjectTerminal(ctx context.Context, terminalJSON []byte) ([]byte, error) {
	if f.ProjectTerminalFunc == nil {
		return nil, invalidRuntimeClient("events project-terminal transport function is required")
	}
	return f.ProjectTerminalFunc(ctx, terminalJSON)
}

// EventClient is the Events profile facade.
type EventClient struct {
	transport EventTransport
}

func NewEventClient(transport EventTransport) (*EventClient, error) {
	if transport == nil {
		return nil, invalidRuntimeClient("events transport is required")
	}
	return &EventClient{transport: transport}, nil
}

func (c *EventClient) BuildDirectorySubscriptionInvocation(ctx context.Context, req EventsDirectorySubscriptionRequest) (InvocationDraft, error) {
	if err := c.requireReady(ctx); err != nil {
		return InvocationDraft{}, err
	}
	requestJSON, err := marshalEventsDirectorySubscriptionRequest(req)
	if err != nil {
		return InvocationDraft{}, err
	}
	raw, err := c.transport.BuildDirectorySubscriptionInvocation(ctx, requestJSON)
	if err != nil {
		return InvocationDraft{}, wrapEventsTransportError("events directory subscription invocation failed", err)
	}
	return NewInvocationDraftFromJSON(raw)
}

func (c *EventClient) SubscribeDirectory(ctx context.Context, req EventsDirectorySubscriptionRequest) (EventStream, error) {
	if err := c.requireReady(ctx); err != nil {
		return EventStream{}, err
	}
	requestJSON, err := marshalEventsDirectorySubscriptionRequest(req)
	if err != nil {
		return EventStream{}, err
	}
	raw, err := c.transport.SubscribeDirectory(ctx, requestJSON)
	if err != nil {
		return EventStream{}, wrapEventsTransportError("events subscribe directory failed", err)
	}
	return NewEventStreamFromJSON(raw)
}

func (c *EventClient) ProjectDirectoryEvent(ctx context.Context, input EventProjectionInput) (DirectoryEvent, error) {
	return c.projectFrame(ctx, input, validateEventProjectionInput, c.transport.ProjectDirectoryEvent, "events project directory event failed")
}

func (c *EventClient) ProjectDropReport(ctx context.Context, input EventDropReportInput) (EventDropReport, error) {
	return c.projectFrame(ctx, input, validateEventDropReportInput, c.transport.ProjectDropReport, "events project drop report failed")
}

func (c *EventClient) ProjectTerminal(ctx context.Context, input EventTerminalInput) (EventFrame, error) {
	return c.projectFrame(ctx, input, validateEventTerminalInput, c.transport.ProjectTerminal, "events project terminal failed")
}

func (c *EventClient) projectFrame(ctx context.Context, input any, validate func(any) error, fn func(context.Context, []byte) ([]byte, error), label string) (EventFrame, error) {
	if err := c.requireReady(ctx); err != nil {
		return EventFrame{}, err
	}
	if err := validate(input); err != nil {
		return EventFrame{}, err
	}
	requestJSON, err := json.Marshal(input)
	if err != nil {
		return EventFrame{}, invalidRuntimePayload(fmt.Sprintf("encode events projection input: %v", err), err)
	}
	raw, err := fn(ctx, requestJSON)
	if err != nil {
		return EventFrame{}, wrapEventsTransportError(label, err)
	}
	return NewEventFrameFromJSON(raw)
}

func (c *EventClient) requireReady(ctx context.Context) error {
	if c == nil || c.transport == nil {
		return invalidRuntimeClient("events client is not initialized")
	}
	if ctx == nil {
		return invalidRuntimeClient("context is required")
	}
	return nil
}

func NewEventStreamFromJSON(raw []byte) (EventStream, error) {
	var stream EventStream
	if err := json.Unmarshal(raw, &stream); err != nil {
		return EventStream{}, invalidRuntimePayload(fmt.Sprintf("decode event stream JSON: %v", err), err)
	}
	if stream.Stream != directoryEventStream || stream.State == "" || stream.Metadata == nil {
		return EventStream{}, invalidRuntimePayload("invalid event stream projection", nil)
	}
	return stream, nil
}

func NewEventFrameFromJSON(raw []byte) (EventFrame, error) {
	var frame EventFrame
	if err := json.Unmarshal(raw, &frame); err != nil {
		return EventFrame{}, invalidRuntimePayload(fmt.Sprintf("decode event frame JSON: %v", err), err)
	}
	if frame.Profile != eventsProfile || frame.Stream != directoryEventStream ||
		frame.Kind == "" || frame.EventID == "" || frame.ResumeToken == "" ||
		frame.OccurredUnixMS < 0 || frame.OccurredAt == "" || frame.Metadata == nil {
		return EventFrame{}, invalidRuntimePayload("invalid event frame projection", nil)
	}
	if err := validateEventCursor(frame.Cursor); err != nil {
		return EventFrame{}, err
	}
	if frame.Cursor.Token == "" {
		frame.Cursor.Token = frame.Cursor.ResumeToken()
	}
	if frame.DroppedCount < 0 {
		return EventFrame{}, invalidRuntimePayload("dropped_count must be non-negative", nil)
	}
	if strings.Contains(frame.Kind, "drop_report") && frame.DroppedCount == 0 {
		return EventFrame{}, invalidRuntimePayload("dropped_count must be greater than zero", nil)
	}
	if strings.Contains(frame.Kind, "terminal") && !frame.Terminal {
		return EventFrame{}, invalidRuntimePayload("terminal event frame must be terminal", nil)
	}
	return frame, nil
}

func marshalEventsDirectorySubscriptionRequest(req EventsDirectorySubscriptionRequest) ([]byte, error) {
	if err := validateEventsDirectorySubscriptionRequest(req); err != nil {
		return nil, err
	}
	requestJSON, err := json.Marshal(req)
	if err != nil {
		return nil, invalidRuntimePayload(fmt.Sprintf("encode events directory subscription request: %v", err), err)
	}
	return requestJSON, nil
}

func validateEventsDirectorySubscriptionRequest(req EventsDirectorySubscriptionRequest) error {
	if err := validateEventsCarrierBase(req.EventsCarrierBase); err != nil {
		return err
	}
	for field, value := range map[string]string{
		"realm":      req.Realm,
		"owner_ura":  req.OwnerURA,
		"device_ura": req.DeviceURA,
		"agent_ura":  req.AgentURA,
	} {
		if strings.TrimSpace(value) != value {
			return invalidRuntimePayload(field+" must not contain surrounding whitespace", nil)
		}
	}
	if req.ResumeCursor != nil {
		if err := validateEventCursor(*req.ResumeCursor); err != nil {
			return err
		}
	}
	if req.HeartbeatIntervalMS != 0 &&
		(req.HeartbeatIntervalMS < MinEventHeartbeatIntervalMS || req.HeartbeatIntervalMS > MaxEventHeartbeatIntervalMS) {
		return invalidRuntimePayload("heartbeat_interval_ms exceeds bounds", nil)
	}
	return nil
}

func validateEventsCarrierBase(base EventsCarrierBase) error {
	if base.CallerURA == "" || base.CalleeURA == "" || base.SubjectURA == "" ||
		base.DescriptorVersion == "" || base.NonceBase64 == "" || base.CausalContext == nil {
		return invalidRuntimePayload("complete events invocation carrier is required", nil)
	}
	return nil
}

func validateEventProjectionInput(input any) error {
	value := input.(EventProjectionInput)
	if err := validateEventCursor(value.Cursor); err != nil {
		return err
	}
	if value.Event == nil {
		return invalidRuntimePayload("directory event payload is required", nil)
	}
	return nil
}

func validateEventDropReportInput(input any) error {
	value := input.(EventDropReportInput)
	if err := validateEventCursor(value.Cursor); err != nil {
		return err
	}
	if value.OccurredUnixMS < 0 {
		return invalidRuntimePayload("occurred_unix_ms must be non-negative", nil)
	}
	if value.DroppedCount <= 0 {
		return invalidRuntimePayload("dropped_count must be greater than zero", nil)
	}
	return validateReconnectAfterMS(value.ReconnectAfterMS)
}

func validateEventTerminalInput(input any) error {
	value := input.(EventTerminalInput)
	if err := validateEventCursor(value.Cursor); err != nil {
		return err
	}
	if value.OccurredUnixMS < 0 {
		return invalidRuntimePayload("occurred_unix_ms must be non-negative", nil)
	}
	return validateReconnectAfterMS(value.ReconnectAfterMS)
}

func validateEventCursor(cursor EventCursor) error {
	if cursor.Stream != directoryEventStream {
		return invalidRuntimePayload("unsupported event stream", nil)
	}
	token := cursor.ResumeToken()
	if token == "" {
		return invalidRuntimePayload("event cursor token is required", nil)
	}
	if strings.ContainsAny(cursor.Stream, " \t\r\n") || strings.ContainsAny(token, " \t\r\n") {
		return invalidRuntimePayload("event cursor must not contain whitespace", nil)
	}
	if want := fmt.Sprintf("%s:%d", cursor.Stream, cursor.Sequence); token != want {
		return invalidRuntimePayload("event cursor token must match stream sequence", nil)
	}
	return nil
}

func validateReconnectAfterMS(value *int) error {
	if value == nil {
		return nil
	}
	if *value < 0 || *value > MaxEventHeartbeatIntervalMS {
		return invalidRuntimePayload("reconnect_after_ms exceeds bounds", nil)
	}
	return nil
}

func wrapEventsTransportError(message string, cause error) error {
	var sdkErr *SDKError
	if errors.As(cause, &sdkErr) {
		return sdkErr
	}
	return transportRuntimeError(message, cause)
}
