package easynet

import (
	"context"
	"encoding/json"
	"errors"
	"fmt"
	"sync"
)

// RuntimeTransport is the narrow Runtime Core invocation transport seam.
type RuntimeTransport interface {
	Invoke(ctx context.Context, draftJSON []byte) ([]byte, error)
	OpenStream(ctx context.Context, draftJSON []byte) (StreamTransport, []byte, error)
	OpenBidi(ctx context.Context, draftJSON []byte, streamsJSON []byte) (BidiTransport, []byte, error)
	Prepare(ctx context.Context, draftJSON []byte, optionsJSON []byte) ([]byte, error)
	SubmitSigned(ctx context.Context, signedJSON []byte) ([]byte, error)
	AwaitHandle(ctx context.Context, handleID uint64) ([]byte, error)
	CancelHandle(ctx context.Context, handleID uint64, reason string) ([]byte, error)
	HandleEvents(ctx context.Context, handleID uint64) ([]byte, error)
	FreeHandle(ctx context.Context, handleID uint64) error
	Close(ctx context.Context) error
}

// RuntimeTransportFunc adapts functions into a RuntimeTransport.
type RuntimeTransportFunc struct {
	InvokeFunc       func(ctx context.Context, draftJSON []byte) ([]byte, error)
	OpenStreamFunc   func(ctx context.Context, draftJSON []byte) (StreamTransport, []byte, error)
	OpenBidiFunc     func(ctx context.Context, draftJSON []byte, streamsJSON []byte) (BidiTransport, []byte, error)
	PrepareFunc      func(ctx context.Context, draftJSON []byte, optionsJSON []byte) ([]byte, error)
	SubmitSignedFunc func(ctx context.Context, signedJSON []byte) ([]byte, error)
	AwaitHandleFunc  func(ctx context.Context, handleID uint64) ([]byte, error)
	CancelHandleFunc func(ctx context.Context, handleID uint64, reason string) ([]byte, error)
	HandleEventsFunc func(ctx context.Context, handleID uint64) ([]byte, error)
	FreeHandleFunc   func(ctx context.Context, handleID uint64) error
	CloseFunc        func(ctx context.Context) error
}

func (f RuntimeTransportFunc) Invoke(ctx context.Context, draftJSON []byte) ([]byte, error) {
	if f.InvokeFunc == nil {
		return nil, invalidRuntimeClient("runtime invoke transport function is required")
	}
	return f.InvokeFunc(ctx, draftJSON)
}

func (f RuntimeTransportFunc) OpenStream(ctx context.Context, draftJSON []byte) (StreamTransport, []byte, error) {
	if f.OpenStreamFunc == nil {
		return nil, nil, invalidRuntimeClient("runtime open-stream transport function is required")
	}
	return f.OpenStreamFunc(ctx, draftJSON)
}

func (f RuntimeTransportFunc) OpenBidi(ctx context.Context, draftJSON []byte, streamsJSON []byte) (BidiTransport, []byte, error) {
	if f.OpenBidiFunc == nil {
		return nil, nil, invalidRuntimeClient("runtime open-bidi transport function is required")
	}
	return f.OpenBidiFunc(ctx, draftJSON, streamsJSON)
}

func (f RuntimeTransportFunc) Prepare(ctx context.Context, draftJSON []byte, optionsJSON []byte) ([]byte, error) {
	if f.PrepareFunc == nil {
		return nil, invalidRuntimeClient("runtime prepare transport function is required")
	}
	return f.PrepareFunc(ctx, draftJSON, optionsJSON)
}

func (f RuntimeTransportFunc) SubmitSigned(ctx context.Context, signedJSON []byte) ([]byte, error) {
	if f.SubmitSignedFunc == nil {
		return nil, invalidRuntimeClient("runtime submit-signed transport function is required")
	}
	return f.SubmitSignedFunc(ctx, signedJSON)
}

