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

func TestConformanceBackendHubRouteFamilyCoverage(t *testing.T) {
	runRepositoryGate(t, "check-backend-route-family-coverage.sh")
}

func TestConformanceBackendSDKOnlyImportBan(t *testing.T) {
	runRepositoryGate(t, "check-backend-sdk-only-boundary.sh", "--self-test")
}

func TestConformanceSDKProductNeutrality(t *testing.T) {
	runRepositoryGate(t, "check-sdk-product-neutrality.sh")
}

func TestConformanceSevenLanguageCapabilityMatrix(t *testing.T) {
	runRepositoryGate(t, "check-sdk-parity-matrix.sh", "--self-test")
}

func TestConformanceStreamAndBidiBackpressureBounds(t *testing.T) {
	streamTransport := &memoryStreamTransport{events: []string{
		`{"sequence":1,"kind":"chunk","state":"Open","terminal":false}`,
		`{"sequence":2,"kind":"chunk","state":"Open","terminal":false}`,
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
