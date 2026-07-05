package easynet

import (
	"context"
	"encoding/json"
	"fmt"
)

// HealthTransport supplies Runtime Core health JSON.
//
// Implementations may call the daemon SDK boundary. Product code consumes this
// interface rather than Axon packages, generated protobufs, C ABI handles, or
// raw daemon sockets.
type HealthTransport interface {
	RuntimeHealth(ctx context.Context) ([]byte, error)
}

// DiagnosticsTransport supplies the Health profile diagnostics report.
type DiagnosticsTransport interface {
	RuntimeDiagnostics(ctx context.Context) ([]byte, error)
}

// HealthTransportFunc adapts a function into a HealthTransport.
type HealthTransportFunc func(ctx context.Context) ([]byte, error)

func (f HealthTransportFunc) RuntimeHealth(ctx context.Context) ([]byte, error) {
	return f(ctx)
}

// HealthClient is the Go Runtime Core health facade.
type HealthClient struct {
	transport HealthTransport
}

// NewHealthClient creates a health facade over a daemon health transport.
func NewHealthClient(transport HealthTransport) (*HealthClient, error) {
	if transport == nil {
		return nil, &SDKError{
			Code:      ErrorInvalidArgument,
			Stage:     "sdk",
			Retry:     RetryNever,
			Retryable: RetryableForHint(RetryNever),
			Message:   "health transport is required",
		}
	}
	return &HealthClient{transport: transport}, nil
}

// RuntimeHealth is the language-neutral SDK health DTO.
type RuntimeHealth struct {
	APIReady        bool           `json:"api_ready"`
	DaemonReady     bool           `json:"daemon_ready"`
	InvocationReady bool           `json:"invocation_ready"`
	DirectoryReady  bool           `json:"directory_ready"`
	TrustReady      bool           `json:"trust_ready"`
	RuntimeReady    bool           `json:"runtime_ready"`
	Version         *string        `json:"version,omitempty"`
	ABIVersion      *uint32        `json:"abi_version,omitempty"`
	Mismatch        map[string]any `json:"mismatch,omitempty"`
	Diagnostics     []string       `json:"diagnostics"`
}

// DiagnosticsReport is the language-neutral SDK diagnostics DTO.
type DiagnosticsReport struct {
	Profile            string            `json:"profile"`
	Kind               string            `json:"kind"`
	State              string            `json:"state"`
	Ready              bool              `json:"ready"`
	Version            string            `json:"version"`
	ABIVersion         uint32            `json:"abi_version"`
	ControlEndpoint    string            `json:"control_endpoint"`
	InvocationEndpoint *string           `json:"invocation_endpoint"`
	Checks             []DiagnosticCheck `json:"checks"`
	Diagnostics        []string          `json:"diagnostics"`
}

// DiagnosticCheck is one named readiness check in a diagnostics report.
type DiagnosticCheck struct {
	Name    string  `json:"name"`
	Ready   bool    `json:"ready"`
	Message *string `json:"message"`
}

// APIAlive reports process/API liveness, not full runtime readiness.
func (h RuntimeHealth) APIAlive() bool {
	return h.APIReady && h.DaemonReady
}

// Ready reports full runtime readiness.
func (h RuntimeHealth) Ready() bool {
	return h.RuntimeReady
}

// RuntimeHealth reads and decodes daemon runtime health.
func (c *HealthClient) RuntimeHealth(ctx context.Context) (RuntimeHealth, error) {
	if c == nil || c.transport == nil {
		return RuntimeHealth{}, &SDKError{
			Code:      ErrorInvalidArgument,
			Stage:     "sdk",
			Retry:     RetryNever,
			Retryable: RetryableForHint(RetryNever),
			Message:   "health client is not initialized",
		}
	}
	if ctx == nil {
		return RuntimeHealth{}, &SDKError{
			Code:      ErrorInvalidArgument,
			Stage:     "sdk",
			Retry:     RetryNever,
			Retryable: RetryableForHint(RetryNever),
			Message:   "context is required",
		}
	}
	raw, err := c.transport.RuntimeHealth(ctx)
	if err != nil {
		return RuntimeHealth{}, &SDKError{
			Code:      ErrRouteUnavailable,
			Stage:     "transport",
			Retry:     RetrySafe,
			Retryable: RetryableForHint(RetrySafe),
			Message:   "runtime health transport failed",
			Cause:     err,
		}
	}
	return decodeRuntimeHealth(raw)
}

// Diagnostics reads and decodes daemon runtime diagnostics.
func (c *HealthClient) Diagnostics(ctx context.Context) (DiagnosticsReport, error) {
	if c == nil || c.transport == nil {
		return DiagnosticsReport{}, &SDKError{
			Code:      ErrorInvalidArgument,
			Stage:     "sdk",
			Retry:     RetryNever,
			Retryable: RetryableForHint(RetryNever),
			Message:   "health client is not initialized",
		}
	}
	if ctx == nil {
		return DiagnosticsReport{}, &SDKError{
			Code:      ErrorInvalidArgument,
			Stage:     "sdk",
			Retry:     RetryNever,
			Retryable: RetryableForHint(RetryNever),
			Message:   "context is required",
		}
	}
	transport, ok := c.transport.(DiagnosticsTransport)
	if !ok {
		return DiagnosticsReport{}, &SDKError{
			Code:      ErrNotImplemented,
			Stage:     "transport",
			Retry:     RetryNever,
			Retryable: RetryableForHint(RetryNever),
			Message:   "health diagnostics transport is not available",
		}
	}
	raw, err := transport.RuntimeDiagnostics(ctx)
	if err != nil {
		return DiagnosticsReport{}, &SDKError{
			Code:      ErrRouteUnavailable,
			Stage:     "transport",
			Retry:     RetrySafe,
			Retryable: RetryableForHint(RetrySafe),
			Message:   "runtime diagnostics transport failed",
			Cause:     err,
		}
	}
	return decodeDiagnosticsReport(raw)
}

