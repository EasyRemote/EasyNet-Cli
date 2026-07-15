package easynet

import (
	"context"
	"encoding/json"
	"fmt"
	"os"
	"path/filepath"
	"strings"
)

const runtimeCredentialsFilename = "credentials.json"

// RuntimeIdentityProjection is the public local runtime identity projection.
// It contains only routable public facts needed by SDK consumers; private-key
// material and key-service endpoints are deliberately excluded.
type RuntimeIdentityProjection struct {
	Realm       string `json:"realm"`
	DeviceID    string `json:"device_id"`
	Username    string `json:"username,omitempty"`
	HubEndpoint string `json:"hub_endpoint,omitempty"`
}

// RuntimeStateRoot resolves the SDK-owned local runtime state directory.
func RuntimeStateRoot(controlPath string) (string, error) {
	resolved, err := ResolveControlDiscoveryPath(controlPath)
	if err != nil {
		return "", err
	}
	return filepath.Dir(resolved), nil
}

// RuntimeCredentialsPath resolves the paired runtime identity projection path.
func RuntimeCredentialsPath(controlPath string) (string, error) {
	root, err := RuntimeStateRoot(controlPath)
	if err != nil {
		return "", err
	}
	return filepath.Join(root, runtimeCredentialsFilename), nil
}

// ReadRuntimeIdentityProjection reads and validates the paired runtime identity
// projection. If credentialsPath is empty it is derived from controlPath.
func ReadRuntimeIdentityProjection(ctx context.Context, credentialsPath string, controlPath string) (RuntimeIdentityProjection, error) {
	if ctx == nil {
		return RuntimeIdentityProjection{}, invalidRuntimeEnvironment("context is required", nil)
	}
	select {
	case <-ctx.Done():
		return RuntimeIdentityProjection{}, transportRuntimeError("read runtime identity projection cancelled", ctx.Err())
	default:
	}
	path := strings.TrimSpace(credentialsPath)
	if path == "" {
		resolved, err := RuntimeCredentialsPath(controlPath)
		if err != nil {
			return RuntimeIdentityProjection{}, err
		}
		path = resolved
	}
	raw, err := os.ReadFile(path)
	if err != nil {
		return RuntimeIdentityProjection{}, &SDKError{
			Code:      ErrDaemonOffline,
			Stage:     "runtime_environment",
			Retry:     RetrySafe,
			Retryable: RetryableForHint(RetrySafe),
			Message:   fmt.Sprintf("runtime identity projection not readable at %s", path),
			Details:   map[string]any{"credentials_path": path},
			Cause:     err,
		}
	}
	projection, err := NewRuntimeIdentityProjectionFromJSON(raw)
	if err != nil {
		return RuntimeIdentityProjection{}, err
	}
	return projection, nil
}

// NewRuntimeIdentityProjectionFromJSON decodes a credentials projection.
func NewRuntimeIdentityProjectionFromJSON(raw []byte) (RuntimeIdentityProjection, error) {
	var decoded map[string]any
	if err := json.Unmarshal(raw, &decoded); err != nil {
		return RuntimeIdentityProjection{}, invalidRuntimeEnvironment(fmt.Sprintf("decode runtime identity projection JSON: %v", err), err)
	}
	if decoded == nil {
		return RuntimeIdentityProjection{}, invalidRuntimeEnvironment("runtime identity projection must be a JSON object", nil)
	}
	projection := RuntimeIdentityProjection{
		Realm:       runtimeEnvironmentString(decoded, "realm"),
		DeviceID:    runtimeEnvironmentString(decoded, "device_id"),
		Username:    runtimeEnvironmentString(decoded, "username"),
		HubEndpoint: runtimeEnvironmentString(decoded, "hub_endpoint"),
	}
	if projection.Realm == "" {
		return RuntimeIdentityProjection{}, invalidRuntimeEnvironment("runtime identity projection missing realm", nil)
	}
	if projection.DeviceID == "" {
		return RuntimeIdentityProjection{}, invalidRuntimeEnvironment("runtime identity projection missing device_id", nil)
	}
	return projection, nil
}

// RuntimeStateRoot resolves the environment's runtime state root.
func (e *SdkEnvironment) RuntimeStateRoot() (string, error) {
	if e == nil {
		return "", invalidRuntimeEnvironment("sdk environment is not initialized", nil)
	}
	return RuntimeStateRoot(e.options.Discover.ControlPath)
}

// RuntimeCredentialsPath resolves the environment's credentials projection.
func (e *SdkEnvironment) RuntimeCredentialsPath() (string, error) {
	if e == nil {
		return "", invalidRuntimeEnvironment("sdk environment is not initialized", nil)
	}
	return RuntimeCredentialsPath(e.options.Discover.ControlPath)
}

// ReadRuntimeIdentityProjection reads the environment's paired identity
// projection unless an explicit credentialsPath is supplied.
func (e *SdkEnvironment) ReadRuntimeIdentityProjection(ctx context.Context, credentialsPath string) (RuntimeIdentityProjection, error) {
	if e == nil {
		return RuntimeIdentityProjection{}, invalidRuntimeEnvironment("sdk environment is not initialized", nil)
	}
	return ReadRuntimeIdentityProjection(ctx, credentialsPath, e.options.Discover.ControlPath)
}

func runtimeEnvironmentString(raw map[string]any, key string) string {
	value, ok := raw[key]
	if !ok || value == nil {
		return ""
	}
	if text, ok := value.(string); ok {
		return strings.TrimSpace(text)
	}
	return strings.TrimSpace(fmt.Sprint(value))
}

func invalidRuntimeEnvironment(message string, cause error) error {
	return &SDKError{
		Code:      ErrInvalidArgument,
		Stage:     "runtime_environment",
		Retry:     RetryNever,
		Retryable: false,
		Message:   message,
		Cause:     cause,
	}
}