func (f RuntimeTransportFunc) AwaitHandle(ctx context.Context, handleID uint64) ([]byte, error) {
	if f.AwaitHandleFunc == nil {
		return nil, invalidRuntimeClient("runtime await-handle transport function is required")
	}
	return f.AwaitHandleFunc(ctx, handleID)
}

func (f RuntimeTransportFunc) CancelHandle(ctx context.Context, handleID uint64, reason string) ([]byte, error) {
	if f.CancelHandleFunc == nil {
		return nil, invalidRuntimeClient("runtime cancel-handle transport function is required")
	}
	return f.CancelHandleFunc(ctx, handleID, reason)
}

func (f RuntimeTransportFunc) HandleEvents(ctx context.Context, handleID uint64) ([]byte, error) {
	if f.HandleEventsFunc == nil {
		return nil, invalidRuntimeClient("runtime handle-events transport function is required")
	}
	return f.HandleEventsFunc(ctx, handleID)
}

func (f RuntimeTransportFunc) FreeHandle(ctx context.Context, handleID uint64) error {
	if f.FreeHandleFunc == nil {
		return invalidRuntimeClient("runtime free-handle transport function is required")
	}
	return f.FreeHandleFunc(ctx, handleID)
}

func (f RuntimeTransportFunc) Close(ctx context.Context) error {
	if f.CloseFunc == nil {
		return nil
	}
	return f.CloseFunc(ctx)
}

// RuntimeClient is the Runtime Core invocation facade.
type RuntimeClient struct {
	mu        sync.Mutex
	transport RuntimeTransport
	closed    bool
}

// NewRuntimeClient creates a Runtime Core facade over a daemon invocation transport.
func NewRuntimeClient(transport RuntimeTransport) (*RuntimeClient, error) {
	if transport == nil {
		return nil, &SDKError{
			Code:      ErrInvalidArgument,
			Stage:     "sdk",
			Retry:     RetryNever,
			Retryable: RetryableForHint(RetryNever),
			Message:   "runtime transport is required",
		}
	}
	return &RuntimeClient{transport: transport}, nil
}

func (c *RuntimeClient) runtimeTransport(ctx context.Context) (RuntimeTransport, error) {
	if c == nil {
		return nil, invalidRuntimeClient("runtime client is not initialized")
	}
	if ctx == nil {
		return nil, invalidRuntimeClient("context is required")
	}
	c.mu.Lock()
	defer c.mu.Unlock()
	if c.closed {
		return nil, invalidRuntimeClient("runtime client is closed")
	}
	if c.transport == nil {
		return nil, invalidRuntimeClient("runtime client is not initialized")
	}
	return c.transport, nil
}

// PrepareOptions are daemon-owned prepare policy knobs.
type PrepareOptions struct {
	ResolveDescriptor bool  `json:"resolve_descriptor,omitempty"`
	FillNonce         bool  `json:"fill_nonce,omitempty"`
	RequireUserSig    bool  `json:"require_user_sig,omitempty"`
	ExpiresInMS       int64 `json:"expires_in_ms,omitempty"`
}

// Invoke submits a complete Invocation tuple and decodes the daemon result projection.
func (c *RuntimeClient) Invoke(ctx context.Context, draft InvocationDraft) (InvocationResult, error) {
	transport, err := c.runtimeTransport(ctx)
	if err != nil {
		return InvocationResult{}, err
	}
	draftJSON, err := json.Marshal(draft)
	if err != nil {
		return InvocationResult{}, invalidRuntimePayload(fmt.Sprintf("encode invocation draft: %v", err), err)
	}
	raw, err := transport.Invoke(ctx, draftJSON)
	if err != nil {
		var sdkErr *SDKError
		if errors.As(err, &sdkErr) {
			return InvocationResult{}, sdkErr
		}
		return InvocationResult{}, transportRuntimeError("invoke transport failed", err)
	}
	return NewInvocationResultFromJSON(raw)
}

