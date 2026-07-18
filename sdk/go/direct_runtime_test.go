//go:build easynet_direct_runtime

package easynet

import (
	"bytes"
	"context"
	"crypto/sha256"
	"encoding/base64"
	"encoding/json"
	"errors"
	"net"
	"os"
	"path/filepath"
	"sync"
	"testing"
	"time"

	axoninv "axon.run/sdk/go/axon/invocation"
	"easynet.run/cli/sdk/go/internal/axonpb"
	"google.golang.org/grpc"
	"google.golang.org/grpc/codes"
	"google.golang.org/grpc/status"
)

type directRuntimeFakeDaemon struct {
	axonpb.UnimplementedInvocationServer
	t *testing.T

	timingMu sync.RWMutex

	seenInvoke *axonpb.InvokeRequest
	seenStream *axonpb.InvokeServerStreamRequest
	seenBidi   []*axonpb.InvokeBidiUp

	invokeDelay   time.Duration
	invokeStarted chan struct{}
	streamDelay   time.Duration
	streamStarted chan struct{}
	bidiDelay     time.Duration
	bidiStarted   chan struct{}
}

func (d *directRuntimeFakeDaemon) Invoke(ctx context.Context, req *axonpb.InvokeRequest) (*axonpb.InvokeResponse, error) {
	d.seenInvoke = req
	delay, started := d.invokeTiming()
	if started != nil {
		close(started)
	}
	if delay > 0 {
		select {
		case <-time.After(delay):
		case <-ctx.Done():
			return nil, status.Error(codes.DeadlineExceeded, "deadline elapsed")
		}
	}
	admissionReceipt, terminalReceipt := canonicalDirectRuntimeReceiptPair("inv-1")
	return &axonpb.InvokeResponse{
		State:             axonpb.InvocationState_INVOCATION_STATE_COMPLETED,
		Result:            []byte(`{"ok":true}`),
		ResultContentType: "application/json",
		ElapsedMs:         7,
		AdmissionReceipt:  admissionReceipt,
		TerminalReceipt:   terminalReceipt,
	}, nil
}

func canonicalDirectRuntimeReceiptPair(invocationID string) (*axonpb.InvocationReceipt, *axonpb.InvocationReceipt) {
	const (
		callerURA = "easynet:///r/example/agent/alice.sdk"
		calleeURA = "easynet:///r/example/device/dev-a"
		profile   = "axon-strict-v2"
	)
	proofPayload := []byte("canonical-direct-runtime-test-proof")
	proofHash := sha256.Sum256(proofPayload)
	binding := func() *axonpb.AuthorityBinding {
		return &axonpb.AuthorityBinding{
			Authority: &axonpb.AuthorityBinding_SelfAuthority{
				SelfAuthority: &axonpb.SelfAuthority{PrincipalUra: callerURA},
			},
		}
	}
	receipt := func(
		index uint64,
		receiptType string,
		state axonpb.InvocationState,
		previousHash []byte,
		selfHashByte byte,
		cleanupComplete bool,
	) *axonpb.InvocationReceipt {
		return &axonpb.InvocationReceipt{
			Index:             index,
			InvocationId:      invocationID,
			ReceiptType:       receiptType,
			State:             state,
			TimestampUnixMs:   1_783_100_000_000 + int64(index),
			PrevReceiptHash:   bytes.Repeat(previousHash, 32),
			SelfHash:          bytes.Repeat([]byte{selfHashByte}, 32),
			CleanupComplete:   cleanupComplete,
			CallerBinding:     &axonpb.AgentIdentity{Ura: callerURA, Profile: profile},
			CalleeBinding:     &axonpb.AgentIdentity{Ura: calleeURA, Profile: profile},
			SubjectBinding:    &axonpb.SubjectIdentity{Ura: calleeURA, Profile: profile},
			InvocationNonce:   []byte{1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16},
			CausalBinding:     &axonpb.CausalContext{Form: &axonpb.CausalContext_None{None: &axonpb.Empty{}}},
			CalleeSignature:   &axonpb.CalleeSignature{Algorithm: "ed25519", Signature: bytes.Repeat([]byte{0x71}, 64)},
			AuthorityBinding:  binding(),
			AbilityBinding:    runtimeTestDescriptorRef,
			Usage:             &axonpb.InvocationUsage{},
			SubjectRef:        &axonpb.EntityRef{Kind: axonpb.EntityRefKind_ENTITY_REF_KIND_DEVICE, Ura: calleeURA, Profile: profile},
			DescriptorVersion: "1.0.0",
			SchemaHash:        bytes.Repeat([]byte{0x11}, 32),
			ImplHash:          bytes.Repeat([]byte{0x22}, 32),
			RuntimeEnv:        "go-direct-runtime-test",
			AuthorityProof: &axonpb.InvocationAuthorityProof{
				ProofType:     "self",
				Binding:       binding(),
				ProofPayload:  append([]byte(nil), proofPayload...),
				ProofHash:     append([]byte(nil), proofHash[:]...),
				Issuer:        &axonpb.AgentIdentity{Ura: calleeURA, Profile: profile},
				Signature:     &axonpb.CalleeSignature{Algorithm: "ed25519", Signature: bytes.Repeat([]byte{0x72}, 64)},
				AdmissionHook: "test.direct_runtime.admission",
			},
			InputHash:  bytes.Repeat([]byte{0x33}, 32),
			OutputHash: bytes.Repeat([]byte{0x44}, 32),
		}
	}
	admission := receipt(
		1,
		"admitted",
		axonpb.InvocationState_INVOCATION_STATE_ADMITTED,
		[]byte{0},
		0x51,
		false,
	)
	terminal := receipt(
		7,
		"completed",
		axonpb.InvocationState_INVOCATION_STATE_COMPLETED,
		[]byte{0x61},
		0x71,
		true,
	)
	return admission, terminal
}

func (d *directRuntimeFakeDaemon) configureInvokeTiming(delay time.Duration) <-chan struct{} {
	d.timingMu.Lock()
	defer d.timingMu.Unlock()
	d.invokeDelay = delay
	if delay <= 0 {
		d.invokeStarted = nil
		return nil
	}
	d.invokeStarted = make(chan struct{})
	return d.invokeStarted
}

