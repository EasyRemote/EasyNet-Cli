package easynet

import (
	"context"
	"crypto/sha256"
	"encoding/hex"
	"encoding/json"
	"os"
	"path/filepath"
	"strings"
	"testing"
)

type memoryAdminTransport struct {
	agentListInvocation    string
	agentStartInvocation   string
	agentStopInvocation    string
	agentRefreshInvocation string
	sessionListInvocation  string
	revokeDeviceInvocation string
	gatewayStatus          string
	agentRecords           string
	lifecycleResult        string
	pairingPreflight       string
	deviceCredential       string
	credentialVerification string
	pairingToken           string
	deviceSession          string
	deviceSessionPage      string
	seen                   map[string]map[string]any
	closeCalls             int
}

func (m *memoryAdminTransport) remember(name string, requestJSON []byte) {
	if m.seen == nil {
		m.seen = map[string]map[string]any{}
	}
	var decoded map[string]any
	_ = json.Unmarshal(requestJSON, &decoded)
	m.seen[name] = decoded
}

func (m *memoryAdminTransport) BuildAgentListInvocation(_ context.Context, requestJSON []byte) ([]byte, error) {
	m.remember("build_agent_list", requestJSON)
	return []byte(m.agentListInvocation), nil
}

func (m *memoryAdminTransport) BuildAgentStartInvocation(_ context.Context, requestJSON []byte) ([]byte, error) {
	m.remember("build_agent_start", requestJSON)
	return []byte(m.agentStartInvocation), nil
}

func (m *memoryAdminTransport) BuildAgentStopInvocation(_ context.Context, requestJSON []byte) ([]byte, error) {
	m.remember("build_agent_stop", requestJSON)
	return []byte(m.agentStopInvocation), nil
}

func (m *memoryAdminTransport) BuildAgentRefreshInvocation(_ context.Context, requestJSON []byte) ([]byte, error) {
	m.remember("build_agent_refresh", requestJSON)
	return []byte(m.agentRefreshInvocation), nil
}

func (m *memoryAdminTransport) BuildSessionListInvocation(_ context.Context, requestJSON []byte) ([]byte, error) {
	m.remember("build_session_list", requestJSON)
	return []byte(m.sessionListInvocation), nil
}

func (m *memoryAdminTransport) BuildRevokeDeviceInvocation(_ context.Context, requestJSON []byte) ([]byte, error) {
	m.remember("build_revoke_device", requestJSON)
	return []byte(m.revokeDeviceInvocation), nil
}

func (m *memoryAdminTransport) GatewayStatus(_ context.Context, requestJSON []byte) ([]byte, error) {
	m.remember("gateway_status", requestJSON)
	return []byte(m.gatewayStatus), nil
}

func (m *memoryAdminTransport) ListAgents(_ context.Context, requestJSON []byte) ([]byte, error) {
	m.remember("list_agents", requestJSON)
	return []byte(m.agentRecords), nil
}

func (m *memoryAdminTransport) AgentStart(_ context.Context, requestJSON []byte) ([]byte, error) {
	m.remember("agent_start", requestJSON)
	return []byte(m.lifecycleResult), nil
}

func (m *memoryAdminTransport) AgentStop(_ context.Context, requestJSON []byte) ([]byte, error) {
	m.remember("agent_stop", requestJSON)
	return []byte(m.lifecycleResult), nil
}

func (m *memoryAdminTransport) AgentRefresh(_ context.Context, requestJSON []byte) ([]byte, error) {
	m.remember("agent_refresh", requestJSON)
	return []byte(m.lifecycleResult), nil
}

func (m *memoryAdminTransport) ListDeviceSessions(_ context.Context, requestJSON []byte) ([]byte, error) {
	m.remember("list_device_sessions", requestJSON)
	if m.deviceSessionPage != "" {
		return []byte(m.deviceSessionPage), nil
	}
	return []byte(`{"profile":"admin_gateway","kind":"device_sessions","state":"ok","items":[],"next_cursor":null,"metadata":{"profile":"admin_gateway","source":"session.list"}}`), nil
}

func (m *memoryAdminTransport) JoinHub(_ context.Context, requestJSON []byte) ([]byte, error) {
	m.remember("join_hub", requestJSON)
	return []byte(adminJoinResultJSON), nil
}

func (m *memoryAdminTransport) LeaveHub(_ context.Context, requestJSON []byte) ([]byte, error) {
	m.remember("leave_hub", requestJSON)
	return []byte(adminLeaveResultJSON), nil
}

func (m *memoryAdminTransport) PairingPreflight(_ context.Context, requestJSON []byte) ([]byte, error) {
	m.remember("pairing_preflight", requestJSON)
	return []byte(m.pairingPreflight), nil
}

func (m *memoryAdminTransport) ValidatePairing(_ context.Context, requestJSON []byte) ([]byte, error) {
	m.remember("validate_pairing", requestJSON)
	return []byte(m.deviceCredential), nil
}

