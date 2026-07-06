package easynet

import (
	"os"
	"path/filepath"
	"testing"
)

func TestResolveDaemonSocketPathDefaultsToHomeDaemonSocket(t *testing.T) {
	home := t.TempDir()
	t.Setenv("HOME", home)
	got, err := ResolveDaemonSocketPath("")
	if err != nil {
		t.Fatalf("ResolveDaemonSocketPath: %v", err)
	}
	want := filepath.Join(home, ".easynet", "daemon.sock")
	if got != want {
		t.Fatalf("socket path = %q, want %q", got, want)
	}
}

func TestResolveDaemonSocketPathFromEnvPrefersOverride(t *testing.T) {
	dir := t.TempDir()
	override := filepath.Join(dir, "custom.sock")
	t.Setenv(DaemonSocketPathEnv, override)
	got, err := ResolveDaemonSocketPathFromEnv()
	if err != nil {
		t.Fatalf("ResolveDaemonSocketPathFromEnv: %v", err)
	}
	if got != override {
		t.Fatalf("socket path = %q, want %q", got, override)
	}
}

func TestResolveDaemonSocketPathRejectsUserHomeForm(t *testing.T) {
	if _, err := ResolveDaemonSocketPath("~other/.easynet/socket"); err == nil {
		t.Fatal("ResolveDaemonSocketPath accepted ~user form")
	}
}

func TestResolveDaemonSocketPathResolvesRelativeAgainstWorkingDirectory(t *testing.T) {
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
	got, err := ResolveDaemonSocketPath("runtime.sock")
	if err != nil {
		t.Fatalf("ResolveDaemonSocketPath: %v", err)
	}
	want := filepath.Join(resolvedCwd, "runtime.sock")
	if got != want {
		t.Fatalf("socket path = %q, want %q", got, want)
	}
}