func (d *directRuntimeFakeDaemon) invokeTiming() (time.Duration, chan struct{}) {
	d.timingMu.RLock()
	defer d.timingMu.RUnlock()
	return d.invokeDelay, d.invokeStarted
}

func TestDirectRuntimeInvokeDeadlineIsTypedTimeout(t *testing.T) {
	transport, daemon, cleanup := openDirectRuntimeTestTransportWithOptions(t, DirectRuntimeOptions{
		DialTimeoutMS:   3000,
		InvokeTimeoutMS: 50,
	})
	defer cleanup()
	invokeStarted := daemon.configureInvokeTiming(time.Second)

	client, err := NewRuntimeClient(transport)
	if err != nil {
		t.Fatalf("NewRuntimeClient: %v", err)
	}

	done := make(chan error, 1)
	go func() {
		_, err := client.Invoke(context.Background(), directRuntimeDraft(t))
		done <- err
	}()

	select {
	case <-invokeStarted:
	case <-time.After(time.Second):
		t.Fatalf("deadline test did not dispatch the runtime invocation")
	}

	err = <-done
	if !IsCode(err, ErrTimeout) {
		t.Fatalf("Invoke deadline = %v, want %s", err, ErrTimeout)
	}
	sdkErr := new(SDKError)
	if !errors.As(err, &sdkErr) {
		t.Fatalf("deadline error is not SDKError: %T", err)
	}
	if sdkErr.Stage != "direct_runtime" || sdkErr.Retry != RetrySafe || !sdkErr.Retryable {
		t.Fatalf("deadline classification = stage %q retry %s retryable %v", sdkErr.Stage, sdkErr.Retry, sdkErr.Retryable)
	}
	if sdkErr.Details["grpc_status"] != codes.DeadlineExceeded.String() {
		t.Fatalf("grpc status = %#v", sdkErr.Details)
	}
	if daemon.seenInvoke == nil {
		t.Fatalf("deadline test did not dispatch the runtime invocation")
	}
}

func (d *directRuntimeFakeDaemon) InvokeStream(req *axonpb.InvokeServerStreamRequest, stream grpc.ServerStreamingServer[axonpb.InvokeStreamChunk]) error {
	d.seenStream = req
	delay, started := d.streamTiming()
	if started != nil {
		close(started)
	}
	if delay > 0 {
		select {
		case <-time.After(delay):
		case <-stream.Context().Done():
			return status.Error(codes.DeadlineExceeded, "deadline elapsed")
		}
	}
	if err := stream.Send(&axonpb.InvokeStreamChunk{
		Sequence:    0,
		State:       axonpb.InvocationState_INVOCATION_STATE_RUNNING,
		Payload:     []byte(`{"delta":1}`),
		ContentType: "application/json",
		AdmissionReceipt: &axonpb.InvocationReceipt{
			Index:        0,
			InvocationId: "inv-stream",
			ReceiptType:  "admitted",
			State:        axonpb.InvocationState_INVOCATION_STATE_ADMITTED,
		},
	}); err != nil {
		return err
	}
	return stream.Send(&axonpb.InvokeStreamChunk{
		Sequence:     1,
		InvocationId: "inv-stream",
		State:        axonpb.InvocationState_INVOCATION_STATE_COMPLETED,
		Terminal:     true,
		Payload:      []byte(`{"done":true}`),
		ContentType:  "application/json",
		TerminalReceipt: &axonpb.InvocationReceipt{
			Index:           1,
			InvocationId:    "inv-stream",
			ReceiptType:     "completed",
			State:           axonpb.InvocationState_INVOCATION_STATE_COMPLETED,
			CleanupComplete: true,
		},
	})
}

func (d *directRuntimeFakeDaemon) configureStreamTiming(delay time.Duration) <-chan struct{} {
	d.timingMu.Lock()
	defer d.timingMu.Unlock()
	d.streamDelay = delay
	if delay <= 0 {
		d.streamStarted = nil
		return nil
	}
	d.streamStarted = make(chan struct{})
	return d.streamStarted
}

func (d *directRuntimeFakeDaemon) streamTiming() (time.Duration, chan struct{}) {
	d.timingMu.RLock()
	defer d.timingMu.RUnlock()
	return d.streamDelay, d.streamStarted
}

func (d *directRuntimeFakeDaemon) InvokeBidi(stream grpc.BidiStreamingServer[axonpb.InvokeBidiUp, axonpb.InvokeBidiDown]) error {
	open, err := stream.Recv()
	if err != nil {
		return err
	}
	d.seenBidi = append(d.seenBidi, open)
	delay, started := d.bidiTiming()
	if started != nil {
		close(started)
	}
	if delay > 0 {
		select {
		case <-time.After(delay):
		case <-stream.Context().Done():
			return status.Error(codes.DeadlineExceeded, "deadline elapsed")
		}
	}
	if err := stream.Send(&axonpb.InvokeBidiDown{
		Sequence: 0,
		Payload: &axonpb.InvokeBidiDown_Receipt{Receipt: &axonpb.InvocationReceipt{
			Index:        0,
			InvocationId: "inv-bidi",
			ReceiptType:  "admitted",
			State:        axonpb.InvocationState_INVOCATION_STATE_ADMITTED,
		}},
	}); err != nil {
		return err
	}
	frame, err := stream.Recv()
	if err != nil {
		return err
	}
	d.seenBidi = append(d.seenBidi, frame)
	if err := stream.Send(&axonpb.InvokeBidiDown{
		Sequence: 1,
		Payload: &axonpb.InvokeBidiDown_BinaryChunk{BinaryChunk: &axonpb.BinaryChunk{
			StreamId: 1,
			Data:     []byte("pong"),
		}},
	}); err != nil {
		return err
	}
	return stream.Send(&axonpb.InvokeBidiDown{
		Sequence: 2,
		Payload: &axonpb.InvokeBidiDown_Receipt{Receipt: &axonpb.InvocationReceipt{
			Index:              1,
			InvocationId:       "inv-bidi",
			ReceiptType:        "completed",
			State:              axonpb.InvocationState_INVOCATION_STATE_COMPLETED,
			Payload:            []byte(`{"sha256":"bidi-sha"}`),
			PayloadContentType: "application/json",
			CleanupComplete:    true,
		}},
	})
}

