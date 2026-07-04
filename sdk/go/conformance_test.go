package easynet

import (
	"context"
	"encoding/json"
	"os"
	"path/filepath"
	"runtime"
	"strings"
	"testing"
)

const sharedConformanceCaseRoot = "sdk/conformance/cases"
const sharedConformanceFixtureRoot = "sdk/conformance/fixtures"

func TestGoFacadeExecutesSharedRuntimeCoreConformanceCases(t *testing.T) {
	root := repositoryRoot(t)

	completeTupleCase := sharedCase(t, root, "invocation-complete-tuple.yaml")
	requireCaseID(t, completeTupleCase, "invocation/complete_tuple")
	requireCaseAction(t, completeTupleCase, "build_invocation")
	requireCaseAction(t, completeTupleCase, "remove_field")
	requireCaseAction(t, completeTupleCase, "prepare")
	requireCaseFixture(t, completeTupleCase, "invocation.complete.v4.json")
	requireCaseExpectation(t, completeTupleCase, "error_code: InvalidArgument")

	draft, err := NewInvocationDraftFromJSON(sharedFixture(t, root, "invocation.complete.v4.json"))
	if err != nil {
		t.Fatalf("NewInvocationDraftFromJSON(shared fixture): %v", err)
	}
	if draft.CallerURA() != "easynet:///r/example/agent/alice.sdk" || !draft.HasJSONArgs() {
		t.Fatalf("unexpected invocation draft from shared fixture: %#v", draft)
	}

	missingCaller := map[string]any{}
	if err := json.Unmarshal(sharedFixture(t, root, "invocation.complete.v4.json"), &missingCaller); err != nil {
		t.Fatalf("decode invocation fixture for remove_field action: %v", err)
	}
	delete(missingCaller, "caller_ura")
	missingCallerRaw, err := json.Marshal(missingCaller)
	if err != nil {
		t.Fatalf("encode invocation fixture after remove_field action: %v", err)
	}
	if _, err := NewInvocationDraftFromJSON(missingCallerRaw); !IsCode(err, ErrInvalidArgument) {
		t.Fatalf("remove_field caller_ura did not produce InvalidArgument: %v", err)
	}

	prepared, err := NewPreparedInvocationFromJSON(sharedFixture(t, root, "prepared.signing-material.v4.json"))
	if err != nil {
		t.Fatalf("NewPreparedInvocationFromJSON(shared fixture): %v", err)
	}
	if prepared.SubmitReady() || prepared.SigningMaterial().Algorithm() != "ed25519" {
		t.Fatalf("unexpected prepared invocation from shared fixture: %#v", prepared)
	}

	runtimeErr, err := DecodeDaemonErrorJSON(sharedFixture(t, root, "runtime.error.v4.json"))
	if err != nil {
		t.Fatalf("DecodeDaemonErrorJSON(shared fixture): %v", err)
	}
	if runtimeErr == nil || runtimeErr.Code != ErrInvalidArgument || runtimeErr.Retry != RetryNever {
		t.Fatalf("unexpected runtime error from shared fixture: %#v", runtimeErr)
	}

	healthCase := sharedCase(t, root, "health-api-vs-runtime.yaml")
	requireCaseID(t, healthCase, "health/api_vs_runtime")
	requireCaseAction(t, healthCase, "read_health")
	requireCaseFixture(t, healthCase, "health.ready.v4.json")
	requireCaseExpectation(t, healthCase, "api_ready_field: api_ready")
	requireCaseExpectation(t, healthCase, "runtime_ready_field: runtime_ready")

	health, err := NewRuntimeHealthFromJSON(sharedFixture(t, root, "health.ready.v4.json"))
	if err != nil {
		t.Fatalf("NewRuntimeHealthFromJSON(shared fixture): %v", err)
	}
	if !health.APIAlive() || !health.Ready() {
		t.Fatalf("unexpected runtime health from shared fixture: %#v", health)
	}
}