// InvokeStream opens a server stream over a complete Invocation tuple.
func (c *RuntimeClient) InvokeStream(ctx context.Context, draft InvocationDraft) (*StreamHandle, error) {
	transport, err := c.runtimeTransport(ctx)
	if err != nil {
		return nil, err
	}
	draftJSON, err := json.Marshal(draft)
	if err != nil {
		return nil, invalidRuntimePayload(fmt.Sprintf("encode invocation draft: %v", err), err)
	}
	streamTransport, rawOpen, err := transport.OpenStream(ctx, draftJSON)
	if err != nil {
		var sdkErr *SDKError
		if errors.As(err, &sdkErr) {
			return nil, sdkErr
		}
		return nil, transportRuntimeError("open stream transport failed", err)
	}
	return NewStreamHandleFromJSON(streamTransport, rawOpen)
}

// OpenBidi opens a bidirectional session over a complete Invocation tuple.
func (c *RuntimeClient) OpenBidi(ctx context.Context, draft InvocationDraft, streams []BidiStreamDescriptor) (*BidiSession, error) {
	transport, err := c.runtimeTransport(ctx)
	if err != nil {
		return nil, err
	}
	draftJSON, err := json.Marshal(draft)
	if err != nil {
		return nil, invalidRuntimePayload(fmt.Sprintf("encode invocation draft: %v", err), err)
	}
	streamsJSON, err := json.Marshal(streams)
	if err != nil {
		return nil, invalidRuntimePayload(fmt.Sprintf("encode bidi stream descriptors: %v", err), err)
	}
	bidiTransport, rawOpen, err := transport.OpenBidi(ctx, draftJSON, streamsJSON)
	if err != nil {
		var sdkErr *SDKError
		if errors.As(err, &sdkErr) {
			return nil, sdkErr
		}
		return nil, transportRuntimeError("open bidi transport failed", err)
	}
	return NewBidiSessionFromJSON(bidiTransport, rawOpen)
}

// Prepare delegates canonical material generation to the daemon transport.
func (c *RuntimeClient) Prepare(ctx context.Context, draft InvocationDraft, opts PrepareOptions) (PreparedInvocation, SigningMaterial, error) {
	transport, err := c.runtimeTransport(ctx)
	if err != nil {
		return PreparedInvocation{}, SigningMaterial{}, err
	}
	draftJSON, err := json.Marshal(draft)
	if err != nil {
		return PreparedInvocation{}, SigningMaterial{}, invalidRuntimePayload(fmt.Sprintf("encode invocation draft: %v", err), err)
	}
	optionsJSON, err := json.Marshal(opts)
	if err != nil {
		return PreparedInvocation{}, SigningMaterial{}, invalidRuntimePayload(fmt.Sprintf("encode prepare options: %v", err), err)
	}
	raw, err := transport.Prepare(ctx, draftJSON, optionsJSON)
	if err != nil {
		var sdkErr *SDKError
		if errors.As(err, &sdkErr) {
			return PreparedInvocation{}, SigningMaterial{}, sdkErr
		}
		return PreparedInvocation{}, SigningMaterial{}, transportRuntimeError("prepare transport failed", err)
	}
	prepared, err := NewPreparedInvocationFromJSON(raw)
	if err != nil {
		return PreparedInvocation{}, SigningMaterial{}, err
	}
	return prepared, prepared.SigningMaterial(), nil
}

// PrepareBuilder inspects a complete builder, prepares canonical signing
// material, and consumes the builder only after prepare succeeds.
func (c *RuntimeClient) PrepareBuilder(ctx context.Context, builder *InvocationBuilder, opts PrepareOptions) (PreparedInvocation, SigningMaterial, error) {
	if builder == nil {
		return PreparedInvocation{}, SigningMaterial{}, invalidRuntimePayload("invocation builder is required", nil)
	}
	draft, err := builder.Inspect()
	if err != nil {
		return PreparedInvocation{}, SigningMaterial{}, err
	}
	prepared, material, err := c.Prepare(ctx, draft, opts)
	if err != nil {
		return PreparedInvocation{}, SigningMaterial{}, err
	}
	if err := builder.consume(); err != nil {
		return PreparedInvocation{}, SigningMaterial{}, err
	}
	return prepared, material, nil
}

