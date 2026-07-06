package easynet

const (
	DefaultDaemonSocketPath = "~/.easynet/daemon.sock"
	DaemonSocketPathEnv     = "EASYNET_DAEMON_SOCKET_PATH"
)

// ResolveDaemonSocketPathFromEnv resolves the configured daemon invocation UDS
// path from environment override or the SDK-owned default.
func ResolveDaemonSocketPathFromEnv() (string, error) {
	return ResolveDaemonSocketPath("")
}

// ResolveDaemonSocketPath turns a configured daemon UDS path into an absolute
// filesystem path. Empty uses DefaultDaemonSocketPath; relative paths resolve
// against the current working directory.
func ResolveDaemonSocketPath(path string) (string, error) {
	return ResolveLocalRuntimeEndpointPath(LocalRuntimeEndpointOptions{
		Path:        path,
		EnvVar:      DaemonSocketPathEnv,
		DefaultPath: DefaultDaemonSocketPath,
	})
}