func TestGoHostBindingFacadeExecutesSharedConformanceCase(t *testing.T) {
	root := repositoryRoot(t)
	hostBindingCase := sharedCase(t, root, "host-binding-codec-hash.yaml")
	requireCaseID(t, hostBindingCase, "host_binding/codec_hash")
	for _, action := range []string{
		"build_host_stream_binding",
		"decode_request",
		"encode_item",
		"fold_output_hash",
		"encode_terminal",
	} {
		requireCaseAction(t, hostBindingCase, action)
	}
	for _, fixture := range []string{
		"host-stream-binding.v4.json",
		"host-stream-request.v4.json",
		"host-stream-frame.v4.json",
		"host-stream-terminal.v4.json",
		"host-stream-hash-state.v4.json",
	} {
		requireCaseFixture(t, hostBindingCase, fixture)
	}
	requireCaseExpectation(t, hostBindingCase, `canonical_json: '{"token":"hello"}'`)
	requireCaseExpectation(t, hostBindingCase, "rejects_hash_gap_or_reorder: true")

	transport := &sharedHostBindingTransport{
		bindingJSON:  sharedFixture(t, root, "host-stream-binding.v4.json"),
		requestJSON:  sharedFixture(t, root, "host-stream-request.v4.json"),
		itemJSON:     sharedFixture(t, root, "host-stream-frame.v4.json"),
		terminalJSON: sharedTerminalFrameFixture(t, root),
		hashJSON:     sharedFixture(t, root, "host-stream-hash-state.v4.json"),
	}
	client, err := NewHostBindingClient(transport)
	if err != nil {
		t.Fatalf("NewHostBindingClient: %v", err)
	}

	binding, err := client.BuildHostStreamBinding(context.Background(), HostStreamBindingRequest{
		BindingID:     "binding-weather-1",
		DescriptorRef: "easynet:///r/example/ability/device.dev-a.weather.stream@1.0.0",
		Endpoint:      "/tmp/easynet-weather.sock",
		FrameSchema:   hostStreamFrameSchema,
		Cleanup:       map[string]any{"mode": "unlink_socket"},
	})
	if err != nil {
		t.Fatalf("BuildHostStreamBinding(shared fixture): %v", err)
	}
	if binding.BindingID != "binding-weather-1" || binding.Lifecycle["frame_contract_owner"] != "daemon_sdk" {
		t.Fatalf("unexpected binding from shared fixture: %#v", binding)
	}

	request, err := client.DecodeRequest(context.Background(), HostStreamEnvelope{
		Request: HostStreamEnvelopeRequest{
			Fn:     "weather.stream",
			Args:   map[string]any{"city": "Singapore"},
			CallID: "call-weather-1",
			Caller: "easynet:///r/example/user/alice",
		},
	})
	if err != nil {
		t.Fatalf("DecodeRequest(shared fixture): %v", err)
	}
	if request.Function != "weather.stream" || request.Metadata["source"] != "fixture" {
		t.Fatalf("unexpected request from shared fixture: %#v", request)
	}

	item, err := client.EncodeItem(context.Background(), 0, map[string]any{"token": "hello"})
	if err != nil {
		t.Fatalf("EncodeItem(shared fixture): %v", err)
	}
	if item.FrameType != "item" || item.Seq == nil || *item.Seq != 0 {
		t.Fatalf("unexpected item frame from shared fixture: %#v", item)
	}

	terminal, err := client.EncodeTerminal(context.Background(), HostStreamTerminalSummary{
		OutputHash: "sha256:8196e03ca122ac3b47b3527c8f555735e53c0d3fe1eb8e30c0f974293cd5cd15",
		Frames:     1,
	})
	if err != nil {
		t.Fatalf("EncodeTerminal(shared fixture): %v", err)
	}
	if terminal.Terminal == nil || terminal.OutputHash == nil {
		t.Fatalf("unexpected terminal frame from shared fixture: %#v", terminal)
	}

	state := HostStreamHashState{
		Algorithm:  hostStreamHashAlgorithm,
		OutputHash: "sha256:e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
		Frames:     0,
	}
	folded, err := client.FoldOutputHash(context.Background(), state, 0, map[string]any{"token": "hello"})
	if err != nil {
		t.Fatalf("FoldOutputHash(shared fixture): %v", err)
	}
	if folded.LastSeq == nil || *folded.LastSeq != 0 || folded.CanonicalJSON != `{"token":"hello"}` {
		t.Fatalf("unexpected hash state from shared fixture: %#v", folded)
	}

	if _, err := client.FoldOutputHash(context.Background(), state, 2, map[string]any{"token": "skip"}); !IsCode(err, ErrInvalidArgument) {
		t.Fatalf("hash gap did not produce InvalidArgument: %v", err)
	}
}

