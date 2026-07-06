package easynet

import (
	"context"
	"encoding/json"
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
	if got := string(daemon.seenInvoke.GetArguments()); got != `{"city":"Singapore"}` {
		t.Fatalf("arguments = %s", got)
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
	if daemon.seenStream == nil || daemon.seenStream.GetFunctionName() != directRuntimeDraft(t).DescriptorRef() {
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
	if err := session.Close(context.Background()); err != nil {
		t.Fatalf("Close bidi: %v", err)
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
	transport, err := OpenDirectDaemonRuntimeTransport(context.Background(), socket, DirectRuntimeOptions{DialTimeoutMS: 3000})
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
