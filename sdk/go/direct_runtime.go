//go:build easynet_direct_runtime

package easynet

import (
	"context"
	"crypto/sha256"
	"crypto/tls"
	"encoding/base64"
	"encoding/hex"
	"encoding/json"
	"errors"
	"fmt"
	"io"
	"net"
	"net/url"
	"strconv"
	"strings"
	"sync"
	"time"

	"easynet.run/cli/sdk/go/internal/axonpb"
	"google.golang.org/grpc"
	"google.golang.org/grpc/codes"
	"google.golang.org/grpc/credentials"
	"google.golang.org/grpc/credentials/insecure"
	"google.golang.org/grpc/status"
)

const (
	defaultDirectRuntimeDialTimeout   = 3 * time.Second
	defaultDirectRuntimeInvokeTimeout = 60 * time.Second
	defaultDirectRuntimeTransportName = "direct-axon-grpc-uds"
	defaultURAProfile                 = "easynet-strict-v2"
	directSignedDescriptorRefMetadata = "x-easynet-signed-descriptor-ref"
)

// DirectDaemonRuntimeConnector opens a concrete daemon Runtime Core transport
// over the daemon's Axon Invocation gRPC endpoint.
type DirectDaemonRuntimeConnector struct {
	ControlPath string
	Reader      ControlDiscoveryReader

	mu                   sync.Mutex
	handle               RuntimeTransport
	identity             *IdentityClient
	closeHandleTransport bool
	transports           map[*DirectDaemonRuntimeTransport]struct{}
	closed               bool
}

// DirectDaemonRuntimeConnectorOptions configures a concrete direct runtime
// connector. HandleTransport is the SDK-owned Runtime Core surface for
// prepare/submit/handle operations; direct gRPC remains the invoke/stream/bidi
// transport.
type DirectDaemonRuntimeConnectorOptions struct {
	ControlPath          string
	Reader               ControlDiscoveryReader
	HandleTransport      RuntimeTransport
	Identity             *IdentityClient
	CloseHandleTransport bool
}

// NewDirectDaemonRuntimeConnector creates a direct daemon Runtime connector.
func NewDirectDaemonRuntimeConnector(controlPath string, reader ControlDiscoveryReader) *DirectDaemonRuntimeConnector {
	return NewDirectDaemonRuntimeConnectorWithOptions(DirectDaemonRuntimeConnectorOptions{
		ControlPath: controlPath,
		Reader:      reader,
	})
}

// NewDirectDaemonRuntimeConnectorWithOptions creates a direct daemon Runtime
// connector with explicit handle-transport ownership.
func NewDirectDaemonRuntimeConnectorWithOptions(options DirectDaemonRuntimeConnectorOptions) *DirectDaemonRuntimeConnector {
	reader := options.Reader
	if reader == nil {
		reader = FileControlDiscoveryReader{}
	}
	return &DirectDaemonRuntimeConnector{
		ControlPath:          options.ControlPath,
		Reader:               reader,
		handle:               options.HandleTransport,
		identity:             options.Identity,
		closeHandleTransport: options.CloseHandleTransport,
		transports:           map[*DirectDaemonRuntimeTransport]struct{}{},
	}
}

// WithHandleTransport sets the Runtime Core handle transport used for
// prepare/submit/handle operations. The caller must set closeOnConnectorClose
// only when this connector owns the handle transport lifecycle.
func (c *DirectDaemonRuntimeConnector) WithHandleTransport(handle RuntimeTransport, closeOnConnectorClose bool) *DirectDaemonRuntimeConnector {
	if c == nil {
		return c
	}
	c.mu.Lock()
	defer c.mu.Unlock()
	if c.closed {
		return c
	}
	c.handle = handle
	c.closeHandleTransport = closeOnConnectorClose
	return c
}

// WithIdentityClient sets the Directory + Identity facade used to project
// DescriptorRef values before direct daemon dispatch.
func (c *DirectDaemonRuntimeConnector) WithIdentityClient(identity *IdentityClient) *DirectDaemonRuntimeConnector {
	if c == nil {
		return c
	}
	c.mu.Lock()
	defer c.mu.Unlock()
	if c.closed {
		return c
	}
	c.identity = identity
	return c
}

func (c *DirectDaemonRuntimeConnector) Resolve(ctx context.Context, optionsJSON []byte) ([]byte, error) {
	if err := c.requireOpen(ctx); err != nil {
		return nil, err
	}
	options, err := decodeConnectOptionsJSON(optionsJSON)
	if err != nil {
		return nil, err
	}
	controlPath := options.ControlPath
	if controlPath == "" {
		controlPath = c.ControlPath
	}
	endpoint := options.Endpoint
	if endpoint == "" {
		discovery, err := c.Reader.ReadControlDiscovery(ctx, controlPath)
		if err != nil {
			return nil, err
		}
		if discovery.InvocationEndpoint == "" {
			return nil, &SDKError{
				Code:      ErrControlOnly,
				Stage:     "direct_runtime.resolve",
				Retry:     RetrySafe,
				Retryable: RetryableForHint(RetrySafe),
				Message:   "control discovery did not advertise invocation_endpoint",
				Details:   map[string]any{"control_path": controlPath},
			}
		}
		endpoint = discovery.InvocationEndpoint
	}
	return directRuntimeEndpointJSON(RuntimeEndpoint{
		Endpoint:        endpoint,
		ControlPath:     controlPath,
		ProtocolVersion: "axon.v1.Invocation",
	}, options)
}

func (c *DirectDaemonRuntimeConnector) Handshake(ctx context.Context, endpointJSON []byte) (RuntimeTransport, []byte, error) {
	if err := c.requireOpen(ctx); err != nil {
		return nil, nil, err
	}
	options, endpoint, err := decodeDirectRuntimeEndpoint(endpointJSON)
	if err != nil {
		return nil, nil, err
	}
	handleTransport, identity, _ := c.transportConfig(ctx)
	transport, err := OpenDirectDaemonRuntimeTransport(ctx, endpoint.Endpoint, DirectRuntimeOptions{
		DialTimeoutMS:   options.DialTimeoutMS,
		InvokeTimeoutMS: options.InvokeTimeoutMS,
		MaxMessageBytes: options.MaxMessageBytes,
		HandleTransport: handleTransport,
		Identity:        identity,
	})
	if err != nil {
		return nil, nil, err
	}
	c.mu.Lock()
	if c.closed {
		c.mu.Unlock()
		_ = transport.Close(ctx)
		return nil, nil, invalidRuntimeClient("runtime connector is closed")
	}
	c.transports[transport] = struct{}{}
	c.mu.Unlock()
	raw, err := json.Marshal(map[string]any{
		"transport":     defaultDirectRuntimeTransportName,
		"endpoint":      endpoint.Endpoint,
		"protocol":      "axon.v1.Invocation",
		"unary":         true,
		"stream":        true,
		"bidi":          true,
		"prepare":       handleTransport != nil,
		"submit_signed": handleTransport != nil,
	})
	if err != nil {
		_ = transport.Close(ctx)
		return nil, nil, invalidRuntimePayload(fmt.Sprintf("encode direct runtime handshake: %v", err), err)
	}
	return transport, raw, nil
}

func (c *DirectDaemonRuntimeConnector) Close(ctx context.Context) error {
	if c == nil {
		return invalidRuntimeClient("runtime connector is not initialized")
	}
	if ctx == nil {
		return invalidRuntimeClient("context is required")
	}
	c.mu.Lock()
	if c.closed {
		c.mu.Unlock()
		return nil
	}
	handle := c.handle
	closeHandle := c.closeHandleTransport
	c.handle = nil
	c.identity = nil
	c.closeHandleTransport = false
	transports := make([]*DirectDaemonRuntimeTransport, 0, len(c.transports))
	for transport := range c.transports {
		transports = append(transports, transport)
	}
	c.transports = map[*DirectDaemonRuntimeTransport]struct{}{}
	c.closed = true
	c.mu.Unlock()

	var closeErr error
	for _, transport := range transports {
		if err := transport.Close(ctx); err != nil && closeErr == nil {
			closeErr = err
		}
	}
	if closeHandle && handle != nil {
		closeErr = errors.Join(closeErr, handle.Close(ctx))
	}
	return closeErr
}

func (c *DirectDaemonRuntimeConnector) requireOpen(ctx context.Context) error {
	if c == nil || c.Reader == nil {
		return invalidRuntimeClient("runtime connector is not initialized")
	}
	if ctx == nil {
		return invalidRuntimeClient("context is required")
	}
	c.mu.Lock()
	defer c.mu.Unlock()
	if c.closed {
		return invalidRuntimeClient("runtime connector is closed")
	}
	return nil
}

func (c *DirectDaemonRuntimeConnector) transportConfig(ctx context.Context) (RuntimeTransport, *IdentityClient, bool) {
	if c == nil || ctx == nil {
		return nil, nil, false
	}
	c.mu.Lock()
	defer c.mu.Unlock()
	return c.handle, c.identity, c.closeHandleTransport
}

