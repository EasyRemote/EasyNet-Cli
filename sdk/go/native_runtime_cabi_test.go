//go:build easynet_cabi && cgo && !windows

package easynet

import (
	"context"
	"path/filepath"
	"testing"
)

func TestOpenNativeRuntimeUsesNativeProviderBuild(t *testing.T) {
	missing := filepath.Join(t.TempDir(), "libeasynet_cli_missing.dylib")

	_, err := OpenNativeRuntime(context.Background(), NativeRuntimeOptions{
		LibraryPath: missing,
	})

	if !IsCode(err, ErrTransport) {
		t.Fatalf("OpenNativeRuntime error = %v, want %s", err, ErrTransport)
	}
}
