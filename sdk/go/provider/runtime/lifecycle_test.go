package runtimeprovider_test

import (
	"context"
	"encoding/json"
	"os"
	"path/filepath"
	"runtime"
	"testing"

	runtimesdk "easynet.run/cli/sdk/go"
	runtimeprovider "easynet.run/cli/sdk/go/provider/runtime"
)

type lifecycleTransport struct {
	startCalls int
	start      map[string]any
}

func (t *lifecycleTransport) Discover(context.Context, []byte) ([]byte, error) {
	return []byte(`{"control_endpoint":"unix:///tmp/control.sock","invocation_endpoint":"unix:///tmp/daemon.sock"}`), nil
}

func (t *lifecycleTransport) Start(_ context.Context, payload []byte) ([]byte, error) {
	t.startCalls++
	if err := json.Unmarshal(payload, &t.start); err != nil {
		return nil, err
	}
	return []byte(`{"handle_id":"daemon-1","state":"Running","mode":"authority","endpoints":{"invocation_endpoint":"unix:///tmp/daemon.sock"}}`), nil
}

func (t *lifecycleTransport) Attach(context.Context, []byte) ([]byte, error) {
	return []byte(`{"handle_id":"daemon-1","state":"Running","mode":"authority","endpoints":{"invocation_endpoint":"unix:///tmp/daemon.sock"}}`), nil
}

func (t *lifecycleTransport) Status(context.Context, string) ([]byte, error) {
	return []byte(`{"handle_id":"daemon-1","state":"Running","mode":"authority","endpoints":{"invocation_endpoint":"unix:///tmp/daemon.sock"}}`), nil
}

func (t *lifecycleTransport) OpenRuntime(context.Context, string, []byte) (runtimesdk.RuntimeTransport, []byte, error) {
	return runtimesdk.RuntimeTransportFunc{}, []byte(`{}`), nil
}

func (t *lifecycleTransport) Stop(context.Context, string, []byte) ([]byte, error) {
	return []byte(`{"handle_id":"daemon-1","state":"Stopped","mode":"authority"}`), nil
}

func (t *lifecycleTransport) Detach(context.Context, string) error {
	return nil
}

func TestLifecycleDelegatesToCanonicalRuntimeHost(t *testing.T) {
	transport := &lifecycleTransport{}
	provider, err := runtimeprovider.NewLifecycle(transport)
	if err != nil {
		t.Fatalf("NewLifecycle: %v", err)
	}

	handle, err := provider.Start(context.Background(), runtimeprovider.RuntimeHostStartConfig{
		Mode:       runtimeprovider.ModeAuthority,
		Realm:      "example.com",
		RuntimeBin: "/usr/local/bin/runtime-host",
		HomeDir:    "/var/lib/runtime-host",
	})
	if err != nil {
		t.Fatalf("Start: %v", err)
	}

	if transport.startCalls != 1 || transport.start["runtime_bin"] != "/usr/local/bin/runtime-host" {
		t.Fatalf("provider did not lower start config exactly once: %#v", transport.start)
	}
	if transport.start["mode"] != "authority" {
		t.Fatalf("provider leaked host wire mode = %v, want authority", transport.start["mode"])
	}
	if handle.HandleID() != "daemon-1" || handle.State() != runtimesdk.RuntimeRunning {
		t.Fatalf("provider returned a non-canonical handle: %#v", handle)
	}
}

func TestLifecycleRejectsRuntimeProviderPolicyBeforeTransport(t *testing.T) {
	transport := &lifecycleTransport{}
	provider, err := runtimeprovider.NewLifecycle(transport)
	if err != nil {
		t.Fatalf("NewLifecycle: %v", err)
	}

	_, err = provider.Start(context.Background(), runtimeprovider.RuntimeHostStartConfig{
		Mode:      runtimeprovider.ModeEdge,
		ListenTCP: "0.0.0.0:9443",
	})
	if err == nil {
		t.Fatal("Start accepted a public edge listener")
	}
	if transport.startCalls != 0 {
		t.Fatal("transport was called before provider policy validation")
	}
}

func TestRuntimeLifecycleDTOsAreOwnedByRuntimeProvider(t *testing.T) {
	_, currentFile, _, ok := runtime.Caller(0)
	if !ok {
		t.Fatal("resolve current test file")
	}
	rootFile := filepath.Clean(filepath.Join(filepath.Dir(currentFile), "..", "..", "daemon_compat.go"))
	if _, err := os.Stat(rootFile); err == nil {
		t.Fatalf("canonical root must not restore daemon compatibility module: %s", rootFile)
	} else if !os.IsNotExist(err) {
		t.Fatalf("stat canonical daemon compatibility module: %v", err)
	}

	_ = runtimeprovider.RuntimeHostStartConfig{}
	_ = runtimeprovider.RuntimeHostAttachOptions{}
	_ = runtimeprovider.RuntimeHostDiscoverOptions{}
	_ = runtimeprovider.RuntimeHostStopOptions{}
}