// DirectRuntimeOptions are SDK-internal direct daemon transport knobs.
type DirectRuntimeOptions struct {
	DialTimeoutMS        int64
	InvokeTimeoutMS      int64
	MaxMessageBytes      int
	HandleTransport      RuntimeTransport
	Identity             *IdentityClient
	CloseHandleTransport bool
}

// DirectDaemonRuntimeTransport is a concrete RuntimeTransport over Axon gRPC UDS.
type DirectDaemonRuntimeTransport struct {
	mu                   sync.Mutex
	conn                 *grpc.ClientConn
	client               axonpb.InvocationClient
	endpoint             string
	invokeTimeout        time.Duration
	handle               RuntimeTransport
	identity             *IdentityClient
	closeHandleTransport bool
	nextHandleID         uint64
	handles              map[uint64]directRuntimeHandleSnapshot
	closed               bool
}

type directRuntimeHandleSnapshot struct {
	handleID uint64
	state    string
	terminal bool
	events   []map[string]any
	result   json.RawMessage
}

// OpenDirectDaemonRuntimeTransport opens a direct daemon Runtime transport.
func OpenDirectDaemonRuntimeTransport(ctx context.Context, endpoint string, options DirectRuntimeOptions) (*DirectDaemonRuntimeTransport, error) {
	if ctx == nil {
		return nil, invalidRuntimeClient("context is required")
	}
	if strings.TrimSpace(endpoint) == "" {
		return nil, invalidRuntimeClient("endpoint is required")
	}
	if options.Identity == nil {
		return nil, invalidProfileClient(directoryIdentityProfile, "identity client is required for direct runtime descriptor projection")
	}
	dialTimeout := durationFromMillis(options.DialTimeoutMS, defaultDirectRuntimeDialTimeout)
	invokeTimeout := durationFromMillis(options.InvokeTimeoutMS, defaultDirectRuntimeInvokeTimeout)
	dialCtx, cancel := context.WithTimeout(ctx, dialTimeout)
	defer cancel()
	target, transportOptions, err := directRuntimeDialTarget(endpoint)
	if err != nil {
		return nil, err
	}
	dialOptions := append(transportOptions, grpc.WithBlock())
	if options.MaxMessageBytes > 0 {
		dialOptions = append(
			dialOptions,
			grpc.WithDefaultCallOptions(
				grpc.MaxCallRecvMsgSize(options.MaxMessageBytes),
				grpc.MaxCallSendMsgSize(options.MaxMessageBytes),
			),
		)
	}
	conn, err := grpc.DialContext(dialCtx, target, dialOptions...)
	if err != nil {
		return nil, directRuntimeError(
			"daemon invocation endpoint is not ready",
			ErrDaemonOffline,
			RetrySafe,
			map[string]any{"endpoint": endpoint},
			err,
		)
	}
	return &DirectDaemonRuntimeTransport{
		conn:                 conn,
		client:               axonpb.NewInvocationClient(conn),
		endpoint:             endpoint,
		invokeTimeout:        invokeTimeout,
		handle:               options.HandleTransport,
		identity:             options.Identity,
		closeHandleTransport: options.CloseHandleTransport,
		handles:              map[uint64]directRuntimeHandleSnapshot{},
	}, nil
}

func (t *DirectDaemonRuntimeTransport) Invoke(ctx context.Context, draftJSON []byte) ([]byte, error) {
	client, timeout, err := t.requireOpen(ctx)
	if err != nil {
		return nil, err
	}
	draft, request, err := directInvokeRequestFromDraftJSON(ctx, t.identity, draftJSON)
	if err != nil {
		return nil, err
	}
	callCtx, cancel := context.WithTimeout(ctx, timeout)
	defer cancel()
	response, err := client.Invoke(callCtx, request)
	if err != nil {
		return nil, directRuntimeGRPCError(err, t.endpoint)
	}
	return directInvokeResponseJSON(draft, response)
}

func (t *DirectDaemonRuntimeTransport) OpenStream(ctx context.Context, draftJSON []byte) (StreamTransport, []byte, error) {
	client, timeout, err := t.requireOpen(ctx)
	if err != nil {
		return nil, nil, err
	}
	_, request, err := directStreamRequestFromDraftJSON(ctx, t.identity, draftJSON)
	if err != nil {
		return nil, nil, err
	}
	callCtx, cancel := context.WithTimeout(ctx, timeout)
	stream, err := client.InvokeStream(callCtx, request)
	if err != nil {
		cancel()
		return nil, nil, directRuntimeGRPCError(err, t.endpoint)
	}
	transport := newDirectDaemonStreamTransport(stream, cancel, t.endpoint)
	openJSON, err := json.Marshal(map[string]any{
		"stream_id":           transport.streamID,
		"state":               "Open",
		"max_buffered_events": MaxStreamBufferedEvents,
	})
	if err != nil {
		_ = transport.Close(ctx)
		return nil, nil, invalidRuntimePayload(fmt.Sprintf("encode direct stream open JSON: %v", err), err)
	}
	return transport, openJSON, nil
}

func (t *DirectDaemonRuntimeTransport) OpenBidi(ctx context.Context, draftJSON []byte, streamsJSON []byte) (BidiTransport, []byte, error) {
	client, timeout, err := t.requireOpen(ctx)
	if err != nil {
		return nil, nil, err
	}
	draft, err := NewInvocationDraftFromJSON(draftJSON)
	if err != nil {
		return nil, nil, err
	}
	streams, err := directBidiStreamDescriptors(streamsJSON)
	if err != nil {
		return nil, nil, err
	}
	openFrame, err := directBidiOpenFrame(ctx, t.identity, draft, streams)
	if err != nil {
		return nil, nil, err
	}
	callCtx, cancel := context.WithTimeout(ctx, timeout)
	stream, err := client.InvokeBidi(callCtx)
	if err != nil {
		cancel()
		return nil, nil, directRuntimeGRPCError(err, t.endpoint)
	}
	transport, err := newDirectDaemonBidiTransport(stream, cancel, t.endpoint, openFrame)
	if err != nil {
		return nil, nil, err
	}
	openJSON, err := json.Marshal(map[string]any{
		"session_id":          transport.sessionID,
		"state":               "Open",
		"max_buffered_frames": MaxBidiBufferedFrames,
	})
	if err != nil {
		_ = transport.Close(ctx)
		return nil, nil, invalidRuntimePayload(fmt.Sprintf("encode direct bidi open JSON: %v", err), err)
	}
	return transport, openJSON, nil
}

func (t *DirectDaemonRuntimeTransport) Prepare(ctx context.Context, draftJSON []byte, optionsJSON []byte) ([]byte, error) {
	if handle, ok, err := t.optionalHandleTransport(ctx); err != nil {
		return nil, err
	} else if ok {
		return handle.Prepare(ctx, draftJSON, optionsJSON)
	}
	return directRuntimePrepare(ctx, t.identity, draftJSON, optionsJSON)
}

func (t *DirectDaemonRuntimeTransport) SubmitSigned(ctx context.Context, signedJSON []byte) ([]byte, error) {
	if handle, ok, err := t.optionalHandleTransport(ctx); err != nil {
		return nil, err
	} else if ok {
		return handle.SubmitSigned(ctx, signedJSON)
	}
	draftJSON, err := directSignedInvocationDraftJSON(signedJSON)
	if err != nil {
		return nil, err
	}
	resultJSON, err := t.Invoke(ctx, draftJSON)
	if err != nil {
		return nil, err
	}
	result, err := NewInvocationResultFromJSON(resultJSON)
	if err != nil {
		return nil, err
	}
	snapshot := t.storeDirectHandle(result.TerminalState(), true, resultJSON)
	return directRuntimeHandleSnapshotJSON(snapshot)
}

func (t *DirectDaemonRuntimeTransport) AwaitHandle(ctx context.Context, handleID uint64) ([]byte, error) {
	if handle, ok, err := t.optionalHandleTransport(ctx); err != nil {
		return nil, err
	} else if ok {
		return handle.AwaitHandle(ctx, handleID)
	}
	snapshot, err := t.directHandleSnapshot(ctx, handleID)
	if err != nil {
		return nil, err
	}
	return append([]byte(nil), snapshot.result...), nil
}

func (t *DirectDaemonRuntimeTransport) CancelHandle(ctx context.Context, handleID uint64, reason string) ([]byte, error) {
	if handle, ok, err := t.optionalHandleTransport(ctx); err != nil {
		return nil, err
	} else if ok {
		return handle.CancelHandle(ctx, handleID, reason)
	}
	snapshot, err := t.directHandleSnapshot(ctx, handleID)
	if err != nil {
		return nil, err
	}
	return json.Marshal(map[string]any{
		"handle_id": handleID,
		"cancelled": false,
		"state":     snapshot.state,
		"terminal":  snapshot.terminal,
	})
}

