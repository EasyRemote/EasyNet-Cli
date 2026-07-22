package easynet

import (
	"bytes"
	"os"
	"os/exec"
	"path/filepath"
	"strings"
	"testing"
)

func TestPublicGoSDKDoesNotImportForbiddenRuntimeBoundaries(t *testing.T) {
	forbidden := []string{
		`import "C"`,
		`axon.run/sdk/go`,
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
				if needle == `axon.run/sdk/go` && allowedDelegatedAxonFacade(path) {
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
	if !strings.Contains(text, `axonsdk "axon.run/sdk/go/axon"`) {
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
	for _, path := range []string{"ura.go"} {
		body, err := os.ReadFile(path)
		if err != nil {
			t.Fatalf("read %s: %v", path, err)
		}
		text := string(body)
		for _, needle := range []string{
			"type JSONByteSlice = axonsdk.",
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

func TestTrackedGoSDKSourcesDoNotOwnKeyringPersistence(t *testing.T) {
	root := gitRepositoryRoot(t)
	cmd := exec.Command("git", "-C", root, "ls-files", "-z", "--", "sdk/go/*.go")
	output, err := cmd.Output()
	if err != nil {
		t.Fatalf("list tracked Go SDK sources: %v", err)
	}
	for _, rawPath := range bytes.Split(output, []byte{0}) {
		if len(rawPath) == 0 {
			continue
		}
		path := string(rawPath)
		if strings.HasSuffix(path, "_test.go") {
			continue
		}
		body, err := os.ReadFile(filepath.Join(root, path))
		if os.IsNotExist(err) {
			// A tracked source may be intentionally deleted by the change under test.
			continue
		}
		if err != nil {
			t.Fatalf("read %s: %v", path, err)
		}
		for _, marker := range []string{
			"keyring.enc",
			"VaultCiphertextB64",
			"PrivateKeySeedHex",
			"argon2.IDKey(",
		} {
			if strings.Contains(string(body), marker) {
				t.Fatalf("%s owns runtime keyring persistence marker %q; key custody belongs to the daemon keyring service", path, marker)
			}
		}
	}
}

func TestGoSDKInternalsDoNotReviveProductProfileLifecycleHelpers(t *testing.T) {
	root := gitRepositoryRoot(t)
	if _, err := os.Stat(filepath.Join(root, "sdk/go/profile_lifecycle.go")); !os.IsNotExist(err) {
		t.Fatalf("sdk/go/profile_lifecycle.go must not exist; use runtime capability lifecycle helpers")
	}
	for _, relativePath := range []string{
		"sdk/go/client_lifecycle.go",
		"sdk/go/authority.go",
	} {
		body, err := os.ReadFile(filepath.Join(root, relativePath))
		if err != nil {
			t.Fatalf("read %s: %v", relativePath, err)
		}
		for _, marker := range []string{
			"profileClientLifecycle",
			"profileCloseTransport",
		} {
			if strings.Contains(string(body), marker) {
				t.Fatalf("%s revives old product-profile lifecycle marker %q", relativePath, marker)
			}
		}
	}
}

func gitRepositoryRoot(t *testing.T) string {
	t.Helper()
	output, err := exec.Command("git", "rev-parse", "--show-toplevel").Output()
	if err != nil {
		t.Fatalf("resolve repository root: %v", err)
	}
	return strings.TrimSpace(string(output))
}

func allowedTaggedDirectRuntimeProvider(path, text string) bool {
	base := filepath.Base(path)
	if !strings.Contains(text, "//go:build easynet_direct_runtime") {
		return false
	}
	switch base {
	case "direct_runtime.go":
		return strings.Contains(text, "type directRuntimeTransport struct")
	case "direct_runtime_codec.go":
		return strings.Contains(text, "type directDescriptorBoundCodec struct")
	default:
		return false
	}
}

func allowedPrivateAxonAdapter(path string) bool {
	return strings.HasPrefix(filepath.ToSlash(path), "internal/axonpb/")
}

func allowedDelegatedAxonFacade(path string) bool {
	switch filepath.ToSlash(path) {
	case "ability_descriptor_axon.go", "authority_axon.go", "direct_runtime.go", "direct_runtime_codec.go", "directory.go", "invocation_canonical.go", "receipt.go", "resource_namespace.go", "runtime.go", "ura.go":
		return true
	default:
		return false
	}
}

func allowedPrivateCABIAdapter(path, text string) bool {
	base := filepath.Base(path)
	if !strings.Contains(text, "runtime_cabi") || !strings.Contains(text, "cgo") {
		return false
	}
	return (base == "cabi_dynamic.go" && strings.Contains(text, "func openCABIDynamicLibrary(")) ||
		(base == "cabi_runtime.go" && strings.Contains(text, "type cabiRuntimeLifecycleTransport struct")) ||
		(base == "cabi_callbacks.go" && strings.Contains(text, "easynetGoStreamCallback"))
}
