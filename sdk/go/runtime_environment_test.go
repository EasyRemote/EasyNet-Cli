package easynet

import (
	"context"
	"os"
	"path/filepath"
	"strings"
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
	if !strings.Contains(err.Error(), "unknown fields: node_id") {
		t.Fatalf("error = %v, want unknown node_id rejection", err)
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
	if !strings.Contains(err.Error(), "unknown fields: device_id") {
		t.Fatalf("error = %v, want unknown device_id rejection", err)
	}
}

func TestRuntimeIdentityProjectionRejectsRetiredAliasesEvenWithCanonicalRuntimeID(t *testing.T) {
	_, err := NewRuntimeIdentityProjectionFromJSON([]byte(`{
		"realm":"acme",
		"runtime_instance_id":"runtime-a",
		"node_id":"dev-a",
		"device_id":"device-a"
	}`))
	if err == nil {
		t.Fatal("expected retired aliases to fail")
	}
	if !IsCode(err, ErrInvalidArgument) ||
		!strings.Contains(err.Error(), "device_id") ||
		!strings.Contains(err.Error(), "node_id") {
		t.Fatalf("error = %v, want unknown retired alias rejection", err)
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

func TestRuntimeIdentityProjectionRejectsNonStringIdentityFacts(t *testing.T) {
	for _, raw := range []string{
		`{"realm":7,"runtime_instance_id":"runtime-a"}`,
		`{"realm":"acme","runtime_instance_id":7}`,
		`{"realm":"acme","runtime_instance_id":"runtime-a","principal":true}`,
		`{"realm":"acme","runtime_instance_id":"runtime-a","control_plane_endpoint":443}`,
	} {
		_, err := NewRuntimeIdentityProjectionFromJSON([]byte(raw))
		if err == nil {
			t.Fatalf("expected non-string fact to fail: %s", raw)
		}
		if !IsCode(err, ErrInvalidArgument) || !strings.Contains(err.Error(), "must be a string") {
			t.Fatalf("error = %v, want string type rejection", err)
		}
	}
}