func (t *DirectDaemonRuntimeTransport) HandleEvents(ctx context.Context, handleID uint64) ([]byte, error) {
	if handle, ok, err := t.optionalHandleTransport(ctx); err != nil {
		return nil, err
	} else if ok {
		return handle.HandleEvents(ctx, handleID)
	}
	snapshot, err := t.directHandleSnapshot(ctx, handleID)
	if err != nil {
		return nil, err
	}
	return directRuntimeHandleSnapshotJSON(snapshot)
}

func (t *DirectDaemonRuntimeTransport) FreeHandle(ctx context.Context, handleID uint64) error {
	if handle, ok, err := t.optionalHandleTransport(ctx); err != nil {
		return err
	} else if ok {
		return handle.FreeHandle(ctx, handleID)
	}
	if ctx == nil {
		return invalidRuntimeClient("context is required")
	}
	t.mu.Lock()
	defer t.mu.Unlock()
	if t.closed {
		return invalidRuntimeClient("runtime transport is closed")
	}
	delete(t.handles, handleID)
	return nil
}

func directRuntimePrepare(ctx context.Context, identity *IdentityClient, draftJSON []byte, optionsJSON []byte) ([]byte, error) {
	draft, err := NewInvocationDraftFromJSON(draftJSON)
	if err != nil {
		return nil, err
	}
	draft, err = directPreparedInvocationDraft(ctx, identity, draft)
	if err != nil {
		return nil, err
	}
	var options PrepareOptions
	if len(optionsJSON) > 0 && string(optionsJSON) != "null" {
		if err := json.Unmarshal(optionsJSON, &options); err != nil {
			return nil, invalidRuntimePayload(fmt.Sprintf("decode prepare options: %v", err), err)
		}
	}
	material, err := signingMaterialForInvocationDraft(draft)
	if err != nil {
		return nil, err
	}
	expiresIn := time.Duration(options.ExpiresInMS) * time.Millisecond
	if expiresIn <= 0 {
		expiresIn = 5 * time.Minute
	}
	expiresAtUnixMS := time.Now().Add(expiresIn).UnixMilli()
	material.expiresAtUnixMS = expiresAtUnixMS
	if options.LocalDaemonSigning || strings.TrimSpace(options.SignerID) != "" || strings.TrimSpace(options.PolicyRef) != "" {
		mode := "caller_signing"
		if options.LocalDaemonSigning {
			mode = "local_daemon_signing"
		}
		material.signerPolicy = &SignerPolicy{
			mode:            mode,
			signerID:        strings.TrimSpace(options.SignerID),
			policyRef:       strings.TrimSpace(options.PolicyRef),
			expiresAtUnixMS: expiresAtUnixMS,
		}
	}
	canonical, err := decodeCanonicalBytesBase64(material.CanonicalBytesBase64())
	if err != nil {
		return nil, err
	}
	canonicalHash := sha256.Sum256(canonical)
	preparedID := fmt.Sprintf("direct-prepared-%d", time.Now().UnixNano())
	return json.Marshal(map[string]any{
		"prepared_id":        preparedID,
		"request_id":         "req-" + preparedID,
		"descriptor_ref":     draft.DescriptorRef(),
		"canonical_hash_hex": hex.EncodeToString(canonicalHash[:]),
		"expires_at_unix_ms": expiresAtUnixMS,
		"tuple":              draft,
		"signing_material":   directRuntimeSigningMaterialJSON(material),
		"submit_ready":       false,
	})
}

func directPreparedInvocationDraft(ctx context.Context, identity *IdentityClient, draft InvocationDraft) (InvocationDraft, error) {
	abilityName, err := directLocalAbilityName(ctx, identity, draft)
	if err != nil {
		return InvocationDraft{}, err
	}
	return directPreparedInvocationDraftWithAbility(ctx, identity, draft, abilityName)
}

func directPreparedInvocationDraftWithAbility(ctx context.Context, identity *IdentityClient, draft InvocationDraft, abilityName string) (InvocationDraft, error) {
	subjectURA, err := descriptorBoundSubjectURA(ctx, identity, draft.SubjectURA(), abilityName)
	if err != nil {
		return InvocationDraft{}, err
	}
	if subjectURA == draft.SubjectURA() {
		return draft, nil
	}
	return invocationDraftWithSubjectURA(draft, subjectURA)
}

func invocationDraftWithSubjectURA(draft InvocationDraft, subjectURA string) (InvocationDraft, error) {
	builder := NewInvocationBuilder().
		WithCallerURA(draft.CallerURA()).
		WithCalleeURA(draft.CalleeURA()).
		WithDescriptorRef(draft.DescriptorRef()).
		WithSubjectURA(subjectURA).
		WithNonceBase64(draft.NonceBase64()).
		WithCausalContext(draft.CausalContext()).
		WithContentType(draft.ContentType()).
		WithMetadata(draft.Metadata())
	if signature := draft.CallerSignature(); signature != nil {
		builder.WithCallerSignature(*signature)
	}
	if draft.HasJSONArgs() {
		builder.WithJSONArgs(draft.JSONArgs())
	} else {
		builder.WithArgumentsBase64(draft.ArgumentsBase64())
	}
	return builder.Build()
}

func directRuntimeSigningMaterialJSON(material SigningMaterial) map[string]any {
	value := map[string]any{
		"algorithm":              material.Algorithm(),
		"canonical_bytes_base64": material.CanonicalBytesBase64(),
		"args_digest_hex":        material.ArgsDigestHex(),
		"descriptor_ref":         material.DescriptorRef(),
		"nonce_base64":           material.NonceBase64(),
		"signed_fields":          material.SignedFields(),
		"expires_at_unix_ms":     material.ExpiresAtUnixMS(),
	}
	if policy := material.SignerPolicy(); policy != nil {
		value["signer_policy"] = map[string]any{
			"mode":               policy.Mode(),
			"signer_id":          policy.SignerID(),
			"policy_ref":         policy.PolicyRef(),
			"expires_at_unix_ms": policy.ExpiresAtUnixMS(),
		}
	}
	return value
}

func directSignedInvocationDraftJSON(signedJSON []byte) ([]byte, error) {
	var signed struct {
		Prepared struct {
			Tuple json.RawMessage `json:"tuple"`
		} `json:"prepared"`
		Signature InvocationSignature `json:"signature"`
	}
	if err := json.Unmarshal(signedJSON, &signed); err != nil {
		return nil, invalidRuntimePayload(fmt.Sprintf("decode signed invocation: %v", err), err)
	}
	if len(signed.Prepared.Tuple) == 0 || string(signed.Prepared.Tuple) == "null" {
		return nil, invalidRuntimePayload("signed invocation prepared.tuple is required", nil)
	}
	if strings.TrimSpace(signed.Signature.Algorithm) == "" || strings.TrimSpace(signed.Signature.SignatureBase64) == "" {
		return nil, invalidRuntimePayload("signed invocation signature is required", nil)
	}
	draft, err := NewInvocationDraftFromJSON(signed.Prepared.Tuple)
	if err != nil {
		return nil, err
	}
	builder := NewInvocationBuilder().
		WithCallerURA(draft.CallerURA()).
		WithCalleeURA(draft.CalleeURA()).
		WithDescriptorRef(draft.DescriptorRef()).
		WithSubjectURA(draft.SubjectURA()).
		WithNonceBase64(draft.NonceBase64()).
		WithCausalContext(draft.CausalContext()).
		WithContentType(draft.ContentType()).
		WithMetadata(draft.Metadata()).
		WithCallerSignature(signed.Signature)
	if draft.HasJSONArgs() {
		builder.WithJSONArgs(draft.JSONArgs())
	} else {
		builder.WithArgumentsBase64(draft.ArgumentsBase64())
	}
	signedDraft, err := builder.Build()
	if err != nil {
		return nil, err
	}
	return json.Marshal(signedDraft)
}

func (t *DirectDaemonRuntimeTransport) storeDirectHandle(state string, terminal bool, result json.RawMessage) directRuntimeHandleSnapshot {
	t.mu.Lock()
	defer t.mu.Unlock()
	t.nextHandleID++
	handleID := t.nextHandleID
	if handleID == 0 {
		t.nextHandleID++
		handleID = t.nextHandleID
	}
	eventKind := "terminal"
	if !terminal {
		eventKind = "submitted"
	}
	snapshot := directRuntimeHandleSnapshot{
		handleID: handleID,
		state:    state,
		terminal: terminal,
		events: []map[string]any{{
			"sequence": uint64(1),
			"kind":     eventKind,
			"state":    state,
			"terminal": terminal,
			"result":   json.RawMessage(result),
		}},
		result: append(json.RawMessage(nil), result...),
	}
	t.handles[handleID] = snapshot
	return snapshot
}

