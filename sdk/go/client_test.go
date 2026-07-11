package easynet

import (
	"context"
	"errors"
	"testing"
)

type memoryDiscoveryTransport struct {
	payload      []byte
	featureCalls int
	closeCalls   int
	featureErr   error
	closeErr     error
}

func (m *memoryDiscoveryTransport) FeatureDiscovery(ctx context.Context) ([]byte, error) {
	m.featureCalls++
	if m.featureErr != nil {
		return nil, m.featureErr
	}
	return m.payload, nil
}

func (m *memoryDiscoveryTransport) Close(ctx context.Context) error {
	m.closeCalls++
	return m.closeErr
}

func TestFeatureDiscoveryDecodesRuntimeCoreFacts(t *testing.T) {
	client, err := NewClient(DiscoveryTransportFunc(func(ctx context.Context) ([]byte, error) {
		return []byte(`{
			"abi_version": 5,
			"sdk_version": "0.91.30",
			"profiles": {"runtime_core": "provider-backed"},
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

	if features.Version().ABIVersion != 5 {
		t.Fatalf("ABI version = %d, want 5", features.Version().ABIVersion)
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

	_, err = client.RequireABI(context.Background(), 5)
	if err == nil {
		t.Fatalf("RequireABI succeeded, want version error")
	}
	var sdkErr *SDKError
	if !errors.As(err, &sdkErr) || sdkErr.Code != ErrVersionMismatch {
		t.Fatalf("error code = %v, want %s", err, ErrVersionMismatch)
	}
	if !IsCode(err, ErrVersionMismatch) {
		t.Fatalf("canonical version-mismatch request did not match error")
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
	if !IsCode(err, ErrTransport) {
		t.Fatalf("error code = %v, want %s", err, ErrTransport)
	}
	if !errors.Is(err, down) {
		t.Fatalf("transport cause not preserved")
	}
}

func TestRequireABIMapsZeroDaemonABIToVersionMismatch(t *testing.T) {
	client, err := NewClient(DiscoveryTransportFunc(func(ctx context.Context) ([]byte, error) {
		return []byte(`{"abi_version": 0, "sdk_version": "0.91.30"}`), nil
	}))
	if err != nil {
		t.Fatalf("NewClient: %v", err)
	}

	_, err = client.RequireABI(context.Background(), 5)
	if err == nil {
		t.Fatalf("RequireABI succeeded, want version error")
	}
	var sdkErr *SDKError
	if !errors.As(err, &sdkErr) || sdkErr.Code != ErrVersionMismatch {
		t.Fatalf("error code = %v, want %s", err, ErrVersionMismatch)
	}
	if !IsCode(err, ErrVersionMismatch) {
		t.Fatalf("canonical version-mismatch request did not match error")
	}
}

func TestFeatureDiscoveryRejectsMalformedJSON(t *testing.T) {
	client, err := NewClient(DiscoveryTransportFunc(func(ctx context.Context) ([]byte, error) {
		return []byte(`{"abi_version": true}`), nil
	}))
	if err != nil {
		t.Fatalf("NewClient: %v", err)
	}

	_, err = client.FeatureDiscovery(context.Background())
	if err == nil {
		t.Fatalf("FeatureDiscovery succeeded, want invalid argument")
	}
	if !IsCode(err, ErrInvalidArgument) {
		t.Fatalf("error code = %v, want %s", err, ErrInvalidArgument)
	}
}

func TestClientCloseDelegatesOnceAndFailsClosed(t *testing.T) {
	transport := &memoryDiscoveryTransport{payload: []byte(`{"abi_version": 5, "sdk_version": "0.91.30"}`)}
	client, err := NewClient(transport)
	if err != nil {
		t.Fatalf("NewClient: %v", err)
	}

	if err := client.Close(context.Background()); err != nil {
		t.Fatalf("Close: %v", err)
	}
	if err := client.Close(context.Background()); err != nil {
		t.Fatalf("second Close: %v", err)
	}
	if transport.closeCalls != 1 {
		t.Fatalf("close calls = %d, want 1", transport.closeCalls)
	}
	_, err = client.FeatureDiscovery(context.Background())
	if err == nil {
		t.Fatalf("FeatureDiscovery after close succeeded")
	}
	if !IsCode(err, ErrInvalidArgument) {
		t.Fatalf("error code = %v, want %s", err, ErrInvalidArgument)
	}
	if transport.featureCalls != 0 {
		t.Fatalf("feature discovery reached transport after close: %d calls", transport.featureCalls)
	}
}

func TestClientCloseFailureIsTerminal(t *testing.T) {
	down := errors.New("close failed")
	transport := &memoryDiscoveryTransport{
		payload:  []byte(`{"abi_version": 5, "sdk_version": "0.91.30"}`),
		closeErr: down,
	}
	client, err := NewClient(transport)
	if err != nil {
		t.Fatalf("NewClient: %v", err)
	}

	err = client.Close(context.Background())
	if err == nil {
		t.Fatalf("Close succeeded, want transport error")
	}
	if !IsCode(err, ErrTransport) || !errors.Is(err, down) {
		t.Fatalf("close error not wrapped as transport cause: %v", err)
	}
	_, err = client.RequireABI(context.Background(), 5)
	if err == nil {
		t.Fatalf("RequireABI after failed close succeeded")
	}
	if !IsCode(err, ErrInvalidArgument) {
		t.Fatalf("error code = %v, want %s", err, ErrInvalidArgument)
	}
	if transport.featureCalls != 0 {
		t.Fatalf("require ABI reached transport after failed close: %d calls", transport.featureCalls)
	}
}