func (d *directRuntimeFakeDaemon) configureBidiTiming(delay time.Duration) <-chan struct{} {
	d.timingMu.Lock()
	defer d.timingMu.Unlock()
	d.bidiDelay = delay
	if delay <= 0 {
		d.bidiStarted = nil
		return nil
	}
	d.bidiStarted = make(chan struct{})
	return d.bidiStarted
}

func (d *directRuntimeFakeDaemon) bidiTiming() (time.Duration, chan struct{}) {
	d.timingMu.RLock()
	defer d.timingMu.RUnlock()
	return d.bidiDelay, d.bidiStarted
}

func TestDirectRuntimeTransportInvokesOverUnixSocket(t *testing.T) {
	transport, daemon, cleanup := openDirectRuntimeTestTransport(t)
	defer cleanup()

	client, err := NewRuntimeClient(transport)
	if err != nil {
		t.Fatalf("NewRuntimeClient: %v", err)
	}
	result, err := client.Invoke(context.Background(), directRuntimeDraft(t))
	if err != nil {
		t.Fatalf("Invoke: %v", err)
	}
	if !result.OK() || result.TerminalState() != "Completed" {
		t.Fatalf("result = ok %v state %s", result.OK(), result.TerminalState())
	}
	if got := string(result.OutputJSON()); got != `{"ok":true}` {
		t.Fatalf("OutputJSON = %s", got)
	}
	if daemon.seenInvoke == nil || daemon.seenInvoke.GetEnvelope().GetCaller().GetUra() != "easynet:///r/example/agent/alice" {
		t.Fatalf("daemon did not receive caller envelope: %#v", daemon.seenInvoke)
	}
	if daemon.seenInvoke.GetTarget().GetAbility().GetFunctionName() != "er.weather" {
		t.Fatalf("function name = %q, want er.weather", daemon.seenInvoke.GetTarget().GetAbility().GetFunctionName())
	}
	if got := daemon.seenInvoke.GetTarget().GetAbility().GetAbilityName(); got != directRuntimeDraft(t).DescriptorRef() {
		t.Fatalf("descriptor-bound target = %q", got)
	}
	if got := string(daemon.seenInvoke.GetArguments()); got != `{"city":"Singapore"}` {
		t.Fatalf("arguments = %s", got)
	}
}

func TestDirectRuntimeTransportStringifiesTypedMetadata(t *testing.T) {
	transport, daemon, cleanup := openDirectRuntimeTestTransport(t)
	defer cleanup()

	client, err := NewRuntimeClient(transport)
	if err != nil {
		t.Fatalf("NewRuntimeClient: %v", err)
	}
	_, err = client.Invoke(context.Background(), directRuntimeDraftWithMetadata(t, map[string]any{
		"trace_id":    "direct-test",
		"timeout_ms":  int64(1500),
		"system":      true,
		"authority":   map[string]any{"subject_ura": "easynet:///r/example/device/dev-a"},
		"empty_value": nil,
	}))
	if err != nil {
		t.Fatalf("Invoke: %v", err)
	}
	metadata := daemon.seenInvoke.GetMetadata()
	if metadata["trace_id"] != "direct-test" {
		t.Fatalf("trace_id metadata = %q", metadata["trace_id"])
	}
	if metadata["timeout_ms"] != "1500" {
		t.Fatalf("timeout_ms metadata = %q", metadata["timeout_ms"])
	}
	if metadata["system"] != "true" {
		t.Fatalf("system metadata = %q", metadata["system"])
	}
	if metadata["authority"] != `{"subject_ura":"easynet:///r/example/device/dev-a"}` {
		t.Fatalf("authority metadata = %q", metadata["authority"])
	}
	if _, ok := metadata["empty_value"]; ok {
		t.Fatalf("nil metadata value should be omitted: %#v", metadata)
	}
}

func TestDirectRuntimeDialTargetClassifiesEndpointTransport(t *testing.T) {
	cases := []struct {
		name     string
		endpoint string
		target   string
	}{
		{name: "uds path", endpoint: "/tmp/runtime.sock", target: "passthrough:///runtime-invocation"},
		{name: "uds scheme", endpoint: "unix:///tmp/runtime.sock", target: "passthrough:///runtime-invocation"},
		{name: "bare tcp", endpoint: "127.0.0.1:50051", target: "127.0.0.1:50051"},
		{name: "plain grpc tcp", endpoint: "grpc://127.0.0.1:50051", target: "127.0.0.1:50051"},
		{name: "tls grpc tcp", endpoint: "grpcs://hub:50443", target: "hub:50443"},
		{name: "axon grpc tcp", endpoint: "axon://hub:50443", target: "hub:50443"},
	}
	for _, tc := range cases {
		t.Run(tc.name, func(t *testing.T) {
			target, options, err := directRuntimeDialTarget(tc.endpoint)
			if err != nil {
				t.Fatalf("directRuntimeDialTarget: %v", err)
			}
			if target != tc.target {
				t.Fatalf("target = %q, want %q", target, tc.target)
			}
			if len(options) == 0 {
				t.Fatal("dial options must include transport credentials")
			}
		})
	}
}

func TestDirectRuntimeDialTargetRejectsPublicHTTPEndpoints(t *testing.T) {
	for _, endpoint := range []string{
		"http://127.0.0.1:50051",
		"https://hub.example:50443",
	} {
		_, _, err := directRuntimeDialTarget(endpoint)
		if !IsCode(err, ErrProtocolMismatch) {
			t.Fatalf("directRuntimeDialTarget(%q) error = %#v, want PROTOCOL_MISMATCH", endpoint, err)
		}
	}
}

func TestDirectRuntimeGRPCErrorClassifiesHTTP2ProtocolReset(t *testing.T) {
	err := directRuntimeGRPCError(
		status.Error(codes.Internal, "stream terminated by RST_STREAM with error code: PROTOCOL_ERROR"),
		"https://hub.example:50443",
	)
	if !IsCode(err, ErrProtocolMismatch) {
		t.Fatalf("directRuntimeGRPCError = %#v, want PROTOCOL_MISMATCH", err)
	}
}

