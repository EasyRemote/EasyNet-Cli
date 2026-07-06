package easynet

import (
	"os"
	"path/filepath"
	"strings"
	"testing"
)

func TestPublicGoSDKDoesNotImportForbiddenRuntimeBoundaries(t *testing.T) {
	forbidden := []string{
		`import "C"`,
		`easynet.run/axon`,
		`easynet.run/cli/sdk/go/internal/axonpb`,
		`github.com/easynet/axon`,
		`google.golang.org/grpc`,
		`google.golang.org/protobuf`,
		`src/daemon`,
		`src/ffi`,
	}
	err := filepath.WalkDir(".", func(path string, entry os.DirEntry, err error) error {
		if err != nil {
			return err
		}
		if entry.IsDir() || !strings.HasSuffix(path, ".go") || strings.HasSuffix(path, "_test.go") {
			return nil
		}
		body, err := os.ReadFile(path)
		if err != nil {
			return err
		}
		text := string(body)
		for _, needle := range forbidden {
			if strings.Contains(text, needle) {
				if needle == `import "C"` && allowedPrivateCABIAdapter(path, text) {
					continue
				}
				if (needle == `easynet.run/cli/sdk/go/internal/axonpb` || needle == `google.golang.org/grpc`) &&
					allowedTaggedDirectRuntimeProvider(path, text) {
					continue
				}
				if needle == `easynet.run/axon` && allowedDelegatedAxonFacade(path) {
					continue
				}
				if needle == `google.golang.org/grpc` && allowedPrivateAxonAdapter(path) {
					continue
				}
				if needle == `google.golang.org/protobuf` && allowedPrivateAxonAdapter(path) {
					continue
				}
				t.Fatalf("%s contains forbidden dependency marker %q", path, needle)
			}
		}
		return nil
	})
	if err != nil {
		t.Fatalf("walk SDK package: %v", err)
	}
}

func TestPublicGoSDKDoesNotHandParseDescriptorRefs(t *testing.T) {
	err := filepath.WalkDir(".", func(path string, entry os.DirEntry, err error) error {
		if err != nil {
			return err
		}
		if entry.IsDir() || strings.HasPrefix(filepath.ToSlash(path), "internal/axonpb/") ||
			!strings.HasSuffix(path, ".go") || strings.HasSuffix(path, "_test.go") {
			return nil
		}
		body, err := os.ReadFile(path)
		if err != nil {
			return err
		}
		text := string(body)
		for _, needle := range []string{`LastIndex(`, `Split(`, `SplitN(`, `Cut(`} {
			if strings.Contains(text, needle) && strings.Contains(text, `"@"`) {
				t.Fatalf("%s appears to hand-parse DescriptorRef grammar with %s; use Axon/Identity DescriptorRef helpers", path, needle)
			}
		}
		return nil
	})
	if err != nil {
		t.Fatalf("walk SDK package: %v", err)
	}
}

func TestPublicGoSDKDoesNotOwnURAGrammar(t *testing.T) {
	body, err := os.ReadFile("ura.go")
	if err != nil {
		t.Fatalf("read ura.go: %v", err)
	}
	text := string(body)
	if !strings.Contains(text, `axonsdk "easynet.run/axon/sdk/go/easynet"`) {
		t.Fatalf("ura.go must delegate URA grammar to the Axon Go SDK")
	}
	for _, needle := range []string{
		`fmt.Sprintf("%s%s/user/`,
		`fmt.Sprintf("%s%s/device/`,
		`fmt.Sprintf("%s%s/agent/`,
		`fmt.Sprintf("%s%s/ability/`,
		`fmt.Sprintf("%s%s/resource/`,
		`strings.Cut(`,
		`strings.CutPrefix(`,
		`strings.Split(`,
		`url.PathEscape(`,
	} {
		if strings.Contains(text, needle) {
			t.Fatalf("ura.go contains SDK-owned URA grammar fragment %q; delegate to Axon/Identity", needle)
		}
	}
}

func TestPublicGoSDKDoesNotAliasAxonBridgeTypes(t *testing.T) {
	for _, path := range []string{"invoke_remote.go", "ura.go"} {
		body, err := os.ReadFile(path)
		if err != nil {
			t.Fatalf("read %s: %v", path, err)
		}
		text := string(body)
		for _, needle := range []string{
			"type JSONByteSlice = axonsdk.",
			"type InvokeRemoteContentEnvelope = axonsdk.",
			"type OriginCallerClaim = axonsdk.",
			"type InvokeRemoteUpRequest = axonsdk.",
			"type InvokeRemoteDownChunk = axonsdk.",
			"type InvokeRemoteDownResult = axonsdk.",
			"type InvokeRemoteDownFrame = axonsdk.",
			"type DelegationProofRaw = axonsdk.",
			"type SessionAuthorityRaw = axonsdk.",
			"type Ura = axonsdk.",
			"type ParsedURA = axonsdk.",
		} {
			if strings.Contains(text, needle) {
				t.Fatalf("%s exposes Axon public alias %q; keep SDK DTOs local and convert internally", path, needle)
			}
		}
	}
}

func allowedTaggedDirectRuntimeProvider(path, text string) bool {
	base := filepath.Base(path)
	if base != "direct_runtime.go" {
		return false
	}
	return strings.Contains(text, "//go:build easynet_direct_runtime") &&
		strings.Contains(text, "type DirectDaemonRuntimeTransport struct")
}

func allowedPrivateAxonAdapter(path string) bool {
	return strings.HasPrefix(filepath.ToSlash(path), "internal/axonpb/")
}

func allowedDelegatedAxonFacade(path string) bool {
	switch filepath.ToSlash(path) {
	case "ability_descriptor_axon.go", "authority_axon.go", "invocation_canonical.go", "invoke_remote.go", "ura.go":
		return true
	default:
		return false
	}
}

func allowedPrivateCABIAdapter(path, text string) bool {
	base := filepath.Base(path)
	if !strings.Contains(text, "easynet_cabi") || !strings.Contains(text, "cgo") {
		return false
	}
	return (base == "cabi_dynamic.go" && strings.Contains(text, "type CABIDiscoveryTransport struct")) ||
		(base == "cabi_runtime.go" && strings.Contains(text, "type CABIDaemonTransport struct")) ||
		(base == "cabi_receipt.go" && strings.Contains(text, "type CABIReceiptTransport struct")) ||
		(base == "cabi_identity.go" && strings.Contains(text, "type CABIIdentityTransport struct")) ||
		(base == "cabi_publication.go" && strings.Contains(text, "type CABIPublicationTransport struct")) ||
		(base == "cabi_mission.go" && strings.Contains(text, "type CABIMissionTransport struct")) ||
		(base == "cabi_host_binding.go" && strings.Contains(text, "type CABIHostBindingTransport struct")) ||
		(base == "cabi_events.go" && strings.Contains(text, "type CABIEventsTransport struct")) ||
		(base == "cabi_admin.go" && strings.Contains(text, "type CABIAdminTransport struct")) ||
		(base == "cabi_surface.go" && strings.Contains(text, "type CABISurfaceTransport struct")) ||
		(base == "cabi_compatibility.go" && strings.Contains(text, "type CABICompatibilityTransport struct")) ||
		(base == "cabi_authority.go" && strings.Contains(text, "type CABIAuthorityTransport struct")) ||
		(base == "cabi_callbacks.go" && strings.Contains(text, "easynetGoStreamCallback"))
}