func (t *DirectDaemonRuntimeTransport) directHandleSnapshot(ctx context.Context, handleID uint64) (directRuntimeHandleSnapshot, error) {
	if ctx == nil {
		return directRuntimeHandleSnapshot{}, invalidRuntimeClient("context is required")
	}
	if handleID == 0 {
		return directRuntimeHandleSnapshot{}, invalidRuntimePayload("handle_id is required", nil)
	}
	t.mu.Lock()
	defer t.mu.Unlock()
	if t.closed {
		return directRuntimeHandleSnapshot{}, invalidRuntimeClient("runtime transport is closed")
	}
	snapshot, ok := t.handles[handleID]
	if !ok {
		return directRuntimeHandleSnapshot{}, &SDKError{
			Code:      ErrNotFound,
			Stage:     "runtime",
			Retry:     RetryNever,
			Retryable: RetryableForHint(RetryNever),
			Message:   fmt.Sprintf("direct runtime handle %d not found", handleID),
		}
	}
	return snapshot, nil
}

func directRuntimeHandleSnapshotJSON(snapshot directRuntimeHandleSnapshot) ([]byte, error) {
	result := json.RawMessage("null")
	if len(snapshot.result) > 0 {
		result = snapshot.result
	}
	return json.Marshal(map[string]any{
		"handle_id": snapshot.handleID,
		"state":     snapshot.state,
		"terminal":  snapshot.terminal,
		"events":    snapshot.events,
		"result":    result,
	})
}

func (t *DirectDaemonRuntimeTransport) Close(ctx context.Context) error {
	if t == nil {
		return invalidRuntimeClient("runtime transport is not initialized")
	}
	if ctx == nil {
		return invalidRuntimeClient("context is required")
	}
	t.mu.Lock()
	if t.closed {
		t.mu.Unlock()
		return nil
	}
	conn := t.conn
	handle := t.handle
	closeHandle := t.closeHandleTransport
	t.conn = nil
	t.client = nil
	t.handle = nil
	t.closeHandleTransport = false
	t.closed = true
	t.mu.Unlock()
	var closeErr error
	if conn == nil {
		if closeHandle && handle != nil {
			return handle.Close(ctx)
		}
		return nil
	}
	if err := conn.Close(); err != nil {
		closeErr = transportRuntimeError("close direct runtime transport failed", err)
	}
	if closeHandle && handle != nil {
		closeErr = errors.Join(closeErr, handle.Close(ctx))
	}
	return closeErr
}

func (t *DirectDaemonRuntimeTransport) requireOpen(ctx context.Context) (axonpb.InvocationClient, time.Duration, error) {
	if t == nil {
		return nil, 0, invalidRuntimeClient("runtime transport is not initialized")
	}
	if ctx == nil {
		return nil, 0, invalidRuntimeClient("context is required")
	}
	t.mu.Lock()
	defer t.mu.Unlock()
	if t.closed || t.client == nil {
		return nil, 0, invalidRuntimeClient("runtime transport is closed")
	}
	return t.client, t.invokeTimeout, nil
}

func (t *DirectDaemonRuntimeTransport) optionalHandleTransport(ctx context.Context) (RuntimeTransport, bool, error) {
	if _, _, err := t.requireOpen(ctx); err != nil {
		return nil, false, err
	}
	t.mu.Lock()
	defer t.mu.Unlock()
	if t.handle == nil {
		return nil, false, nil
	}
	return t.handle, true, nil
}

type directDaemonStreamTransport struct {
	stream   grpc.ServerStreamingClient[axonpb.InvokeStreamChunk]
	cancel   context.CancelFunc
	endpoint string
	streamID string

	mu     sync.Mutex
	closed bool
}

func newDirectDaemonStreamTransport(stream grpc.ServerStreamingClient[axonpb.InvokeStreamChunk], cancel context.CancelFunc, endpoint string) *directDaemonStreamTransport {
	return &directDaemonStreamTransport{
		stream:   stream,
		cancel:   cancel,
		endpoint: endpoint,
		streamID: fmt.Sprintf("direct-stream-%d", time.Now().UnixNano()),
	}
}

func (t *directDaemonStreamTransport) Recv(ctx context.Context) ([]byte, error) {
	if err := t.requireOpen(ctx); err != nil {
		return nil, err
	}
	chunk, err := t.stream.Recv()
	if err != nil {
		if err == io.EOF {
			return nil, directRuntimeError(
				"daemon stream ended without a terminal frame",
				ErrProtocol,
				RetryNever,
				map[string]any{"endpoint": t.endpoint, "stream_id": t.streamID},
				err,
			)
		}
		return nil, directRuntimeGRPCError(err, t.endpoint)
	}
	return directStreamChunkJSON(chunk)
}

func (t *directDaemonStreamTransport) Cancel(ctx context.Context, reason string) ([]byte, error) {
	if ctx == nil {
		return nil, invalidRuntimeClient("context is required")
	}
	t.close()
	return json.Marshal(map[string]any{
		"stream_id": t.streamID,
		"cancelled": true,
		"state":     "Cancelled",
		"terminal":  true,
		"reason":    reason,
	})
}

func (t *directDaemonStreamTransport) Close(ctx context.Context) error {
	if ctx == nil {
		return invalidRuntimeClient("context is required")
	}
	t.close()
	return nil
}

func (t *directDaemonStreamTransport) close() {
	t.mu.Lock()
	defer t.mu.Unlock()
	if t.closed {
		return
	}
	t.closed = true
	t.cancel()
}

func (t *directDaemonStreamTransport) requireOpen(ctx context.Context) error {
	if ctx == nil {
		return invalidRuntimeClient("context is required")
	}
	t.mu.Lock()
	defer t.mu.Unlock()
	if t.closed {
		return invalidRuntimeClient("stream transport is closed")
	}
	return nil
}

type directDaemonBidiTransport struct {
	stream    grpc.BidiStreamingClient[axonpb.InvokeBidiUp, axonpb.InvokeBidiDown]
	cancel    context.CancelFunc
	endpoint  string
	sessionID string

	mu             sync.Mutex
	closed         bool
	sendClosed     bool
	lastUpSequence uint64
}

func newDirectDaemonBidiTransport(
	stream grpc.BidiStreamingClient[axonpb.InvokeBidiUp, axonpb.InvokeBidiDown],
	cancel context.CancelFunc,
	endpoint string,
	openFrame *axonpb.InvokeBidiUp,
) (*directDaemonBidiTransport, error) {
	transport := &directDaemonBidiTransport{
		stream:    stream,
		cancel:    cancel,
		endpoint:  endpoint,
		sessionID: fmt.Sprintf("direct-bidi-%d", time.Now().UnixNano()),
	}
	if err := stream.Send(openFrame); err != nil {
		cancel()
		return nil, directRuntimeGRPCError(err, endpoint)
	}
	return transport, nil
}

func (t *directDaemonBidiTransport) Send(ctx context.Context, frameJSON []byte) ([]byte, error) {
	frame, err := NewBidiFrameFromJSON(frameJSON)
	if err != nil {
		return nil, err
	}
	up, err := directBidiFrameToUp(frame)
	if err != nil {
		return nil, err
	}
	t.mu.Lock()
	if t.closed {
		t.mu.Unlock()
		return nil, invalidRuntimeClient("bidi transport is closed")
	}
	if t.sendClosed {
		t.mu.Unlock()
		return nil, &SDKError{
			Code:      ErrCancelled,
			Stage:     "direct_runtime.bidi",
			Retry:     RetryNever,
			Retryable: false,
			Message:   "bidi send path is closed",
		}
	}
	if frame.Sequence() != t.lastUpSequence+1 {
		expected := t.lastUpSequence + 1
		t.mu.Unlock()
		return nil, invalidRuntimePayload(fmt.Sprintf("bidi up frames must be contiguous: expected %d got %d", expected, frame.Sequence()), nil)
	}
	t.lastUpSequence = frame.Sequence()
	t.mu.Unlock()
	if err := t.stream.Send(up); err != nil {
		return nil, directRuntimeGRPCError(err, t.endpoint)
	}
	return frameJSON, nil
}

func (t *directDaemonBidiTransport) Recv(ctx context.Context) ([]byte, error) {
	if err := t.requireOpen(ctx); err != nil {
		return nil, err
	}
	for {
		down, err := t.stream.Recv()
		if err != nil {
			if err == io.EOF {
				return nil, directRuntimeError(
					"daemon bidi ended without a terminal frame",
					ErrProtocol,
					RetryNever,
					map[string]any{"endpoint": t.endpoint, "session_id": t.sessionID},
					err,
				)
			}
			return nil, directRuntimeGRPCError(err, t.endpoint)
		}
		if directBidiDownIsInternalAdmission(down) {
			continue
		}
		return directBidiDownJSON(down)
	}
}