// NewRuntimeHealthFromJSON decodes the shared health.schema.json DTO.
func NewRuntimeHealthFromJSON(raw []byte) (RuntimeHealth, error) {
	return decodeRuntimeHealth(raw)
}

// NewDiagnosticsReportFromJSON decodes the shared diagnostics.schema.json DTO.
func NewDiagnosticsReportFromJSON(raw []byte) (DiagnosticsReport, error) {
	return decodeDiagnosticsReport(raw)
}

func decodeRuntimeHealth(raw []byte) (RuntimeHealth, error) {
	var fields map[string]json.RawMessage
	if err := json.Unmarshal(raw, &fields); err != nil {
		return RuntimeHealth{}, &SDKError{
			Code:      ErrorInvalidArgument,
			Stage:     "decode",
			Retry:     RetryNever,
			Retryable: RetryableForHint(RetryNever),
			Message:   fmt.Sprintf("decode runtime health JSON: %v", err),
			Cause:     err,
		}
	}
	apiReady, err := requiredHealthBool(fields, "api_ready")
	if err != nil {
		return RuntimeHealth{}, err
	}
	daemonReady, err := requiredHealthBool(fields, "daemon_ready")
	if err != nil {
		return RuntimeHealth{}, err
	}
	invocationReady, err := requiredHealthBool(fields, "invocation_ready")
	if err != nil {
		return RuntimeHealth{}, err
	}
	directoryReady, err := requiredHealthBool(fields, "directory_ready")
	if err != nil {
		return RuntimeHealth{}, err
	}
	trustReady, err := requiredHealthBool(fields, "trust_ready")
	if err != nil {
		return RuntimeHealth{}, err
	}
	runtimeReady, err := requiredHealthBool(fields, "runtime_ready")
	if err != nil {
		return RuntimeHealth{}, err
	}

	health := RuntimeHealth{
		APIReady:        apiReady,
		DaemonReady:     daemonReady,
		InvocationReady: invocationReady,
		DirectoryReady:  directoryReady,
		TrustReady:      trustReady,
		RuntimeReady:    runtimeReady,
		Diagnostics:     []string{},
	}
	if rawField, ok := fields["version"]; ok && string(rawField) != "null" {
		var value string
		if err := json.Unmarshal(rawField, &value); err != nil {
			return RuntimeHealth{}, invalidHealthField("version", "must be a string or null")
		}
		health.Version = &value
	}
	if rawField, ok := fields["abi_version"]; ok && string(rawField) != "null" {
		var value uint32
		if err := json.Unmarshal(rawField, &value); err != nil {
			return RuntimeHealth{}, invalidHealthField("abi_version", "must be an unsigned integer or null")
		}
		health.ABIVersion = &value
	}
	if rawField, ok := fields["mismatch"]; ok && string(rawField) != "null" {
		var value map[string]any
		if err := json.Unmarshal(rawField, &value); err != nil {
			return RuntimeHealth{}, invalidHealthField("mismatch", "must be an object or null")
		}
		health.Mismatch = value
	}
	if rawField, ok := fields["diagnostics"]; ok {
		var value []string
		if err := json.Unmarshal(rawField, &value); err != nil {
			return RuntimeHealth{}, invalidHealthField("diagnostics", "must be an array of strings")
		}
		health.Diagnostics = value
	}
	return health, nil
}

func decodeDiagnosticsReport(raw []byte) (DiagnosticsReport, error) {
	var report DiagnosticsReport
	if err := json.Unmarshal(raw, &report); err != nil {
		return DiagnosticsReport{}, &SDKError{
			Code:      ErrorInvalidArgument,
			Stage:     "decode",
			Retry:     RetryNever,
			Retryable: RetryableForHint(RetryNever),
			Message:   fmt.Sprintf("decode diagnostics JSON: %v", err),
			Cause:     err,
		}
	}
	if report.Profile != "health" {
		return DiagnosticsReport{}, invalidHealthField("profile", "must be health")
	}
	if report.Kind != "diagnostics_report" {
		return DiagnosticsReport{}, invalidHealthField("kind", "must be diagnostics_report")
	}
	if report.State == "" {
		return DiagnosticsReport{}, invalidHealthField("state", "is required")
	}
	if report.Version == "" {
		return DiagnosticsReport{}, invalidHealthField("version", "is required")
	}
	if report.Checks == nil || len(report.Checks) == 0 {
		return DiagnosticsReport{}, invalidHealthField("checks", "must be non-empty")
	}
	if report.Diagnostics == nil {
		report.Diagnostics = []string{}
	}
	for _, check := range report.Checks {
		if check.Name == "" {
			return DiagnosticsReport{}, invalidHealthField("checks.name", "is required")
		}
	}
	return report, nil
}

func requiredHealthBool(fields map[string]json.RawMessage, name string) (bool, error) {
	rawField, ok := fields[name]
	if !ok {
		return false, invalidHealthField(name, "is required")
	}
	var value bool
	if err := json.Unmarshal(rawField, &value); err != nil {
		return false, invalidHealthField(name, "must be a boolean")
	}
	return value, nil
}

func invalidHealthField(name string, message string) error {
	return &SDKError{
		Code:      ErrorInvalidArgument,
		Stage:     "decode",
		Retry:     RetryNever,
		Retryable: RetryableForHint(RetryNever),
		Message:   fmt.Sprintf("%s %s", name, message),
	}
}
