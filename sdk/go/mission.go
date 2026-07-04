package easynet

import (
	"context"
	"encoding/json"
	"errors"
	"fmt"
	"strings"
)

const missionProfile = "mission"

// MissionCarrierBase is the complete carrier context shared by Mission operations.
type MissionCarrierBase struct {
	CallerURA         string         `json:"caller_ura"`
	CalleeURA         string         `json:"callee_ura"`
	SubjectURA        string         `json:"subject_ura"`
	DescriptorVersion string         `json:"descriptor_version"`
	NonceBase64       string         `json:"nonce_base64"`
	CausalContext     map[string]any `json:"causal_context"`
	Metadata          map[string]any `json:"metadata,omitempty"`
}

// MissionRunRequest submits EAL source to daemon-owned Mission execution.
type MissionRunRequest struct {
	MissionCarrierBase
	Source string `json:"source"`
	Label  string `json:"label,omitempty"`
}

// MissionRunFileRequest submits an EAL source file to daemon-owned Mission execution.
type MissionRunFileRequest struct {
	MissionCarrierBase
	Path  string `json:"path"`
	Label string `json:"label,omitempty"`
}

// MissionTrackRequest tracks one daemon Mission run.
type MissionTrackRequest struct {
	MissionCarrierBase
	MissionID string `json:"mission_id"`
}

// MissionCancelRequest cancels one daemon Mission run.
type MissionCancelRequest struct {
	MissionCarrierBase
	MissionID string `json:"mission_id"`
}

// MissionEventListRequest requests mission timeline events from daemon-owned storage.
type MissionEventListRequest struct {
	MissionCarrierBase
	MissionID      string `json:"mission_id"`
	CursorSequence int64  `json:"cursor_sequence"`
	Limit          int    `json:"limit,omitempty"`
}

type MissionID string

// MissionRun is a Mission run projection. The status carries the durable state.
type MissionRun struct {
	Status MissionStatus `json:"status"`
}

type MissionCancelResult = MissionStatus

// MissionStatus is the sdk/schemas/mission-status.schema.json projection.
type MissionStatus struct {
	Profile            string                   `json:"profile"`
	Kind               string                   `json:"kind"`
	MissionID          string                   `json:"mission_id"`
	State              string                   `json:"state"`
	Terminal           bool                     `json:"terminal"`
	PartialFailures    int                      `json:"partial_failures"`
	Cancelled          bool                     `json:"cancelled"`
	ParentInvocationID *string                  `json:"parent_invocation_id"`
	ParentReceiptURA   *string                  `json:"parent_receipt_ura"`
	ParentInvocation   map[string]any           `json:"parent_invocation"`
	ChildInvocations   []MissionChildInvocation `json:"child_invocations"`
	ChildReceipts      []MissionChildReceipt    `json:"child_receipts"`
	OutputRefs         []MissionOutputRef       `json:"output_refs"`
	Error              *SDKError                `json:"error"`
	Metadata           map[string]any           `json:"metadata"`
}

type MissionChildInvocation struct {
	StepID        *string        `json:"step_id"`
	RequestID     *string        `json:"request_id"`
	TraceID       *string        `json:"trace_id"`
	Ability       *string        `json:"ability"`
	InvocationURA *string        `json:"invocation_ura"`
	CallerURA     *string        `json:"caller_ura"`
	CalleeURA     *string        `json:"callee_ura"`
	SubjectURA    *string        `json:"subject_ura"`
	MetadataState *string        `json:"metadata_state"`
	LedgerState   any            `json:"ledger_state"`
	Receipt       map[string]any `json:"receipt"`
}

type MissionChildReceipt struct {
	StepID        *string `json:"step_id"`
	InvocationURA *string `json:"invocation_ura"`
	ReceiptURA    string  `json:"receipt_ura"`
	ReceiptHash   string  `json:"receipt_hash"`
}

