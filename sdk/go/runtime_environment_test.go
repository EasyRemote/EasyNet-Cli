package easynet

import (
	"context"
	"os"
	"path/filepath"
	"testing"
)

func TestRuntimeIdentityProjectionReadsCredentials(t *testing.T) {
	dir := t.TempDir()
	credentials := filepath.Join(dir, "credentials.json")
	if err := os.WriteFile(credentials, []byte(`{
		"realm": "acme",
		"runtime_instance_id": "runtime-a",
		"principal": "alice",
		"control_plane_endpoint": "hub:443"
	}`), 0o600); err != nil {
		t.Fatalf("write credentials: %v", err)
	}

	projection, err := ReadRuntimeIdentityProjection(context.Background(), credentials, "")
	if err != nil {
		t.Fatalf("ReadRuntimeIdentityProjection: %v", err)
	}
	if projection.Realm != "acme" ||
		projection.RuntimeInstanceID != "runtime-a" ||
		projection.Principal != "alice" ||
		projection.ControlPlaneEndpoint != "hub:443" {
		t.Fatalf("unexpected projection: %#v", projection)
	}
}

func TestRuntimeIdentityProjectionRejectsNodeIDAlias(t *testing.T) {
	_, err := NewRuntimeIdentityProjectionFromJSON([]byte(`{"realm":"acme","node_id":"dev-a"}`))
	if err == nil {
		t.Fatal("expected node_id-only projection to fail")
	}
	if !IsCode(err, ErrInvalidArgument) {
		t.Fatalf("error = %v, want %s", err, ErrInvalidArgument)
	}
}

func TestRuntimeIdentityProjectionRejectsDeviceIDAlias(t *testing.T) {
	_, err := NewRuntimeIdentityProjectionFromJSON([]byte(`{"realm":"acme","device_id":"dev-a"}`))
	if err == nil {
		t.Fatal("expected device_id-only projection to fail")
	}
	if !IsCode(err, ErrInvalidArgument) {
		t.Fatalf("error = %v, want %s", err, ErrInvalidArgument)
	}
}

func TestRuntimeCredentialsPathDerivesFromControlPath(t *testing.T) {
	got, err := RuntimeCredentialsPath(filepath.Join("/tmp", "easynet-state", "control.json"))
	if err != nil {
		t.Fatalf("RuntimeCredentialsPath: %v", err)
	}
	if want := filepath.Join("/tmp", "easynet-state", "credentials.json"); got != want {
		t.Fatalf("credentials path = %q, want %q", got, want)
	}
}

func TestRuntimeIdentityProjectionRejectsMissingRuntimeInstanceID(t *testing.T) {
	_, err := NewRuntimeIdentityProjectionFromJSON([]byte(`{"realm":"acme"}`))
	if err == nil {
		t.Fatal("expected missing runtime_instance_id to fail")
	}
	if !IsCode(err, ErrInvalidArgument) {
		t.Fatalf("error = %v, want %s", err, ErrInvalidArgument)
	}
}
