package easynet

import (
	"crypto/sha256"
	"encoding/base64"
	"encoding/hex"
	"fmt"
	"os"
	"path/filepath"
	"strings"
	"sync"
)

// GatewayLifecycleState is the explicit lifecycle state owned by the Admin +
// Gateway profile lifecycle facade.
type GatewayLifecycleState string

const (
	GatewayLifecycleIdle    GatewayLifecycleState = "idle"
	GatewayLifecycleRunning GatewayLifecycleState = "running"
)

// GatewayDaemonHandle is the minimal daemon handle required by the gateway
// lifecycle facade.
type GatewayDaemonHandle interface {
	Stop() error
}

// GatewayStarter starts the daemon process for the normalized gateway realm.
type GatewayStarter func(realm string) (GatewayDaemonHandle, error)

// GatewayConfig is the generic Admin + Gateway listener configuration projected
// by the SDK. It is not a product route or backend session model.
type GatewayConfig struct {
	Port        int
	Realm       string
	HomeDir     string
	TLSCertPath string
	TLSKeyPath  string
	Hostname    string
}

// Normalize validates filesystem and listener facts before daemon start.
func (c GatewayConfig) Normalize() (GatewayConfig, error) {
	normalized := c
	normalized.Realm = strings.TrimSpace(normalized.Realm)
	if normalized.Realm == "" {
		normalized.Realm = "localhost"
	}
	if normalized.Port <= 0 || normalized.Port > 65535 {
		return GatewayConfig{}, invalidProfilePayload(adminGatewayProfile, "gateway port must be between 1 and 65535", nil)
	}
	if strings.TrimSpace(normalized.HomeDir) == "" {
		return GatewayConfig{}, invalidProfilePayload(adminGatewayProfile, "gateway home_dir must not be empty", nil)
	}
	cert, err := existingGatewayFile(normalized.TLSCertPath, "TLS certificate")
	if err != nil {
		return GatewayConfig{}, err
	}
	key, err := existingGatewayFile(normalized.TLSKeyPath, "TLS private key")
	if err != nil {
		return GatewayConfig{}, err
	}
	normalized.HomeDir = filepath.Clean(normalized.HomeDir)
	normalized.TLSCertPath = cert
	normalized.TLSKeyPath = key
	normalized.Hostname = strings.TrimSpace(normalized.Hostname)
	return normalized, nil
}

// ConfigPath returns the daemon config path materialized by the lifecycle
// facade.
func (c GatewayConfig) ConfigPath() string {
	return filepath.Join(c.HomeDir, "daemon-config.toml")
}

// Endpoint returns the host:port endpoint projected for callers.
func (c GatewayConfig) Endpoint() string {
	host := strings.TrimSpace(c.Hostname)
	if host == "" {
		host = "localhost"
	}
	return fmt.Sprintf("%s:%d", host, c.Port)
}

// GatewayRuntime is one running gateway daemon process projection.
type GatewayRuntime struct {
	State       GatewayLifecycleState
	Endpoint    string
	Fingerprint string
	ConfigPath  string
	Daemon      GatewayDaemonHandle
}

// GatewayLifecycleFacade owns the reusable Admin + Gateway start mechanics.
type GatewayLifecycleFacade struct {
	mu      sync.Mutex
	starter GatewayStarter
	state   GatewayLifecycleState
	runtime GatewayRuntime
}

// NewGatewayLifecycleFacade creates a lifecycle facade over a daemon starter.
func NewGatewayLifecycleFacade(starter GatewayStarter) (*GatewayLifecycleFacade, error) {
	if starter == nil {
		return nil, invalidProfileClient(adminGatewayProfile, "gateway daemon starter is required")
	}
	return &GatewayLifecycleFacade{starter: starter, state: GatewayLifecycleIdle}, nil
}

// State returns the current gateway lifecycle state.
func (f *GatewayLifecycleFacade) State() GatewayLifecycleState {
	if f == nil {
		return GatewayLifecycleIdle
	}
	f.mu.Lock()
	defer f.mu.Unlock()
	return f.state
}

// Runtime returns the current runtime projection snapshot, if the gateway is
// running.
func (f *GatewayLifecycleFacade) Runtime() (GatewayRuntime, bool) {
	if f == nil {
		return GatewayRuntime{}, false
	}
	f.mu.Lock()
	defer f.mu.Unlock()
	if f.state != GatewayLifecycleRunning {
		return GatewayRuntime{}, false
	}
	return f.runtime, true
}

// Start materializes daemon hub config once and starts the gateway daemon.
func (f *GatewayLifecycleFacade) Start(config GatewayConfig) (GatewayRuntime, error) {
	if f == nil {
		return GatewayRuntime{}, invalidProfileClient(adminGatewayProfile, "gateway lifecycle facade is not initialized")
	}
	normalized, err := config.Normalize()
	if err != nil {
		return GatewayRuntime{}, err
	}
	f.mu.Lock()
	defer f.mu.Unlock()
	if f.state == GatewayLifecycleRunning {
		if f.runtime.Daemon == nil {
			return GatewayRuntime{}, invalidProfileClient(adminGatewayProfile, "gateway runtime state is inconsistent")
		}
		return f.runtime, nil
	}
	if err := ensureGatewayHubConfig(normalized); err != nil {
		return GatewayRuntime{}, err
	}
	daemon, err := f.starter(normalized.Realm)
	if err != nil {
		return GatewayRuntime{}, transportProfileError(adminGatewayProfile, "gateway daemon start failed", err)
	}
	fingerprint, err := CertificateFingerprint(normalized.TLSCertPath)
	if err != nil {
		return GatewayRuntime{}, err
	}
	runtime := GatewayRuntime{
		State:       GatewayLifecycleRunning,
		Endpoint:    normalized.Endpoint(),
		Fingerprint: fingerprint,
		ConfigPath:  normalized.ConfigPath(),
		Daemon:      daemon,
	}
	f.runtime = runtime
	f.state = GatewayLifecycleRunning
	return runtime, nil
}

