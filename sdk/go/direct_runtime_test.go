//go:build easynet_direct_runtime

package easynet

import (
	"context"
	"encoding/json"
	"errors"
	"net"
	"os"
	"path/filepath"
	"testing"
	"time"

	"easynet.run/cli/sdk/go/internal/axonpb"
	"google.golang.org/grpc"
	"google.golang.org/grpc/codes"
	"google.golang.org/grpc/status"
)

type directRuntimeFakeDaemon struct {
	axonpb.UnimplementedInvocationServer
	t *testing.T

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
	if d.invokeStarted != nil {
		close(d.invokeStarted)
	}
	if d.invokeDelay > 0 {
		select {
		case <-time.After(d.invokeDelay):
		case <-ctx.Done():
			return nil, status.Error(codes.DeadlineExceeded, "deadline elapsed")
		}
	}
	return &axonpb.InvokeResponse{
		State:             axonpb.InvocationState_INVOCATION_STATE_COMPLETED,
		SelectedNodeId:    "node-a",
		SchedulingReason:  "direct-test",
		Result:            []byte(`{"ok":true}`),
		ResultContentType: "application/json",
		ElapsedMs:         7,
		TerminalReceipt: &axonpb.InvocationReceipt{
			Index:           1,
			InvocationId:    "inv-1",
			ReceiptType:     "completed",
			State:           axonpb.InvocationState_INVOCATION_STATE_COMPLETED,
			TimestampUnixMs: 42,
			SelfHash:        []byte{1, 2, 3},
			CleanupComplete: true,
		},
	}, nil
}

