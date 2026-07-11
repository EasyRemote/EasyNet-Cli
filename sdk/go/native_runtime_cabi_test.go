//go:build easynet_cabi && cgo && !windows

package easynet

import (
	"context"
	"encoding/json"
	"path/filepath"
	"testing"
)

func TestOpenNativeRuntimeUsesNativeProviderBuild(t *testing.T) {
	missing := filepath.Join(t.TempDir(), "libeasynet_cli_missing.dylib")

	_, err := OpenNativeRuntime(context.Background(), NativeRuntimeOptions{
		LibraryPath: missing,
	})

	if !IsCode(err, ErrTransport) {
		t.Fatalf("OpenNativeRuntime error = %v, want %s", err, ErrTransport)
	}
}

func TestConnectNativeRuntimeAttachesByDefault(t *testing.T) {
	transport := &nativeRuntimeTestDaemonTransport{}

	runtime, health, closeFn, err := connectNativeRuntime(context.Background(), transport, NativeRuntimeOptions{
		ControlPath: "/run/easynet/control.sock",
	})
	if err != nil {
		t.Fatalf("connect native runtime: %v", err)
	}
	if runtime == nil || health == nil || closeFn == nil {
		t.Fatalf("connect returned incomplete clients: runtime=%v health=%v closeFnNil=%t", runtime, health, closeFn == nil)
	}
	if transport.startCalls != 0 || transport.discoverCalls != 1 || transport.attachCalls != 1 || transport.detachCalls != 1 {
		t.Fatalf("unexpected lifecycle calls: %#v", transport)
	}
}

func TestConnectNativeRuntimeStartsWhenConfigured(t *testing.T) {
	transport := &nativeRuntimeTestDaemonTransport{}

	runtime, health, closeFn, err := connectNativeRuntime(context.Background(), transport, NativeRuntimeOptions{
		StartConfig: &StartConfig{
			Mode:  ModeHub,
			Realm: "example.com",
		},
	})
	if err != nil {
		t.Fatalf("connect native runtime: %v", err)
	}
	if runtime == nil || health == nil || closeFn == nil {
		t.Fatalf("connect returned incomplete clients: runtime=%v health=%v closeFnNil=%t", runtime, health, closeFn == nil)
	}
	if transport.startCalls != 1 || transport.discoverCalls != 0 || transport.attachCalls != 0 || transport.detachCalls != 1 {
		t.Fatalf("unexpected lifecycle calls: %#v", transport)
	}
	if transport.startConfig["mode"] != "hub" || transport.startConfig["realm"] != "example.com" {
		t.Fatalf("unexpected start config: %v", transport.startConfig)
	}
}

func TestConnectNativeRuntimeStopOnCloseKeepsStartedHandle(t *testing.T) {
	transport := &nativeRuntimeTestDaemonTransport{}

	runtime, _, closeFn, err := connectNativeRuntime(context.Background(), transport, NativeRuntimeOptions{
		StartConfig:       &StartConfig{Mode: ModeHub},
		StopDaemonOnClose: true,
	})
	if err != nil {
		t.Fatalf("connect native runtime: %v", err)
	}
	if transport.detachCalls != 0 {
		t.Fatalf("started handle should remain attached for stop-on-close, detach calls=%d", transport.detachCalls)
	}
	if err := runtime.Close(context.Background()); err != nil {
		t.Fatalf("close runtime: %v", err)
	}
	if err := closeFn(context.Background()); err != nil {
		t.Fatalf("close lifecycle: %v", err)
	}
	if transport.stopCalls != 1 || transport.closeCalls != 0 {
		t.Fatalf("unexpected close lifecycle calls: %#v", transport)
	}
}

type nativeRuntimeTestDaemonTransport struct {
	discoverCalls int
	startCalls    int
	attachCalls   int
	openCalls     int
	stopCalls     int
	detachCalls   int
	closeCalls    int
	startConfig   map[string]any
}

func (t *nativeRuntimeTestDaemonTransport) Discover(context.Context, []byte) ([]byte, error) {
	t.discoverCalls++
	return []byte(`{"control_endpoint":"unix:///run/easynet/control.sock","invocation_endpoint":"unix:///run/easynet/daemon.sock"}`), nil
}

func (t *nativeRuntimeTestDaemonTransport) Start(_ context.Context, configJSON []byte) ([]byte, error) {
	t.startCalls++
	_ = json.Unmarshal(configJSON, &t.startConfig)
	return nativeRuntimeTestStatusJSON("started-handle"), nil
}

func (t *nativeRuntimeTestDaemonTransport) Attach(context.Context, []byte) ([]byte, error) {
	t.attachCalls++
	return nativeRuntimeTestStatusJSON("attached-handle"), nil
}

func (t *nativeRuntimeTestDaemonTransport) Status(context.Context, string) ([]byte, error) {
	return nativeRuntimeTestStatusJSON("status-handle"), nil
}

func (t *nativeRuntimeTestDaemonTransport) OpenRuntime(context.Context, string, []byte) (RuntimeTransport, []byte, error) {
	t.openCalls++
	return nativeRuntimeTestRuntimeTransport{}, nil, nil
}

func (t *nativeRuntimeTestDaemonTransport) Stop(context.Context, string, []byte) ([]byte, error) {
	t.stopCalls++
	return []byte(`{"state":"Stopped"}`), nil
}

func (t *nativeRuntimeTestDaemonTransport) Detach(context.Context, string) error {
	t.detachCalls++
	return nil
}

func (t *nativeRuntimeTestDaemonTransport) Close(context.Context) error {
	t.closeCalls++
	return nil
}

type nativeRuntimeTestRuntimeTransport struct {
	RuntimeTransportFunc
}

func (nativeRuntimeTestRuntimeTransport) RuntimeHealth(context.Context) ([]byte, error) {
	return []byte(`{"runtime_ready":true}`), nil
}

func nativeRuntimeTestStatusJSON(handleID string) []byte {
	return []byte(`{"handle_id":"` + handleID + `","state":"Running","mode":"hub","endpoints":{"control_endpoint":"unix:///run/easynet/control.sock","invocation_endpoint":"unix:///run/easynet/daemon.sock"}}`)
}
