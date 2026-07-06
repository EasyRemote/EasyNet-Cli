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

	"easynet.run/cli/sdk/go/internal/axonpb"
	"google.golang.org/grpc"
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
			Index:           1,
			InvocationId:    "inv-bidi",
			ReceiptType:     "completed",
			State:           axonpb.InvocationState_INVOCATION_STATE_COMPLETED,
			CleanupComplete: true,
		}},
	})
}

func TestDirectDaemonRuntimeTransportInvokesOverUnixSocket(t *testing.T) {
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

func TestDirectDaemonRuntimeTransportProjectsDescriptorRefThroughIdentity(t *testing.T) {
	identityTransport := &memoryIdentityTransport{descriptorJSON: directRuntimeDescriptorProjectionJSON}
	identity, err := NewIdentityClient(identityTransport)
	if err != nil {
		t.Fatalf("NewIdentityClient: %v", err)
	}
	transport, daemon, cleanup := openDirectRuntimeTestTransportWithOptions(t, DirectRuntimeOptions{
		DialTimeoutMS: 3000,
		Identity:      identity,
	})
	defer cleanup()

	client, err := NewRuntimeClient(transport)
	if err != nil {
		t.Fatalf("NewRuntimeClient: %v", err)
	}
	if _, err := client.Invoke(context.Background(), directRuntimeDraft(t)); err != nil {
		t.Fatalf("Invoke: %v", err)
	}
	if identityTransport.seenRequest["descriptor_ref"] != directRuntimeDraft(t).DescriptorRef() {
		t.Fatalf("descriptor projection request = %#v", identityTransport.seenRequest)
	}
	if daemon.seenInvoke == nil || daemon.seenInvoke.GetFunctionName() != "er.weather" {
		t.Fatalf("daemon function name = %#v", daemon.seenInvoke)
	}
}

func TestDirectDaemonRuntimeTransportRejectsDescriptorProjectionWithoutIdentity(t *testing.T) {
	_, err := directLocalAbilityName(context.Background(), nil, directRuntimeDraft(t))
	if err == nil {
		t.Fatalf("directLocalAbilityName accepted descriptor_ref without identity projection")
	}
	if !IsCode(err, ErrInvalidArgument) {
		t.Fatalf("error code = %v, want %s", err, ErrInvalidArgument)
	}
}

func TestDirectDaemonRuntimeTransportStreamsOverUnixSocket(t *testing.T) {
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

func TestDirectDaemonRuntimeTransportBidiOverUnixSocket(t *testing.T) {
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

func TestDirectDaemonRuntimeTransportDelegatesHandleOperations(t *testing.T) {
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
	if _, err := transport.AwaitHandle(context.Background(), 7); err != nil {
		t.Fatalf("AwaitHandle: %v", err)
	}
	if _, err := transport.CancelHandle(context.Background(), 7, "client stop"); err != nil {
		t.Fatalf("CancelHandle: %v", err)
	}
	if _, err := transport.HandleEvents(context.Background(), 7); err != nil {
		t.Fatalf("HandleEvents: %v", err)
	}
	if err := transport.FreeHandle(context.Background(), 7); err != nil {
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

func TestDirectDaemonRuntimeConnectorProjectsHandleCapabilities(t *testing.T) {
	handle := &directRuntimeFakeHandleTransport{}
	transport, _, cleanup := openDirectRuntimeTestTransportWithOptions(t, DirectRuntimeOptions{DialTimeoutMS: 3000})
	defer cleanup()
	endpoint := transport.endpoint

	connector := NewDirectDaemonRuntimeConnectorWithOptions(DirectDaemonRuntimeConnectorOptions{
		Reader: ControlDiscoveryReaderFunc(func(context.Context, string) (ControlDiscovery, error) {
			return ControlDiscovery{InvocationEndpoint: endpoint}, nil
		}),
		HandleTransport:      handle,
		Identity:             directRuntimeIdentityClient(t),
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

func TestDirectDaemonRuntimeConnectorResolvesControlDiscovery(t *testing.T) {
	reader := ControlDiscoveryReaderFunc(func(ctx context.Context, controlPath string) (ControlDiscovery, error) {
		return ControlDiscovery{InvocationEndpoint: "/tmp/direct.sock"}, nil
	})
	connector := NewDirectDaemonRuntimeConnector("/tmp/control.json", reader)
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

func openDirectRuntimeTestTransport(t *testing.T) (*DirectDaemonRuntimeTransport, *directRuntimeFakeDaemon, func()) {
	return openDirectRuntimeTestTransportWithOptions(t, DirectRuntimeOptions{DialTimeoutMS: 3000})
}

func openDirectRuntimeTestTransportWithOptions(t *testing.T, options DirectRuntimeOptions) (*DirectDaemonRuntimeTransport, *directRuntimeFakeDaemon, func()) {
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
	if options.Identity == nil {
		options.Identity = directRuntimeIdentityClient(t)
	}
	transport, err := OpenDirectDaemonRuntimeTransport(context.Background(), socket, options)
	if err != nil {
		server.Stop()
		<-done
		_ = os.RemoveAll(dir)
		t.Fatalf("OpenDirectDaemonRuntimeTransport: %v", err)
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

func (f *directRuntimeFakeHandleTransport) AwaitHandle(context.Context, uint64) ([]byte, error) {
	f.awaitCalls++
	return []byte(`{"ok":true,"state":"Completed","terminal_state":"Completed","output_json":{"done":true}}`), nil
}

func (f *directRuntimeFakeHandleTransport) CancelHandle(context.Context, uint64, string) ([]byte, error) {
	f.cancelCalls++
	return []byte(`{"handle_id":7,"cancelled":true,"state":"Cancelled","terminal":true}`), nil
}

func (f *directRuntimeFakeHandleTransport) HandleEvents(context.Context, uint64) ([]byte, error) {
	f.eventsCalls++
	return []byte(`{"handle_id":7,"state":"Submitted","terminal":false,"events":[],"result":null}`), nil
}

func (f *directRuntimeFakeHandleTransport) FreeHandle(context.Context, uint64) error {
	f.freeCalls++
	return nil
}

func (f *directRuntimeFakeHandleTransport) Close(context.Context) error {
	f.closeCalls++
	return nil
}

func directRuntimeDraft(t *testing.T) InvocationDraft {
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
		"metadata":       map[string]any{"trace_id": "direct-test"},
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

func directRuntimeIdentityClient(t *testing.T) *IdentityClient {
	t.Helper()
	identity, err := NewIdentityClient(&memoryIdentityTransport{descriptorJSON: directRuntimeDescriptorProjectionJSON})
	if err != nil {
		t.Fatalf("NewIdentityClient: %v", err)
	}
	return identity
}

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