func TestDirectRuntimeTransportUsesAxonCanonicalPublicRoute(t *testing.T) {
	transport, daemon, cleanup := openDirectRuntimeTestTransportWithOptions(t, DirectRuntimeOptions{
		DialTimeoutMS: 3000,
	})
	defer cleanup()

	client, err := NewRuntimeClient(transport)
	if err != nil {
		t.Fatalf("NewRuntimeClient: %v", err)
	}
	if _, err := client.Invoke(context.Background(), directRuntimeDraft(t)); err != nil {
		t.Fatalf("Invoke: %v", err)
	}
	if daemon.seenInvoke == nil || daemon.seenInvoke.GetTarget().GetAbility().GetFunctionName() != "er.weather" {
		t.Fatalf("daemon function name = %#v", daemon.seenInvoke)
	}
}

func TestDirectRuntimeTransportRejectsUnprojectedUserSubject(t *testing.T) {
	handle := &directRuntimeFakeHandleTransport{}
	transport, _, cleanup := openDirectRuntimeTestTransportWithOptions(t, DirectRuntimeOptions{
		DialTimeoutMS:   3000,
		HandleTransport: handle,
	})
	defer cleanup()
	draft := directRuntimeUserSubjectDraft(t)

	_, err := transport.Prepare(
		context.Background(),
		mustMarshalDirectRuntimeDraft(t, draft),
		[]byte(`{"expires_in_ms":60000}`),
	)
	if !IsCode(err, ErrInvalidArgument) {
		t.Fatalf("Prepare unprojected subject error = %v, want %s", err, ErrInvalidArgument)
	}
	if handle.prepareCalls != 0 || len(handle.preparedDraft) != 0 {
		t.Fatalf("invalid tuple reached handle transport: calls=%d draft=%s", handle.prepareCalls, handle.preparedDraft)
	}
}

func TestDirectRuntimeTransportSubmitSignedDelegatesWithoutDirectDispatch(t *testing.T) {
	handle := &directRuntimeFakeHandleTransport{}
	transport, daemon, cleanup := openDirectRuntimeTestTransportWithOptions(t, DirectRuntimeOptions{
		DialTimeoutMS:   3000,
		HandleTransport: handle,
	})
	defer cleanup()
	signedJSON := []byte(`{"signed":true}`)

	if _, err := transport.SubmitSigned(context.Background(), signedJSON); err != nil {
		t.Fatalf("SubmitSigned: %v", err)
	}
	if handle.submitCalls != 1 || string(handle.signedInvocation) != string(signedJSON) {
		t.Fatalf("signed submit delegation = calls %d payload %q", handle.submitCalls, handle.signedInvocation)
	}
	if daemon.seenInvoke != nil {
		t.Fatal("signed submission must not bypass the handle owner through direct gRPC")
	}
}

func TestDirectRuntimeTransportRejectsDescriptorOwnedByDifferentCallee(t *testing.T) {
	var fields map[string]any
	if err := json.Unmarshal(mustMarshalDirectRuntimeDraft(t, directRuntimeDraft(t)), &fields); err != nil {
		t.Fatalf("decode draft: %v", err)
	}
	fields["callee_ura"] = "easynet:///r/example/device/dev-b"
	fields["subject_ura"] = "easynet:///r/example/device/dev-b"
	raw, err := json.Marshal(fields)
	if err != nil {
		t.Fatalf("encode mismatched draft: %v", err)
	}
	draft, err := NewInvocationDraftFromJSON(raw)
	if err != nil {
		t.Fatalf("NewInvocationDraftFromJSON: %v", err)
	}
	codec, err := newDirectDescriptorBoundCodec(time.Second)
	if err != nil {
		t.Fatalf("newDirectDescriptorBoundCodec: %v", err)
	}
	if _, err := codec.build(context.Background(), draft, axoninv.CallModeRPC); !IsCode(err, ErrInvalidArgument) {
		t.Fatalf("descriptor owner mismatch error = %v, want %s", err, ErrInvalidArgument)
	}
}

func TestDirectRuntimeTransportStreamsOverUnixSocket(t *testing.T) {
	transport, daemon, cleanup := openDirectRuntimeTestTransport(t)
	defer cleanup()

	client, err := NewRuntimeClient(transport)
	if err != nil {
		t.Fatalf("NewRuntimeClient: %v", err)
	}
	stream, err := client.InvokeStream(context.Background(), directRuntimeDraft(t))
	if err != nil {
		t.Fatalf("InvokeStream: %v", err)
	}
	first, err := stream.Next(context.Background())
	if err != nil {
		t.Fatalf("Next first: %v", err)
	}
	if first.Sequence() != 1 || first.Kind() != "data" || first.Terminal() {
		t.Fatalf("first = seq %d kind %s terminal %v", first.Sequence(), first.Kind(), first.Terminal())
	}
	if first.State() != "Running" {
		t.Fatalf("first lifecycle state = %q", first.State())
	}
	terminal, err := stream.Next(context.Background())
	if err != nil {
		t.Fatalf("Next terminal: %v", err)
	}
	if !terminal.Terminal() || stream.State() != StreamTerminalFrameSeen {
		t.Fatalf("terminal = %v state %s", terminal.Terminal(), stream.State())
	}
	if len(terminal.AdmissionReceiptJSON()) == 0 || len(terminal.TerminalReceiptJSON()) == 0 {
		t.Fatalf(
			"terminal stream checkpoints = admission:%s terminal:%s",
			terminal.AdmissionReceiptJSON(),
			terminal.TerminalReceiptJSON(),
		)
	}
	if daemon.seenStream == nil || daemon.seenStream.GetTarget().GetAbility().GetFunctionName() != "er.weather" {
		t.Fatalf("daemon did not receive stream request")
	}
	if err := stream.Close(context.Background()); err != nil {
		t.Fatalf("Close stream: %v", err)
	}
}