func (m *memoryAdminTransport) VerifyDeviceCredential(_ context.Context, requestJSON []byte) ([]byte, error) {
	m.remember("verify_device_credential", requestJSON)
	return []byte(m.credentialVerification), nil
}

func (m *memoryAdminTransport) CreatePairing(_ context.Context, requestJSON []byte) ([]byte, error) {
	m.remember("create_pairing", requestJSON)
	return []byte(m.pairingToken), nil
}

func (m *memoryAdminTransport) RevokeDevice(_ context.Context, requestJSON []byte) ([]byte, error) {
	m.remember("revoke_device", requestJSON)
	return []byte(adminRevokeDeviceResultJSON), nil
}

func (m *memoryAdminTransport) CreateDeviceSession(_ context.Context, requestJSON []byte) ([]byte, error) {
	m.remember("create_device_session", requestJSON)
	return []byte(m.deviceSession), nil
}

func (m *memoryAdminTransport) DeleteDeviceSession(_ context.Context, requestJSON []byte) ([]byte, error) {
	m.remember("delete_device_session", requestJSON)
	return []byte(adminDeleteSessionResultJSON), nil
}

func (m *memoryAdminTransport) Close(ctx context.Context) error {
	m.closeCalls++
	return nil
}

func adminBaseForTest() AdminCarrierBase {
	return AdminCarrierBase{
		CallerURA:         "easynet:///r/example/agent/alice.sdk",
		CalleeURA:         "easynet:///r/example/device/dev-a",
		SubjectURA:        "easynet:///r/example/device/dev-a",
		DescriptorVersion: "1.0.0",
		NonceBase64:       "AQIDBAUGBwgJCgsMDQ4PEA==",
		CausalContext:     map[string]any{"form": "none"},
		Metadata:          map[string]any{"request_id": "admin-agent-list-1"},
	}
}

type fakeGatewayDaemon struct {
	stopped bool
}

func (d *fakeGatewayDaemon) Stop() error {
	d.stopped = true
	return nil
}

func writeGatewayPEM(t *testing.T, root string) (string, string) {
	t.Helper()
	cert := filepath.Join(root, "cert.pem")
	key := filepath.Join(root, "key.pem")
	if err := os.WriteFile(cert, []byte("-----BEGIN CERTIFICATE-----\nAAECAwQFBgc=\n-----END CERTIFICATE-----\n"), 0o644); err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(key, []byte("-----BEGIN PRIVATE KEY-----\nAAA=\n-----END PRIVATE KEY-----\n"), 0o644); err != nil {
		t.Fatal(err)
	}
	return cert, key
}

func TestGatewayLifecycleFacadeMaterializesHubConfigOnce(t *testing.T) {
	root := t.TempDir()
	cert, key := writeGatewayPEM(t, root)
	var started []string
	facade, err := NewGatewayLifecycleFacade(func(realm string) (GatewayDaemonHandle, error) {
		started = append(started, realm)
		return &fakeGatewayDaemon{}, nil
	})
	if err != nil {
		t.Fatal(err)
	}

	runtime, err := facade.Start(GatewayConfig{
		Port:        8443,
		Realm:       `ac"me`,
		HomeDir:     root,
		TLSCertPath: cert,
		TLSKeyPath:  key,
		Hostname:    "hub.example",
	})
	if err != nil {
		t.Fatal(err)
	}
	second, err := facade.Start(GatewayConfig{
		Port:        9443,
		Realm:       "ignored",
		HomeDir:     root,
		TLSCertPath: cert,
		TLSKeyPath:  key,
	})
	if err != nil {
		t.Fatal(err)
	}

	configBytes, err := os.ReadFile(filepath.Join(root, "daemon-config.toml"))
	if err != nil {
		t.Fatal(err)
	}
	config := string(configBytes)
	if runtime != second {
		t.Fatal("start while running must return the same runtime")
	}
	if len(started) != 1 || started[0] != `ac"me` {
		t.Fatalf("started realms = %#v", started)
	}
	if facade.State() != GatewayLifecycleRunning {
		t.Fatalf("state = %s", facade.State())
	}
	for _, want := range []string{
		`realm = "ac\"me"`,
		`listen_tcp = "0.0.0.0:8443"`,
		`tls_cert_pem = "` + cert + `"`,
	} {
		if !strings.Contains(config, want) {
			t.Fatalf("config missing %q:\n%s", want, config)
		}
	}
	if runtime.Endpoint != "hub.example:8443" {
		t.Fatalf("endpoint = %q", runtime.Endpoint)
	}
	expected := sha256.Sum256([]byte{0, 1, 2, 3, 4, 5, 6, 7})
	if strings.ReplaceAll(runtime.Fingerprint, ":", "") != strings.ToUpper(hex.EncodeToString(expected[:])) {
		t.Fatalf("fingerprint = %q", runtime.Fingerprint)
	}
}