type MissionOutputRef struct {
	Kind     string         `json:"kind"`
	Path     string         `json:"path,omitempty"`
	Metadata map[string]any `json:"metadata,omitempty"`
}

// MissionEvent is one daemon Mission timeline event projection.
type MissionEvent struct {
	Profile        string         `json:"profile"`
	Kind           string         `json:"kind"`
	MissionID      string         `json:"mission_id"`
	Sequence       int64          `json:"sequence"`
	EventType      string         `json:"event_type"`
	OccurredUnixMS int64          `json:"occurred_unix_ms"`
	Terminal       bool           `json:"terminal"`
	Payload        any            `json:"payload"`
	Receipt        map[string]any `json:"receipt"`
	Metadata       map[string]any `json:"metadata"`
}

// MissionEventPage is a replay page over daemon Mission timeline events.
type MissionEventPage struct {
	Profile            string         `json:"profile"`
	Kind               string         `json:"kind"`
	MissionID          string         `json:"mission_id"`
	CursorSequence     int64          `json:"cursor_sequence"`
	NextCursorSequence int64          `json:"next_cursor_sequence"`
	HasMore            bool           `json:"has_more"`
	DroppedCount       int64          `json:"dropped_count"`
	Events             []MissionEvent `json:"events"`
	Metadata           map[string]any `json:"metadata"`
}

// MissionTransport supplies daemon Mission operations behind the facade.
type MissionTransport interface {
	BuildRunEALInvocation(ctx context.Context, requestJSON []byte) ([]byte, error)
	BuildRunFileInvocation(ctx context.Context, requestJSON []byte) ([]byte, error)
	BuildTrackInvocation(ctx context.Context, requestJSON []byte) ([]byte, error)
	BuildCancelInvocation(ctx context.Context, requestJSON []byte) ([]byte, error)
	RunEAL(ctx context.Context, requestJSON []byte) ([]byte, error)
	RunFile(ctx context.Context, requestJSON []byte) ([]byte, error)
	Track(ctx context.Context, requestJSON []byte) ([]byte, error)
	Cancel(ctx context.Context, requestJSON []byte) ([]byte, error)
	Events(ctx context.Context, requestJSON []byte) ([]byte, error)
}

// MissionTransportFunc adapts functions into a MissionTransport.
type MissionTransportFunc struct {
	BuildRunEALInvocationFunc  func(ctx context.Context, requestJSON []byte) ([]byte, error)
	BuildRunFileInvocationFunc func(ctx context.Context, requestJSON []byte) ([]byte, error)
	BuildTrackInvocationFunc   func(ctx context.Context, requestJSON []byte) ([]byte, error)
	BuildCancelInvocationFunc  func(ctx context.Context, requestJSON []byte) ([]byte, error)
	RunEALFunc                 func(ctx context.Context, requestJSON []byte) ([]byte, error)
	RunFileFunc                func(ctx context.Context, requestJSON []byte) ([]byte, error)
	TrackFunc                  func(ctx context.Context, requestJSON []byte) ([]byte, error)
	CancelFunc                 func(ctx context.Context, requestJSON []byte) ([]byte, error)
	EventsFunc                 func(ctx context.Context, requestJSON []byte) ([]byte, error)
}

func (f MissionTransportFunc) BuildRunEALInvocation(ctx context.Context, requestJSON []byte) ([]byte, error) {
	if f.BuildRunEALInvocationFunc == nil {
		return nil, invalidRuntimeClient("mission run invocation transport function is required")
	}
	return f.BuildRunEALInvocationFunc(ctx, requestJSON)
}

func (f MissionTransportFunc) BuildRunFileInvocation(ctx context.Context, requestJSON []byte) ([]byte, error) {
	if f.BuildRunFileInvocationFunc == nil {
		return nil, invalidRuntimeClient("mission run-file invocation transport function is required")
	}
	return f.BuildRunFileInvocationFunc(ctx, requestJSON)
}