func TestDirectRuntimeStreamDeadlineIsTypedTimeout(t *testing.T) {
	transport, daemon, cleanup := openDirectRuntimeTestTransportWithOptions(t, DirectRuntimeOptions{
		DialTimeoutMS:   3000,
		InvokeTimeoutMS: 50,
	})
	defer cleanup()
	streamStarted := daemon.configureStreamTiming(time.Second)

	client, err := NewRuntimeClient(transport)
	if err != nil {
		t.Fatalf("NewRuntimeClient: %v", err)
	}
	stream, err := client.InvokeStream(context.Background(), directRuntimeDraft(t))
	if err != nil {
		t.Fatalf("InvokeStream open before deadline observation: %v", err)
	}
	select {
	case <-streamStarted:
	case <-time.After(time.Second):
		t.Fatalf("deadline test did not dispatch the runtime stream")
	}
	if _, err := stream.Next(context.Background()); !IsCode(err, ErrTimeout) {
		t.Fatalf("stream deadline = %v, want %s", err, ErrTimeout)
	}
	_ = stream.Close(context.Background())
	if daemon.seenStream == nil {
		t.Fatalf("deadline test did not dispatch the runtime stream")
	}

	daemon.configureStreamTiming(0)
	retry, err := client.InvokeStream(context.Background(), directRuntimeDraft(t))
	if err != nil {
		t.Fatalf("retry stream after deadline cleanup: %v", err)
	}
	first, err := retry.Next(context.Background())
	if err != nil {
		t.Fatalf("retry stream first event: %v", err)
	}
	if first.Terminal() {
		t.Fatalf("retry stream returned terminal first event: %#v", first)
	}
	if err := retry.Close(context.Background()); err != nil {
		t.Fatalf("retry stream close: %v", err)
	}
}

func TestDirectRuntimeStreamCancelIsExplicitlyUnsupported(t *testing.T) {
	transport, _, cleanup := openDirectRuntimeTestTransport(t)
	defer cleanup()

	streamTransport, _, err := transport.OpenStream(context.Background(), mustMarshalDirectRuntimeDraft(t, directRuntimeDraft(t)))
	if err != nil {
		t.Fatalf("OpenStream: %v", err)
	}
	if _, err := streamTransport.Cancel(context.Background(), "client stop"); !IsCode(err, ErrNotImplemented) {
		t.Fatalf("Cancel stream error = %v, want %s", err, ErrNotImplemented)
	}
}

func TestDirectRuntimeTransportBidiOverUnixSocket(t *testing.T) {
	transport, daemon, cleanup := openDirectRuntimeTestTransport(t)
	defer cleanup()

	client, err := NewRuntimeClient(transport)
	if err != nil {
		t.Fatalf("NewRuntimeClient: %v", err)
	}
	session, err := client.OpenBidi(context.Background(), directRuntimeDraft(t), []BidiStreamDescriptor{{StreamID: 1, ContentType: "text/plain"}})
	if err != nil {
		t.Fatalf("OpenBidi: %v", err)
	}
	frame, err := NewBidiBinaryFrame(1, 1, []byte("ping"), "text/plain")
	if err != nil {
		t.Fatalf("NewBidiBinaryFrame: %v", err)
	}
	if _, err := session.Send(context.Background(), frame); err != nil {
		t.Fatalf("Send: %v", err)
	}
	received, err := session.Receive(context.Background())
	if err != nil {
		t.Fatalf("Receive data: %v", err)
	}
	if len(received.AdmissionReceiptJSON()) != 0 {
		t.Fatalf("data frame carried admission receipt: %s", received.AdmissionReceiptJSON())
	}
	if received.Kind() != "data" || received.StreamID() != 1 {
		t.Fatalf("received = kind %s stream %d", received.Kind(), received.StreamID())
	}
	terminal, err := session.Receive(context.Background())
	if err != nil {
		t.Fatalf("Receive terminal: %v", err)
	}
	if !terminal.Terminal() || session.State() != BidiTerminal {
		t.Fatalf("terminal = %v state %s", terminal.Terminal(), session.State())
	}
	if len(terminal.AdmissionReceiptJSON()) == 0 {
		t.Fatal("terminal bidi frame omitted cached admission receipt")
	}
	terminalFrame, err := session.TerminalFrame()
	if err != nil {
		t.Fatalf("TerminalFrame: %v", err)
	}
	var receipt struct {
		Payload map[string]string `json:"payload"`
	}
	if err := json.Unmarshal(terminalFrame.TerminalReceiptJSON(), &receipt); err != nil {
		t.Fatalf("decode terminal receipt: %v; raw=%s", err, terminalFrame.TerminalReceiptJSON())
	}
	if receipt.Payload["sha256"] != "bidi-sha" {
		t.Fatalf("terminal receipt payload = %#v", receipt.Payload)
	}
	if len(daemon.seenBidi) < 2 || daemon.seenBidi[0].GetEnvelopeOpen() == nil {
		t.Fatalf("daemon did not receive bidi open and data frames")
	}
	if got := daemon.seenBidi[0].GetEnvelopeOpen().GetTarget().GetAbility().GetAbilityName(); got != directRuntimeDraft(t).DescriptorRef() {
		t.Fatalf("descriptor-bound target = %q", got)
	}
	if err := session.Close(context.Background()); err != nil {
		t.Fatalf("Close bidi: %v", err)
	}
}

