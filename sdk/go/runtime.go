package easynet

import (
	"context"
	"encoding/hex"
	"encoding/json"
	"errors"
	"fmt"
	"strings"
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
	ExpiresInMS        int64  `json:"expires_in_ms,omitempty"`
	SignerID           string `json:"signer_id,omitempty"`
	PolicyRef          string `json:"policy_ref,omitempty"`
	LocalDaemonSigning bool   `json:"local_daemon_signing,omitempty"`
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
	ok                      bool
	tuple                   InvocationDraft
	invocationID            string
	terminalState           string
	outputContentType       string
	outputBase64            string
	outputJSON              json.RawMessage
	selectedNodeID          string
	schedulingReason        string
	elapsedMS               int64
	admissionReceipt        json.RawMessage
	terminalReceipt         json.RawMessage
	admissionReceiptSummary *RuntimeReceipt
	terminalReceiptSummary  *RuntimeReceipt
	failure                 *InvocationFailure
}

// RuntimeReceipt is a non-verifying terminal/admission receipt projection.
// Cryptographic verification requires a full Axon InvocationReceipt and is
// deliberately exposed by ReceiptClient instead of this summary.
type RuntimeReceipt struct {
	Raw                map[string]any `json:"-"`
	ReceiptID          string         `json:"receipt_id,omitempty"`
	ReceiptURA         string         `json:"receipt_ura,omitempty"`
	InvocationID       string         `json:"invocation_id,omitempty"`
	ReceiptType        string         `json:"receipt_type,omitempty"`
	State              string         `json:"state,omitempty"`
	Index              uint64         `json:"index,omitempty"`
	TimestampUnixMS    int64          `json:"timestamp_unix_ms,omitempty"`
	PrevReceiptHashHex string         `json:"prev_receipt_hash_hex,omitempty"`
	SelfHashHex        string         `json:"self_hash_hex,omitempty"`
	CleanupComplete    *bool          `json:"cleanup_complete,omitempty"`
	Reason             string         `json:"reason,omitempty"`
	ChildInvocationID  string         `json:"child_invocation_id,omitempty"`
}

func NewRuntimeReceiptFromJSON(raw []byte) (RuntimeReceipt, error) {
	var dto struct {
		ReceiptID          string `json:"receipt_id"`
		ReceiptURA         string `json:"receipt_ura"`
		InvocationID       string `json:"invocation_id"`
		ReceiptType        string `json:"receipt_type"`
		State              string `json:"state"`
		Index              int64  `json:"index"`
		TimestampUnixMS    int64  `json:"timestamp_unix_ms"`
		PrevReceiptHashHex string `json:"prev_receipt_hash_hex"`
		SelfHashHex        string `json:"self_hash_hex"`
		CleanupComplete    *bool  `json:"cleanup_complete"`
		Reason             string `json:"reason"`
		ChildInvocationID  string `json:"child_invocation_id"`
	}
	if err := json.Unmarshal(raw, &dto); err != nil {
		return RuntimeReceipt{}, invalidRuntimePayload(fmt.Sprintf("decode runtime receipt JSON: %v", err), err)
	}
	if dto.Index < 0 || dto.TimestampUnixMS < 0 {
		return RuntimeReceipt{}, invalidRuntimePayload("runtime receipt index and timestamp_unix_ms must be non-negative", nil)
	}
	var rawMap map[string]any
	if err := json.Unmarshal(raw, &rawMap); err != nil || rawMap == nil {
		return RuntimeReceipt{}, invalidRuntimePayload("runtime receipt must be an object", err)
	}
	return RuntimeReceipt{
		Raw:                rawMap,
		ReceiptID:          dto.ReceiptID,
		ReceiptURA:         dto.ReceiptURA,
		InvocationID:       dto.InvocationID,
		ReceiptType:        dto.ReceiptType,
		State:              dto.State,
		Index:              uint64(dto.Index),
		TimestampUnixMS:    dto.TimestampUnixMS,
		PrevReceiptHashHex: dto.PrevReceiptHashHex,
		SelfHashHex:        dto.SelfHashHex,
		CleanupComplete:    dto.CleanupComplete,
		Reason:             dto.Reason,
		ChildInvocationID:  dto.ChildInvocationID,
	}, nil
}