func TestDirectRuntimeInvokeDeadlineIsTypedTimeout(t *testing.T) {
	transport, daemon, cleanup := openDirectRuntimeTestTransportWithOptions(t, DirectRuntimeOptions{
		DialTimeoutMS:   3000,
		InvokeTimeoutMS: 50,
	})
	defer cleanup()
	daemon.invokeDelay = time.Second
	daemon.invokeStarted = make(chan struct{})

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
	case <-daemon.invokeStarted:
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
	if d.streamStarted != nil {
		close(d.streamStarted)
	}
	if d.streamDelay > 0 {
		select {
		case <-time.After(d.streamDelay):
		case <-stream.Context().Done():
			return status.Error(codes.DeadlineExceeded, "deadline elapsed")
		}
	}
	if err := stream.Send(&axonpb.InvokeStreamChunk{
		Sequence:         0,
		State:            axonpb.InvocationState_INVOCATION_STATE_RUNNING,
		SelectedNodeId:   "node-stream",
		SchedulingReason: "direct-stream",
		Payload:          []byte(`{"delta":1}`),
		ContentType:      "application/json",
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

func (d *directRuntimeFakeDaemon) InvokeBidi(stream grpc.BidiStreamingServer[axonpb.InvokeBidiUp, axonpb.InvokeBidiDown]) error {
	open, err := stream.Recv()
	if err != nil {
		return err
	}
	d.seenBidi = append(d.seenBidi, open)
	if d.bidiStarted != nil {
		close(d.bidiStarted)
	}
	if d.bidiDelay > 0 {
		select {
		case <-time.After(d.bidiDelay):
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
	if daemon.seenInvoke.GetFunctionName() != "er.weather" {
		t.Fatalf("function name = %q, want er.weather", daemon.seenInvoke.GetFunctionName())
	}
	if got := daemon.seenInvoke.GetMetadata()[directSignedDescriptorRefMetadata]; got != directRuntimeDraft(t).DescriptorRef() {
		t.Fatalf("signed descriptor metadata = %q", got)
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
	if got := metadata[directSignedDescriptorRefMetadata]; got != directRuntimeDraft(t).DescriptorRef() {
		t.Fatalf("signed descriptor metadata = %q", got)
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

func TestDirectRuntimeTransportProjectsDescriptorRefThroughAddressing(t *testing.T) {
	addressing := NewCanonicalAddressing()
	transport, daemon, cleanup := openDirectRuntimeTestTransportWithOptions(t, DirectRuntimeOptions{
		DialTimeoutMS: 3000,
		Addressing:    addressing,
	})
	defer cleanup()

	client, err := NewRuntimeClient(transport)
	if err != nil {
		t.Fatalf("NewRuntimeClient: %v", err)
	}
	if _, err := client.Invoke(context.Background(), directRuntimeDraft(t)); err != nil {
		t.Fatalf("Invoke: %v", err)
	}
	if daemon.seenInvoke == nil || daemon.seenInvoke.GetFunctionName() != "er.weather" {
		t.Fatalf("daemon function name = %#v", daemon.seenInvoke)
	}
}

func TestDirectRuntimeTransportPrepareProjectsSignedUserSubject(t *testing.T) {
	identity := directRuntimeUserSubjectIdentity(t)
	handle := &directRuntimeFakeHandleTransport{}
	transport, _, cleanup := openDirectRuntimeTestTransportWithOptions(t, DirectRuntimeOptions{
		DialTimeoutMS:   3000,
		Addressing:      identity,
		HandleTransport: handle,
	})
	defer cleanup()
	draft := directRuntimeUserSubjectDraft(t)

	if _, err := transport.Prepare(context.Background(), mustMarshalDirectRuntimeDraft(t, draft), []byte(`{"expires_in_ms":60000}`)); err != nil {
		t.Fatalf("Prepare: %v", err)
	}
	prepared, err := NewInvocationDraftFromJSON(handle.preparedDraft)
	if err != nil {
		t.Fatalf("NewInvocationDraftFromJSON: %v", err)
	}
	if got := prepared.SubjectURA(); got != directRuntimeUserSubjectResourceURA {
		t.Fatalf("delegated draft subject = %q, want %q", got, directRuntimeUserSubjectResourceURA)
	}
	if draft.SubjectURA() == prepared.SubjectURA() {
		t.Fatalf("prepare did not project user subject: %q", draft.SubjectURA())
	}
	if handle.prepareCalls != 1 {
		t.Fatalf("prepare delegation calls = %d, want 1", handle.prepareCalls)
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

func TestDirectRuntimeTransportRejectsDescriptorProjectionWithoutIdentity(t *testing.T) {
	_, err := directLocalAbilityName(context.Background(), nil, directRuntimeDraft(t))
	if err == nil {
		t.Fatalf("directLocalAbilityName accepted descriptor_ref without identity projection")
	}
	if !IsCode(err, ErrInvalidArgument) {
		t.Fatalf("error code = %v, want %s", err, ErrInvalidArgument)
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
	if first.State() != "Running" || first.SelectedNodeID() != "node-stream" || first.SchedulingReason() != "direct-stream" {
		t.Fatalf("first dispatch projection = state %q node %q reason %q", first.State(), first.SelectedNodeID(), first.SchedulingReason())
	}
	terminal, err := stream.Next(context.Background())
	if err != nil {
		t.Fatalf("Next terminal: %v", err)
	}
	if !terminal.Terminal() || stream.State() != StreamTerminalFrameSeen {
		t.Fatalf("terminal = %v state %s", terminal.Terminal(), stream.State())
	}
	if daemon.seenStream == nil || daemon.seenStream.GetFunctionName() != "er.weather" {
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
	daemon.streamDelay = time.Second
	daemon.streamStarted = make(chan struct{})

	client, err := NewRuntimeClient(transport)
	if err != nil {
		t.Fatalf("NewRuntimeClient: %v", err)
	}
	stream, err := client.InvokeStream(context.Background(), directRuntimeDraft(t))
	if err != nil {
		t.Fatalf("InvokeStream open before deadline observation: %v", err)
	}
	select {
	case <-daemon.streamStarted:
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

	daemon.streamDelay = 0
	daemon.streamStarted = nil
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

func TestDirectRuntimeStreamCancelProjectsNonTerminalRequest(t *testing.T) {
	transport, _, cleanup := openDirectRuntimeTestTransport(t)
	defer cleanup()

	streamTransport, _, err := transport.OpenStream(context.Background(), mustMarshalDirectRuntimeDraft(t, directRuntimeDraft(t)))
	if err != nil {
		t.Fatalf("OpenStream: %v", err)
	}
	raw, err := streamTransport.Cancel(context.Background(), "client stop")
	if err != nil {
		t.Fatalf("Cancel stream: %v", err)
	}

	var cancel struct {
		State     StreamState `json:"state"`
		Terminal  bool        `json:"terminal"`
		Cancelled bool        `json:"cancelled"`
		Reason    string      `json:"reason"`
	}
	if err := json.Unmarshal(raw, &cancel); err != nil {
		t.Fatalf("decode stream cancel: %v; raw=%s", err, raw)
	}
	if cancel.State != StreamCancelRequested || cancel.Terminal || cancel.Cancelled || cancel.Reason != "client stop" {
		t.Fatalf("stream cancel = %#v", cancel)
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
	if got := daemon.seenBidi[0].GetEnvelopeOpen().GetMetadata()[directSignedDescriptorRefMetadata]; got != directRuntimeDraft(t).DescriptorRef() {
		t.Fatalf("signed descriptor metadata = %q", got)
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
	daemon.bidiDelay = time.Second
	daemon.bidiStarted = make(chan struct{})

	client, err := NewRuntimeClient(transport)
	if err != nil {
		t.Fatalf("NewRuntimeClient: %v", err)
	}
	session, err := client.OpenBidi(context.Background(), directRuntimeDraft(t), []BidiStreamDescriptor{{StreamID: 1, ContentType: "text/plain"}})
	if err != nil {
		t.Fatalf("OpenBidi before deadline observation: %v", err)
	}
	select {
	case <-daemon.bidiStarted:
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

	daemon.bidiDelay = 0
	daemon.bidiStarted = nil
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

func TestDirectRuntimeBidiCancelProjectsNonTerminalRequest(t *testing.T) {
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
	raw, err := bidiTransport.Cancel(context.Background(), "client stop")
	if err != nil {
		t.Fatalf("Cancel bidi: %v", err)
	}

	var cancel struct {
		State    BidiState `json:"state"`
		Terminal bool      `json:"terminal"`
		Reason   string    `json:"reason"`
	}
	if err := json.Unmarshal(raw, &cancel); err != nil {
		t.Fatalf("decode bidi cancel: %v; raw=%s", err, raw)
	}
	if cancel.State != BidiCancelRequested || cancel.Terminal || cancel.Reason != "client stop" {
		t.Fatalf("bidi cancel = %#v", cancel)
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
		Addressing:           NewCanonicalAddressing(),
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
		Addressing:           directRuntimeIdentityClient(t),
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
	if options.Addressing == nil {
		options.Addressing = directRuntimeIdentityClient(t)
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
	raw, err := json.Marshal(map[string]any{
		"caller_ura":     "easynet:///r/example/agent/alice",
		"callee_ura":     "easynet:///r/example/device/dev-a",
		"descriptor_ref": "easynet:///r/example/ability/device.dev-a.er.weather@1.0.0",
		"subject_ura":    "easynet:///r/example/device/dev-a",
		"nonce_base64":   "AQIDBAUGBwgJCgsMDQ4PEA==",
		"causal_context": map[string]any{"form": "none"},
		"args":           map[string]any{"city": "Singapore"},
		"content_type":   "application/json",
		"metadata":       metadata,
	})
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
		"descriptor_ref": "easynet:///r/example/ability/device.dev-a.meta.list_resources@1.0.0",
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

func directRuntimeIdentityClient(t *testing.T) Addressing {
	t.Helper()
	return NewCanonicalAddressing()
}

func directRuntimeUserSubjectIdentity(t *testing.T) Addressing {
	t.Helper()
	return NewCanonicalAddressing()
}

const directRuntimeUserSubjectResourceURA = "easynet:///r/example/resource/user.alice/invoke/meta.list_resources"
const directRuntimeUserSubjectPubkeyBase64 = "AQIDBAUGBwgJCgsMDQ4PEBESExQVFhcYGRobHB0eHyA="

const directRuntimeDescriptorProjectionJSON = `{
  "kind":"descriptor_ref",
  "valid":true,
  "descriptor_ref":"easynet:///r/example/ability/device.dev-a.er.weather@1.0.0",
  "ability_ura":"easynet:///r/example/ability/device.dev-a.er.weather",
  "descriptor_version":"1.0.0",
  "profile":"easynet-strict-v2",
  "components":{"owner_ura":"easynet:///r/example/device/dev-a"},
  "metadata":{"grammar_owner":"axon"}
}`

const directRuntimeMetaListResourcesDescriptorProjectionJSON = `{
  "kind":"descriptor_ref",
  "valid":true,
  "descriptor_ref":"easynet:///r/example/ability/device.dev-a.meta.list_resources@1.0.0",
  "ability_ura":"easynet:///r/example/ability/device.dev-a.meta.list_resources",
  "descriptor_version":"1.0.0",
  "profile":"easynet-strict-v2",
  "components":{"owner_ura":"easynet:///r/example/device/dev-a"},
  "metadata":{"grammar_owner":"axon"}
}`

func directRuntimeProjectedSubjectJSON(ura string) string {
	raw, err := json.Marshal(map[string]any{
		"kind":       "resource",
		"valid":      true,
		"ura":        ura,
		"profile":    "addressing",
		"components": map[string]any{"owner_ura": "easynet:///r/example/user/alice"},
		"metadata":   map[string]any{"source": "direct-runtime-test"},
	})
	if err != nil {
		panic(err)
	}
	return string(raw)
}
