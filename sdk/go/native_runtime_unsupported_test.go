//go:build !easynet_cabi || !cgo || windows

package easynet

import (
	"context"
	"testing"
)

func TestOpenNativeRuntimeUnsupportedWithoutProviderBuild(t *testing.T) {
	_, err := OpenNativeRuntime(context.Background(), NativeRuntimeOptions{})
	if !IsCode(err, ErrNotImplemented) {
		t.Fatalf("OpenNativeRuntime error = %v, want %s", err, ErrNotImplemented)
	}
}
