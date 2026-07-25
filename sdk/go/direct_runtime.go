//go:build runtime_direct

package easynet

import (
	"context"
	"crypto/tls"
	"encoding/base64"
	"encoding/hex"
	"encoding/json"
	"errors"
	"fmt"
	"io"
	"net"
	"net/url"
	"strings"
	"sync"
	"time"

	axoninv "axon.run/sdk/go/axon/invocation"
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
)

// directRuntimeConnector opens a concrete Runtime Core transport over an Axon
// Invocation gRPC endpoint.
type directRuntimeConnector struct {
	controlPath string
	reader      controlDiscoveryReader

	mu                   sync.Mutex
	handle               RuntimeTransport
	closeHandleTransport bool
	transports           map[*directRuntimeTransport]struct{}
	closed               bool
}

type directRuntimeConnectorOptions struct {
	controlPath          string
	reader               controlDiscoveryReader
	handleTransport      RuntimeTransport
	closeHandleTransport bool
}

func newDirectRuntimeConnector(controlPath string, reader controlDiscoveryReader) *directRuntimeConnector {
	return newDirectRuntimeConnectorWithOptions(directRuntimeConnectorOptions{
		controlPath: controlPath,
		reader:      reader,
	})
}

func newDirectRuntimeConnectorWithOptions(options directRuntimeConnectorOptions) *directRuntimeConnector {
	reader := options.reader
	if reader == nil {
		reader = fileControlDiscoveryReader{}
	}
	return &directRuntimeConnector{
		controlPath:          options.controlPath,
		reader:               reader,
		handle:               options.handleTransport,
		closeHandleTransport: options.closeHandleTransport,
		transports:           map[*directRuntimeTransport]struct{}{},
	}
}

func (c *directRuntimeConnector) Resolve(ctx context.Context, optionsJSON []byte) ([]byte, error) {
	if err := c.requireOpen(ctx); err != nil {
		return nil, err
	}
	options, err := decodeConnectOptionsJSON(optionsJSON)
	if err != nil {
		return nil, err
	}
	controlPath := options.ControlPath
	if controlPath == "" {
		controlPath = c.controlPath
	}
	endpoint := options.Endpoint
	if endpoint == "" {
		discovery, err := c.reader.readControlDiscovery(ctx, controlPath)
		if err != nil {
			return nil, err
		}
		if discovery.invocationEndpoint == "" {
			return nil, &SDKError{
				Code:      ErrControlOnly,
				Stage:     "direct_runtime.resolve",
				Retry:     RetrySafe,
				Retryable: RetryableForHint(RetrySafe),
				Message:   "control discovery did not advertise invocation_endpoint",
				Details:   map[string]any{"control_path": controlPath},
			}
		}
		endpoint = discovery.invocationEndpoint
	}
	return directRuntimeEndpointJSON(RuntimeEndpoint{
		Endpoint:        endpoint,
		ControlPath:     controlPath,
		ProtocolVersion: "axon.v1.Invocation",
	}, options)
}

