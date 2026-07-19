package easynet

import (
	"context"
	"encoding/base64"
	"encoding/hex"
	"encoding/json"
	"errors"
	"fmt"
	"reflect"
	"strings"
	"sync"

	axoninv "axon.run/sdk/go/axon/invocation"
)

// InvocationControlCapability is the opaque authority for controlling a
// submitted Invocation observation lifecycle.
//
// The current daemon registry is still backed by a process-local numeric
// handle, but that number is adapter-private. SDK consumers operate on
// InvocationHandle; transport adapters receive this capability object.
type InvocationControlCapability struct {
	handleID     uint64
	runtimeBound bool
}

func newInvocationControlCapability(handleID uint64) (InvocationControlCapability, error) {
	return newRuntimeInvocationControlCapability(handleID)
}

func newRuntimeInvocationControlCapability(handleID uint64) (InvocationControlCapability, error) {
	if handleID == 0 {
		return InvocationControlCapability{}, invalidRuntimePayload("invocation control capability is required", nil)
	}
	return InvocationControlCapability{handleID: handleID, runtimeBound: true}, nil
}

func newSnapshotInvocationControlCapability(handleID uint64) (InvocationControlCapability, error) {
	if handleID == 0 {
		return InvocationControlCapability{}, invalidRuntimePayload("handle_id is required", nil)
	}
	return InvocationControlCapability{handleID: handleID}, nil
}

func (c InvocationControlCapability) valid() bool {
	return c.handleID != 0 && c.runtimeBound
}

// AdapterHandleID projects the adapter-private daemon handle behind this
// capability. RuntimeTransport implementations use it to address their handle
// store; application code should keep treating InvocationHandle as the public
// lifecycle object.
func (c InvocationControlCapability) AdapterHandleID() (uint64, error) {
	if !c.valid() {
		return 0, invalidRuntimePayload("runtime-bound invocation control capability is required", nil)
	}
	return c.handleID, nil
}

func (c InvocationControlCapability) adapterHandleID() uint64 {
	return c.handleID
}

// RuntimeTransport is the narrow Runtime Core invocation transport seam.
type RuntimeTransport interface {
	Invoke(ctx context.Context, draftJSON []byte) ([]byte, error)
	OpenStream(ctx context.Context, draftJSON []byte) (StreamTransport, []byte, error)
	OpenBidi(ctx context.Context, draftJSON []byte, streamsJSON []byte) (BidiTransport, []byte, error)
	Prepare(ctx context.Context, draftJSON []byte, optionsJSON []byte) ([]byte, error)
	SubmitSigned(ctx context.Context, signedJSON []byte) ([]byte, error)
	AwaitHandle(ctx context.Context, control InvocationControlCapability) ([]byte, error)
	CancelHandle(ctx context.Context, control InvocationControlCapability, reason string) ([]byte, error)
	HandleEvents(ctx context.Context, control InvocationControlCapability) ([]byte, error)
	FreeHandle(ctx context.Context, control InvocationControlCapability) error
	Close(ctx context.Context) error
}

// RuntimeRecoveryTransport is an optional provider seam for restart recovery.
// Providers that implement it own bounded orphan scans, replayed terminal
// facts, and cleanup before returning a ready runtime state.
type RuntimeRecoveryTransport interface {
	Recover(ctx context.Context, requestJSON []byte) ([]byte, error)
}

// DescriptorResolverTransport is an optional provider seam for resolving
// runtime-governed AbilityDescriptorRefs before building Invocation drafts.
type DescriptorResolverTransport interface {
	ResolveDescriptorRef(ctx context.Context, requestJSON []byte) ([]byte, error)
}

// RuntimeDescriptorRefRequest selects one runtime-owned ability descriptor.
type RuntimeDescriptorRefRequest struct {
	CalleeURA  string `json:"callee_ura"`
	Ability    string `json:"ability"`
	CallMode   string `json:"call_mode,omitempty"`
	CallerURA  string `json:"caller_ura,omitempty"`
	SubjectURA string `json:"subject_ura,omitempty"`
}

// RuntimeTransportFunc adapts functions into a RuntimeTransport.
type RuntimeTransportFunc struct {
	InvokeFunc               func(ctx context.Context, draftJSON []byte) ([]byte, error)
	OpenStreamFunc           func(ctx context.Context, draftJSON []byte) (StreamTransport, []byte, error)
	OpenBidiFunc             func(ctx context.Context, draftJSON []byte, streamsJSON []byte) (BidiTransport, []byte, error)
	PrepareFunc              func(ctx context.Context, draftJSON []byte, optionsJSON []byte) ([]byte, error)
	SubmitSignedFunc         func(ctx context.Context, signedJSON []byte) ([]byte, error)
	AwaitHandleFunc          func(ctx context.Context, control InvocationControlCapability) ([]byte, error)
	CancelHandleFunc         func(ctx context.Context, control InvocationControlCapability, reason string) ([]byte, error)
	HandleEventsFunc         func(ctx context.Context, control InvocationControlCapability) ([]byte, error)
	FreeHandleFunc           func(ctx context.Context, control InvocationControlCapability) error
	RecoverFunc              func(ctx context.Context, requestJSON []byte) ([]byte, error)
	ResolveDescriptorRefFunc func(ctx context.Context, requestJSON []byte) ([]byte, error)
	CloseFunc                func(ctx context.Context) error
}

