package easynet

import (
	"context"
	"encoding/base64"
	"encoding/hex"
	"encoding/json"
	"fmt"
	"io"
	"net"
	"strings"
	"sync"
	"time"

	"easynet.run/cli/sdk/go/internal/axonpb"
	"google.golang.org/grpc"
	"google.golang.org/grpc/codes"
	"google.golang.org/grpc/credentials/insecure"
	"google.golang.org/grpc/status"
)

const (
	defaultDirectRuntimeDialTimeout   = 3 * time.Second
	defaultDirectRuntimeInvokeTimeout = 60 * time.Second
	defaultDirectRuntimeTransportName = "direct-axon-grpc-uds"
	defaultURAProfile                 = "easynet-strict-v2"
)

// DirectDaemonRuntimeConnector opens a concrete daemon Runtime Core transport
// over the daemon's Axon Invocation gRPC endpoint.
type DirectDaemonRuntimeConnector struct {
	ControlPath string
	Reader      ControlDiscoveryReader

	mu         sync.Mutex
	transports map[*DirectDaemonRuntimeTransport]struct{}
	closed     bool
}

// NewDirectDaemonRuntimeConnector creates a direct daemon Runtime connector.
func NewDirectDaemonRuntimeConnector(controlPath string, reader ControlDiscoveryReader) *DirectDaemonRuntimeConnector {
	if reader == nil {
		reader = FileControlDiscoveryReader{}
	}
	return &DirectDaemonRuntimeConnector{
		ControlPath: controlPath,
		Reader:      reader,
		transports:  map[*DirectDaemonRuntimeTransport]struct{}{},
	}
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
	transport, err := OpenDirectDaemonRuntimeTransport(ctx, endpoint.Endpoint, DirectRuntimeOptions{
		DialTimeoutMS:   options.DialTimeoutMS,
		InvokeTimeoutMS: options.InvokeTimeoutMS,
		MaxMessageBytes: options.MaxMessageBytes,
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
		"prepare":       false,
		"submit_signed": false,
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

// DirectRuntimeOptions are SDK-internal direct daemon transport knobs.
type DirectRuntimeOptions struct {
	DialTimeoutMS   int64
	InvokeTimeoutMS int64
	MaxMessageBytes int
	HandleTransport RuntimeTransport
}

// DirectDaemonRuntimeTransport is a concrete RuntimeTransport over Axon gRPC UDS.
type DirectDaemonRuntimeTransport struct {
	mu            sync.Mutex
	conn          *grpc.ClientConn
	client        axonpb.InvocationClient
	endpoint      string
	invokeTimeout time.Duration
	handle        RuntimeTransport
	closed        bool
}

// OpenDirectDaemonRuntimeTransport opens a direct daemon Runtime transport.
func OpenDirectDaemonRuntimeTransport(ctx context.Context, endpoint string, options DirectRuntimeOptions) (*DirectDaemonRuntimeTransport, error) {
	if ctx == nil {
		return nil, invalidRuntimeClient("context is required")
	}
	if strings.TrimSpace(endpoint) == "" {
		return nil, invalidRuntimeClient("endpoint is required")
	}
	dialTimeout := durationFromMillis(options.DialTimeoutMS, defaultDirectRuntimeDialTimeout)
	invokeTimeout := durationFromMillis(options.InvokeTimeoutMS, defaultDirectRuntimeInvokeTimeout)
	dialCtx, cancel := context.WithTimeout(ctx, dialTimeout)
	defer cancel()
	dialOptions := []grpc.DialOption{
		grpc.WithTransportCredentials(insecure.NewCredentials()),
		grpc.WithContextDialer(directRuntimeDialer),
		grpc.WithBlock(),
	}
	if options.MaxMessageBytes > 0 {
		dialOptions = append(
			dialOptions,
			grpc.WithDefaultCallOptions(
				grpc.MaxCallRecvMsgSize(options.MaxMessageBytes),
				grpc.MaxCallSendMsgSize(options.MaxMessageBytes),
			),
		)
	}
	conn, err := grpc.DialContext(dialCtx, grpcUDSTarget(endpoint), dialOptions...)
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
		conn:          conn,
		client:        axonpb.NewInvocationClient(conn),
		endpoint:      endpoint,
		invokeTimeout: invokeTimeout,
		handle:        options.HandleTransport,
	}, nil
}

func (t *DirectDaemonRuntimeTransport) Invoke(ctx context.Context, draftJSON []byte) ([]byte, error) {
	client, timeout, err := t.requireOpen(ctx)
	if err != nil {
		return nil, err
	}
	draft, request, err := directInvokeRequestFromDraftJSON(draftJSON)
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
	_, request, err := directStreamRequestFromDraftJSON(draftJSON)
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
	openFrame, err := directBidiOpenFrame(draft, streams)
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
	handle, err := t.requireHandleTransport(ctx)
	if err != nil {
		return nil, err
	}
	return handle.Prepare(ctx, draftJSON, optionsJSON)
}

func (t *DirectDaemonRuntimeTransport) SubmitSigned(ctx context.Context, signedJSON []byte) ([]byte, error) {
	handle, err := t.requireHandleTransport(ctx)
	if err != nil {
		return nil, err
	}
	return handle.SubmitSigned(ctx, signedJSON)
}

func (t *DirectDaemonRuntimeTransport) AwaitHandle(ctx context.Context, handleID uint64) ([]byte, error) {
	handle, err := t.requireHandleTransport(ctx)
	if err != nil {
		return nil, err
	}
	return handle.AwaitHandle(ctx, handleID)
}

func (t *DirectDaemonRuntimeTransport) CancelHandle(ctx context.Context, handleID uint64, reason string) ([]byte, error) {
	handle, err := t.requireHandleTransport(ctx)
	if err != nil {
		return nil, err
	}
	return handle.CancelHandle(ctx, handleID, reason)
}

func (t *DirectDaemonRuntimeTransport) HandleEvents(ctx context.Context, handleID uint64) ([]byte, error) {
	handle, err := t.requireHandleTransport(ctx)
	if err != nil {
		return nil, err
	}
	return handle.HandleEvents(ctx, handleID)
}

func (t *DirectDaemonRuntimeTransport) FreeHandle(ctx context.Context, handleID uint64) error {
	handle, err := t.requireHandleTransport(ctx)
	if err != nil {
		return err
	}
	return handle.FreeHandle(ctx, handleID)
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
	t.conn = nil
	t.client = nil
	t.closed = true
	t.mu.Unlock()
	if conn == nil {
		return nil
	}
	if err := conn.Close(); err != nil {
		return transportRuntimeError("close direct runtime transport failed", err)
	}
	return nil
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

func (t *DirectDaemonRuntimeTransport) requireHandleTransport(ctx context.Context) (RuntimeTransport, error) {
	if _, _, err := t.requireOpen(ctx); err != nil {
		return nil, err
	}
	t.mu.Lock()
	defer t.mu.Unlock()
	if t.handle == nil {
		return nil, directRuntimeError(
			"direct daemon handle transport is not configured",
			ErrNotImplemented,
			RetryNever,
			map[string]any{"transport": defaultDirectRuntimeTransportName},
			nil,
		)
	}
	return t.handle, nil
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

func directInvokeRequestFromDraftJSON(raw []byte) (InvocationDraft, *axonpb.InvokeRequest, error) {
	draft, err := NewInvocationDraftFromJSON(raw)
	if err != nil {
		return InvocationDraft{}, nil, err
	}
	fields, err := directInvokeFields(draft)
	if err != nil {
		return InvocationDraft{}, nil, err
	}
	return draft, &axonpb.InvokeRequest{
		Envelope:        fields.envelope,
		Target:          directInvocationTarget(draft),
		FunctionName:    draft.DescriptorRef(),
		Arguments:       fields.arguments,
		ContentType:     draft.ContentType(),
		Metadata:        fields.metadata,
		ContentEnvelope: fields.contentEnvelope,
	}, nil
}

func directStreamRequestFromDraftJSON(raw []byte) (InvocationDraft, *axonpb.InvokeServerStreamRequest, error) {
	draft, err := NewInvocationDraftFromJSON(raw)
	if err != nil {
		return InvocationDraft{}, nil, err
	}
	fields, err := directInvokeFields(draft)
	if err != nil {
		return InvocationDraft{}, nil, err
	}
	return draft, &axonpb.InvokeServerStreamRequest{
		Envelope:        fields.envelope,
		Target:          directInvocationTarget(draft),
		FunctionName:    draft.DescriptorRef(),
		Arguments:       fields.arguments,
		ContentType:     draft.ContentType(),
		Metadata:        fields.metadata,
		ContentEnvelope: fields.contentEnvelope,
	}, nil
}

type directInvokeFieldSet struct {
	envelope        *axonpb.Envelope
	arguments       []byte
	metadata        map[string]string
	contentEnvelope *axonpb.ContentEnvelope
}

func directInvokeFields(draft InvocationDraft) (directInvokeFieldSet, error) {
	nonce, err := base64.StdEncoding.DecodeString(draft.NonceBase64())
	if err != nil {
		return directInvokeFieldSet{}, invalidRuntimePayload(fmt.Sprintf("decode nonce_base64: %v", err), err)
	}
	causal, err := directCausalContext(draft.CausalContext())
	if err != nil {
		return directInvokeFieldSet{}, err
	}
	args, err := directArguments(draft)
	if err != nil {
		return directInvokeFieldSet{}, err
	}
	metadata, err := directMetadata(draft.Metadata())
	if err != nil {
		return directInvokeFieldSet{}, err
	}
	callerSignature, err := directCallerSignature(draft)
	if err != nil {
		return directInvokeFieldSet{}, err
	}
	return directInvokeFieldSet{
		envelope: &axonpb.Envelope{
			RequestId:       fmt.Sprintf("req-%d", time.Now().UnixNano()),
			Caller:          directAgentIdentity(draft.CallerURA()),
			Callee:          directAgentIdentity(draft.CalleeURA()),
			Subject:         &axonpb.SubjectIdentity{Ura: draft.SubjectURA(), Profile: defaultURAProfile},
			InvocationNonce: nonce,
			CausalContext:   causal,
			CallerSignature: callerSignature,
		},
		arguments: args,
		metadata:  metadata,
		contentEnvelope: &axonpb.ContentEnvelope{
			ContentType: draft.ContentType(),
			Encoding:    "identity",
		},
	}, nil
}

func directBidiOpenFrame(draft InvocationDraft, streams []*axonpb.StreamDescriptor) (*axonpb.InvokeBidiUp, error) {
	fields, err := directInvokeFields(draft)
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
			Target:          directInvocationTarget(draft),
			InitialArgs:     fields.arguments,
			ArgsContentType: draft.ContentType(),
			Streams:         streams,
			Metadata:        fields.metadata,
			ContentEnvelope: fields.contentEnvelope,
		}},
	}, nil
}

func directInvocationTarget(draft InvocationDraft) *axonpb.InvocationTarget {
	return &axonpb.InvocationTarget{
		AbilityName: draft.DescriptorRef(),
		TypedTarget: &axonpb.InvocationTarget_Ability{
			Ability: &axonpb.AbilityTarget{
				AbilityName:  draft.DescriptorRef(),
				FunctionName: draft.DescriptorRef(),
			},
		},
	}
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
	return &axonpb.CallerSignature{
		Algorithm: signature.Algorithm,
		Signature: decoded,
		KeyIdHint: signature.KeyIDHint,
	}, nil
}

func directArguments(draft InvocationDraft) ([]byte, error) {
	if draft.ArgumentsBase64() != "" {
		decoded, err := base64.StdEncoding.DecodeString(draft.ArgumentsBase64())
		if err != nil {
			return nil, invalidRuntimePayload(fmt.Sprintf("decode arguments_base64: %v", err), err)
		}
		return decoded, nil
	}
	raw, err := json.Marshal(draft.JSONArgs())
	if err != nil {
		return nil, invalidRuntimePayload(fmt.Sprintf("encode args JSON: %v", err), err)
	}
	return raw, nil
}

func directMetadata(metadata map[string]any) (map[string]string, error) {
	result := map[string]string{}
	for key, value := range metadata {
		stringValue, ok := value.(string)
		if !ok {
			return nil, invalidRuntimePayload("metadata must be a string-to-string map for Axon InvokeRequest", nil)
		}
		result[key] = stringValue
	}
	return result, nil
}

func directCausalContext(value map[string]any) (*axonpb.CausalContext, error) {
	form, _ := value["form"].(string)
	if form == "" {
		form, _ = value["kind"].(string)
	}
	switch form {
	case "", "none", "empty", "null":
		return &axonpb.CausalContext{Form: &axonpb.CausalContext_None{None: &axonpb.Empty{}}}, nil
	case "scalar":
		ref, err := directReceiptRef(value)
		if err != nil {
			return nil, err
		}
		return &axonpb.CausalContext{Form: &axonpb.CausalContext_Scalar{Scalar: ref}}, nil
	case "list":
		rawPrior, ok := value["prior"].([]any)
		if !ok {
			return nil, invalidRuntimePayload("causal_context.prior must be an array", nil)
		}
		prior := make([]*axonpb.ReceiptRef, 0, len(rawPrior))
		for _, item := range rawPrior {
			ref, err := directReceiptRef(item)
			if err != nil {
				return nil, err
			}
			prior = append(prior, ref)
		}
		return &axonpb.CausalContext{Form: &axonpb.CausalContext_List{List: &axonpb.ReceiptList{Prior: prior}}}, nil
	case "merkle":
		rootHex, _ := value["root_hex"].(string)
		root, err := hex.DecodeString(rootHex)
		if err != nil {
			return nil, invalidRuntimePayload(fmt.Sprintf("decode root_hex: %v", err), err)
		}
		proofURA, _ := value["proof_ura"].(string)
		if proofURA == "" {
			return nil, invalidRuntimePayload("proof_ura is required", nil)
		}
		return &axonpb.CausalContext{Form: &axonpb.CausalContext_Merkle{Merkle: &axonpb.MerkleRoot{Root: root, ProofUra: proofURA}}}, nil
	default:
		return nil, invalidRuntimePayload(fmt.Sprintf("unknown causal_context form: %s", form), nil)
	}
}

func directReceiptRef(value any) (*axonpb.ReceiptRef, error) {
	item, ok := value.(map[string]any)
	if !ok {
		return nil, invalidRuntimePayload("causal receipt ref must be an object", nil)
	}
	receiptURA, _ := item["receipt_ura"].(string)
	hashHex, _ := item["receipt_hash_hex"].(string)
	if receiptURA == "" || hashHex == "" {
		return nil, invalidRuntimePayload("receipt_ura and receipt_hash_hex are required", nil)
	}
	receiptHash, err := hex.DecodeString(hashHex)
	if err != nil {
		return nil, invalidRuntimePayload(fmt.Sprintf("decode receipt_hash_hex: %v", err), err)
	}
	return &axonpb.ReceiptRef{ReceiptUra: receiptURA, ReceiptHash: receiptHash}, nil
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
	code := NormalizeErrorCode(errorValue.GetCode())
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

func grpcUDSTarget(endpoint string) string {
	return strings.TrimPrefix(endpoint, "unix://")
}

func directRuntimeDialer(ctx context.Context, address string) (net.Conn, error) {
	dialer := net.Dialer{}
	return dialer.DialContext(ctx, "unix", strings.TrimPrefix(address, "unix:"))
}

func numericJSONValue(value any) float64 {
	number, _ := value.(float64)
	return number
}
