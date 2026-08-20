//go:build runtime_cabi && cgo && !windows

package easynet

import (
	"runtime"
	"strings"
	"testing"
)

func TestCABILibraryCandidatesPreferExplicitPath(t *testing.T) {
	candidates := cabiLibraryCandidates("/opt/easynet/libeasynet_cli.custom")
	if len(candidates) != 1 || candidates[0] != "/opt/easynet/libeasynet_cli.custom" {
		t.Fatalf("explicit path was not the only C ABI candidate: %#v", candidates)
	}
}

func TestCABILibraryCandidatesUsePlatformLibraryName(t *testing.T) {
	candidates := cabiLibraryCandidates("")
	if len(candidates) != 1 {
		t.Fatalf("default loading must use only the system library name: %#v", candidates)
	}
	want := "libeasynet_cli.so"
	if runtime.GOOS == "darwin" {
		want = "libeasynet_cli.dylib"
	}
	if !strings.Contains(candidates[0], want) {
		t.Fatalf("first C ABI candidate %q does not contain %q", candidates[0], want)
	}
	if strings.Contains(candidates[0], "target/") {
		t.Fatalf("production SDK must not load repository build artifacts: %#v", candidates)
	}
}
