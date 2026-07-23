package easynet

import (
	"context"
	"encoding/json"
	"fmt"
	"os"
	"strings"

	runtimesdk "easynet.run/cli/sdk/go"
)

// ReadDaemonRuntimeIdentityProjection adapts EasyNet daemon credentials into
// the canonical runtime identity projection.
//
// EasyNet credentials are product-owned and expose a device identity fact. The
// canonical SDK root does not accept product fields as runtime identity; this
// provider adapter is the only place that translates device_id into
// RuntimeInstanceID.
func ReadDaemonRuntimeIdentityProjection(ctx context.Context, credentialsPath string) (runtimesdk.RuntimeIdentityProjection, error) {
	projection, err := runtimesdk.ReadRuntimeIdentityProjection(ctx, credentialsPath, "")
	if err == nil {
		return projection, nil
	}
	if !runtimesdk.IsCode(err, runtimesdk.ErrInvalidArgument) {
		return runtimesdk.RuntimeIdentityProjection{}, err
	}
	raw, readErr := os.ReadFile(strings.TrimSpace(credentialsPath))
	if readErr != nil {
		return runtimesdk.RuntimeIdentityProjection{}, readErr
	}
	var decoded map[string]any
	if unmarshalErr := json.Unmarshal(raw, &decoded); unmarshalErr != nil {
		return runtimesdk.RuntimeIdentityProjection{}, unmarshalErr
	}
	realm := providerIdentityString(decoded, "realm")
	runtimeInstanceID, idErr := providerRuntimeInstanceID(decoded)
	if idErr != nil {
		return runtimesdk.RuntimeIdentityProjection{}, idErr
	}
	if realm == "" || runtimeInstanceID == "" {
		return runtimesdk.RuntimeIdentityProjection{}, fmt.Errorf("daemon credentials missing runtime identity")
	}
	return runtimesdk.RuntimeIdentityProjection{
		Realm:                realm,
		RuntimeInstanceID:    runtimeInstanceID,
		Principal:            providerIdentityString(decoded, "username"),
		ControlPlaneEndpoint: providerIdentityString(decoded, "hub_endpoint"),
	}, nil
}

func providerRuntimeInstanceID(decoded map[string]any) (string, error) {
	if retired := providerIdentityString(decoded, "node_id"); retired != "" {
		return "", fmt.Errorf("daemon credentials contain retired node_id identity alias")
	}
	deviceID := providerIdentityString(decoded, "device_id")
	return deviceID, nil
}

func providerIdentityString(raw map[string]any, key string) string {
	value, ok := raw[key]
	if !ok || value == nil {
		return ""
	}
	if text, ok := value.(string); ok {
		return strings.TrimSpace(text)
	}
	return strings.TrimSpace(fmt.Sprint(value))
}