func TestDirectRuntimeBidiDeadlineIsTypedTimeout(t *testing.T) {
	transport, daemon, cleanup := openDirectRuntimeTestTransportWithOptions(t, DirectRuntimeOptions{
		DialTimeoutMS:   3000,
		InvokeTimeoutMS: 50,
	})
	defer cleanup()
	bidiStarted := daemon.configureBidiTiming(time.Second)

	client, err := NewRuntimeClient(transport)
	if err != nil {
		t.Fatalf("NewRuntimeClient: %v", err)
	}
	session, err := client.OpenBidi(context.Background(), directRuntimeDraft(t), []BidiStreamDescriptor{{StreamID: 1, ContentType: "text/plain"}})
	if err != nil {
		t.Fatalf("OpenBidi before deadline observation: %v", err)
	}
	select {
	case <-bidiStarted:
	case <-time.After(time.Second):
		t.Fatalf("deadline test did not dispatch the runtime bidi open")
	}
	if _, err := session.Receive(context.Background()); !IsCode(err, ErrTimeout) {
		t.Fatalf("bidi deadline = %v, want %s", err, ErrTimeout)
	}
	_ = session.Close(context.Background())
	if len(daemon.seenBidi) == 0 || daemon.seenBidi[0].GetEnvelopeOpen() == nil {
		t.Fatalf("deadline test did not dispatch the runtime bidi open")
	}

	daemon.configureBidiTiming(0)
	retry, err := client.OpenBidi(context.Background(), directRuntimeDraft(t), []BidiStreamDescriptor{{StreamID: 1, ContentType: "text/plain"}})
	if err != nil {
		t.Fatalf("retry bidi after deadline cleanup: %v", err)
	}
	frame, err := NewBidiBinaryFrame(1, 1, []byte("ping"), "text/plain")
	if err != nil {
		t.Fatalf("NewBidiBinaryFrame: %v", err)
	}
	if _, err := retry.Send(context.Background(), frame); err != nil {
		t.Fatalf("retry bidi send: %v", err)
	}
	if _, err := retry.Receive(context.Background()); err != nil {
		t.Fatalf("retry bidi receive: %v", err)
	}
	if terminal, err := retry.Receive(context.Background()); err != nil {
		t.Fatalf("retry bidi terminal: %v", err)
	} else if !terminal.Terminal() {
		t.Fatalf("retry bidi terminal flag = false: %#v", terminal)
	}
	if err := retry.Close(context.Background()); err != nil {
		t.Fatalf("retry bidi close: %v", err)
	}
}

func TestDirectRuntimeBidiCancelIsExplicitlyUnsupported(t *testing.T) {
	transport, _, cleanup := openDirectRuntimeTestTransport(t)
	defer cleanup()

	streams, err := json.Marshal([]BidiStreamDescriptor{{StreamID: 1, ContentType: "text/plain"}})
	if err != nil {
		t.Fatalf("Marshal streams: %v", err)
	}
	bidiTransport, _, err := transport.OpenBidi(context.Background(), mustMarshalDirectRuntimeDraft(t, directRuntimeDraft(t)), streams)
	if err != nil {
		t.Fatalf("OpenBidi: %v", err)
	}
	if _, err := bidiTransport.Cancel(context.Background(), "client stop"); !IsCode(err, ErrNotImplemented) {
		t.Fatalf("Cancel bidi error = %v, want %s", err, ErrNotImplemented)
	}
}

func TestDirectRuntimeBidiRejectsMissingFrame0BeforeSessionEntry(t *testing.T) {
	cancelCalled := false
	cancel := func() { cancelCalled = true }
	if _, err := newDirectRuntimeBidiTransport(nil, cancel, "unix:///direct-test", nil); !IsCode(err, ErrInvalidArgument) {
		t.Fatalf("missing frame0 error = %v, want %s", err, ErrInvalidArgument)
	}
	if !cancelCalled {
		t.Fatalf("missing frame0 did not cancel pending bidi context")
	}

	cancelCalled = false
	if _, err := newDirectRuntimeBidiTransport(
		nil,
		cancel,
		"unix:///direct-test",
		&axonpb.InvokeBidiUp{Sequence: 1, Payload: &axonpb.InvokeBidiUp_Control{Control: &axonpb.BidiControl{Control: &axonpb.BidiControl_Eof{Eof: true}}}},
	); !IsCode(err, ErrInvalidArgument) {
		t.Fatalf("non-frame0 error = %v, want %s", err, ErrInvalidArgument)
	}
	if !cancelCalled {
		t.Fatalf("non-frame0 did not cancel pending bidi context")
	}
}

func TestDirectRuntimeTransportDelegatesHandleOperations(t *testing.T) {
	handle := &directRuntimeFakeHandleTransport{}
	transport, _, cleanup := openDirectRuntimeTestTransportWithOptions(t, DirectRuntimeOptions{
		DialTimeoutMS:        3000,
		HandleTransport:      handle,
		CloseHandleTransport: true,
	})
	defer cleanup()

	prepared, err := transport.Prepare(
		context.Background(),
		mustMarshalDirectRuntimeDraft(t, directRuntimeDraft(t)),
		[]byte(`{"expires_in_ms":1000}`),
	)
	if err != nil {
		t.Fatalf("Prepare: %v", err)
	}
	if string(prepared) != preparedFixture || handle.prepareCalls != 1 {
		t.Fatalf("prepare delegation = calls %d payload %s", handle.prepareCalls, prepared)
	}
	submitted, err := transport.SubmitSigned(context.Background(), []byte(`{"signed":true}`))
	if err != nil {
		t.Fatalf("SubmitSigned: %v", err)
	}
	if string(submitted) != `{"handle_id":7,"state":"Submitted","terminal":false,"events":[],"result":null}` ||
		handle.submitCalls != 1 {
		t.Fatalf("submit delegation = calls %d payload %s", handle.submitCalls, submitted)
	}
	control, err := newRuntimeInvocationControlCapability(7)
	if err != nil {
		t.Fatalf("control capability: %v", err)
	}
	if _, err := transport.AwaitHandle(context.Background(), control); err != nil {
		t.Fatalf("AwaitHandle: %v", err)
	}
	if _, err := transport.CancelHandle(context.Background(), control, "client stop"); err != nil {
		t.Fatalf("CancelHandle: %v", err)
	}
	if _, err := transport.HandleEvents(context.Background(), control); err != nil {
		t.Fatalf("HandleEvents: %v", err)
	}
	if err := transport.FreeHandle(context.Background(), control); err != nil {
		t.Fatalf("FreeHandle: %v", err)
	}
	if handle.awaitCalls != 1 || handle.cancelCalls != 1 || handle.eventsCalls != 1 || handle.freeCalls != 1 {
		t.Fatalf("handle delegation counts = await %d cancel %d events %d free %d",
			handle.awaitCalls, handle.cancelCalls, handle.eventsCalls, handle.freeCalls)
	}
	if err := transport.Close(context.Background()); err != nil {
		t.Fatalf("Close: %v", err)
	}
	if handle.closeCalls != 1 {
		t.Fatalf("handle close calls = %d, want 1", handle.closeCalls)
	}
}

