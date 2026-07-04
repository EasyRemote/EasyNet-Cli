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
		`github.com/easynet/axon`,
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
				t.Fatalf("%s contains forbidden dependency marker %q", path, needle)
			}
		}
		return nil
	})
	if err != nil {
		t.Fatalf("walk SDK package: %v", err)
	}
}

func allowedPrivateCABIAdapter(path, text string) bool {
	base := filepath.Base(path)
	if !strings.Contains(text, "easynet_cabi") || !strings.Contains(text, "cgo") {
		return false
	}
	return (base == "cabi_dynamic.go" && strings.Contains(text, "type CABIDiscoveryTransport struct")) ||
		(base == "cabi_runtime.go" && strings.Contains(text, "type CABIDaemonTransport struct"))
}