func (r RuntimeReceipt) HasCausalAnchor() bool {
	return strings.TrimSpace(r.ReceiptURA) != "" && strings.TrimSpace(r.SelfHashHex) != ""
}

func (r RuntimeReceipt) ValidateSummary() error {
	if strings.TrimSpace(r.InvocationID) == "" {
		return invalidRuntimePayload("runtime receipt summary is missing invocation_id", nil)
	}
	if strings.TrimSpace(r.ReceiptType) == "" {
		return invalidRuntimePayload("runtime receipt summary is missing receipt_type", nil)
	}
	if _, err := r.PrevReceiptHash(); err != nil {
		return err
	}
	if _, err := r.SelfReceiptHash(); err != nil {
		return err
	}
	return nil
}

func (r RuntimeReceipt) PrevReceiptHash() ([]byte, error) {
	return runtimeReceiptHash(r.PrevReceiptHashHex, "prev_receipt_hash_hex")
}

func (r RuntimeReceipt) SelfReceiptHash() ([]byte, error) {
	return runtimeReceiptHash(r.SelfHashHex, "self_hash_hex")
}

func (r RuntimeReceipt) RawProjection() map[string]any {
	encoded, err := json.Marshal(r.Raw)
	if err != nil {
		return nil
	}
	var clone map[string]any
	if err := json.Unmarshal(encoded, &clone); err != nil {
		return nil
	}
	return clone
}