func TestDirectRuntimeTransportRejectsHandleOperationsWithoutDelegate(t *testing.T) {
	transport, _, cleanup := openDirectRuntimeTestTransportWithOptions(t, DirectRuntimeOptions{DialTimeoutMS: 3000})
	defer cleanup()

	control, err := newRuntimeInvocationControlCapability(1)
	if err != nil {
		t.Fatalf("newRuntimeInvocationControlCapability: %v", err)
	}
	operations := []struct {
		name string
		call func() error
	}{
		{
			name: "prepare",
			call: func() error {
				_, err := transport.Prepare(context.Background(), mustMarshalDirectRuntimeDraft(t, directRuntimeDraft(t)), []byte(`{}`))
				return err
			},
		},
		{
			name: "submit signed",
			call: func() error {
				_, err := transport.SubmitSigned(context.Background(), []byte(`{}`))
				return err
			},
		},
		{
			name: "await handle",
			call: func() error {
				_, err := transport.AwaitHandle(context.Background(), control)
				return err
			},
		},
		{
			name: "cancel handle",
			call: func() error {
				_, err := transport.CancelHandle(context.Background(), control, "client stop")
				return err
			},
		},
		{
			name: "handle events",
			call: func() error {
				_, err := transport.HandleEvents(context.Background(), control)
				return err
			},
		},
		{
			name: "free handle",
			call: func() error {
				return transport.FreeHandle(context.Background(), control)
			},
		},
	}
	for _, operation := range operations {
		if err := operation.call(); !IsCode(err, ErrNotImplemented) {
			t.Fatalf("%s error = %v, want %s", operation.name, err, ErrNotImplemented)
		}
	}
}

func TestDirectRuntimeConnectorProjectsHandleCapabilities(t *testing.T) {
	handle := &directRuntimeFakeHandleTransport{}
	transport, _, cleanup := openDirectRuntimeTestTransportWithOptions(t, DirectRuntimeOptions{DialTimeoutMS: 3000})
	defer cleanup()
	endpoint := transport.endpoint

	connector := NewDirectRuntimeConnectorWithOptions(DirectRuntimeConnectorOptions{
		Reader: ControlDiscoveryReaderFunc(func(context.Context, string) (ControlDiscovery, error) {
			return ControlDiscovery{InvocationEndpoint: endpoint}, nil
		}),
		HandleTransport:      handle,
		CloseHandleTransport: true,
	})
	connection, err := NewRuntimeConnection(connector)
	if err != nil {
		t.Fatalf("NewRuntimeConnection: %v", err)
	}
	if err := connection.Connect(context.Background(), ConnectOptions{DialTimeoutMS: 3000}); err != nil {
		t.Fatalf("Connect: %v", err)
	}
	facts := connection.HandshakeFacts()
	if facts["prepare"] != true || facts["submit_signed"] != true {
		t.Fatalf("handshake facts = %#v", facts)
	}
	if err := connection.Close(context.Background()); err != nil {
		t.Fatalf("Close: %v", err)
	}
	if handle.closeCalls != 1 {
		t.Fatalf("connector-owned handle close calls = %d, want 1", handle.closeCalls)
	}
}

func TestDirectRuntimeConnectorResolvesControlDiscovery(t *testing.T) {
	reader := ControlDiscoveryReaderFunc(func(ctx context.Context, controlPath string) (ControlDiscovery, error) {
		return ControlDiscovery{InvocationEndpoint: "/tmp/direct.sock"}, nil
	})
	connector := NewDirectRuntimeConnector("/tmp/control.json", reader)
	raw, err := connector.Resolve(context.Background(), []byte(`{
		"dial_timeout_ms": 123,
		"invoke_timeout_ms": 456,
		"max_message_bytes": 789
	}`))
	if err != nil {
		t.Fatalf("Resolve: %v", err)
	}
	endpoint, err := NewRuntimeEndpointFromJSON(raw)
	if err != nil {
		t.Fatalf("NewRuntimeEndpointFromJSON: %v", err)
	}
	if endpoint.Endpoint != "/tmp/direct.sock" || endpoint.ProtocolVersion != "axon.v1.Invocation" {
		t.Fatalf("endpoint = %#v", endpoint)
	}
	options, err := decodeConnectOptionsJSON(raw)
	if err != nil {
		t.Fatalf("decodeConnectOptionsJSON: %v", err)
	}
	if options.DialTimeoutMS != 123 || options.InvokeTimeoutMS != 456 || options.MaxMessageBytes != 789 {
		t.Fatalf("options = %#v", options)
	}
}

func openDirectRuntimeTestTransport(t *testing.T) (*DirectRuntimeTransport, *directRuntimeFakeDaemon, func()) {
	return openDirectRuntimeTestTransportWithOptions(t, DirectRuntimeOptions{DialTimeoutMS: 3000})
}

func openDirectRuntimeTestTransportWithOptions(t *testing.T, options DirectRuntimeOptions) (*DirectRuntimeTransport, *directRuntimeFakeDaemon, func()) {
	t.Helper()
	dir, err := os.MkdirTemp("/tmp", "easynet-go-direct-*")
	if err != nil {
		t.Fatalf("mkdir temp: %v", err)
	}
	socket := filepath.Join(dir, "daemon.sock")
	listener, err := net.Listen("unix", socket)
	if err != nil {
		_ = os.RemoveAll(dir)
		t.Fatalf("listen unix: %v", err)
	}
	server := grpc.NewServer()
	daemon := &directRuntimeFakeDaemon{t: t}
	axonpb.RegisterInvocationServer(server, daemon)
	done := make(chan struct{})
	go func() {
		defer close(done)
		_ = server.Serve(listener)
	}()
	if options.DialTimeoutMS == 0 {
		options.DialTimeoutMS = 3000
	}
	transport, err := OpenDirectRuntimeTransport(context.Background(), socket, options)
	if err != nil {
		server.Stop()
		<-done
		_ = os.RemoveAll(dir)
		t.Fatalf("OpenDirectRuntimeTransport: %v", err)
	}
	cleanup := func() {
		_ = transport.Close(context.Background())
		server.Stop()
		<-done
		_ = os.RemoveAll(dir)
	}
	return transport, daemon, cleanup
}

