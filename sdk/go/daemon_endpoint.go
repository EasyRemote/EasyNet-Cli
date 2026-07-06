package easynet

import (
	"fmt"
	"os"
	"path/filepath"
	"strings"
)

const (
	DefaultDaemonSocketPath = "~/.easynet/daemon.sock"
	DaemonSocketPathEnv     = "EASYNET_DAEMON_SOCKET_PATH"
)

// ResolveDaemonSocketPathFromEnv resolves the configured daemon invocation UDS
// path from environment override or the SDK-owned default.
func ResolveDaemonSocketPathFromEnv() (string, error) {
	return ResolveDaemonSocketPath(strings.TrimSpace(os.Getenv(DaemonSocketPathEnv)))
}

// ResolveDaemonSocketPath turns a configured daemon UDS path into an absolute
// filesystem path. Empty uses DefaultDaemonSocketPath; relative paths resolve
// against the current working directory.
func ResolveDaemonSocketPath(path string) (string, error) {
	if strings.TrimSpace(path) == "" {
		path = DefaultDaemonSocketPath
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
		return "", fmt.Errorf("resolve daemon socket path: unsupported ~user form in %q", path)
	}
	home, err := os.UserHomeDir()
	if err != nil {
		return "", fmt.Errorf("resolve daemon socket path: locate home dir: %w", err)
	}
	if path == "~" {
		return home, nil
	}
	return filepath.Join(home, path[2:]), nil
}