func (f MissionTransportFunc) BuildTrackInvocation(ctx context.Context, requestJSON []byte) ([]byte, error) {
	if f.BuildTrackInvocationFunc == nil {
		return nil, invalidRuntimeClient("mission track invocation transport function is required")
	}
	return f.BuildTrackInvocationFunc(ctx, requestJSON)
}

func (f MissionTransportFunc) BuildCancelInvocation(ctx context.Context, requestJSON []byte) ([]byte, error) {
	if f.BuildCancelInvocationFunc == nil {
		return nil, invalidRuntimeClient("mission cancel invocation transport function is required")
	}
	return f.BuildCancelInvocationFunc(ctx, requestJSON)
}

func (f MissionTransportFunc) RunEAL(ctx context.Context, requestJSON []byte) ([]byte, error) {
	if f.RunEALFunc == nil {
		return nil, invalidRuntimeClient("mission run transport function is required")
	}
	return f.RunEALFunc(ctx, requestJSON)
}

func (f MissionTransportFunc) RunFile(ctx context.Context, requestJSON []byte) ([]byte, error) {
	if f.RunFileFunc == nil {
		return nil, invalidRuntimeClient("mission run-file transport function is required")
	}
	return f.RunFileFunc(ctx, requestJSON)
}

func (f MissionTransportFunc) Track(ctx context.Context, requestJSON []byte) ([]byte, error) {
	if f.TrackFunc == nil {
		return nil, invalidRuntimeClient("mission track transport function is required")
	}
	return f.TrackFunc(ctx, requestJSON)
}

func (f MissionTransportFunc) Cancel(ctx context.Context, requestJSON []byte) ([]byte, error) {
	if f.CancelFunc == nil {
		return nil, invalidRuntimeClient("mission cancel transport function is required")
	}
	return f.CancelFunc(ctx, requestJSON)
}

func (f MissionTransportFunc) Events(ctx context.Context, requestJSON []byte) ([]byte, error) {
	if f.EventsFunc == nil {
		return nil, invalidRuntimeClient("mission events transport function is required")
	}
	return f.EventsFunc(ctx, requestJSON)
}

// MissionClient is the Mission profile facade.
type MissionClient struct {
	transport MissionTransport
	lifecycle profileClientLifecycle
}

func NewMissionClient(transport MissionTransport) (*MissionClient, error) {
	if transport == nil {
		return nil, invalidRuntimeClient("mission transport is required")
	}
	return &MissionClient{transport: transport}, nil
}

func (c *MissionClient) BuildRunEALInvocation(ctx context.Context, req MissionRunRequest) (InvocationDraft, error) {
	return c.buildInvocation(ctx, req, validateMissionRunRequest, c.transport.BuildRunEALInvocation, "mission run invocation failed")
}

func (c *MissionClient) BuildRunFileInvocation(ctx context.Context, req MissionRunFileRequest) (InvocationDraft, error) {
	return c.buildInvocation(ctx, req, validateMissionRunFileRequest, c.transport.BuildRunFileInvocation, "mission run-file invocation failed")
}

func (c *MissionClient) BuildTrackInvocation(ctx context.Context, req MissionTrackRequest) (InvocationDraft, error) {
	return c.buildInvocation(ctx, req, validateMissionTrackRequest, c.transport.BuildTrackInvocation, "mission track invocation failed")
}

func (c *MissionClient) BuildCancelInvocation(ctx context.Context, req MissionCancelRequest) (InvocationDraft, error) {
	return c.buildInvocation(ctx, req, validateMissionCancelRequest, c.transport.BuildCancelInvocation, "mission cancel invocation failed")
}

func (c *MissionClient) RunEAL(ctx context.Context, req MissionRunRequest) (MissionRun, error) {
	status, err := c.statusOperation(ctx, req, validateMissionRunRequest, c.transport.RunEAL, "mission run failed")
	return MissionRun{Status: status}, err
}