func TestGatewayLifecycleFacadePreservesOperatorConfig(t *testing.T) {
	root := t.TempDir()
	cert, key := writeGatewayPEM(t, root)
	configPath := filepath.Join(root, "daemon-config.toml")
	if err := os.WriteFile(configPath, []byte("# operator-authored\n[daemon]\nmode = \"hub\"\n"), 0o644); err != nil {
		t.Fatal(err)
	}
	facade, err := NewGatewayLifecycleFacade(func(string) (GatewayDaemonHandle, error) {
		return &fakeGatewayDaemon{}, nil
	})
	if err != nil {
		t.Fatal(err)
	}
	if _, err := facade.Start(GatewayConfig{Port: 8443, Realm: "acme", HomeDir: root, TLSCertPath: cert, TLSKeyPath: key}); err != nil {
		t.Fatal(err)
	}
	content, err := os.ReadFile(configPath)
	if err != nil {
		t.Fatal(err)
	}
	if !strings.HasPrefix(string(content), "# operator-authored") {
		t.Fatalf("operator config was overwritten:\n%s", content)
	}
}

func TestGatewayLifecycleFacadeStopsDaemonAndValidatesTLS(t *testing.T) {
	root := t.TempDir()
	cert, key := writeGatewayPEM(t, root)
	daemon := &fakeGatewayDaemon{}
	facade, err := NewGatewayLifecycleFacade(func(string) (GatewayDaemonHandle, error) {
		return daemon, nil
	})
	if err != nil {
		t.Fatal(err)
	}
	if _, err := facade.Start(GatewayConfig{Port: 8443, Realm: "acme", HomeDir: root, TLSCertPath: cert, TLSKeyPath: key}); err != nil {
		t.Fatal(err)
	}

	if err := facade.Stop(); err != nil {
		t.Fatal(err)
	}

	if !daemon.stopped {
		t.Fatal("daemon was not stopped")
	}
	if facade.State() != GatewayLifecycleIdle {
		t.Fatalf("state = %s", facade.State())
	}
	_, err = facade.Start(GatewayConfig{Port: 8443, Realm: "acme", HomeDir: root, TLSCertPath: filepath.Join(root, "missing.pem"), TLSKeyPath: key})
	if !IsCode(err, ErrInvalidArgument) {
		t.Fatalf("missing TLS cert error = %v, want %s", err, ErrInvalidArgument)
	}
}

func TestCertificateFingerprintRejectsNonPEM(t *testing.T) {
	cert := filepath.Join(t.TempDir(), "bad.pem")
	if err := os.WriteFile(cert, []byte("not pem"), 0o644); err != nil {
		t.Fatal(err)
	}
	if _, err := CertificateFingerprint(cert); !IsCode(err, ErrInvalidArgument) {
		t.Fatalf("CertificateFingerprint error = %v, want %s", err, ErrInvalidArgument)
	}
}

func TestAdminBuildsAgentAndSessionInvocations(t *testing.T) {
	transport := &memoryAdminTransport{
		agentListInvocation:    adminAgentListInvocationJSON,
		agentStartInvocation:   adminAgentStartInvocationJSON,
		agentStopInvocation:    adminAgentStopInvocationJSON,
		agentRefreshInvocation: adminAgentRefreshInvocationJSON,
		sessionListInvocation:  adminSessionListInvocationJSON,
		revokeDeviceInvocation: adminRevokeDeviceInvocationJSON,
	}
	client, err := NewAdminClient(transport)
	if err != nil {
		t.Fatal(err)
	}

	listDraft, err := client.BuildAgentListInvocation(context.Background(), AdminAgentListRequest{AdminCarrierBase: adminBaseForTest()})
	if err != nil {
		t.Fatal(err)
	}
	if listDraft.DescriptorRef() != "easynet:///r/example/ability/device.dev-a.agent.list@1.0.0" {
		t.Fatalf("agent.list descriptor = %q", listDraft.DescriptorRef())
	}
	if transport.seen["build_agent_list"]["caller_ura"] != "easynet:///r/example/agent/alice.sdk" {
		t.Fatalf("admin carrier caller not preserved: %#v", transport.seen["build_agent_list"])
	}

	startReq := AdminAgentStartRequest{
		AdminCarrierBase: adminBaseForTest(),
		Name:             "codex",
		AgentType:        "codex",
		Model:            "gpt-5",
		Label:            "primary",
	}
	startDraft, err := client.BuildAgentStartInvocation(context.Background(), startReq)
	if err != nil {
		t.Fatal(err)
	}
	if startDraft.DescriptorRef() != "easynet:///r/example/ability/device.dev-a.agent.start@1.0.0" {
		t.Fatalf("agent.start descriptor = %q", startDraft.DescriptorRef())
	}
	if got := transport.seen["build_agent_start"]["name"]; got != "codex" {
		t.Fatalf("agent.start name = %#v", got)
	}

	stopDraft, err := client.BuildAgentStopInvocation(context.Background(), AdminAgentStopRequest{AdminCarrierBase: adminBaseForTest(), Name: "codex"})
	if err != nil {
		t.Fatal(err)
	}
	if stopDraft.DescriptorRef() != "easynet:///r/example/ability/device.dev-a.agent.stop@1.0.0" {
		t.Fatalf("agent.stop descriptor = %q", stopDraft.DescriptorRef())
	}

	refreshDraft, err := client.BuildAgentRefreshInvocation(context.Background(), AdminAgentRefreshRequest{AdminCarrierBase: adminBaseForTest(), Name: "codex"})
	if err != nil {
		t.Fatal(err)
	}
	if refreshDraft.DescriptorRef() != "easynet:///r/example/ability/device.dev-a.agent.refresh@1.0.0" {
		t.Fatalf("agent.refresh descriptor = %q", refreshDraft.DescriptorRef())
	}

	includeTerminated := false
	sessionDraft, err := client.BuildSessionListInvocation(context.Background(), AdminSessionListRequest{AdminCarrierBase: adminBaseForTest(), IncludeTerminated: &includeTerminated})
	if err != nil {
		t.Fatal(err)
	}
	if sessionDraft.DescriptorRef() != "easynet:///r/example/ability/device.dev-a.session.list@1.0.0" {
		t.Fatalf("session.list descriptor = %q", sessionDraft.DescriptorRef())
	}
	if got := transport.seen["build_session_list"]["include_terminated"]; got != false {
		t.Fatalf("include_terminated = %#v", got)
	}

	revokeDraft, err := client.BuildRevokeDeviceInvocation(context.Background(), RevokeDeviceRequest{
		AdminCarrierBase: adminBaseForTest(),
		DeviceURA:        "easynet:///r/example/device/dev-a",
		Reason:           "operator/key rotation",
	})
	if err != nil {
		t.Fatal(err)
	}
	if revokeDraft.DescriptorRef() != "easynet:///r/example/ability/device.dev-a.federation.revoke@1.0.0" {
		t.Fatalf("federation.revoke descriptor = %q", revokeDraft.DescriptorRef())
	}
	if got := transport.seen["build_revoke_device"]["device_ura"]; got != "easynet:///r/example/device/dev-a" {
		t.Fatalf("revoke device_ura = %#v", got)
	}
}

