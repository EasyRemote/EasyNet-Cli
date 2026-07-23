package easynet_test

import (
	"context"
	"encoding/json"
	"go/ast"
	"go/parser"
	"go/token"
	"os"
	"path/filepath"
	"runtime"
	"strings"
	"testing"

	runtimesdk "easynet.run/cli/sdk/go"
	easynetprovider "easynet.run/cli/sdk/go/provider/easynet"
)

type lifecycleTransport struct {
	startCalls int
	start      map[string]any
}

func TestProviderMapsDaemonCredentialsToCanonicalRuntimeIdentity(t *testing.T) {
	dir := t.TempDir()
	credentials := filepath.Join(dir, "credentials.json")
	if err := os.WriteFile(credentials, []byte(`{
		"realm": "acme",
		"device_id": "dev-a",
		"username": "alice",
		"hub_endpoint": "https://hub.example"
	}`), 0o600); err != nil {
		t.Fatalf("write credentials: %v", err)
	}

	projection, err := easynetprovider.ReadDaemonRuntimeIdentityProjection(context.Background(), credentials)
	if err != nil {
		t.Fatalf("ReadDaemonRuntimeIdentityProjection: %v", err)
	}
	if projection.Realm != "acme" ||
		projection.RuntimeInstanceID != "dev-a" ||
		projection.Principal != "alice" ||
		projection.ControlPlaneEndpoint != "https://hub.example" {
		t.Fatalf("unexpected projection: %#v", projection)
	}
}

func TestProviderRejectsRetiredDaemonNodeIDAlias(t *testing.T) {
	dir := t.TempDir()
	credentials := filepath.Join(dir, "credentials.json")
	if err := os.WriteFile(credentials, []byte(`{
		"realm": "acme",
		"node_id": "node-a"
	}`), 0o600); err != nil {
		t.Fatalf("write credentials: %v", err)
	}

	_, err := easynetprovider.ReadDaemonRuntimeIdentityProjection(context.Background(), credentials)
	if err == nil {
		t.Fatal("expected retired node_id daemon identity alias to fail")
	}
	if !strings.Contains(err.Error(), "retired node_id identity alias") {
		t.Fatalf("error = %v", err)
	}
}

func TestProviderRejectsNodeIDEvenWhenDeviceIDIsPresent(t *testing.T) {
	dir := t.TempDir()
	credentials := filepath.Join(dir, "credentials.json")
	if err := os.WriteFile(credentials, []byte(`{
		"realm": "acme",
		"device_id": "dev-a",
		"node_id": "node-b"
	}`), 0o600); err != nil {
		t.Fatalf("write credentials: %v", err)
	}

	_, err := easynetprovider.ReadDaemonRuntimeIdentityProjection(context.Background(), credentials)
	if err == nil {
		t.Fatal("expected retired node_id daemon identity alias to fail")
	}
	if !strings.Contains(err.Error(), "retired node_id identity alias") {
		t.Fatalf("error = %v", err)
	}
}

func (t *lifecycleTransport) Discover(context.Context, []byte) ([]byte, error) {
	return []byte(`{"control_endpoint":"unix:///tmp/control.sock","invocation_endpoint":"unix:///tmp/daemon.sock"}`), nil
}

func (t *lifecycleTransport) Start(_ context.Context, payload []byte) ([]byte, error) {
	t.startCalls++
	if err := json.Unmarshal(payload, &t.start); err != nil {
		return nil, err
	}
	return []byte(`{"handle_id":"daemon-1","state":"Running","mode":"hub","endpoints":{"invocation_endpoint":"unix:///tmp/daemon.sock"}}`), nil
}

func (t *lifecycleTransport) Attach(context.Context, []byte) ([]byte, error) {
	return []byte(`{"handle_id":"daemon-1","state":"Running","mode":"hub","endpoints":{"invocation_endpoint":"unix:///tmp/daemon.sock"}}`), nil
}

func (t *lifecycleTransport) Status(context.Context, string) ([]byte, error) {
	return []byte(`{"handle_id":"daemon-1","state":"Running","mode":"hub","endpoints":{"invocation_endpoint":"unix:///tmp/daemon.sock"}}`), nil
}

func (t *lifecycleTransport) OpenRuntime(context.Context, string, []byte) (runtimesdk.RuntimeTransport, []byte, error) {
	return runtimesdk.RuntimeTransportFunc{}, []byte(`{}`), nil
}

func (t *lifecycleTransport) Stop(context.Context, string, []byte) ([]byte, error) {
	return []byte(`{"handle_id":"daemon-1","state":"Stopped","mode":"hub"}`), nil
}

func (t *lifecycleTransport) Detach(context.Context, string) error {
	return nil
}

func TestLifecycleDelegatesToCanonicalRuntimeHost(t *testing.T) {
	transport := &lifecycleTransport{}
	provider, err := easynetprovider.NewLifecycle(transport)
	if err != nil {
		t.Fatalf("NewLifecycle: %v", err)
	}

	handle, err := provider.Start(context.Background(), easynetprovider.StartConfig{
		Mode:      easynetprovider.ModeHub,
		Realm:     "example.com",
		DaemonBin: "/usr/local/bin/easynet",
		HomeDir:   "/var/lib/easynet",
	})
	if err != nil {
		t.Fatalf("Start: %v", err)
	}

	if transport.startCalls != 1 || transport.start["daemon_bin"] != "/usr/local/bin/easynet" {
		t.Fatalf("provider did not lower start config exactly once: %#v", transport.start)
	}
	if handle.HandleID() != "daemon-1" || handle.State() != runtimesdk.RuntimeRunning {
		t.Fatalf("provider returned a non-canonical handle: %#v", handle)
	}
}

func TestLifecycleRejectsEasyNetPolicyBeforeTransport(t *testing.T) {
	transport := &lifecycleTransport{}
	provider, err := easynetprovider.NewLifecycle(transport)
	if err != nil {
		t.Fatalf("NewLifecycle: %v", err)
	}

	_, err = provider.Start(context.Background(), easynetprovider.StartConfig{
		Mode:      easynetprovider.ModeDevice,
		ListenTCP: "0.0.0.0:9443",
	})
	if err == nil {
		t.Fatal("Start accepted a public device listener")
	}
	if transport.startCalls != 0 {
		t.Fatal("transport was called before provider policy validation")
	}
}

func TestEasyNetLifecycleDTOsAreOwnedByProviderContract(t *testing.T) {
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

	contractFile := filepath.Clean(filepath.Join(filepath.Dir(currentFile), "contract", "lifecycle.go"))
	parsed, err := parser.ParseFile(token.NewFileSet(), contractFile, nil, 0)
	if err != nil {
		t.Fatalf("parse provider lifecycle contract: %v", err)
	}

	implemented := map[string]bool{}
	for _, declaration := range parsed.Decls {
		general, ok := declaration.(*ast.GenDecl)
		if !ok || general.Tok != token.TYPE {
			continue
		}
		for _, spec := range general.Specs {
			typeSpec, ok := spec.(*ast.TypeSpec)
			if !ok {
				continue
			}
			switch typeSpec.Name.Name {
			case "StartConfig", "AttachOptions", "DiscoverOptions", "StopOptions":
				implemented[typeSpec.Name.Name] = true
			}
		}
	}
	for _, name := range []string{"StartConfig", "AttachOptions", "DiscoverOptions", "StopOptions"} {
		if !implemented[name] {
			t.Fatalf("provider contract does not own %s", name)
		}
	}
}
