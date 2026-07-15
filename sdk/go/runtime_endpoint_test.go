package easynet

import (
	"os"
	"path/filepath"
	"testing"
)

func TestResolveRuntimeEndpointPathUsesDefault(t *testing.T) {
	home := t.TempDir()
	t.Setenv("HOME", home)
	got, err := ResolveRuntimeEndpointPath(RuntimeEndpointPathOptions{
		DefaultPath: "~/runtime/daemon.sock",
	})
	if err != nil {
		t.Fatalf("ResolveRuntimeEndpointPath: %v", err)
	}
	want := filepath.Join(home, "runtime", "daemon.sock")
	if got != want {
		t.Fatalf("endpoint path = %q, want %q", got, want)
	}
}

func TestResolveRuntimeEndpointPathPrefersExplicitPath(t *testing.T) {
	dir := t.TempDir()
	explicit := filepath.Join(dir, "explicit.sock")
	t.Setenv("RUNTIME_ENDPOINT_PATH", filepath.Join(dir, "env.sock"))
	got, err := ResolveRuntimeEndpointPath(RuntimeEndpointPathOptions{
		Path:        explicit,
		EnvVar:      "RUNTIME_ENDPOINT_PATH",
		DefaultPath: "~/runtime/default.sock",
	})
	if err != nil {
		t.Fatalf("ResolveRuntimeEndpointPath: %v", err)
	}
	if got != explicit {
		t.Fatalf("endpoint path = %q, want %q", got, explicit)
	}
}

func TestResolveRuntimeEndpointPathReadsEnvironment(t *testing.T) {
	dir := t.TempDir()
	override := filepath.Join(dir, "runtime.sock")
	t.Setenv("RUNTIME_ENDPOINT_PATH", override)
	got, err := ResolveRuntimeEndpointPath(RuntimeEndpointPathOptions{
		EnvVar:      "RUNTIME_ENDPOINT_PATH",
		DefaultPath: "~/runtime/default.sock",
	})
	if err != nil {
		t.Fatalf("ResolveRuntimeEndpointPath: %v", err)
	}
	if got != override {
		t.Fatalf("endpoint path = %q, want %q", got, override)
	}
}

func TestResolveRuntimeEndpointPathRequiresConfiguredPath(t *testing.T) {
	if _, err := ResolveRuntimeEndpointPath(RuntimeEndpointPathOptions{}); err == nil {
		t.Fatal("ResolveRuntimeEndpointPath accepted empty options")
	}
}

func TestResolveRuntimeEndpointPathRejectsUserHomeForm(t *testing.T) {
	if _, err := ResolveRuntimeEndpointPath(RuntimeEndpointPathOptions{Path: "~other/runtime.sock"}); err == nil {
		t.Fatal("ResolveRuntimeEndpointPath accepted ~user form")
	}
}

func TestResolveRuntimeEndpointPathResolvesRelativeAgainstWorkingDirectory(t *testing.T) {
	dir := t.TempDir()
	cwd, err := os.Getwd()
	if err != nil {
		t.Fatalf("Getwd: %v", err)
	}
	if err := os.Chdir(dir); err != nil {
		t.Fatalf("Chdir: %v", err)
	}
	t.Cleanup(func() { _ = os.Chdir(cwd) })
	resolvedCwd, err := os.Getwd()
	if err != nil {
		t.Fatalf("Getwd after Chdir: %v", err)
	}
	got, err := ResolveRuntimeEndpointPath(RuntimeEndpointPathOptions{Path: "runtime.sock"})
	if err != nil {
		t.Fatalf("ResolveRuntimeEndpointPath: %v", err)
	}
	want := filepath.Join(resolvedCwd, "runtime.sock")
	if got != want {
		t.Fatalf("endpoint path = %q, want %q", got, want)
	}
}