func TestGoReceiptFacadeExecutesSharedProjectionConformanceCase(t *testing.T) {
	root := repositoryRoot(t)
	receiptCase := sharedCase(t, root, "receipt-projection-causal-ref.yaml")
	requireCaseID(t, receiptCase, "receipt/projection_causal_ref")
	for _, action := range []string{
		"project_receipt_summary",
		"verify_receipt_summary",
		"build_causal_ref",
	} {
		requireCaseAction(t, receiptCase, action)
	}
	requireCaseFixture(t, receiptCase, "receipt.summary.v4.json")
	requireCaseExpectation(t, receiptCase, "summary_verified: false")
	requireCaseExpectation(t, receiptCase, "verify_summary_claims_cryptographic_validity: false")
	requireCaseExpectation(t, receiptCase, "causal_ref_fixture_result: err_invalid_arg")

	summary, err := NewReceiptSummaryFromJSON(sharedFixture(t, root, "receipt.summary.v4.json"))
	if err != nil {
		t.Fatalf("NewReceiptSummaryFromJSON(shared fixture): %v", err)
	}
	if summary.Verified || summary.State != "completed" || summary.ReceiptURA != nil {
		t.Fatalf("unexpected receipt summary projection: %#v", summary)
	}

	verification, err := NewReceiptVerificationFromJSON([]byte(`{"verified":false,"method":"summary-only","reason":"full receipt required","metadata":{"source":"sdk_conformance"}}`))
	if err != nil {
		t.Fatalf("NewReceiptVerificationFromJSON(summary-only): %v", err)
	}
	if verification.Verified || verification.Method != "summary-only" {
		t.Fatalf("summary-only verification claimed cryptographic validity: %#v", verification)
	}

	if _, err := NewCausalRefFromJSON([]byte(`{"metadata":{}}`)); !IsCode(err, ErrInvalidArgument) {
		t.Fatalf("summary-only causal ref did not produce InvalidArgument: %v", err)
	}
}

func TestGoWrapperFacadeExecutesSharedProjectionConformanceCase(t *testing.T) {
	root := repositoryRoot(t)
	wrapperCase := sharedCase(t, root, "wrapper-profile-records.yaml")
	requireCaseID(t, wrapperCase, "wrappers/profile_records")
	for _, action := range []string{
		"project_file_record",
		"project_terminal_session",
		"project_remote_desktop_session",
		"project_browser_session",
		"project_media_session",
	} {
		requireCaseAction(t, wrapperCase, action)
	}
	for _, fixture := range []string{
		"wrapper-file-record.v4.json",
		"wrapper-terminal-session.v4.json",
		"wrapper-remote-desktop-session.v4.json",
		"wrapper-browser-session.v4.json",
		"wrapper-media-session.v4.json",
	} {
		requireCaseFixture(t, wrapperCase, fixture)
	}
	requireCaseExpectation(t, wrapperCase, "execution_transport_owner: runtime_core")
	requireCaseExpectation(t, wrapperCase, "product_http_websocket_owner: backend")
	requireCaseExpectation(t, wrapperCase, "rejects_invalid_owner_ura: true")
	requireCaseExpectation(t, wrapperCase, "rejects_missing_session_state: true")

	file, err := NewWrapperFileRecordFromJSON(sharedFixture(t, root, "wrapper-file-record.v4.json"))
	if err != nil {
		t.Fatalf("NewWrapperFileRecordFromJSON(shared fixture): %v", err)
	}
	if file.Kind != "file_record" || file.Metadata["source"] != "wrappers.file_record" {
		t.Fatalf("unexpected file wrapper record: %#v", file)
	}

	terminal, err := NewWrapperTerminalSessionRecordFromJSON(sharedFixture(t, root, "wrapper-terminal-session.v4.json"))
	if err != nil {
		t.Fatalf("NewWrapperTerminalSessionRecordFromJSON(shared fixture): %v", err)
	}
	if terminal.Kind != "terminal_session" || terminal.TerminalRef == nil {
		t.Fatalf("unexpected terminal wrapper record: %#v", terminal)
	}

	remote, err := NewWrapperRemoteDesktopSessionRecordFromJSON(sharedFixture(t, root, "wrapper-remote-desktop-session.v4.json"))
	if err != nil {
		t.Fatalf("NewWrapperRemoteDesktopSessionRecordFromJSON(shared fixture): %v", err)
	}
	if remote.Kind != "remote_desktop_session" || remote.DisplayRef == nil {
		t.Fatalf("unexpected remote desktop wrapper record: %#v", remote)
	}

	browser, err := NewWrapperBrowserSessionRecordFromJSON(sharedFixture(t, root, "wrapper-browser-session.v4.json"))
	if err != nil {
		t.Fatalf("NewWrapperBrowserSessionRecordFromJSON(shared fixture): %v", err)
	}
	if browser.Kind != "browser_session" || browser.State != "starting" {
		t.Fatalf("unexpected browser wrapper record: %#v", browser)
	}

	media, err := NewWrapperMediaSessionRecordFromJSON(sharedFixture(t, root, "wrapper-media-session.v4.json"))
	if err != nil {
		t.Fatalf("NewWrapperMediaSessionRecordFromJSON(shared fixture): %v", err)
	}
	if media.Kind != "media_session" || media.MediaKind != "voice" || media.StreamRef == nil {
		t.Fatalf("unexpected media wrapper record: %#v", media)
	}

	client := NewWrapperClient()
	if _, err := client.ProjectFileRecord(WrapperFileRecordRequest{
		FileRef:     file.FileRef,
		OwnerURA:    "not-a-ura",
		ContentType: file.ContentType,
	}); !IsCode(err, ErrInvalidArgument) {
		t.Fatalf("invalid wrapper owner_ura did not produce InvalidArgument: %v", err)
	}
	if _, err := client.ProjectTerminalSession(WrapperTerminalSessionRequest{
		SessionID: terminal.SessionID,
		OwnerURA:  terminal.OwnerURA,
	}); !IsCode(err, ErrInvalidArgument) {
		t.Fatalf("missing wrapper session state did not produce InvalidArgument: %v", err)
	}
}

