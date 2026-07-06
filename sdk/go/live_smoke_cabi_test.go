//go:build easynet_live_smoke && easynet_cabi && cgo && !windows

package easynet

import (
	"context"
	"encoding/base64"
	"encoding/json"
	"os"
	"path/filepath"
	"strconv"
	"testing"
	"time"
)

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
	t.Setenv("EASYNET_PAGES_PORT", goLiveSmokePagesPort())

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

	runtime, err := handle.OpenRuntime(ctx, ConnectOptions{})
	if err != nil {
		t.Fatalf("open runtime: %v", err)
	}
	defer func() {
		if err := runtime.Close(context.Background()); err != nil {
			t.Fatalf("runtime close: %v", err)
		}
	}()

	identityTransport, err := transport.OpenIdentityTransport(ctx, handle.HandleID())
	if err != nil {
		t.Fatalf("OpenIdentityTransport: %v", err)
	}
	defer func() {
		if err := identityTransport.Close(context.Background()); err != nil {
			t.Fatalf("identity transport close: %v", err)
		}
	}()
	identity, err := NewIdentityClient(identityTransport)
	if err != nil {
		t.Fatalf("NewIdentityClient: %v", err)
	}

	healthTransport, ok := runtime.transport.(HealthTransport)
	if !ok {
		t.Fatalf("runtime transport does not expose health")
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

	unary, err := runtime.Invoke(ctx, goLiveSmokeDraft(t, identity, deviceURA, "observe.health", map[string]any{"smoke": "go-sdk"}, 1))
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

	browser, err := runtime.Invoke(ctx, goLiveSmokeDraft(t, identity, deviceURA, "browser.open_session", map[string]any{"url": "https://example.com"}, 17))
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
	stream, err := runtime.InvokeStream(ctx, goLiveSmokeDraft(t, identity, deviceURA, "browser.capture_viewport", map[string]any{"session_ura": sessionURA}, 33))
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
	var streamPayload map[string]any
	if err := json.Unmarshal(streamEvent.PayloadJSON(), &streamPayload); err != nil {
		t.Fatalf("decode stream payload: %v", err)
	}
	if streamPayload["is_placeholder"] != true {
		t.Fatalf("stream payload = %#v", streamPayload)
	}
	if _, err := stream.Cancel(ctx, "go-sdk-live-smoke"); err != nil {
		t.Fatalf("stream cancel: %v", err)
	}
	if err := stream.Close(ctx); err != nil {
		t.Fatalf("stream close: %v", err)
	}
	t.Log("StreamHandle received daemon frame")

	downloadPath := filepath.Join(home, ".easynet", "go-sdk-smoke-download.bin")
	downloadBytes := []byte("go sdk bidi proof\n")
	if err := os.WriteFile(downloadPath, downloadBytes, 0o600); err != nil {
		t.Fatalf("write bidi payload: %v", err)
	}
	resourceRef, err := identity.BuildResourceRef(ctx, LocalResourceRefRequest{Path: downloadPath, Capability: "read"})
	if err != nil {
		t.Fatalf("BuildResourceRef: %v", err)
	}
	bidi, err := runtime.OpenBidi(
		ctx,
		goLiveSmokeDraft(t, identity, deviceURA, "fs.transfer", map[string]any{
			"mode":         "download",
			"resource_ref": resourceRef,
		}, 49),
		[]BidiStreamDescriptor{{
			StreamID:    1,
			ContentType: "application/octet-stream",
			Ordering:    "STRICT",
		}},
	)
	if err != nil {
		t.Fatalf("fs.transfer open bidi: %v", err)
	}
	if _, err := bidi.CloseSend(ctx); err != nil {
		t.Fatalf("bidi close-send: %v", err)
	}
	sawBinary := false
	sawTerminal := false
	deadline := time.Now().Add(8 * time.Second)
	for time.Now().Before(deadline) && !(sawBinary && sawTerminal) {
		recvCtx, recvCancel := context.WithTimeout(ctx, time.Second)
		frame, err := bidi.Receive(recvCtx)
		recvCancel()
		if err != nil {
			t.Fatalf("bidi receive: %v", err)
		}
		if frame.Kind() == "binary_chunk" {
			sawBinary = true
		}
		if frame.Terminal() {
			sawTerminal = true
		}
	}
	if !sawBinary || !sawTerminal {
		t.Fatalf("bidi did not observe data+terminal: binary=%v terminal=%v state=%s", sawBinary, sawTerminal, bidi.State())
	}
	if err := bidi.Close(ctx); err != nil {
		t.Fatalf("bidi close: %v", err)
	}
	t.Log("BidiSession received data and terminal frame")
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
	credentials := map[string]any{
		"node_id":          deviceID,
		"credential_token": "go-sdk-smoke-token",
		"hub_endpoint":     "https://127.0.0.1:50443",
		"realm":            realm,
		"username":         "go-sdk-smoke-user",
		"user_id":          "go-sdk-smoke-user-id",
	}
	writeGoLiveSmokeJSON(t, filepath.Join(stateDir, "credentials.json"), credentials)
	daemonConfig := `[daemon]
mode = "device"
realm = "cli"
hub_endpoint = "https://127.0.0.1:50443"
uds_path = "~/.easynet/custom-invocation.sock"
`
	if err := os.WriteFile(filepath.Join(stateDir, "daemon-config.toml"), []byte(daemonConfig), 0o600); err != nil {
		t.Fatalf("write daemon-config.toml: %v", err)
	}
	publicKeyBytes := make([]byte, 32)
	for i := range publicKeyBytes {
		publicKeyBytes[i] = 1
	}
	fakePublicKey := base64.StdEncoding.EncodeToString(publicKeyBytes)
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
	raw = append(raw, '\n')
	if err := os.WriteFile(path, raw, 0o600); err != nil {
		t.Fatalf("write %s: %v", path, err)
	}
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

func goLiveSmokeDraft(t *testing.T, identity *IdentityClient, deviceURA string, ability string, args map[string]any, nonceStart byte) InvocationDraft {
	t.Helper()
	descriptorRef, err := identity.OwnerAbilityDescriptorRef(context.Background(), deviceURA, ability, "1.0.0")
	if err != nil {
		t.Fatalf("OwnerAbilityDescriptorRef(%s): %v", ability, err)
	}
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

func goLiveSmokePagesPort() string {
	return strconv.Itoa(19000 + os.Getpid()%1000)
}