func (c *MissionClient) RunFile(ctx context.Context, path string, opts MissionRunFileRequest) (MissionRun, error) {
	opts.Path = path
	status, err := c.statusOperation(ctx, opts, validateMissionRunFileRequest, c.transport.RunFile, "mission run-file failed")
	return MissionRun{Status: status}, err
}

func (c *MissionClient) Track(ctx context.Context, req MissionTrackRequest) (MissionStatus, error) {
	return c.statusOperation(ctx, req, validateMissionTrackRequest, c.transport.Track, "mission track failed")
}

func (c *MissionClient) Cancel(ctx context.Context, req MissionCancelRequest) (MissionCancelResult, error) {
	return c.statusOperation(ctx, req, validateMissionCancelRequest, c.transport.Cancel, "mission cancel failed")
}

func (c *MissionClient) Events(ctx context.Context, req MissionEventListRequest) (MissionEventPage, error) {
	if err := c.requireReady(ctx); err != nil {
		return MissionEventPage{}, err
	}
	requestJSON, err := marshalMissionRequest(req, validateMissionEventListRequest)
	if err != nil {
		return MissionEventPage{}, err
	}
	raw, err := c.transport.Events(ctx, requestJSON)
	if err != nil {
		return MissionEventPage{}, wrapMissionTransportError("mission events failed", err)
	}
	return NewMissionEventPageFromJSON(raw)
}

func (c *MissionClient) buildInvocation(ctx context.Context, req any, validate func(any) error, fn func(context.Context, []byte) ([]byte, error), label string) (InvocationDraft, error) {
	if err := c.requireReady(ctx); err != nil {
		return InvocationDraft{}, err
	}
	requestJSON, err := marshalMissionRequest(req, validate)
	if err != nil {
		return InvocationDraft{}, err
	}
	raw, err := fn(ctx, requestJSON)
	if err != nil {
		return InvocationDraft{}, wrapMissionTransportError(label, err)
	}
	return NewInvocationDraftFromJSON(raw)
}

func (c *MissionClient) statusOperation(ctx context.Context, req any, validate func(any) error, fn func(context.Context, []byte) ([]byte, error), label string) (MissionStatus, error) {
	if err := c.requireReady(ctx); err != nil {
		return MissionStatus{}, err
	}
	requestJSON, err := marshalMissionRequest(req, validate)
	if err != nil {
		return MissionStatus{}, err
	}
	raw, err := fn(ctx, requestJSON)
	if err != nil {
		return MissionStatus{}, wrapMissionTransportError(label, err)
	}
	return NewMissionStatusFromJSON(raw)
}

func (c *MissionClient) Close(ctx context.Context) error {
	if c == nil || c.transport == nil {
		return invalidRuntimeClient("mission client is not initialized")
	}
	return c.lifecycle.Close(ctx, c.transport, "mission")
}

func (c *MissionClient) requireReady(ctx context.Context) error {
	if c == nil || c.transport == nil {
		return invalidRuntimeClient("mission client is not initialized")
	}
	return c.lifecycle.RequireOpen(ctx, "mission")
}

