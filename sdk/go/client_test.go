package easynet

import (
	"context"
	"errors"
	"testing"
)

func TestFeatureDiscoveryDecodesRuntimeCoreFacts(t *testing.T) {
	client, err := NewClient(DiscoveryTransportFunc(func(ctx context.Context) ([]byte, error) {
		return []byte(`{
			"abi_version": 4,
			"sdk_version": "0.91.30",
			"profiles": {"runtime_core": "partial"},
			"symbols": {"runtime_health": true},
			"axon_pb": true
		}`), nil
	}))
	if err != nil {
		t.Fatalf("NewClient: %v", err)
	}

	features, err := client.FeatureDiscovery(context.Background())
	if err != nil {
		t.Fatalf("FeatureDiscovery: %v", err)
	}

	if features.Version().ABIVersion != 4 {
		t.Fatalf("ABI version = %d, want 4", features.Version().ABIVersion)
	}
	if !features.Symbols["runtime_health"] {
		t.Fatalf("runtime_health symbol not projected")
	}
}

func TestRequireABIReturnsTypedVersionMismatch(t *testing.T) {
	client, err := NewClient(DiscoveryTransportFunc(func(ctx context.Context) ([]byte, error) {
		return []byte(`{"abi_version": 3, "sdk_version": "0.91.30"}`), nil
	}))
	if err != nil {
		t.Fatalf("NewClient: %v", err)
	}

	_, err = client.RequireABI(context.Background(), 4)
	if err == nil {
		t.Fatalf("RequireABI succeeded, want version error")
	}
	if !IsCode(err, ErrorVersionIncompatible) {
		t.Fatalf("error code = %v, want %s", err, ErrorVersionIncompatible)
	}
}

func TestFeatureDiscoveryWrapsTransportFailure(t *testing.T) {
	down := errors.New("daemon unavailable")
	client, err := NewClient(DiscoveryTransportFunc(func(ctx context.Context) ([]byte, error) {
		return nil, down
	}))
	if err != nil {
		t.Fatalf("NewClient: %v", err)
	}

	_, err = client.FeatureDiscovery(context.Background())
	if err == nil {
		t.Fatalf("FeatureDiscovery succeeded, want transport error")
	}
	if !IsCode(err, ErrorTransport) {
		t.Fatalf("error code = %v, want %s", err, ErrorTransport)
	}
	if !errors.Is(err, down) {
		t.Fatalf("transport cause not preserved")
	}
}

func TestFeatureDiscoveryRejectsMalformedJSON(t *testing.T) {
	client, err := NewClient(DiscoveryTransportFunc(func(ctx context.Context) ([]byte, error) {
		return []byte(`{"abi_version": 0}`), nil
	}))
	if err != nil {
		t.Fatalf("NewClient: %v", err)
	}

	_, err = client.FeatureDiscovery(context.Background())
	if err == nil {
		t.Fatalf("FeatureDiscovery succeeded, want invalid argument")
	}
	if !IsCode(err, ErrorInvalidArgument) {
		t.Fatalf("error code = %v, want %s", err, ErrorInvalidArgument)
	}
}
