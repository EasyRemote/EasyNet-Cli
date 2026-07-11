//go:build easynet_live_smoke && easynet_cabi && cgo && !windows

package easynet

import (
	"context"
	"encoding/base64"
	"encoding/json"
	"os"
	"path/filepath"
	"strconv"
	"strings"
	"testing"
	"time"
)

// TestGoSDKLiveDaemonSmoke proves the generic C ABI v5 boundary through the
// public Go facade. Product/profile helpers are deliberately absent: complete
// Invocation descriptors are supplied to Runtime Core directly.
func TestGoSDKLiveDaemonSmoke(t *testing.T) {
	ctx, cancel := context.WithTimeout(context.Background(), 30*time.Second)
	defer cancel()

	libPath := requireLiveSmokeEnv(t, "EASYNET_GO_LIVE_SMOKE_LIB")
	daemonBin := requireLiveSmokeEnv(t, "EASYNET_GO_LIVE_SMOKE_DAEMON")
	home := os.Getenv("EASYNET_GO_LIVE_SMOKE_HOME")
	if home == "" {
		home = t.TempDir()
	} else if err := os.MkdirAll(home, 0o700); err != nil {
		t.Fatalf("create smoke home: %v", err)
	}
	realm, deviceID, deviceURA, trustPath := writeGoLiveSmokeIdentity(t, home)
	t.Setenv("HOME", home)
	t.Setenv("EASYNET_REALM_TRUST_PATH", trustPath)
	t.Setenv("EASYNET_PAGES_PORT", strconv.Itoa(19000+os.Getpid()%1000))

	transport, err := OpenCABIDaemonTransport(libPath)
	if err != nil {
		t.Fatalf("OpenCABIDaemonTransport: %v", err)
	}
	defer func() {
		if err := transport.Close(context.Background()); err != nil {
			t.Fatalf("close C ABI transport: %v", err)
		}
	}()

	control, err := NewDaemonControl(transport)
	if err != nil {
		t.Fatalf("NewDaemonControl: %v", err)
	}
	logPath := filepath.Join(home, ".easynet", "go-sdk-smoke-daemon.log")
	handle, err := control.Start(ctx, StartConfig{
		Mode:      ModeDevice,
		Realm:     realm,
		DeviceID:  deviceID,
		DaemonBin: daemonBin,
		LogPath:   logPath,
		Env: map[string]string{
			"HOME":                     home,
			"EASYNET_REALM_TRUST_PATH": trustPath,
			"EASYNET_PAGES_PORT":       os.Getenv("EASYNET_PAGES_PORT"),
		},
	})
	if err != nil {
		dumpGoLiveSmokeLog(t, logPath)
		t.Fatalf("daemon start: %v", err)
	}
	defer func() {
		stopCtx, stopCancel := context.WithTimeout(context.Background(), 8*time.Second)
		defer stopCancel()
		if err := handle.Stop(stopCtx, StopOptions{}); err != nil {
			t.Fatalf("daemon stop: %v", err)
		}
	}()

	status, err := handle.Status(ctx)
	if err != nil {
		t.Fatalf("daemon status: %v", err)
	}
	if status.Endpoints.InvocationEndpoint == "" {
		t.Fatalf("daemon status has no invocation endpoint: %#v", status)
	}

	runtime, err := handle.OpenRuntime(ctx, ConnectOptions{})
	if err != nil {
		t.Fatalf("open runtime: %v", err)
	}
	defer func() {
		if err := runtime.Close(context.Background()); err != nil {
			t.Fatalf("runtime close: %v", err)
		}
	}()

	healthTransport, ok := runtime.transport.(HealthTransport)
	if !ok {
		t.Fatal("runtime transport does not expose health")
	}
	healthClient, err := NewHealthClient(healthTransport)
	if err != nil {
		t.Fatalf("NewHealthClient: %v", err)
	}
	health, err := healthClient.RuntimeHealth(ctx)
	if err != nil {
		t.Fatalf("RuntimeHealth: %v", err)
	}
	if !health.Ready() {
		t.Fatalf("runtime health is not ready: %#v", health)
	}

	unary, err := runtime.Invoke(ctx, goLiveSmokeDraft(t, deviceURA, "observe.health", map[string]any{"smoke": "go-sdk"}, 1))
	if err != nil {
		t.Fatalf("RuntimeClient.Invoke: %v", err)
	}
	if !unary.OK() || unary.TerminalState() != "Completed" {
		t.Fatalf("unary result = ok:%v state:%q failure:%#v", unary.OK(), unary.TerminalState(), unary.Failure())
	}
	var unaryOutput map[string]any
	if err := json.Unmarshal(unary.OutputJSON(), &unaryOutput); err != nil {
		t.Fatalf("decode unary output: %v", err)
	}
	if unaryOutput["status"] != "healthy" {
		t.Fatalf("unary status = %#v", unaryOutput)
	}
	t.Log("unary RuntimeClient.Invoke OK")

	prepared, _, err := runtime.Prepare(ctx, goLiveSmokeDraft(t, deviceURA, "sdk.live_smoke_missing", map[string]any{"smoke": "go-sdk-terminal-failure"}, 65), PrepareOptions{})
	if err != nil {
		t.Fatalf("typed terminal failure prepare: %v", err)
	}
	signed, err := prepared.SignWithCallerSignature(InvocationSignature{
		Algorithm:       "ed25519",
		SignatureBase64: "c2lnbmF0dXJl",
		KeyIDHint:       "go-sdk-live-smoke-invalid-signature",
	})
	if err != nil {
		t.Fatalf("typed terminal failure sign: %v", err)
	}
	failureHandle, err := runtime.SubmitSigned(ctx, signed)
	if err != nil {
		t.Fatalf("typed terminal failure submit: %v", err)
	}
	terminalFailure, err := runtime.Await(ctx, failureHandle)
	if err != nil {
		t.Fatalf("typed terminal failure await: %v", err)
	}
	failure := terminalFailure.Failure()
	if terminalFailure.OK() || terminalFailure.TerminalState() != "Failed" || failure == nil {
		t.Fatalf("typed terminal failure result = ok:%v state:%q failure:%#v", terminalFailure.OK(), terminalFailure.TerminalState(), failure)
	}
	if failure.Code() == "" || failure.Stage() == "" || failure.Message() == "" {
		t.Fatalf("typed terminal failure is incomplete: %#v", failure)
	}
	t.Logf("typed terminal failure decoded: code=%s stage=%s", failure.Code(), failure.Stage())

	browser, err := runtime.Invoke(ctx, goLiveSmokeDraft(t, deviceURA, "browser.open_session", map[string]any{"url": "https://example.com"}, 17))
	if err != nil {
		t.Fatalf("browser.open_session: %v", err)
	}
	var browserOutput map[string]any
	if err := json.Unmarshal(browser.OutputJSON(), &browserOutput); err != nil {
		t.Fatalf("decode browser output: %v", err)
	}
	sessionURA, ok := browserOutput["session_ura"].(string)
	if !ok || sessionURA == "" {
		t.Fatalf("browser session_ura missing: %#v", browserOutput)
	}
	stream, err := runtime.InvokeStream(ctx, goLiveSmokeDraft(t, deviceURA, "browser.capture_viewport", map[string]any{"session_ura": sessionURA}, 33))
	if err != nil {
		t.Fatalf("browser.capture_viewport stream: %v", err)
	}
	streamCtx, streamCancel := context.WithTimeout(ctx, 5*time.Second)
	streamEvent, err := stream.Next(streamCtx)
	streamCancel()
	if err != nil {
		t.Fatalf("stream next: %v", err)
	}
	if streamEvent.Kind() != "chunk" {
		t.Fatalf("stream event kind = %q", streamEvent.Kind())
	}
	if _, err := stream.Cancel(ctx, "go-sdk-live-smoke"); err != nil {
		t.Fatalf("stream cancel: %v", err)
	}
	if err := stream.Close(ctx); err != nil {
		t.Fatalf("stream close: %v", err)
	}
	t.Log("StreamHandle received daemon frame")
}

