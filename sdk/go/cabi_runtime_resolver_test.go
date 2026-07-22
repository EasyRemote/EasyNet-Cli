//go:build runtime_cabi && cgo && !windows

package easynet

import (
	"encoding/json"
	"strings"
	"testing"
)

func TestResolveDescriptorRefFromDiagnosticsNameSelectorRequiresCalleeOwner(t *testing.T) {
	diagnostics := []byte(`{
		"descriptor_catalog": {
			"source": "test",
			"entries": [
				{
					"name": "page.fetch",
					"owner_ura": "easynet:///r/test/agent/alice.pages",
					"ability_ura": "easynet:///r/test/ability/alice.pages.page.fetch",
					"descriptor_ref": "easynet:///r/test/ability/alice.pages.page.fetch@1.0.0#aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa!invoke",
					"call_mode": "rpc"
				}
			]
		}
	}`)
	_, err := resolveDescriptorRefFromDiagnostics(
		[]byte(`{"callee_ura":"easynet:///r/test/device/dev-a","ability":"page.fetch","call_mode":"rpc"}`),
		diagnostics,
	)
	if err == nil {
		t.Fatal("name selector resolved an ability owned by a different owner")
	}
	if !IsCode(err, ErrDescriptorNotFound) {
		t.Fatalf("error = %v, want %s", err, ErrDescriptorNotFound)
	}
}

func TestResolveDescriptorRefFromDiagnosticsAbilityURASelectorAllowsHostedOwner(t *testing.T) {
	const descriptorRef = "easynet:///r/test/ability/alice.pages.page.fetch@1.0.0#aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa!invoke"
	diagnostics := []byte(`{
		"descriptor_catalog": {
			"source": "test",
			"entries": [
				{
					"name": "page.fetch",
					"owner_ura": "easynet:///r/test/agent/alice.pages",
					"ability_ura": "easynet:///r/test/ability/alice.pages.page.fetch",
					"descriptor_ref": "` + descriptorRef + `",
					"call_mode": "rpc"
				}
			]
		}
	}`)
	raw, err := resolveDescriptorRefFromDiagnostics(
		[]byte(`{
			"callee_ura":"easynet:///r/test/device/dev-a",
			"ability":"easynet:///r/test/ability/alice.pages.page.fetch",
			"call_mode":"rpc"
		}`),
		diagnostics,
	)
	if err != nil {
		t.Fatalf("ability URA selector should resolve hosted owner descriptor: %v", err)
	}
	var decoded map[string]any
	if err := json.Unmarshal(raw, &decoded); err != nil {
		t.Fatalf("decode resolver response: %v", err)
	}
	if decoded["descriptor_ref"] != descriptorRef {
		t.Fatalf("descriptor_ref = %#v", decoded["descriptor_ref"])
	}
	if decoded["owner_ura"] != "easynet:///r/test/agent/alice.pages" {
		t.Fatalf("owner_ura = %#v", decoded["owner_ura"])
	}
}

func TestResolveDescriptorRefFromDiagnosticsRequiresCallMode(t *testing.T) {
	diagnostics := []byte(`{
		"descriptor_catalog": {
			"source": "test",
			"entries": [
				{
					"name": "page.fetch",
					"owner_ura": "easynet:///r/test/device/dev-a",
					"ability_ura": "easynet:///r/test/ability/device.dev-a.page.fetch",
					"descriptor_ref": "easynet:///r/test/ability/device.dev-a.page.fetch@1.0.0#aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa!invoke",
					"call_mode": "rpc"
				}
			]
		}
	}`)
	_, err := resolveDescriptorRefFromDiagnostics(
		[]byte(`{"callee_ura":"easynet:///r/test/device/dev-a","ability":"page.fetch"}`),
		diagnostics,
	)
	if err == nil {
		t.Fatal("descriptor_ref diagnostics resolver accepted missing call_mode")
	}
	if !IsCode(err, ErrInvalidArgument) {
		t.Fatalf("error = %v, want %s", err, ErrInvalidArgument)
	}
}

func TestResolveDescriptorRefFromDiagnosticsRejectsMatchingRowWithoutDescriptorRef(t *testing.T) {
	diagnostics := []byte(`{
		"descriptor_catalog": {
			"source": "test",
			"entries": [
				{
					"name": "page.fetch",
					"owner_ura": "easynet:///r/test/device/dev-a",
					"ability_ura": "easynet:///r/test/ability/device.dev-a.page.fetch",
					"call_mode": "rpc"
				}
			]
		}
	}`)
	_, err := resolveDescriptorRefFromDiagnostics(
		[]byte(`{"callee_ura":"easynet:///r/test/device/dev-a","ability":"page.fetch","call_mode":"rpc"}`),
		diagnostics,
	)
	if err == nil {
		t.Fatal("descriptor_ref diagnostics resolver accepted a matching row without descriptor_ref")
	}
	if !IsCode(err, ErrInvalidArgument) {
		t.Fatalf("error = %v, want %s", err, ErrInvalidArgument)
	}
	if got := err.Error(); !strings.Contains(got, "descriptor catalog row") || !strings.Contains(got, "missing descriptor_ref") {
		t.Fatalf("error = %v, want missing descriptor_ref diagnostic", err)
	}
}
