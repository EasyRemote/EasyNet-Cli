package easynet

import (
	"os"
	"path/filepath"
	"testing"
)

func TestResolveLocalRuntimeEndpointPathUsesDefault(t *testing.T) {
	home := t.TempDir()
	t.Setenv("HOME", home)
	got, err := ResolveLocalRuntimeEndpointPath(LocalRuntimeEndpointOptions{
		DefaultPath: "~/runtime/daemon.sock",
	})
	if err != nil {
		t.Fatalf("ResolveLocalRuntimeEndpointPath: %v", err)
	}
	want := filepath.Join(home, "runtime", "daemon.sock")
	if got != want {
		t.Fatalf("endpoint path = %q, want %q", got, want)
	}
}

func TestResolveLocalRuntimeEndpointPathPrefersExplicitPath(t *testing.T) {
	dir := t.TempDir()
	explicit := filepath.Join(dir, "explicit.sock")
	t.Setenv("RUNTIME_ENDPOINT_PATH", filepath.Join(dir, "env.sock"))
	got, err := ResolveLocalRuntimeEndpointPath(LocalRuntimeEndpointOptions{
		Path:        explicit,
		EnvVar:      "RUNTIME_ENDPOINT_PATH",
		DefaultPath: "~/runtime/default.sock",
	})
	if err != nil {
		t.Fatalf("ResolveLocalRuntimeEndpointPath: %v", err)
	}
	if got != explicit {
		t.Fatalf("endpoint path = %q, want %q", got, explicit)
	}
}

func TestResolveLocalRuntimeEndpointPathReadsEnvironment(t *testing.T) {
	dir := t.TempDir()
	override := filepath.Join(dir, "runtime.sock")
	t.Setenv("RUNTIME_ENDPOINT_PATH", override)
	got, err := ResolveLocalRuntimeEndpointPath(LocalRuntimeEndpointOptions{
		EnvVar:      "RUNTIME_ENDPOINT_PATH",
		DefaultPath: "~/runtime/default.sock",
	})
	if err != nil {
		t.Fatalf("ResolveLocalRuntimeEndpointPath: %v", err)
	}
	if got != override {
		t.Fatalf("endpoint path = %q, want %q", got, override)
	}
}

func TestResolveLocalRuntimeEndpointPathRequiresConfiguredPath(t *testing.T) {
	if _, err := ResolveLocalRuntimeEndpointPath(LocalRuntimeEndpointOptions{}); err == nil {
		t.Fatal("ResolveLocalRuntimeEndpointPath accepted empty options")
	}
}

func TestResolveLocalRuntimeEndpointPathRejectsUserHomeForm(t *testing.T) {
	if _, err := ResolveLocalRuntimeEndpointPath(LocalRuntimeEndpointOptions{Path: "~other/runtime.sock"}); err == nil {
		t.Fatal("ResolveLocalRuntimeEndpointPath accepted ~user form")
	}
}

func TestResolveLocalRuntimeEndpointPathResolvesRelativeAgainstWorkingDirectory(t *testing.T) {
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
	got, err := ResolveLocalRuntimeEndpointPath(LocalRuntimeEndpointOptions{Path: "runtime.sock"})
	if err != nil {
		t.Fatalf("ResolveLocalRuntimeEndpointPath: %v", err)
	}
	want := filepath.Join(resolvedCwd, "runtime.sock")
	if got != want {
		t.Fatalf("endpoint path = %q, want %q", got, want)
	}
}