func requireLiveSmokeEnv(t *testing.T, name string) string {
	t.Helper()
	value := os.Getenv(name)
	if value == "" {
		t.Fatalf("%s is required", name)
	}
	return value
}

func writeGoLiveSmokeIdentity(t *testing.T, home string) (string, string, string, string) {
	t.Helper()
	stateDir := filepath.Join(home, ".easynet")
	if err := os.RemoveAll(stateDir); err != nil {
		t.Fatalf("reset state dir: %v", err)
	}
	if err := os.MkdirAll(stateDir, 0o700); err != nil {
		t.Fatalf("create state dir: %v", err)
	}
	realm := "cli"
	deviceID := "local"
	deviceURA := "easynet:///r/cli/device/local"
	writeGoLiveSmokeJSON(t, filepath.Join(stateDir, "credentials.json"), map[string]any{
		"node_id": deviceID, "credential_token": "go-sdk-smoke-token",
		"hub_endpoint": "https://127.0.0.1:50443", "realm": realm,
		"username": "go-sdk-smoke-user", "user_id": "go-sdk-smoke-user-id",
	})
	daemonConfig := `[daemon]
mode = "device"
realm = "cli"
hub_endpoint = "https://127.0.0.1:50443"
uds_path = "~/.easynet/custom-invocation.sock"
`
	if err := os.WriteFile(filepath.Join(stateDir, "daemon-config.toml"), []byte(daemonConfig), 0o600); err != nil {
		t.Fatalf("write daemon-config.toml: %v", err)
	}
	fakePublicKey := base64.StdEncoding.EncodeToString(make([]byte, 32))
	trustPath := filepath.Join(stateDir, "realm-trust.toml")
	trust := `[[trusted_agent]]
agent_ura = "` + deviceURA + `"
public_key_b64 = "` + fakePublicKey + `"
role = "device"
added_at_unix_ms = 0
`
	if err := os.WriteFile(trustPath, []byte(trust), 0o600); err != nil {
		t.Fatalf("write realm trust: %v", err)
	}
	return realm, deviceID, deviceURA, trustPath
}