func NewMissionStatusFromJSON(raw []byte) (MissionStatus, error) {
	var dto struct {
		Profile            string                   `json:"profile"`
		Kind               string                   `json:"kind"`
		MissionID          string                   `json:"mission_id"`
		State              string                   `json:"state"`
		Terminal           bool                     `json:"terminal"`
		PartialFailures    int                      `json:"partial_failures"`
		Cancelled          bool                     `json:"cancelled"`
		ParentInvocationID *string                  `json:"parent_invocation_id"`
		ParentReceiptURA   *string                  `json:"parent_receipt_ura"`
		ParentInvocation   map[string]any           `json:"parent_invocation"`
		ChildInvocations   []MissionChildInvocation `json:"child_invocations"`
		ChildReceipts      []MissionChildReceipt    `json:"child_receipts"`
		OutputRefs         []MissionOutputRef       `json:"output_refs"`
		Error              json.RawMessage          `json:"error"`
		Metadata           map[string]any           `json:"metadata"`
	}
	if err := json.Unmarshal(raw, &dto); err != nil {
		return MissionStatus{}, invalidRuntimePayload(fmt.Sprintf("decode mission status JSON: %v", err), err)
	}
	if dto.Profile != missionProfile || dto.Kind != "mission_status" || dto.MissionID == "" ||
		dto.State == "" || dto.PartialFailures < 0 || dto.ChildInvocations == nil ||
		dto.ChildReceipts == nil || dto.OutputRefs == nil || dto.Metadata == nil {
		return MissionStatus{}, invalidRuntimePayload("invalid mission status projection", nil)
	}
	for _, receipt := range dto.ChildReceipts {
		if receipt.ReceiptURA == "" || receipt.ReceiptHash == "" {
			return MissionStatus{}, invalidRuntimePayload("invalid mission child receipt projection", nil)
		}
	}
	for _, ref := range dto.OutputRefs {
		if ref.Kind == "" {
			return MissionStatus{}, invalidRuntimePayload("invalid mission output ref projection", nil)
		}
	}
	var sdkErr *SDKError
	if len(dto.Error) > 0 && string(dto.Error) != "null" {
		decoded, err := DecodeDaemonErrorJSON(dto.Error)
		if err != nil {
			return MissionStatus{}, err
		}
		sdkErr = decoded
	}
	return MissionStatus{
		Profile:            dto.Profile,
		Kind:               dto.Kind,
		MissionID:          dto.MissionID,
		State:              dto.State,
		Terminal:           dto.Terminal,
		PartialFailures:    dto.PartialFailures,
		Cancelled:          dto.Cancelled,
		ParentInvocationID: dto.ParentInvocationID,
		ParentReceiptURA:   dto.ParentReceiptURA,
		ParentInvocation:   dto.ParentInvocation,
		ChildInvocations:   dto.ChildInvocations,
		ChildReceipts:      dto.ChildReceipts,
		OutputRefs:         dto.OutputRefs,
		Error:              sdkErr,
		Metadata:           dto.Metadata,
	}, nil
}

func NewMissionEventPageFromJSON(raw []byte) (MissionEventPage, error) {
	var dto struct {
		Profile            string         `json:"profile"`
		Kind               string         `json:"kind"`
		MissionID          string         `json:"mission_id"`
		CursorSequence     int64          `json:"cursor_sequence"`
		NextCursorSequence int64          `json:"next_cursor_sequence"`
		HasMore            bool           `json:"has_more"`
		DroppedCount       int64          `json:"dropped_count"`
		Events             []MissionEvent `json:"events"`
		Metadata           map[string]any `json:"metadata"`
	}
	if err := json.Unmarshal(raw, &dto); err != nil {
		return MissionEventPage{}, invalidRuntimePayload(fmt.Sprintf("decode mission event page JSON: %v", err), err)
	}
	if dto.Profile != missionProfile || dto.Kind != "mission_event_page" || dto.MissionID == "" ||
		dto.CursorSequence < 0 || dto.NextCursorSequence < dto.CursorSequence || dto.DroppedCount < 0 ||
		dto.Events == nil || dto.Metadata == nil {
		return MissionEventPage{}, invalidRuntimePayload("invalid mission event page projection", nil)
	}
	var previousSequence int64
	hasPrevious := false
	for index := range dto.Events {
		event := &dto.Events[index]
		if event.Profile != missionProfile || event.Kind != "mission_event" || event.MissionID != dto.MissionID ||
			event.Sequence < 0 || event.EventType == "" || event.OccurredUnixMS < 0 || event.Metadata == nil {
			return MissionEventPage{}, invalidRuntimePayload("invalid mission event projection", nil)
		}
		if hasPrevious && event.Sequence <= previousSequence {
			return MissionEventPage{}, invalidRuntimePayload("mission events must be strictly ordered by sequence", nil)
		}
		if event.Terminal && !missionEventTypeIsTerminal(event.EventType) {
			return MissionEventPage{}, invalidRuntimePayload("terminal mission event has non-terminal event_type", nil)
		}
		previousSequence = event.Sequence
		hasPrevious = true
		if event.Receipt == nil {
			event.Receipt = map[string]any{}
		}
	}
	return MissionEventPage{
		Profile:            dto.Profile,
		Kind:               dto.Kind,
		MissionID:          dto.MissionID,
		CursorSequence:     dto.CursorSequence,
		NextCursorSequence: dto.NextCursorSequence,
		HasMore:            dto.HasMore,
		DroppedCount:       dto.DroppedCount,
		Events:             dto.Events,
		Metadata:           dto.Metadata,
	}, nil
}

