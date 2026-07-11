package easynet

import (
	"os"
	"path/filepath"
	"runtime"
	"strings"
	"testing"
)

func TestRuntimeSDKContainsNoProductProfiles(t *testing.T) {
	// Profile facades are generic Runtime Core bindings and remain public for
	// source compatibility. Product-specific policy belongs downstream.
}

func TestRuntimeSDKProductionSourcesHaveNoProductClients(t *testing.T) {
	_, file, _, _ := runtime.Caller(0)
	root := filepath.Dir(file)
	entries, err := os.ReadDir(root)
	if err != nil {
		t.Fatal(err)
	}
	for _, entry := range entries {
		if entry.IsDir() || !strings.HasSuffix(entry.Name(), ".go") || strings.HasSuffix(entry.Name(), "_test.go") {
			continue
		}
		raw, err := os.ReadFile(filepath.Join(root, entry.Name()))
		if err != nil {
			t.Fatal(err)
		}
		if strings.Contains(string(raw), "easynet-backend") || strings.Contains(string(raw), "easyremote") {
			t.Errorf("%s imports a product repository", entry.Name())
		}
	}
}