func (c *directRuntimeConnector) Handshake(ctx context.Context, endpointJSON []byte) (RuntimeTransport, []byte, error) {
	if err := c.requireOpen(ctx); err != nil {
		return nil, nil, err
	}
	options, endpoint, err := decodeDirectRuntimeEndpoint(endpointJSON)
	if err != nil {
		return nil, nil, err
	}
	handleTransport := c.handleTransport(ctx)
	transport, err := openDirectRuntimeTransport(ctx, endpoint.Endpoint, directRuntimeOptions{
		DialTimeoutMS:   options.DialTimeoutMS,
		InvokeTimeoutMS: options.InvokeTimeoutMS,
		MaxMessageBytes: options.MaxMessageBytes,
		HandleTransport: handleTransport,
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

func (c *directRuntimeConnector) Close(ctx context.Context) error {
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
	c.closeHandleTransport = false
	transports := make([]*directRuntimeTransport, 0, len(c.transports))
	for transport := range c.transports {
		transports = append(transports, transport)
	}
	c.transports = map[*directRuntimeTransport]struct{}{}
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

func (c *directRuntimeConnector) requireOpen(ctx context.Context) error {
	if c == nil || c.reader == nil {
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

func (c *directRuntimeConnector) handleTransport(ctx context.Context) RuntimeTransport {
	if c == nil || ctx == nil {
		return nil
	}
	c.mu.Lock()
	defer c.mu.Unlock()
	return c.handle
}

type directRuntimeOptions struct {
	DialTimeoutMS   int64
	InvokeTimeoutMS int64
	MaxMessageBytes int
	HandleTransport RuntimeTransport
}

// directRuntimeTransport is a concrete RuntimeTransport over Axon gRPC UDS.
type directRuntimeTransport struct {
	mu            sync.Mutex
	conn          *grpc.ClientConn
	client        axonpb.InvocationClient
	endpoint      string
	invokeTimeout time.Duration
	handle        RuntimeTransport
	codec         *directDescriptorBoundCodec
	closed        bool
}

func openDirectRuntimeTransport(ctx context.Context, endpoint string, options directRuntimeOptions) (*directRuntimeTransport, error) {
	if ctx == nil {
		return nil, invalidRuntimeClient("context is required")
	}
	if strings.TrimSpace(endpoint) == "" {
		return nil, invalidRuntimeClient("endpoint is required")
	}
	dialTimeout := durationFromMillis(options.DialTimeoutMS, defaultDirectRuntimeDialTimeout)
	invokeTimeout := durationFromMillis(options.InvokeTimeoutMS, defaultDirectRuntimeInvokeTimeout)
	codec, err := newDirectDescriptorBoundCodec(invokeTimeout)
	if err != nil {
		return nil, err
	}
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
			"runtime invocation endpoint is not ready",
			ErrRuntimeOffline,
			RetrySafe,
			map[string]any{"endpoint": endpoint},
			err,
		)
	}
	return &directRuntimeTransport{
		conn:          conn,
		client:        axonpb.NewInvocationClient(conn),
		endpoint:      endpoint,
		invokeTimeout: invokeTimeout,
		handle:        options.HandleTransport,
		codec:         codec,
	}, nil
}

func (t *directRuntimeTransport) Invoke(ctx context.Context, draftJSON []byte) ([]byte, error) {
	client, timeout, err := t.requireOpen(ctx)
	if err != nil {
		return nil, err
	}
	invocation, err := t.codec.decode(ctx, draftJSON, axoninv.CallModeRPC)
	if err != nil {
		return nil, err
	}
	request, err := invocation.unary()
	if err != nil {
		return nil, err
	}
	callCtx, cancel := context.WithTimeout(ctx, timeout)
	defer cancel()
	response, err := client.Invoke(callCtx, request)
	if err != nil {
		return nil, directRuntimeGRPCError(err, t.endpoint)
	}
	return directInvokeResponseJSON(invocation.draft, response)
}

func (t *directRuntimeTransport) OpenStream(ctx context.Context, draftJSON []byte) (StreamTransport, []byte, error) {
	client, timeout, err := t.requireOpen(ctx)
	if err != nil {
		return nil, nil, err
	}
	invocation, err := t.codec.decode(ctx, draftJSON, axoninv.CallModeStream)
	if err != nil {
		return nil, nil, err
	}
	request, err := invocation.stream()
	if err != nil {
		return nil, nil, err
	}
	callCtx, cancel := context.WithTimeout(ctx, timeout)
	stream, err := client.InvokeStream(callCtx, request)
	if err != nil {
		cancel()
		return nil, nil, directRuntimeGRPCError(err, t.endpoint)
	}
	transport := newDirectRuntimeStreamTransport(stream, cancel, t.endpoint)
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

func (t *directRuntimeTransport) OpenBidi(ctx context.Context, draftJSON []byte, streamsJSON []byte) (BidiTransport, []byte, error) {
	client, timeout, err := t.requireOpen(ctx)
	if err != nil {
		return nil, nil, err
	}
	invocation, err := t.codec.decode(ctx, draftJSON, axoninv.CallModeBidi)
	if err != nil {
		return nil, nil, err
	}
	streams, err := directBidiStreamDescriptors(streamsJSON)
	if err != nil {
		return nil, nil, err
	}
	openFrame, err := invocation.bidi(streams)
	if err != nil {
		return nil, nil, err
	}
	callCtx, cancel := context.WithTimeout(ctx, timeout)
	stream, err := client.InvokeBidi(callCtx)
	if err != nil {
		cancel()
		return nil, nil, directRuntimeGRPCError(err, t.endpoint)
	}
	transport, err := newDirectRuntimeBidiTransport(stream, cancel, t.endpoint, openFrame)
	if err != nil {
		return nil, nil, err
	}
	openJSON, err := runtimeBidiOpenJSON(transport.sessionID, MaxBidiBufferedFrames)
	if err != nil {
		_ = transport.Close(ctx)
		return nil, nil, invalidRuntimePayload(fmt.Sprintf("encode direct bidi open JSON: %v", err), err)
	}
	return transport, openJSON, nil
}

func (t *directRuntimeTransport) Prepare(ctx context.Context, draftJSON []byte, optionsJSON []byte) ([]byte, error) {
	handle, err := t.requireHandleTransport(ctx)
	if err != nil {
		return nil, err
	}
	draft, err := NewInvocationDraftFromJSON(draftJSON)
	if err != nil {
		return nil, err
	}
	if _, err := descriptorBoundInvocationDraft(draft); err != nil {
		return nil, invalidRuntimePayload(
			fmt.Sprintf("build Axon descriptor-bound prepare draft: %v", err),
			err,
		)
	}
	projectedJSON, err := json.Marshal(draft)
	if err != nil {
		return nil, invalidRuntimePayload(fmt.Sprintf("encode direct prepare draft: %v", err), err)
	}
	return handle.Prepare(ctx, projectedJSON, optionsJSON)
}

func (t *directRuntimeTransport) SubmitSigned(ctx context.Context, signedJSON []byte) ([]byte, error) {
	handle, err := t.requireHandleTransport(ctx)
	if err != nil {
		return nil, err
	}
	return handle.SubmitSigned(ctx, signedJSON)
}

func (t *directRuntimeTransport) AwaitHandle(ctx context.Context, control InvocationControlCapability) ([]byte, error) {
	handle, err := t.requireHandleTransport(ctx)
	if err != nil {
		return nil, err
	}
	return handle.AwaitHandle(ctx, control)
}

func (t *directRuntimeTransport) CancelHandle(ctx context.Context, control InvocationControlCapability, reason string) ([]byte, error) {
	handle, err := t.requireHandleTransport(ctx)
	if err != nil {
		return nil, err
	}
	return handle.CancelHandle(ctx, control, reason)
}

func (t *directRuntimeTransport) HandleEvents(ctx context.Context, control InvocationControlCapability) ([]byte, error) {
	handle, err := t.requireHandleTransport(ctx)
	if err != nil {
		return nil, err
	}
	return handle.HandleEvents(ctx, control)
}

func (t *directRuntimeTransport) FreeHandle(ctx context.Context, control InvocationControlCapability) error {
	handle, err := t.requireHandleTransport(ctx)
	if err != nil {
		return err
	}
	return handle.FreeHandle(ctx, control)
}

func (t *directRuntimeTransport) Close(ctx context.Context) error {
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
	t.handle = nil
	t.codec = nil
	t.closed = true
	t.mu.Unlock()
	var closeErr error
	if conn == nil {
		return nil
	}
	if err := conn.Close(); err != nil {
		closeErr = transportRuntimeError("close direct runtime transport failed", err)
	}
	return closeErr
}

func (t *directRuntimeTransport) requireOpen(ctx context.Context) (axonpb.InvocationClient, time.Duration, error) {
	if t == nil {
		return nil, 0, invalidRuntimeClient("runtime transport is not initialized")
	}
	if ctx == nil {
		return nil, 0, invalidRuntimeClient("context is required")
	}
	t.mu.Lock()
	defer t.mu.Unlock()
	if t.closed || t.client == nil || t.codec == nil {
		return nil, 0, invalidRuntimeClient("runtime transport is closed")
	}
	return t.client, t.invokeTimeout, nil
}

func (t *directRuntimeTransport) requireHandleTransport(ctx context.Context) (RuntimeTransport, error) {
	if _, _, err := t.requireOpen(ctx); err != nil {
		return nil, err
	}
	t.mu.Lock()
	defer t.mu.Unlock()
	if t.handle == nil {
		return nil, &SDKError{
			Code:      ErrNotImplemented,
			Stage:     "direct_runtime",
			Retry:     RetryNever,
			Retryable: false,
			Message:   "direct runtime handle transport is not configured",
			Details:   map[string]any{"transport": defaultDirectRuntimeTransportName},
		}
	}
	return t.handle, nil
}

type directRuntimeStreamTransport struct {
	stream   grpc.ServerStreamingClient[axonpb.InvokeStreamChunk]
	cancel   context.CancelFunc
	endpoint string
	streamID string

	mu               sync.Mutex
	closed           bool
	admissionReceipt *axonpb.InvocationReceipt
}

func newDirectRuntimeStreamTransport(stream grpc.ServerStreamingClient[axonpb.InvokeStreamChunk], cancel context.CancelFunc, endpoint string) *directRuntimeStreamTransport {
	return &directRuntimeStreamTransport{
		stream:   stream,
		cancel:   cancel,
		endpoint: endpoint,
		streamID: fmt.Sprintf("direct-stream-%d", time.Now().UnixNano()),
	}
}

func (t *directRuntimeStreamTransport) Recv(ctx context.Context) ([]byte, error) {
	if err := t.requireOpen(ctx); err != nil {
		return nil, err
	}
	chunk, err := t.stream.Recv()
	if err != nil {
		if err == io.EOF {
			return nil, directRuntimeError(
				"runtime stream ended without a terminal frame",
				ErrProtocol,
				RetryNever,
				map[string]any{"endpoint": t.endpoint, "stream_id": t.streamID},
				err,
			)
		}
		return nil, directRuntimeGRPCError(err, t.endpoint)
	}
	t.mu.Lock()
	priorAdmission := t.admissionReceipt
	t.mu.Unlock()
	raw, err := directStreamChunkJSONWithAdmission(chunk, priorAdmission)
	if err != nil {
		return nil, err
	}
	if chunk.GetAdmissionReceipt() != nil {
		t.mu.Lock()
		t.admissionReceipt = chunk.GetAdmissionReceipt()
		t.mu.Unlock()
	}
	return raw, nil
}

func (t *directRuntimeStreamTransport) Cancel(ctx context.Context, reason string) ([]byte, error) {
	if ctx == nil {
		return nil, invalidRuntimeClient("context is required")
	}
	_ = reason
	return nil, unsupportedDirectCancellation(t.endpoint, t.streamID, "stream_cancel")
}

func (t *directRuntimeStreamTransport) Close(ctx context.Context) error {
	if ctx == nil {
		return invalidRuntimeClient("context is required")
	}
	t.close()
	return nil
}

func (t *directRuntimeStreamTransport) close() {
	t.mu.Lock()
	defer t.mu.Unlock()
	if t.closed {
		return
	}
	t.closed = true
	t.cancel()
}

func (t *directRuntimeStreamTransport) requireOpen(ctx context.Context) error {
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

type directRuntimeBidiTransport struct {
	stream    grpc.BidiStreamingClient[axonpb.InvokeBidiUp, axonpb.InvokeBidiDown]
	cancel    context.CancelFunc
	endpoint  string
	sessionID string

	mu               sync.Mutex
	closed           bool
	sendClosed       bool
	lastUpSequence   uint64
	admissionReceipt *axonpb.InvocationReceipt
}

func newDirectRuntimeBidiTransport(
	stream grpc.BidiStreamingClient[axonpb.InvokeBidiUp, axonpb.InvokeBidiDown],
	cancel context.CancelFunc,
	endpoint string,
	openFrame *axonpb.InvokeBidiUp,
) (*directRuntimeBidiTransport, error) {
	if err := validateDirectBidiOpenFrame(openFrame); err != nil {
		cancel()
		return nil, err
	}
	if stream == nil {
		cancel()
		return nil, invalidRuntimeClient("bidi stream is not initialized")
	}
	transport := &directRuntimeBidiTransport{
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

func validateDirectBidiOpenFrame(openFrame *axonpb.InvokeBidiUp) error {
	if openFrame == nil {
		return invalidRuntimePayload("bidi frame0 EnvelopeOpen is required", nil)
	}
	if openFrame.GetSequence() != 0 {
		return invalidRuntimePayload("bidi frame0 sequence must be 0", nil)
	}
	if openFrame.GetEnvelopeOpen() == nil {
		return invalidRuntimePayload("bidi frame0 must carry EnvelopeOpen", nil)
	}
	return nil
}

func (t *directRuntimeBidiTransport) Send(ctx context.Context, frameJSON []byte) ([]byte, error) {
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

func (t *directRuntimeBidiTransport) Recv(ctx context.Context) ([]byte, error) {
	if err := t.requireOpen(ctx); err != nil {
		return nil, err
	}
	for {
		down, err := t.stream.Recv()
		if err != nil {
			if err == io.EOF {
				return nil, directRuntimeError(
					"runtime bidi ended without a terminal frame",
					ErrProtocol,
					RetryNever,
					map[string]any{"endpoint": t.endpoint, "session_id": t.sessionID},
					err,
				)
			}
			return nil, directRuntimeGRPCError(err, t.endpoint)
		}
		role, err := directBidiDownReceiptRole(down)
		if err != nil {
			return nil, err
		}
		if role == directReceiptAdmission {
			if down.GetSequence() != 0 {
				return nil, directRuntimeProtocolError(
					"direct_runtime.bidi",
					"runtime bidi admission receipt must be frame 0",
				)
			}
			t.mu.Lock()
			t.admissionReceipt = down.GetReceipt()
			t.mu.Unlock()
			continue
		}
		if role == directReceiptTerminal {
			t.mu.Lock()
			admissionReceipt := t.admissionReceipt
			t.mu.Unlock()
			terminalStateName, err := directStateName(
				down.GetReceipt().GetState(),
				"direct_runtime.bidi",
			)
			if err != nil {
				return nil, err
			}
			if err := validateDirectReceiptPair(
				admissionReceipt,
				down.GetReceipt(),
				terminalStateName,
				"direct_runtime.bidi",
			); err != nil {
				return nil, err
			}
			admissionProjection, err := directReceipt(admissionReceipt, "direct_runtime.bidi")
			if err != nil {
				return nil, err
			}
			return directBidiDownJSON(down, admissionProjection)
		}
		return directBidiDownJSON(down, nil)
	}
}

func (t *directRuntimeBidiTransport) CloseSend(ctx context.Context) ([]byte, error) {
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

func (t *directRuntimeBidiTransport) Cancel(ctx context.Context, reason string) ([]byte, error) {
	if ctx == nil {
		return nil, invalidRuntimeClient("context is required")
	}
	_ = reason
	return nil, unsupportedDirectCancellation(t.endpoint, t.sessionID, "bidi_cancel")
}

func (t *directRuntimeBidiTransport) Close(ctx context.Context) error {
	if ctx == nil {
		return invalidRuntimeClient("context is required")
	}
	t.close()
	return nil
}

func (t *directRuntimeBidiTransport) close() {
	t.mu.Lock()
	defer t.mu.Unlock()
	if t.closed {
		return
	}
	t.closed = true
	t.sendClosed = true
	t.cancel()
}

func (t *directRuntimeBidiTransport) requireOpen(ctx context.Context) error {
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
	if response == nil {
		return nil, directRuntimeProtocolError("direct_runtime.invoke", "runtime unary response is required")
	}
	stateName, err := directStateName(response.GetState(), "direct_runtime.invoke")
	if err != nil {
		return nil, err
	}
	if response.GetProofError() != nil && response.GetTerminalReceipt() == nil {
		return nil, directRuntimeProtocolError(
			"direct_runtime.invoke",
			"runtime unary proof failure omitted terminal_receipt",
		)
	}
	if (response.GetAdmissionReceipt() == nil) != (response.GetTerminalReceipt() == nil) {
		return nil, directRuntimeProtocolError(
			"direct_runtime.invoke",
			"runtime unary outcome must carry both admission_receipt and terminal_receipt or neither",
		)
	}
	if err := validateDirectReceiptPair(
		response.GetAdmissionReceipt(),
		response.GetTerminalReceipt(),
		stateName,
		"direct_runtime.invoke",
	); err != nil {
		return nil, err
	}
	errorValue := directResponseFailure(response.GetError(), stateName, "direct_runtime.invoke")
	if response.GetProofError() != nil {
		errorValue = directAxonFailure(response.GetProofError(), directErrorStage(response.GetProofError().GetStage()))
	}
	if errorValue == nil && response.GetTerminalReceipt() != nil && response.GetTerminalReceipt().GetFailure() != nil {
		errorValue = directAxonFailure(
			response.GetTerminalReceipt().GetFailure(),
			directErrorStage(response.GetTerminalReceipt().GetFailure().GetStage()),
		)
	}
	if response.GetTerminalReceipt() == nil {
		if err := validateDirectReceiptFreeUnaryRejection(response); err != nil {
			return nil, err
		}
	}
	admissionReceipt, err := directReceipt(response.GetAdmissionReceipt(), "direct_runtime.invoke")
	if err != nil {
		return nil, err
	}
	terminalReceipt, err := directReceipt(response.GetTerminalReceipt(), "direct_runtime.invoke")
	if err != nil {
		return nil, err
	}
	value := map[string]any{
		"ok":                  errorValue == nil && terminalReceipt != nil,
		"tuple":               draft,
		"terminal_state":      stateName,
		"output_content_type": response.GetResultContentType(),
		"output_base64":       base64.StdEncoding.EncodeToString(response.GetResult()),
		"output_json":         directOutputJSON(response.GetResult(), response.GetResultContentType()),
		"elapsed_ms":          response.GetElapsedMs(),
		"admission_receipt":   admissionReceipt,
		"terminal_receipt":    terminalReceipt,
		"error":               errorValue,
	}
	return json.Marshal(value)
}

func validateDirectReceiptFreeUnaryRejection(response *axonpb.InvokeResponse) error {
	if response.GetState() != axonpb.InvocationState_INVOCATION_STATE_FAILED {
		return directRuntimeProtocolError(
			"direct_runtime.invoke",
			"receipt-free unary outcome must be Failed before lifecycle admission",
		)
	}
	if response.GetProofError() != nil {
		return directRuntimeProtocolError(
			"direct_runtime.invoke",
			"receipt-free unary outcome must not carry proof_error",
		)
	}
	errorValue := response.GetError()
	if errorValue == nil {
		return directRuntimeProtocolError(
			"direct_runtime.invoke",
			"receipt-free unary rejection requires a typed pre-admission error",
		)
	}
	if !directPreAdmissionErrorStage(errorValue.GetStage()) {
		return directRuntimeProtocolError(
			"direct_runtime.invoke",
			"receipt-free unary rejection has a non-admission error stage",
		)
	}
	return nil
}

func directPreAdmissionErrorStage(stage axonpb.ErrorStage) bool {
	return isCanonicalPreAdmissionErrorStage(directErrorStage(stage))
}

func directStreamChunkJSON(chunk *axonpb.InvokeStreamChunk) ([]byte, error) {
	return directStreamChunkJSONWithAdmission(chunk, nil)
}

func directStreamChunkJSONWithAdmission(
	chunk *axonpb.InvokeStreamChunk,
	priorAdmission *axonpb.InvocationReceipt,
) ([]byte, error) {
	if chunk == nil {
		return nil, directRuntimeProtocolError("direct_runtime.stream", "runtime stream chunk is required")
	}
	stateName, err := directStateName(chunk.GetState(), "direct_runtime.stream")
	if err != nil {
		return nil, err
	}
	admission := chunk.GetAdmissionReceipt()
	if admission == nil && chunk.GetTerminalReceipt() != nil {
		admission = priorAdmission
	}
	if err := validateDirectReceiptPair(
		admission,
		chunk.GetTerminalReceipt(),
		stateName,
		"direct_runtime.stream",
	); err != nil {
		return nil, err
	}
	errorSource := chunk.GetError()
	if chunk.GetProofError() != nil {
		errorSource = chunk.GetProofError()
	}
	errorValue := directResponseFailure(errorSource, stateName, "direct_runtime.stream")
	terminal := chunk.GetTerminalReceipt() != nil
	terminalClaim := chunk.GetTerminal() || directStateTerminal(chunk.GetState())
	transportTerminal := false
	if !terminal && errorValue != nil {
		transportTerminal = true
		terminalClaim = false
	}
	if terminalClaim != terminal {
		return nil, directRuntimeProtocolError(
			"direct_runtime.stream",
			"runtime stream terminal claim must match terminal_receipt presence",
		)
	}
	kind := directStreamEventKind(terminal)
	if transportTerminal {
		kind = "error"
	}
	value := map[string]any{
		"sequence":             chunk.GetSequence() + 1,
		"kind":                 kind,
		"state":                stateName,
		"terminal":             terminal,
		"transport_terminal":   transportTerminal,
		"payload_content_type": chunk.GetContentType(),
		"payload_base64":       base64.StdEncoding.EncodeToString(chunk.GetPayload()),
		"payload_json":         directOutputJSON(chunk.GetPayload(), chunk.GetContentType()),
		"error":                errorValue,
	}
	if chunk.GetInvocationId() != "" {
		value["invocation_id"] = chunk.GetInvocationId()
	}
	if chunk.GetElapsedMs() != 0 {
		value["elapsed_ms"] = chunk.GetElapsedMs()
	}
	if admission != nil {
		admissionProjection, err := directReceipt(admission, "direct_runtime.stream")
		if err != nil {
			return nil, err
		}
		value["admission_receipt"] = admissionProjection
	}
	if chunk.GetTerminalReceipt() != nil {
		terminalProjection, err := directReceipt(chunk.GetTerminalReceipt(), "direct_runtime.stream")
		if err != nil {
			return nil, err
		}
		value["terminal_receipt"] = terminalProjection
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

func directBidiDownJSON(frame *axonpb.InvokeBidiDown, admissionReceipt map[string]any) ([]byte, error) {
	if frame == nil {
		return nil, directRuntimeProtocolError("direct_runtime.bidi", "runtime bidi frame is required")
	}
	role, err := directBidiDownReceiptRole(frame)
	if err != nil {
		return nil, err
	}
	terminal := role == directReceiptTerminal
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
		receipt, err := directReceipt(payload.Receipt, "direct_runtime.bidi")
		if err != nil {
			return nil, err
		}
		if terminal {
			value["terminal_receipt"] = receipt
			if admissionReceipt != nil {
				value["admission_receipt"] = admissionReceipt
			}
		} else {
			value["admission_receipt"] = receipt
		}
		if failure := payload.Receipt.GetFailure(); failure != nil {
			value["error"] = directAxonFailure(failure, "direct_runtime.bidi")
		}
	case *axonpb.InvokeBidiDown_DispatchCall, *axonpb.InvokeBidiDown_ReverseDispatchResult:
		return nil, directRuntimeProtocolError(
			"direct_runtime.bidi",
			"runtime bidi callback frame is unsupported by the direct invocation capability",
		)
	default:
		return nil, directRuntimeProtocolError("direct_runtime.bidi", "runtime bidi frame did not include a payload")
	}
	return json.Marshal(value)
}

func directBidiDownReceiptRole(frame *axonpb.InvokeBidiDown) (directReceiptRole, error) {
	receipt, ok := frame.GetPayload().(*axonpb.InvokeBidiDown_Receipt)
	if !ok {
		return directReceiptNone, nil
	}
	return directCanonicalReceiptRole(receipt.Receipt, "direct_runtime.bidi")
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
		role, err := directCanonicalReceiptRole(payload.Receipt, "direct_runtime.bidi")
		if err != nil {
			return "invalid_receipt"
		}
		if role == directReceiptTerminal {
			return "terminal"
		}
		return "admission"
	case *axonpb.InvokeBidiDown_DispatchCall, *axonpb.InvokeBidiDown_ReverseDispatchResult:
		return "unsupported_frame"
	default:
		return "unknown"
	}
}

type directReceiptRole uint8

const (
	directReceiptNone directReceiptRole = iota
	directReceiptAdmission
	directReceiptTerminal
)

func directCanonicalReceiptRole(receipt *axonpb.InvocationReceipt, stage string) (directReceiptRole, error) {
	if receipt == nil {
		return directReceiptNone, directRuntimeProtocolError(stage, "runtime receipt payload is required")
	}
	switch receipt.GetState() {
	case axonpb.InvocationState_INVOCATION_STATE_ACCEPTED,
		axonpb.InvocationState_INVOCATION_STATE_ADMITTED:
		if receipt.GetCleanupComplete() {
			return directReceiptNone, directRuntimeProtocolError(stage, "admission receipt must not claim cleanup completion")
		}
		return directReceiptAdmission, nil
	case axonpb.InvocationState_INVOCATION_STATE_COMPLETED,
		axonpb.InvocationState_INVOCATION_STATE_FAILED,
		axonpb.InvocationState_INVOCATION_STATE_TIMED_OUT,
		axonpb.InvocationState_INVOCATION_STATE_CANCELLED:
		return directReceiptTerminal, nil
	default:
		return directReceiptNone, directRuntimeProtocolError(
			stage,
			fmt.Sprintf("runtime receipt has unsupported lifecycle state %d", receipt.GetState()),
		)
	}
}

func validateDirectReceiptPair(
	admission *axonpb.InvocationReceipt,
	terminal *axonpb.InvocationReceipt,
	wireState string,
	stage string,
) error {
	if admission != nil {
		role, err := directCanonicalReceiptRole(admission, stage)
		if err != nil {
			return err
		}
		if role != directReceiptAdmission {
			return directRuntimeProtocolError(stage, "admission_receipt does not carry an admission state")
		}
	}
	if terminal == nil {
		return nil
	}
	role, err := directCanonicalReceiptRole(terminal, stage)
	if err != nil {
		return err
	}
	if role != directReceiptTerminal {
		return directRuntimeProtocolError(stage, "terminal_receipt does not carry a terminal state")
	}
	terminalStateName, err := directStateName(terminal.GetState(), stage)
	if err != nil {
		return err
	}
	if wireState != terminalStateName {
		return directRuntimeProtocolError(stage, "wire state does not match terminal_receipt state")
	}
	if admission == nil {
		return nil
	}
	if admission.GetInvocationId() != "" &&
		terminal.GetInvocationId() != "" &&
		admission.GetInvocationId() != terminal.GetInvocationId() {
		return directRuntimeProtocolError(stage, "admission and terminal receipts bind different invocations")
	}
	if terminal.GetIndex() <= admission.GetIndex() {
		return directRuntimeProtocolError(stage, "terminal receipt index must follow admission receipt index")
	}
	return nil
}

func directRuntimeProtocolError(stage string, message string) error {
	return &SDKError{
		Code:      ErrProtocol,
		Stage:     stage,
		Retry:     RetryNever,
		Retryable: false,
		Message:   message,
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
		return directAxonFailure(errorValue, directErrorStage(errorValue.GetStage()))
	}
	switch terminalState {
	case "Completed", "Accepted", "Admitted", "Dispatched", "Running":
		return nil
	case "TimedOut":
		return map[string]any{"code": string(ErrTimeout), "stage": stage, "message": "runtime invocation ended in TimedOut", "retryable": true}
	case "Cancelled":
		return map[string]any{"code": string(ErrCancelled), "stage": stage, "message": "runtime invocation ended in Cancelled", "retryable": false}
	case "Failed":
		return map[string]any{"code": string(ErrAbilityFailed), "stage": stage, "message": "runtime invocation ended in Failed", "retryable": false}
	default:
		return nil
	}
}

func directAxonFailure(errorValue *axonpb.Error, stage string) map[string]any {
	code := runtimeFailureCode(errorValue.GetCode())
	if code == ErrGeneric {
		code = ErrAdmissionDenied
	}
	return map[string]any{
		"code":      string(code),
		"stage":     stage,
		"message":   errorValue.GetMessage(),
		"retryable": errorValue.GetRetryable(),
	}
}

func directErrorStage(stage axonpb.ErrorStage) string {
	name := axonpb.ErrorStage_name[int32(stage)]
	if name == "" {
		return "direct_runtime.invoke"
	}
	name = strings.TrimPrefix(name, "ERROR_STAGE_")
	if name == "" {
		return "direct_runtime.invoke"
	}
	return strings.ToLower(name)
}

func directReceipt(receipt *axonpb.InvocationReceipt, stage string) (map[string]any, error) {
	if receipt == nil {
		return nil, nil
	}
	stateName, err := directStateName(receipt.GetState(), stage)
	if err != nil {
		return nil, err
	}
	receiptURA, err := directReceiptURA(receipt)
	if err != nil {
		return nil, err
	}
	value := map[string]any{
		"receipt_ura":             receiptURA,
		"index":                   receipt.GetIndex(),
		"invocation_id":           receipt.GetInvocationId(),
		"receipt_type":            receipt.GetReceiptType(),
		"state":                   stateName,
		"timestamp_unix_ms":       receipt.GetTimestampUnixMs(),
		"prev_receipt_hash_hex":   hex.EncodeToString(receipt.GetPrevReceiptHash()),
		"self_hash_hex":           hex.EncodeToString(receipt.GetSelfHash()),
		"payload_content_type":    receipt.GetPayloadContentType(),
		"cleanup_complete":        receipt.GetCleanupComplete(),
		"reason":                  receipt.GetReason(),
		"child_invocation_id":     receipt.GetChildInvocationId(),
		"payload_base64":          base64.StdEncoding.EncodeToString(receipt.GetPayload()),
		"caller_binding":          directAgentBinding(receipt.GetCallerBinding()),
		"callee_binding":          directAgentBinding(receipt.GetCalleeBinding()),
		"subject_binding":         directSubjectBinding(receipt.GetSubjectBinding()),
		"invocation_nonce_base64": base64.StdEncoding.EncodeToString(receipt.GetInvocationNonce()),
		"causal_binding_kind":     directCausalBindingKind(receipt.GetCausalBinding()),
		"causal_binding":          directCausalBinding(receipt.GetCausalBinding()),
		"callee_signature":        directSignature(receipt.GetCalleeSignature()),
		"signer_binding":          directReceiptSignerBinding(receipt),
		"host_attestation_base64": base64.StdEncoding.EncodeToString(receipt.GetHostAttestation()),
		"authority_binding_kind":  directAuthorityBindingKind(receipt.GetAuthorityBinding()),
		"authority_binding":       directAuthorityBinding(receipt.GetAuthorityBinding()),
		"ability_binding":         receipt.GetAbilityBinding(),
		"failure":                 directReceiptFailure(receipt.GetFailure()),
		"usage":                   directReceiptUsage(receipt.GetUsage()),
		"subject_ref":             directEntityRef(receipt.GetSubjectRef()),
		"descriptor_version":      receipt.GetDescriptorVersion(),
		"schema_hash_hex":         hex.EncodeToString(receipt.GetSchemaHash()),
		"impl_hash_hex":           hex.EncodeToString(receipt.GetImplHash()),
		"runtime_env":             receipt.GetRuntimeEnv(),
		"authority_proof":         directAuthorityProof(receipt.GetAuthorityProof()),
		"input_hash_hex":          hex.EncodeToString(receipt.GetInputHash()),
		"output_hash_hex":         hex.EncodeToString(receipt.GetOutputHash()),
		"parent_receipts":         directReceiptRefs(receipt.GetParentReceipts()),
	}
	if payload := receipt.GetPayload(); len(payload) > 0 {
		if json.Valid(payload) {
			var decoded any
			if err := json.Unmarshal(payload, &decoded); err == nil {
				value["payload"] = decoded
			} else {
				value["payload_base64"] = base64.StdEncoding.EncodeToString(payload)
			}
		} else {
			value["payload_base64"] = base64.StdEncoding.EncodeToString(payload)
		}
	}
	return value, nil
}

func directReceiptURA(receipt *axonpb.InvocationReceipt) (string, error) {
	if receipt == nil {
		return "", invalidRuntimePayload("direct runtime receipt is missing", nil)
	}
	callee := receipt.GetCalleeBinding()
	if callee == nil {
		return "", invalidRuntimePayload("direct runtime receipt is missing callee_binding", nil)
	}
	parts, err := ParseURAParts(strings.TrimSpace(callee.GetUra()))
	if err != nil || strings.TrimSpace(parts.Realm) == "" {
		return "", invalidRuntimePayload("direct runtime receipt callee_binding.ura must be a canonical URA", err)
	}
	invocationID := strings.TrimSpace(receipt.GetInvocationId())
	if invocationID == "" || strings.Contains(invocationID, "/") {
		return "", invalidRuntimePayload("direct runtime receipt invocation_id must be owner-local for receipt URA", nil)
	}
	return fmt.Sprintf(
		"%s/resource/runtime/invocation/%s/receipt/%d",
		RealmResourcePrefix(parts.Realm),
		invocationID,
		receipt.GetIndex(),
	), nil
}

func directAgentBinding(binding *axonpb.AgentIdentity) map[string]any {
	if binding == nil {
		return nil
	}
	return map[string]any{"ura": binding.GetUra(), "profile": binding.GetProfile()}
}

func directReceiptSignerBinding(receipt *axonpb.InvocationReceipt) map[string]any {
	if receipt == nil {
		return nil
	}
	if signer := receipt.GetSignerBinding(); signer != nil {
		return directAgentBinding(signer)
	}
	return directAgentBinding(receipt.GetCalleeBinding())
}

func directSubjectBinding(binding *axonpb.SubjectIdentity) map[string]any {
	if binding == nil {
		return nil
	}
	return map[string]any{"ura": binding.GetUra(), "profile": binding.GetProfile()}
}

func directEntityRef(reference *axonpb.EntityRef) map[string]any {
	if reference == nil {
		return nil
	}
	return map[string]any{
		"kind":    int32(reference.GetKind()),
		"ura":     reference.GetUra(),
		"profile": reference.GetProfile(),
	}
}

func directSignature(signature *axonpb.CalleeSignature) map[string]any {
	if signature == nil {
		return nil
	}
	return map[string]any{
		"algorithm":        signature.GetAlgorithm(),
		"signature_base64": base64.StdEncoding.EncodeToString(signature.GetSignature()),
		"key_id_hint":      signature.GetKeyIdHint(),
	}
}

func directReceiptFailure(failure *axonpb.Error) map[string]any {
	if failure == nil {
		return nil
	}
	return map[string]any{
		"code":           failure.GetCode(),
		"message":        failure.GetMessage(),
		"retryable":      failure.GetRetryable(),
		"stage":          int32(failure.GetStage()),
		"security_class": int32(failure.GetSecurityClass()),
	}
}

func directReceiptUsage(usage *axonpb.InvocationUsage) map[string]any {
	if usage == nil {
		return nil
	}
	return map[string]any{
		"tokens_in":      usage.GetTokensIn(),
		"tokens_out":     usage.GetTokensOut(),
		"duration_ms":    usage.GetDurationMs(),
		"external_calls": usage.GetExternalCalls(),
	}
}

func directReceiptRefs(receipts []*axonpb.ReceiptRef) []map[string]any {
	refs := make([]map[string]any, 0, len(receipts))
	for _, receipt := range receipts {
		if receipt == nil {
			continue
		}
		refs = append(refs, map[string]any{
			"receipt_hash_hex": hex.EncodeToString(receipt.GetReceiptHash()),
			"receipt_ura":      receipt.GetReceiptUra(),
		})
	}
	return refs
}

func directAuthorityProof(proof *axonpb.InvocationAuthorityProof) map[string]any {
	if proof == nil {
		return nil
	}
	return map[string]any{
		"proof_type":           proof.GetProofType(),
		"binding_kind":         directAuthorityBindingKind(proof.GetBinding()),
		"binding":              directAuthorityBinding(proof.GetBinding()),
		"proof_payload_base64": base64.StdEncoding.EncodeToString(proof.GetProofPayload()),
		"proof_hash_hex":       hex.EncodeToString(proof.GetProofHash()),
		"issuer":               directAgentBinding(proof.GetIssuer()),
		"signature":            directSignature(proof.GetSignature()),
		"admission_hook":       proof.GetAdmissionHook(),
	}
}

func directCausalBinding(context *axonpb.CausalContext) map[string]any {
	switch form := context.GetForm().(type) {
	case *axonpb.CausalContext_None:
		return map[string]any{"form": "none"}
	case *axonpb.CausalContext_Scalar:
		return map[string]any{"form": "scalar", "receipt": directReceiptRefProjection(form.Scalar)}
	case *axonpb.CausalContext_List:
		prior := make([]map[string]any, 0, len(form.List.GetPrior()))
		for _, receipt := range form.List.GetPrior() {
			prior = append(prior, directReceiptRefProjection(receipt))
		}
		return map[string]any{"form": "list", "prior": prior}
	case *axonpb.CausalContext_Merkle:
		return map[string]any{
			"form":      "merkle",
			"root_hex":  hex.EncodeToString(form.Merkle.GetRoot()),
			"proof_ura": form.Merkle.GetProofUra(),
		}
	default:
		return nil
	}
}

func directReceiptRefProjection(receipt *axonpb.ReceiptRef) map[string]any {
	if receipt == nil {
		return nil
	}
	return map[string]any{
		"receipt_hash_hex": hex.EncodeToString(receipt.GetReceiptHash()),
		"receipt_ura":      receipt.GetReceiptUra(),
	}
}

func directCausalBindingKind(context *axonpb.CausalContext) string {
	switch context.GetForm().(type) {
	case *axonpb.CausalContext_None:
		return "none"
	case *axonpb.CausalContext_Scalar:
		return "scalar"
	case *axonpb.CausalContext_List:
		return "list"
	case *axonpb.CausalContext_Merkle:
		return "merkle"
	default:
		return ""
	}
}

func directAuthorityBinding(binding *axonpb.AuthorityBinding) map[string]any {
	switch authority := binding.GetAuthority().(type) {
	case *axonpb.AuthorityBinding_SelfAuthority:
		return map[string]any{
			"kind":          "self",
			"principal_ura": authority.SelfAuthority.GetPrincipalUra(),
		}
	case *axonpb.AuthorityBinding_DelegatedAuthority:
		value := authority.DelegatedAuthority
		return map[string]any{
			"kind":             "delegation",
			"issuer_ura":       value.GetIssuerUra(),
			"subject_ura":      value.GetSubjectUra(),
			"caller_ura":       value.GetCallerUra(),
			"audience":         value.GetAudience(),
			"scopes":           append([]string(nil), value.GetScopes()...),
			"issued_at_ms":     value.GetIssuedAtMs(),
			"expires_at_ms":    value.GetExpiresAtMs(),
			"signature_base64": base64.StdEncoding.EncodeToString(value.GetSignature()),
		}
	case *axonpb.AuthorityBinding_CapabilityGrant:
		return map[string]any{
			"kind":           "capability",
			"capability_ura": authority.CapabilityGrant.GetCapabilityUra(),
		}
	case *axonpb.AuthorityBinding_PolicyGrant:
		return map[string]any{
			"kind":       "policy",
			"policy_ura": authority.PolicyGrant.GetPolicyUra(),
		}
	case *axonpb.AuthorityBinding_SessionAuthority:
		value := authority.SessionAuthority
		return map[string]any{
			"kind":             "session",
			"issuer_ura":       value.GetBackendUra(),
			"subject_ura":      value.GetUserUra(),
			"session_id":       value.GetSessionId(),
			"scopes":           append([]string(nil), value.GetScopes()...),
			"audiences":        append([]string(nil), value.GetAudiences()...),
			"issued_at_ms":     value.GetIssuedAtMs(),
			"expires_at_ms":    value.GetExpiresAtMs(),
			"signature_base64": base64.StdEncoding.EncodeToString(value.GetSignature()),
		}
	case *axonpb.AuthorityBinding_BootstrapAuthority:
		return map[string]any{
			"kind":          "bootstrap",
			"principal_ura": authority.BootstrapAuthority.GetPrincipalUra(),
			"realm":         authority.BootstrapAuthority.GetRealm(),
			"ability":       authority.BootstrapAuthority.GetAbility(),
		}
	default:
		return nil
	}
}

func directAuthorityBindingKind(binding *axonpb.AuthorityBinding) string {
	switch binding.GetAuthority().(type) {
	case *axonpb.AuthorityBinding_SelfAuthority:
		return "self"
	case *axonpb.AuthorityBinding_DelegatedAuthority:
		return "delegation"
	case *axonpb.AuthorityBinding_CapabilityGrant:
		return "capability"
	case *axonpb.AuthorityBinding_PolicyGrant:
		return "policy"
	case *axonpb.AuthorityBinding_SessionAuthority:
		return "session"
	case *axonpb.AuthorityBinding_BootstrapAuthority:
		return "bootstrap"
	default:
		return ""
	}
}

func directStateName(state axonpb.InvocationState, stage string) (string, error) {
	switch state {
	case axonpb.InvocationState_INVOCATION_STATE_ACCEPTED:
		return "Accepted", nil
	case axonpb.InvocationState_INVOCATION_STATE_ADMITTED:
		return "Admitted", nil
	case axonpb.InvocationState_INVOCATION_STATE_DISPATCHED:
		return "Dispatched", nil
	case axonpb.InvocationState_INVOCATION_STATE_RUNNING:
		return "Running", nil
	case axonpb.InvocationState_INVOCATION_STATE_COMPLETED:
		return "Completed", nil
	case axonpb.InvocationState_INVOCATION_STATE_FAILED:
		return "Failed", nil
	case axonpb.InvocationState_INVOCATION_STATE_TIMED_OUT:
		return "TimedOut", nil
	case axonpb.InvocationState_INVOCATION_STATE_CANCELLED:
		return "Cancelled", nil
	default:
		return "", directRuntimeProtocolError(stage, fmt.Sprintf("runtime invocation state is unsupported: %d", state))
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
	return "data"
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
			fmt.Sprintf("runtime invocation endpoint failed: %v", err),
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
		code, retry, retryable = ErrRuntimeOffline, RetrySafe, true
	case codes.InvalidArgument:
		code, retry, retryable = ErrInvalidInvocation, RetryNever, false
	case codes.PermissionDenied:
		code, retry, retryable = ErrPermissionDenied, RetryNever, false
	case codes.NotFound:
		code, retry, retryable = ErrDescriptorNotFound, RetryNever, false
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

func unsupportedDirectCancellation(endpoint, runtimeID, capability string) *SDKError {
	return &SDKError{
		Code:      ErrNotImplemented,
		Stage:     "direct_runtime",
		Retry:     RetryNever,
		Retryable: false,
		Message:   "direct gRPC cancellation is unsupported because the transport cannot submit canonical lifecycle control and deliver its terminal",
		Details: map[string]any{
			"endpoint":   endpoint,
			"runtime_id": runtimeID,
			"capability": capability,
		},
	}
}

func durationFromMillis(value int64, defaultValue time.Duration) time.Duration {
	if value <= 0 {
		return defaultValue
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
		return "passthrough:///runtime-invocation", []grpc.DialOption{
			grpc.WithTransportCredentials(insecure.NewCredentials()),
			grpc.WithContextDialer(directRuntimeUDSDialer(socketPath)),
		}, nil
	}
	if !strings.Contains(endpoint, "://") {
		return endpoint, []grpc.DialOption{grpc.WithTransportCredentials(insecure.NewCredentials())}, nil
	}
	parsed, err := url.Parse(endpoint)
	if err != nil {
		return "", nil, invalidRuntimePayload(fmt.Sprintf("parse runtime invocation endpoint: %v", err), err)
	}
	switch parsed.Scheme {
	case "http", "https":
		return "", nil, directRuntimeError(
			"http(s) endpoints are public transport endpoints, not direct runtime Invocation endpoints; use unix://, grpc://, grpcs://, axon://, or host:port",
			ErrProtocolMismatch,
			RetryNever,
			map[string]any{"endpoint": endpoint, "scheme": parsed.Scheme},
			nil,
		)
	case "grpc":
		target := parsed.Host
		if target == "" {
			return "", nil, invalidRuntimePayload("grpc runtime invocation endpoint requires host", nil)
		}
		return target, []grpc.DialOption{grpc.WithTransportCredentials(insecure.NewCredentials())}, nil
	case "grpcs", "axon":
		target := parsed.Host
		if target == "" {
			return "", nil, invalidRuntimePayload(parsed.Scheme+" runtime invocation endpoint requires host", nil)
		}
		// Direct runtime discovery currently does not carry a CA bundle. The
		// endpoint is read from the local daemon control plane; use TLS for the
		// transport class and leave endpoint authenticity to daemon discovery.
		tlsConfig := &tls.Config{ServerName: parsed.Hostname(), InsecureSkipVerify: true}
		return target, []grpc.DialOption{grpc.WithTransportCredentials(credentials.NewTLS(tlsConfig))}, nil
	default:
		return "", nil, invalidRuntimePayload(fmt.Sprintf("unsupported runtime invocation endpoint scheme %q", parsed.Scheme), nil)
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