func runtimeReceiptHash(value string, field string) ([]byte, error) {
	value = strings.TrimSpace(value)
	if value == "" {
		return nil, invalidRuntimePayload(field+" is required", nil)
	}
	hash, err := hex.DecodeString(value)
	if err != nil {
		return nil, invalidRuntimePayload(field+" must be hexadecimal", err)
	}
	if len(hash) != 32 {
		return nil, invalidRuntimePayload(field+" must be exactly 32 bytes", nil)
	}
	return hash, nil
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

func (r InvocationResult) InvocationID() string {
	return r.invocationID
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
	return r.TerminalReceipt()
}

// AdmissionReceipt returns the pre-execution admission checkpoint, when the
// runtime emitted one.
func (r InvocationResult) AdmissionReceipt() json.RawMessage {
	return append(json.RawMessage(nil), r.admissionReceipt...)
}

// TerminalReceipt returns the execution terminal checkpoint. Receipt is the
// public compatibility projection of this same value.
func (r InvocationResult) TerminalReceipt() json.RawMessage {
	return append(json.RawMessage(nil), r.terminalReceipt...)
}

func (r InvocationResult) ReceiptSummary() *RuntimeReceipt {
	return r.TerminalReceiptSummary()
}

func (r InvocationResult) AdmissionReceiptSummary() *RuntimeReceipt {
	return cloneRuntimeReceipt(r.admissionReceiptSummary)
}

func (r InvocationResult) TerminalReceiptSummary() *RuntimeReceipt {
	return cloneRuntimeReceipt(r.terminalReceiptSummary)
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
		InvocationID      string          `json:"invocation_id"`
		TerminalState     string          `json:"terminal_state"`
		OutputContentType string          `json:"output_content_type"`
		OutputBase64      string          `json:"output_base64"`
		OutputJSON        json.RawMessage `json:"output_json"`
		SelectedNodeID    string          `json:"selected_node_id"`
		SchedulingReason  string          `json:"scheduling_reason"`
		ElapsedMS         int64           `json:"elapsed_ms"`
		Receipt           json.RawMessage `json:"receipt"`
		AdmissionReceipt  json.RawMessage `json:"admission_receipt"`
		TerminalReceipt   json.RawMessage `json:"terminal_receipt"`
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
	terminalReceipt, err := normalizeTerminalReceipt(dto.Receipt, dto.TerminalReceipt)
	if err != nil {
		return InvocationResult{}, err
	}
	admissionReceiptSummary, err := decodeRuntimeReceiptSummary(dto.AdmissionReceipt)
	if err != nil {
		return InvocationResult{}, err
	}
	terminalReceiptSummary, err := decodeRuntimeReceiptSummary(terminalReceipt)
	if err != nil {
		return InvocationResult{}, err
	}
	return InvocationResult{
		ok:                      *dto.OK,
		tuple:                   tuple,
		invocationID:            dto.InvocationID,
		terminalState:           dto.TerminalState,
		outputContentType:       dto.OutputContentType,
		outputBase64:            dto.OutputBase64,
		outputJSON:              append(json.RawMessage(nil), dto.OutputJSON...),
		selectedNodeID:          dto.SelectedNodeID,
		schedulingReason:        dto.SchedulingReason,
		elapsedMS:               dto.ElapsedMS,
		admissionReceipt:        cloneOptionalJSON(dto.AdmissionReceipt),
		terminalReceipt:         terminalReceipt,
		admissionReceiptSummary: admissionReceiptSummary,
		terminalReceiptSummary:  terminalReceiptSummary,
		failure:                 failure,
	}, nil
}

func decodeRuntimeReceiptSummary(raw json.RawMessage) (*RuntimeReceipt, error) {
	raw = cloneOptionalJSON(raw)
	if len(raw) == 0 {
		return nil, nil
	}
	receipt, err := NewRuntimeReceiptFromJSON(raw)
	if err != nil {
		return nil, err
	}
	return &receipt, nil
}

func cloneRuntimeReceipt(receipt *RuntimeReceipt) *RuntimeReceipt {
	if receipt == nil {
		return nil
	}
	clone := *receipt
	clone.Raw = receipt.RawProjection()
	if receipt.CleanupComplete != nil {
		value := *receipt.CleanupComplete
		clone.CleanupComplete = &value
	}
	return &clone
}

func normalizeTerminalReceipt(compatibility, terminal json.RawMessage) (json.RawMessage, error) {
	compatibility = cloneOptionalJSON(compatibility)
	terminal = cloneOptionalJSON(terminal)
	if len(terminal) == 0 {
		terminal = compatibility
	}
	if len(compatibility) == 0 {
		compatibility = terminal
	}
	if len(compatibility) != 0 && len(terminal) != 0 {
		var compatibilityValue any
		var terminalValue any
		if err := json.Unmarshal(compatibility, &compatibilityValue); err != nil {
			return nil, invalidRuntimePayload(fmt.Sprintf("decode receipt JSON: %v", err), err)
		}
		if err := json.Unmarshal(terminal, &terminalValue); err != nil {
			return nil, invalidRuntimePayload(fmt.Sprintf("decode terminal_receipt JSON: %v", err), err)
		}
		if !jsonValuesEqual(compatibilityValue, terminalValue) {
			return nil, invalidRuntimePayload("receipt must equal terminal_receipt", nil)
		}
	}
	return terminal, nil
}

func cloneOptionalJSON(raw json.RawMessage) json.RawMessage {
	if len(raw) == 0 || string(raw) == "null" {
		return nil
	}
	return append(json.RawMessage(nil), raw...)
}

func jsonValuesEqual(left, right any) bool {
	leftJSON, leftErr := json.Marshal(left)
	rightJSON, rightErr := json.Marshal(right)
	return leftErr == nil && rightErr == nil && string(leftJSON) == string(rightJSON)
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
	details := map[string]any{}
	if cause != nil {
		details["cause"] = cause.Error()
		message = fmt.Sprintf("%s: %v", message, cause)
	}
	return &SDKError{
		Code:      ErrTransport,
		Stage:     "transport",
		Retry:     RetrySafe,
		Retryable: RetryableForHint(RetrySafe),
		Message:   message,
		Details:   details,
		Cause:     cause,
	}
}
