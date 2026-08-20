package easynet

import (
	"context"
	"testing"
)

type closeableDaemonTransport struct {
	*memoryDaemonTransport
	closeCalls int
}

func (t *closeableDaemonTransport) Close(context.Context) error {
	t.closeCalls++
	return nil
}

func TestSdkEnvironmentOwnsProcessRootAndConnectsRuntime(t *testing.T) {
	discovery := &memoryDiscoveryTransport{payload: []byte(`{
		"abi_version": 5,
		"sdk_version": "0.91.30",
		"profiles": {"runtime_core": "provider-backed"},
		"symbols": {"runtime_health": true}
	}`)}
	daemon := &closeableDaemonTransport{memoryDaemonTransport: &memoryDaemonTransport{
		discoverJSON: `{"control_endpoint":"/tmp/control.sock","invocation_endpoint":"/tmp/daemon.sock"}`,
		attachJSON:   readyDaemonStatus(),
	}}
	env, err := NewSdkEnvironment(discovery, daemon, SdkEnvironmentOptions{
		ExpectedABIVersion: 5,
		Discover:           RuntimeHostDiscoverOptions{ControlPath: "/tmp/control.sock"},
		Connect:            ConnectOptions{MaxMessageBytes: 4096},
	})
	if err != nil {
		t.Fatalf("NewSdkEnvironment: %v", err)
	}

	features, err := env.RequireABI(context.Background())
	if err != nil {
		t.Fatalf("RequireABI: %v", err)
	}
	if features.ABIVersion != 5 || discovery.featureCalls != 1 {
		t.Fatalf("unexpected feature discovery: %#v calls=%d", features, discovery.featureCalls)
	}

	endpoints, err := env.DiscoverRuntime(context.Background(), RuntimeHostDiscoverOptions{})
	if err != nil {
		t.Fatalf("DiscoverRuntime: %v", err)
	}
	if endpoints.InvocationEndpoint != "/tmp/daemon.sock" || daemon.seenOptions["control_path"] != "/tmp/control.sock" {
		t.Fatalf("unexpected daemon discovery endpoints=%#v options=%#v", endpoints, daemon.seenOptions)
	}

	runtime, err := env.ConnectLocal(context.Background(), ConnectOptions{InvokeTimeoutMS: 5000})
	if err != nil {
		t.Fatalf("ConnectLocal: %v", err)
	}
	if runtime == nil || daemon.openCalls != 1 || daemon.detachCalls != 1 {
		t.Fatalf("runtime=%#v openCalls=%d detachCalls=%d", runtime, daemon.openCalls, daemon.detachCalls)
	}
	if daemon.seenOptions["max_message_bytes"] != float64(4096) || daemon.seenOptions["invoke_timeout_ms"] != float64(5000) {
		t.Fatalf("connect defaults/overrides not projected: %#v", daemon.seenOptions)
	}

	if err := env.Close(context.Background()); err != nil {
		t.Fatalf("Close: %v", err)
	}
	if err := env.Close(context.Background()); err != nil {
		t.Fatalf("second Close: %v", err)
	}
	if discovery.closeCalls != 1 || daemon.closeCalls != 1 {
		t.Fatalf("close calls discovery=%d daemon=%d", discovery.closeCalls, daemon.closeCalls)
	}
	if _, err := env.FeatureDiscovery(context.Background()); !IsCode(err, ErrInvalidArgument) {
		t.Fatalf("FeatureDiscovery after Close = %v, want %s", err, ErrInvalidArgument)
	}
	if _, err := env.RuntimeHost(context.Background()); !IsCode(err, ErrInvalidArgument) {
		t.Fatalf("RuntimeHost after Close = %v, want %s", err, ErrInvalidArgument)
	}
}

func TestSdkEnvironmentRejectsMissingRequiredBoundaries(t *testing.T) {
	daemon := &memoryDaemonTransport{}
	if _, err := NewSdkEnvironment(nil, daemon, SdkEnvironmentOptions{}); !IsCode(err, ErrInvalidArgument) {
		t.Fatalf("nil discovery error = %v", err)
	}
	if _, err := NewSdkEnvironment(DiscoveryTransportFunc(func(context.Context) ([]byte, error) {
		return []byte(`{"abi_version":5}`), nil
	}), nil, SdkEnvironmentOptions{}); !IsCode(err, ErrInvalidArgument) {
		t.Fatalf("nil runtime error = %v", err)
	}
}

func TestSdkEnvironmentRequireABIRequiresConfiguredVersion(t *testing.T) {
	env, err := NewSdkEnvironment(
		DiscoveryTransportFunc(func(context.Context) ([]byte, error) { return []byte(`{"abi_version":5}`), nil }),
		&memoryDaemonTransport{},
		SdkEnvironmentOptions{},
	)
	if err != nil {
		t.Fatalf("NewSdkEnvironment: %v", err)
	}
	if _, err := env.RequireABI(context.Background()); !IsCode(err, ErrInvalidArgument) {
		t.Fatalf("RequireABI without expected version = %v, want %s", err, ErrInvalidArgument)
	}
}