func TestAdminProjectsGatewayAgentsAndLifecycle(t *testing.T) {
	transport := &memoryAdminTransport{
		gatewayStatus:   adminGatewayStatusJSON,
		agentRecords:    adminAgentRecordsJSON,
		lifecycleResult: adminLifecycleResultJSON,
	}
	client, err := NewAdminClient(transport)
	if err != nil {
		t.Fatal(err)
	}

	status, err := client.GatewayStatus(context.Background(), AdminGatewayStatusRequest{})
	if err != nil {
		t.Fatal(err)
	}
	if !status.ControlReady || !status.RuntimeReady || status.PublicListenerReady {
		t.Fatalf("unexpected gateway flags: %#v", status)
	}
	if len(status.Listeners) != 2 || status.Metadata["source"] != "daemon_lifecycle_status" {
		t.Fatalf("unexpected gateway projection: %#v", status)
	}

	page, err := client.ListAgents(context.Background(), AdminAgentListRequest{AdminCarrierBase: adminBaseForTest()})
	if err != nil {
		t.Fatal(err)
	}
	if len(page.Items) != 1 || page.Items[0].Name != "codex" || page.Items[0].Runtime != "codex" {
		t.Fatalf("unexpected admin agent page: %#v", page)
	}

	result, err := client.AgentStart(context.Background(), AdminAgentStartRequest{
		AdminCarrierBase: adminBaseForTest(),
		Name:             "codex",
		AgentType:        "codex",
	})
	if err != nil {
		t.Fatal(err)
	}
	if result.Operation != "agent.start" || result.State != "ok" || result.AgentURA == nil {
		t.Fatalf("unexpected lifecycle result: %#v", result)
	}
}

