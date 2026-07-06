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
	case "ability_descriptor.go", "authority.go", "invocation_canonical.go", "invoke_remote.go", "ura.go":
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
