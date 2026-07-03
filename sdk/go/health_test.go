package easynet

import (
	"context"
	"errors"
	"testing"
)

func TestRuntimeHealthDecodesReadyFixture(t *testing.T) {
	client, err := NewHealthClient(HealthTransportFunc(func(ctx context.Context) ([]byte, error) {
		return []byte(`{
			"api_ready": true,
			"daemon_ready": true,
			"invocation_ready": true,
			"directory_ready": true,
			"trust_ready": true,
			"runtime_ready": true,
			"version": "0.1.0",
			"abi_version": 4,
			"mismatch": null,
			"diagnostics": []
		}`), nil
	}))
	if err != nil {
		t.Fatalf("NewHealthClient: %v", err)
	}

	health, err := client.RuntimeHealth(context.Background())
	if err != nil {
		t.Fatalf("RuntimeHealth: %v", err)
	}

	if !health.APIAlive() {
		t.Fatalf("APIAlive = false, want true")
	}
	if !health.Ready() {
		t.Fatalf("Ready = false, want true")
	}
	if health.ABIVersion == nil || *health.ABIVersion != 4 {
		t.Fatalf("ABI version = %v, want 4", health.ABIVersion)
	}
}

func TestRuntimeHealthExposesControlOnlyState(t *testing.T) {
	client, err := NewHealthClient(HealthTransportFunc(func(ctx context.Context) ([]byte, error) {
		return []byte(`{
			"api_ready": true,
			"daemon_ready": true,
			"invocation_ready": false,
			"directory_ready": true,
			"trust_ready": true,
			"runtime_ready": false,
			"diagnostics": ["invocation endpoint unavailable"]
		}`), nil
	}))
	if err != nil {
		t.Fatalf("NewHealthClient: %v", err)
	}

	health, err := client.RuntimeHealth(context.Background())
	if err != nil {
		t.Fatalf("RuntimeHealth: %v", err)
	}

	if !health.APIAlive() {
		t.Fatalf("APIAlive = false, want true")
	}
	if health.Ready() {
		t.Fatalf("Ready = true, want false")
	}
	if health.InvocationReady {
		t.Fatalf("InvocationReady = true, want false")
	}
}

func TestRuntimeHealthWrapsTransportFailure(t *testing.T) {
	down := errors.New("daemon unavailable")
	client, err := NewHealthClient(HealthTransportFunc(func(ctx context.Context) ([]byte, error) {
		return nil, down
	}))
	if err != nil {
		t.Fatalf("NewHealthClient: %v", err)
	}

	_, err = client.RuntimeHealth(context.Background())
	if err == nil {
		t.Fatalf("RuntimeHealth succeeded, want transport error")
	}
	if !IsCode(err, ErrorTransport) {
		t.Fatalf("error code = %v, want %s", err, ErrorTransport)
	}
	if !errors.Is(err, down) {
		t.Fatalf("transport cause not preserved")
	}
}

func TestRuntimeHealthRejectsMalformedPayload(t *testing.T) {
	client, err := NewHealthClient(HealthTransportFunc(func(ctx context.Context) ([]byte, error) {
		return []byte(`{"api_ready": true, "runtime_ready": false}`), nil
	}))
	if err != nil {
		t.Fatalf("NewHealthClient: %v", err)
	}

	_, err = client.RuntimeHealth(context.Background())
	if err == nil {
		t.Fatalf("RuntimeHealth succeeded, want invalid argument")
	}
	if !IsCode(err, ErrorInvalidArgument) {
		t.Fatalf("error code = %v, want %s", err, ErrorInvalidArgument)
	}
}