func TestAdminTrustAndDeviceSessionLifecycle(t *testing.T) {
	transport := &memoryAdminTransport{
		pairingPreflight:       adminPairingPreflightJSON,
		deviceCredential:       adminDeviceCredentialJSON,
		credentialVerification: adminCredentialVerificationJSON,
		pairingToken:           adminPairingTokenJSON,
		deviceSession:          adminDeviceSessionJSON,
		deviceSessionPage:      adminDeviceSessionPageJSON,
	}
	client, err := NewAdminClient(transport)
	if err != nil {
		t.Fatal(err)
	}

	join, err := client.JoinHub(context.Background(), AdminJoinHubRequest{
		AdminCarrierBase: adminBaseForTest(),
		HubURA:           "easynet:///r/example/hub/main",
		DeviceURA:        "easynet:///r/example/device/dev-a",
	})
	if err != nil {
		t.Fatal(err)
	}
	if join.Operation != "hub.join" || transport.seen["join_hub"]["hub_ura"] != "easynet:///r/example/hub/main" {
		t.Fatalf("unexpected join result/request: %#v %#v", join, transport.seen["join_hub"])
	}

	preflight, err := client.PairingPreflight(context.Background(), PairingPreflightRequest{
		AdminCarrierBase: adminBaseForTest(),
		HubURA:           "easynet:///r/example/hub/main",
		DeviceURA:        "easynet:///r/example/device/dev-a",
		RequestedScopes:  []string{"invoke", "events"},
	})
	if err != nil {
		t.Fatal(err)
	}
	if !preflight.PairingRequired || preflight.TrustReady {
		t.Fatalf("unexpected preflight: %#v", preflight)
	}

	token, err := client.CreatePairing(context.Background(), CreatePairingRequest{
		AdminCarrierBase: adminBaseForTest(),
		HubURA:           "easynet:///r/example/hub/main",
		DeviceURA:        "easynet:///r/example/device/dev-a",
		ExpiresUnixMS:    1893456000000,
		Scopes:           []string{"invoke", "events"},
	})
	if err != nil {
		t.Fatal(err)
	}
	if token.TokenID != "pair-token-1" || token.Token == "" {
		t.Fatalf("unexpected pairing token: %#v", token)
	}

	credential, err := client.ValidatePairing(context.Background(), ValidatePairingRequest{
		AdminCarrierBase: adminBaseForTest(),
		Token:            "pair-token-value",
		DeviceURA:        "easynet:///r/example/device/dev-a",
	})
	if err != nil {
		t.Fatal(err)
	}
	if credential.CredentialID != "cred-dev-a" || credential.State != "active" {
		t.Fatalf("unexpected credential: %#v", credential)
	}

	verification, err := client.VerifyDeviceCredential(context.Background(), VerifyDeviceCredentialRequest{
		AdminCarrierBase: adminBaseForTest(),
		CredentialID:     "cred-dev-a",
		DeviceURA:        "easynet:///r/example/device/dev-a",
		HubURA:           "easynet:///r/example/hub/main",
	})
	if err != nil {
		t.Fatal(err)
	}
	if !verification.Verified || verification.Method != "daemon-trust-store" {
		t.Fatalf("unexpected verification: %#v", verification)
	}

	session, err := client.CreateDeviceSession(context.Background(), CreateDeviceSessionRequest{
		AdminCarrierBase: adminBaseForTest(),
		DeviceURA:        "easynet:///r/example/device/dev-a",
		HubURA:           "easynet:///r/example/hub/main",
		SessionKind:      "remote_desktop",
		ExpiresUnixMS:    1893456000000,
	})
	if err != nil {
		t.Fatal(err)
	}
	if session.SessionID != "dev-session-1" || session.SessionKind != "remote_desktop" {
		t.Fatalf("unexpected device session: %#v", session)
	}

	page, err := client.ListDeviceSessions(context.Background(), AdminSessionListRequest{AdminCarrierBase: adminBaseForTest()})
	if err != nil {
		t.Fatal(err)
	}
	if len(page.Items) != 1 || page.Items[0].SessionID != "dev-session-1" {
		t.Fatalf("unexpected device session page: %#v", page)
	}

	leave, err := client.LeaveHub(context.Background(), AdminLeaveHubRequest{AdminCarrierBase: adminBaseForTest(), HubURA: "easynet:///r/example/hub/main", Reason: "rotation"})
	if err != nil {
		t.Fatal(err)
	}
	if leave.Operation != "hub.leave" {
		t.Fatalf("unexpected leave result: %#v", leave)
	}

	revoke, err := client.RevokeDevice(context.Background(), RevokeDeviceRequest{AdminCarrierBase: adminBaseForTest(), DeviceURA: "easynet:///r/example/device/dev-a", Reason: "credential/key rotation"})
	if err != nil {
		t.Fatal(err)
	}
	if revoke.Operation != "device.revoke" {
		t.Fatalf("unexpected revoke result: %#v", revoke)
	}

	deleted, err := client.DeleteDeviceSession(context.Background(), DeleteDeviceSessionRequest{AdminCarrierBase: adminBaseForTest(), SessionID: "dev-session-1", Reason: "done"})
	if err != nil {
		t.Fatal(err)
	}
	if deleted.Operation != "session.delete" {
		t.Fatalf("unexpected delete result: %#v", deleted)
	}
}

