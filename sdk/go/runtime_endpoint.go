package easynet

import (
	"fmt"
	"os"
	"path/filepath"
	"strings"
)

// LocalRuntimeEndpointOptions describes how a facade resolves a local runtime
// UDS endpoint. The SDK owns path normalization; product facades supply their
// own environment variable and default path while they are still on local UDS.
type LocalRuntimeEndpointOptions struct {
	Path        string
	EnvVar      string
	DefaultPath string
}

// ResolveLocalRuntimeEndpointPath turns a configured local runtime UDS endpoint
// into an absolute filesystem path.
func ResolveLocalRuntimeEndpointPath(options LocalRuntimeEndpointOptions) (string, error) {
	path := strings.TrimSpace(options.Path)
	if path == "" && strings.TrimSpace(options.EnvVar) != "" {
		path = strings.TrimSpace(os.Getenv(strings.TrimSpace(options.EnvVar)))
	}
	if path == "" {
		path = strings.TrimSpace(options.DefaultPath)
	}
	if path == "" {
		return "", invalidInvocation("local runtime endpoint path is required", nil)
	}
	expanded, err := expandHomePath(path)
	if err != nil {
		return "", err
	}
	if filepath.IsAbs(expanded) {
		return expanded, nil
	}
	return filepath.Abs(expanded)
}

func expandHomePath(path string) (string, error) {
	if path == "" || path[0] != '~' {
		return path, nil
	}
	if len(path) > 1 && path[1] != '/' {
		return "", fmt.Errorf("resolve local runtime endpoint path: unsupported ~user form in %q", path)
	}
	home, err := os.UserHomeDir()
	if err != nil {
		return "", fmt.Errorf("resolve local runtime endpoint path: locate home dir: %w", err)
	}
	if path == "~" {
		return home, nil
	}
	return filepath.Join(home, path[2:]), nil
}