type sharedHostBindingTransport struct {
	bindingJSON  []byte
	requestJSON  []byte
	itemJSON     []byte
	terminalJSON []byte
	hashJSON     []byte
}

func (t *sharedHostBindingTransport) BuildHostStreamBinding(context.Context, []byte) ([]byte, error) {
	return t.bindingJSON, nil
}

func (t *sharedHostBindingTransport) DecodeRequest(context.Context, []byte) ([]byte, error) {
	return t.requestJSON, nil
}

func (t *sharedHostBindingTransport) EncodeItem(context.Context, []byte) ([]byte, error) {
	return t.itemJSON, nil
}

func (t *sharedHostBindingTransport) EncodeError(context.Context, []byte) ([]byte, error) {
	return nil, &SDKError{Code: ErrNotImplemented, Stage: "test", Retry: RetryNever}
}

func (t *sharedHostBindingTransport) EncodeTerminal(context.Context, []byte) ([]byte, error) {
	return t.terminalJSON, nil
}

func (t *sharedHostBindingTransport) FoldOutputHash(context.Context, []byte) ([]byte, error) {
	return t.hashJSON, nil
}

func (t *sharedHostBindingTransport) Close(context.Context) error {
	return nil
}

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
	path := filepath.Join(root, sharedConformanceFixtureRoot, name)
	raw, err := os.ReadFile(path)
	if err != nil {
		t.Fatalf("read shared fixture %s: %v", path, err)
	}
	return raw
}

func sharedCase(t *testing.T, root, name string) string {
	t.Helper()
	path := filepath.Join(root, sharedConformanceCaseRoot, name)
	raw, err := os.ReadFile(path)
	if err != nil {
		t.Fatalf("read shared case %s: %v", path, err)
	}
	return string(raw)
}

func requireCaseID(t *testing.T, raw, id string) {
	t.Helper()
	requireCaseLiteral(t, raw, "id: "+id)
}

func requireCaseAction(t *testing.T, raw, action string) {
	t.Helper()
	requireCaseLiteral(t, raw, "action: "+action)
}

func requireCaseFixture(t *testing.T, raw, fixture string) {
	t.Helper()
	requireCaseLiteral(t, raw, "fixture: "+fixture)
}

func requireCaseExpectation(t *testing.T, raw, expected string) {
	t.Helper()
	requireCaseLiteral(t, raw, expected)
}

func requireCaseLiteral(t *testing.T, raw, expected string) {
	t.Helper()
	if !strings.Contains(raw, expected) {
		t.Fatalf("shared conformance case is missing %q", expected)
	}
}

func sharedTerminalFrameFixture(t *testing.T, root string) []byte {
	t.Helper()
	var summary HostStreamTerminalSummary
	if err := json.Unmarshal(sharedFixture(t, root, "host-stream-terminal.v4.json"), &summary); err != nil {
		t.Fatalf("decode shared terminal summary fixture: %v", err)
	}
	raw, err := json.Marshal(map[string]any{
		"frame_type":  "terminal",
		"seq":         uint64(summary.Frames),
		"value":       nil,
		"error":       nil,
		"terminal":    summary,
		"output_hash": summary.OutputHash,
	})
	if err != nil {
		t.Fatalf("build shared terminal frame fixture: %v", err)
	}
	return raw
}