// SubmitSigned submits an immutable signed envelope and returns an observation handle.
func (c *RuntimeClient) SubmitSigned(ctx context.Context, signed SignedInvocation) (InvocationHandle, error) {
	transport, err := c.runtimeTransport(ctx)
	if err != nil {
		return InvocationHandle{}, err
	}
	if !signed.SubmitReady() {
		return InvocationHandle{}, invalidRuntimePayload("signed invocation is not submit-ready", nil)
	}
	signedJSON, err := json.Marshal(signed)
	if err != nil {
		return InvocationHandle{}, invalidRuntimePayload(fmt.Sprintf("encode signed invocation: %v", err), err)
	}
	raw, err := transport.SubmitSigned(ctx, signedJSON)
	if err != nil {
		var sdkErr *SDKError
		if errors.As(err, &sdkErr) {
			return InvocationHandle{}, sdkErr
		}
		return InvocationHandle{}, transportRuntimeError("submit signed transport failed", err)
	}
	return NewInvocationHandleFromJSON(raw)
}

// Await waits for a submitted invocation handle to reach a terminal result.
func (c *RuntimeClient) Await(ctx context.Context, handle InvocationHandle) (InvocationResult, error) {
	transport, err := c.runtimeTransport(ctx)
	if err != nil {
		return InvocationResult{}, err
	}
	if handle.HandleID() == 0 {
		return InvocationResult{}, invalidRuntimePayload("handle_id is required", nil)
	}
	raw, err := transport.AwaitHandle(ctx, handle.HandleID())
	if err != nil {
		var sdkErr *SDKError
		if errors.As(err, &sdkErr) {
			return InvocationResult{}, sdkErr
		}
		return InvocationResult{}, transportRuntimeError("await handle transport failed", err)
	}
	return NewInvocationResultFromJSON(raw)
}

// Cancel requests terminal cancellation for a submitted invocation handle.
func (c *RuntimeClient) Cancel(ctx context.Context, handle InvocationHandle, reason string) (InvocationCancel, error) {
	transport, err := c.runtimeTransport(ctx)
	if err != nil {
		return InvocationCancel{}, err
	}
	if handle.HandleID() == 0 {
		return InvocationCancel{}, invalidRuntimePayload("handle_id is required", nil)
	}
	raw, err := transport.CancelHandle(ctx, handle.HandleID(), reason)
	if err != nil {
		var sdkErr *SDKError
		if errors.As(err, &sdkErr) {
			return InvocationCancel{}, sdkErr
		}
		return InvocationCancel{}, transportRuntimeError("cancel handle transport failed", err)
	}
	return NewInvocationCancelFromJSON(raw)
}

// Events returns the current event snapshot for a submitted invocation handle.
func (c *RuntimeClient) Events(ctx context.Context, handle InvocationHandle) (InvocationHandle, error) {
	transport, err := c.runtimeTransport(ctx)
	if err != nil {
		return InvocationHandle{}, err
	}
	if handle.HandleID() == 0 {
		return InvocationHandle{}, invalidRuntimePayload("handle_id is required", nil)
	}
	raw, err := transport.HandleEvents(ctx, handle.HandleID())
	if err != nil {
		var sdkErr *SDKError
		if errors.As(err, &sdkErr) {
			return InvocationHandle{}, sdkErr
		}
		return InvocationHandle{}, transportRuntimeError("handle events transport failed", err)
	}
	return NewInvocationHandleFromJSON(raw)
}

// CloseHandle releases daemon-side observation state for a submitted invocation handle.
func (c *RuntimeClient) CloseHandle(ctx context.Context, handle InvocationHandle) error {
	transport, err := c.runtimeTransport(ctx)
	if err != nil {
		return err
	}
	if handle.HandleID() == 0 {
		return invalidRuntimePayload("handle_id is required", nil)
	}
	if err := transport.FreeHandle(ctx, handle.HandleID()); err != nil {
		var sdkErr *SDKError
		if errors.As(err, &sdkErr) {
			return sdkErr
		}
		return transportRuntimeError("free handle transport failed", err)
	}
	return nil
}