func runtimeBidiOpenJSON(sessionID string, maxBufferedFrames int) ([]byte, error) {
	return json.Marshal(struct {
		SessionID         string `json:"session_id"`
		State             string `json:"state"`
		MaxBufferedFrames int    `json:"max_buffered_frames"`
	}{
		SessionID:         sessionID,
		State:             "Open",
		MaxBufferedFrames: maxBufferedFrames,
	})
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

func (f RuntimeTransportFunc) AwaitHandle(ctx context.Context, control InvocationControlCapability) ([]byte, error) {
	if f.AwaitHandleFunc == nil {
		return nil, invalidRuntimeClient("runtime await-handle transport function is required")
	}
	if !control.valid() {
		return nil, invalidRuntimePayload("invocation control capability is required", nil)
	}
	return f.AwaitHandleFunc(ctx, control)
}

func (f RuntimeTransportFunc) CancelHandle(ctx context.Context, control InvocationControlCapability, reason string) ([]byte, error) {
	if f.CancelHandleFunc == nil {
		return nil, invalidRuntimeClient("runtime cancel-handle transport function is required")
	}
	if !control.valid() {
		return nil, invalidRuntimePayload("invocation control capability is required", nil)
	}
	return f.CancelHandleFunc(ctx, control, reason)
}

func (f RuntimeTransportFunc) HandleEvents(ctx context.Context, control InvocationControlCapability) ([]byte, error) {
	if f.HandleEventsFunc == nil {
		return nil, invalidRuntimeClient("runtime handle-events transport function is required")
	}
	if !control.valid() {
		return nil, invalidRuntimePayload("invocation control capability is required", nil)
	}
	return f.HandleEventsFunc(ctx, control)
}

func (f RuntimeTransportFunc) FreeHandle(ctx context.Context, control InvocationControlCapability) error {
	if f.FreeHandleFunc == nil {
		return invalidRuntimeClient("runtime free-handle transport function is required")
	}
	if !control.valid() {
		return invalidRuntimePayload("invocation control capability is required", nil)
	}
	return f.FreeHandleFunc(ctx, control)
}

func (f RuntimeTransportFunc) Recover(ctx context.Context, requestJSON []byte) ([]byte, error) {
	if f.RecoverFunc == nil {
		return nil, invalidRuntimeClient("runtime recovery transport function is required")
	}
	return f.RecoverFunc(ctx, requestJSON)
}

func (f RuntimeTransportFunc) ResolveDescriptorRef(ctx context.Context, requestJSON []byte) ([]byte, error) {
	if f.ResolveDescriptorRefFunc == nil {
		return nil, invalidRuntimeClient("runtime descriptor resolver transport function is required")
	}
	return f.ResolveDescriptorRefFunc(ctx, requestJSON)
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

// ResolveDescriptorRef asks the runtime provider for the complete
// descriptor-bound Ability ref selected by a callee, ability, and call mode.
func (c *RuntimeClient) ResolveDescriptorRef(ctx context.Context, req RuntimeDescriptorRefRequest) (string, error) {
	transport, err := c.runtimeTransport(ctx)
	if err != nil {
		return "", err
	}
	resolver, ok := transport.(DescriptorResolverTransport)
	if !ok {
		return "", invalidRuntimeClient("runtime transport does not expose descriptor resolution")
	}
	if strings.TrimSpace(req.CallMode) == "" {
		req.CallMode = "rpc"
	}
	requestJSON, err := json.Marshal(req)
	if err != nil {
		return "", invalidRuntimePayload(fmt.Sprintf("encode descriptor_ref resolution request: %v", err), err)
	}
	raw, err := resolver.ResolveDescriptorRef(ctx, requestJSON)
	if err != nil {
		var sdkErr *SDKError
		if errors.As(err, &sdkErr) {
			return "", sdkErr
		}
		return "", transportRuntimeError("resolve descriptor_ref transport failed", err)
	}
	var decoded map[string]any
	if err := json.Unmarshal(raw, &decoded); err != nil {
		return "", invalidRuntimePayload(fmt.Sprintf("decode descriptor_ref resolution: %v", err), err)
	}
	descriptorRef, _ := decoded["descriptor_ref"].(string)
	if strings.TrimSpace(descriptorRef) == "" {
		return "", invalidRuntimePayload("descriptor_ref resolution omitted descriptor_ref", nil)
	}
	return descriptorRef, nil
}

// PrepareOptions are daemon-owned prepare policy knobs.
type PrepareOptions struct {
	ExpiresInMS        int64  `json:"expires_in_ms,omitempty"`
	SignerID           string `json:"signer_id,omitempty"`
	PolicyRef          string `json:"policy_ref,omitempty"`
	LocalDaemonSigning bool   `json:"local_daemon_signing,omitempty"`
}

// RuntimeRecoveryRequest declares one bounded runtime restart-recovery scan.
type RuntimeRecoveryRequest struct {
	RecoveryID     string `json:"recovery_id"`
	DeadlineUnixMS int64  `json:"deadline_unix_ms"`
	MaxInvocations int    `json:"max_invocations"`
}

// RuntimeRecoveryEvent is an observable recovery lifecycle event.
type RuntimeRecoveryEvent struct {
	Sequence     uint64 `json:"sequence"`
	Kind         string `json:"kind"`
	InvocationID string `json:"invocation_id,omitempty"`
	State        string `json:"state,omitempty"`
	Terminal     bool   `json:"terminal"`
	ReceiptURA   string `json:"receipt_ura,omitempty"`
	Reason       string `json:"reason,omitempty"`
}

// RuntimeRecoveryReport proves that the provider completed a bounded restart
// recovery transition without fabricating successful terminal facts.
type RuntimeRecoveryReport struct {
	RecoveryID               string                 `json:"recovery_id"`
	State                    string                 `json:"state"`
	RecoveredInvocations     int                    `json:"recovered_invocations"`
	ReapedOrphans            int                    `json:"reaped_orphans"`
	ReplayedTerminalReceipts int                    `json:"replayed_terminal_receipts"`
	BoundedScan              bool                   `json:"bounded_scan"`
	CleanupComplete          bool                   `json:"cleanup_complete"`
	Events                   []RuntimeRecoveryEvent `json:"events"`
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
	return c.prepare(ctx, draft, opts)
}

// PrepareSigningMaterial returns canonical caller-signing material without
// retaining a native prepared invocation. It supports stateless external
// signer flows whose later request submits a signed envelope rather than using
// a process-local prepared handle.
func (c *RuntimeClient) PrepareSigningMaterial(ctx context.Context, draft InvocationDraft, opts PrepareOptions) (SigningMaterial, error) {
	raw, err := c.prepareRaw(ctx, draft, opts, true)
	if err != nil {
		return SigningMaterial{}, err
	}
	return signingMaterialFromPrepareJSON(raw)
}

func (c *RuntimeClient) prepare(ctx context.Context, draft InvocationDraft, opts PrepareOptions) (PreparedInvocation, SigningMaterial, error) {
	raw, err := c.prepareRaw(ctx, draft, opts, false)
	if err != nil {
		return PreparedInvocation{}, SigningMaterial{}, err
	}
	prepared, err := NewPreparedInvocationFromJSON(raw)
	if err != nil {
		return PreparedInvocation{}, SigningMaterial{}, err
	}
	return prepared, prepared.SigningMaterial(), nil
}

func (c *RuntimeClient) prepareRaw(ctx context.Context, draft InvocationDraft, opts PrepareOptions, materialOnly bool) ([]byte, error) {
	transport, err := c.runtimeTransport(ctx)
	if err != nil {
		return nil, err
	}
	draftJSON, err := json.Marshal(draft)
	if err != nil {
		return nil, invalidRuntimePayload(fmt.Sprintf("encode invocation draft: %v", err), err)
	}
	optionsJSON, err := prepareOptionsJSON(opts, materialOnly)
	if err != nil {
		return nil, invalidRuntimePayload(fmt.Sprintf("encode prepare options: %v", err), err)
	}
	raw, err := transport.Prepare(ctx, draftJSON, optionsJSON)
	if err != nil {
		var sdkErr *SDKError
		if errors.As(err, &sdkErr) {
			return nil, sdkErr
		}
		return nil, transportRuntimeError("prepare transport failed", err)
	}
	return raw, nil
}

func prepareOptionsJSON(opts PrepareOptions, materialOnly bool) ([]byte, error) {
	if !materialOnly {
		return json.Marshal(opts)
	}
	return json.Marshal(struct {
		PrepareOptions
		MaterialOnly bool `json:"material_only"`
	}{PrepareOptions: opts, MaterialOnly: true})
}

func validateRuntimeRecoveryRequest(request RuntimeRecoveryRequest) error {
	if strings.TrimSpace(request.RecoveryID) == "" {
		return invalidRuntimePayload("recovery_id is required", nil)
	}
	if request.DeadlineUnixMS <= 0 {
		return invalidRuntimePayload("deadline_unix_ms is required", nil)
	}
	if request.MaxInvocations <= 0 {
		return invalidRuntimePayload("max_invocations is required", nil)
	}
	return nil
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
	return newRuntimeInvocationHandleFromJSON(raw)
}

// Recover delegates restart recovery to the Runtime provider.
func (c *RuntimeClient) Recover(ctx context.Context, request RuntimeRecoveryRequest) (RuntimeRecoveryReport, error) {
	transport, err := c.runtimeTransport(ctx)
	if err != nil {
		return RuntimeRecoveryReport{}, err
	}
	recovery, ok := transport.(RuntimeRecoveryTransport)
	if !ok {
		return RuntimeRecoveryReport{}, invalidRuntimeClient("runtime transport does not expose restart recovery")
	}
	if err := validateRuntimeRecoveryRequest(request); err != nil {
		return RuntimeRecoveryReport{}, err
	}
	requestJSON, err := json.Marshal(request)
	if err != nil {
		return RuntimeRecoveryReport{}, invalidRuntimePayload(fmt.Sprintf("encode runtime recovery request: %v", err), err)
	}
	raw, err := recovery.Recover(ctx, requestJSON)
	if err != nil {
		var sdkErr *SDKError
		if errors.As(err, &sdkErr) {
			return RuntimeRecoveryReport{}, sdkErr
		}
		return RuntimeRecoveryReport{}, transportRuntimeError("runtime recovery transport failed", err)
	}
	return NewRuntimeRecoveryReportFromJSON(raw)
}

// Await waits for a submitted invocation handle to reach a terminal result.
func (c *RuntimeClient) Await(ctx context.Context, handle InvocationHandle) (InvocationResult, error) {
	transport, err := c.runtimeTransport(ctx)
	if err != nil {
		return InvocationResult{}, err
	}
	control, err := handle.controlCapability()
	if err != nil {
		return InvocationResult{}, err
	}
	raw, err := transport.AwaitHandle(ctx, control)
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
	control, err := handle.controlCapability()
	if err != nil {
		return InvocationCancel{}, err
	}
	raw, err := transport.CancelHandle(ctx, control, reason)
	if err != nil {
		var sdkErr *SDKError
		if errors.As(err, &sdkErr) {
			return InvocationCancel{}, sdkErr
		}
		return InvocationCancel{}, transportRuntimeError("cancel handle transport failed", err)
	}
	return newInvocationCancelFromJSON(raw, &control)
}

// Events returns the current event snapshot for a submitted invocation handle.
func (c *RuntimeClient) Events(ctx context.Context, handle InvocationHandle) (InvocationHandle, error) {
	transport, err := c.runtimeTransport(ctx)
	if err != nil {
		return InvocationHandle{}, err
	}
	control, err := handle.controlCapability()
	if err != nil {
		return InvocationHandle{}, err
	}
	raw, err := transport.HandleEvents(ctx, control)
	if err != nil {
		var sdkErr *SDKError
		if errors.As(err, &sdkErr) {
			return InvocationHandle{}, sdkErr
		}
		return InvocationHandle{}, transportRuntimeError("handle events transport failed", err)
	}
	return newInvocationHandleSnapshotFromJSON(raw, &control)
}

// CloseHandle releases daemon-side observation state for a submitted invocation handle.
func (c *RuntimeClient) CloseHandle(ctx context.Context, handle InvocationHandle) error {
	transport, err := c.runtimeTransport(ctx)
	if err != nil {
		return err
	}
	control, err := handle.controlCapability()
	if err != nil {
		return err
	}
	if err := transport.FreeHandle(ctx, control); err != nil {
		var sdkErr *SDKError
		if errors.As(err, &sdkErr) {
			return sdkErr
		}
		return transportRuntimeError("free handle transport failed", err)
	}
	return nil
}

// NewRuntimeRecoveryReportFromJSON decodes a provider restart-recovery report.
func NewRuntimeRecoveryReportFromJSON(raw []byte) (RuntimeRecoveryReport, error) {
	var dto struct {
		RecoveryID               string                 `json:"recovery_id"`
		State                    string                 `json:"state"`
		RecoveredInvocations     int                    `json:"recovered_invocations"`
		ReapedOrphans            int                    `json:"reaped_orphans"`
		ReplayedTerminalReceipts int                    `json:"replayed_terminal_receipts"`
		BoundedScan              *bool                  `json:"bounded_scan"`
		CleanupComplete          *bool                  `json:"cleanup_complete"`
		Events                   []RuntimeRecoveryEvent `json:"events"`
	}
	if err := json.Unmarshal(raw, &dto); err != nil {
		return RuntimeRecoveryReport{}, invalidRuntimePayload(fmt.Sprintf("decode runtime recovery report JSON: %v", err), err)
	}
	if strings.TrimSpace(dto.RecoveryID) == "" {
		return RuntimeRecoveryReport{}, invalidRuntimePayload("recovery_id is required", nil)
	}
	if dto.State != "runtime_started" {
		return RuntimeRecoveryReport{}, invalidRuntimePayload("runtime recovery state must be runtime_started", nil)
	}
	if dto.RecoveredInvocations < 0 || dto.ReapedOrphans < 0 || dto.ReplayedTerminalReceipts < 0 {
		return RuntimeRecoveryReport{}, invalidRuntimePayload("runtime recovery counters must be non-negative", nil)
	}
	if dto.BoundedScan == nil || !*dto.BoundedScan {
		return RuntimeRecoveryReport{}, invalidRuntimePayload("bounded_scan must be true", nil)
	}
	if dto.CleanupComplete == nil || !*dto.CleanupComplete {
		return RuntimeRecoveryReport{}, invalidRuntimePayload("cleanup_complete must be true", nil)
	}
	for _, event := range dto.Events {
		if event.Sequence == 0 {
			return RuntimeRecoveryReport{}, invalidRuntimePayload("recovery event sequence is required", nil)
		}
		if strings.TrimSpace(event.Kind) == "" {
			return RuntimeRecoveryReport{}, invalidRuntimePayload("recovery event kind is required", nil)
		}
	}
	return RuntimeRecoveryReport{
		RecoveryID:               dto.RecoveryID,
		State:                    dto.State,
		RecoveredInvocations:     dto.RecoveredInvocations,
		ReapedOrphans:            dto.ReapedOrphans,
		ReplayedTerminalReceipts: dto.ReplayedTerminalReceipts,
		BoundedScan:              *dto.BoundedScan,
		CleanupComplete:          *dto.CleanupComplete,
		Events:                   append([]RuntimeRecoveryEvent(nil), dto.Events...),
	}, nil
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
	Raw                   map[string]any                `json:"-"`
	ReceiptID             string                        `json:"receipt_id,omitempty"`
	ReceiptURA            string                        `json:"receipt_ura,omitempty"`
	InvocationID          string                        `json:"invocation_id,omitempty"`
	ReceiptType           string                        `json:"receipt_type,omitempty"`
	State                 string                        `json:"state,omitempty"`
	Index                 uint64                        `json:"index,omitempty"`
	TimestampUnixMS       int64                         `json:"timestamp_unix_ms,omitempty"`
	PrevReceiptHashHex    string                        `json:"prev_receipt_hash_hex,omitempty"`
	SelfHashHex           string                        `json:"self_hash_hex,omitempty"`
	CleanupComplete       *bool                         `json:"cleanup_complete,omitempty"`
	Reason                string                        `json:"reason,omitempty"`
	ChildInvocationID     string                        `json:"child_invocation_id,omitempty"`
	PayloadBase64         string                        `json:"payload_base64,omitempty"`
	CallerBinding         *RuntimeReceiptAgentBinding   `json:"caller_binding,omitempty"`
	CalleeBinding         *RuntimeReceiptAgentBinding   `json:"callee_binding,omitempty"`
	SubjectBinding        *RuntimeReceiptSubjectBinding `json:"subject_binding,omitempty"`
	InvocationNonceBase64 string                        `json:"invocation_nonce_base64,omitempty"`
	CausalBindingKind     string                        `json:"causal_binding_kind,omitempty"`
	CausalBinding         map[string]any                `json:"causal_binding,omitempty"`
	CalleeSignature       *RuntimeReceiptSignature      `json:"callee_signature,omitempty"`
	SignerBinding         *RuntimeReceiptAgentBinding   `json:"signer_binding,omitempty"`
	HostAttestationBase64 string                        `json:"host_attestation_base64,omitempty"`
	AuthorityBindingKind  string                        `json:"authority_binding_kind,omitempty"`
	AuthorityBinding      map[string]any                `json:"authority_binding,omitempty"`
	AbilityBinding        string                        `json:"ability_binding,omitempty"`
	Failure               *RuntimeReceiptFailure        `json:"failure,omitempty"`
	Usage                 *RuntimeReceiptUsage          `json:"usage,omitempty"`
	SubjectRef            *RuntimeReceiptEntityRef      `json:"subject_ref,omitempty"`
	DescriptorVersion     string                        `json:"descriptor_version,omitempty"`
	SchemaHashHex         string                        `json:"schema_hash_hex,omitempty"`
	ImplHashHex           string                        `json:"impl_hash_hex,omitempty"`
	RuntimeEnv            string                        `json:"runtime_env,omitempty"`
	AuthorityProof        *RuntimeReceiptAuthorityProof `json:"authority_proof,omitempty"`
	InputHashHex          string                        `json:"input_hash_hex,omitempty"`
	OutputHashHex         string                        `json:"output_hash_hex,omitempty"`
	ParentReceipts        []RuntimeReceiptRef           `json:"parent_receipts,omitempty"`
}

type RuntimeReceiptAgentBinding struct {
	URA     string `json:"ura,omitempty"`
	Profile string `json:"profile,omitempty"`
}

type RuntimeReceiptSubjectBinding struct {
	URA     string `json:"ura,omitempty"`
	Profile string `json:"profile,omitempty"`
}

type RuntimeReceiptEntityRef struct {
	Kind    int32  `json:"kind,omitempty"`
	URA     string `json:"ura,omitempty"`
	Profile string `json:"profile,omitempty"`
}

type RuntimeReceiptSignature struct {
	Algorithm       string `json:"algorithm,omitempty"`
	SignatureBase64 string `json:"signature_base64,omitempty"`
	KeyIDHint       string `json:"key_id_hint,omitempty"`
}

type RuntimeReceiptFailure struct {
	Code          string `json:"code,omitempty"`
	Message       string `json:"message,omitempty"`
	Retryable     bool   `json:"retryable,omitempty"`
	Stage         int32  `json:"stage,omitempty"`
	SecurityClass int32  `json:"security_class,omitempty"`
}

type RuntimeReceiptUsage struct {
	TokensIn      uint64 `json:"tokens_in,omitempty"`
	TokensOut     uint64 `json:"tokens_out,omitempty"`
	DurationMS    uint64 `json:"duration_ms,omitempty"`
	ExternalCalls uint32 `json:"external_calls,omitempty"`
}

type RuntimeReceiptRef struct {
	ReceiptHashHex string `json:"receipt_hash_hex,omitempty"`
	ReceiptURA     string `json:"receipt_ura,omitempty"`
}

type RuntimeReceiptAuthorityProof struct {
	ProofType          string                      `json:"proof_type,omitempty"`
	BindingKind        string                      `json:"binding_kind,omitempty"`
	Binding            map[string]any              `json:"binding,omitempty"`
	ProofPayloadBase64 string                      `json:"proof_payload_base64,omitempty"`
	ProofHashHex       string                      `json:"proof_hash_hex,omitempty"`
	Issuer             *RuntimeReceiptAgentBinding `json:"issuer,omitempty"`
	Signature          *RuntimeReceiptSignature    `json:"signature,omitempty"`
	AdmissionHook      string                      `json:"admission_hook,omitempty"`
}

func NewRuntimeReceiptFromJSON(raw []byte) (RuntimeReceipt, error) {
	receipt, err := decodeRuntimeReceiptProjectionFromJSON(raw)
	if err != nil {
		return RuntimeReceipt{}, err
	}
	if err := receipt.ValidateSummary(); err != nil {
		return RuntimeReceipt{}, err
	}
	return receipt, nil
}

func decodeRuntimeReceiptProjectionFromJSON(raw []byte) (RuntimeReceipt, error) {
	var dto struct {
		ReceiptID             string                        `json:"receipt_id"`
		ReceiptURA            string                        `json:"receipt_ura"`
		InvocationID          string                        `json:"invocation_id"`
		ReceiptType           string                        `json:"receipt_type"`
		State                 string                        `json:"state"`
		Index                 int64                         `json:"index"`
		TimestampUnixMS       int64                         `json:"timestamp_unix_ms"`
		PrevReceiptHashHex    string                        `json:"prev_receipt_hash_hex"`
		SelfHashHex           string                        `json:"self_hash_hex"`
		CleanupComplete       *bool                         `json:"cleanup_complete"`
		Reason                string                        `json:"reason"`
		ChildInvocationID     string                        `json:"child_invocation_id"`
		PayloadBase64         string                        `json:"payload_base64"`
		CallerBinding         *RuntimeReceiptAgentBinding   `json:"caller_binding"`
		CalleeBinding         *RuntimeReceiptAgentBinding   `json:"callee_binding"`
		SubjectBinding        *RuntimeReceiptSubjectBinding `json:"subject_binding"`
		InvocationNonceBase64 string                        `json:"invocation_nonce_base64"`
		CausalBindingKind     string                        `json:"causal_binding_kind"`
		CausalBinding         map[string]any                `json:"causal_binding"`
		CalleeSignature       *RuntimeReceiptSignature      `json:"callee_signature"`
		SignerBinding         *RuntimeReceiptAgentBinding   `json:"signer_binding"`
		HostAttestationBase64 string                        `json:"host_attestation_base64"`
		AuthorityBindingKind  string                        `json:"authority_binding_kind"`
		AuthorityBinding      map[string]any                `json:"authority_binding"`
		AbilityBinding        string                        `json:"ability_binding"`
		Failure               *RuntimeReceiptFailure        `json:"failure"`
		Usage                 *RuntimeReceiptUsage          `json:"usage"`
		SubjectRef            *RuntimeReceiptEntityRef      `json:"subject_ref"`
		DescriptorVersion     string                        `json:"descriptor_version"`
		SchemaHashHex         string                        `json:"schema_hash_hex"`
		ImplHashHex           string                        `json:"impl_hash_hex"`
		RuntimeEnv            string                        `json:"runtime_env"`
		AuthorityProof        *RuntimeReceiptAuthorityProof `json:"authority_proof"`
		InputHashHex          string                        `json:"input_hash_hex"`
		OutputHashHex         string                        `json:"output_hash_hex"`
		ParentReceipts        []RuntimeReceiptRef           `json:"parent_receipts"`
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
		Raw:                   rawMap,
		ReceiptID:             dto.ReceiptID,
		ReceiptURA:            dto.ReceiptURA,
		InvocationID:          dto.InvocationID,
		ReceiptType:           dto.ReceiptType,
		State:                 dto.State,
		Index:                 uint64(dto.Index),
		TimestampUnixMS:       dto.TimestampUnixMS,
		PrevReceiptHashHex:    dto.PrevReceiptHashHex,
		SelfHashHex:           dto.SelfHashHex,
		CleanupComplete:       dto.CleanupComplete,
		Reason:                dto.Reason,
		ChildInvocationID:     dto.ChildInvocationID,
		PayloadBase64:         dto.PayloadBase64,
		CallerBinding:         dto.CallerBinding,
		CalleeBinding:         dto.CalleeBinding,
		SubjectBinding:        dto.SubjectBinding,
		InvocationNonceBase64: dto.InvocationNonceBase64,
		CausalBindingKind:     dto.CausalBindingKind,
		CausalBinding:         cloneRuntimeReceiptObject(dto.CausalBinding),
		CalleeSignature:       dto.CalleeSignature,
		SignerBinding:         dto.SignerBinding,
		HostAttestationBase64: dto.HostAttestationBase64,
		AuthorityBindingKind:  dto.AuthorityBindingKind,
		AuthorityBinding:      cloneRuntimeReceiptObject(dto.AuthorityBinding),
		AbilityBinding:        dto.AbilityBinding,
		Failure:               dto.Failure,
		Usage:                 dto.Usage,
		SubjectRef:            dto.SubjectRef,
		DescriptorVersion:     dto.DescriptorVersion,
		SchemaHashHex:         dto.SchemaHashHex,
		ImplHashHex:           dto.ImplHashHex,
		RuntimeEnv:            dto.RuntimeEnv,
		AuthorityProof:        dto.AuthorityProof,
		InputHashHex:          dto.InputHashHex,
		OutputHashHex:         dto.OutputHashHex,
		ParentReceipts:        dto.ParentReceipts,
	}, nil
}

func (r RuntimeReceipt) HasCausalAnchor() bool {
	return strings.TrimSpace(r.ReceiptURA) != "" && strings.TrimSpace(r.SelfHashHex) != ""
}

// LifecycleState returns the canonical typed state carried by this receipt.
// Receipt projections must never use UNSPECIFIED as a fallback.
func (r RuntimeReceipt) LifecycleState() (InvocationLifecycleState, error) {
	state, err := ParseInvocationLifecycleState(r.State)
	if err != nil {
		return InvocationLifecycleUnspecified, err
	}
	if state == InvocationLifecycleUnspecified {
		return InvocationLifecycleUnspecified, invalidRuntimePayload(
			"runtime receipt lifecycle state must not be UNSPECIFIED",
			nil,
		)
	}
	return state, nil
}

func (r RuntimeReceipt) ValidateSummary() error {
	if r.Raw == nil {
		return invalidRuntimePayload("runtime receipt summary is missing raw proof projection", nil)
	}
	raw, err := json.Marshal(r.Raw)
	if err != nil {
		return invalidRuntimePayload("encode runtime receipt raw projection", err)
	}
	canonical, err := decodeRuntimeReceiptProjectionFromJSON(raw)
	if err != nil {
		return err
	}
	projected := r
	projected.Raw = nil
	canonical.Raw = nil
	if !reflect.DeepEqual(projected, canonical) {
		return invalidRuntimePayload("runtime receipt typed fields do not match raw projection", nil)
	}
	if strings.TrimSpace(r.InvocationID) == "" {
		return invalidRuntimePayload("runtime receipt summary is missing invocation_id", nil)
	}
	if strings.TrimSpace(r.ReceiptType) == "" {
		return invalidRuntimePayload("runtime receipt summary is missing receipt_type", nil)
	}
	if strings.TrimSpace(r.State) == "" {
		return invalidRuntimePayload("runtime receipt summary is missing state", nil)
	}
	if _, err := r.LifecycleState(); err != nil {
		return err
	}
	if _, err := r.PrevReceiptHash(); err != nil {
		return err
	}
	if _, err := r.SelfReceiptHash(); err != nil {
		return err
	}
	return r.ValidateProofFacts()
}

// ValidateProofFacts rejects receipt projections that omit canonical proof
// facts required for descriptor-bound audit and causal continuation.
func (r RuntimeReceipt) ValidateProofFacts() error {
	if err := requireRuntimeReceiptAgentBinding(r.CallerBinding, "caller_binding"); err != nil {
		return err
	}
	if err := requireRuntimeReceiptAgentBinding(r.CalleeBinding, "callee_binding"); err != nil {
		return err
	}
	if r.SubjectBinding == nil || strings.TrimSpace(r.SubjectBinding.URA) == "" {
		return invalidRuntimePayload("runtime receipt summary is missing subject_binding.ura", nil)
	}
	if _, err := runtimeReceiptBase64(r.InvocationNonceBase64, "invocation_nonce_base64", 16, false); err != nil {
		return err
	}
	if strings.TrimSpace(r.CausalBindingKind) == "" || r.CausalBinding == nil {
		return invalidRuntimePayload("runtime receipt summary is missing causal binding", nil)
	}
	if err := validateRuntimeReceiptCausalBinding(r.CausalBindingKind, r.CausalBinding); err != nil {
		return err
	}
	if err := requireRuntimeReceiptSignature(r.CalleeSignature, "callee_signature"); err != nil {
		return err
	}
	if _, err := runtimeReceiptBase64(r.CalleeSignature.SignatureBase64, "callee_signature.signature_base64", 0, false); err != nil {
		return err
	}
	if err := requireRuntimeReceiptAgentBinding(r.SignerBinding, "signer_binding"); err != nil {
		return err
	}
	if err := validateRuntimeReceiptSigningModel(r); err != nil {
		return err
	}
	if strings.TrimSpace(r.AuthorityBindingKind) == "" || r.AuthorityBinding == nil {
		return invalidRuntimePayload("runtime receipt summary is missing authority binding", nil)
	}
	authorityKind, ok := runtimeReceiptObjectText(r.AuthorityBinding, "kind")
	if !ok {
		return invalidRuntimePayload("runtime receipt summary is missing authority_binding.kind", nil)
	}
	if authorityKind != r.AuthorityBindingKind {
		return invalidRuntimePayload("runtime receipt authority_binding kind does not match authority_binding_kind", nil)
	}
	if strings.TrimSpace(r.AbilityBinding) == "" {
		return invalidRuntimePayload("runtime receipt summary is missing ability_binding", nil)
	}
	if r.SubjectRef == nil || strings.TrimSpace(r.SubjectRef.URA) == "" {
		return invalidRuntimePayload("runtime receipt summary is missing subject_ref.ura", nil)
	}
	if strings.TrimSpace(r.DescriptorVersion) == "" {
		return invalidRuntimePayload("runtime receipt summary is missing descriptor_version", nil)
	}
	if _, err := runtimeReceiptHash(r.SchemaHashHex, "schema_hash_hex"); err != nil {
		return err
	}
	if _, err := runtimeReceiptHash(r.ImplHashHex, "impl_hash_hex"); err != nil {
		return err
	}
	if strings.TrimSpace(r.RuntimeEnv) == "" {
		return invalidRuntimePayload("runtime receipt summary is missing runtime_env", nil)
	}
	if r.AuthorityProof == nil {
		return invalidRuntimePayload("runtime receipt summary is missing authority_proof", nil)
	}
	rawParents, ok := r.Raw["parent_receipts"]
	if !ok {
		return invalidRuntimePayload("runtime receipt summary is missing parent_receipts", nil)
	}
	if _, ok := rawParents.([]any); !ok {
		return invalidRuntimePayload("parent_receipts must be an array", nil)
	}
	return validateRuntimeReceiptCanonicalProofFacts(r)
}

func validateRuntimeReceiptSigningModel(r RuntimeReceipt) error {
	signerURA := strings.TrimSpace(r.SignerBinding.URA)
	calleeURA := strings.TrimSpace(r.CalleeBinding.URA)
	hostAttestation := strings.TrimSpace(r.HostAttestationBase64)
	if signerURA == calleeURA {
		if hostAttestation != "" {
			return invalidRuntimePayload(
				"self-signed runtime receipt must not carry host_attestation_base64",
				nil,
			)
		}
		return nil
	}
	if hostAttestation == "" {
		return invalidRuntimePayload(
			"hosted runtime receipt is missing host_attestation_base64",
			nil,
		)
	}
	_, err := runtimeReceiptBase64(
		hostAttestation,
		"host_attestation_base64",
		64,
		false,
	)
	return err
}

func (r RuntimeReceipt) PrevReceiptHash() ([]byte, error) {
	return runtimeReceiptHashValue(
		r.PrevReceiptHashHex,
		"prev_receipt_hash_hex",
		true,
	)
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
	return runtimeReceiptHashValue(value, field, false)
}

func runtimeReceiptHashValue(value string, field string, allowZero bool) ([]byte, error) {
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
	if !allowZero {
		allZero := true
		for _, value := range hash {
			allZero = allZero && value == 0
		}
		if allZero {
			return nil, invalidRuntimePayload(field+" must not be all-zero", nil)
		}
	}
	return hash, nil
}

func runtimeReceiptBase64(value string, field string, expectedLength int, allowEmpty bool) ([]byte, error) {
	value = strings.TrimSpace(value)
	if value == "" {
		if allowEmpty {
			return []byte{}, nil
		}
		return nil, invalidRuntimePayload(field+" is required", nil)
	}
	decoded, err := base64.StdEncoding.Strict().DecodeString(value)
	if err != nil {
		return nil, invalidRuntimePayload(field+" must be valid base64", err)
	}
	if len(decoded) == 0 && !allowEmpty {
		return nil, invalidRuntimePayload(field+" must decode to non-empty bytes", nil)
	}
	if expectedLength > 0 && len(decoded) != expectedLength {
		return nil, invalidRuntimePayload(
			fmt.Sprintf("%s must decode to exactly %d bytes", field, expectedLength),
			nil,
		)
	}
	return decoded, nil
}

func runtimeReceiptObjectText(value map[string]any, field string) (string, bool) {
	text, ok := value[field].(string)
	text = strings.TrimSpace(text)
	return text, ok && text != ""
}

func validateRuntimeReceiptRef(value any, field string) error {
	decoded, ok := value.(map[string]any)
	if !ok {
		return invalidRuntimePayload(field+" must be an object", nil)
	}
	hash, ok := decoded["receipt_hash_hex"].(string)
	if !ok {
		return invalidRuntimePayload(field+".receipt_hash_hex is required", nil)
	}
	if _, err := runtimeReceiptHash(hash, field+".receipt_hash_hex"); err != nil {
		return err
	}
	ura, ok := decoded["receipt_ura"].(string)
	if !ok || strings.TrimSpace(ura) == "" {
		return invalidRuntimePayload(field+".receipt_ura is required", nil)
	}
	return nil
}

func validateRuntimeReceiptCausalBinding(kind string, binding map[string]any) error {
	form, ok := runtimeReceiptObjectText(binding, "form")
	if !ok {
		return invalidRuntimePayload("runtime receipt summary is missing causal_binding.form", nil)
	}
	if form != kind {
		return invalidRuntimePayload(
			"runtime receipt causal_binding form does not match causal_binding_kind",
			nil,
		)
	}
	switch form {
	case "none":
		return nil
	case "scalar":
		return validateRuntimeReceiptRef(binding["receipt"], "causal_binding.receipt")
	case "list":
		prior, ok := binding["prior"].([]any)
		if !ok || len(prior) == 0 {
			return invalidRuntimePayload("causal_binding.prior must be a non-empty array", nil)
		}
		for index, receipt := range prior {
			if err := validateRuntimeReceiptRef(
				receipt,
				fmt.Sprintf("causal_binding.prior[%d]", index),
			); err != nil {
				return err
			}
		}
		return nil
	case "merkle":
		root, ok := binding["root_hex"].(string)
		if !ok {
			return invalidRuntimePayload("causal_binding.root_hex is required", nil)
		}
		if _, err := runtimeReceiptHash(root, "causal_binding.root_hex"); err != nil {
			return err
		}
		proofURA, ok := binding["proof_ura"].(string)
		if !ok || strings.TrimSpace(proofURA) == "" {
			return invalidRuntimePayload("causal_binding.proof_ura is required", nil)
		}
		return nil
	default:
		return invalidRuntimePayload("unsupported causal_binding form "+form, nil)
	}
}

func validateRuntimeReceiptCanonicalProofFacts(r RuntimeReceipt) error {
	for _, binding := range []struct {
		field   string
		ura     string
		profile string
	}{
		{field: "caller_binding", ura: r.CallerBinding.URA, profile: r.CallerBinding.Profile},
		{field: "callee_binding", ura: r.CalleeBinding.URA, profile: r.CalleeBinding.Profile},
		{field: "subject_binding", ura: r.SubjectBinding.URA, profile: r.SubjectBinding.Profile},
		{field: "signer_binding", ura: r.SignerBinding.URA, profile: r.SignerBinding.Profile},
	} {
		if strings.TrimSpace(binding.ura) == "" {
			return invalidRuntimePayload("runtime receipt summary is missing "+binding.field+".ura", nil)
		}
		if _, err := runtimeReceiptURAProfile(binding.profile, binding.field+".profile"); err != nil {
			return err
		}
	}

	authority, err := runtimeReceiptAuthorityBinding(r.AuthorityBinding, "authority_binding")
	if err != nil {
		return err
	}
	proof := r.AuthorityProof
	if strings.TrimSpace(proof.ProofType) == "" {
		return invalidRuntimePayload("runtime receipt summary is missing authority_proof.proof_type", nil)
	}
	if strings.TrimSpace(proof.BindingKind) == "" {
		return invalidRuntimePayload("runtime receipt summary is missing authority_proof.binding_kind", nil)
	}
	if proof.BindingKind != r.AuthorityBindingKind {
		return invalidRuntimePayload(
			"runtime receipt authority_proof binding_kind does not match authority_binding_kind",
			nil,
		)
	}
	proofBinding, err := runtimeReceiptAuthorityBinding(proof.Binding, "authority_proof.binding")
	if err != nil {
		return err
	}
	if !reflect.DeepEqual(proofBinding, authority) {
		return invalidRuntimePayload(
			"runtime receipt authority_proof binding does not match authority_binding",
			nil,
		)
	}

	if err := requireRuntimeReceiptAgentBinding(proof.Issuer, "authority_proof.issuer"); err != nil {
		return err
	}
	issuerProfile, err := runtimeReceiptURAProfile(
		proof.Issuer.Profile,
		"authority_proof.issuer.profile",
	)
	if err != nil {
		return err
	}
	calleeProfile, err := runtimeReceiptURAProfile(
		r.CalleeBinding.Profile,
		"callee_binding.profile",
	)
	if err != nil {
		return err
	}
	issuer := axoninv.NewAgentIdentity(proof.Issuer.URA, issuerProfile)
	callee := axoninv.NewAgentIdentity(r.CalleeBinding.URA, calleeProfile)
	if issuer != callee {
		return invalidRuntimePayload(
			"runtime receipt authority_proof issuer does not match callee_binding",
			nil,
		)
	}

	var proofSignature *axoninv.CalleeSignature
	if proof.Signature != nil {
		if err := requireRuntimeReceiptSignature(proof.Signature, "authority_proof.signature"); err != nil {
			return err
		}
		signature, err := runtimeReceiptBase64(
			proof.Signature.SignatureBase64,
			"authority_proof.signature.signature_base64",
			0,
			false,
		)
		if err != nil {
			return err
		}
		proofSignature = &axoninv.CalleeSignature{
			Algorithm: strings.TrimSpace(proof.Signature.Algorithm),
			Signature: signature,
			KeyIDHint: proof.Signature.KeyIDHint,
		}
	}

	if r.SubjectRef.Kind < int32(axoninv.EntityRefResource) ||
		r.SubjectRef.Kind > int32(axoninv.EntityRefDevice) {
		return invalidRuntimePayload("subject_ref.kind is not a canonical EntityRef kind", nil)
	}
	subjectProfile, err := runtimeReceiptURAProfile(r.SubjectRef.Profile, "subject_ref.profile")
	if err != nil {
		return err
	}
	subjectRef := &axoninv.EntityRef{
		Kind:    axoninv.EntityRefKind(r.SubjectRef.Kind),
		URA:     strings.TrimSpace(r.SubjectRef.URA),
		Profile: subjectProfile,
	}

	proofPayload, err := runtimeReceiptBase64(
		proof.ProofPayloadBase64,
		"authority_proof.proof_payload_base64",
		0,
		true,
	)
	if err != nil {
		return err
	}
	proofHash, err := runtimeReceiptHash32(
		proof.ProofHashHex,
		"authority_proof.proof_hash_hex",
	)
	if err != nil {
		return err
	}
	schemaHash, err := runtimeReceiptHash32(r.SchemaHashHex, "schema_hash_hex")
	if err != nil {
		return err
	}
	implHash, err := runtimeReceiptHash32(r.ImplHashHex, "impl_hash_hex")
	if err != nil {
		return err
	}
	inputHash, err := runtimeReceiptHash32(r.InputHashHex, "input_hash_hex")
	if err != nil {
		return err
	}
	outputHash, err := runtimeReceiptHash32(r.OutputHashHex, "output_hash_hex")
	if err != nil {
		return err
	}
	parentReceipts := make([]axoninv.ReceiptRef, 0, len(r.ParentReceipts))
	for index, parent := range r.ParentReceipts {
		parentHash, err := runtimeReceiptHash32(
			parent.ReceiptHashHex,
			fmt.Sprintf("parent_receipts[%d].receipt_hash_hex", index),
		)
		if err != nil {
			return err
		}
		if strings.TrimSpace(parent.ReceiptURA) == "" {
			return invalidRuntimePayload(
				fmt.Sprintf("runtime receipt summary is missing parent_receipts[%d].receipt_ura", index),
				nil,
			)
		}
		parentReceipts = append(parentReceipts, axoninv.ReceiptRef{
			ReceiptHash: parentHash,
			ReceiptURA:  strings.TrimSpace(parent.ReceiptURA),
		})
	}

	_, err = axoninv.TryNewReceiptProofFacts(
		subjectRef,
		strings.TrimSpace(r.DescriptorVersion),
		schemaHash,
		implHash,
		strings.TrimSpace(r.RuntimeEnv),
		axoninv.InvocationAuthorityProof{
			ProofType:     strings.TrimSpace(proof.ProofType),
			Binding:       &proofBinding,
			ProofPayload:  proofPayload,
			ProofHash:     proofHash,
			Issuer:        &issuer,
			Signature:     proofSignature,
			AdmissionHook: strings.TrimSpace(proof.AdmissionHook),
		},
		inputHash,
		outputHash,
		parentReceipts,
	)
	if err != nil {
		return invalidRuntimePayload(
			fmt.Sprintf("runtime receipt proof facts are not canonical: %v", err),
			err,
		)
	}
	return nil
}

func runtimeReceiptAuthorityBinding(value map[string]any, field string) (axoninv.AuthorityBinding, error) {
	if value == nil {
		return axoninv.AuthorityBinding{}, invalidRuntimePayload(
			"runtime receipt summary is missing "+field,
			nil,
		)
	}
	kind, err := requiredRuntimeReceiptObjectText(value, "kind", field+".kind")
	if err != nil {
		return axoninv.AuthorityBinding{}, err
	}
	switch kind {
	case "self":
		principal, err := requiredRuntimeReceiptObjectText(value, "principal_ura", field+".principal_ura")
		if err != nil {
			return axoninv.AuthorityBinding{}, err
		}
		return axoninv.SelfAuthority(principal), nil
	case "delegation":
		issuer, err := requiredRuntimeReceiptObjectText(value, "issuer_ura", field+".issuer_ura")
		if err != nil {
			return axoninv.AuthorityBinding{}, err
		}
		subject, err := requiredRuntimeReceiptObjectText(value, "subject_ura", field+".subject_ura")
		if err != nil {
			return axoninv.AuthorityBinding{}, err
		}
		caller, err := requiredRuntimeReceiptObjectText(value, "caller_ura", field+".caller_ura")
		if err != nil {
			return axoninv.AuthorityBinding{}, err
		}
		audience, err := requiredRuntimeReceiptObjectText(value, "audience", field+".audience")
		if err != nil {
			return axoninv.AuthorityBinding{}, err
		}
		scopes, err := runtimeReceiptTextList(value["scopes"], field+".scopes")
		if err != nil {
			return axoninv.AuthorityBinding{}, err
		}
		issuedAt, err := runtimeReceiptNonNegativeInt64(value["issued_at_ms"], field+".issued_at_ms")
		if err != nil {
			return axoninv.AuthorityBinding{}, err
		}
		expiresAt, err := runtimeReceiptNonNegativeInt64(value["expires_at_ms"], field+".expires_at_ms")
		if err != nil {
			return axoninv.AuthorityBinding{}, err
		}
		signatureText, err := requiredRuntimeReceiptObjectText(value, "signature_base64", field+".signature_base64")
		if err != nil {
			return axoninv.AuthorityBinding{}, err
		}
		signature, err := runtimeReceiptBase64(signatureText, field+".signature_base64", 64, false)
		if err != nil {
			return axoninv.AuthorityBinding{}, err
		}
		return axoninv.DelegatedAuthority(axoninv.DelegationProof{
			IssuerURA:   issuer,
			SubjectURA:  subject,
			CallerURA:   caller,
			Audience:    audience,
			Scopes:      scopes,
			IssuedAtMs:  issuedAt,
			ExpiresAtMs: expiresAt,
			Signature:   signature,
		}), nil
	case "capability":
		capability, err := requiredRuntimeReceiptObjectText(value, "capability_ura", field+".capability_ura")
		if err != nil {
			return axoninv.AuthorityBinding{}, err
		}
		return axoninv.CapabilityAuthority(capability), nil
	case "policy":
		policy, err := requiredRuntimeReceiptObjectText(value, "policy_ura", field+".policy_ura")
		if err != nil {
			return axoninv.AuthorityBinding{}, err
		}
		return axoninv.PolicyAuthority(policy), nil
	case "session":
		backend, err := requiredRuntimeReceiptObjectText(value, "backend_ura", field+".backend_ura")
		if err != nil {
			return axoninv.AuthorityBinding{}, err
		}
		user, err := requiredRuntimeReceiptObjectText(value, "user_ura", field+".user_ura")
		if err != nil {
			return axoninv.AuthorityBinding{}, err
		}
		sessionID, err := requiredRuntimeReceiptObjectText(value, "session_id", field+".session_id")
		if err != nil {
			return axoninv.AuthorityBinding{}, err
		}
		scopes, err := runtimeReceiptTextList(value["scopes"], field+".scopes")
		if err != nil {
			return axoninv.AuthorityBinding{}, err
		}
		audiences, err := runtimeReceiptTextList(value["audiences"], field+".audiences")
		if err != nil {
			return axoninv.AuthorityBinding{}, err
		}
		issuedAt, err := runtimeReceiptNonNegativeInt64(value["issued_at_ms"], field+".issued_at_ms")
		if err != nil {
			return axoninv.AuthorityBinding{}, err
		}
		expiresAt, err := runtimeReceiptNonNegativeInt64(value["expires_at_ms"], field+".expires_at_ms")
		if err != nil {
			return axoninv.AuthorityBinding{}, err
		}
		signatureText, err := requiredRuntimeReceiptObjectText(value, "signature_base64", field+".signature_base64")
		if err != nil {
			return axoninv.AuthorityBinding{}, err
		}
		signature, err := runtimeReceiptBase64(signatureText, field+".signature_base64", 64, false)
		if err != nil {
			return axoninv.AuthorityBinding{}, err
		}
		return axoninv.SessionAuthority(axoninv.SessionAuthorityBody{
			BackendURA:  backend,
			UserURA:     user,
			SessionID:   sessionID,
			Scopes:      scopes,
			Audiences:   audiences,
			IssuedAtMs:  issuedAt,
			ExpiresAtMs: expiresAt,
			Signature:   signature,
		}), nil
	case "bootstrap":
		principal, err := requiredRuntimeReceiptObjectText(value, "principal_ura", field+".principal_ura")
		if err != nil {
			return axoninv.AuthorityBinding{}, err
		}
		realm, err := requiredRuntimeReceiptObjectText(value, "realm", field+".realm")
		if err != nil {
			return axoninv.AuthorityBinding{}, err
		}
		ability, err := requiredRuntimeReceiptObjectText(value, "ability", field+".ability")
		if err != nil {
			return axoninv.AuthorityBinding{}, err
		}
		return axoninv.BootstrapAuthority(principal, realm, ability), nil
	default:
		return axoninv.AuthorityBinding{}, invalidRuntimePayload(
			fmt.Sprintf("%s is not canonical: %q", field+".kind", kind),
			nil,
		)
	}
}

func runtimeReceiptURAProfile(value string, field string) (axoninv.UraProfile, error) {
	profile, err := axoninv.ParseUraProfile(strings.TrimSpace(value))
	if err != nil {
		return "", invalidRuntimePayload(field+" is not canonical", err)
	}
	return profile, nil
}

func runtimeReceiptHash32(value string, field string) ([32]byte, error) {
	decoded, err := runtimeReceiptHash(value, field)
	if err != nil {
		return [32]byte{}, err
	}
	var hash [32]byte
	copy(hash[:], decoded)
	return hash, nil
}

func requiredRuntimeReceiptObjectText(value map[string]any, key string, field string) (string, error) {
	text, ok := runtimeReceiptObjectText(value, key)
	if !ok {
		return "", invalidRuntimePayload("runtime receipt summary is missing "+field, nil)
	}
	return text, nil
}

func runtimeReceiptTextList(value any, field string) ([]string, error) {
	var values []string
	switch items := value.(type) {
	case []string:
		values = append(values, items...)
	case []any:
		values = make([]string, 0, len(items))
		for index, item := range items {
			text, ok := item.(string)
			if !ok || strings.TrimSpace(text) == "" {
				return nil, invalidRuntimePayload(
					fmt.Sprintf("%s[%d] must be a non-empty string", field, index),
					nil,
				)
			}
			values = append(values, strings.TrimSpace(text))
		}
	default:
		return nil, invalidRuntimePayload(field+" must be a non-empty array", nil)
	}
	if len(values) == 0 {
		return nil, invalidRuntimePayload(field+" must be a non-empty array", nil)
	}
	for index, value := range values {
		if strings.TrimSpace(value) == "" {
			return nil, invalidRuntimePayload(
				fmt.Sprintf("%s[%d] must be a non-empty string", field, index),
				nil,
			)
		}
		values[index] = strings.TrimSpace(value)
	}
	return values, nil
}

func runtimeReceiptNonNegativeInt64(value any, field string) (int64, error) {
	switch number := value.(type) {
	case int:
		if number >= 0 {
			return int64(number), nil
		}
	case int64:
		if number >= 0 {
			return number, nil
		}
	case uint64:
		if number <= uint64(^uint64(0)>>1) {
			return int64(number), nil
		}
	case float64:
		if number >= 0 && number <= float64(^uint64(0)>>1) && number == float64(int64(number)) {
			return int64(number), nil
		}
	}
	return 0, invalidRuntimePayload(field+" must be a non-negative integer", nil)
}

func requireRuntimeReceiptAgentBinding(binding *RuntimeReceiptAgentBinding, field string) error {
	if binding == nil || strings.TrimSpace(binding.URA) == "" {
		return invalidRuntimePayload("runtime receipt summary is missing "+field+".ura", nil)
	}
	return nil
}

func requireRuntimeReceiptSignature(signature *RuntimeReceiptSignature, field string) error {
	if signature == nil || strings.TrimSpace(signature.SignatureBase64) == "" {
		return invalidRuntimePayload("runtime receipt summary is missing "+field+".signature_base64", nil)
	}
	if strings.TrimSpace(signature.Algorithm) == "" {
		return invalidRuntimePayload("runtime receipt summary is missing "+field+".algorithm", nil)
	}
	return nil
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

// LifecycleState returns the fail-closed canonical terminal lifecycle state.
func (r InvocationResult) LifecycleState() (InvocationLifecycleState, error) {
	state, err := ParseInvocationLifecycleState(r.terminalState)
	if err != nil {
		return InvocationLifecycleUnspecified, err
	}
	if !state.IsTerminal() {
		return InvocationLifecycleUnspecified, invalidRuntimePayload(
			"invocation result lifecycle state must be terminal",
			nil,
		)
	}
	return state, nil
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

func (r InvocationResult) ElapsedMS() int64 {
	return r.elapsedMS
}

// AdmissionReceipt returns the pre-execution admission checkpoint, when the
// runtime emitted one.
func (r InvocationResult) AdmissionReceipt() json.RawMessage {
	return append(json.RawMessage(nil), r.admissionReceipt...)
}

// TerminalReceipt returns the execution terminal checkpoint.
func (r InvocationResult) TerminalReceipt() json.RawMessage {
	return append(json.RawMessage(nil), r.terminalReceipt...)
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
		ElapsedMS         int64           `json:"elapsed_ms"`
		AdmissionReceipt  json.RawMessage `json:"admission_receipt"`
		TerminalReceipt   json.RawMessage `json:"terminal_receipt"`
		Error             json.RawMessage `json:"error"`
	}
	if err := json.Unmarshal(raw, &dto); err != nil {
		return InvocationResult{}, invalidRuntimePayload(fmt.Sprintf("decode invocation result JSON: %v", err), err)
	}
	if err := rejectRetiredTopLevelReceiptAlias(raw, "invocation result"); err != nil {
		return InvocationResult{}, err
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
	terminalReceipt := cloneOptionalJSON(dto.TerminalReceipt)
	admissionReceipt := cloneOptionalJSON(dto.AdmissionReceipt)
	if err := validateInvocationResultReceiptPresence(
		*dto.OK,
		dto.TerminalState,
		failure,
		admissionReceipt,
		terminalReceipt,
	); err != nil {
		return InvocationResult{}, err
	}
	admissionReceiptSummary, err := decodeRuntimeReceiptSummary(admissionReceipt)
	if err != nil {
		return InvocationResult{}, err
	}
	terminalReceiptSummary, err := decodeRuntimeReceiptSummary(terminalReceipt)
	if err != nil {
		return InvocationResult{}, err
	}
	if err := validateInvocationResultReceiptTopology(
		*dto.OK,
		dto.TerminalState,
		dto.InvocationID,
		admissionReceiptSummary,
		terminalReceiptSummary,
	); err != nil {
		return InvocationResult{}, err
	}
	outputJSON, err := normalizedInvocationOutputJSON(
		dto.OutputJSON,
		dto.OutputBase64,
		dto.OutputContentType,
	)
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
		outputJSON:              outputJSON,
		elapsedMS:               dto.ElapsedMS,
		admissionReceipt:        admissionReceipt,
		terminalReceipt:         terminalReceipt,
		admissionReceiptSummary: admissionReceiptSummary,
		terminalReceiptSummary:  terminalReceiptSummary,
		failure:                 failure,
	}, nil
}

func normalizedInvocationOutputJSON(
	raw json.RawMessage,
	outputBase64 string,
	outputContentType string,
) (json.RawMessage, error) {
	if len(raw) != 0 && string(raw) != "null" {
		return append(json.RawMessage(nil), raw...), nil
	}
	if !runtimeResultContentTypeIsJSON(outputContentType) || strings.TrimSpace(outputBase64) == "" {
		return append(json.RawMessage(nil), raw...), nil
	}
	payload, err := base64.StdEncoding.DecodeString(strings.TrimSpace(outputBase64))
	if err != nil {
		return nil, invalidRuntimePayload("output_base64 must be valid base64 for JSON invocation output", err)
	}
	if !json.Valid(payload) {
		return nil, invalidRuntimePayload("output_base64 must decode to valid JSON for JSON invocation output", nil)
	}
	return append(json.RawMessage(nil), payload...), nil
}

func runtimeResultContentTypeIsJSON(contentType string) bool {
	return strings.Contains(strings.ToLower(contentType), "json")
}

func validateInvocationResultReceiptPresence(
	ok bool,
	terminalState string,
	failure *InvocationFailure,
	admissionReceipt json.RawMessage,
	terminalReceipt json.RawMessage,
) error {
	hasAdmission := len(admissionReceipt) != 0
	hasTerminal := len(terminalReceipt) != 0
	if hasAdmission != hasTerminal {
		return invalidRuntimePayload(
			"invocation result must carry both admission_receipt and terminal_receipt or neither",
			nil,
		)
	}
	if hasAdmission {
		return nil
	}
	if ok {
		return invalidRuntimePayload("successful invocation result requires canonical receipt checkpoints", nil)
	}
	state, err := ParseInvocationLifecycleState(terminalState)
	if err != nil {
		return err
	}
	if state != InvocationLifecycleFailed {
		return invalidRuntimePayload("receipt-free invocation result must use terminal_state Failed", nil)
	}
	if failure == nil || !isCanonicalPreAdmissionErrorStage(failure.stage) {
		return invalidRuntimePayload(
			"receipt-free invocation result requires a typed pre-admission error stage",
			nil,
		)
	}
	return nil
}

func isCanonicalPreAdmissionErrorStage(stage string) bool {
	switch stage {
	case "global_admission",
		"caller_authentication",
		"authority_validation",
		"bootstrap_authorization",
		"quota",
		"ability_resolution",
		"ability_policy",
		"request_validation":
		return true
	default:
		return false
	}
}

func validateInvocationResultReceiptTopology(
	ok bool,
	terminalState string,
	invocationID string,
	admission *RuntimeReceipt,
	terminal *RuntimeReceipt,
) error {
	if admission == nil && terminal == nil {
		return nil
	}
	if admission == nil || terminal == nil {
		return invalidRuntimePayload(
			"invocation result must carry both admission_receipt and terminal_receipt or neither",
			nil,
		)
	}
	admissionState, err := admission.LifecycleState()
	if err != nil {
		return err
	}
	if admissionState != InvocationLifecycleAdmitted {
		return invalidRuntimePayload(
			"admission_receipt does not carry a canonical admission state",
			nil,
		)
	}
	if admission.ReceiptType != canonicalReceiptType(admissionState) {
		return invalidRuntimePayload(
			"admission_receipt does not carry canonical receipt_type admitted",
			nil,
		)
	}
	if admission.CleanupComplete == nil || *admission.CleanupComplete {
		return invalidRuntimePayload(
			"admission_receipt cleanup_complete must be false",
			nil,
		)
	}
	terminalReceiptState, err := terminal.LifecycleState()
	if err != nil {
		return err
	}
	if !terminalReceiptState.IsTerminal() {
		return invalidRuntimePayload(
			"terminal_receipt does not carry a canonical terminal state",
			nil,
		)
	}
	if terminal.ReceiptType != canonicalReceiptType(terminalReceiptState) {
		return invalidRuntimePayload(
			"terminal_receipt receipt_type does not match its terminal state",
			nil,
		)
	}
	resultTerminalState, err := ParseInvocationLifecycleState(terminalState)
	if err != nil {
		return err
	}
	if terminalReceiptState != resultTerminalState {
		return invalidRuntimePayload(
			"terminal_receipt state does not match invocation terminal_state",
			nil,
		)
	}
	if ok != (terminalReceiptState == InvocationLifecycleCompleted) {
		return invalidRuntimePayload(
			"invocation result ok flag does not match terminal receipt state",
			nil,
		)
	}
	if terminal.CleanupComplete == nil || !*terminal.CleanupComplete {
		return invalidRuntimePayload(
			"terminal_receipt cleanup_complete must be true",
			nil,
		)
	}
	if terminal.Index <= admission.Index {
		return invalidRuntimePayload(
			"terminal_receipt index must follow admission_receipt index",
			nil,
		)
	}
	if admission.InvocationID != terminal.InvocationID {
		return invalidRuntimePayload(
			"admission_receipt and terminal_receipt bind different invocations",
			nil,
		)
	}
	if terminal.TimestampUnixMS < admission.TimestampUnixMS {
		return invalidRuntimePayload(
			"terminal_receipt timestamp precedes admission_receipt",
			nil,
		)
	}
	if strings.TrimSpace(invocationID) != "" && invocationID != terminal.InvocationID {
		return invalidRuntimePayload(
			"invocation result id does not match canonical receipt checkpoints",
			nil,
		)
	}
	bindingsMatch := reflect.DeepEqual(admission.CallerBinding, terminal.CallerBinding) &&
		reflect.DeepEqual(admission.CalleeBinding, terminal.CalleeBinding) &&
		reflect.DeepEqual(admission.SubjectBinding, terminal.SubjectBinding) &&
		admission.InvocationNonceBase64 == terminal.InvocationNonceBase64 &&
		admission.CausalBindingKind == terminal.CausalBindingKind &&
		reflect.DeepEqual(admission.CausalBinding, terminal.CausalBinding) &&
		reflect.DeepEqual(admission.SignerBinding, terminal.SignerBinding) &&
		admission.HostAttestationBase64 == terminal.HostAttestationBase64 &&
		admission.AuthorityBindingKind == terminal.AuthorityBindingKind &&
		reflect.DeepEqual(admission.AuthorityBinding, terminal.AuthorityBinding) &&
		admission.AbilityBinding == terminal.AbilityBinding &&
		reflect.DeepEqual(admission.SubjectRef, terminal.SubjectRef) &&
		admission.DescriptorVersion == terminal.DescriptorVersion &&
		admission.SchemaHashHex == terminal.SchemaHashHex &&
		admission.ImplHashHex == terminal.ImplHashHex &&
		admission.RuntimeEnv == terminal.RuntimeEnv &&
		reflect.DeepEqual(admission.AuthorityProof, terminal.AuthorityProof) &&
		admission.InputHashHex == terminal.InputHashHex &&
		reflect.DeepEqual(admission.ParentReceipts, terminal.ParentReceipts)
	if !bindingsMatch {
		return invalidRuntimePayload(
			"canonical receipt checkpoints contain conflicting invocation bindings",
			nil,
		)
	}
	return nil
}

func canonicalReceiptType(state InvocationLifecycleState) string {
	switch state {
	case InvocationLifecycleAccepted:
		return "accepted"
	case InvocationLifecycleAdmitted:
		return "admitted"
	case InvocationLifecycleDispatched:
		return "dispatched"
	case InvocationLifecycleRunning:
		return "running"
	case InvocationLifecycleCompleted:
		return "completed"
	case InvocationLifecycleFailed:
		return "failed"
	case InvocationLifecycleTimedOut:
		return "timed_out"
	case InvocationLifecycleCancelled:
		return "cancelled"
	default:
		return ""
	}
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
	clone.CausalBinding = cloneRuntimeReceiptObject(receipt.CausalBinding)
	clone.AuthorityBinding = cloneRuntimeReceiptObject(receipt.AuthorityBinding)
	if receipt.CleanupComplete != nil {
		value := *receipt.CleanupComplete
		clone.CleanupComplete = &value
	}
	return &clone
}

func cloneRuntimeReceiptObject(value map[string]any) map[string]any {
	if value == nil {
		return nil
	}
	encoded, err := json.Marshal(value)
	if err != nil {
		return nil
	}
	var clone map[string]any
	if err := json.Unmarshal(encoded, &clone); err != nil {
		return nil
	}
	return clone
}

func cloneOptionalJSON(raw json.RawMessage) json.RawMessage {
	if len(raw) == 0 || string(raw) == "null" {
		return nil
	}
	return append(json.RawMessage(nil), raw...)
}

func rejectRetiredTopLevelReceiptAlias(raw []byte, projection string) error {
	var fields map[string]json.RawMessage
	if err := json.Unmarshal(raw, &fields); err != nil {
		return invalidRuntimePayload(fmt.Sprintf("decode %s JSON: %v", projection, err), err)
	}
	if _, ok := fields["receipt"]; ok {
		return invalidRuntimePayload(projection+" must use terminal_receipt; retired receipt alias is not accepted", nil)
	}
	return nil
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
	control         InvocationControlCapability
	requestAccepted bool
	deduplicated    bool
	cancelled       bool
	state           string
	terminal        bool
}

func (c InvocationCancel) ControlCapability() InvocationControlCapability {
	return c.control
}

func (c InvocationCancel) RequestAccepted() bool {
	return c.requestAccepted
}

func (c InvocationCancel) Deduplicated() bool {
	return c.deduplicated
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
	return newInvocationCancelFromJSON(raw, nil)
}

func newInvocationCancelFromJSON(raw []byte, expectedControl *InvocationControlCapability) (InvocationCancel, error) {
	var dto struct {
		HandleID        uint64 `json:"handle_id"`
		RequestAccepted *bool  `json:"request_accepted"`
		Deduplicated    *bool  `json:"deduplicated"`
		Cancelled       bool   `json:"cancelled"`
		State           string `json:"state"`
		Terminal        bool   `json:"terminal"`
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
	if dto.RequestAccepted == nil {
		return InvocationCancel{}, invalidRuntimePayload("request_accepted is required", nil)
	}
	if dto.Deduplicated == nil {
		return InvocationCancel{}, invalidRuntimePayload("deduplicated is required", nil)
	}
	var control InvocationControlCapability
	var err error
	if expectedControl != nil {
		if expectedControl.adapterHandleID() != dto.HandleID {
			return InvocationCancel{}, invalidRuntimePayload("handle_id does not match invocation control capability", nil)
		}
		control = *expectedControl
	} else {
		control, err = newSnapshotInvocationControlCapability(dto.HandleID)
		if err != nil {
			return InvocationCancel{}, err
		}
	}
	return InvocationCancel{
		control:         control,
		requestAccepted: *dto.RequestAccepted,
		deduplicated:    *dto.Deduplicated,
		cancelled:       dto.Cancelled,
		state:           dto.State,
		terminal:        dto.Terminal,
	}, nil
}

// InvocationHandle is the submitted invocation observation handle projection.
type InvocationHandle struct {
	control  InvocationControlCapability
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

func (h InvocationHandle) ControlCapability() InvocationControlCapability {
	return h.control
}

func (h InvocationHandle) controlCapability() (InvocationControlCapability, error) {
	if !h.control.valid() {
		return InvocationControlCapability{}, invalidRuntimePayload("runtime-bound invocation control capability is required", nil)
	}
	return h.control, nil
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
	return newInvocationHandleSnapshotFromJSON(raw, nil)
}

func newRuntimeInvocationHandleFromJSON(raw []byte) (InvocationHandle, error) {
	return newInvocationHandleSnapshotFromJSON(raw, nil, true)
}

func newInvocationHandleSnapshotFromJSON(raw []byte, expectedControl *InvocationControlCapability, trusted ...bool) (InvocationHandle, error) {
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
	if expectedControl != nil && expectedControl.adapterHandleID() != dto.HandleID {
		return InvocationHandle{}, invalidRuntimePayload("handle_id does not match invocation control capability", nil)
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
	var control InvocationControlCapability
	var err error
	switch {
	case expectedControl != nil:
		control = *expectedControl
	case len(trusted) > 0 && trusted[0]:
		control, err = newRuntimeInvocationControlCapability(dto.HandleID)
	default:
		control, err = newSnapshotInvocationControlCapability(dto.HandleID)
	}
	if err != nil {
		return InvocationHandle{}, err
	}
	return InvocationHandle{
		control:  control,
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
