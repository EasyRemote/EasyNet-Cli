package easynet

import (
	"errors"
	"testing"
)

func TestCABIErrorFallbackProjectsStableABICodeMetadata(t *testing.T) {
	err := cabiErrorFromLastErrorJSON(nil, false, 13, "C ABI runtime descriptor_ref resolver failed")

	if !IsCode(err, ErrAbilityNotFound) {
		t.Fatalf("fallback code = %v, want %s", err, ErrAbilityNotFound)
	}
	var sdkErr *SDKError
	if !errors.As(err, &sdkErr) {
		t.Fatalf("fallback error type = %T, want SDKError", err)
	}
	if sdkErr.Stage != "runtime" {
		t.Fatalf("fallback stage = %q, want runtime", sdkErr.Stage)
	}
	if sdkErr.Source != "c_abi" {
		t.Fatalf("fallback source = %q, want c_abi", sdkErr.Source)
	}
	if sdkErr.Details["abi_symbol"] != "ERR_NOT_FOUND" {
		t.Fatalf("fallback abi_symbol = %v, want ERR_NOT_FOUND", sdkErr.Details["abi_symbol"])
	}
	if sdkErr.Message != "C ABI runtime descriptor_ref resolver failed with code 13" {
		t.Fatalf("fallback message = %q", sdkErr.Message)
	}
}

func TestCABIErrorFallbackKeepsStructuredLastErrorProjection(t *testing.T) {
	raw := []byte(`{"code":"DESCRIPTOR_NOT_FOUND","stage":"routing","message":"descriptor_ref not found","retry":"never","details":{"source":"native"}}`)

	err := cabiErrorFromLastErrorJSON(raw, true, 13, "C ABI runtime descriptor_ref resolver failed")

	if !IsCode(err, ErrDescriptorNotFound) {
		t.Fatalf("structured code = %v, want %s", err, ErrDescriptorNotFound)
	}
	var sdkErr *SDKError
	if !errors.As(err, &sdkErr) {
		t.Fatalf("structured error type = %T, want SDKError", err)
	}
	if sdkErr.Stage != "routing" {
		t.Fatalf("structured stage = %q, want routing", sdkErr.Stage)
	}
	if sdkErr.Message != "descriptor_ref not found" {
		t.Fatalf("structured message = %q", sdkErr.Message)
	}
}
