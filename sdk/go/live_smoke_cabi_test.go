//go:build runtime_live_smoke && runtime_cabi && cgo && !windows

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

// TestGoSDKLiveDaemonSmoke proves the base C ABI v7 boundary and the additive
// ABI v9 leased-stream extension through the public Go facade. Product/profile
// helpers are deliberately absent: complete Invocation descriptors are
// supplied to Runtime Core directly.
func TestGoSDKLiveDaemonSmoke(t *testing.T) {
	ctx, cancel := context.WithTimeout(context.Background(), 30*time.Second)
	defer cancel()

	libPath := requireLiveSmokeEnv(t, "EASYNET_GO_LIVE_SMOKE_LIB")
	daemonBin := requireLiveSmokeEnv(t, "EASYNET_GO_LIVE_SMOKE_DAEMON")
	repoRoot := os.Getenv("EASYNET_GO_LIVE_SMOKE_REPO_ROOT")
	if repoRoot == "" {
		repoRoot = filepath.Dir(filepath.Dir(filepath.Dir(daemonBin)))
	}
	home := os.Getenv("EASYNET_GO_LIVE_SMOKE_HOME")
	if home == "" {
		home = t.TempDir()
	} else if err := os.MkdirAll(home, 0o700); err != nil {
		t.Fatalf("create smoke home: %v", err)
	}
	realm, deviceID, deviceURA, userURA, trustPath := writeGoLiveSmokeIdentity(t, home)
	t.Setenv("HOME", home)
	t.Setenv("EASYNET_REALM_TRUST_PATH", trustPath)
	t.Setenv("EASYNET_PAGES_PORT", strconv.Itoa(19000+os.Getpid()%1000))

	transport, err := openCABIRuntimeLifecycleTransport(libPath)
	if err != nil {
		t.Fatalf("openCABIRuntimeLifecycleTransport: %v", err)
	}
	defer func() {
		if err := transport.Close(context.Background()); err != nil {
			t.Fatalf("close C ABI transport: %v", err)
		}
	}()

	control, err := NewRuntimeHost(transport)
	if err != nil {
		t.Fatalf("NewRuntimeHost: %v", err)
	}
	logPath := filepath.Join(home, ".easynet", "go-sdk-smoke-daemon.log")
	handle, err := control.StartRuntime(ctx, testRuntimeHostStartRequest{
		payload: map[string]any{
			"mode":                "edge",
			"realm":               realm,
			"runtime_instance_id": deviceID,
			"runtime_bin":         daemonBin,
			"working_dir":         repoRoot,
			"log_path":            logPath,
			"env": map[string]string{
				"HOME":                     home,
				"EASYNET_REALM_TRUST_PATH": trustPath,
				"EASYNET_PAGES_PORT":       os.Getenv("EASYNET_PAGES_PORT"),
			},
		},
	})
	if err != nil {
		dumpGoLiveSmokeLog(t, logPath)
		t.Fatalf("daemon start: %v", err)
	}
	defer func() {
		stopCtx, stopCancel := context.WithTimeout(context.Background(), 8*time.Second)
		defer stopCancel()
		if err := handle.StopRuntime(stopCtx, RuntimeHostStopOptions{}); err != nil {
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

	duplicateDraft := goLiveSmokeDraft(t, runtime, realm, deviceID, userURA, deviceURA, "observe.health", map[string]any{"smoke": "duplicate-prepare"}, 2)
	firstPrepared, _, err := runtime.Prepare(ctx, duplicateDraft, PrepareOptions{})
	if err != nil {
		t.Fatalf("first duplicate-draft prepare: %v", err)
	}
	secondPrepared, _, err := runtime.Prepare(ctx, duplicateDraft, PrepareOptions{})
	if err != nil {
		t.Fatalf("second duplicate-draft prepare: %v", err)
	}
	if firstPrepared.PreparedID() == secondPrepared.PreparedID() {
		t.Fatalf("duplicate draft reused prepared id %q", firstPrepared.PreparedID())
	}
	if firstPrepared.RequestID() == secondPrepared.RequestID() {
		t.Fatalf("duplicate draft reused request id %q", firstPrepared.RequestID())
	}
	if firstPrepared.CanonicalHashHex() != secondPrepared.CanonicalHashHex() {
		t.Fatalf(
			"duplicate draft changed canonical hash: %q != %q",
			firstPrepared.CanonicalHashHex(), secondPrepared.CanonicalHashHex(),
		)
	}
	t.Log("duplicate draft allocated independent prepared and request ids")

	unary, err := runtime.Invoke(ctx, goLiveSmokeDraft(t, runtime, realm, deviceID, userURA, deviceURA, "observe.health", map[string]any{"smoke": "go-sdk"}, 1))
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

	prepared, _, err := runtime.Prepare(ctx, goLiveSmokeDraft(t, runtime, realm, deviceID, userURA, deviceURA, "observe.health", map[string]any{"smoke": "go-sdk-terminal-failure"}, 65), PrepareOptions{})
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

	eventProvider, err := NewRuntimeHandleEventProvider(runtime)
	if err != nil {
		t.Fatalf("NewRuntimeHandleEventProvider: %v", err)
	}
	eventClient, err := NewRuntimeEventClient(eventProvider)
	if err != nil {
		t.Fatalf("NewRuntimeEventClient: %v", err)
	}
	eventPage, err := eventClient.Read(ctx, RuntimeEventReadRequest{
		Handle: failureHandle,
		Limit:  8,
	})
	if err != nil {
		t.Fatalf("RuntimeEventClient.Read: %v", err)
	}
	if !eventPage.Terminal || eventPage.State != RuntimeEventStreamTerminal {
		t.Fatalf("runtime event page is not terminal: %#v", eventPage)
	}
	if len(eventPage.Events) == 0 {
		t.Fatalf("runtime event page is empty: %#v", eventPage)
	}
	lastEvent := eventPage.Events[len(eventPage.Events)-1]
	if !lastEvent.Terminal || lastEvent.State != "Failed" {
		t.Fatalf("runtime event terminal projection = %#v", lastEvent)
	}
	if eventPage.Cursor.Sequence != lastEvent.Sequence {
		t.Fatalf("runtime event cursor = %d, want %d", eventPage.Cursor.Sequence, lastEvent.Sequence)
	}
	t.Log("RuntimeEventClient read live daemon handle events")

	stream, err := runtime.InvokeStream(ctx, goLiveSmokeDraft(t, runtime, realm, deviceID, userURA, deviceURA, "session.attach", map[string]any{"session_id": "go-sdk-live-smoke-no-such-session"}, 33, "stream"))
	if err != nil {
		t.Fatalf("session.attach stream: %v", err)
	}
	streamCtx, streamCancel := context.WithTimeout(ctx, 5*time.Second)
	streamEvent, err := stream.Next(streamCtx)
	streamCancel()
	if err != nil {
		t.Fatalf("stream next: %v", err)
	}
	if streamEvent.Kind() != "terminal" || !streamEvent.Terminal() {
		t.Fatalf("stream terminal event = kind:%q terminal:%v state:%q", streamEvent.Kind(), streamEvent.Terminal(), streamEvent.State())
	}
	if streamEvent.TerminalReceiptJSON() == nil {
		t.Fatalf("stream terminal event is missing terminal receipt")
	}
	if err := stream.Close(ctx); err != nil {
		t.Fatalf("stream close: %v", err)
	}
	t.Log("StreamHandle received receipt-backed daemon terminal frame")

	leasedSubjectURA, err := descriptorBoundSubjectURA(
		ctx,
		NewCanonicalAddressing(),
		userURA,
		"resource.watch_remote_targets",
	)
	if err != nil {
		t.Fatalf("project resource.watch_remote_targets subject: %v", err)
	}
	mediaCalleeURA := goLiveSmokeSystemAgentCallee(t, realm, deviceID, "resource.watch_remote_targets")
	managedSigning, err := NewManagedSigningClient(ManagedSigningClientOptions{
		SocketPath: filepath.Join(home, ".easynet", "keyring.sock"),
		Timeout:    2 * time.Second,
	})
	if err != nil {
		t.Fatalf("NewManagedSigningClient: %v", err)
	}
	signingIdentity, err := managedSigning.ActiveSignerForSubject(userURA, "user_signing.cli")
	if err != nil {
		t.Fatalf("resolve paired-user runtime signing identity: %v", err)
	}
	authorityClient, err := NewCanonicalAuthorityClient(signingIdentity)
	if err != nil {
		t.Fatalf("NewCanonicalAuthorityClient: %v", err)
	}
	defer func() {
		if err := authorityClient.Close(context.Background()); err != nil {
			t.Fatalf("close authority client: %v", err)
		}
	}()
	nowMS := time.Now().UnixMilli()
	delegation, err := authorityClient.MintDelegationProof(ctx, DelegationRequest{
		IssuerURA:   userURA,
		SubjectURA:  leasedSubjectURA,
		CallerURA:   userURA,
		Audience:    mediaCalleeURA,
		Scopes:      []string{"resource.watch_remote_targets"},
		IssuedAtMS:  nowMS,
		ExpiresAtMS: nowMS + int64((5*time.Minute)/time.Millisecond),
	})
	if err != nil {
		t.Fatalf("mint resource.watch_remote_targets delegation: %v", err)
	}
	abilityClient, err := NewRuntimeAbilityClient(runtime, NewCanonicalAddressing())
	if err != nil {
		t.Fatalf("NewRuntimeAbilityClient: %v", err)
	}
	leasedStream, err := abilityClient.OpenLeasedStream(ctx, RuntimeCallContext{
		CallerURA:     userURA,
		CalleeURA:     mediaCalleeURA,
		SubjectURA:    userURA,
		NonceBase64:   goLiveSmokeNonce(49),
		CausalContext: map[string]any{"form": "none"},
		Authority:     delegation,
	}, "resource.watch_remote_targets", map[string]any{"max_events": 1, "types": []string{"display"}})
	if err != nil {
		t.Fatalf("resource.watch_remote_targets leased stream: %v", err)
	}
	leasedCtx, leasedCancel := context.WithTimeout(ctx, 5*time.Second)
	leasedEvent, err := leasedStream.Next(leasedCtx)
	leasedCancel()
	if err != nil {
		_ = leasedStream.Close(context.Background())
		t.Fatalf("leased stream data next: %v", err)
	}
	if leasedEvent.Kind() != "data" || leasedEvent.Terminal() {
		_ = leasedEvent.Release()
		_ = leasedStream.Close(context.Background())
		t.Fatalf(
			"leased stream data event = kind:%q terminal:%v state:%q",
			leasedEvent.Kind(), leasedEvent.Terminal(), leasedEvent.State(),
		)
	}
	if leasedEvent.PayloadContentType() != "application/json" || leasedEvent.Payload() == nil {
		_ = leasedEvent.Release()
		_ = leasedStream.Close(context.Background())
		t.Fatalf(
			"leased stream payload = content-type:%q payload:%#v",
			leasedEvent.PayloadContentType(), leasedEvent.Payload(),
		)
	}
	leasedPayload, err := leasedEvent.Payload().ToBytes()
	if err != nil {
		_ = leasedStream.Close(context.Background())
		t.Fatalf("copy and release leased payload: %v", err)
	}
	if !leasedEvent.Payload().Released() {
		_ = leasedStream.Close(context.Background())
		t.Fatal("leased payload owner remained live after ToBytes")
	}
	var inventoryEvent map[string]any
	if err := json.Unmarshal(leasedPayload, &inventoryEvent); err != nil {
		_ = leasedStream.Close(context.Background())
		t.Fatalf("decode leased inventory payload: %v", err)
	}
	if inventoryEvent["event_type"] == "" || inventoryEvent["resources"] == nil {
		_ = leasedStream.Close(context.Background())
		t.Fatalf("leased inventory payload is incomplete: %#v", inventoryEvent)
	}
	leasedTerminalCtx, leasedTerminalCancel := context.WithTimeout(ctx, 5*time.Second)
	leasedTerminal, err := leasedStream.Next(leasedTerminalCtx)
	leasedTerminalCancel()
	if err != nil {
		_ = leasedStream.Close(context.Background())
		t.Fatalf("leased stream terminal next: %v", err)
	}
	defer func() { _ = leasedTerminal.Release() }()
	if leasedTerminal.Kind() != "terminal" || !leasedTerminal.Terminal() || leasedTerminal.TerminalReceiptJSON() == nil {
		_ = leasedStream.Close(context.Background())
		t.Fatalf(
			"leased stream terminal event = kind:%q terminal:%v receipt:%s",
			leasedTerminal.Kind(), leasedTerminal.Terminal(), leasedTerminal.TerminalReceiptJSON(),
		)
	}
	if err := leasedStream.Close(ctx); err != nil {
		t.Fatalf("leased stream close: %v", err)
	}
	t.Log("ABI v9 LeasedStreamHandle received raw daemon payload and receipt-backed terminal frame")
}

func requireLiveSmokeEnv(t *testing.T, name string) string {
	t.Helper()
	value := os.Getenv(name)
	if value == "" {
		t.Fatalf("%s is required", name)
	}
	return value
}

func writeGoLiveSmokeIdentity(t *testing.T, home string) (string, string, string, string, string) {
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
	userID := "go-sdk-smoke-user-id"
	deviceURA := "easynet:///r/cli/device/local"
	userURA := "easynet:///r/cli/user/" + userID
	writeGoLiveSmokeJSON(t, filepath.Join(stateDir, "credentials.json"), map[string]any{
		"node_id": deviceID, "credential_token": "go-sdk-smoke-token",
		"hub_endpoint": "https://127.0.0.1:50443", "realm": realm,
		"username": "go-sdk-smoke-user", "user_id": userID,
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
	return realm, deviceID, deviceURA, userURA, trustPath
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

func goLiveSmokeDraft(t *testing.T, runtime *RuntimeClient, realm, deviceID, userURA, deviceURA, ability string, args map[string]any, nonceStart byte, callMode ...string) InvocationDraft {
	t.Helper()
	mode := "rpc"
	if len(callMode) > 0 {
		mode = callMode[0]
	}
	calleeURA := goLiveSmokeSystemAgentCallee(t, realm, deviceID, ability)
	descriptorRef := goLiveSmokeDescriptorRef(t, runtime, userURA, calleeURA, deviceURA, ability, mode)
	draft, err := NewInvocationBuilder().
		WithCallerURA(userURA).
		WithCalleeURA(calleeURA).
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

func goLiveSmokeSystemAgentCallee(t *testing.T, realm, deviceID, ability string) string {
	t.Helper()
	systemAgentID := ""
	switch ability {
	case "observe.health":
		systemAgentID = "runtime-health"
	case "session.attach":
		systemAgentID = "session"
	case "resource.watch_remote_targets":
		systemAgentID = "media"
	default:
		t.Fatalf("Go SDK smoke does not know SystemAgent owner for %s", ability)
	}
	return "easynet:///r/" + realm + "/agent/device." + deviceID + "." + systemAgentID
}

func goLiveSmokeDescriptorRef(t *testing.T, runtime *RuntimeClient, callerURA, calleeURA, subjectURA, ability, callMode string) string {
	t.Helper()
	if runtime == nil {
		t.Fatalf("live-smoke RuntimeClient missing from test context")
	}
	descriptorRef, err := runtime.ResolveDescriptorRef(context.Background(), RuntimeDescriptorRefRequest{
		CalleeURA:  calleeURA,
		CallerURA:  callerURA,
		SubjectURA: subjectURA,
		Ability:    ability,
		CallMode:   callMode,
	})
	if err != nil {
		t.Fatalf("resolve descriptor_ref(%s): %v", ability, err)
	}
	return descriptorRef
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