// Close releases the Runtime Core client transport without stopping the daemon.
func (c *RuntimeClient) Close(ctx context.Context) error {
	if c == nil {
		return invalidRuntimeClient("runtime client is not initialized")
	}
	if ctx == nil {
		return invalidRuntimeClient("context is required")
	}
	c.mu.Lock()
	if c.closed {
		c.mu.Unlock()
		return nil
	}
	transport := c.transport
	c.closed = true
	c.transport = nil
	c.mu.Unlock()

	if transport == nil {
		return invalidRuntimeClient("runtime client is not initialized")
	}
	if err := transport.Close(ctx); err != nil {
		var sdkErr *SDKError
		if errors.As(err, &sdkErr) {
			return sdkErr
		}
		return transportRuntimeError("runtime close transport failed", err)
	}
	return nil
}

// InvocationResult is the unary invocation terminal result projection.
type InvocationResult struct {
	ok                bool
	tuple             InvocationDraft
	terminalState     string
	outputContentType string
	outputBase64      string
	outputJSON        json.RawMessage
	selectedNodeID    string
	schedulingReason  string
	elapsedMS         int64
	receipt           json.RawMessage
	failure           *InvocationFailure
}

// InvocationFailure is the runtime error embedded in a terminal invocation result.
type InvocationFailure struct {
	code      string
	stage     string
	message   string
	retryable bool
}

func (r InvocationResult) OK() bool {
	return r.ok
}

func (r InvocationResult) Tuple() InvocationDraft {
	return r.tuple
}

func (r InvocationResult) TerminalState() string {
	return r.terminalState
}

func (r InvocationResult) OutputContentType() string {
	return r.outputContentType
}

func (r InvocationResult) OutputBase64() string {
	return r.outputBase64
}

func (r InvocationResult) OutputJSON() json.RawMessage {
	return append(json.RawMessage(nil), r.outputJSON...)
}

func (r InvocationResult) SelectedNodeID() string {
	return r.selectedNodeID
}

func (r InvocationResult) SchedulingReason() string {
	return r.schedulingReason
}

func (r InvocationResult) ElapsedMS() int64 {
	return r.elapsedMS
}

func (r InvocationResult) Receipt() json.RawMessage {
	return append(json.RawMessage(nil), r.receipt...)
}

func (r InvocationResult) Failure() *InvocationFailure {
	if r.failure == nil {
		return nil
	}
	value := *r.failure
	return &value
}

func (f InvocationFailure) Code() string {
	return f.code
}

func (f InvocationFailure) Stage() string {
	return f.stage
}

func (f InvocationFailure) Message() string {
	return f.message
}

func (f InvocationFailure) Retryable() bool {
	return f.retryable
}