func TestAdminRejectsIncompleteCarrierAndSystemLifecycle(t *testing.T) {
	client, err := NewAdminClient(&memoryAdminTransport{agentStartInvocation: adminAgentStartInvocationJSON})
	if err != nil {
		t.Fatal(err)
	}
	if _, err := client.BuildAgentStartInvocation(context.Background(), AdminAgentStartRequest{Name: "codex", AgentType: "codex"}); err == nil {
		t.Fatal("expected incomplete carrier rejection")
	}
	if _, err := client.BuildAgentStartInvocation(context.Background(), AdminAgentStartRequest{AdminCarrierBase: adminBaseForTest(), Name: "device", AgentType: "codex"}); err == nil {
		t.Fatal("expected device system-agent rejection")
	}
	if _, err := client.BuildAgentStartInvocation(context.Background(), AdminAgentStartRequest{AdminCarrierBase: adminBaseForTest(), Name: "../codex", AgentType: "codex"}); err == nil {
		t.Fatal("expected path-like agent name rejection")
	}
	if _, err := client.BuildAgentStopInvocation(context.Background(), AdminAgentStopRequest{AdminCarrierBase: adminBaseForTest(), AgentURA: "easynet:///r/example/device/dev-a"}); err == nil {
		t.Fatal("expected non-agent URA rejection")
	}
	if _, err := client.CreatePairing(context.Background(), CreatePairingRequest{AdminCarrierBase: adminBaseForTest(), HubURA: "not-a-hub", DeviceURA: "easynet:///r/example/device/dev-a", ExpiresUnixMS: 1}); err == nil {
		t.Fatal("expected non-hub URA rejection")
	}
	if _, err := client.ValidatePairing(context.Background(), ValidatePairingRequest{AdminCarrierBase: adminBaseForTest(), Token: "../pairing", DeviceURA: "easynet:///r/example/device/dev-a"}); err == nil {
		t.Fatal("expected path-like pairing token rejection")
	}
	if _, err := client.RevokeDevice(context.Background(), RevokeDeviceRequest{AdminCarrierBase: adminBaseForTest(), DeviceURA: "easynet:///r/example/device/dev-a", Reason: "operator/key rotation"}); err != nil {
		t.Fatalf("reason text with slash rejected: %v", err)
	}
	if _, err := client.RevokeDevice(context.Background(), RevokeDeviceRequest{AdminCarrierBase: adminBaseForTest(), DeviceURA: "easynet:///r/example/device/dev-a", Reason: "operator\nrotation"}); err == nil {
		t.Fatal("expected control-character reason rejection")
	}
	if _, err := client.DeleteDeviceSession(context.Background(), DeleteDeviceSessionRequest{AdminCarrierBase: adminBaseForTest(), SessionID: "browser-session-1"}); err == nil {
		t.Fatal("expected browser session id rejection")
	}
}

func TestAdminClientCloseDelegatesOnceAndFailsClosed(t *testing.T) {
	transport := &memoryAdminTransport{agentListInvocation: adminAgentListInvocationJSON}
	client, err := NewAdminClient(transport)
	if err != nil {
		t.Fatal(err)
	}
	if err := client.Close(context.Background()); err != nil {
		t.Fatalf("Close: %v", err)
	}
	if err := client.Close(context.Background()); err != nil {
		t.Fatalf("second Close: %v", err)
	}
	if transport.closeCalls != 1 {
		t.Fatalf("close calls = %d, want 1", transport.closeCalls)
	}
	_, err = client.BuildAgentListInvocation(context.Background(), AdminAgentListRequest{AdminCarrierBase: adminBaseForTest()})
	if err == nil {
		t.Fatalf("BuildAgentListInvocation after close succeeded")
	}
	if !IsCode(err, ErrInvalidArgument) {
		t.Fatalf("error code = %v, want %s", err, ErrInvalidArgument)
	}
	if len(transport.seen) != 0 {
		t.Fatalf("transport called after close: %#v", transport.seen)
	}
}

const adminAgentListInvocationJSON = `{
  "caller_ura": "easynet:///r/example/agent/alice.sdk",
  "callee_ura": "easynet:///r/example/device/dev-a",
  "descriptor_ref": "easynet:///r/example/ability/device.dev-a.agent.list@1.0.0",
  "subject_ura": "easynet:///r/example/device/dev-a",
  "nonce_base64": "AQIDBAUGBwgJCgsMDQ4PEA==",
  "causal_context": {"form": "none"},
  "args": {},
  "content_type": "application/json",
  "metadata": {
    "request_id": "admin-agent-list-1",
    "profile": "admin_gateway",
    "system_ability": "agent.list",
    "carrier_owner": "daemon_sdk"
  }
}`

const adminAgentStartInvocationJSON = `{
  "caller_ura": "easynet:///r/example/agent/alice.sdk",
  "callee_ura": "easynet:///r/example/device/dev-a",
  "descriptor_ref": "easynet:///r/example/ability/device.dev-a.agent.start@1.0.0",
  "subject_ura": "easynet:///r/example/device/dev-a",
  "nonce_base64": "AQIDBAUGBwgJCgsMDQ4PEA==",
  "causal_context": {"form": "none"},
  "args": {"name": "codex", "agent_type": "codex", "model": "gpt-5", "label": "primary"},
  "content_type": "application/json",
  "metadata": {
    "request_id": "admin-agent-start-1",
    "profile": "admin_gateway",
    "system_ability": "agent.start",
    "carrier_owner": "daemon_sdk"
  }
}`