func (t *directDaemonBidiTransport) CloseSend(ctx context.Context) ([]byte, error) {
	if err := t.requireOpen(ctx); err != nil {
		return nil, err
	}
	t.mu.Lock()
	if t.sendClosed {
		t.mu.Unlock()
		return nil, &SDKError{
			Code:      ErrCancelled,
			Stage:     "direct_runtime.bidi",
			Retry:     RetryNever,
			Retryable: false,
			Message:   "bidi send path is closed",
		}
	}
	t.lastUpSequence++
	sequence := t.lastUpSequence
	t.sendClosed = true
	t.mu.Unlock()
	if err := t.stream.Send(&axonpb.InvokeBidiUp{
		Sequence: sequence,
		Payload:  &axonpb.InvokeBidiUp_Control{Control: &axonpb.BidiControl{Control: &axonpb.BidiControl_Eof{Eof: true}}},
	}); err != nil {
		return nil, directRuntimeGRPCError(err, t.endpoint)
	}
	if err := t.stream.CloseSend(); err != nil {
		return nil, directRuntimeGRPCError(err, t.endpoint)
	}
	return json.Marshal(map[string]any{
		"session_id": t.sessionID,
		"state":      "HalfClosedLocal",
		"terminal":   false,
	})
}

func (t *directDaemonBidiTransport) Cancel(ctx context.Context, reason string) ([]byte, error) {
	if ctx == nil {
		return nil, invalidRuntimeClient("context is required")
	}
	t.close()
	return json.Marshal(map[string]any{
		"session_id": t.sessionID,
		"state":      "Cancelled",
		"terminal":   true,
		"reason":     reason,
	})
}

func (t *directDaemonBidiTransport) Close(ctx context.Context) error {
	if ctx == nil {
		return invalidRuntimeClient("context is required")
	}
	t.close()
	return nil
}

func (t *directDaemonBidiTransport) close() {
	t.mu.Lock()
	defer t.mu.Unlock()
	if t.closed {
		return
	}
	t.closed = true
	t.sendClosed = true
	t.cancel()
}

func (t *directDaemonBidiTransport) requireOpen(ctx context.Context) error {
	if ctx == nil {
		return invalidRuntimeClient("context is required")
	}
	t.mu.Lock()
	defer t.mu.Unlock()
	if t.closed {
		return invalidRuntimeClient("bidi transport is closed")
	}
	return nil
}

func decodeDirectRuntimeEndpoint(raw []byte) (ConnectOptions, RuntimeEndpoint, error) {
	var options ConnectOptions
	_ = json.Unmarshal(raw, &options)
	endpoint, err := NewRuntimeEndpointFromJSON(raw)
	return options, endpoint, err
}

func directRuntimeEndpointJSON(endpoint RuntimeEndpoint, options ConnectOptions) ([]byte, error) {
	value := map[string]any{
		"endpoint":         endpoint.Endpoint,
		"control_path":     endpoint.ControlPath,
		"protocol_version": endpoint.ProtocolVersion,
		"abi_version":      endpoint.ABIVersion,
	}
	if options.DialTimeoutMS > 0 {
		value["dial_timeout_ms"] = options.DialTimeoutMS
	}
	if options.InvokeTimeoutMS > 0 {
		value["invoke_timeout_ms"] = options.InvokeTimeoutMS
	}
	if options.MaxMessageBytes > 0 {
		value["max_message_bytes"] = options.MaxMessageBytes
	}
	raw, err := json.Marshal(value)
	if err != nil {
		return nil, invalidRuntimePayload(fmt.Sprintf("encode runtime endpoint JSON: %v", err), err)
	}
	return raw, nil
}

func directInvokeRequestFromDraftJSON(ctx context.Context, identity *IdentityClient, raw []byte) (InvocationDraft, *axonpb.InvokeRequest, error) {
	draft, err := NewInvocationDraftFromJSON(raw)
	if err != nil {
		return InvocationDraft{}, nil, err
	}
	fields, err := directInvokeFields(ctx, identity, draft)
	if err != nil {
		return InvocationDraft{}, nil, err
	}
	return fields.draft, &axonpb.InvokeRequest{
		Envelope:        fields.envelope,
		Target:          fields.target,
		FunctionName:    fields.abilityName,
		Arguments:       fields.arguments,
		ContentType:     draft.ContentType(),
		Metadata:        fields.metadata,
		ContentEnvelope: fields.contentEnvelope,
	}, nil
}

func directStreamRequestFromDraftJSON(ctx context.Context, identity *IdentityClient, raw []byte) (InvocationDraft, *axonpb.InvokeServerStreamRequest, error) {
	draft, err := NewInvocationDraftFromJSON(raw)
	if err != nil {
		return InvocationDraft{}, nil, err
	}
	fields, err := directInvokeFields(ctx, identity, draft)
	if err != nil {
		return InvocationDraft{}, nil, err
	}
	return draft, &axonpb.InvokeServerStreamRequest{
		Envelope:        fields.envelope,
		Target:          fields.target,
		FunctionName:    fields.abilityName,
		Arguments:       fields.arguments,
		ContentType:     draft.ContentType(),
		Metadata:        fields.metadata,
		ContentEnvelope: fields.contentEnvelope,
	}, nil
}

type directInvokeFieldSet struct {
	draft           InvocationDraft
	envelope        *axonpb.Envelope
	target          *axonpb.InvocationTarget
	abilityName     string
	arguments       []byte
	metadata        map[string]string
	contentEnvelope *axonpb.ContentEnvelope
}

func directInvokeFields(ctx context.Context, identity *IdentityClient, draft InvocationDraft) (directInvokeFieldSet, error) {
	projectedDraft, abilityName, err := directExecutableInvocationDraft(ctx, identity, draft)
	if err != nil {
		return directInvokeFieldSet{}, err
	}
	draft = projectedDraft
	nonce, err := base64.StdEncoding.DecodeString(draft.NonceBase64())
	if err != nil {
		return directInvokeFieldSet{}, invalidRuntimePayload(fmt.Sprintf("decode nonce_base64: %v", err), err)
	}
	causal, err := directCausalContext(draft.CausalContext())
	if err != nil {
		return directInvokeFieldSet{}, err
	}
	args, err := invocationDraftArgumentBytes(draft)
	if err != nil {
		return directInvokeFieldSet{}, err
	}
	metadata, err := directMetadata(draft)
	if err != nil {
		return directInvokeFieldSet{}, err
	}
	callerSignature, err := directCallerSignature(draft)
	if err != nil {
		return directInvokeFieldSet{}, err
	}
	return directInvokeFieldSet{
		draft: draft,
		envelope: &axonpb.Envelope{
			RequestId:       fmt.Sprintf("req-%d", time.Now().UnixNano()),
			Caller:          directAgentIdentity(draft.CallerURA()),
			Callee:          directAgentIdentity(draft.CalleeURA()),
			Subject:         &axonpb.SubjectIdentity{Ura: draft.SubjectURA(), Profile: defaultURAProfile},
			InvocationNonce: nonce,
			CausalContext:   causal,
			CallerSignature: callerSignature,
		},
		target:      directInvocationTarget(abilityName),
		abilityName: abilityName,
		arguments:   args,
		metadata:    metadata,
		contentEnvelope: &axonpb.ContentEnvelope{
			ContentType: draft.ContentType(),
			Encoding:    "identity",
		},
	}, nil
}

func directExecutableInvocationDraft(ctx context.Context, identity *IdentityClient, draft InvocationDraft) (InvocationDraft, string, error) {
	abilityName, err := directLocalAbilityName(ctx, identity, draft)
	if err != nil {
		return InvocationDraft{}, "", err
	}
	projected, err := directPreparedInvocationDraftWithAbility(ctx, identity, draft, abilityName)
	if err != nil {
		return InvocationDraft{}, "", err
	}
	return projected, abilityName, nil
}

func directBidiOpenFrame(ctx context.Context, identity *IdentityClient, draft InvocationDraft, streams []*axonpb.StreamDescriptor) (*axonpb.InvokeBidiUp, error) {
	fields, err := directInvokeFields(ctx, identity, draft)
	if err != nil {
		return nil, err
	}
	mac := []byte(nil)
	if signature := draft.CallerSignature(); signature != nil {
		decoded, err := base64.StdEncoding.DecodeString(signature.SignatureBase64)
		if err != nil {
			return nil, invalidRuntimePayload(fmt.Sprintf("decode caller_signature.signature_base64: %v", err), err)
		}
		mac = decoded
	}
	return &axonpb.InvokeBidiUp{
		Sequence: 0,
		Mac:      mac,
		Payload: &axonpb.InvokeBidiUp_EnvelopeOpen{EnvelopeOpen: &axonpb.EnvelopeOpen{
			Envelope:        fields.envelope,
			Target:          fields.target,
			InitialArgs:     fields.arguments,
			ArgsContentType: draft.ContentType(),
			Streams:         streams,
			Metadata:        fields.metadata,
			ContentEnvelope: fields.contentEnvelope,
		}},
	}, nil
}

