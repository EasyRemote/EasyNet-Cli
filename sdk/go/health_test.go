package easynet

import (
	"context"
	"errors"
	"testing"
)

type staticHealthTransport struct {
	health      []byte
	diagnostics []byte
}

func (s staticHealthTransport) RuntimeHealth(ctx context.Context) ([]byte, error) {
	return s.health, nil
}

func (s staticHealthTransport) RuntimeDiagnostics(ctx context.Context) ([]byte, error) {
	return s.diagnostics, nil
}

func TestRuntimeHealthDecodesReadyFixture(t *testing.T) {
	client, err := NewHealthClient(HealthTransportFunc(func(ctx context.Context) ([]byte, error) {
		return []byte(`{
			"api_ready": true,
			"invocation_ready": true,
			"directory_ready": true,
			"trust_ready": true,
			"runtime_ready": true,
			"version": "0.1.0",
			"abi_version": 5,
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
	if health.ABIVersion == nil || *health.ABIVersion != 5 {
		t.Fatalf("ABI version = %v, want 5", health.ABIVersion)
	}
}

func TestRuntimeDiagnosticsDecodesReportFixture(t *testing.T) {
	client, err := NewHealthClient(staticHealthTransport{
		health: []byte(`{
			"api_ready": true,
			"invocation_ready": true,
			"directory_ready": true,
			"trust_ready": true,
			"runtime_ready": true,
			"diagnostics": []
		}`),
		diagnostics: []byte(`{
			"profile": "health",
			"kind": "diagnostics_report",
			"state": "Running",
			"ready": true,
			"version": "0.91.30",
			"abi_version": 5,
			"control_endpoint": "/tmp/easynet/control.json",
			"invocation_endpoint": "/tmp/easynet/daemon.sock",
			"checks": [{"name": "runtime", "ready": true, "message": null}],
			"diagnostics": []
		}`),
	})
	if err != nil {
		t.Fatalf("NewHealthClient: %v", err)
	}

	report, err := client.Diagnostics(context.Background())
	if err != nil {
		t.Fatalf("Diagnostics: %v", err)
	}

	if !report.Ready || report.Kind != "diagnostics_report" || len(report.Checks) != 1 {
		t.Fatalf("unexpected diagnostics report: %#v", report)
	}
}

func TestRuntimeDiagnosticsRequiresTransportCapability(t *testing.T) {
	client, err := NewHealthClient(HealthTransportFunc(func(ctx context.Context) ([]byte, error) {
		return []byte(`{
			"api_ready": true,
			"invocation_ready": true,
			"directory_ready": true,
			"trust_ready": true,
			"runtime_ready": true,
			"diagnostics": []
		}`), nil
	}))
	if err != nil {
		t.Fatalf("NewHealthClient: %v", err)
	}

	_, err = client.Diagnostics(context.Background())
	if err == nil {
		t.Fatalf("Diagnostics succeeded without diagnostics transport")
	}
	if !IsCode(err, ErrNotImplemented) {
		t.Fatalf("error code = %v, want %s", err, ErrNotImplemented)
	}
}

func TestRuntimeHealthExposesControlOnlyState(t *testing.T) {
	client, err := NewHealthClient(HealthTransportFunc(func(ctx context.Context) ([]byte, error) {
		return []byte(`{
			"api_ready": true,
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
	if !IsCode(err, ErrTransport) {
		t.Fatalf("error code = %v, want %s", err, ErrTransport)
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
	if !IsCode(err, ErrInvalidArgument) {
		t.Fatalf("error code = %v, want %s", err, ErrInvalidArgument)
	}
}
