// Package runtimeprovider binds provider-owned runtime host policy to the
// canonical runtime lifecycle model.
package runtimeprovider

import (
	"context"
	"encoding/json"
	"errors"
	"strings"

	runtimesdk "easynet.run/cli/sdk/go"
)

// Mode is the runtime host deployment role understood by the host provider.
type Mode string

const (
	ModeEdge      Mode = "device"
	ModeAuthority Mode = "hub"
	ModeCombined  Mode = "both"
)

// RuntimeHostStartConfig is the provider-owned runtime host start policy.
type RuntimeHostStartConfig struct {
	Mode              Mode              `json:"mode"`
	Realm             string            `json:"realm,omitempty"`
	RuntimeInstanceID string            `json:"runtime_instance_id,omitempty"`
	HomeDir           string            `json:"home_dir,omitempty"`
	RuntimeBin        string            `json:"runtime_bin,omitempty"`
	WorkingDir        string            `json:"working_dir,omitempty"`
	LogPath           string            `json:"log_path,omitempty"`
	Detached          bool              `json:"detached,omitempty"`
	Env               map[string]string `json:"env,omitempty"`
	UDSPath           string            `json:"uds_path,omitempty"`
	ListenTCP         string            `json:"listen_tcp,omitempty"`
	TLSCertPath       string            `json:"tls_cert_path,omitempty"`
	TLSKeyPath        string            `json:"tls_key_path,omitempty"`
	AuthorityEndpoint string            `json:"authority_endpoint,omitempty"`
	TrustPath         string            `json:"trust_path,omitempty"`
}

// RuntimeHostAttachOptions identifies an existing runtime host.
type RuntimeHostAttachOptions = runtimesdk.RuntimeHostAttachOptions

// RuntimeHostDiscoverOptions controls runtime host discovery.
type RuntimeHostDiscoverOptions struct {
	ControlEndpoint string `json:"control_endpoint,omitempty"`
	ControlPath     string `json:"control_path,omitempty"`
	HomeDir         string `json:"home_dir,omitempty"`
}

// RuntimeHostStopOptions controls runtime host shutdown.
type RuntimeHostStopOptions = runtimesdk.RuntimeHostStopOptions

// ValidationError reports invalid runtime provider policy before transport use.
type ValidationError struct {
	message string
}

func (e *ValidationError) Error() string {
	if e == nil {
		return ""
	}
	return e.message
}

// Validate enforces runtime host deployment policy before transport use.
func (c RuntimeHostStartConfig) Validate() error {
	switch c.Mode {
	case ModeEdge:
		if strings.TrimSpace(c.ListenTCP) != "" {
			return &ValidationError{message: "edge runtime host mode must not accept a public TCP listener"}
		}
	case ModeAuthority, ModeCombined:
		if strings.TrimSpace(c.ListenTCP) != "" &&
			(strings.TrimSpace(c.TLSCertPath) == "" || strings.TrimSpace(c.TLSKeyPath) == "") {
			return &ValidationError{message: "public TCP listener requires TLS material"}
		}
	default:
		return &ValidationError{message: "mode must be edge, authority, or combined"}
	}
	return nil
}

// RuntimeHostStartPayload lowers validated runtime process policy for the
// canonical runtime-host lifecycle transport.
func (c RuntimeHostStartConfig) RuntimeHostStartPayload() ([]byte, error) {
	return json.Marshal(c)
}

// RuntimeHostDiscoverPayload lowers runtime host discovery policy.
func (o RuntimeHostDiscoverOptions) RuntimeHostDiscoverPayload() ([]byte, error) {
	return json.Marshal(o)
}

// IsValidationError reports whether an error came from runtime provider policy.
func IsValidationError(err error) bool {
	var target *ValidationError
	return errors.As(err, &target)
}

// Lifecycle is the runtime provider facade over one canonical RuntimeHost.
// It owns no lifecycle state and performs no implicit provider selection.
type Lifecycle struct {
	host *runtimesdk.RuntimeHost
}

// NewLifecycle binds an explicit runtime lifecycle transport to the canonical host.
func NewLifecycle(transport runtimesdk.RuntimeLifecycleTransport) (*Lifecycle, error) {
	host, err := runtimesdk.NewRuntimeHost(transport)
	if err != nil {
		return nil, err
	}
	return &Lifecycle{host: host}, nil
}

func (l *Lifecycle) Discover(ctx context.Context, opts RuntimeHostDiscoverOptions) (runtimesdk.RuntimeHostEndpoints, error) {
	return l.host.DiscoverRuntime(ctx, opts)
}

func (l *Lifecycle) Start(ctx context.Context, cfg RuntimeHostStartConfig) (*runtimesdk.RuntimeHandle, error) {
	return l.host.StartRuntime(ctx, cfg)
}

func (l *Lifecycle) Attach(ctx context.Context, opts RuntimeHostAttachOptions) (*runtimesdk.RuntimeHandle, error) {
	return l.host.AttachRuntime(ctx, opts)
}

func (l *Lifecycle) ConnectLocal(ctx context.Context, opts runtimesdk.ConnectOptions) (*runtimesdk.RuntimeClient, error) {
	return l.host.ConnectLocal(ctx, opts)
}