// NewInvocationResultFromJSON decodes the daemon unary result projection.
func NewInvocationResultFromJSON(raw []byte) (InvocationResult, error) {
	var dto struct {
		OK                *bool           `json:"ok"`
		Tuple             json.RawMessage `json:"tuple"`
		TerminalState     string          `json:"terminal_state"`
		OutputContentType string          `json:"output_content_type"`
		OutputBase64      string          `json:"output_base64"`
		OutputJSON        json.RawMessage `json:"output_json"`
		SelectedNodeID    string          `json:"selected_node_id"`
		SchedulingReason  string          `json:"scheduling_reason"`
		ElapsedMS         int64           `json:"elapsed_ms"`
		Receipt           json.RawMessage `json:"receipt"`
		Error             json.RawMessage `json:"error"`
	}
	if err := json.Unmarshal(raw, &dto); err != nil {
		return InvocationResult{}, invalidRuntimePayload(fmt.Sprintf("decode invocation result JSON: %v", err), err)
	}
	if dto.OK == nil {
		return InvocationResult{}, invalidRuntimePayload("ok is required", nil)
	}
	if len(dto.Tuple) == 0 || string(dto.Tuple) == "null" {
		return InvocationResult{}, invalidRuntimePayload("tuple is required", nil)
	}
	tuple, err := NewInvocationDraftFromJSON(dto.Tuple)
	if err != nil {
		return InvocationResult{}, err
	}
	if dto.TerminalState == "" {
		return InvocationResult{}, invalidRuntimePayload("terminal_state is required", nil)
	}
	if dto.ElapsedMS < 0 {
		return InvocationResult{}, invalidRuntimePayload("elapsed_ms must be non-negative", nil)
	}
	failure, err := decodeInvocationFailure(dto.Error)
	if err != nil {
		return InvocationResult{}, err
	}
	if *dto.OK && failure != nil {
		return InvocationResult{}, invalidRuntimePayload("ok result must not include error", nil)
	}
	if !*dto.OK && failure == nil {
		return InvocationResult{}, invalidRuntimePayload("failed result must include error", nil)
	}
	return InvocationResult{
		ok:                *dto.OK,
		tuple:             tuple,
		terminalState:     dto.TerminalState,
		outputContentType: dto.OutputContentType,
		outputBase64:      dto.OutputBase64,
		outputJSON:        append(json.RawMessage(nil), dto.OutputJSON...),
		selectedNodeID:    dto.SelectedNodeID,
		schedulingReason:  dto.SchedulingReason,
		elapsedMS:         dto.ElapsedMS,
		receipt:           append(json.RawMessage(nil), dto.Receipt...),
		failure:           failure,
	}, nil
}

func decodeInvocationFailure(raw json.RawMessage) (*InvocationFailure, error) {
	if len(raw) == 0 || string(raw) == "null" {
		return nil, nil
	}
	var dto struct {
		Code      string `json:"code"`
		Stage     string `json:"stage"`
		Message   string `json:"message"`
		Retryable bool   `json:"retryable"`
	}
	if err := json.Unmarshal(raw, &dto); err != nil {
		return nil, invalidRuntimePayload(fmt.Sprintf("decode invocation error JSON: %v", err), err)
	}
	if dto.Code == "" || dto.Stage == "" {
		return nil, invalidRuntimePayload("error code and stage are required", nil)
	}
	return &InvocationFailure{
		code:      dto.Code,
		stage:     dto.Stage,
		message:   dto.Message,
		retryable: dto.Retryable,
	}, nil
}

// InvocationCancel is the daemon cancellation outcome for a submitted handle.
type InvocationCancel struct {
	handleID  uint64
	cancelled bool
	state     string
	terminal  bool
}

func (c InvocationCancel) HandleID() uint64 {
	return c.handleID
}

func (c InvocationCancel) Cancelled() bool {
	return c.cancelled
}

func (c InvocationCancel) State() string {
	return c.state
}

func (c InvocationCancel) Terminal() bool {
	return c.terminal
}

// NewInvocationCancelFromJSON decodes the daemon cancellation outcome projection.
func NewInvocationCancelFromJSON(raw []byte) (InvocationCancel, error) {
	var dto struct {
		HandleID  uint64 `json:"handle_id"`
		Cancelled bool   `json:"cancelled"`
		State     string `json:"state"`
		Terminal  bool   `json:"terminal"`
	}
	if err := json.Unmarshal(raw, &dto); err != nil {
		return InvocationCancel{}, invalidRuntimePayload(fmt.Sprintf("decode invocation cancel JSON: %v", err), err)
	}
	if dto.HandleID == 0 {
		return InvocationCancel{}, invalidRuntimePayload("handle_id is required", nil)
	}
	if dto.State == "" {
		return InvocationCancel{}, invalidRuntimePayload("state is required", nil)
	}
	return InvocationCancel{
		handleID:  dto.HandleID,
		cancelled: dto.Cancelled,
		state:     dto.State,
		terminal:  dto.Terminal,
	}, nil
}

// InvocationHandle is the submitted invocation observation handle projection.
type InvocationHandle struct {
	handleID uint64
	state    string
	terminal bool
	events   []InvocationHandleEvent
	result   json.RawMessage
}