const adminAgentStopInvocationJSON = `{
  "caller_ura": "easynet:///r/example/agent/alice.sdk",
  "callee_ura": "easynet:///r/example/device/dev-a",
  "descriptor_ref": "easynet:///r/example/ability/device.dev-a.agent.stop@1.0.0",
  "subject_ura": "easynet:///r/example/device/dev-a",
  "nonce_base64": "AQIDBAUGBwgJCgsMDQ4PEA==",
  "causal_context": {"form": "none"},
  "args": {"name": "codex"},
  "content_type": "application/json",
  "metadata": {
    "request_id": "admin-agent-stop-1",
    "profile": "admin_gateway",
    "system_ability": "agent.stop",
    "carrier_owner": "daemon_sdk"
  }
}`

const adminAgentRefreshInvocationJSON = `{
  "caller_ura": "easynet:///r/example/agent/alice.sdk",
  "callee_ura": "easynet:///r/example/device/dev-a",
  "descriptor_ref": "easynet:///r/example/ability/device.dev-a.agent.refresh@1.0.0",
  "subject_ura": "easynet:///r/example/device/dev-a",
  "nonce_base64": "AQIDBAUGBwgJCgsMDQ4PEA==",
  "causal_context": {"form": "none"},
  "args": {"name": "codex"},
  "content_type": "application/json",
  "metadata": {
    "request_id": "admin-agent-refresh-1",
    "profile": "admin_gateway",
    "system_ability": "agent.refresh",
    "carrier_owner": "daemon_sdk"
  }
}`

const adminSessionListInvocationJSON = `{
  "caller_ura": "easynet:///r/example/agent/alice.sdk",
  "callee_ura": "easynet:///r/example/device/dev-a",
  "descriptor_ref": "easynet:///r/example/ability/device.dev-a.session.list@1.0.0",
  "subject_ura": "easynet:///r/example/device/dev-a",
  "nonce_base64": "AQIDBAUGBwgJCgsMDQ4PEA==",
  "causal_context": {"form": "none"},
  "args": {"include_terminated": false},
  "content_type": "application/json",
  "metadata": {
    "request_id": "admin-session-list-1",
    "profile": "admin_gateway",
    "system_ability": "session.list",
    "carrier_owner": "daemon_sdk"
  }
}`

const adminRevokeDeviceInvocationJSON = `{
  "caller_ura": "easynet:///r/example/agent/alice.sdk",
  "callee_ura": "easynet:///r/example/device/dev-a",
  "descriptor_ref": "easynet:///r/example/ability/device.dev-a.federation.revoke@1.0.0",
  "subject_ura": "easynet:///r/example/device/dev-a",
  "nonce_base64": "AQIDBAUGBwgJCgsMDQ4PEA==",
  "causal_context": {"form": "none"},
  "args": {
    "agent_ura": "easynet:///r/example/device/dev-a",
    "reason": "operator/key rotation"
  },
  "content_type": "application/json",
  "metadata": {
    "request_id": "admin-revoke-device-1",
    "profile": "admin_gateway",
    "system_ability": "federation.revoke",
    "carrier_owner": "daemon_sdk"
  }
}`

const adminGatewayStatusJSON = `{
  "profile": "admin_gateway",
  "gateway_id": "device:example:dev-a",
  "ready": true,
  "state": "ready",
  "process_live": true,
  "control_ready": true,
  "runtime_ready": true,
  "directory_ready": true,
  "trust_ready": true,
  "public_listener_ready": false,
  "listeners": [
    {"kind": "control", "endpoint": "/tmp/easynet-control.sock", "ready": true, "public": false},
    {"kind": "invocation", "endpoint": "/tmp/easynet-daemon.sock", "ready": true, "public": false}
  ],
  "identity": {"mode": "device", "realm": "example", "node_id": "dev-a"},
  "metadata": {
    "profile": "admin_gateway",
    "source": "daemon_lifecycle_status",
    "lifecycle_state": "running",
    "requires_public_listener": false
  }
}`

const adminAgentRecordsJSON = `{
  "profile": "admin_gateway",
  "kind": "agent_records",
  "state": "ok",
  "items": [{
    "name": "codex",
    "agent_ura": "easynet:///r/example/agent/alice.codex",
    "owner_ura": "easynet:///r/example/user/alice",
    "device_ura": null,
    "state": "registered",
    "runtime": "codex",
    "model": "gpt-5",
    "label": "primary",
    "abilities": [],
    "metadata": {
      "profile": "admin_gateway",
      "source": "agent.list",
      "root_path": "/tmp/easynet/agents/codex",
      "root_exists": true,
      "timeout_secs": 600
    }
  }],
  "next_cursor": null,
  "metadata": {"profile": "admin_gateway", "source": "agent.list", "count": 1}
}`