func directInvocationTarget(abilityName string) *axonpb.InvocationTarget {
	return &axonpb.InvocationTarget{
		AbilityName: abilityName,
		TypedTarget: &axonpb.InvocationTarget_Ability{
			Ability: &axonpb.AbilityTarget{
				AbilityName:  abilityName,
				FunctionName: abilityName,
			},
		},
	}
}

func directLocalAbilityName(ctx context.Context, identity *IdentityClient, draft InvocationDraft) (string, error) {
	ref, err := ProjectAbilityDescriptorRef(ctx, identity, draft.DescriptorRef())
	if err != nil {
		return "", invalidRuntimePayload(fmt.Sprintf("project descriptor_ref: %v", err), err)
	}
	abilityName, ok := PublicAbilityNameFromAbilityURA(draft.CalleeURA(), ref.AbilityURA)
	if !ok || strings.TrimSpace(abilityName) == "" {
		return "", invalidRuntimePayload(
			fmt.Sprintf("descriptor_ref %q is not owned by callee %q", draft.DescriptorRef(), draft.CalleeURA()),
			nil,
		)
	}
	return abilityName, nil
}

func directAgentIdentity(ura string) *axonpb.AgentIdentity {
	return &axonpb.AgentIdentity{Ura: ura, Profile: defaultURAProfile}
}

func directCallerSignature(draft InvocationDraft) (*axonpb.CallerSignature, error) {
	signature := draft.CallerSignature()
	if signature == nil {
		return nil, nil
	}
	decoded, err := base64.StdEncoding.DecodeString(signature.SignatureBase64)
	if err != nil {
		return nil, invalidRuntimePayload(fmt.Sprintf("decode caller_signature.signature_base64: %v", err), err)
	}
	keyIDHint := strings.TrimSpace(signature.KeyIDHint)
	if keyIDHint == "" {
		keyIDHint = strings.TrimSpace(signature.SignerPublicKeyBase64)
	}
	return &axonpb.CallerSignature{
		Algorithm: signature.Algorithm,
		Signature: decoded,
		KeyIdHint: keyIDHint,
	}, nil
}

func directMetadata(draft InvocationDraft) (map[string]string, error) {
	result := map[string]string{}
	for key, value := range draft.Metadata() {
		stringValue, ok, err := directMetadataValueString(key, value)
		if err != nil {
			return nil, err
		}
		if !ok {
			continue
		}
		result[key] = stringValue
	}
	result[directSignedDescriptorRefMetadata] = draft.DescriptorRef()
	return result, nil
}

func directMetadataValueString(key string, value any) (string, bool, error) {
	switch v := value.(type) {
	case nil:
		return "", false, nil
	case string:
		return v, true, nil
	case bool:
		return strconv.FormatBool(v), true, nil
	case int:
		return strconv.FormatInt(int64(v), 10), true, nil
	case int8:
		return strconv.FormatInt(int64(v), 10), true, nil
	case int16:
		return strconv.FormatInt(int64(v), 10), true, nil
	case int32:
		return strconv.FormatInt(int64(v), 10), true, nil
	case int64:
		return strconv.FormatInt(v, 10), true, nil
	case uint:
		return strconv.FormatUint(uint64(v), 10), true, nil
	case uint8:
		return strconv.FormatUint(uint64(v), 10), true, nil
	case uint16:
		return strconv.FormatUint(uint64(v), 10), true, nil
	case uint32:
		return strconv.FormatUint(uint64(v), 10), true, nil
	case uint64:
		return strconv.FormatUint(v, 10), true, nil
	case float32:
		return strconv.FormatFloat(float64(v), 'f', -1, 32), true, nil
	case float64:
		return strconv.FormatFloat(v, 'f', -1, 64), true, nil
	default:
		raw, err := json.Marshal(v)
		if err != nil {
			return "", false, invalidRuntimePayload(fmt.Sprintf("metadata[%q] must be JSON-encodable for Axon InvokeRequest: %v", key, err), err)
		}
		return string(raw), true, nil
	}
}

func directCausalContext(value map[string]any) (*axonpb.CausalContext, error) {
	causal, err := causalContextForInvocationDraft(value)
	if err != nil {
		return nil, err
	}
	switch causal.Kind {
	case CausalContextNull:
		return &axonpb.CausalContext{Form: &axonpb.CausalContext_None{None: &axonpb.Empty{}}}, nil
	case CausalContextScalar:
		ref, err := directReceiptRef(causal.Scalar)
		if err != nil {
			return nil, err
		}
		return &axonpb.CausalContext{Form: &axonpb.CausalContext_Scalar{Scalar: ref}}, nil
	case CausalContextVector:
		prior := make([]*axonpb.ReceiptRef, 0, len(causal.Vector))
		for _, item := range causal.Vector {
			ref, err := directReceiptRef(item)
			if err != nil {
				return nil, err
			}
			prior = append(prior, ref)
		}
		return &axonpb.CausalContext{Form: &axonpb.CausalContext_List{List: &axonpb.ReceiptList{Prior: prior}}}, nil
	case CausalContextDAG:
		root, err := hex.DecodeString(causal.DAGRootHex)
		if err != nil {
			return nil, invalidRuntimePayload(fmt.Sprintf("decode root_hex: %v", err), err)
		}
		return &axonpb.CausalContext{Form: &axonpb.CausalContext_Merkle{Merkle: &axonpb.MerkleRoot{Root: root, ProofUra: causal.DAGProofURA}}}, nil
	default:
		return nil, invalidRuntimePayload("unknown causal_context kind", nil)
	}
}

func directReceiptRef(value any) (*axonpb.ReceiptRef, error) {
	ref, ok := value.(CausalReceiptRef)
	if !ok {
		return nil, invalidRuntimePayload("causal receipt ref must be a CausalReceiptRef", nil)
	}
	if ref.URA == "" || ref.HashHex == "" {
		return nil, invalidRuntimePayload("receipt_ura and receipt_hash_hex are required", nil)
	}
	receiptHash, err := hex.DecodeString(ref.HashHex)
	if err != nil {
		return nil, invalidRuntimePayload(fmt.Sprintf("decode receipt_hash_hex: %v", err), err)
	}
	return &axonpb.ReceiptRef{ReceiptUra: ref.URA, ReceiptHash: receiptHash}, nil
}

func directBidiStreamDescriptors(raw []byte) ([]*axonpb.StreamDescriptor, error) {
	var decoded []struct {
		StreamID    uint64 `json:"stream_id"`
		ContentType string `json:"content_type"`
		CodecParams string `json:"codec_params"`
		Ordering    string `json:"ordering"`
	}
	if err := json.Unmarshal(raw, &decoded); err != nil {
		return nil, invalidRuntimePayload(fmt.Sprintf("decode bidi streams JSON: %v", err), err)
	}
	if len(decoded) == 0 {
		return nil, invalidRuntimePayload("bidi streams must not be empty", nil)
	}
	seen := map[uint64]struct{}{}
	streams := make([]*axonpb.StreamDescriptor, 0, len(decoded))
	for _, item := range decoded {
		if item.StreamID == 0 {
			return nil, invalidRuntimePayload("stream_id is required", nil)
		}
		if _, ok := seen[item.StreamID]; ok {
			return nil, invalidRuntimePayload("bidi stream ids must be unique", nil)
		}
		seen[item.StreamID] = struct{}{}
		streams = append(streams, &axonpb.StreamDescriptor{
			StreamId:    uint32(item.StreamID),
			ContentType: item.ContentType,
			CodecParams: item.CodecParams,
			Ordering:    item.Ordering,
		})
	}
	return streams, nil
}

func directInvokeResponseJSON(draft InvocationDraft, response *axonpb.InvokeResponse) ([]byte, error) {
	stateName := directStateName(response.GetState())
	errorValue := directResponseFailure(response.GetError(), stateName, "direct_runtime.invoke")
	value := map[string]any{
		"ok":                  errorValue == nil,
		"tuple":               draft,
		"terminal_state":      stateName,
		"output_content_type": response.GetResultContentType(),
		"output_base64":       base64.StdEncoding.EncodeToString(response.GetResult()),
		"output_json":         directOutputJSON(response.GetResult(), response.GetResultContentType()),
		"selected_node_id":    response.GetSelectedNodeId(),
		"scheduling_reason":   response.GetSchedulingReason(),
		"elapsed_ms":          response.GetElapsedMs(),
		"receipt":             directReceipt(response.GetTerminalReceipt()),
		"error":               errorValue,
	}
	return json.Marshal(value)
}

