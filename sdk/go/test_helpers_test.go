package easynet

import (
	"context"
	"encoding/json"
	"fmt"
	"os"
	"path/filepath"
	"reflect"
	"runtime"
	"strings"
	"testing"
)

func repositoryRoot(t *testing.T) string {
	t.Helper()
	_, file, _, ok := runtime.Caller(0)
	if !ok {
		t.Fatal("locate test file")
	}
	return filepath.Clean(filepath.Join(filepath.Dir(file), "..", ".."))
}

func sharedFixture(t *testing.T, root, name string) []byte {
	t.Helper()
	path := filepath.Join(root, "sdk", "conformance", "fixtures", name)
	raw, err := os.ReadFile(path)
	if err != nil {
		t.Fatalf("read shared fixture %s: %v", path, err)
	}
	return raw
}

func assertJSONEquivalent(t *testing.T, actual, expected []byte) {
	t.Helper()
	var got, want any
	if err := json.Unmarshal(actual, &got); err != nil {
		t.Fatalf("decode actual JSON: %v", err)
	}
	if err := json.Unmarshal(expected, &want); err != nil {
		t.Fatalf("decode expected JSON: %v", err)
	}
	if !reflect.DeepEqual(got, want) {
		t.Fatalf("JSON mismatch\nactual: %s\nexpected: %s", actual, expected)
	}
}

func testResolveDescriptorRef(t *testing.T) func(context.Context, []byte) ([]byte, error) {
	t.Helper()
	return func(ctx context.Context, requestJSON []byte) ([]byte, error) {
		var request RuntimeDescriptorRefRequest
		if err := json.Unmarshal(requestJSON, &request); err != nil {
			return nil, err
		}
		ability, err := NewCanonicalAddressing().OwnerAbilityURA(ctx, request.CalleeURA, request.Ability)
		if err != nil {
			return nil, err
		}
		action := "read"
		if strings.TrimSpace(request.CallMode) == "stream" {
			action = "stream"
		}
		return json.Marshal(map[string]any{
			"descriptor_ref": fmt.Sprintf(
				"%s@1.0.0#%s!%s",
				ability,
				"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
				action,
			),
		})
	}
}