type InvocationHandleEvent struct {
	sequence uint64
	kind     string
	state    string
	terminal bool
	reason   string
	result   json.RawMessage
}

func (h InvocationHandle) HandleID() uint64 {
	return h.handleID
}

func (h InvocationHandle) State() string {
	return h.state
}

func (h InvocationHandle) Terminal() bool {
	return h.terminal
}

func (h InvocationHandle) Events() []InvocationHandleEvent {
	return append([]InvocationHandleEvent(nil), h.events...)
}

func (h InvocationHandle) Result() json.RawMessage {
	return append(json.RawMessage(nil), h.result...)
}

func (e InvocationHandleEvent) Sequence() uint64 {
	return e.sequence
}

func (e InvocationHandleEvent) Kind() string {
	return e.kind
}

func (e InvocationHandleEvent) State() string {
	return e.state
}

func (e InvocationHandleEvent) Terminal() bool {
	return e.terminal
}

func (e InvocationHandleEvent) Reason() string {
	return e.reason
}

func (e InvocationHandleEvent) Result() json.RawMessage {
	return append(json.RawMessage(nil), e.result...)
}

// NewInvocationHandleFromJSON decodes the daemon handle snapshot projection.
func NewInvocationHandleFromJSON(raw []byte) (InvocationHandle, error) {
	var dto struct {
		HandleID uint64 `json:"handle_id"`
		State    string `json:"state"`
		Terminal bool   `json:"terminal"`
		Events   []struct {
			Sequence uint64          `json:"sequence"`
			Kind     string          `json:"kind"`
			State    string          `json:"state"`
			Terminal bool            `json:"terminal"`
			Reason   string          `json:"reason"`
			Result   json.RawMessage `json:"result"`
		} `json:"events"`
		Result json.RawMessage `json:"result"`
	}
	if err := json.Unmarshal(raw, &dto); err != nil {
		return InvocationHandle{}, invalidRuntimePayload(fmt.Sprintf("decode invocation handle JSON: %v", err), err)
	}
	if dto.HandleID == 0 {
		return InvocationHandle{}, invalidRuntimePayload("handle_id is required", nil)
	}
	if dto.State == "" {
		return InvocationHandle{}, invalidRuntimePayload("state is required", nil)
	}
	events := make([]InvocationHandleEvent, 0, len(dto.Events))
	for _, event := range dto.Events {
		if event.Sequence == 0 {
			return InvocationHandle{}, invalidRuntimePayload("event sequence is required", nil)
		}
		if event.Kind == "" || event.State == "" {
			return InvocationHandle{}, invalidRuntimePayload("event kind and state are required", nil)
		}
		events = append(events, InvocationHandleEvent{
			sequence: event.Sequence,
			kind:     event.Kind,
			state:    event.State,
			terminal: event.Terminal,
			reason:   event.Reason,
			result:   append(json.RawMessage(nil), event.Result...),
		})
	}
	return InvocationHandle{
		handleID: dto.HandleID,
		state:    dto.State,
		terminal: dto.Terminal,
		events:   events,
		result:   append(json.RawMessage(nil), dto.Result...),
	}, nil
}

func invalidRuntimeClient(message string) error {
	return &SDKError{
		Code:      ErrInvalidArgument,
		Stage:     "sdk",
		Retry:     RetryNever,
		Retryable: RetryableForHint(RetryNever),
		Message:   message,
	}
}

func invalidRuntimePayload(message string, cause error) error {
	return &SDKError{
		Code:      ErrInvalidArgument,
		Stage:     "runtime",
		Retry:     RetryNever,
		Retryable: RetryableForHint(RetryNever),
		Message:   message,
		Cause:     cause,
	}
}

func transportRuntimeError(message string, cause error) error {
	return &SDKError{
		Code:      ErrTransport,
		Stage:     "transport",
		Retry:     RetrySafe,
		Retryable: RetryableForHint(RetrySafe),
		Message:   message,
		Cause:     cause,
	}
}
