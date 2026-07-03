package easynet

import (
	"context"
	"encoding/json"
	"errors"
	"fmt"
)

// RuntimeTransport is the narrow Runtime Core invocation transport seam.
type RuntimeTransport interface {
	Prepare(ctx context.Context, draftJSON []byte, optionsJSON []byte) ([]byte, error)
	SubmitSigned(ctx context.Context, signedJSON []byte) ([]byte, error)
}

// RuntimeTransportFunc adapts functions into a RuntimeTransport.
type RuntimeTransportFunc struct {
	PrepareFunc      func(ctx context.Context, draftJSON []byte, optionsJSON []byte) ([]byte, error)
	SubmitSignedFunc func(ctx context.Context, signedJSON []byte) ([]byte, error)
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

// RuntimeClient is the Runtime Core invocation facade.
type RuntimeClient struct {
	transport RuntimeTransport
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

// PrepareOptions are daemon-owned prepare policy knobs.
type PrepareOptions struct {
	ResolveDescriptor bool  `json:"resolve_descriptor,omitempty"`
	FillNonce         bool  `json:"fill_nonce,omitempty"`
	RequireUserSig    bool  `json:"require_user_sig,omitempty"`
	ExpiresInMS       int64 `json:"expires_in_ms,omitempty"`
}

// Prepare delegates canonical material generation to the daemon transport.
func (c *RuntimeClient) Prepare(ctx context.Context, draft InvocationDraft, opts PrepareOptions) (PreparedInvocation, SigningMaterial, error) {
	if c == nil || c.transport == nil {
		return PreparedInvocation{}, SigningMaterial{}, invalidRuntimeClient("runtime client is not initialized")
	}
	if ctx == nil {
		return PreparedInvocation{}, SigningMaterial{}, invalidRuntimeClient("context is required")
	}
	draftJSON, err := json.Marshal(draft)
	if err != nil {
		return PreparedInvocation{}, SigningMaterial{}, invalidRuntimePayload(fmt.Sprintf("encode invocation draft: %v", err), err)
	}
	optionsJSON, err := json.Marshal(opts)
	if err != nil {
		return PreparedInvocation{}, SigningMaterial{}, invalidRuntimePayload(fmt.Sprintf("encode prepare options: %v", err), err)
	}
	raw, err := c.transport.Prepare(ctx, draftJSON, optionsJSON)
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

// SubmitSigned submits an immutable signed envelope and returns an observation handle.
func (c *RuntimeClient) SubmitSigned(ctx context.Context, signed SignedInvocation) (InvocationHandle, error) {
	if c == nil || c.transport == nil {
		return InvocationHandle{}, invalidRuntimeClient("runtime client is not initialized")
	}
	if ctx == nil {
		return InvocationHandle{}, invalidRuntimeClient("context is required")
	}
	if !signed.SubmitReady() {
		return InvocationHandle{}, invalidRuntimePayload("signed invocation is not submit-ready", nil)
	}
	signedJSON, err := json.Marshal(signed)
	if err != nil {
		return InvocationHandle{}, invalidRuntimePayload(fmt.Sprintf("encode signed invocation: %v", err), err)
	}
	raw, err := c.transport.SubmitSigned(ctx, signedJSON)
	if err != nil {
		var sdkErr *SDKError
		if errors.As(err, &sdkErr) {
			return InvocationHandle{}, sdkErr
		}
		return InvocationHandle{}, transportRuntimeError("submit signed transport failed", err)
	}
	return NewInvocationHandleFromJSON(raw)
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