func marshalMissionRequest(req any, validate func(any) error) ([]byte, error) {
	if err := validate(req); err != nil {
		return nil, err
	}
	requestJSON, err := json.Marshal(req)
	if err != nil {
		return nil, invalidRuntimePayload(fmt.Sprintf("encode mission request: %v", err), err)
	}
	return requestJSON, nil
}

func validateMissionRunRequest(req any) error {
	value := req.(MissionRunRequest)
	if err := validateMissionCarrierBase(value.MissionCarrierBase); err != nil {
		return err
	}
	if value.Source == "" {
		return invalidRuntimePayload("mission source is required", nil)
	}
	return nil
}

func validateMissionRunFileRequest(req any) error {
	value := req.(MissionRunFileRequest)
	if err := validateMissionCarrierBase(value.MissionCarrierBase); err != nil {
		return err
	}
	if value.Path == "" || !strings.HasPrefix(value.Path, "/") {
		return invalidRuntimePayload("absolute mission file path is required", nil)
	}
	return nil
}

func validateMissionTrackRequest(req any) error {
	value := req.(MissionTrackRequest)
	if err := validateMissionCarrierBase(value.MissionCarrierBase); err != nil {
		return err
	}
	return validateMissionID(value.MissionID)
}

func validateMissionCancelRequest(req any) error {
	value := req.(MissionCancelRequest)
	if err := validateMissionCarrierBase(value.MissionCarrierBase); err != nil {
		return err
	}
	return validateMissionID(value.MissionID)
}

func validateMissionEventListRequest(req any) error {
	value := req.(MissionEventListRequest)
	if err := validateMissionCarrierBase(value.MissionCarrierBase); err != nil {
		return err
	}
	if err := validateMissionID(value.MissionID); err != nil {
		return err
	}
	if value.CursorSequence < 0 {
		return invalidRuntimePayload("mission event cursor_sequence must be non-negative", nil)
	}
	if value.Limit < 0 {
		return invalidRuntimePayload("mission event limit must be non-negative", nil)
	}
	if value.Limit > 1000 {
		return invalidRuntimePayload("mission event limit exceeds bounds", nil)
	}
	return nil
}

func validateMissionCarrierBase(base MissionCarrierBase) error {
	if base.CallerURA == "" || base.CalleeURA == "" || base.SubjectURA == "" ||
		base.DescriptorVersion == "" || base.NonceBase64 == "" || base.CausalContext == nil {
		return invalidRuntimePayload("complete mission invocation carrier is required", nil)
	}
	return nil
}

func validateMissionID(missionID string) error {
	if missionID == "" {
		return invalidRuntimePayload("mission_id is required", nil)
	}
	if strings.Contains(missionID, "/") || strings.Contains(missionID, "\\") || strings.Contains(missionID, "://") {
		return invalidRuntimePayload("mission_id must not be path-like", nil)
	}
	return nil
}

func missionEventTypeIsTerminal(eventType string) bool {
	switch eventType {
	case "completed", "failed", "cancelled", "canceled":
		return true
	default:
		return false
	}
}

func wrapMissionTransportError(message string, cause error) error {
	var sdkErr *SDKError
	if errors.As(cause, &sdkErr) {
		return sdkErr
	}
	return transportRuntimeError(message, cause)
}
