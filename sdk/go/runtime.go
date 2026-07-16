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
	ProofPayloadBase64 string                      `json:"proof_payload_base64,omitempty"`
	ProofHashHex       string                      `json:"proof_hash_hex,omitempty"`
	Issuer             *RuntimeReceiptAgentBinding `json:"issuer,omitempty"`
	Signature          *RuntimeReceiptSignature    `json:"signature,omitempty"`
	AdmissionHook      string                      `json:"admission_hook,omitempty"`
}

func NewRuntimeReceiptFromJSON(raw []byte) (RuntimeReceipt, error) {
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
		SelectedNodeID    string          `json:"selected_node_id"`
		SchedulingReason  string          `json:"scheduling_reason"`
		ElapsedMS         int64           `json:"elapsed_ms"`
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
	terminalReceipt := cloneOptionalJSON(dto.TerminalReceipt)
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