func directStreamChunkJSON(chunk *axonpb.InvokeStreamChunk) ([]byte, error) {
	terminal := chunk.GetTerminal() || directStateTerminal(chunk.GetState())
	errorValue := directResponseFailure(chunk.GetError(), directStateName(chunk.GetState()), "direct_runtime.stream")
	value := map[string]any{
		"sequence":             chunk.GetSequence() + 1,
		"kind":                 directStreamEventKind(terminal),
		"state":                directStateName(chunk.GetState()),
		"terminal":             terminal,
		"payload_content_type": chunk.GetContentType(),
		"payload_base64":       base64.StdEncoding.EncodeToString(chunk.GetPayload()),
		"payload_json":         directOutputJSON(chunk.GetPayload(), chunk.GetContentType()),
		"error":                errorValue,
	}
	if chunk.GetInvocationId() != "" {
		value["invocation_id"] = chunk.GetInvocationId()
	}
	if chunk.GetSelectedNodeId() != "" {
		value["selected_node_id"] = chunk.GetSelectedNodeId()
	}
	if chunk.GetSchedulingReason() != "" {
		value["scheduling_reason"] = chunk.GetSchedulingReason()
	}
	if chunk.GetElapsedMs() != 0 {
		value["elapsed_ms"] = chunk.GetElapsedMs()
	}
	if chunk.GetTerminalReceipt() != nil {
		value["receipt"] = directReceipt(chunk.GetTerminalReceipt())
	}
	return json.Marshal(value)
}

func directBidiFrameToUp(frame BidiFrame) (*axonpb.InvokeBidiUp, error) {
	switch frame.Kind() {
	case "data", "binary_chunk", "chunk":
		payload, err := directBidiPayloadBytes(frame)
		if err != nil {
			return nil, err
		}
		return &axonpb.InvokeBidiUp{
			Sequence: frame.Sequence(),
			Payload: &axonpb.InvokeBidiUp_BinaryChunk{BinaryChunk: &axonpb.BinaryChunk{
				StreamId: uint32(frame.StreamID()),
				Data:     payload,
			}},
		}, nil
	case "eof", "close_send":
		return &axonpb.InvokeBidiUp{
			Sequence: frame.Sequence(),
			Payload:  &axonpb.InvokeBidiUp_Control{Control: &axonpb.BidiControl{Control: &axonpb.BidiControl_Eof{Eof: true}}},
		}, nil
	case "control":
		control, err := directBidiControl(frame.PayloadJSON())
		if err != nil {
			return nil, err
		}
		return &axonpb.InvokeBidiUp{Sequence: frame.Sequence(), Payload: &axonpb.InvokeBidiUp_Control{Control: control}}, nil
	default:
		return nil, invalidRuntimePayload(fmt.Sprintf("unsupported bidi frame kind: %s", frame.Kind()), nil)
	}
}

func directBidiPayloadBytes(frame BidiFrame) ([]byte, error) {
	if frame.PayloadBase64() != "" {
		decoded, err := base64.StdEncoding.DecodeString(frame.PayloadBase64())
		if err != nil {
			return nil, invalidRuntimePayload(fmt.Sprintf("decode payload_base64: %v", err), err)
		}
		return decoded, nil
	}
	if len(frame.PayloadJSON()) != 0 {
		return json.Marshal(json.RawMessage(frame.PayloadJSON()))
	}
	return []byte{}, nil
}

func directBidiControl(raw json.RawMessage) (*axonpb.BidiControl, error) {
	var payload map[string]any
	if err := json.Unmarshal(raw, &payload); err != nil {
		return nil, invalidRuntimePayload(fmt.Sprintf("decode bidi control payload_json: %v", err), err)
	}
	if eof, _ := payload["eof"].(bool); eof {
		return &axonpb.BidiControl{Control: &axonpb.BidiControl_Eof{Eof: true}}, nil
	}
	if resize, ok := payload["pty_resize"].(map[string]any); ok {
		return &axonpb.BidiControl{Control: &axonpb.BidiControl_PtyResize{PtyResize: &axonpb.PtyResize{
			Cols: uint32(numericJSONValue(resize["cols"])),
			Rows: uint32(numericJSONValue(resize["rows"])),
		}}}, nil
	}
	if signal, ok := payload["pty_signal"].(map[string]any); ok {
		return &axonpb.BidiControl{Control: &axonpb.BidiControl_PtySignal{PtySignal: &axonpb.PtySignal{
			Signal: int32(numericJSONValue(signal["signal"])),
		}}}, nil
	}
	if media, ok := payload["media_pts"].(map[string]any); ok {
		return &axonpb.BidiControl{Control: &axonpb.BidiControl_MediaPts{MediaPts: &axonpb.MediaTimestamp{
			StreamId: uint32(numericJSONValue(media["stream_id"])),
			Pts:      uint64(numericJSONValue(media["pts"])),
		}}}, nil
	}
	return nil, invalidRuntimePayload("unsupported bidi control payload", nil)
}

func directBidiDownJSON(frame *axonpb.InvokeBidiDown) ([]byte, error) {
	terminal := directBidiDownTerminal(frame)
	value := map[string]any{
		"sequence":  frame.GetSequence() + 1,
		"kind":      directBidiDownKind(frame),
		"stream_id": uint64(0),
		"terminal":  terminal,
	}
	switch payload := frame.GetPayload().(type) {
	case *axonpb.InvokeBidiDown_BinaryChunk:
		value["stream_id"] = uint64(payload.BinaryChunk.GetStreamId())
		value["payload_base64"] = base64.StdEncoding.EncodeToString(payload.BinaryChunk.GetData())
	case *axonpb.InvokeBidiDown_Control:
		value["payload_json"] = directBidiControlJSON(payload.Control)
	case *axonpb.InvokeBidiDown_Receipt:
		receipt := directReceipt(payload.Receipt)
		value["payload_json"] = map[string]any{"receipt": receipt}
		if failure := payload.Receipt.GetFailure(); failure != nil {
			value["error"] = directAxonFailure(failure, "direct_runtime.bidi")
		}
	case *axonpb.InvokeBidiDown_DispatchCall, *axonpb.InvokeBidiDown_ReverseDispatchResult:
		value["error"] = map[string]any{
			"code":      string(ErrProtocolMismatch),
			"stage":     "direct_runtime.bidi",
			"message":   "carrier-v1 dispatch frame before SDK dual-read support",
			"retryable": false,
		}
	default:
		return nil, invalidRuntimePayload("daemon bidi frame did not include a payload", nil)
	}
	return json.Marshal(value)
}

func directBidiDownIsInternalAdmission(frame *axonpb.InvokeBidiDown) bool {
	receipt, ok := frame.GetPayload().(*axonpb.InvokeBidiDown_Receipt)
	return ok && frame.GetSequence() == 0 && !directBidiReceiptTerminal(receipt.Receipt)
}

func directBidiDownTerminal(frame *axonpb.InvokeBidiDown) bool {
	receipt, ok := frame.GetPayload().(*axonpb.InvokeBidiDown_Receipt)
	return ok && directBidiReceiptTerminal(receipt.Receipt)
}

func directBidiReceiptTerminal(receipt *axonpb.InvocationReceipt) bool {
	return receipt.GetCleanupComplete() || directStateTerminal(receipt.GetState())
}

func directBidiDownKind(frame *axonpb.InvokeBidiDown) string {
	switch payload := frame.GetPayload().(type) {
	case *axonpb.InvokeBidiDown_BinaryChunk:
		return "data"
	case *axonpb.InvokeBidiDown_Control:
		if payload.Control.GetEof() {
			return "remote_close_send"
		}
		return "control"
	case *axonpb.InvokeBidiDown_Receipt:
		if directBidiReceiptTerminal(payload.Receipt) {
			return "terminal"
		}
		return "receipt"
	case *axonpb.InvokeBidiDown_DispatchCall, *axonpb.InvokeBidiDown_ReverseDispatchResult:
		return "unsupported_frame"
	default:
		return "unknown"
	}
}

func directBidiControlJSON(control *axonpb.BidiControl) map[string]any {
	switch payload := control.GetControl().(type) {
	case *axonpb.BidiControl_Eof:
		return map[string]any{"eof": payload.Eof}
	case *axonpb.BidiControl_PtyResize:
		return map[string]any{"pty_resize": map[string]any{"cols": payload.PtyResize.GetCols(), "rows": payload.PtyResize.GetRows()}}
	case *axonpb.BidiControl_PtySignal:
		return map[string]any{"pty_signal": map[string]any{"signal": payload.PtySignal.GetSignal()}}
	case *axonpb.BidiControl_MediaPts:
		return map[string]any{"media_pts": map[string]any{"stream_id": payload.MediaPts.GetStreamId(), "pts": payload.MediaPts.GetPts()}}
	default:
		return map[string]any{}
	}
}

func directResponseFailure(errorValue *axonpb.Error, terminalState string, stage string) map[string]any {
	if errorValue != nil {
		return directAxonFailure(errorValue, directErrorStage(errorValue.GetStage(), stage))
	}
	switch terminalState {
	case "Completed", "Accepted", "Admitted", "Dispatched", "Running":
		return nil
	case "TimedOut":
		return map[string]any{"code": string(ErrTimeout), "stage": stage, "message": "daemon invocation ended in TimedOut", "retryable": true}
	case "Cancelled":
		return map[string]any{"code": string(ErrCancelled), "stage": stage, "message": "daemon invocation ended in Cancelled", "retryable": false}
	case "Failed":
		return map[string]any{"code": string(ErrAbilityFailed), "stage": stage, "message": "daemon invocation ended in Failed", "retryable": false}
	default:
		return nil
	}
}