func writeGoLiveSmokeJSON(t *testing.T, path string, value any) {
	t.Helper()
	raw, err := json.MarshalIndent(value, "", "  ")
	if err != nil {
		t.Fatalf("marshal %s: %v", path, err)
	}
	if err := os.WriteFile(path, append(raw, '\n'), 0o600); err != nil {
		t.Fatalf("write %s: %v", path, err)
	}
}

func goLiveSmokeDraft(t *testing.T, deviceURA, ability string, args map[string]any, nonceStart byte) InvocationDraft {
	t.Helper()
	realmAndDevice := strings.TrimPrefix(deviceURA, "easynet:///r/")
	parts := strings.SplitN(realmAndDevice, "/device/", 2)
	if len(parts) != 2 {
		t.Fatalf("invalid live-smoke device URA %q", deviceURA)
	}
	descriptorRef := "easynet:///r/" + parts[0] + "/ability/device." + parts[1] + "." + ability + "@1.0.0"
	draft, err := NewInvocationBuilder().
		WithCallerURA(deviceURA).
		WithCalleeURA(deviceURA).
		WithDescriptorRef(descriptorRef).
		WithSubjectURA(deviceURA).
		WithNonceBase64(goLiveSmokeNonce(nonceStart)).
		WithCausalContext(map[string]any{"form": "none"}).
		WithContentType("application/json").
		WithJSONArgs(args).
		Build()
	if err != nil {
		t.Fatalf("build draft(%s): %v", ability, err)
	}
	return draft
}

func goLiveSmokeNonce(start byte) string {
	nonce := make([]byte, 16)
	for i := range nonce {
		nonce[i] = start + byte(i)
	}
	return base64.StdEncoding.EncodeToString(nonce)
}

func dumpGoLiveSmokeLog(t *testing.T, path string) {
	t.Helper()
	raw, err := os.ReadFile(path)
	if err != nil {
		t.Logf("daemon log unavailable at %s: %v", path, err)
		return
	}
	t.Logf("daemon log:\n%s", string(raw))
}
