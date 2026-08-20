package easynet

import (
	"context"
	"encoding/json"
	"fmt"
	"os"
	"path/filepath"
	"sort"
	"strings"
)

const runtimeCredentialsFilename = "credentials.json"

var runtimeIdentityProjectionAllowedFields = map[string]struct{}{
	"realm":                  {},
	"runtime_instance_id":    {},
	"principal":              {},
	"control_plane_endpoint": {},
}

// RuntimeIdentityProjection is the public local runtime identity projection.
// It contains only routable public facts needed by SDK consumers; private-key
// material and key-service endpoints are deliberately excluded.
type RuntimeIdentityProjection struct {
	Realm                string `json:"realm"`
	RuntimeInstanceID    string `json:"runtime_instance_id"`
	Principal            string `json:"principal,omitempty"`
	ControlPlaneEndpoint string `json:"control_plane_endpoint,omitempty"`
}

// RuntimeStateRoot resolves the SDK-owned local runtime state directory.
func RuntimeStateRoot(controlPath string) (string, error) {
	resolved, err := resolveControlDiscoveryPath(controlPath)
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

// ReadRuntimeIdentityProjection reads the standalone identity projection.
// An empty credentialsPath is derived from controlPath for compatibility.
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
			Code:      ErrRuntimeOffline,
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

func runtimeIdentityProjectionFromControlDiscovery(discovery controlDiscovery, controlPath string) (RuntimeIdentityProjection, error) {
	identity := discovery.runtimeHostIdentity
	if identity == nil || strings.TrimSpace(identity.Realm) == "" ||
		(identity.RuntimeInstanceID == nil || strings.TrimSpace(*identity.RuntimeInstanceID) == "") {
		return RuntimeIdentityProjection{}, &SDKError{
			Code:      ErrCallerIdentityUnavailable,
			Stage:     "runtime_environment",
			Retry:     RetryNever,
			Retryable: false,
			Message:   "runtime control discovery has no complete runtime host identity",
			Details:   map[string]any{"control_path": controlPath},
		}
	}
	return RuntimeIdentityProjection{
		Realm:             identity.Realm,
		RuntimeInstanceID: strings.TrimSpace(*identity.RuntimeInstanceID),
	}, nil
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
	if err := rejectUnknownRuntimeIdentityProjectionFields(decoded); err != nil {
		return RuntimeIdentityProjection{}, err
	}
	realm, err := runtimeEnvironmentRequiredString(decoded, "realm")
	if err != nil {
		return RuntimeIdentityProjection{}, err
	}
	runtimeInstanceID, err := runtimeEnvironmentRequiredString(decoded, "runtime_instance_id")
	if err != nil {
		return RuntimeIdentityProjection{}, err
	}
	principal, err := runtimeEnvironmentOptionalString(decoded, "principal")
	if err != nil {
		return RuntimeIdentityProjection{}, err
	}
	controlPlaneEndpoint, err := runtimeEnvironmentOptionalString(decoded, "control_plane_endpoint")
	if err != nil {
		return RuntimeIdentityProjection{}, err
	}
	projection := RuntimeIdentityProjection{
		Realm:                realm,
		RuntimeInstanceID:    runtimeInstanceID,
		Principal:            principal,
		ControlPlaneEndpoint: controlPlaneEndpoint,
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

// ReadRuntimeIdentityProjection reads the environment's control-discovery
// identity unless an explicit credentialsPath is supplied.
func (e *SdkEnvironment) ReadRuntimeIdentityProjection(ctx context.Context, credentialsPath string) (RuntimeIdentityProjection, error) {
	if e == nil {
		return RuntimeIdentityProjection{}, invalidRuntimeEnvironment("sdk environment is not initialized", nil)
	}
	if strings.TrimSpace(credentialsPath) == "" {
		discovery, err := fileControlDiscoveryReader{}.readControlDiscovery(
			ctx,
			e.options.Discover.ControlPath,
		)
		if err != nil {
			return RuntimeIdentityProjection{}, err
		}
		return runtimeIdentityProjectionFromControlDiscovery(
			discovery,
			e.options.Discover.ControlPath,
		)
	}
	return ReadRuntimeIdentityProjection(ctx, credentialsPath, e.options.Discover.ControlPath)
}

func rejectUnknownRuntimeIdentityProjectionFields(raw map[string]any) error {
	var unknown []string
	for key := range raw {
		if _, ok := runtimeIdentityProjectionAllowedFields[key]; !ok {
			unknown = append(unknown, key)
		}
	}
	if len(unknown) > 0 {
		sort.Strings(unknown)
		return invalidRuntimeEnvironment(
			"runtime identity projection contains unknown fields: "+strings.Join(unknown, ", "),
			nil,
		)
	}
	return nil
}

func runtimeEnvironmentRequiredString(raw map[string]any, key string) (string, error) {
	value, ok := raw[key]
	if !ok || value == nil {
		return "", invalidRuntimeEnvironment("runtime identity projection missing "+key, nil)
	}
	text, ok := value.(string)
	if !ok {
		return "", invalidRuntimeEnvironment("runtime identity projection "+key+" must be a string", nil)
	}
	text = strings.TrimSpace(text)
	if text == "" {
		return "", invalidRuntimeEnvironment("runtime identity projection missing "+key, nil)
	}
	return text, nil
}

func runtimeEnvironmentOptionalString(raw map[string]any, key string) (string, error) {
	value, ok := raw[key]
	if !ok || value == nil {
		return "", nil
	}
	if text, ok := value.(string); ok {
		return strings.TrimSpace(text), nil
	}
	return "", invalidRuntimeEnvironment("runtime identity projection "+key+" must be a string", nil)
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
