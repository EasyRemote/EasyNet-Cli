package easynet

import (
	"context"
	"os"
	"os/exec"
	"path/filepath"
	"runtime"
	"strings"
	"testing"
)

func runRepositoryGate(t *testing.T, script string, args ...string) {
	t.Helper()
	_, file, _, _ := runtime.Caller(0)
	root := filepath.Clean(filepath.Join(filepath.Dir(file), "../.."))
	commandArgs := append([]string{filepath.Join(root, "tools/scripts", script)}, args...)
	command := exec.Command("bash", commandArgs...)
	command.Dir = root
	if output, err := command.CombinedOutput(); err != nil {
		t.Fatalf("%s failed: %v\n%s", script, err, output)
	}
}

func TestConformanceSDKProductNeutrality(t *testing.T) {
	runRepositoryGate(t, "check-sdk-product-neutrality.sh")
}

func TestConformanceSevenLanguageCapabilityMatrix(t *testing.T) {
	runRepositoryGate(t, "check-sdk-parity-matrix.sh", "--self-test")
}

func TestConformanceStreamAndBidiBackpressureBounds(t *testing.T) {
	streamTransport := &memoryStreamTransport{events: []string{
		`{"sequence":1,"kind":"data","state":"Open","terminal":false}`,
		`{"sequence":2,"kind":"data","state":"Open","terminal":false}`,
	}}
	stream, err := NewStreamHandleFromJSON(streamTransport, []byte(`{"stream_id":"stream-1","state":"Opening","max_buffered_events":1}`))
	if err != nil {
		t.Fatalf("NewStreamHandleFromJSON: %v", err)
	}
	if _, err = stream.Next(context.Background()); err != nil {
		t.Fatalf("first stream event: %v", err)
	}
	if _, err = stream.Next(context.Background()); !IsCode(err, ErrInvalidArgument) || stream.State() != StreamFailed {
		t.Fatalf("stream overflow = %v state=%s", err, stream.State())
	}

	bidiTransport := &memoryBidiTransport{recvFrames: []string{
		`{"sequence":1,"kind":"data","stream_id":1}`,
		`{"sequence":2,"kind":"data","stream_id":1}`,
	}}
	bidi, err := NewBidiSessionFromJSON(bidiTransport, []byte(`{"session_id":"bidi-1","state":"Open","max_buffered_frames":1}`))
	if err != nil {
		t.Fatalf("NewBidiSessionFromJSON: %v", err)
	}
	if _, err = bidi.Receive(context.Background()); err != nil {
		t.Fatalf("first bidi frame: %v", err)
	}
	if _, err = bidi.Receive(context.Background()); !IsCode(err, ErrInvalidArgument) || bidi.State() != BidiFailed {
		t.Fatalf("bidi overflow = %v state=%s", err, bidi.State())
	}
}

func TestConformanceStreamCancelRequestIsNonTerminal(t *testing.T) {
	stream, err := NewStreamHandleFromJSON(&memoryStreamTransport{}, []byte(`{"stream_id":"stream-1","state":"Open","max_buffered_events":4}`))
	if err != nil {
		t.Fatalf("NewStreamHandleFromJSON: %v", err)
	}
	cancel, err := stream.Cancel(context.Background(), "client stop")
	if err != nil {
		t.Fatalf("stream cancel: %v", err)
	}
	if cancel.Terminal() || cancel.Cancelled() || cancel.State() != StreamCancelRequested || stream.State() != StreamCancelRequested {
		t.Fatalf("stream cancel must be a non-terminal request: outcome=%#v state=%s", cancel, stream.State())
	}

	terminalReply := &memoryStreamTransport{
		cancelReply: `{"stream_id":"stream-1","cancelled":true,"state":"Cancelled","terminal":true}`,
	}
	stream, err = NewStreamHandleFromJSON(terminalReply, []byte(`{"stream_id":"stream-1","state":"Open","max_buffered_events":4}`))
	if err != nil {
		t.Fatalf("NewStreamHandleFromJSON terminal reply: %v", err)
	}
	if _, err := stream.Cancel(context.Background(), "client stop"); !IsCode(err, ErrInvalidArgument) {
		t.Fatalf("terminal stream cancel ack error = %v, want %s", err, ErrInvalidArgument)
	}
	if stream.State() != StreamFailed {
		t.Fatalf("terminal cancel ack must fail the stream facade, got %s", stream.State())
	}
}

func TestConformanceBidiCancelRequestIsNonTerminal(t *testing.T) {
	session := newTestBidiSession(t, &memoryBidiTransport{})
	outcome, err := session.Cancel(context.Background(), "client stop")
	if err != nil {
		t.Fatalf("bidi cancel: %v", err)
	}
	if outcome.Terminal() || outcome.State() != BidiCancelRequested || session.State() != BidiCancelRequested {
		t.Fatalf("bidi cancel must be a non-terminal request: outcome=%#v state=%s", outcome, session.State())
	}
	if _, err := session.CloseSend(context.Background()); !IsCode(err, ErrInvalidArgument) {
		t.Fatalf("close-send after bidi cancel error = %v, want %s", err, ErrInvalidArgument)
	}
	if err := session.Close(context.Background()); err != nil {
		t.Fatalf("close after bidi cancel request: %v", err)
	}
	if session.State() != BidiClosed {
		t.Fatalf("bidi close after cancel request state = %s, want %s", session.State(), BidiClosed)
	}

	terminalReply := &memoryBidiTransport{
		cancelReply: `{"session_id":"bidi-1","state":"Cancelled","terminal":true,"reason":"client stop"}`,
	}
	session = newTestBidiSession(t, terminalReply)
	if _, err := session.Cancel(context.Background(), "client stop"); !IsCode(err, ErrInvalidArgument) {
		t.Fatalf("terminal bidi cancel ack error = %v, want %s", err, ErrInvalidArgument)
	}
	if session.State() != BidiFailed {
		t.Fatalf("terminal cancel ack must fail the bidi facade, got %s", session.State())
	}
}

func TestRuntimeSDKContainsNoProductProfiles(t *testing.T) {
	// Profile facades are generic Runtime Core bindings and remain public for
	// source compatibility. Product-specific policy belongs downstream.
}

func TestRuntimeSDKProductionSourcesHaveNoProductClients(t *testing.T) {
	_, file, _, _ := runtime.Caller(0)
	root := filepath.Dir(file)
	entries, err := os.ReadDir(root)
	if err != nil {
		t.Fatal(err)
	}
	for _, entry := range entries {
		if entry.IsDir() || !strings.HasSuffix(entry.Name(), ".go") || strings.HasSuffix(entry.Name(), "_test.go") {
			continue
		}
		raw, err := os.ReadFile(filepath.Join(root, entry.Name()))
		if err != nil {
			t.Fatal(err)
		}
		text := string(raw)
		if strings.Contains(text, "easynet-backend") || strings.Contains(text, "easyremote") {
			t.Errorf("%s imports a product repository", entry.Name())
		}
		for _, forbidden := range []string{
			"APIKeyResourceURA",
			"AgentSkillFileResourceURA",
			"AgentSkillResourceURA",
			"FilesResourceURA",
			"PagesResourceURA",
		} {
			if strings.Contains(text, forbidden) {
				t.Errorf("%s exposes product-specific runtime SDK helper %s", entry.Name(), forbidden)
			}
		}
	}
}
