//go:build easynet_direct_runtime

package easynet

import (
	"context"
	"crypto/ed25519"
	"encoding/base64"
	"encoding/json"
	"errors"
	"net"
	"os"
	"path/filepath"
	"testing"

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
}

func (d *directRuntimeFakeDaemon) Invoke(_ context.Context, req *axonpb.InvokeRequest) (*axonpb.InvokeResponse, error) {
	d.seenInvoke = req
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

func (d *directRuntimeFakeDaemon) InvokeStream(req *axonpb.InvokeServerStreamRequest, stream grpc.ServerStreamingServer[axonpb.InvokeStreamChunk]) error {
	d.seenStream = req
	if err := stream.Send(&axonpb.InvokeStreamChunk{
		Sequence:    0,
		State:       axonpb.InvocationState_INVOCATION_STATE_RUNNING,
		Payload:     []byte(`{"delta":1}`),
		ContentType: "application/json",
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
	transport, _, cleanup := openDirectRuntimeTestTransportWithOptions(t, DirectRuntimeOptions{
		DialTimeoutMS: 3000,
		Addressing:    identity,
	})
	defer cleanup()
	draft := directRuntimeUserSubjectDraft(t)

	preparedJSON, err := transport.Prepare(context.Background(), mustMarshalDirectRuntimeDraft(t, draft), []byte(`{"expires_in_ms":60000}`))
	if err != nil {
		t.Fatalf("Prepare: %v", err)
	}
	prepared, err := NewPreparedInvocationFromJSON(preparedJSON)
	if err != nil {
		t.Fatalf("NewPreparedInvocationFromJSON: %v", err)
	}
	if got := prepared.Tuple().SubjectURA(); got != directRuntimeUserSubjectResourceURA {
		t.Fatalf("prepared tuple subject = %q, want %q", got, directRuntimeUserSubjectResourceURA)
	}
	if draft.SubjectURA() == prepared.Tuple().SubjectURA() {
		t.Fatalf("prepare did not project user subject: %q", draft.SubjectURA())
	}
	expectedMaterial, err := signingMaterialForInvocationDraft(prepared.Tuple())
	if err != nil {
		t.Fatalf("signingMaterialForInvocationDraft(projected tuple): %v", err)
	}
	if prepared.SigningMaterial().CanonicalBytesBase64() != expectedMaterial.CanonicalBytesBase64() {
		t.Fatalf("prepared signing material was not built from projected tuple")
	}
}

func TestDirectRuntimeTransportSubmitSignedDispatchesPreparedProjectedSubject(t *testing.T) {
	identity := directRuntimeUserSubjectIdentity(t)
	transport, daemon, cleanup := openDirectRuntimeTestTransportWithOptions(t, DirectRuntimeOptions{
		DialTimeoutMS: 3000,
		Addressing:    identity,
	})
	defer cleanup()
	preparedJSON, err := transport.Prepare(context.Background(), mustMarshalDirectRuntimeDraft(t, directRuntimeUserSubjectDraft(t)), []byte(`{"expires_in_ms":60000,"signer_id":"caller-key"}`))
	if err != nil {
		t.Fatalf("Prepare: %v", err)
	}
	prepared, err := NewPreparedInvocationFromJSON(preparedJSON)
	if err != nil {
		t.Fatalf("NewPreparedInvocationFromJSON: %v", err)
	}
	seed := [32]byte{}
	for i := range seed {
		seed[i] = byte(i + 1)
	}
	privateKey := ed25519.NewKeyFromSeed(seed[:])
	publicKey := privateKey.Public().(ed25519.PublicKey)
	publicKeyBase64 := base64.StdEncoding.EncodeToString(publicKey)
	canonicalBytes, err := base64.StdEncoding.DecodeString(prepared.SigningMaterial().CanonicalBytesBase64())
	if err != nil {
		t.Fatalf("canonical decode: %v", err)
	}
	signature := ed25519.Sign(privateKey, canonicalBytes)
	signed, err := prepared.SignWithCallerSignature(InvocationSignature{
		Algorithm:             "ed25519",
		SignatureBase64:       base64.StdEncoding.EncodeToString(signature),
		SignerPublicKeyBase64: publicKeyBase64,
	})
	if err != nil {
		t.Fatalf("SignWithCallerSignature: %v", err)
	}
	signedJSON, err := json.Marshal(signed)
	if err != nil {
		t.Fatalf("marshal signed: %v", err)
	}

	if _, err := transport.SubmitSigned(context.Background(), signedJSON); err != nil {
		t.Fatalf("SubmitSigned: %v", err)
	}
	if daemon.seenInvoke == nil {
		t.Fatal("daemon did not receive signed invocation")
	}
	if got := daemon.seenInvoke.GetEnvelope().GetSubject().GetUra(); got != prepared.Tuple().SubjectURA() {
		t.Fatalf("dispatched subject = %q, want prepared tuple subject %q", got, prepared.Tuple().SubjectURA())
	}
	if got := daemon.seenInvoke.GetEnvelope().GetSubject().GetUra(); got != directRuntimeUserSubjectResourceURA {
		t.Fatalf("dispatched subject = %q, want %q", got, directRuntimeUserSubjectResourceURA)
	}
	if got := daemon.seenInvoke.GetEnvelope().GetCallerSignature().GetKeyIdHint(); got != publicKeyBase64 {
		t.Fatalf("caller signature key hint = %q, want user pubkey", got)
	}
	wireEnvelope := daemon.seenInvoke.GetEnvelope()
	wireCanonical, err := CanonicalInvocationBytes(Envelope{
		Caller:        AgentRef{URA: wireEnvelope.GetCaller().GetUra()},
		Callee:        AgentRef{URA: wireEnvelope.GetCallee().GetUra()},
		Subject:       SubjectRef{URA: wireEnvelope.GetSubject().GetUra()},
		CausalContext: CausalNullWithReason(""),
		Nonce:         wireEnvelope.GetInvocationNonce(),
	}, prepared.Tuple().DescriptorRef(), daemon.seenInvoke.GetArguments())
	if err != nil {
		t.Fatalf("wire canonical bytes: %v", err)
	}
	if !ed25519.Verify(publicKey, wireCanonical, signature) {
		t.Fatal("daemon wire request must verify against prepare-time caller signature")
	}
	if got := daemon.seenInvoke.GetMetadata()[directSignedDescriptorRefMetadata]; got != prepared.Tuple().DescriptorRef() {
		t.Fatalf("signed descriptor metadata = %q, want %q", got, prepared.Tuple().DescriptorRef())
	}
	if got := daemon.seenInvoke.GetFunctionName(); got != prepared.Tuple().DescriptorRef() {
		t.Fatalf("signed dispatch function_name = %q, want descriptor ref %q", got, prepared.Tuple().DescriptorRef())
	}
	if got := daemon.seenInvoke.GetTarget().GetAbilityName(); got != prepared.Tuple().DescriptorRef() {
		t.Fatalf("signed dispatch target ability_name = %q, want descriptor ref %q", got, prepared.Tuple().DescriptorRef())
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
	if first.Sequence() != 1 || first.Terminal() {
		t.Fatalf("first = seq %d terminal %v", first.Sequence(), first.Terminal())
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

func TestDirectRuntimeTransportDelegatesHandleOperations(t *testing.T) {
	handle := &directRuntimeFakeHandleTransport{}
	transport, _, cleanup := openDirectRuntimeTestTransportWithOptions(t, DirectRuntimeOptions{
		DialTimeoutMS:        3000,
		HandleTransport:      handle,
		CloseHandleTransport: true,
	})
	defer cleanup()

	prepared, err := transport.Prepare(context.Background(), []byte(`{"draft":true}`), []byte(`{"expires_in_ms":1000}`))
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

func TestDirectRuntimeTransportProvidesHandleOperationsWithoutDelegate(t *testing.T) {
	transport, daemon, cleanup := openDirectRuntimeTestTransportWithOptions(t, DirectRuntimeOptions{DialTimeoutMS: 3000})
	defer cleanup()

	preparedJSON, err := transport.Prepare(context.Background(), mustMarshalDirectRuntimeDraft(t, directRuntimeDraft(t)), []byte(`{"expires_in_ms":60000,"signer_id":"caller-key"}`))
	if err != nil {
		t.Fatalf("Prepare: %v", err)
	}
	prepared, err := NewPreparedInvocationFromJSON(preparedJSON)
	if err != nil {
		t.Fatalf("NewPreparedInvocationFromJSON: %v", err)
	}
	signed, err := prepared.SignWithCallerSignature(InvocationSignature{
		Algorithm:       "ed25519",
		SignatureBase64: base64.StdEncoding.EncodeToString([]byte("direct-signature")),
		KeyIDHint:       "caller-key",
	})
	if err != nil {
		t.Fatalf("SignWithCallerSignature: %v", err)
	}
	signedJSON, err := json.Marshal(signed)
	if err != nil {
		t.Fatalf("marshal signed: %v", err)
	}
	handleJSON, err := transport.SubmitSigned(context.Background(), signedJSON)
	if err != nil {
		t.Fatalf("SubmitSigned: %v", err)
	}
	handle, err := newRuntimeInvocationHandleFromJSON(handleJSON)
	if err != nil {
		t.Fatalf("newRuntimeInvocationHandleFromJSON: %v", err)
	}
	if !handle.ControlCapability().valid() || !handle.Terminal() || daemon.seenInvoke.GetEnvelope().GetCallerSignature() == nil {
		t.Fatalf("direct handle submit did not invoke signed request: handle=%#v request=%#v", handle, daemon.seenInvoke)
	}
	control := handle.ControlCapability()
	resultJSON, err := transport.AwaitHandle(context.Background(), control)
	if err != nil {
		t.Fatalf("AwaitHandle: %v", err)
	}
	result, err := NewInvocationResultFromJSON(resultJSON)
	if err != nil {
		t.Fatalf("NewInvocationResultFromJSON: %v", err)
	}
	if !result.OK() {
		t.Fatalf("await result not ok: %#v", result.Failure())
	}
	eventsJSON, err := transport.HandleEvents(context.Background(), control)
	if err != nil {
		t.Fatalf("HandleEvents: %v", err)
	}
	events, err := NewInvocationHandleFromJSON(eventsJSON)
	if err != nil {
		t.Fatalf("events handle decode: %v", err)
	}
	if len(events.Events()) != 1 {
		t.Fatalf("events = %#v", events.Events())
	}
	if err := transport.FreeHandle(context.Background(), control); err != nil {
		t.Fatalf("FreeHandle: %v", err)
	}
	if _, err := transport.AwaitHandle(context.Background(), control); !IsCode(err, ErrNotFound) {
		t.Fatalf("AwaitHandle after free = %v, want %s", err, ErrNotFound)
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
	prepareCalls int
	submitCalls  int
	awaitCalls   int
	cancelCalls  int
	eventsCalls  int
	freeCalls    int
	closeCalls   int
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

func (f *directRuntimeFakeHandleTransport) Prepare(context.Context, []byte, []byte) ([]byte, error) {
	f.prepareCalls++
	return []byte(preparedFixture), nil
}

func (f *directRuntimeFakeHandleTransport) SubmitSigned(context.Context, []byte) ([]byte, error) {
	f.submitCalls++
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