// Stop stops the owned daemon handle and returns to idle. Stop is idempotent.
func (f *GatewayLifecycleFacade) Stop() error {
	if f == nil {
		return invalidProfileClient(adminGatewayProfile, "gateway lifecycle facade is not initialized")
	}
	f.mu.Lock()
	runtime := f.runtime
	f.runtime = GatewayRuntime{}
	f.state = GatewayLifecycleIdle
	f.mu.Unlock()
	if runtime.Daemon == nil {
		return nil
	}
	return runtime.Daemon.Stop()
}

// CertificateFingerprint returns the SHA-256 DER fingerprint for a PEM
// certificate.
func CertificateFingerprint(certPath string) (string, error) {
	cert, err := existingGatewayFile(certPath, "TLS certificate")
	if err != nil {
		return "", err
	}
	pemBytes, err := os.ReadFile(cert)
	if err != nil {
		return "", invalidProfilePayload(adminGatewayProfile, fmt.Sprintf("read TLS certificate: %v", err), err)
	}
	der, err := gatewayPEMToDER(pemBytes)
	if err != nil {
		return "", err
	}
	sum := sha256.Sum256(der)
	encoded := strings.ToUpper(hex.EncodeToString(sum[:]))
	parts := make([]string, 0, len(encoded)/2)
	for i := 0; i < len(encoded); i += 2 {
		parts = append(parts, encoded[i:i+2])
	}
	return strings.Join(parts, ":"), nil
}

func ensureGatewayHubConfig(config GatewayConfig) error {
	path := config.ConfigPath()
	if _, err := os.Stat(path); err == nil {
		return nil
	} else if !os.IsNotExist(err) {
		return invalidProfilePayload(adminGatewayProfile, fmt.Sprintf("stat gateway config: %v", err), err)
	}
	if err := os.MkdirAll(filepath.Dir(path), 0o755); err != nil {
		return invalidProfilePayload(adminGatewayProfile, fmt.Sprintf("create gateway config directory: %v", err), err)
	}
	content := "# Auto-generated by daemon SDK gateway facade. Edit by hand to add\n" +
		"# [daemon.federated_peers] or override the UDS path.\n" +
		"[daemon]\n" +
		"mode = \"hub\"\n" +
		fmt.Sprintf("realm = %s\n", tomlString(config.Realm)) +
		fmt.Sprintf("listen_tcp = %s\n", tomlString(fmt.Sprintf("0.0.0.0:%d", config.Port))) +
		fmt.Sprintf("tls_cert_pem = %s\n", tomlString(config.TLSCertPath)) +
		fmt.Sprintf("tls_key_pem = %s\n", tomlString(config.TLSKeyPath))
	if err := os.WriteFile(path, []byte(content), 0o644); err != nil {
		return invalidProfilePayload(adminGatewayProfile, fmt.Sprintf("write gateway config: %v", err), err)
	}
	return nil
}

func gatewayPEMToDER(pemBytes []byte) ([]byte, error) {
	lines := strings.Split(string(pemBytes), "\n")
	var body strings.Builder
	for _, line := range lines {
		line = strings.TrimSpace(line)
		if line == "" || strings.HasPrefix(line, "-----") {
			continue
		}
		body.WriteString(line)
	}
	der, err := base64.StdEncoding.Strict().DecodeString(body.String())
	if err != nil {
		return nil, invalidProfilePayload(adminGatewayProfile, "TLS certificate must be PEM encoded", err)
	}
	return der, nil
}

func existingGatewayFile(value string, label string) (string, error) {
	if strings.TrimSpace(value) == "" {
		return "", invalidProfilePayload(adminGatewayProfile, label+" path must not be empty", nil)
	}
	path := filepath.Clean(value)
	info, err := os.Stat(path)
	if err != nil || info.IsDir() {
		if err == nil {
			err = fmt.Errorf("%s is a directory", path)
		}
		return "", invalidProfilePayload(adminGatewayProfile, fmt.Sprintf("%s file not found: %s", label, path), err)
	}
	return path, nil
}

func tomlString(value string) string {
	encoded := strings.Builder{}
	encoded.WriteByte('"')
	for _, r := range value {
		switch r {
		case '\\', '"':
			encoded.WriteByte('\\')
			encoded.WriteRune(r)
		case '\n':
			encoded.WriteString(`\n`)
		case '\r':
			encoded.WriteString(`\r`)
		case '\t':
			encoded.WriteString(`\t`)
		default:
			encoded.WriteRune(r)
		}
	}
	encoded.WriteByte('"')
	return encoded.String()
}