const adminLifecycleResultJSON = `{
  "profile": "admin_gateway",
  "kind": "agent_lifecycle_result",
  "operation": "agent.start",
  "state": "ok",
  "agent_ura": "easynet:///r/example/agent/alice.codex",
  "ack": null,
  "runtime_not_ready": false,
  "runtime_catalog_not_ready": false,
  "metadata": {
    "profile": "admin_gateway",
    "source": "agent_lifecycle",
    "runtime_registered": 3,
    "runtime_failed": 0,
    "runtime_removed": 0,
    "raw_result": {
      "agent_ura": "easynet:///r/example/agent/alice.codex",
      "replaced_prior": false,
      "runtime_registered": 3,
      "runtime_failed": 0,
      "runtime_removed": 0,
      "runtime_not_ready": false,
      "runtime_catalog_not_ready": false
    }
  }
}`

const adminJoinResultJSON = `{
  "profile": "admin_gateway",
  "kind": "hub_membership_result",
  "operation": "hub.join",
  "state": "ok",
  "device_ura": "easynet:///r/example/device/dev-a",
  "ack": true,
  "metadata": {"profile": "admin_gateway", "source": "hub.join"}
}`

const adminLeaveResultJSON = `{
  "profile": "admin_gateway",
  "kind": "hub_membership_result",
  "operation": "hub.leave",
  "state": "ok",
  "device_ura": "easynet:///r/example/device/dev-a",
  "ack": true,
  "metadata": {"profile": "admin_gateway", "source": "hub.leave"}
}`

const adminPairingPreflightJSON = `{
  "profile": "admin_gateway",
  "kind": "pairing_preflight",
  "state": "requires_pairing",
  "hub_ura": "easynet:///r/example/hub/main",
  "device_ura": "easynet:///r/example/device/dev-a",
  "pairing_required": true,
  "trust_ready": false,
  "scopes": ["invoke", "events"],
  "metadata": {"profile": "admin_gateway", "source": "pairing.preflight"}
}`

const adminPairingTokenJSON = `{
  "profile": "admin_gateway",
  "kind": "pairing_token",
  "token_id": "pair-token-1",
  "token": "pair-token-value",
  "hub_ura": "easynet:///r/example/hub/main",
  "device_ura": "easynet:///r/example/device/dev-a",
  "state": "issued",
  "expires_unix_ms": 1893456000000,
  "scopes": ["invoke", "events"],
  "metadata": {"profile": "admin_gateway", "source": "pairing.create"}
}`

const adminDeviceCredentialJSON = `{
  "profile": "admin_gateway",
  "kind": "device_credential",
  "credential_id": "cred-dev-a",
  "device_ura": "easynet:///r/example/device/dev-a",
  "hub_ura": "easynet:///r/example/hub/main",
  "state": "active",
  "issued_unix_ms": 1767225600000,
  "expires_unix_ms": 1893456000000,
  "scopes": ["invoke", "events"],
  "metadata": {"profile": "admin_gateway", "source": "pairing.validate"}
}`

const adminCredentialVerificationJSON = `{
  "profile": "admin_gateway",
  "kind": "device_credential_verification",
  "verified": true,
  "credential_id": "cred-dev-a",
  "device_ura": "easynet:///r/example/device/dev-a",
  "hub_ura": "easynet:///r/example/hub/main",
  "method": "daemon-trust-store",
  "metadata": {"profile": "admin_gateway", "source": "credential.verify"}
}`

const adminDeviceSessionJSON = `{
  "profile": "admin_gateway",
  "kind": "device_session",
  "session_id": "dev-session-1",
  "device_ura": "easynet:///r/example/device/dev-a",
  "hub_ura": "easynet:///r/example/hub/main",
  "state": "active",
  "session_kind": "remote_desktop",
  "created_unix_ms": 1767225600000,
  "expires_unix_ms": 1893456000000,
  "metadata": {"profile": "admin_gateway", "source": "session.create"}
}`

const adminDeviceSessionPageJSON = `{
  "profile": "admin_gateway",
  "kind": "device_sessions",
  "state": "ok",
  "items": [{
    "profile": "admin_gateway",
    "kind": "device_session",
    "session_id": "dev-session-1",
    "device_ura": "easynet:///r/example/device/dev-a",
    "hub_ura": "easynet:///r/example/hub/main",
    "state": "active",
    "session_kind": "remote_desktop",
    "created_unix_ms": 1767225600000,
    "expires_unix_ms": 1893456000000,
    "metadata": {"profile": "admin_gateway", "source": "session.list"}
  }],
  "next_cursor": null,
  "metadata": {"profile": "admin_gateway", "source": "session.list"}
}`

const adminRevokeDeviceResultJSON = `{
  "profile": "admin_gateway",
  "kind": "device_admin_result",
  "operation": "device.revoke",
  "state": "revoked",
  "device_ura": "easynet:///r/example/device/dev-a",
  "ack": true,
  "metadata": {"profile": "admin_gateway", "source": "device.revoke"}
}`

const adminDeleteSessionResultJSON = `{
  "profile": "admin_gateway",
  "kind": "device_admin_result",
  "operation": "session.delete",
  "state": "deleted",
  "device_ura": "easynet:///r/example/device/dev-a",
  "ack": true,
  "metadata": {"profile": "admin_gateway", "source": "session.delete"}
}`