func directAxonFailure(errorValue *axonpb.Error, stage string) map[string]any {
	code := runtimeFailureCode(errorValue.GetCode(), ErrAdmissionDenied)
	if code == "" || code == ErrGeneric {
		code = ErrAdmissionDenied
	}
	return map[string]any{
		"code":      string(code),
		"stage":     stage,
		"message":   errorValue.GetMessage(),
		"retryable": errorValue.GetRetryable(),
	}
}

func directErrorStage(stage axonpb.ErrorStage, fallback string) string {
	name := axonpb.ErrorStage_name[int32(stage)]
	if name == "" {
		return fallback
	}
	name = strings.TrimPrefix(name, "ERROR_STAGE_")
	if name == "" {
		return fallback
	}
	return strings.ToLower(name)
}

func directReceipt(receipt *axonpb.InvocationReceipt) map[string]any {
	if receipt == nil {
		return nil
	}
	return map[string]any{
		"index":                 receipt.GetIndex(),
		"invocation_id":         receipt.GetInvocationId(),
		"receipt_type":          receipt.GetReceiptType(),
		"state":                 directStateName(receipt.GetState()),
		"timestamp_unix_ms":     receipt.GetTimestampUnixMs(),
		"prev_receipt_hash_hex": hex.EncodeToString(receipt.GetPrevReceiptHash()),
		"self_hash_hex":         hex.EncodeToString(receipt.GetSelfHash()),
		"payload_content_type":  receipt.GetPayloadContentType(),
		"cleanup_complete":      receipt.GetCleanupComplete(),
		"reason":                receipt.GetReason(),
		"child_invocation_id":   receipt.GetChildInvocationId(),
	}
}

func directStateName(state axonpb.InvocationState) string {
	switch state {
	case axonpb.InvocationState_INVOCATION_STATE_ACCEPTED:
		return "Accepted"
	case axonpb.InvocationState_INVOCATION_STATE_ADMITTED:
		return "Admitted"
	case axonpb.InvocationState_INVOCATION_STATE_DISPATCHED:
		return "Dispatched"
	case axonpb.InvocationState_INVOCATION_STATE_RUNNING:
		return "Running"
	case axonpb.InvocationState_INVOCATION_STATE_COMPLETED:
		return "Completed"
	case axonpb.InvocationState_INVOCATION_STATE_FAILED:
		return "Failed"
	case axonpb.InvocationState_INVOCATION_STATE_TIMED_OUT:
		return "TimedOut"
	case axonpb.InvocationState_INVOCATION_STATE_CANCELLED:
		return "Cancelled"
	default:
		return "Unspecified"
	}
}

func directStateTerminal(state axonpb.InvocationState) bool {
	switch state {
	case axonpb.InvocationState_INVOCATION_STATE_COMPLETED,
		axonpb.InvocationState_INVOCATION_STATE_FAILED,
		axonpb.InvocationState_INVOCATION_STATE_TIMED_OUT,
		axonpb.InvocationState_INVOCATION_STATE_CANCELLED:
		return true
	default:
		return false
	}
}

func directStreamEventKind(terminal bool) string {
	if terminal {
		return "terminal"
	}
	return "chunk"
}

func directOutputJSON(payload []byte, contentType string) any {
	if len(payload) == 0 || !strings.Contains(strings.ToLower(contentType), "json") {
		return nil
	}
	var value any
	if err := json.Unmarshal(payload, &value); err != nil {
		return nil
	}
	return value
}

func directRuntimeGRPCError(err error, endpoint string) error {
	statusValue, ok := status.FromError(err)
	if !ok {
		return directRuntimeError(
			fmt.Sprintf("daemon invocation endpoint failed: %v", err),
			ErrRouteUnavailable,
			RetryUnknown,
			map[string]any{"endpoint": endpoint},
			err,
		)
	}
	code, retry := ErrRouteUnavailable, RetryUnknown
	retryable := false
	switch statusValue.Code() {
	case codes.Canceled:
		code, retry, retryable = ErrCancelled, RetryUnknown, false
	case codes.DeadlineExceeded:
		code, retry, retryable = ErrTimeout, RetrySafe, true
	case codes.Unavailable:
		code, retry, retryable = ErrDaemonOffline, RetrySafe, true
	case codes.InvalidArgument:
		code, retry, retryable = ErrInvalidInvocation, RetryNever, false
	case codes.PermissionDenied:
		code, retry, retryable = ErrPermissionDenied, RetryNever, false
	case codes.NotFound:
		code, retry, retryable = ErrAbilityNotFound, RetryNever, false
	case codes.Unimplemented:
		code, retry, retryable = ErrProtocolMismatch, RetryNever, false
	case codes.Unknown, codes.Internal, codes.DataLoss:
		code, retry, retryable = ErrProtocolMismatch, RetryNever, false
	}
	return &SDKError{
		Code:      code,
		Stage:     "direct_runtime",
		Retry:     retry,
		Retryable: retryable,
		Message:   statusValue.Message(),
		Details:   map[string]any{"endpoint": endpoint, "grpc_status": statusValue.Code().String()},
		Cause:     err,
	}
}

func directRuntimeError(message string, code ErrorCode, retry RetryHint, details map[string]any, cause error) error {
	return &SDKError{
		Code:      code,
		Stage:     "direct_runtime",
		Retry:     retry,
		Retryable: RetryableForHint(retry),
		Message:   message,
		Details:   details,
		Cause:     cause,
	}
}

func durationFromMillis(value int64, fallback time.Duration) time.Duration {
	if value <= 0 {
		return fallback
	}
	return time.Duration(value) * time.Millisecond
}

func directRuntimeDialTarget(endpoint string) (string, []grpc.DialOption, error) {
	endpoint = strings.TrimSpace(endpoint)
	if endpoint == "" {
		return "", nil, invalidRuntimeClient("endpoint is required")
	}
	if directRuntimeEndpointIsUDS(endpoint) {
		socketPath := strings.TrimPrefix(endpoint, "unix://")
		return "passthrough:///easynet-daemon", []grpc.DialOption{
			grpc.WithTransportCredentials(insecure.NewCredentials()),
			grpc.WithContextDialer(directRuntimeUDSDialer(socketPath)),
		}, nil
	}
	if !strings.Contains(endpoint, "://") {
		return endpoint, []grpc.DialOption{grpc.WithTransportCredentials(insecure.NewCredentials())}, nil
	}
	parsed, err := url.Parse(endpoint)
	if err != nil {
		return "", nil, invalidRuntimePayload(fmt.Sprintf("parse daemon invocation endpoint: %v", err), err)
	}
	switch parsed.Scheme {
	case "http", "https":
		return "", nil, directRuntimeError(
			"http(s) endpoints are Hub/public endpoints, not direct daemon Invocation endpoints; use unix://, grpc://, grpcs://, axon://, or host:port",
			ErrProtocolMismatch,
			RetryNever,
			map[string]any{"endpoint": endpoint, "scheme": parsed.Scheme},
			nil,
		)
	case "grpc":
		target := parsed.Host
		if target == "" {
			return "", nil, invalidRuntimePayload("grpc daemon invocation endpoint requires host", nil)
		}
		return target, []grpc.DialOption{grpc.WithTransportCredentials(insecure.NewCredentials())}, nil
	case "grpcs", "axon":
		target := parsed.Host
		if target == "" {
			return "", nil, invalidRuntimePayload(parsed.Scheme+" daemon invocation endpoint requires host", nil)
		}
		// Direct runtime discovery currently does not carry a CA bundle. The
		// endpoint is read from the local daemon control plane; use TLS for the
		// transport class and leave endpoint authenticity to daemon discovery.
		tlsConfig := &tls.Config{ServerName: parsed.Hostname(), InsecureSkipVerify: true}
		return target, []grpc.DialOption{grpc.WithTransportCredentials(credentials.NewTLS(tlsConfig))}, nil
	default:
		return "", nil, invalidRuntimePayload(fmt.Sprintf("unsupported daemon invocation endpoint scheme %q", parsed.Scheme), nil)
	}
}

func directRuntimeEndpointIsUDS(endpoint string) bool {
	return strings.HasPrefix(endpoint, "/") || strings.HasPrefix(endpoint, "unix://")
}

func directRuntimeUDSDialer(socketPath string) func(context.Context, string) (net.Conn, error) {
	return func(ctx context.Context, _ string) (net.Conn, error) {
		dialer := net.Dialer{}
		return dialer.DialContext(ctx, "unix", socketPath)
	}
}

func numericJSONValue(value any) float64 {
	number, _ := value.(float64)
	return number
}
