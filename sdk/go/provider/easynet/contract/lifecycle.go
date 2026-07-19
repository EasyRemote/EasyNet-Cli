// Package contract owns the EasyNet provider's daemon lifecycle wire contract.
//
// Runtime lifecycle state and handle ownership remain in the canonical SDK.
package contract

import (
	"encoding/json"
	"errors"
	"strings"
)

// Mode is the easynet-daemon deployment role.
type Mode string

const (
	ModeDevice Mode = "device"
	ModeHub    Mode = "hub"
	ModeBoth   Mode = "both"
)

// StartConfig is the easynet-daemon process start policy.
type StartConfig struct {
	Mode        Mode              `json:"mode"`
	Realm       string            `json:"realm,omitempty"`
	DeviceID    string            `json:"device_id,omitempty"`
	HomeDir     string            `json:"home_dir,omitempty"`
	DaemonBin   string            `json:"daemon_bin,omitempty"`
	WorkingDir  string            `json:"working_dir,omitempty"`
	LogPath     string            `json:"log_path,omitempty"`
	Detached    bool              `json:"detached,omitempty"`
	Env         map[string]string `json:"env,omitempty"`
	UDSPath     string            `json:"uds_path,omitempty"`
	ListenTCP   string            `json:"listen_tcp,omitempty"`
	TLSCertPath string            `json:"tls_cert_path,omitempty"`
	TLSKeyPath  string            `json:"tls_key_path,omitempty"`
	HubEndpoint string            `json:"hub_endpoint,omitempty"`
	TrustPath   string            `json:"trust_path,omitempty"`
}

// AttachOptions identifies an existing easynet-daemon process.
type AttachOptions struct {
	ControlEndpoint    string `json:"control_endpoint,omitempty"`
	InvocationEndpoint string `json:"invocation_endpoint,omitempty"`
	ControlPath        string `json:"control_path,omitempty"`
}

// DiscoverOptions controls easynet-daemon discovery.
type DiscoverOptions struct {
	ControlEndpoint string `json:"control_endpoint,omitempty"`
	ControlPath     string `json:"control_path,omitempty"`
	HomeDir         string `json:"home_dir,omitempty"`
}

// StopOptions controls easynet-daemon shutdown.
type StopOptions struct {
	GracefulTimeoutMS int64 `json:"graceful_timeout_ms,omitempty"`
	Force             bool  `json:"force,omitempty"`
}

// ValidationError reports invalid EasyNet provider policy before transport use.
type ValidationError struct {
	message string
}

func (e *ValidationError) Error() string {
	if e == nil {
		return ""
	}
	return e.message
}

// Validate enforces easynet-daemon deployment policy.
func (c StartConfig) Validate() error {
	switch c.Mode {
	case ModeDevice:
		if strings.TrimSpace(c.ListenTCP) != "" {
			return &ValidationError{message: "device mode must not accept a public TCP listener"}
		}
	case ModeHub, ModeBoth:
		if strings.TrimSpace(c.ListenTCP) != "" &&
			(strings.TrimSpace(c.TLSCertPath) == "" || strings.TrimSpace(c.TLSKeyPath) == "") {
			return &ValidationError{message: "public TCP listener requires TLS material"}
		}
	default:
		return &ValidationError{message: "mode must be device, hub, or both"}
	}
	return nil
}

// RuntimeHostStartPayload lowers validated EasyNet process policy for the
// canonical runtime-host lifecycle transport.
func (c StartConfig) RuntimeHostStartPayload() ([]byte, error) {
	return json.Marshal(c)
}

// RuntimeHostDiscoverPayload lowers EasyNet directory policy for canonical
// endpoint discovery.
func (o DiscoverOptions) RuntimeHostDiscoverPayload() ([]byte, error) {
	return json.Marshal(o)
}

// IsValidationError reports whether an error came from EasyNet provider policy.
func IsValidationError(err error) bool {
	var target *ValidationError
	return errors.As(err, &target)
}