type directRuntimeFakeHandleTransport struct {
	prepareCalls     int
	submitCalls      int
	awaitCalls       int
	cancelCalls      int
	eventsCalls      int
	freeCalls        int
	closeCalls       int
	preparedDraft    []byte
	signedInvocation []byte
}

func (f *directRuntimeFakeHandleTransport) Invoke(context.Context, []byte) ([]byte, error) {
	return nil, errors.New("unexpected Invoke on handle transport")
}

func (f *directRuntimeFakeHandleTransport) OpenStream(context.Context, []byte) (StreamTransport, []byte, error) {
	return nil, nil, errors.New("unexpected OpenStream on handle transport")
}

func (f *directRuntimeFakeHandleTransport) OpenBidi(context.Context, []byte, []byte) (BidiTransport, []byte, error) {
	return nil, nil, errors.New("unexpected OpenBidi on handle transport")
}

func (f *directRuntimeFakeHandleTransport) Prepare(_ context.Context, draftJSON []byte, _ []byte) ([]byte, error) {
	f.prepareCalls++
	f.preparedDraft = append(f.preparedDraft[:0], draftJSON...)
	return []byte(preparedFixture), nil
}

func (f *directRuntimeFakeHandleTransport) SubmitSigned(_ context.Context, signedJSON []byte) ([]byte, error) {
	f.submitCalls++
	f.signedInvocation = append(f.signedInvocation[:0], signedJSON...)
	return []byte(`{"handle_id":7,"state":"Submitted","terminal":false,"events":[],"result":null}`), nil
}

func (f *directRuntimeFakeHandleTransport) AwaitHandle(context.Context, InvocationControlCapability) ([]byte, error) {
	f.awaitCalls++
	return []byte(`{"ok":true,"state":"Completed","terminal_state":"Completed","output_json":{"done":true}}`), nil
}

func (f *directRuntimeFakeHandleTransport) CancelHandle(context.Context, InvocationControlCapability, string) ([]byte, error) {
	f.cancelCalls++
	return []byte(`{"handle_id":7,"request_accepted":true,"deduplicated":false,"cancelled":false,"state":"CancelRequested","terminal":false}`), nil
}

func (f *directRuntimeFakeHandleTransport) HandleEvents(context.Context, InvocationControlCapability) ([]byte, error) {
	f.eventsCalls++
	return []byte(`{"handle_id":7,"state":"Submitted","terminal":false,"events":[],"result":null}`), nil
}

func (f *directRuntimeFakeHandleTransport) FreeHandle(context.Context, InvocationControlCapability) error {
	f.freeCalls++
	return nil
}

func (f *directRuntimeFakeHandleTransport) Close(context.Context) error {
	f.closeCalls++
	return nil
}

func directRuntimeDraft(t *testing.T) InvocationDraft {
	t.Helper()
	return directRuntimeDraftWithMetadata(t, map[string]any{"trace_id": "direct-test"})
}

func directRuntimeDraftWithMetadata(t *testing.T, metadata map[string]any) InvocationDraft {
	t.Helper()
	return directRuntimeDraftWithMetadataAndSignature(t, metadata, true)
}

func directRuntimeUnsignedDraft(t *testing.T) InvocationDraft {
	t.Helper()
	return directRuntimeDraftWithMetadataAndSignature(
		t,
		map[string]any{"trace_id": "direct-unsigned-test"},
		false,
	)
}

func directRuntimeDraftWithMetadataAndSignature(
	t *testing.T,
	metadata map[string]any,
	withSignature bool,
) InvocationDraft {
	t.Helper()
	fields := map[string]any{
		"caller_ura":     "easynet:///r/example/agent/alice",
		"callee_ura":     "easynet:///r/example/device/dev-a",
		"descriptor_ref": "easynet:///r/example/ability/device.dev-a.er.weather@1.0.0#aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa!invoke",
		"subject_ura":    "easynet:///r/example/device/dev-a",
		"nonce_base64":   "AQIDBAUGBwgJCgsMDQ4PEA==",
		"causal_context": map[string]any{"form": "none"},
		"args":           map[string]any{"city": "Singapore"},
		"content_type":   "application/json",
		"metadata":       metadata,
	}
	if withSignature {
		fields["caller_signature"] = map[string]any{
			"algorithm":        "ed25519",
			"signature_base64": base64.StdEncoding.EncodeToString(bytes.Repeat([]byte{0x5a}, 64)),
			"key_id_hint":      "direct-test-key",
		}
	}
	raw, err := json.Marshal(fields)
	if err != nil {
		t.Fatalf("marshal draft: %v", err)
	}
	draft, err := NewInvocationDraftFromJSON(raw)
	if err != nil {
		t.Fatalf("NewInvocationDraftFromJSON: %v", err)
	}
	return draft
}

func directRuntimeUserSubjectDraft(t *testing.T) InvocationDraft {
	t.Helper()
	raw, err := json.Marshal(map[string]any{
		"caller_ura":     "easynet:///r/example/user/alice",
		"callee_ura":     "easynet:///r/example/device/dev-a",
		"descriptor_ref": "easynet:///r/example/ability/device.dev-a.meta.list_resources@1.0.0#aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa!invoke",
		"subject_ura":    "easynet:///r/example/user/alice",
		"nonce_base64":   "AQIDBAUGBwgJCgsMDQ4PEA==",
		"causal_context": map[string]any{"form": "none"},
		"args":           map[string]any{},
		"content_type":   "application/json",
		"metadata":       map[string]any{"trace_id": "direct-user-subject-test"},
	})
	if err != nil {
		t.Fatalf("marshal user subject draft: %v", err)
	}
	draft, err := NewInvocationDraftFromJSON(raw)
	if err != nil {
		t.Fatalf("NewInvocationDraftFromJSON: %v", err)
	}
	return draft
}

func mustMarshalDirectRuntimeDraft(t *testing.T, draft InvocationDraft) []byte {
	t.Helper()
	raw, err := json.Marshal(draft)
	if err != nil {
		t.Fatalf("marshal draft: %v", err)
	}
	return raw
}
