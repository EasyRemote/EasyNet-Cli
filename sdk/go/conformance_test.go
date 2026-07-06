package easynet

import (
	"context"
	"encoding/json"
	"errors"
	"fmt"
	"os"
	"os/exec"
	"path/filepath"
	"reflect"
	"runtime"
	"sort"
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
	requireCaseAction(t, healthCase, "read_diagnostics")
	requireCaseFixture(t, healthCase, "health.ready.v4.json")
	requireCaseFixture(t, healthCase, "diagnostics.ready.v4.json")
	requireCaseExpectation(t, healthCase, "api_ready_field: api_ready")
	requireCaseExpectation(t, healthCase, "runtime_ready_field: runtime_ready")
	requireCaseExpectation(t, healthCase, "diagnostics_kind: diagnostics_report")

	health, err := NewRuntimeHealthFromJSON(sharedFixture(t, root, "health.ready.v4.json"))
	if err != nil {
		t.Fatalf("NewRuntimeHealthFromJSON(shared fixture): %v", err)
	}
	if !health.APIAlive() || !health.Ready() {
		t.Fatalf("unexpected runtime health from shared fixture: %#v", health)
	}
	diagnostics, err := NewDiagnosticsReportFromJSON(sharedFixture(t, root, "diagnostics.ready.v4.json"))
	if err != nil {
		t.Fatalf("NewDiagnosticsReportFromJSON(shared fixture): %v", err)
	}
	if !diagnostics.Ready || diagnostics.Kind != "diagnostics_report" {
		t.Fatalf("unexpected diagnostics report from shared fixture: %#v", diagnostics)
	}
}

func TestGoEnvironmentExecutesSharedProcessRootConformanceCase(t *testing.T) {
	root := repositoryRoot(t)
	environmentCase := sharedCase(t, root, "environment-process-root.yaml")
	requireCaseID(t, environmentCase, "environment/process_root")
	for _, action := range []string{
		"create_environment",
		"require_abi",
		"discover_daemon",
		"connect_local_runtime",
		"close_environment",
		"reject_after_close",
	} {
		requireCaseAction(t, environmentCase, action)
	}
	requireCaseExpectation(t, environmentCase, "exposes_raw_axon: false")
	requireCaseExpectation(t, environmentCase, "owns_invocation_tuple: false")
	requireCaseExpectation(t, environmentCase, "close_idempotent: true")
	requireCaseExpectation(t, environmentCase, "connect_local_starts_daemon: false")

	discovery := &memoryDiscoveryTransport{payload: sharedFeatureDiscoveryJSON(t, 4)}
	daemon := &memoryDaemonTransport{
		discoverJSON: `{"control_endpoint":"/tmp/control.sock","invocation_endpoint":"/tmp/daemon.sock"}`,
		attachJSON:   readyDaemonStatus(),
	}
	env, err := NewSdkEnvironment(discovery, daemon, SdkEnvironmentOptions{
		ExpectedABIVersion: 4,
		Discover:           DiscoverOptions{ControlPath: "/tmp/control.sock"},
		Connect:            ConnectOptions{MaxMessageBytes: 4096},
	})
	if err != nil {
		t.Fatalf("NewSdkEnvironment(shared process root): %v", err)
	}
	if _, err := env.RequireABI(context.Background()); err != nil {
		t.Fatalf("RequireABI(shared process root): %v", err)
	}
	if _, err := env.DiscoverDaemon(context.Background(), DiscoverOptions{}); err != nil {
		t.Fatalf("DiscoverDaemon(shared process root): %v", err)
	}
	runtimeClient, err := env.ConnectLocal(context.Background(), ConnectOptions{InvokeTimeoutMS: 5000})
	if err != nil {
		t.Fatalf("ConnectLocal(shared process root): %v", err)
	}
	if runtimeClient == nil || daemon.openCalls != 1 || daemon.startCalls != 0 {
		t.Fatalf("unexpected process-root connect state: runtime=%#v openCalls=%d startCalls=%d", runtimeClient, daemon.openCalls, daemon.startCalls)
	}
	if err := env.Close(context.Background()); err != nil {
		t.Fatalf("Close(shared process root): %v", err)
	}
	if err := env.Close(context.Background()); err != nil {
		t.Fatalf("second Close(shared process root): %v", err)
	}
	if _, err := env.ConnectLocal(context.Background(), ConnectOptions{}); !IsCode(err, ErrInvalidArgument) {
		t.Fatalf("ConnectLocal after Close = %v, want %s", err, ErrInvalidArgument)
	}
}

func TestGoRuntimeCoreExecutesSharedLifecycleVersionErrorConformanceCases(t *testing.T) {
	root := repositoryRoot(t)

	compatibleCase := sharedCase(t, root, "version-abi-compatible.yaml")
	requireCaseID(t, compatibleCase, "version/abi_compatible")
	requireCaseAction(t, compatibleCase, "feature_discovery")
	requireCaseExpectation(t, compatibleCase, "result: ok")
	requireCaseExpectation(t, compatibleCase, "abi_version: 4")

	compatible, err := NewClient(DiscoveryTransportFunc(func(context.Context) ([]byte, error) {
		return sharedFeatureDiscoveryJSON(t, 4), nil
	}))
	if err != nil {
		t.Fatalf("NewClient(compatible): %v", err)
	}
	features, err := compatible.RequireABI(context.Background(), 4)
	if err != nil {
		t.Fatalf("RequireABI(shared compatible case): %v", err)
	}
	if features.Version().ABIVersion != 4 {
		t.Fatalf("unexpected shared compatible ABI version: %#v", features.Version())
	}

	incompatibleCase := sharedCase(t, root, "version-abi-incompatible.yaml")
	requireCaseID(t, incompatibleCase, "version/abi_incompatible")
	requireCaseAction(t, incompatibleCase, "feature_discovery")
	requireCaseExpectation(t, incompatibleCase, "result: error")
	requireCaseExpectation(t, incompatibleCase, "error_code: VersionMismatch")

	incompatible, err := NewClient(DiscoveryTransportFunc(func(context.Context) ([]byte, error) {
		return sharedFeatureDiscoveryJSON(t, 0), nil
	}))
	if err != nil {
		t.Fatalf("NewClient(incompatible): %v", err)
	}
	_, err = incompatible.RequireABI(context.Background(), 4)
	var sdkErr *SDKError
	if !errors.As(err, &sdkErr) || sdkErr.Code != ErrVersionMismatch {
		t.Fatalf("shared incompatible ABI code = %v, want %s", err, ErrVersionMismatch)
	}

	controlOnlyCase := sharedCase(t, root, "daemon-control-only.yaml")
	requireCaseID(t, controlOnlyCase, "daemon/control_only")
	requireCaseAction(t, controlOnlyCase, "attach_daemon")
	requireCaseFixture(t, controlOnlyCase, "health.ready.v4.json")
	requireCaseExpectation(t, controlOnlyCase, "error_code: ControlOnly")

	health, err := NewRuntimeHealthFromJSON(sharedControlOnlyHealthJSON(t, root))
	if err != nil {
		t.Fatalf("NewRuntimeHealthFromJSON(shared control-only health): %v", err)
	}
	if !health.APIAlive() || health.Ready() || health.InvocationReady {
		t.Fatalf("unexpected shared control-only health: %#v", health)
	}
	control, err := NewDaemonControl(DaemonTransportFunc{
		AttachFunc: func(context.Context, []byte) ([]byte, error) {
			return []byte(`{"handle_id":"daemon-control-only","state":"ControlOnly","mode":"hub","endpoints":{"control_endpoint":"unix:///tmp/easynet-control.sock"},"diagnostics":["invocation endpoint unavailable"]}`), nil
		},
	})
	if err != nil {
		t.Fatalf("NewDaemonControl: %v", err)
	}
	if _, err := control.Attach(context.Background(), AttachOptions{}); !IsCode(err, ErrControlOnly) {
		t.Fatalf("shared control-only attach did not produce ControlOnly: %v", err)
	}

	errorCase := sharedCase(t, root, "error-typed-json.yaml")
	requireCaseID(t, errorCase, "error/typed_json")
	requireCaseAction(t, errorCase, "trigger_invalid_handle_error")
	requireCaseAction(t, errorCase, "read_last_error_json")
	requireCaseAction(t, errorCase, "project_explicit_error_code")
	requireCaseExpectation(t, errorCase, "schema: error.schema.json")
	requireCaseExpectation(t, errorCase, "invalid_handle_code: INVALID_HANDLE")
	requireCaseExpectation(t, errorCase, "explicit_timeout_code: TIMEOUT")
	requireCaseExpectation(t, errorCase, "human_message_parse_required: false")

	invalidHandle, err := DecodeDaemonErrorJSON([]byte(`{"code":"INVALID_HANDLE","stage":"sdk","message":"invalid handle","retry":"never","source":"sdk","details":{}}`))
	if err != nil {
		t.Fatalf("DecodeDaemonErrorJSON(invalid handle): %v", err)
	}
	if invalidHandle.Code != ErrInvalidHandle || invalidHandle.Stage != "sdk" || invalidHandle.Retry != RetryNever {
		t.Fatalf("unexpected shared invalid-handle error: %#v", invalidHandle)
	}
	timeout, err := DecodeDaemonErrorJSON([]byte(`{"code":"TIMEOUT","stage":"invoke","message":"deadline exceeded","retry":"safe","source":"daemon","details":{}}`))
	if err != nil {
		t.Fatalf("DecodeDaemonErrorJSON(timeout): %v", err)
	}
	if timeout.Code != ErrTimeout || timeout.Retry != RetrySafe || !timeout.Retryable {
		t.Fatalf("unexpected shared timeout error: %#v", timeout)
	}

	profileErrorCase := sharedCase(t, root, "error-profile-source-refs.yaml")
	requireCaseID(t, profileErrorCase, "error/profile_source_refs")
	requireCaseAction(t, profileErrorCase, "trigger_profile_validation_error")
	requireCaseAction(t, profileErrorCase, "inspect_error_details")
	requireCaseExpectation(t, profileErrorCase, "profile: publication")
	requireCaseExpectation(t, profileErrorCase, "go_source_ref: go_sdk.profile.publication")
	requireCaseExpectation(t, profileErrorCase, "top_level_schema_change: false")

	publication, err := NewPublicationClient(&sharedPublicationTransport{
		resourceJSON: sharedFixture(t, root, "resource-ref.local-fs.v4.json"),
	})
	if err != nil {
		t.Fatalf("NewPublicationClient(profile error case): %v", err)
	}
	_, err = publication.BuildLocalResourceRef(context.Background(), LocalResourceRefRequest{
		Path:       "tmp/easynet-weather-package",
		Capability: "read",
	})
	if !IsCode(err, ErrInvalidArgument) {
		t.Fatalf("shared profile validation error = %v, want %s", err, ErrInvalidArgument)
	}
	details := sdkErrorDetails(t, err)
	if details["profile"] != publicationProfile {
		t.Fatalf("profile detail = %#v, want %s", details["profile"], publicationProfile)
	}
	if details["source_ref"] != "go_sdk.profile.publication" {
		t.Fatalf("source_ref detail = %#v", details["source_ref"])
	}
}

func TestGoBackendCutoverExecutesSharedImportBanConformanceCase(t *testing.T) {
	root := repositoryRoot(t)
	backendCase := sharedCase(t, root, "backend-sdk-only-import-ban.yaml")
	requireCaseID(t, backendCase, "backend/import_ban")
	for _, action := range []string{
		"scan_backend_go_imports",
		"reject_raw_axon_imports",
		"reject_generated_axon_pb_imports",
		"reject_direct_daemon_transport",
		"reject_cabi_ffi_markers",
		"reject_runtime_subprocess",
	} {
		requireCaseAction(t, backendCase, action)
	}
	requireCaseExpectation(t, backendCase, "allowed_runtime_import: easynet.run/cli/sdk/go")
	requireCaseExpectation(t, backendCase, "raw_axon_import: false")
	requireCaseExpectation(t, backendCase, "generated_axon_pb_import: false")
	requireCaseExpectation(t, backendCase, "direct_daemon_transport: false")
	requireCaseExpectation(t, backendCase, "runtime_subprocess: false")

	cmd := exec.Command(filepath.Join(root, "tools/scripts/check-backend-sdk-only-boundary.sh"), "--self-test")
	output, err := cmd.CombinedOutput()
	if err != nil {
		t.Fatalf("backend SDK-only boundary self-test failed: %v\n%s", err, output)
	}
	if !strings.Contains(string(output), "self-test ok") {
		t.Fatalf("backend SDK-only boundary self-test output = %q", output)
	}
}

func TestGoBackendCutoverExecutesSharedRouteFamilyCoverageConformanceCase(t *testing.T) {
	root := repositoryRoot(t)
	coverageCase := sharedCase(t, root, "backend-route-family-coverage.yaml")
	requireCaseID(t, coverageCase, "backend/hub_route_family_coverage")
	for _, action := range []string{
		"load_hub_route_family_manifest",
		"require_all_spec_29_2_route_families",
		"require_assigned_sdk_clients",
		"require_coverage_evidence",
		"reject_backend_local_runtime_ownership",
	} {
		requireCaseAction(t, coverageCase, action)
	}
	requireCaseExpectation(t, coverageCase, "route_family_count: 14")
	requireCaseExpectation(t, coverageCase, "missing_route_family: false")
	requireCaseExpectation(t, coverageCase, "duplicate_route_family: false")
	requireCaseExpectation(t, coverageCase, "backend_local_runtime_ownership: false")

	cmd := exec.Command(filepath.Join(root, "tools/scripts/check-backend-route-family-coverage.sh"), "--self-test")
	output, err := cmd.CombinedOutput()
	if err != nil {
		t.Fatalf("backend route-family coverage self-test failed: %v\n%s", err, output)
	}
	if !strings.Contains(string(output), "self-test ok") {
		t.Fatalf("backend route-family coverage self-test output = %q", output)
	}
}

func TestGoSDKExecutesSharedParityMatrixConformanceCase(t *testing.T) {
	root := repositoryRoot(t)
	parityCase := sharedCase(t, root, "sdk-go-python-parity-matrix.yaml")
	requireCaseID(t, parityCase, "sdk/go_python_parity_matrix")
	for _, action := range []string{
		"load_sdk_parity_matrix",
		"require_go_python_languages",
		"require_status_taxonomy",
		"require_all_p0_daemon_sdk_capabilities",
		"require_evidence_refs",
		"require_gap_reason_for_status_mismatch",
		"reject_false_cutover_ready",
	} {
		requireCaseAction(t, parityCase, action)
	}
	requireCaseExpectation(t, parityCase, "capability_count: 21")
	requireCaseExpectation(t, parityCase, "product_boundary_count: 2")
	requireCaseExpectation(t, parityCase, "missing_capability: false")
	requireCaseExpectation(t, parityCase, "invalid_status: false")
	requireCaseExpectation(t, parityCase, "product_specific_capability: false")
	requireCaseExpectation(t, parityCase, "false_cutover_ready: false")

	cmd := exec.Command(filepath.Join(root, "tools/scripts/check-sdk-parity-matrix.sh"), "--self-test")
	output, err := cmd.CombinedOutput()
	if err != nil {
		t.Fatalf("SDK parity matrix self-test failed: %v\n%s", err, output)
	}
	if !strings.Contains(string(output), "self-test ok") {
		t.Fatalf("SDK parity matrix self-test output = %q", output)
	}
}

func TestGoRuntimeCoreExecutesSharedInvocationSigningConformanceCases(t *testing.T) {
	root := repositoryRoot(t)

	builderCase := sharedCase(t, root, "invocation-builder-handle-state.yaml")
	requireCaseID(t, builderCase, "invocation/builder_handle_state")
	requireCaseAction(t, builderCase, "create_builder")
	requireCaseAction(t, builderCase, "set_complete_tuple")
	requireCaseAction(t, builderCase, "inspect_builder")
	requireCaseAction(t, builderCase, "build_builder")
	requireCaseExpectation(t, builderCase, "result: error_after_build")
	requireCaseExpectation(t, builderCase, "build_consumes_handle: true")
	requireCaseExpectation(t, builderCase, "error_code: InvalidHandle")

	builder := sharedInvocationBuilder(t, root)
	if _, err := builder.Inspect(); err != nil {
		t.Fatalf("Inspect(shared builder): %v", err)
	}
	if _, err := builder.Build(); err != nil {
		t.Fatalf("Build(shared builder): %v", err)
	}
	if _, err := builder.Inspect(); !IsCode(err, ErrInvalidHandle) {
		t.Fatalf("Inspect after Build(shared builder) = %v, want %s", err, ErrInvalidHandle)
	}

	canonicalCase := sharedCase(t, root, "invocation-canonical-material.yaml")
	requireCaseID(t, canonicalCase, "invocation/canonical_material")
	requireCaseAction(t, canonicalCase, "prepare")
	requireCaseFixture(t, canonicalCase, "invocation.complete.v4.json")
	requireCaseExpectation(t, canonicalCase, "material_owner: axon_delegated")
	requireCaseExpectation(t, canonicalCase, "fixture: prepared.signing-material.v4.json")

	preparedFixtureJSON := sharedFixture(t, root, "prepared.signing-material.v4.json")
	var seenPreparedDraft []byte
	client, err := NewRuntimeClient(RuntimeTransportFunc{
		PrepareFunc: func(ctx context.Context, draftJSON []byte, optionsJSON []byte) ([]byte, error) {
			seenPreparedDraft = append([]byte(nil), draftJSON...)
			return preparedFixtureJSON, nil
		},
		SubmitSignedFunc: func(ctx context.Context, signedJSON []byte) ([]byte, error) {
			return []byte(`{"handle_id":7,"state":"Submitted","terminal":false,"events":[{"sequence":1,"kind":"submitted","state":"Submitted","terminal":false}],"result":null}`), nil
		},
		AwaitHandleFunc: func(ctx context.Context, handleID uint64) ([]byte, error) {
			return []byte(fmt.Sprintf(`{"ok":true,"tuple":%s,"terminal_state":"Completed","output_content_type":"application/json","output_base64":"e30=","output_json":{},"elapsed_ms":1,"receipt":{"receipt_id":"receipt-1"},"error":null}`, sharedFixture(t, root, "invocation.complete.v4.json"))), nil
		},
		CancelHandleFunc: func(ctx context.Context, handleID uint64, reason string) ([]byte, error) {
			return []byte(`{"handle_id":7,"cancelled":false,"state":"Completed","terminal":true}`), nil
		},
		HandleEventsFunc: func(ctx context.Context, handleID uint64) ([]byte, error) {
			return []byte(`{"handle_id":7,"state":"Completed","terminal":true,"events":[{"sequence":1,"kind":"completed","state":"Completed","terminal":true,"result":{"receipt_id":"receipt-1"}}],"result":{"receipt_id":"receipt-1"}}`), nil
		},
	})
	if err != nil {
		t.Fatalf("NewRuntimeClient(shared invocation signing): %v", err)
	}

	prepared, material, err := client.Prepare(context.Background(), sharedInvocationDraft(t, root), PrepareOptions{})
	if err != nil {
		t.Fatalf("Prepare(shared canonical material): %v", err)
	}
	assertJSONEquivalent(t, seenPreparedDraft, sharedFixture(t, root, "invocation.complete.v4.json"))
	if material.Algorithm() != "ed25519" || material.CanonicalBytesBase64() != "ZXhhbXBsZS1jYW5vbmljYWwtYnl0ZXM=" {
		t.Fatalf("unexpected shared signing material: %#v", material)
	}
	if prepared.SubmitReady() {
		t.Fatalf("PreparedInvocation is submit-ready")
	}

	notSubmittableCase := sharedCase(t, root, "invocation-prepared-not-submittable.yaml")
	requireCaseID(t, notSubmittableCase, "invocation/prepared_not_submittable")
	requireCaseAction(t, notSubmittableCase, "submit_prepared")
	requireCaseExpectation(t, notSubmittableCase, "error_code: InvalidArgument")
	if prepared.SubmitReady() {
		t.Fatalf("shared PreparedInvocation crossed submit boundary")
	}

	presignedCase := sharedCase(t, root, "invocation-presigned-submit.yaml")
	requireCaseID(t, presignedCase, "invocation/presigned_submit")
	requireCaseAction(t, presignedCase, "attach_signature")
	requireCaseAction(t, presignedCase, "submit_signed")
	requireCaseExpectation(t, presignedCase, "signature_preserved: true")

	signed, err := prepared.SignWithCallerSignature(sharedInvocationSignature())
	if err != nil {
		t.Fatalf("SignWithCallerSignature(shared): %v", err)
	}
	var seenSigned map[string]any
	signatureClient, err := NewRuntimeClient(RuntimeTransportFunc{
		SubmitSignedFunc: func(ctx context.Context, signedJSON []byte) ([]byte, error) {
			if err := json.Unmarshal(signedJSON, &seenSigned); err != nil {
				t.Fatalf("decode shared signed JSON: %v", err)
			}
			return []byte(`{"handle_id":7,"state":"Submitted","terminal":false,"events":[{"sequence":1,"kind":"submitted","state":"Submitted","terminal":false}],"result":null}`), nil
		},
	})
	if err != nil {
		t.Fatalf("NewRuntimeClient(shared presigned): %v", err)
	}
	handle, err := signatureClient.SubmitSigned(context.Background(), signed)
	if err != nil {
		t.Fatalf("SubmitSigned(shared presigned): %v", err)
	}
	if handle.HandleID() != 7 {
		t.Fatalf("unexpected shared handle: %#v", handle)
	}
	if seenSigned["signature"].(map[string]any)["signature_base64"] != "c2lnbmF0dXJl" {
		t.Fatalf("shared signature was not preserved: %#v", seenSigned)
	}

	localSigningCase := sharedCase(t, root, "invocation-local-daemon-signing-boundary.yaml")
	requireCaseID(t, localSigningCase, "invocation/local_daemon_signing_boundary")
	requireCaseAction(t, localSigningCase, "local_daemon_sign")
	requireCaseExpectation(t, localSigningCase, "public_object: SignedInvocation")
	if !signed.SubmitReady() || signed.Prepared().SubmitReady() {
		t.Fatalf("local signing did not produce SignedInvocation boundary")
	}

	terminalCase := sharedCase(t, root, "invocation-handle-terminal-monotonicity.yaml")
	requireCaseID(t, terminalCase, "invocation/handle_terminal_monotonicity")
	for _, action := range []string{
		"prepare_complete_tuple",
		"sign_prepared",
		"submit_signed_handle",
		"await_handle_terminal",
		"cancel_handle",
		"read_handle_events",
	} {
		requireCaseAction(t, terminalCase, action)
	}
	requireCaseExpectation(t, terminalCase, "submit_consumes_signed: true")
	requireCaseExpectation(t, terminalCase, "terminal_event_count: 1")
	result, err := client.Await(context.Background(), handle)
	if err != nil {
		t.Fatalf("Await(shared terminal): %v", err)
	}
	cancel, err := client.Cancel(context.Background(), handle, "after terminal")
	if err != nil {
		t.Fatalf("Cancel(shared terminal): %v", err)
	}
	events, err := client.Events(context.Background(), handle)
	if err != nil {
		t.Fatalf("Events(shared terminal): %v", err)
	}
	if !result.OK() || result.TerminalState() != "Completed" || cancel.State() != "Completed" || !cancel.Terminal() {
		t.Fatalf("terminal monotonicity not preserved: result=%#v cancel=%#v", result, cancel)
	}
	if !events.Terminal() || len(events.Events()) != 1 || !events.Events()[0].Terminal() {
		t.Fatalf("unexpected shared terminal events: %#v", events.Events())
	}
}

func TestGoRuntimeCoreExecutesSharedStreamBidiLifecycleConformanceCase(t *testing.T) {
	root := repositoryRoot(t)
	lifecycleCase := sharedCase(t, root, "stream-bidi-lifecycle-state.yaml")
	requireCaseID(t, lifecycleCase, "stream_bidi/lifecycle_state")
	for _, action := range []string{
		"open_stream",
		"project_stream_terminal_event",
		"close_stream",
		"open_bidi",
		"project_bidi_terminal_frame",
		"close_bidi_send",
		"send_bidi_after_close_send",
		"close_bidi",
	} {
		requireCaseAction(t, lifecycleCase, action)
	}
	requireCaseFixture(t, lifecycleCase, "invocation.complete.v4.json")
	for _, expected := range []string{
		"stream_terminal_schema: stream-event.schema.json",
		"bidi_terminal_schema: bidi-frame.schema.json",
		"stream_close_unknown_is_idempotent: true",
		"stream_cross_owner_close_error: ERR_INVALID_HANDLE",
		"bidi_close_send_keeps_session_registered: true",
		"bidi_close_send_unknown_error: ERR_INVALID_HANDLE",
		"bidi_send_after_close_send_error: ERR_CANCELLED",
		"bidi_close_releases_session: true",
	} {
		requireCaseExpectation(t, lifecycleCase, expected)
	}

	if _, err := NewInvocationDraftFromJSON(sharedFixture(t, root, "invocation.complete.v4.json")); err != nil {
		t.Fatalf("NewInvocationDraftFromJSON(stream-bidi fixture): %v", err)
	}

	terminalStream, err := NewStreamHandleFromJSON(StreamTransportFunc{
		RecvFunc: func(context.Context) ([]byte, error) {
			return []byte(`{"sequence":1,"kind":"terminal","state":"Completed","terminal":true,"payload_json":{"receipt":{"receipt_ura":"easynet:///r/example/receipt/r1"}}}`), nil
		},
	}, []byte(`{"stream_id":"stream-terminal-1","state":"Open","max_buffered_events":4}`))
	if err != nil {
		t.Fatalf("NewStreamHandleFromJSON(shared terminal projection): %v", err)
	}
	if _, err := terminalStream.Next(context.Background()); err != nil {
		t.Fatalf("Next(shared terminal projection): %v", err)
	}
	streamTerminal, err := terminalStream.TerminalEvent()
	if err != nil {
		t.Fatalf("TerminalEvent(shared terminal projection): %v", err)
	}
	if streamTerminal.StreamID() != "stream-terminal-1" || streamTerminal.EventType() != "terminal" || streamTerminal.Seq() != 1 {
		t.Fatalf("unexpected shared stream terminal projection: %#v", streamTerminal)
	}
	if string(streamTerminal.ReceiptJSON()) != `{"receipt_ura":"easynet:///r/example/receipt/r1"}` {
		t.Fatalf("unexpected shared stream terminal receipt: %s", streamTerminal.ReceiptJSON())
	}

	terminalBidiTransport := &sharedBidiLifecycleTransport{
		recvFrames: [][]byte{
			[]byte(`{"sequence":1,"kind":"terminal","stream_id":1,"terminal":true,"payload_json":{"receipt":{"receipt_ura":"easynet:///r/example/receipt/r1"}}}`),
		},
	}
	terminalBidi, err := NewBidiSessionFromJSON(terminalBidiTransport, []byte(`{"session_id":"bidi-terminal-1","state":"Open","max_buffered_frames":4}`))
	if err != nil {
		t.Fatalf("NewBidiSessionFromJSON(shared terminal projection): %v", err)
	}
	if _, err := terminalBidi.Receive(context.Background()); err != nil {
		t.Fatalf("Receive(shared terminal projection): %v", err)
	}
	bidiTerminal, err := terminalBidi.TerminalFrame()
	if err != nil {
		t.Fatalf("TerminalFrame(shared terminal projection): %v", err)
	}
	if bidiTerminal.SessionID() != "bidi-terminal-1" || bidiTerminal.FrameType() != "terminal" || bidiTerminal.Seq() != 1 {
		t.Fatalf("unexpected shared bidi terminal projection: %#v", bidiTerminal)
	}
	if string(bidiTerminal.ReceiptJSON()) != `{"receipt_ura":"easynet:///r/example/receipt/r1"}` {
		t.Fatalf("unexpected shared bidi terminal receipt: %s", bidiTerminal.ReceiptJSON())
	}

	streamCloseCalls := 0
	stream, err := NewStreamHandleFromJSON(StreamTransportFunc{
		CloseFunc: func(context.Context) error {
			streamCloseCalls++
			return nil
		},
	}, []byte(`{"stream_id":"stream-lifecycle-1","state":"Open","max_buffered_events":4}`))
	if err != nil {
		t.Fatalf("NewStreamHandleFromJSON(shared lifecycle): %v", err)
	}
	if err := stream.Close(context.Background()); err != nil {
		t.Fatalf("Close(shared stream lifecycle): %v", err)
	}
	if err := stream.Close(context.Background()); err != nil {
		t.Fatalf("second Close(shared stream lifecycle): %v", err)
	}
	if stream.State() != StreamClosed || streamCloseCalls != 1 {
		t.Fatalf("stream close not idempotent: state=%s closeCalls=%d", stream.State(), streamCloseCalls)
	}

	crossOwnerStream, err := NewStreamHandleFromJSON(StreamTransportFunc{
		CloseFunc: func(context.Context) error {
			return &SDKError{Code: ErrInvalidHandle, Stage: "stream", Retry: RetryNever, Message: "stream handle is not owned by caller"}
		},
	}, []byte(`{"stream_id":"stream-cross-owner","state":"Open","max_buffered_events":4}`))
	if err != nil {
		t.Fatalf("NewStreamHandleFromJSON(cross-owner): %v", err)
	}
	if err := crossOwnerStream.Close(context.Background()); !IsCode(err, ErrInvalidHandle) {
		t.Fatalf("cross-owner stream close error = %v, want %s", err, ErrInvalidHandle)
	}

	bidiTransport := &sharedBidiLifecycleTransport{
		recvFrames: [][]byte{
			[]byte(`{"sequence":1,"kind":"data","stream_id":1}`),
			[]byte(`{"sequence":2,"kind":"remote_close_send","stream_id":1}`),
		},
	}
	bidi, err := NewBidiSessionFromJSON(bidiTransport, []byte(`{"session_id":"bidi-lifecycle-1","state":"Open","max_buffered_frames":4}`))
	if err != nil {
		t.Fatalf("NewBidiSessionFromJSON(shared lifecycle): %v", err)
	}
	outcome, err := bidi.CloseSend(context.Background())
	if err != nil {
		t.Fatalf("CloseSend(shared lifecycle): %v", err)
	}
	if outcome.State() != BidiHalfClosedLocal || outcome.Terminal() || bidi.State() != BidiHalfClosedLocal {
		t.Fatalf("unexpected close-send lifecycle: outcome=%#v state=%s", outcome, bidi.State())
	}
	received, err := bidi.Receive(context.Background())
	if err != nil {
		t.Fatalf("Receive after CloseSend(shared lifecycle): %v", err)
	}
	if received.Kind() != "data" || bidi.State() != BidiHalfClosedLocal {
		t.Fatalf("bidi receive side not alive after close-send: frame=%#v state=%s", received, bidi.State())
	}
	frame, err := NewBidiFrame(1, "data", 1)
	if err != nil {
		t.Fatalf("NewBidiFrame(shared lifecycle): %v", err)
	}
	if _, err := bidi.Send(context.Background(), frame); !IsCode(err, ErrCancelled) {
		t.Fatalf("send after close-send error = %v, want %s", err, ErrCancelled)
	}
	if bidi.State() != BidiHalfClosedLocal {
		t.Fatalf("send after close-send changed state to %s", bidi.State())
	}
	remoteClose, err := bidi.Receive(context.Background())
	if err != nil {
		t.Fatalf("Receive remote close after CloseSend(shared lifecycle): %v", err)
	}
	if remoteClose.Kind() != "remote_close_send" || bidi.State() != BidiTerminal {
		t.Fatalf("remote close did not terminalize bidi: frame=%#v state=%s", remoteClose, bidi.State())
	}
	if err := bidi.Close(context.Background()); err != nil {
		t.Fatalf("Close(shared bidi lifecycle): %v", err)
	}
	if bidi.State() != BidiClosed || !bidiTransport.closed {
		t.Fatalf("bidi close did not release session: state=%s closed=%v", bidi.State(), bidiTransport.closed)
	}

	unknownBidi, err := NewBidiSessionFromJSON(BidiTransportFunc{
		CloseSendFunc: func(context.Context) ([]byte, error) {
			return nil, &SDKError{Code: ErrInvalidHandle, Stage: "bidi", Retry: RetryNever, Message: "bidi session is not owned by caller"}
		},
	}, []byte(`{"session_id":"bidi-cross-owner","state":"Open","max_buffered_frames":4}`))
	if err != nil {
		t.Fatalf("NewBidiSessionFromJSON(unknown close-send): %v", err)
	}
	if _, err := unknownBidi.CloseSend(context.Background()); !IsCode(err, ErrInvalidHandle) {
		t.Fatalf("unknown bidi close-send error = %v, want %s", err, ErrInvalidHandle)
	}
}

func TestGoRuntimeCoreExecutesSharedStreamBackpressureConformanceCase(t *testing.T) {
	root := repositoryRoot(t)
	backpressureCase := sharedCase(t, root, "stream-backpressure-bound.yaml")
	requireCaseID(t, backpressureCase, "stream/backpressure_bound")
	for _, action := range []string{
		"overflow_stream_callback_queue",
		"project_stream_backpressure_terminal",
		"overflow_bidi_callback_queue",
		"project_bidi_backpressure_terminal",
	} {
		requireCaseAction(t, backpressureCase, action)
	}
	requireCaseFixture(t, backpressureCase, "invocation.complete.v4.json")
	for _, expected := range []string{
		"stream_error_code: ADMISSION_DENIED",
		"bidi_error_code: ADMISSION_DENIED",
		"wire_code: RESOURCE_EXHAUSTED",
		"retry: after_backoff",
		"reason: callback_queue_overflow",
		"terminal: true",
		"bounded_queue: true",
	} {
		requireCaseExpectation(t, backpressureCase, expected)
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
		"check_readiness",
		"cleanup",
	} {
		requireCaseAction(t, hostBindingCase, action)
	}
	for _, fixture := range []string{
		"host-stream-binding.v4.json",
		"host-stream-request.v4.json",
		"host-stream-frame.v4.json",
		"host-stream-terminal.v4.json",
		"host-stream-hash-state.v4.json",
		"host-stream-hash-state-corrupted-zero.v4.json",
		"host-stream-hash-state-corrupted-gap.v4.json",
	} {
		requireCaseFixture(t, hostBindingCase, fixture)
	}
	requireCaseExpectation(t, hostBindingCase, `canonical_json: '{"token":"hello"}'`)
	requireCaseExpectation(t, hostBindingCase, "rejects_hash_gap_or_reorder: true")
	requireCaseExpectation(t, hostBindingCase, "rejects_corrupted_zero_state: true")
	requireCaseExpectation(t, hostBindingCase, "rejects_corrupted_gap_state: true")
	requireCaseExpectation(t, hostBindingCase, "hash_state_invariant: frames_zero_requires_null_last_seq_and_frames_positive_requires_last_seq_equal_frames_minus_one")
	requireCaseExpectation(t, hostBindingCase, "lifecycle_provider_backed: true")
	requireCaseExpectation(t, hostBindingCase, "lifecycle_ready_state: ready")
	requireCaseExpectation(t, hostBindingCase, "cleanup_is_idempotent: true")

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
	provider := &memoryHostLifecycleProvider{}
	lifecycle, err := client.OpenLifecycle(context.Background(), binding, provider)
	if err != nil {
		t.Fatalf("OpenLifecycle(shared fixture): %v", err)
	}
	readiness, err := lifecycle.CheckReadiness(context.Background())
	if err != nil {
		t.Fatalf("CheckReadiness(shared fixture): %v", err)
	}
	cleanup, err := lifecycle.Cleanup(context.Background())
	if err != nil {
		t.Fatalf("Cleanup(shared fixture): %v", err)
	}
	cleanupAgain, err := lifecycle.Cleanup(context.Background())
	if err != nil {
		t.Fatalf("second Cleanup(shared fixture): %v", err)
	}
	if readiness.State != "ready" || readiness.EndpointReady == nil || !*readiness.EndpointReady {
		t.Fatalf("unexpected readiness from shared fixture: %#v", readiness)
	}
	if cleanup.Mode != "unlink_socket" || cleanupAgain.Mode != cleanup.Mode || cleanupAgain.Metadata["cleaned"] != true {
		t.Fatalf("unexpected cleanup from shared fixture: %#v / %#v", cleanup, cleanupAgain)
	}
	if lifecycle.State() != HostStreamLifecycleCleaned {
		t.Fatalf("state = %s, want %s", lifecycle.State(), HostStreamLifecycleCleaned)
	}
	if len(provider.calls) != 2 ||
		provider.calls[0] != "readiness:binding-weather-1" ||
		provider.calls[1] != "cleanup:binding-weather-1" {
		t.Fatalf("provider calls = %#v", provider.calls)
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
	if transport.foldCalls != 1 {
		t.Fatalf("unexpected hash fold transport call count after valid fold: %d", transport.foldCalls)
	}

	if _, err := client.FoldOutputHash(context.Background(), state, 2, map[string]any{"token": "skip"}); !IsCode(err, ErrInvalidArgument) {
		t.Fatalf("hash gap did not produce InvalidArgument: %v", err)
	}
	if transport.foldCalls != 1 {
		t.Fatalf("hash gap crossed transport boundary: %d fold calls", transport.foldCalls)
	}

	for _, fixture := range []string{
		"host-stream-hash-state-corrupted-zero.v4.json",
		"host-stream-hash-state-corrupted-gap.v4.json",
	} {
		if _, err := NewHostStreamHashStateFromJSON(sharedFixture(t, root, fixture)); !IsCode(err, ErrInvalidArgument) {
			t.Fatalf("%s did not produce InvalidArgument: %v", fixture, err)
		}
	}

	corruptedLastSeq := uint64(0)
	corruptedState := HostStreamHashState{
		Algorithm:  hostStreamHashAlgorithm,
		OutputHash: "sha256:8196e03ca122ac3b47b3527c8f555735e53c0d3fe1eb8e30c0f974293cd5cd15",
		Frames:     2,
		LastSeq:    &corruptedLastSeq,
	}
	if _, err := client.FoldOutputHash(context.Background(), corruptedState, 2, map[string]any{"token": "skip"}); !IsCode(err, ErrInvalidArgument) {
		t.Fatalf("corrupted hash state reached fold transport boundary: %v", err)
	}
	if transport.foldCalls != 1 {
		t.Fatalf("corrupted hash state crossed transport boundary: %d fold calls", transport.foldCalls)
	}
}

func TestGoReceiptFacadeExecutesSharedProjectionConformanceCase(t *testing.T) {
	root := repositoryRoot(t)

	fetchCase := sharedCase(t, root, "receipt-fetch-carrier.yaml")
	requireCaseID(t, fetchCase, "receipt/fetch_carrier")
	requireCaseAction(t, fetchCase, "build_receipt_fetch_invocation")
	requireCaseFixture(t, fetchCase, "receipt-fetch-request.v4.json")
	requireCaseExpectation(t, fetchCase, "invocation_fixture: receipt-fetch-invocation.v4.json")
	requireCaseExpectation(t, fetchCase, "daemon_ability: invocation.history.get")
	requireCaseExpectation(t, fetchCase, "descriptor_ref_source: request")
	requireCaseExpectation(t, fetchCase, "selector_cardinality: exactly_one")
	requireCaseExpectation(t, fetchCase, "direct_ledger_read: false")

	descriptorDelegationCase := sharedCase(t, root, "invocation-descriptor-ref-helper-delegation.yaml")
	requireCaseID(t, descriptorDelegationCase, "invocation/descriptor_ref_helper_delegation")
	requireCaseAction(t, descriptorDelegationCase, "project_descriptor_ref")
	requireCaseAction(t, descriptorDelegationCase, "build_receipt_fetch_invocation")
	requireCaseAction(t, descriptorDelegationCase, "inspect_descriptor_ref_source")
	requireCaseExpectation(t, descriptorDelegationCase, "canonical_helper_owner: axon")
	requireCaseExpectation(t, descriptorDelegationCase, "descriptor_ref_source: identity_projection_or_daemon_boundary")
	requireCaseExpectation(t, descriptorDelegationCase, "receipt_fetch_descriptor_ref_from_request: true")
	requireCaseExpectation(t, descriptorDelegationCase, "facade_descriptor_concat: false")
	requireCaseExpectation(t, descriptorDelegationCase, "rejects_missing_descriptor_ref: true")

	var fetchRequest ReceiptFetchRequest
	if err := json.Unmarshal(sharedFixture(t, root, "receipt-fetch-request.v4.json"), &fetchRequest); err != nil {
		t.Fatalf("decode shared receipt fetch request: %v", err)
	}
	fetchDraft, err := BuildReceiptFetchInvocation(fetchRequest)
	if err != nil {
		t.Fatalf("BuildReceiptFetchInvocation(shared): %v", err)
	}
	fetchDraftJSON, err := json.Marshal(fetchDraft)
	if err != nil {
		t.Fatalf("marshal shared receipt fetch invocation: %v", err)
	}
	assertJSONEquivalent(t, fetchDraftJSON, sharedFixture(t, root, "receipt-fetch-invocation.v4.json"))
	if fetchDraft.DescriptorRef() != fetchRequest.DescriptorRef {
		t.Fatalf("receipt fetch descriptor_ref = %q, want request value %q", fetchDraft.DescriptorRef(), fetchRequest.DescriptorRef)
	}
	missingDescriptor := fetchRequest
	missingDescriptor.DescriptorRef = ""
	if _, err := BuildReceiptFetchInvocation(missingDescriptor); !IsCode(err, ErrInvalidArgument) {
		t.Fatalf("missing descriptor_ref = %v, want %s", err, ErrInvalidArgument)
	}
	receiptSource, err := os.ReadFile(filepath.Join(root, "sdk/go/receipt.go"))
	if err != nil {
		t.Fatalf("read receipt source: %v", err)
	}
	if strings.Contains(string(receiptSource), "receiptAbilityDescriptorParts") ||
		strings.Contains(string(receiptSource), "%s/ability/%s.%s@%s") {
		t.Fatalf("receipt facade source reintroduced descriptor_ref synthesis")
	}
	ambiguous := fetchRequest
	ambiguous.TraceID = "trace-1"
	if _, err := BuildReceiptFetchInvocation(ambiguous); !IsCode(err, ErrInvalidArgument) {
		t.Fatalf("ambiguous receipt selector = %v, want %s", err, ErrInvalidArgument)
	}

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

	chainCase := sharedCase(t, root, "receipt-axon-chain-verification.yaml")
	requireCaseID(t, chainCase, "receipt/axon_chain_verification")
	for _, action := range []string{
		"verify_full_receipt_chain",
		"require_axon_provider_projection",
		"require_parent_receipt_closure",
		"reject_language_facade_verifier",
	} {
		requireCaseAction(t, chainCase, action)
	}
	requireCaseExpectation(t, chainCase, "chain_projection: single_invocation_signature_chain_with_parent_closure")
	requireCaseExpectation(t, chainCase, "parent_dag_closed: true")
	requireCaseExpectation(t, chainCase, "cross_invocation_causal_dag: incomplete_until_axon_library_api")
}

func TestGoDirectoryIdentityFacadeExecutesSharedProjectionConformanceCases(t *testing.T) {
	root := repositoryRoot(t)

	listCase := sharedCase(t, root, "directory-list-pagination.yaml")
	requireCaseID(t, listCase, "directory/list_pagination")
	for _, action := range []string{
		"build_list_devices_invocation",
		"build_list_agents_invocation",
		"project_device_page",
		"project_agent_page",
		"list_devices",
	} {
		requireCaseAction(t, listCase, action)
	}
	for _, fixture := range []string{
		"directory-list-devices-request.v4.json",
		"directory-list-agents-request.v4.json",
		"directory-device-page.v4.json",
		"directory-agent-page.v4.json",
	} {
		requireCaseFixture(t, listCase, fixture)
	}
	requireCaseExpectation(t, listCase, "max_page_size: 500")
	requireCaseExpectation(t, listCase, "device_invocation_fixture: directory-list-devices-invocation.v4.json")
	requireCaseExpectation(t, listCase, "agent_invocation_fixture: directory-list-agents-invocation.v4.json")
	requireCaseExpectation(t, listCase, "error_code: InvalidArgument")

	directory, err := NewDirectoryClient(&sharedDirectoryTransport{
		t:                           t,
		expectedDevicesRequest:      sharedFixture(t, root, "directory-list-devices-request.v4.json"),
		expectedAgentsRequest:       sharedFixture(t, root, "directory-list-agents-request.v4.json"),
		expectedAbilityRequest:      sharedFixture(t, root, "directory-list-abilities-request.v4.json"),
		expectedResolveRequest:      sharedFixture(t, root, "directory-resolve-request.v4.json"),
		expectedSubscriptionRequest: sharedFixture(t, root, "directory-subscription-request.v4.json"),
		devicesJSON:                 sharedFixture(t, root, "directory-device-page.v4.json"),
		agentsJSON:                  sharedFixture(t, root, "directory-agent-page.v4.json"),
		abilitiesJSON:               sharedFixture(t, root, "directory-ability-page.v4.json"),
		resolveJSON:                 sharedFixture(t, root, "directory-resolved-ref.v4.json"),
		subscriptionInvocationJSON:  sharedFixture(t, root, "directory-subscription-invocation.v4.json"),
		subscriptionJSON:            sharedFixture(t, root, "directory-subscription.v4.json"),
	})
	if err != nil {
		t.Fatalf("NewDirectoryClient: %v", err)
	}

	devicePage, err := directory.ListDevices(context.Background(), sharedDeviceQuery(t, root))
	if err != nil {
		t.Fatalf("ListDevices(shared fixture): %v", err)
	}
	if devicePage.Limit != 2 || len(devicePage.Items) != 1 || devicePage.Metadata["source_ability"] != "node.list" {
		t.Fatalf("unexpected shared device page: %#v", devicePage)
	}

	agentPage, err := directory.ListAgents(context.Background(), sharedAgentQuery(t, root))
	if err != nil {
		t.Fatalf("ListAgents(shared fixture): %v", err)
	}
	if agentPage.Limit != 2 || len(agentPage.Items) != 1 || agentPage.Metadata["source_ability"] != "agent.list" {
		t.Fatalf("unexpected shared agent page: %#v", agentPage)
	}

	if _, err := directory.ListDevices(context.Background(), DeviceQuery{
		DirectoryQueryBase: sharedDirectoryQueryBase(t, root, "directory-list-devices-request.v4.json", MaxDirectoryPageSize+1),
	}); !IsCode(err, ErrInvalidArgument) {
		t.Fatalf("oversized directory list did not produce InvalidArgument: %v", err)
	}

	fanoutCase := sharedCase(t, root, "directory-no-default-fanout.yaml")
	requireCaseID(t, fanoutCase, "directory/no_default_fanout")
	requireCaseAction(t, fanoutCase, "build_list_abilities_invocation")
	requireCaseAction(t, fanoutCase, "project_ability_page")
	requireCaseFixture(t, fanoutCase, "directory-list-abilities-request.v4.json")
	requireCaseFixture(t, fanoutCase, "directory-ability-page.v4.json")
	requireCaseExpectation(t, fanoutCase, "daemon_ability: meta.list_abilities")
	requireCaseExpectation(t, fanoutCase, "invocation_fixture: directory-list-abilities-invocation.v4.json")
	requireCaseExpectation(t, fanoutCase, "fanout: none")

	abilityPage, err := directory.ListAbilities(context.Background(), sharedAbilityQuery(t, root))
	if err != nil {
		t.Fatalf("ListAbilities(shared fixture): %v", err)
	}
	if abilityPage.Limit != 2 || len(abilityPage.Items) != 1 || abilityPage.Metadata["source_ability"] != "meta.list_abilities" {
		t.Fatalf("unexpected shared ability page: %#v", abilityPage)
	}

	resolveCase := sharedCase(t, root, "directory-resolve.yaml")
	requireCaseID(t, resolveCase, "directory/resolve")
	requireCaseAction(t, resolveCase, "build_resolve_invocation")
	requireCaseAction(t, resolveCase, "project_resolved_ref")
	requireCaseFixture(t, resolveCase, "directory-resolve-request.v4.json")
	requireCaseFixture(t, resolveCase, "directory-resolved-ref.v4.json")
	requireCaseExpectation(t, resolveCase, "daemon_ability: namespace.resolve")
	requireCaseExpectation(t, resolveCase, "invocation_fixture: directory-resolve-invocation.v4.json")
	requireCaseExpectation(t, resolveCase, "fanout: none")
	requireCaseExpectation(t, resolveCase, "route_selection_owner: daemon")

	resolved, err := directory.Resolve(context.Background(), sharedResolveQuery(t, root))
	if err != nil {
		t.Fatalf("Resolve(shared fixture): %v", err)
	}
	if resolved.Kind != "resolved_ref" || resolved.AbilityURA == nil || *resolved.AbilityURA != "easynet:///r/example/ability/device.dev-a.agent.list" {
		t.Fatalf("unexpected shared resolved ref: %#v", resolved)
	}

	subscriptionCase := sharedCase(t, root, "directory-subscription-stream.yaml")
	requireCaseID(t, subscriptionCase, "directory/subscription_stream")
	for _, action := range []string{
		"build_directory_subscription_invocation",
		"subscribe_directory",
		"project_directory_subscription",
	} {
		requireCaseAction(t, subscriptionCase, action)
	}
	for _, fixture := range []string{
		"directory-subscription-request.v4.json",
		"directory-subscription-invocation.v4.json",
		"directory-subscription.v4.json",
	} {
		requireCaseFixture(t, subscriptionCase, fixture)
	}
	requireCaseExpectation(t, subscriptionCase, "stream_system_ability: directory.subscribe")
	requireCaseExpectation(t, subscriptionCase, "max_buffered_events: 1024")
	requireCaseExpectation(t, subscriptionCase, "live_requires_snapshot_complete: true")
	requireCaseExpectation(t, subscriptionCase, "facade_fanout: none")

	subscriptionDraft, err := directory.BuildDirectorySubscriptionInvocation(context.Background(), sharedDirectorySubscriptionRequest(t, root))
	if err != nil {
		t.Fatalf("BuildDirectorySubscriptionInvocation(shared fixture): %v", err)
	}
	if subscriptionDraft.DescriptorRef() != "easynet:///r/example/ability/device.dev-a.directory.subscribe@1.0.0" ||
		subscriptionDraft.Metadata()["system_ability"] != "directory.subscribe" {
		t.Fatalf("unexpected shared subscription invocation: %#v", subscriptionDraft)
	}

	subscription, err := directory.SubscribeDirectory(context.Background(), sharedDirectorySubscriptionRequest(t, root))
	if err != nil {
		t.Fatalf("SubscribeDirectory(shared fixture): %v", err)
	}
	if subscription.State != DirectorySubscriptionLive ||
		subscription.ResumeToken != "directory:3" ||
		len(subscription.Events) != 3 ||
		subscription.Events[2].Phase != "live" {
		t.Fatalf("unexpected shared directory subscription: %#v", subscription)
	}

	projectedSubscription, err := NewDirectorySubscriptionFromJSON(sharedFixture(t, root, "directory-subscription.v4.json"))
	if err != nil {
		t.Fatalf("NewDirectorySubscriptionFromJSON(shared fixture): %v", err)
	}
	if projectedSubscription.Cursor.Sequence != 3 ||
		projectedSubscription.Events[1].Phase != "snapshot_complete" {
		t.Fatalf("unexpected projected directory subscription: %#v", projectedSubscription)
	}

	identityCase := sharedCase(t, root, "identity-ura-descriptor-projection.yaml")
	requireCaseID(t, identityCase, "identity/ura_descriptor_projection")
	for _, action := range []string{
		"project_ura",
		"build_ura",
		"project_descriptor_ref",
		"ability_ura_from_descriptor_ref",
		"build_descriptor_ref",
	} {
		requireCaseAction(t, identityCase, action)
	}
	requireCaseExpectation(t, identityCase, "grammar_owner: axon")
	requireCaseExpectation(t, identityCase, "fixture: identity.descriptor-ref.v4.json")
	requireCaseExpectation(t, identityCase, "rejects_malformed_descriptor_ref: true")
	requireCaseExpectation(t, identityCase, "rejects_hand_built_invalid_ura: true")
	requireCaseExpectation(t, identityCase, "directory_list_runtime: provider_backed")

	identity, err := NewIdentityClient(&sharedIdentityTransport{
		t:                                 t,
		expectedBuildURARequest:           []byte(`{"ability_name":"observe.health","kind":"ability","owner_ura":"easynet:///r/example/device/dev-a"}`),
		expectedProjectIdentityRequest:    []byte(`{"ura":"easynet:///r/example/ability/device.dev-a.observe.health"}`),
		expectedDescriptorProjectionInput: "easynet:///r/example/ability/device.dev-a.observe.health@1.0.0",
		expectedBuildDescriptorRefRequest: []byte(`{"ability_ura":"easynet:///r/example/ability/device.dev-a.observe.health","descriptor_version":"1.0.0"}`),
		descriptorJSON:                    sharedFixture(t, root, "identity.descriptor-ref.v4.json"),
		abilityJSON:                       sharedIdentityAbilityProjectionJSON(t, root),
	})
	if err != nil {
		t.Fatalf("NewIdentityClient: %v", err)
	}
	parsed, err := identity.ProjectIdentity(context.Background(), IdentityProjectionRequest{
		URA: "easynet:///r/example/ability/device.dev-a.observe.health",
	})
	if err != nil {
		t.Fatalf("ProjectIdentity(shared ability): %v", err)
	}
	if parsed.Kind != "ability" || parsed.Components["owner_ura"] != "easynet:///r/example/device/dev-a" {
		t.Fatalf("unexpected shared ability projection: %#v", parsed)
	}
	abilityURA, err := identity.OwnerAbilityURA(
		context.Background(),
		"easynet:///r/example/device/dev-a",
		"observe.health",
	)
	if err != nil {
		t.Fatalf("OwnerAbilityURA(shared request): %v", err)
	}
	if abilityURA != "easynet:///r/example/ability/device.dev-a.observe.health" {
		t.Fatalf("unexpected shared ability URA: %q", abilityURA)
	}
	projection, err := identity.ProjectDescriptorRef(context.Background(), DescriptorRefRequest{
		DescriptorRef: "easynet:///r/example/ability/device.dev-a.observe.health@1.0.0",
	})
	if err != nil {
		t.Fatalf("ProjectDescriptorRef(shared fixture): %v", err)
	}
	if !projection.Valid || projection.Metadata["grammar_owner"] != "axon" || projection.DescriptorVersion != "1.0.0" {
		t.Fatalf("unexpected identity descriptor projection: %#v", projection)
	}
	abilityFromDescriptor, err := identity.AbilityURAFromDescriptorRef(
		context.Background(),
		"easynet:///r/example/ability/device.dev-a.observe.health@1.0.0",
	)
	if err != nil {
		t.Fatalf("AbilityURAFromDescriptorRef(shared fixture): %v", err)
	}
	if abilityFromDescriptor != "easynet:///r/example/ability/device.dev-a.observe.health" {
		t.Fatalf("unexpected ability URA from descriptor: %q", abilityFromDescriptor)
	}
	ownerDescriptor, err := identity.OwnerAbilityDescriptorRef(
		context.Background(),
		"easynet:///r/example/device/dev-a",
		"observe.health",
		"1.0.0",
	)
	if err != nil {
		t.Fatalf("OwnerAbilityDescriptorRef(shared request): %v", err)
	}
	if ownerDescriptor != "easynet:///r/example/ability/device.dev-a.observe.health@1.0.0" {
		t.Fatalf("unexpected owner descriptor ref: %q", ownerDescriptor)
	}
	if _, err := NewIdentityProjectionFromJSON([]byte(`{"kind":"descriptor_ref","valid":true,"profile":"easynet-strict-v2","components":{},"metadata":{}}`)); !IsCode(err, ErrInvalidArgument) {
		t.Fatalf("malformed descriptor projection did not produce InvalidArgument: %v", err)
	}
	if _, err := identity.ProjectIdentity(context.Background(), IdentityProjectionRequest{}); !IsCode(err, ErrInvalidArgument) {
		t.Fatalf("hand-built invalid URA request did not produce InvalidArgument: %v", err)
	}
}

func TestGoPublicationFacadeExecutesSharedCarrierConformanceCase(t *testing.T) {
	root := repositoryRoot(t)
	publicationCase := sharedCase(t, root, "publication-resource-carriers.yaml")
	requireCaseID(t, publicationCase, "publication/resource_carriers")
	for _, action := range []string{
		"build_resource_ref",
		"validate_package",
		"build_deploy_invocation",
		"build_unpublish_invocation",
	} {
		requireCaseAction(t, publicationCase, action)
	}
	for _, fixture := range []string{
		"local-resource-ref-request.v4.json",
		"ability-package-manifest.v4.json",
		"ability-deploy-request.v4.json",
	} {
		requireCaseFixture(t, publicationCase, fixture)
	}
	requireCaseExpectation(t, publicationCase, "resource_ref_fixture: resource-ref.local-fs.v4.json")
	requireCaseExpectation(t, publicationCase, "package_validation_fixture: package-validation.v4.json")
	requireCaseExpectation(t, publicationCase, "deploy_invocation_fixture: publication-deploy-invocation.v4.json")
	requireCaseExpectation(t, publicationCase, "unpublish_invocation_fixture: publication-unpublish-invocation.v4.json")
	requireCaseExpectation(t, publicationCase, "deploy_system_ability: ability.deploy")
	requireCaseExpectation(t, publicationCase, "unpublish_system_ability: ability.unpublish")
	requireCaseExpectation(t, publicationCase, "rejects_relative_path: true")
	requireCaseExpectation(t, publicationCase, "rejects_reserved_namespace: true")
	requireCaseExpectation(t, publicationCase, "rejects_incomplete_invocation_tuple: true")

	publication, err := NewPublicationClient(&sharedPublicationTransport{
		t:                       t,
		expectedResourceRequest: sharedFixture(t, root, "local-resource-ref-request.v4.json"),
		expectedValidateRequest: sharedPublicationValidatePackageRequest(t, root),
		expectedDeployRequest:   sharedFixture(t, root, "ability-deploy-request.v4.json"),
		resourceJSON:            sharedFixture(t, root, "resource-ref.local-fs.v4.json"),
		validationJSON:          sharedFixture(t, root, "package-validation.v4.json"),
		deployInvocationJSON:    sharedFixture(t, root, "publication-deploy-invocation.v4.json"),
		unpublishInvocationJSON: sharedFixture(t, root, "publication-unpublish-invocation.v4.json"),
	})
	if err != nil {
		t.Fatalf("NewPublicationClient: %v", err)
	}

	resourceReq := sharedLocalResourceRefRequest(t, root)
	resource, err := publication.BuildLocalResourceRef(context.Background(), resourceReq)
	if err != nil {
		t.Fatalf("BuildLocalResourceRef(shared fixture): %v", err)
	}
	if resource.Namespace != "fs" || resource.Capability != "read" || resource.Revision != "fs-local-mapping-v1" {
		t.Fatalf("unexpected shared resource ref: %#v", resource)
	}
	if _, err := publication.BuildLocalResourceRef(context.Background(), LocalResourceRefRequest{
		Path:       "tmp/easynet-weather-package",
		Capability: "read",
	}); !IsCode(err, ErrInvalidArgument) {
		t.Fatalf("relative publication resource path did not produce InvalidArgument: %v", err)
	}

	manifest := sharedAbilityPackageManifest(t, root)
	validation, err := publication.ValidatePackage(context.Background(), "", ValidatePackageOptions{Manifest: &manifest})
	if err != nil {
		t.Fatalf("ValidatePackage(shared manifest): %v", err)
	}
	if !validation.Valid || validation.Manifest.WireKey != "er.weather" || validation.Metadata["frame_contract_owner"] != "daemon_sdk" {
		t.Fatalf("unexpected shared package validation: %#v", validation)
	}

	deployReq := sharedAbilityDeployRequest(t, root)
	deploy, err := publication.BuildDeployInvocation(context.Background(), deployReq)
	if err != nil {
		t.Fatalf("BuildDeployInvocation(shared fixture): %v", err)
	}
	if deploy.DescriptorRef() != "easynet:///r/example/ability/device.dev-a.ability.deploy@1.0.0" ||
		deploy.Metadata()["system_ability"] != "ability.deploy" {
		t.Fatalf("unexpected shared deploy invocation: %#v", deploy)
	}
	deployReq.ResourceRef.Namespace = "system"
	if _, err := publication.BuildDeployInvocation(context.Background(), deployReq); !IsCode(err, ErrInvalidArgument) {
		t.Fatalf("reserved publication resource namespace did not produce InvalidArgument: %v", err)
	}
	deployReq = sharedAbilityDeployRequest(t, root)
	deployReq.CallerURA = ""
	if _, err := publication.BuildDeployInvocation(context.Background(), deployReq); !IsCode(err, ErrInvalidArgument) {
		t.Fatalf("incomplete publication deploy carrier did not produce InvalidArgument: %v", err)
	}

	unpublish, err := publication.BuildUnpublishInvocation(context.Background(), sharedUnpublishAbilityRequest(t, root))
	if err != nil {
		t.Fatalf("BuildUnpublishInvocation(shared fixture): %v", err)
	}
	if unpublish.DescriptorRef() != "easynet:///r/example/ability/device.dev-a.ability.unpublish@1.0.0" ||
		unpublish.Metadata()["system_ability"] != "ability.unpublish" {
		t.Fatalf("unexpected shared unpublish invocation: %#v", unpublish)
	}
}

func TestGoMissionFacadeExecutesSharedCarrierStatusConformanceCase(t *testing.T) {
	root := repositoryRoot(t)
	missionCase := sharedCase(t, root, "mission-carrier-status.yaml")
	requireCaseID(t, missionCase, "mission/carrier_status")
	for _, action := range []string{
		"build_run_eal_invocation",
		"build_run_file_invocation",
		"build_track_invocation",
		"build_cancel_invocation",
		"project_status",
		"project_events",
	} {
		requireCaseAction(t, missionCase, action)
	}
	for _, fixture := range []string{
		"mission-run-request.v4.json",
		"mission-run-file-request.v4.json",
		"mission-track-request.v4.json",
		"mission-cancel-request.v4.json",
		"mission-status.v4.json",
		"mission-events-request.v4.json",
		"mission-event-page.v4.json",
	} {
		requireCaseFixture(t, missionCase, fixture)
	}
	requireCaseExpectation(t, missionCase, "run_invocation_fixture: mission-run-invocation.v4.json")
	requireCaseExpectation(t, missionCase, "track_invocation_fixture: mission-track-invocation.v4.json")
	requireCaseExpectation(t, missionCase, "cancel_invocation_fixture: mission-cancel-invocation.v4.json")
	requireCaseExpectation(t, missionCase, "run_system_ability: mission.run")
	requireCaseExpectation(t, missionCase, "track_system_ability: mission.track")
	requireCaseExpectation(t, missionCase, "cancel_system_ability: mission.cancel")
	requireCaseExpectation(t, missionCase, "rejects_incomplete_invocation_tuple: true")
	requireCaseExpectation(t, missionCase, "rejects_path_like_mission_id: true")
	requireCaseExpectation(t, missionCase, "child_receipts_only_when_anchored: true")
	requireCaseExpectation(t, missionCase, "events_system_ability: mission.events")
	requireCaseExpectation(t, missionCase, "mission_events_page_projection: true")
	requireCaseExpectation(t, missionCase, "mission_events_live_tail: bounded_page_state_machine")
	requireCaseExpectation(t, missionCase, "mission_plan_child_invocation_conformance: true")

	mission, err := NewMissionClient(&sharedMissionTransport{
		t:                      t,
		expectedRunRequest:     sharedFixture(t, root, "mission-run-request.v4.json"),
		expectedRunFileRequest: sharedFixture(t, root, "mission-run-file-request.v4.json"),
		expectedTrackRequest:   sharedFixture(t, root, "mission-track-request.v4.json"),
		expectedCancelRequest:  sharedFixture(t, root, "mission-cancel-request.v4.json"),
		expectedEventsRequest:  sharedFixture(t, root, "mission-events-request.v4.json"),
		runInvocationJSON:      sharedFixture(t, root, "mission-run-invocation.v4.json"),
		trackInvocationJSON:    sharedFixture(t, root, "mission-track-invocation.v4.json"),
		cancelInvocationJSON:   sharedFixture(t, root, "mission-cancel-invocation.v4.json"),
		statusJSON:             sharedFixture(t, root, "mission-status.v4.json"),
		eventsJSON:             sharedFixture(t, root, "mission-event-page.v4.json"),
	})
	if err != nil {
		t.Fatalf("NewMissionClient: %v", err)
	}

	run, err := mission.BuildRunEALInvocation(context.Background(), sharedMissionRunRequest(t, root))
	if err != nil {
		t.Fatalf("BuildRunEALInvocation(shared fixture): %v", err)
	}
	if run.DescriptorRef() != "easynet:///r/example/ability/device.dev-a.mission.run@1.0.0" ||
		run.Metadata()["system_ability"] != "mission.run" {
		t.Fatalf("unexpected shared mission run invocation: %#v", run)
	}

	runFile, err := mission.BuildRunFileInvocation(context.Background(), sharedMissionRunFileRequest(t, root))
	if err != nil {
		t.Fatalf("BuildRunFileInvocation(shared fixture): %v", err)
	}
	if runFile.DescriptorRef() != run.DescriptorRef() || runFile.Metadata()["system_ability"] != "mission.run" {
		t.Fatalf("unexpected shared mission run-file invocation: %#v", runFile)
	}

	track, err := mission.BuildTrackInvocation(context.Background(), sharedMissionTrackRequest(t, root))
	if err != nil {
		t.Fatalf("BuildTrackInvocation(shared fixture): %v", err)
	}
	if track.DescriptorRef() != "easynet:///r/example/ability/device.dev-a.mission.track@1.0.0" ||
		track.Metadata()["system_ability"] != "mission.track" {
		t.Fatalf("unexpected shared mission track invocation: %#v", track)
	}

	cancel, err := mission.BuildCancelInvocation(context.Background(), sharedMissionCancelRequest(t, root))
	if err != nil {
		t.Fatalf("BuildCancelInvocation(shared fixture): %v", err)
	}
	if cancel.DescriptorRef() != "easynet:///r/example/ability/device.dev-a.mission.cancel@1.0.0" ||
		cancel.Metadata()["system_ability"] != "mission.cancel" {
		t.Fatalf("unexpected shared mission cancel invocation: %#v", cancel)
	}

	status, err := mission.Track(context.Background(), sharedMissionTrackRequest(t, root))
	if err != nil {
		t.Fatalf("Track(shared status fixture): %v", err)
	}
	if !status.Terminal || status.State != "partial" || status.ParentReceiptURA == nil ||
		len(status.ChildReceipts) != 1 || len(status.OutputRefs) != 4 {
		t.Fatalf("unexpected shared mission status: %#v", status)
	}

	eventPage, err := mission.Events(context.Background(), sharedMissionEventsRequest(t, root))
	if err != nil {
		t.Fatalf("Events(shared fixture): %v", err)
	}
	if eventPage.Kind != "mission_event_page" || eventPage.CursorSequence != 4 ||
		eventPage.NextCursorSequence != 7 || eventPage.HasMore ||
		eventPage.DroppedCount != 0 || len(eventPage.Events) != 2 ||
		!eventPage.Events[1].Terminal || eventPage.Events[1].Receipt["receipt_ura"] != "easynet:///r/example/receipt/parent" {
		t.Fatalf("unexpected shared mission event page: %#v", eventPage)
	}

	badRun := sharedMissionRunRequest(t, root)
	badRun.CallerURA = ""
	if _, err := mission.BuildRunEALInvocation(context.Background(), badRun); !IsCode(err, ErrInvalidArgument) {
		t.Fatalf("incomplete mission carrier did not produce InvalidArgument: %v", err)
	}
	if _, err := mission.BuildTrackInvocation(context.Background(), MissionTrackRequest{
		MissionCarrierBase: sharedMissionCarrierBase(t, root, "mission-track-request.v4.json"),
		MissionID:          "/tmp/mission",
	}); !IsCode(err, ErrInvalidArgument) {
		t.Fatalf("path-like mission id did not produce InvalidArgument: %v", err)
	}
	if _, err := NewMissionStatusFromJSON(sharedMissionStatusWithoutParentAnchor(t, root)); !IsCode(err, ErrInvalidArgument) {
		t.Fatalf("unanchored mission child receipt did not produce InvalidArgument: %v", err)
	}
}

func TestGoMissionFacadeExecutesSharedPlanChildInvocationConformanceCase(t *testing.T) {
	root := repositoryRoot(t)
	planCase := sharedCase(t, root, "mission-plan-child-invocation.yaml")
	requireCaseID(t, planCase, "mission/plan_child_invocation")
	for _, action := range []string{
		"render_plan_eal",
		"project_child_invocation_intents",
		"validate_child_invocation_facts",
		"reject_foreign_step_output",
		"reject_structured_plan_field",
	} {
		requireCaseAction(t, planCase, action)
	}
	requireCaseFixture(t, planCase, "mission-status.v4.json")
	for _, expected := range []string{
		"plan_name: nightly",
		"first_step: observe.health",
		"second_step: notify.user",
		"step_output_ref: health.output",
		"mismatch_reason: mission_child_invocation_mismatch",
		"rejects_foreign_step_output: true",
		"rejects_structured_plan_field: true",
		"receipt_backed_steps: true",
		"sdk_executes_mission: false",
	} {
		requireCaseExpectation(t, planCase, expected)
	}

	plan, err := NewMissionPlan("nightly")
	if err != nil {
		t.Fatalf("NewMissionPlan(shared plan): %v", err)
	}
	health, err := plan.Step("observe.health", MissionPlanStepOptions{})
	if err != nil {
		t.Fatalf("health step(shared plan): %v", err)
	}
	if _, err := plan.Step("notify.user", MissionPlanStepOptions{Args: map[string]any{"source": health.Output()}}); err != nil {
		t.Fatalf("notify step(shared plan): %v", err)
	}
	eal, err := plan.ToEAL()
	if err != nil {
		t.Fatalf("ToEAL(shared plan): %v", err)
	}
	if !strings.Contains(eal, `mission "nightly"`) ||
		!strings.Contains(eal, `let health = call "observe.health"`) ||
		!strings.Contains(eal, `let user = call "notify.user" with { source = health.output }`) {
		t.Fatalf("shared plan EAL mismatch:\n%s", eal)
	}
	intents := plan.ChildInvocationIntents()
	if len(intents) != 2 || intents[0].StepID != "health" || intents[1].Ability != "notify.user" {
		t.Fatalf("shared plan intents mismatch: %#v", intents)
	}

	status, err := NewMissionStatusFromJSON(
		[]byte(strings.ReplaceAll(string(sharedFixture(t, root, "mission-status.v4.json")), `"step_id": "s1"`, `"step_id": "health"`)),
	)
	if err != nil {
		t.Fatalf("NewMissionStatusFromJSON(shared plan status): %v", err)
	}
	observedOnly, err := NewMissionPlan("nightly")
	if err != nil {
		t.Fatalf("NewMissionPlan(observed-only): %v", err)
	}
	if _, err := observedOnly.Step("observe.health", MissionPlanStepOptions{}); err != nil {
		t.Fatalf("observed-only step: %v", err)
	}
	conformance, err := observedOnly.ValidateChildInvocations(status)
	if err != nil {
		t.Fatalf("ValidateChildInvocations(observed-only): %v", err)
	}
	if !conformance.Passed() || len(conformance.ReceiptBackedSteps) != 1 || conformance.ReceiptBackedSteps[0] != "health" {
		t.Fatalf("shared plan conformance mismatch: %#v", conformance)
	}
	if _, err := plan.ValidateChildInvocations(status); !IsCode(err, ErrProtocol) {
		t.Fatalf("missing child Invocation facts error = %v, want %s", err, ErrProtocol)
	}

	foreign, err := NewMissionPlan("foreign")
	if err != nil {
		t.Fatalf("NewMissionPlan(foreign): %v", err)
	}
	foreignStep, err := foreign.Step("er.src", MissionPlanStepOptions{})
	if err != nil {
		t.Fatalf("foreign step(shared plan): %v", err)
	}
	if _, err := observedOnly.Step("er.fn", MissionPlanStepOptions{Args: map[string]any{"data": foreignStep.Output()}}); !IsCode(err, ErrInvalidArgument) {
		t.Fatalf("foreign step output error = %v, want %s", err, ErrInvalidArgument)
	}
	if _, err := observedOnly.Step("er.fn", MissionPlanStepOptions{Args: map[string]any{"payload": map[string]any{"nested": 1}}}); !IsCode(err, ErrInvalidArgument) {
		t.Fatalf("structured plan field error = %v, want %s", err, ErrInvalidArgument)
	}
}

func TestGoAdminGatewayFacadeExecutesSharedCarrierStatusConformanceCase(t *testing.T) {
	root := repositoryRoot(t)
	adminCase := sharedCase(t, root, "admin-gateway-carrier-status.yaml")
	requireCaseID(t, adminCase, "admin_gateway/carrier_status")
	for _, action := range []string{
		"build_agent_list_invocation",
		"build_agent_start_invocation",
		"build_agent_stop_invocation",
		"build_agent_refresh_invocation",
		"build_session_list_invocation",
		"project_gateway_status",
		"project_agent_records",
		"project_agent_lifecycle_result",
	} {
		requireCaseAction(t, adminCase, action)
	}
	for _, fixture := range []string{
		"admin-agent-list-request.v4.json",
		"admin-agent-start-request.v4.json",
		"admin-agent-stop-request.v4.json",
		"admin-agent-refresh-request.v4.json",
		"admin-session-list-request.v4.json",
		"admin-pairing-preflight-request.v4.json",
		"admin-pairing-create-request.v4.json",
		"admin-pairing-validate-request.v4.json",
		"admin-device-session-create-request.v4.json",
		"admin-device-session-delete-request.v4.json",
		"gateway-status.v4.json",
		"admin-agent-records.v4.json",
		"admin-agent-lifecycle-result.v4.json",
		"admin-pairing-preflight.v4.json",
		"admin-pairing-token.v4.json",
		"admin-device-credential.v4.json",
		"admin-device-session.v4.json",
		"admin-device-session-page.v4.json",
		"admin-device-session-delete-result.v4.json",
	} {
		requireCaseFixture(t, adminCase, fixture)
	}
	requireCaseExpectation(t, adminCase, "agent_start_invocation_fixture: admin-agent-start-invocation.v4.json")
	requireCaseExpectation(t, adminCase, "agent_stop_invocation_fixture: admin-agent-stop-invocation.v4.json")
	requireCaseExpectation(t, adminCase, "agent_list_invocation_fixture: admin-agent-list-invocation.v4.json")
	requireCaseExpectation(t, adminCase, "session_list_invocation_fixture: admin-session-list-invocation.v4.json")
	requireCaseExpectation(t, adminCase, "rejects_incomplete_invocation_tuple: true")
	requireCaseExpectation(t, adminCase, "rejects_system_agent_lifecycle: true")
	requireCaseExpectation(t, adminCase, "preserves_control_only_degraded_state: true")
	requireCaseExpectation(t, adminCase, "pairing_preflight_fixture: admin-pairing-preflight.v4.json")
	requireCaseExpectation(t, adminCase, "pairing_token_fixture: admin-pairing-token.v4.json")
	requireCaseExpectation(t, adminCase, "device_credential_fixture: admin-device-credential.v4.json")
	requireCaseExpectation(t, adminCase, "device_session_fixture: admin-device-session.v4.json")
	requireCaseExpectation(t, adminCase, "device_session_page_fixture: admin-device-session-page.v4.json")
	requireCaseExpectation(t, adminCase, "device_session_delete_fixture: admin-device-session-delete-result.v4.json")
	requireCaseExpectation(t, adminCase, "pairing_and_device_session_crud: provider_backed")

	admin, err := NewAdminClient(&sharedAdminGatewayTransport{
		t:                                  t,
		expectedAgentListRequest:           sharedFixture(t, root, "admin-agent-list-request.v4.json"),
		expectedAgentStartRequest:          sharedFixture(t, root, "admin-agent-start-request.v4.json"),
		expectedAgentStopRequest:           sharedFixture(t, root, "admin-agent-stop-request.v4.json"),
		expectedAgentRefreshRequest:        sharedFixture(t, root, "admin-agent-refresh-request.v4.json"),
		expectedSessionListRequest:         sharedFixture(t, root, "admin-session-list-request.v4.json"),
		expectedPairingPreflightRequest:    sharedFixture(t, root, "admin-pairing-preflight-request.v4.json"),
		expectedPairingCreateRequest:       sharedFixture(t, root, "admin-pairing-create-request.v4.json"),
		expectedPairingValidateRequest:     sharedFixture(t, root, "admin-pairing-validate-request.v4.json"),
		expectedDeviceSessionCreateRequest: sharedFixture(t, root, "admin-device-session-create-request.v4.json"),
		expectedDeviceSessionDeleteRequest: sharedFixture(t, root, "admin-device-session-delete-request.v4.json"),
		agentListInvocationJSON:            sharedFixture(t, root, "admin-agent-list-invocation.v4.json"),
		agentStartInvocationJSON:           sharedFixture(t, root, "admin-agent-start-invocation.v4.json"),
		agentStopInvocationJSON:            sharedFixture(t, root, "admin-agent-stop-invocation.v4.json"),
		agentRefreshInvocationJSON:         sharedFixture(t, root, "admin-agent-refresh-invocation.v4.json"),
		sessionListInvocationJSON:          sharedFixture(t, root, "admin-session-list-invocation.v4.json"),
		gatewayStatusJSON:                  sharedFixture(t, root, "gateway-status.v4.json"),
		agentRecordsJSON:                   sharedFixture(t, root, "admin-agent-records.v4.json"),
		agentLifecycleResultJSON:           sharedFixture(t, root, "admin-agent-lifecycle-result.v4.json"),
		pairingPreflightJSON:               sharedFixture(t, root, "admin-pairing-preflight.v4.json"),
		pairingTokenJSON:                   sharedFixture(t, root, "admin-pairing-token.v4.json"),
		deviceCredentialJSON:               sharedFixture(t, root, "admin-device-credential.v4.json"),
		deviceSessionJSON:                  sharedFixture(t, root, "admin-device-session.v4.json"),
		deviceSessionPageJSON:              sharedFixture(t, root, "admin-device-session-page.v4.json"),
		deviceSessionDeleteResultJSON:      sharedFixture(t, root, "admin-device-session-delete-result.v4.json"),
		expectedGatewayStatusRequest:       []byte(`{}`),
	})
	if err != nil {
		t.Fatalf("NewAdminClient: %v", err)
	}

	agentList, err := admin.BuildAgentListInvocation(context.Background(), sharedAdminAgentListRequest(t, root))
	if err != nil {
		t.Fatalf("BuildAgentListInvocation(shared fixture): %v", err)
	}
	if agentList.DescriptorRef() != "easynet:///r/example/ability/device.dev-a.agent.list@1.0.0" ||
		agentList.Metadata()["system_ability"] != "agent.list" {
		t.Fatalf("unexpected shared agent-list invocation: %#v", agentList)
	}

	agentStart, err := admin.BuildAgentStartInvocation(context.Background(), sharedAdminAgentStartRequest(t, root))
	if err != nil {
		t.Fatalf("BuildAgentStartInvocation(shared fixture): %v", err)
	}
	if agentStart.DescriptorRef() != "easynet:///r/example/ability/device.dev-a.agent.start@1.0.0" ||
		agentStart.Metadata()["system_ability"] != "agent.start" {
		t.Fatalf("unexpected shared agent-start invocation: %#v", agentStart)
	}

	agentStop, err := admin.BuildAgentStopInvocation(context.Background(), sharedAdminAgentStopRequest(t, root))
	if err != nil {
		t.Fatalf("BuildAgentStopInvocation(shared fixture): %v", err)
	}
	if agentStop.DescriptorRef() != "easynet:///r/example/ability/device.dev-a.agent.stop@1.0.0" ||
		agentStop.Metadata()["system_ability"] != "agent.stop" {
		t.Fatalf("unexpected shared agent-stop invocation: %#v", agentStop)
	}

	agentRefresh, err := admin.BuildAgentRefreshInvocation(context.Background(), sharedAdminAgentRefreshRequest(t, root))
	if err != nil {
		t.Fatalf("BuildAgentRefreshInvocation(shared fixture): %v", err)
	}
	if agentRefresh.DescriptorRef() != "easynet:///r/example/ability/device.dev-a.agent.refresh@1.0.0" ||
		agentRefresh.Metadata()["system_ability"] != "agent.refresh" {
		t.Fatalf("unexpected shared agent-refresh invocation: %#v", agentRefresh)
	}

	sessionList, err := admin.BuildSessionListInvocation(context.Background(), sharedAdminSessionListRequest(t, root))
	if err != nil {
		t.Fatalf("BuildSessionListInvocation(shared fixture): %v", err)
	}
	if sessionList.DescriptorRef() != "easynet:///r/example/ability/device.dev-a.session.list@1.0.0" ||
		sessionList.Metadata()["system_ability"] != "session.list" {
		t.Fatalf("unexpected shared session-list invocation: %#v", sessionList)
	}

	status, err := admin.GatewayStatus(context.Background(), AdminGatewayStatusRequest{})
	if err != nil {
		t.Fatalf("GatewayStatus(shared fixture): %v", err)
	}
	if !status.Ready || !status.ControlReady || !status.RuntimeReady || status.PublicListenerReady {
		t.Fatalf("unexpected shared gateway status: %#v", status)
	}

	agents, err := admin.ListAgents(context.Background(), sharedAdminAgentListRequest(t, root))
	if err != nil {
		t.Fatalf("ListAgents(shared fixture): %v", err)
	}
	if agents.Kind != "agent_records" || len(agents.Items) != 1 || agents.Items[0].Name != "codex" {
		t.Fatalf("unexpected shared admin agent records: %#v", agents)
	}

	lifecycle, err := admin.AgentStart(context.Background(), sharedAdminAgentStartRequest(t, root))
	if err != nil {
		t.Fatalf("AgentStart(shared fixture): %v", err)
	}
	if lifecycle.Kind != "agent_lifecycle_result" || lifecycle.State != "ok" || lifecycle.RuntimeNotReady {
		t.Fatalf("unexpected shared admin lifecycle result: %#v", lifecycle)
	}

	preflight, err := admin.PairingPreflight(context.Background(), sharedAdminPairingPreflightRequest(t, root))
	if err != nil {
		t.Fatalf("PairingPreflight(shared fixture): %v", err)
	}
	if !preflight.PairingRequired || preflight.TrustReady || len(preflight.Scopes) != 2 {
		t.Fatalf("unexpected shared pairing preflight: %#v", preflight)
	}

	token, err := admin.CreatePairing(context.Background(), sharedAdminPairingCreateRequest(t, root))
	if err != nil {
		t.Fatalf("CreatePairing(shared fixture): %v", err)
	}
	if token.TokenID != "pair-token-1" || token.Token != "pair-token-value" {
		t.Fatalf("unexpected shared pairing token: %#v", token)
	}

	credential, err := admin.ValidatePairing(context.Background(), sharedAdminPairingValidateRequest(t, root))
	if err != nil {
		t.Fatalf("ValidatePairing(shared fixture): %v", err)
	}
	if credential.CredentialID != "cred-dev-a" || credential.State != "active" {
		t.Fatalf("unexpected shared device credential: %#v", credential)
	}

	session, err := admin.CreateDeviceSession(context.Background(), sharedAdminDeviceSessionCreateRequest(t, root))
	if err != nil {
		t.Fatalf("CreateDeviceSession(shared fixture): %v", err)
	}
	if session.SessionID != "dev-session-1" || session.SessionKind != "remote_desktop" {
		t.Fatalf("unexpected shared device session: %#v", session)
	}

	sessions, err := admin.ListDeviceSessions(context.Background(), sharedAdminSessionListRequest(t, root))
	if err != nil {
		t.Fatalf("ListDeviceSessions(shared fixture): %v", err)
	}
	if sessions.Kind != "device_sessions" || len(sessions.Items) != 1 || sessions.Items[0].SessionID != "dev-session-1" {
		t.Fatalf("unexpected shared device session page: %#v", sessions)
	}

	deleted, err := admin.DeleteDeviceSession(context.Background(), sharedAdminDeviceSessionDeleteRequest(t, root))
	if err != nil {
		t.Fatalf("DeleteDeviceSession(shared fixture): %v", err)
	}
	if deleted.Kind != "device_admin_result" || deleted.Operation != "session.delete" || deleted.Ack == nil || !*deleted.Ack {
		t.Fatalf("unexpected shared delete session result: %#v", deleted)
	}

	incomplete := sharedAdminAgentStartRequest(t, root)
	incomplete.CallerURA = ""
	if _, err := admin.BuildAgentStartInvocation(context.Background(), incomplete); !IsCode(err, ErrInvalidArgument) {
		t.Fatalf("incomplete admin carrier did not produce InvalidArgument: %v", err)
	}

	systemAgent := sharedAdminAgentStartRequest(t, root)
	systemAgent.Name = "device"
	if _, err := admin.BuildAgentStartInvocation(context.Background(), systemAgent); !IsCode(err, ErrInvalidArgument) {
		t.Fatalf("system agent lifecycle did not produce InvalidArgument: %v", err)
	}

	degraded, err := NewGatewayStatusFromJSON(sharedControlOnlyGatewayStatus(t, root))
	if err != nil {
		t.Fatalf("NewGatewayStatusFromJSON(control-only degraded fixture): %v", err)
	}
	if degraded.Ready || degraded.State != "degraded" || !degraded.ControlReady || degraded.RuntimeReady {
		t.Fatalf("control-only degraded gateway state was not preserved: %#v", degraded)
	}
}

func TestGoEventsFacadeExecutesSharedDirectoryStreamConformanceCase(t *testing.T) {
	root := repositoryRoot(t)
	eventsCase := sharedCase(t, root, "events-directory-stream.yaml")
	requireCaseID(t, eventsCase, "events/directory_stream")
	for _, action := range []string{
		"build_directory_subscription_invocation",
		"project_directory_event",
		"project_drop_report",
		"project_terminal",
	} {
		requireCaseAction(t, eventsCase, action)
	}
	for _, fixture := range []string{
		"events-directory-subscription-request.v4.json",
		"event.directory.v4.json",
		"event.directory-drop-report.v4.json",
		"event.directory-terminal.v4.json",
	} {
		requireCaseFixture(t, eventsCase, fixture)
	}
	requireCaseExpectation(t, eventsCase, "subscription_invocation_fixture: events-directory-subscription-invocation.v4.json")
	requireCaseExpectation(t, eventsCase, "stream_system_ability: federation.subscribe_directory_v2")
	requireCaseExpectation(t, eventsCase, "cursor_required: true")
	requireCaseExpectation(t, eventsCase, "dropped_events_are_first_class: true")
	requireCaseExpectation(t, eventsCase, "terminal_frame_explicit: true")
	requireCaseExpectation(t, eventsCase, "related_event_streams_case: events-device-invocation-history.yaml")

	events, err := NewEventClient(&sharedEventsTransport{
		t:                                    t,
		expectedDirectorySubscriptionRequest: sharedEventsDirectorySubscriptionRequestJSON(t, root),
		expectedDirectoryProjectionInput:     sharedEventsProjectionInputJSON(t, root),
		expectedDropReportInput:              sharedEventsDropReportInputJSON(t, root),
		expectedTerminalInput:                sharedEventsTerminalInputJSON(t, root),
		directorySubscriptionInvocationJSON:  sharedFixture(t, root, "events-directory-subscription-invocation.v4.json"),
		directoryEventJSON:                   sharedFixture(t, root, "event.directory.v4.json"),
		dropReportJSON:                       sharedFixture(t, root, "event.directory-drop-report.v4.json"),
		terminalJSON:                         sharedFixture(t, root, "event.directory-terminal.v4.json"),
	})
	if err != nil {
		t.Fatalf("NewEventClient: %v", err)
	}

	subscription, err := events.BuildDirectorySubscriptionInvocation(context.Background(), sharedEventsDirectorySubscriptionRequest(t, root))
	if err != nil {
		t.Fatalf("BuildDirectorySubscriptionInvocation(shared fixture): %v", err)
	}
	if subscription.DescriptorRef() != "easynet:///r/example/ability/device.dev-a.federation.subscribe_directory_v2@1.0.0" ||
		subscription.Metadata()["system_ability"] != "federation.subscribe_directory_v2" {
		t.Fatalf("unexpected shared events subscription invocation: %#v", subscription)
	}

	directoryEvent, err := events.ProjectDirectoryEvent(context.Background(), sharedEventsProjectionInput(t, root))
	if err != nil {
		t.Fatalf("ProjectDirectoryEvent(shared fixture): %v", err)
	}
	if directoryEvent.Kind != "directory.agent_advertised" || directoryEvent.Cursor.Token != "directory:8" ||
		directoryEvent.Terminal || directoryEvent.Metadata["stream_ability"] != "federation.subscribe_directory_v2" {
		t.Fatalf("unexpected shared directory event frame: %#v", directoryEvent)
	}

	dropReport, err := events.ProjectDropReport(context.Background(), sharedEventsDropReportInput(t, root))
	if err != nil {
		t.Fatalf("ProjectDropReport(shared fixture): %v", err)
	}
	if dropReport.Kind != "directory.drop_report" || dropReport.DroppedCount != 4 ||
		dropReport.ReconnectAfterMS == nil || *dropReport.ReconnectAfterMS != 1000 {
		t.Fatalf("unexpected shared events drop report: %#v", dropReport)
	}

	terminal, err := events.ProjectTerminal(context.Background(), sharedEventsTerminalInput(t, root))
	if err != nil {
		t.Fatalf("ProjectTerminal(shared fixture): %v", err)
	}
	if terminal.Kind != "directory.terminal" || !terminal.Terminal || terminal.ResumeToken != "terminal" {
		t.Fatalf("unexpected shared events terminal frame: %#v", terminal)
	}

	incomplete := sharedEventsDirectorySubscriptionRequest(t, root)
	incomplete.CallerURA = ""
	if _, err := events.BuildDirectorySubscriptionInvocation(context.Background(), incomplete); !IsCode(err, ErrInvalidArgument) {
		t.Fatalf("incomplete events carrier did not produce InvalidArgument: %v", err)
	}
	if _, err := NewEventFrameFromJSON(sharedEventsFrameWithoutCursorToken(t, root, "event.directory.v4.json")); !IsCode(err, ErrInvalidArgument) {
		t.Fatalf("event frame without cursor token did not produce InvalidArgument: %v", err)
	}
	if _, err := NewEventFrameFromJSON(sharedEventsDropReportWithoutDroppedCount(t, root)); !IsCode(err, ErrInvalidArgument) {
		t.Fatalf("drop report without positive dropped count did not produce InvalidArgument: %v", err)
	}
	if _, err := NewEventFrameFromJSON(sharedEventsTerminalWithoutTerminalFlag(t, root)); !IsCode(err, ErrInvalidArgument) {
		t.Fatalf("terminal frame without terminal=true did not produce InvalidArgument: %v", err)
	}
}

func TestGoEventsFacadeExecutesSharedDeviceInvocationHistoryConformanceCase(t *testing.T) {
	root := repositoryRoot(t)
	eventsCase := sharedCase(t, root, "events-device-invocation-history.yaml")
	requireCaseID(t, eventsCase, "events/device_invocation_history")
	for _, action := range []string{
		"build_device_subscription_invocation",
		"build_invocation_subscription_invocation",
		"build_device_event_history_invocation",
		"project_device_event_page",
	} {
		requireCaseAction(t, eventsCase, action)
	}
	for _, fixture := range []string{
		"events-device-subscription-request.v4.json",
		"events-invocation-subscription-request.v4.json",
		"events-device-event-list-request.v4.json",
		"event.device-page.v4.json",
	} {
		requireCaseFixture(t, eventsCase, fixture)
	}
	requireCaseExpectation(t, eventsCase, "device_subscription_invocation_fixture: events-device-subscription-invocation.v4.json")
	requireCaseExpectation(t, eventsCase, "invocation_subscription_invocation_fixture: events-invocation-subscription-invocation.v4.json")
	requireCaseExpectation(t, eventsCase, "device_history_invocation_fixture: events-device-history-invocation.v4.json")
	requireCaseExpectation(t, eventsCase, "device_stream_system_ability: events.device.subscribe")
	requireCaseExpectation(t, eventsCase, "invocation_stream_system_ability: events.invocation.subscribe")
	requireCaseExpectation(t, eventsCase, "device_history_system_ability: events.device.history")
	requireCaseExpectation(t, eventsCase, "sdk_local_event_bus_allowed: false")
	requireCaseExpectation(t, eventsCase, "daemon_side_filtering_backend_cutover: incomplete")

	events, err := NewEventClient(&sharedEventsTransport{
		t:                                     t,
		expectedDeviceSubscriptionRequest:     sharedEventsDeviceSubscriptionRequestJSON(t, root),
		expectedInvocationSubscriptionRequest: sharedEventsInvocationSubscriptionRequestJSON(t, root),
		expectedDeviceEventListRequest:        sharedEventsDeviceEventListRequestJSON(t, root),
		deviceSubscriptionInvocationJSON:      sharedFixture(t, root, "events-device-subscription-invocation.v4.json"),
		invocationSubscriptionInvocationJSON:  sharedFixture(t, root, "events-invocation-subscription-invocation.v4.json"),
		deviceEventPageJSON:                   sharedFixture(t, root, "event.device-page.v4.json"),
	})
	if err != nil {
		t.Fatalf("NewEventClient: %v", err)
	}

	deviceSubscription, err := events.BuildDeviceSubscriptionInvocation(context.Background(), sharedEventsDeviceSubscriptionRequest(t, root))
	if err != nil {
		t.Fatalf("BuildDeviceSubscriptionInvocation(shared fixture): %v", err)
	}
	if deviceSubscription.Metadata()["system_ability"] != "events.device.subscribe" {
		t.Fatalf("unexpected device subscription invocation: %#v", deviceSubscription)
	}
	if args := deviceSubscription.JSONArgs().(map[string]any); args["resume_cursor"] != "device:2" {
		t.Fatalf("unexpected device subscription args: %#v", args)
	}

	invocationSubscription, err := events.BuildInvocationSubscriptionInvocation(context.Background(), sharedEventsInvocationSubscriptionRequest(t, root))
	if err != nil {
		t.Fatalf("BuildInvocationSubscriptionInvocation(shared fixture): %v", err)
	}
	if invocationSubscription.Metadata()["system_ability"] != "events.invocation.subscribe" {
		t.Fatalf("unexpected invocation subscription invocation: %#v", invocationSubscription)
	}
	if args := invocationSubscription.JSONArgs().(map[string]any); args["invocation_id"] != "inv-1" {
		t.Fatalf("unexpected invocation subscription args: %#v", args)
	}

	page, err := events.ListDeviceEvents(context.Background(), sharedEventsDeviceEventListRequest(t, root))
	if err != nil {
		t.Fatalf("ListDeviceEvents(shared fixture): %v", err)
	}
	if page.Stream != "device" || len(page.Items) != 1 ||
		page.Items[0].Cursor.Token != "device:8" ||
		page.Items[0].Metadata["source"] != "daemon_device_event" {
		t.Fatalf("unexpected device event page: %#v", page)
	}
}

func TestGoEventsFacadeExecutesSharedSessionStreamConformanceCase(t *testing.T) {
	root := repositoryRoot(t)
	eventsCase := sharedCase(t, root, "events-session-stream.yaml")
	requireCaseID(t, eventsCase, "events/session_stream")
	requireCaseAction(t, eventsCase, "build_session_subscription_invocation")
	requireCaseFixture(t, eventsCase, "events-session-subscription-request.v4.json")
	requireCaseExpectation(t, eventsCase, "subscription_invocation_fixture: events-session-subscription-invocation.v4.json")
	requireCaseExpectation(t, eventsCase, "stream_system_ability: session.attach")
	requireCaseExpectation(t, eventsCase, "explicit_session_id_required: true")
	requireCaseExpectation(t, eventsCase, "product_session_ura_parsing_allowed: false")
	requireCaseExpectation(t, eventsCase, "resume_cursor_sequence_maps_to_since_seq: true")
	requireCaseExpectation(t, eventsCase, "other_event_streams: provider_backed")

	events, err := NewEventClient(&sharedEventsTransport{
		t:                                  t,
		expectedSessionSubscriptionRequest: sharedEventsSessionSubscriptionRequestJSON(t, root),
		sessionSubscriptionInvocationJSON:  sharedFixture(t, root, "events-session-subscription-invocation.v4.json"),
	})
	if err != nil {
		t.Fatalf("NewEventClient: %v", err)
	}

	subscription, err := events.BuildSessionSubscriptionInvocation(context.Background(), sharedEventsSessionSubscriptionRequest(t, root))
	if err != nil {
		t.Fatalf("BuildSessionSubscriptionInvocation(shared fixture): %v", err)
	}
	args, ok := subscription.JSONArgs().(map[string]any)
	if !ok {
		t.Fatalf("session subscription args are not an object: %#v", subscription.JSONArgs())
	}
	if subscription.DescriptorRef() != "easynet:///r/example/ability/device.dev-a.session.attach@1.0.0" ||
		subscription.Metadata()["system_ability"] != "session.attach" ||
		args["session_id"] != "run-1" ||
		args["since_seq"].(float64) != 4 {
		t.Fatalf("unexpected shared events session subscription invocation: %#v", subscription)
	}

	productSession := sharedEventsSessionSubscriptionRequest(t, root)
	productSession.SessionID = ""
	productSession.SessionURA = "easynet:///r/example/resource/daemon.browser/run-1"
	if _, err := events.BuildSessionSubscriptionInvocation(context.Background(), productSession); !IsCode(err, ErrInvalidArgument) {
		t.Fatalf("product session_ura parsing did not produce InvalidArgument: %v", err)
	}
}

func TestGoSurfaceFacadeExecutesSharedPageCarrierConformanceCase(t *testing.T) {
	root := repositoryRoot(t)
	surfaceCase := sharedCase(t, root, "surface-page-carriers.yaml")
	requireCaseID(t, surfaceCase, "surface/page_carriers")
	for _, action := range []string{
		"build_surface_list_pages_invocation",
		"build_surface_create_page_invocation",
		"build_surface_delete_page_invocation",
		"build_surface_manifest_invocation",
		"build_surface_health_invocation",
		"project_surface_page_page",
		"project_surface_manifest",
		"project_surface_health",
		"project_surface_status",
	} {
		requireCaseAction(t, surfaceCase, action)
	}
	for _, fixture := range []string{
		"surface-list-pages-request.v4.json",
		"surface-create-page-request.v4.json",
		"surface-delete-page-request.v4.json",
		"surface-manifest-request.v4.json",
		"surface-health-request.v4.json",
		"surface-page-page.v4.json",
		"surface-manifest.v4.json",
		"surface-health.v4.json",
	} {
		requireCaseFixture(t, surfaceCase, fixture)
	}
	for _, ability := range []string{"pages.list", "pages.publish", "pages.get", "pages.unpublish", "pages.health"} {
		requireCaseLiteral(t, surfaceCase, "- "+ability)
	}
	requireCaseExpectation(t, surfaceCase, "health_invocation_fixture: surface-health-invocation.v4.json")
	requireCaseExpectation(t, surfaceCase, "health_fixture: surface-health.v4.json")
	requireCaseExpectation(t, surfaceCase, "surface_status_aliases_health: true")
	requireCaseExpectation(t, surfaceCase, "health_rendering_owner: backend")
	requireCaseExpectation(t, surfaceCase, "backend_rendering_owned_by_sdk: false")
	requireCaseExpectation(t, surfaceCase, "direct_filesystem_page_transport: false")

	surface, err := NewSurfaceClient(&sharedSurfaceTransport{
		t:                       t,
		expectedListRequest:     sharedFixture(t, root, "surface-list-pages-request.v4.json"),
		expectedCreateRequest:   sharedFixture(t, root, "surface-create-page-request.v4.json"),
		expectedDeleteRequest:   sharedFixture(t, root, "surface-delete-page-request.v4.json"),
		expectedManifestRequest: sharedFixture(t, root, "surface-manifest-request.v4.json"),
		expectedHealthRequest:   sharedFixture(t, root, "surface-health-request.v4.json"),
		listInvocationJSON:      sharedFixture(t, root, "surface-list-pages-invocation.v4.json"),
		createInvocationJSON:    sharedFixture(t, root, "surface-create-page-invocation.v4.json"),
		deleteInvocationJSON:    sharedFixture(t, root, "surface-delete-page-invocation.v4.json"),
		manifestInvocationJSON:  sharedFixture(t, root, "surface-manifest-invocation.v4.json"),
		healthInvocationJSON:    sharedFixture(t, root, "surface-health-invocation.v4.json"),
		pagePageJSON:            sharedFixture(t, root, "surface-page-page.v4.json"),
		manifestJSON:            sharedFixture(t, root, "surface-manifest.v4.json"),
		healthJSON:              sharedFixture(t, root, "surface-health.v4.json"),
	})
	if err != nil {
		t.Fatalf("NewSurfaceClient: %v", err)
	}

	list, err := surface.BuildListPagesInvocation(context.Background(), sharedSurfaceListPagesRequest(t, root))
	if err != nil {
		t.Fatalf("BuildListPagesInvocation(shared fixture): %v", err)
	}
	if list.DescriptorRef() != "easynet:///r/example/ability/alice.pages.pages.list@1.0.0" ||
		list.Metadata()["system_ability"] != "pages.list" {
		t.Fatalf("unexpected shared surface list invocation: %#v", list)
	}

	create, err := surface.BuildCreatePageInvocation(context.Background(), sharedSurfaceCreatePageRequest(t, root))
	if err != nil {
		t.Fatalf("BuildCreatePageInvocation(shared fixture): %v", err)
	}
	if create.DescriptorRef() != "easynet:///r/example/ability/alice.pages.pages.publish@1.0.0" ||
		create.Metadata()["system_ability"] != "pages.publish" {
		t.Fatalf("unexpected shared surface create invocation: %#v", create)
	}

	del, err := surface.BuildDeletePageInvocation(context.Background(), sharedSurfaceDeletePageRequest(t, root))
	if err != nil {
		t.Fatalf("BuildDeletePageInvocation(shared fixture): %v", err)
	}
	if del.DescriptorRef() != "easynet:///r/example/ability/alice.pages.pages.unpublish@1.0.0" ||
		del.Metadata()["system_ability"] != "pages.unpublish" {
		t.Fatalf("unexpected shared surface delete invocation: %#v", del)
	}

	manifestDraft, err := surface.BuildManifestInvocation(context.Background(), sharedSurfaceManifestRequest(t, root))
	if err != nil {
		t.Fatalf("BuildManifestInvocation(shared fixture): %v", err)
	}
	if manifestDraft.DescriptorRef() != "easynet:///r/example/ability/alice.pages.pages.get@1.0.0" ||
		manifestDraft.Metadata()["system_ability"] != "pages.get" {
		t.Fatalf("unexpected shared surface manifest invocation: %#v", manifestDraft)
	}

	healthDraft, err := surface.BuildHealthInvocation(context.Background(), sharedSurfaceHealthRequest(t, root))
	if err != nil {
		t.Fatalf("BuildHealthInvocation(shared fixture): %v", err)
	}
	if healthDraft.DescriptorRef() != "easynet:///r/example/ability/alice.pages.pages.health@1.0.0" ||
		healthDraft.Metadata()["system_ability"] != "pages.health" {
		t.Fatalf("unexpected shared surface health invocation: %#v", healthDraft)
	}

	page, err := surface.ListPages(context.Background(), sharedSurfaceListPagesRequest(t, root))
	if err != nil {
		t.Fatalf("ListPages(shared fixture): %v", err)
	}
	if page.Kind != "surface_page_page" || page.Source != "pages_read_model" ||
		len(page.Items) != 1 || page.Items[0].SurfaceRef != "easynet:///r/example/resource/alice.docs" {
		t.Fatalf("unexpected shared surface page page: %#v", page)
	}

	manifest, err := surface.SurfaceManifest(context.Background(), sharedSurfaceManifestRequest(t, root))
	if err != nil {
		t.Fatalf("SurfaceManifest(shared fixture): %v", err)
	}
	if manifest.Kind != "surface_manifest" || manifest.Page.PageID != "docs" ||
		manifest.Entrypoint["kind"] != "public_page_ref" {
		t.Fatalf("unexpected shared surface manifest: %#v", manifest)
	}

	health, err := surface.SurfaceHealth(context.Background(), sharedSurfaceHealthRequest(t, root))
	if err != nil {
		t.Fatalf("SurfaceHealth(shared fixture): %v", err)
	}
	if !health.Ready || health.Metadata["rendering_owner"] != "backend" ||
		health.DescriptorRef != "easynet:///r/example/ability/alice.pages.pages.health@1.0.0" {
		t.Fatalf("unexpected shared surface health: %#v", health)
	}

	status, err := surface.SurfaceStatus(context.Background(), SurfaceStatusRequest(sharedSurfaceHealthRequest(t, root)))
	if err != nil {
		t.Fatalf("SurfaceStatus(shared fixture): %v", err)
	}
	if status.SurfaceRef != health.SurfaceRef || status.State != health.State {
		t.Fatalf("surface status did not preserve health projection: status=%#v health=%#v", status, health)
	}

	incomplete := sharedSurfaceCreatePageRequest(t, root)
	incomplete.CallerURA = ""
	if _, err := surface.BuildCreatePageInvocation(context.Background(), incomplete); !IsCode(err, ErrInvalidArgument) {
		t.Fatalf("incomplete surface carrier did not produce InvalidArgument: %v", err)
	}
	relativeFolder := sharedSurfaceCreatePageRequest(t, root)
	relativeFolder.Folder = "tmp/easynet-pages-docs"
	if _, err := surface.BuildCreatePageInvocation(context.Background(), relativeFolder); !IsCode(err, ErrInvalidArgument) {
		t.Fatalf("relative surface folder did not produce InvalidArgument: %v", err)
	}
	if _, err := NewSurfacePagePageFromJSON(sharedSurfacePagePageWithOversizedLimit(t, root)); !IsCode(err, ErrInvalidArgument) {
		t.Fatalf("oversized surface page projection did not produce InvalidArgument: %v", err)
	}
	if _, err := NewSurfaceManifestFromJSON(sharedSurfaceManifestWithoutEntrypoint(t, root)); !IsCode(err, ErrInvalidArgument) {
		t.Fatalf("surface manifest without entrypoint did not produce InvalidArgument: %v", err)
	}
}

func TestGoCompatibilityFacadeExecutesSharedOpenAICarrierConformanceCase(t *testing.T) {
	root := repositoryRoot(t)
	compatibilityCase := sharedCase(t, root, "compatibility-openai-carrier-projection.yaml")
	requireCaseID(t, compatibilityCase, "compatibility/openai_carrier_projection")
	for _, action := range []string{
		"build_list_models_invocation",
		"build_chat_completion_invocation",
		"build_stream_chat_completion_invocation",
		"project_model_page",
		"project_chat_completion",
		"project_chat_stream",
		"project_file_upload",
		"project_file",
		"project_file_delete_result",
	} {
		requireCaseAction(t, compatibilityCase, action)
	}
	for _, fixture := range []string{
		"compatibility-list-models-request.v4.json",
		"compatibility-list-models-invocation.v4.json",
		"compatibility-chat-completion-request.v4.json",
		"compatibility-chat-completion-invocation.v4.json",
		"compatibility-stream-chat-completion-request.v4.json",
		"compatibility-stream-chat-completion-invocation.v4.json",
		"compatibility-model-page.v4.json",
		"compatibility-chat-completion.v4.json",
		"compatibility-chat-stream.v4.json",
		"compatibility-file-upload-request.v4.json",
		"compatibility-file-request.v4.json",
		"compatibility-file.v4.json",
		"compatibility-file-delete-request.v4.json",
		"compatibility-file-delete-result.v4.json",
	} {
		requireCaseFixture(t, compatibilityCase, fixture)
	}
	requireCaseExpectation(t, compatibilityCase, "rejects_provider_nickname_models: true")
	requireCaseExpectation(t, compatibilityCase, "rejects_unary_stream_true: true")
	requireCaseExpectation(t, compatibilityCase, "files_api: file_wrapper_projection")
	requireCaseExpectation(t, compatibilityCase, "openai_files_daemon_ability_required: false")
	requireCaseExpectation(t, compatibilityCase, "product_http_auth_and_sse_fanout: product_owned")

	compatibility, err := NewCompatibilityClient(&sharedCompatibilityTransport{
		t:                   t,
		expectedListRequest: sharedFixture(t, root, "compatibility-list-models-request.v4.json"),
		expectedChatRequest: sharedFixture(t, root, "compatibility-chat-completion-request.v4.json"),
		expectedStreamRequest: sharedFixture(t, root,
			"compatibility-stream-chat-completion-request.v4.json"),
		listInvocationJSON:   sharedFixture(t, root, "compatibility-list-models-invocation.v4.json"),
		chatInvocationJSON:   sharedFixture(t, root, "compatibility-chat-completion-invocation.v4.json"),
		streamInvocationJSON: sharedFixture(t, root, "compatibility-stream-chat-completion-invocation.v4.json"),
		modelPageJSON:        sharedFixture(t, root, "compatibility-model-page.v4.json"),
		chatCompletionJSON:   sharedFixture(t, root, "compatibility-chat-completion.v4.json"),
		chatStreamJSON:       sharedFixture(t, root, "compatibility-chat-stream.v4.json"),
	})
	if err != nil {
		t.Fatalf("NewCompatibilityClient: %v", err)
	}

	listDraft, err := compatibility.BuildListModelsInvocation(context.Background(), sharedCompatibilityListModelsRequest(t, root))
	if err != nil {
		t.Fatalf("BuildListModelsInvocation(shared fixture): %v", err)
	}
	if listDraft.DescriptorRef() != "easynet:///r/example/ability/device.dev-a.openai.list_models@1.0.0" ||
		listDraft.Metadata()["system_ability"] != "openai.list_models" {
		t.Fatalf("unexpected shared compatibility list invocation: %#v", listDraft)
	}

	chatDraft, err := compatibility.BuildChatCompletionInvocation(context.Background(), sharedCompatibilityChatCompletionRequest(t, root))
	if err != nil {
		t.Fatalf("BuildChatCompletionInvocation(shared fixture): %v", err)
	}
	if chatDraft.DescriptorRef() != "easynet:///r/example/ability/device.dev-a.openai.chat_completions@1.0.0" ||
		chatDraft.Metadata()["system_ability"] != "openai.chat_completions" {
		t.Fatalf("unexpected shared compatibility chat invocation: %#v", chatDraft)
	}

	streamDraft, err := compatibility.BuildStreamChatCompletionInvocation(context.Background(), sharedCompatibilityStreamChatCompletionRequest(t, root))
	if err != nil {
		t.Fatalf("BuildStreamChatCompletionInvocation(shared fixture): %v", err)
	}
	if streamDraft.DescriptorRef() != "easynet:///r/example/ability/device.dev-a.openai.chat_completions@1.0.0" ||
		streamDraft.Metadata()["system_ability"] != "openai.chat_completions" {
		t.Fatalf("unexpected shared compatibility stream invocation: %#v", streamDraft)
	}

	models, err := compatibility.ListModels(context.Background(), sharedCompatibilityListModelsRequest(t, root))
	if err != nil {
		t.Fatalf("ListModels(shared fixture): %v", err)
	}
	if models.Kind != "model_page" || len(models.Data) != 1 ||
		models.Data[0].AbilityRef != "easynet:///r/example/ability/alice.codex.chat" {
		t.Fatalf("unexpected shared compatibility model page: %#v", models)
	}

	chat, err := compatibility.ChatCompletions(context.Background(), sharedCompatibilityChatCompletionRequest(t, root))
	if err != nil {
		t.Fatalf("ChatCompletions(shared fixture): %v", err)
	}
	if chat.Kind != "chat_completion" || chat.Model != "easynet:///r/example/ability/alice.codex.chat" ||
		len(chat.Choices) != 1 {
		t.Fatalf("unexpected shared compatibility chat completion: %#v", chat)
	}

	stream, err := compatibility.StreamChatCompletions(context.Background(), sharedCompatibilityStreamChatCompletionRequest(t, root))
	if err != nil {
		t.Fatalf("StreamChatCompletions(shared fixture): %v", err)
	}
	if stream.Kind != "chat_completion_stream" || !stream.Stream || stream.DoneSentinel != "[DONE]" ||
		len(stream.Items) != 1 {
		t.Fatalf("unexpected shared compatibility chat stream: %#v", stream)
	}

	uploaded, err := compatibility.ProjectFileUpload(sharedCompatibilityFileUploadRequest(t, root))
	if err != nil {
		t.Fatalf("ProjectFileUpload(shared fixture): %v", err)
	}
	uploadedJSON, err := json.Marshal(uploaded)
	if err != nil {
		t.Fatalf("encode shared compatibility uploaded file: %v", err)
	}
	assertJSONEquivalent(t, uploadedJSON, sharedFixture(t, root, "compatibility-file.v4.json"))

	file, err := compatibility.ProjectFile(sharedCompatibilityFileRequest(t, root))
	if err != nil {
		t.Fatalf("ProjectFile(shared fixture): %v", err)
	}
	fileJSON, err := json.Marshal(file)
	if err != nil {
		t.Fatalf("encode shared compatibility file: %v", err)
	}
	assertJSONEquivalent(t, fileJSON, sharedFixture(t, root, "compatibility-file.v4.json"))

	deleted, err := compatibility.ProjectFileDeleteResult(sharedCompatibilityFileDeleteRequest(t, root))
	if err != nil {
		t.Fatalf("ProjectFileDeleteResult(shared fixture): %v", err)
	}
	deletedJSON, err := json.Marshal(deleted)
	if err != nil {
		t.Fatalf("encode shared compatibility file delete result: %v", err)
	}
	assertJSONEquivalent(t, deletedJSON, sharedFixture(t, root, "compatibility-file-delete-result.v4.json"))

	nicknameModel := sharedCompatibilityChatCompletionRequest(t, root)
	nicknameModel.Request["model"] = "gpt-4o-mini"
	if _, err := compatibility.BuildChatCompletionInvocation(context.Background(), nicknameModel); !IsCode(err, ErrInvalidArgument) {
		t.Fatalf("provider nickname model did not produce InvalidArgument: %v", err)
	}
	unaryStream := sharedCompatibilityChatCompletionRequest(t, root)
	unaryStream.Request["stream"] = true
	if _, err := compatibility.BuildChatCompletionInvocation(context.Background(), unaryStream); !IsCode(err, ErrInvalidArgument) {
		t.Fatalf("unary stream=true did not produce InvalidArgument: %v", err)
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

func TestGoMEMCExecutesSharedProfileExclusivityConformanceCase(t *testing.T) {
	root := repositoryRoot(t)
	memcCase := sharedCase(t, root, "memc-profile-exclusivity.yaml")
	requireCaseID(t, memcCase, "memc/profile_exclusivity")
	requireCaseAction(t, memcCase, "inspect_public_api")
	requireCaseExpectation(t, memcCase, "duplicate_profile_owners: 0")

	audits := []sharedProfileOwnershipAudit{
		{
			Owner: "runtime_core",
			Type:  reflect.TypeOf((*Client)(nil)),
			Operations: map[string]string{
				"Close":            "runtime_core.client.close",
				"FeatureDiscovery": "runtime_core.feature_discovery",
				"RequireABI":       "runtime_core.require_abi",
			},
		},
		{
			Owner: "runtime_core",
			Type:  reflect.TypeOf((*DaemonControl)(nil)),
			Operations: map[string]string{
				"Attach":       "runtime_core.daemon.attach",
				"ConnectLocal": "runtime_core.daemon.connect_local",
				"Discover":     "runtime_core.daemon.discover",
				"Start":        "runtime_core.daemon.start",
			},
		},
		{
			Owner: "runtime_core",
			Type:  reflect.TypeOf((*DaemonHandle)(nil)),
			Operations: map[string]string{
				"Detach":      "runtime_core.daemon_handle.detach",
				"Endpoints":   "runtime_core.daemon_handle.endpoints",
				"HandleID":    "runtime_core.daemon_handle.id",
				"OpenRuntime": "runtime_core.daemon_handle.open_runtime",
				"State":       "runtime_core.daemon_handle.state",
				"Status":      "runtime_core.daemon_handle.status",
				"Stop":        "runtime_core.daemon_handle.stop",
			},
		},
		{
			Owner: "runtime_core",
			Type:  reflect.TypeOf((*RuntimeClient)(nil)),
			Operations: map[string]string{
				"Await":          "runtime_core.invocation.await",
				"Cancel":         "runtime_core.invocation.cancel",
				"Close":          "runtime_core.runtime_client.close",
				"CloseHandle":    "runtime_core.invocation.close_handle",
				"Events":         "runtime_core.invocation.events",
				"Invoke":         "runtime_core.invocation.invoke",
				"InvokeStream":   "runtime_core.invocation.invoke_stream",
				"OpenBidi":       "runtime_core.invocation.open_bidi",
				"Prepare":        "runtime_core.invocation.prepare",
				"PrepareBuilder": "runtime_core.invocation.prepare_builder",
				"SubmitSigned":   "runtime_core.invocation.submit_signed",
			},
		},
		{
			Owner: "runtime_core",
			Type:  reflect.TypeOf((*HealthClient)(nil)),
			Operations: map[string]string{
				"Diagnostics":   "runtime_core.health.diagnostics",
				"RuntimeHealth": "runtime_core.health.runtime_health",
			},
		},
		{
			Owner: "directory_identity",
			Type:  reflect.TypeOf((*DirectoryClient)(nil)),
			Operations: map[string]string{
				"BuildDirectorySubscriptionInvocation": "directory_identity.directory.build_subscription_invocation",
				"Close":                                "directory_identity.directory.close",
				"ListAbilities":                        "directory_identity.directory.list_abilities",
				"ListAgents":                           "directory_identity.directory.list_agents",
				"ListDevices":                          "directory_identity.directory.list_devices",
				"ListPeerUserDevices":                  "directory_identity.directory.list_peer_user_devices",
				"Resolve":                              "directory_identity.directory.resolve",
				"SubscribeDirectory":                   "directory_identity.directory.subscribe",
			},
		},
		{
			Owner: "directory_identity",
			Type:  reflect.TypeOf((*IdentityClient)(nil)),
			Operations: map[string]string{
				"AbilityURAFromDescriptorRef":   "directory_identity.identity.ability_ura_from_descriptor_ref",
				"BuildDescriptorRef":            "directory_identity.identity.build_descriptor_ref",
				"BuildResourceRef":              "directory_identity.identity.build_resource_ref",
				"BuildURA":                      "directory_identity.identity.build_ura",
				"CanonicalAbilityDescriptorRef": "directory_identity.identity.canonical_ability_descriptor_ref",
				"Close":                         "directory_identity.identity.close",
				"ListSigningKeys":               "directory_identity.identity.list_signing_keys",
				"OwnerAbilityDescriptorRef":     "directory_identity.identity.owner_ability_descriptor_ref",
				"OwnerAbilityURA":               "directory_identity.identity.owner_ability_ura",
				"OwnerURAForAbility":            "directory_identity.identity.owner_ura_for_ability",
				"ProjectDescriptorRef":          "directory_identity.identity.project_descriptor_ref",
				"ProjectIdentity":               "directory_identity.identity.project_identity",
				"RegisterSigningKey":            "directory_identity.identity.register_signing_key",
				"RevokeSigningKey":              "directory_identity.identity.revoke_signing_key",
				"Signer":                        "directory_identity.identity.signer",
			},
		},
		{
			Owner: "receipt",
			Type:  reflect.TypeOf((*ReceiptClient)(nil)),
			Operations: map[string]string{
				"BuildFetchInvocation":       "receipt.build_fetch_invocation",
				"BuildGetHistoryInvocation":  "receipt.history.build_get_invocation",
				"BuildListHistoryInvocation": "receipt.history.build_list_invocation",
				"BuildTraceInvocation":       "receipt.trace.build_get_invocation",
				"CausalRef":                  "receipt.causal_ref",
				"Close":                      "receipt.close",
				"Fetch":                      "receipt.fetch",
				"GetHistory":                 "receipt.history.get",
				"GetTrace":                   "receipt.trace.get",
				"ListHistory":                "receipt.history.list",
				"Project":                    "receipt.project",
				"Verify":                     "receipt.verify",
				"VerifyChain":                "receipt.verify_chain",
			},
		},
		{
			Owner: "publication",
			Type:  reflect.TypeOf((*PublicationClient)(nil)),
			Operations: map[string]string{
				"BuildDeployInvocation":         "publication.build_deploy_invocation",
				"BuildLocalResourceRef":         "publication.build_local_resource_ref",
				"BuildUnpublishInvocation":      "publication.build_unpublish_invocation",
				"Close":                         "publication.close",
				"DeployAbility":                 "publication.deploy_ability",
				"DisableAbilityImpl":            "publication.disable_ability_impl",
				"DisableAbilityImplWithRequest": "publication.disable_ability_impl_with_request",
				"EnableAbilityImpl":             "publication.enable_ability_impl",
				"EnableAbilityImplWithRequest":  "publication.enable_ability_impl_with_request",
				"InstallPlugin":                 "publication.install_plugin",
				"ListAbilities":                 "publication.list_abilities",
				"ShowAbility":                   "publication.show_ability",
				"ShowAbilityWithRequest":        "publication.show_ability_with_request",
				"UnpublishAbility":              "publication.unpublish_ability",
				"UnpublishAbilityWithRequest":   "publication.unpublish_ability_with_request",
				"ValidatePackage":               "publication.validate_package",
			},
		},
		{
			Owner: "host_binding",
			Type:  reflect.TypeOf((*HostBindingClient)(nil)),
			Operations: map[string]string{
				"BuildHostStreamBinding": "host_binding.build_host_stream_binding",
				"CheckReadiness":         "host_binding.check_readiness",
				"Close":                  "host_binding.close",
				"Cleanup":                "host_binding.cleanup",
				"DecodeRequest":          "host_binding.decode_request",
				"EncodeError":            "host_binding.encode_error",
				"EncodeItem":             "host_binding.encode_item",
				"EncodeTerminal":         "host_binding.encode_terminal",
				"FoldOutputHash":         "host_binding.fold_output_hash",
				"OpenLifecycle":          "host_binding.open_lifecycle",
			},
		},
		{
			Owner: "mission",
			Type:  reflect.TypeOf((*MissionClient)(nil)),
			Operations: map[string]string{
				"BuildCancelInvocation":  "mission.build_cancel_invocation",
				"BuildRunEALInvocation":  "mission.build_run_eal_invocation",
				"BuildRunFileInvocation": "mission.build_run_file_invocation",
				"BuildTrackInvocation":   "mission.build_track_invocation",
				"Cancel":                 "mission.cancel",
				"Close":                  "mission.close",
				"Events":                 "mission.events",
				"RunEAL":                 "mission.run_eal",
				"RunFile":                "mission.run_file",
				"TailEvents":             "mission.tail_events",
				"Track":                  "mission.track",
			},
		},
		{
			Owner: "admin_gateway",
			Type:  reflect.TypeOf((*AdminClient)(nil)),
			Operations: map[string]string{
				"AgentRefresh":                "admin_gateway.agent.refresh",
				"AgentStart":                  "admin_gateway.agent.start",
				"AgentStop":                   "admin_gateway.agent.stop",
				"BuildAgentListInvocation":    "admin_gateway.agent.build_list_invocation",
				"BuildAgentRefreshInvocation": "admin_gateway.agent.build_refresh_invocation",
				"BuildAgentStartInvocation":   "admin_gateway.agent.build_start_invocation",
				"BuildAgentStopInvocation":    "admin_gateway.agent.build_stop_invocation",
				"BuildRevokeDeviceInvocation": "admin_gateway.device.build_revoke_invocation",
				"BuildSessionListInvocation":  "admin_gateway.session.build_list_invocation",
				"Close":                       "admin_gateway.close",
				"CreateDeviceSession":         "admin_gateway.session.create",
				"CreatePairing":               "admin_gateway.pairing.create",
				"DeleteDeviceSession":         "admin_gateway.session.delete",
				"GatewayStatus":               "admin_gateway.gateway.status",
				"JoinHub":                     "admin_gateway.hub.join",
				"LeaveHub":                    "admin_gateway.hub.leave",
				"ListAgents":                  "admin_gateway.agent.list",
				"ListDeviceSessions":          "admin_gateway.session.list",
				"PairingPreflight":            "admin_gateway.pairing.preflight",
				"RevokeDevice":                "admin_gateway.device.revoke",
				"ValidatePairing":             "admin_gateway.pairing.validate",
				"VerifyDeviceCredential":      "admin_gateway.device.verify_credential",
			},
		},
		{
			Owner: "events",
			Type:  reflect.TypeOf((*EventClient)(nil)),
			Operations: map[string]string{
				"BuildDeviceSubscriptionInvocation":     "events.build_device_subscription_invocation",
				"BuildDirectorySubscriptionInvocation":  "events.build_directory_subscription_invocation",
				"BuildInvocationSubscriptionInvocation": "events.build_invocation_subscription_invocation",
				"BuildSessionSubscriptionInvocation":    "events.build_session_subscription_invocation",
				"Close":                                 "events.close",
				"ListDeviceEvents":                      "events.list_device_events",
				"ProjectDirectoryEvent":                 "events.project_directory_event",
				"ProjectDropReport":                     "events.project_drop_report",
				"ProjectTerminal":                       "events.project_terminal",
				"SubscribeDevices":                      "events.subscribe_devices",
				"SubscribeDirectory":                    "events.subscribe_directory",
				"SubscribeInvocations":                  "events.subscribe_invocations",
				"SubscribeSessions":                     "events.subscribe_sessions",
			},
		},
		{
			Owner: "surface",
			Type:  reflect.TypeOf((*SurfaceClient)(nil)),
			Operations: map[string]string{
				"BuildCreatePageInvocation": "surface.build_create_page_invocation",
				"BuildDeletePageInvocation": "surface.build_delete_page_invocation",
				"BuildHealthInvocation":     "surface.build_health_invocation",
				"BuildListPagesInvocation":  "surface.build_list_pages_invocation",
				"BuildManifestInvocation":   "surface.build_manifest_invocation",
				"Close":                     "surface.close",
				"CreatePage":                "surface.create_page",
				"DeletePage":                "surface.delete_page",
				"ListPages":                 "surface.list_pages",
				"PublicPageRef":             "surface.public_page_ref",
				"SurfaceHealth":             "surface.health",
				"SurfaceManifest":           "surface.manifest",
				"SurfaceStatus":             "surface.status",
			},
		},
		{
			Owner: "compatibility",
			Type:  reflect.TypeOf((*CompatibilityClient)(nil)),
			Operations: map[string]string{
				"BuildChatCompletionInvocation":       "compatibility.chat.build_completion_invocation",
				"BuildFileDeleteInvocation":           "compatibility.file.build_delete_invocation",
				"BuildFileGetInvocation":              "compatibility.file.retrieve",
				"BuildFileRetrieveInvocation":         "compatibility.file.retrieve",
				"BuildFileUploadInvocation":           "compatibility.file.build_upload_invocation",
				"BuildListModelsInvocation":           "compatibility.models.build_list_invocation",
				"BuildStreamChatCompletionInvocation": "compatibility.chat.build_stream_invocation",
				"ChatCompletions":                     "compatibility.chat.completions",
				"Close":                               "compatibility.close",
				"CreateChatCompletion":                "compatibility.chat.create_completion",
				"DeleteFile":                          "compatibility.file.delete",
				"GetFile":                             "compatibility.file.retrieve",
				"ListModels":                          "compatibility.models.list",
				"ProjectFile":                         "compatibility.file.project",
				"ProjectFileDeleteResult":             "compatibility.file.project_delete_result",
				"ProjectFileUpload":                   "compatibility.file.project_upload",
				"RetrieveFile":                        "compatibility.file.retrieve",
				"StreamChatCompletions":               "compatibility.chat.stream_completions",
				"StreamChatCompletion":                "compatibility.chat.stream_completion",
				"UploadFile":                          "compatibility.file.upload",
			},
		},
		{
			Owner: "compatibility",
			Type:  reflect.TypeOf((*CompatibilityRuntimeTransport)(nil)),
			Operations: map[string]string{
				"BuildChatCompletionInvocation":       "compatibility.chat.build_completion_invocation",
				"BuildFileDeleteInvocation":           "compatibility.file.build_delete_invocation",
				"BuildFileRetrieveInvocation":         "compatibility.file.retrieve",
				"BuildFileUploadInvocation":           "compatibility.file.build_upload_invocation",
				"BuildListModelsInvocation":           "compatibility.models.build_list_invocation",
				"BuildStreamChatCompletionInvocation": "compatibility.chat.build_stream_invocation",
				"Close":                               "compatibility.close",
				"CreateChatCompletion":                "compatibility.chat.create_completion",
				"DeleteFile":                          "compatibility.file.delete",
				"ListModels":                          "compatibility.models.list",
				"RetrieveFile":                        "compatibility.file.retrieve",
				"StreamChatCompletion":                "compatibility.chat.stream_completion",
				"UploadFile":                          "compatibility.file.upload",
			},
		},
		{
			Owner: "wrappers",
			Type:  reflect.TypeOf((*WrapperClient)(nil)),
			Operations: map[string]string{
				"BuildBrowserSessionInvocation":       "wrappers.browser.build_session_invocation",
				"BuildFileTransferInvocation":         "wrappers.file.build_transfer_invocation",
				"BuildMediaSessionInvocation":         "wrappers.media.build_session_invocation",
				"BuildRemoteDesktopSessionInvocation": "wrappers.remote_desktop.build_session_invocation",
				"BuildTerminalSessionInvocation":      "wrappers.terminal.build_session_invocation",
				"Close":                               "wrappers.close",
				"ProjectBrowserSession":               "wrappers.browser.project_session",
				"ProjectFileRecord":                   "wrappers.file.project_record",
				"ProjectMediaSession":                 "wrappers.media.project_session",
				"ProjectRemoteDesktopSession":         "wrappers.remote_desktop.project_session",
				"ProjectTerminalSession":              "wrappers.terminal.project_session",
				"StartBrowserSession":                 "wrappers.browser.start_session",
				"StartMediaSession":                   "wrappers.media.start_session",
				"StartRemoteDesktopSession":           "wrappers.remote_desktop.start_session",
				"StartTerminalSession":                "wrappers.terminal.start_session",
				"TransferFile":                        "wrappers.file.transfer",
			},
		},
	}

	unmapped, duplicateOwners := auditSharedProfileOwnership(audits)
	if len(unmapped) > 0 {
		t.Fatalf("public SDK methods without MEMC owner:\n%s", strings.Join(unmapped, "\n"))
	}
	if len(duplicateOwners) > 0 {
		t.Fatalf("public SDK operations with duplicate MEMC owners:\n%s", strings.Join(duplicateOwners, "\n"))
	}
}

type sharedProfileOwnershipAudit struct {
	Owner      string
	Type       reflect.Type
	Operations map[string]string
}

func auditSharedProfileOwnership(audits []sharedProfileOwnershipAudit) ([]string, []string) {
	var unmapped []string
	operationOwners := make(map[string]map[string]struct{})
	for _, audit := range audits {
		for i := 0; i < audit.Type.NumMethod(); i++ {
			method := audit.Type.Method(i)
			operation, ok := audit.Operations[method.Name]
			if !ok {
				unmapped = append(unmapped, fmt.Sprintf("%s.%s", audit.Type.Elem().Name(), method.Name))
				continue
			}
			if operationOwners[operation] == nil {
				operationOwners[operation] = make(map[string]struct{})
			}
			operationOwners[operation][audit.Owner] = struct{}{}
		}
	}
	sort.Strings(unmapped)

	var duplicateOwners []string
	for operation, owners := range operationOwners {
		if len(owners) < 2 {
			continue
		}
		ownerNames := make([]string, 0, len(owners))
		for owner := range owners {
			ownerNames = append(ownerNames, owner)
		}
		sort.Strings(ownerNames)
		duplicateOwners = append(duplicateOwners, fmt.Sprintf("%s owned by %s", operation, strings.Join(ownerNames, ", ")))
	}
	sort.Strings(duplicateOwners)
	return unmapped, duplicateOwners
}

func TestGoMEMCExecutesSharedConsumerCoverageConformanceCase(t *testing.T) {
	root := repositoryRoot(t)
	coverageCase := sharedCase(t, root, "memc-consumer-coverage.yaml")
	requireCaseID(t, coverageCase, "memc/consumer_coverage")
	requireCaseAction(t, coverageCase, "inspect_consumer_coverage")
	requireCaseExpectation(t, coverageCase, "raw_lower_layer_dependency: false")
	for _, consumer := range []string{
		"backend_hub",
		"easyremote",
		"cli",
		"desktop_gui",
		"third_party_host_app",
		"future_bindings",
	} {
		requireCaseLiteral(t, coverageCase, "- "+consumer)
	}
	for _, forbidden := range []string{
		"axon_sdk_proto",
		"c_abi_direct",
		"raw_daemon_socket",
		"control_frame_product_call",
		"cli_subprocess",
		"easyremote_dependency",
		"product_local_daemon_transport",
	} {
		requireCaseLiteral(t, coverageCase, "- "+forbidden)
	}

	requirements := []sharedConsumerCoverageRequirement{
		{
			Consumer: "backend_hub",
			Profile:  "runtime_core",
			Surfaces: []sharedConsumerCoverageSurface{
				{Type: reflect.TypeOf((*Client)(nil)), Methods: []string{"RequireABI", "FeatureDiscovery"}},
				{Type: reflect.TypeOf((*RuntimeClient)(nil)), Methods: []string{"Invoke", "InvokeStream", "OpenBidi", "Await", "Cancel"}},
				{Type: reflect.TypeOf((*HealthClient)(nil)), Methods: []string{"RuntimeHealth", "Diagnostics"}},
			},
		},
		{
			Consumer: "backend_hub",
			Profile:  "directory_identity",
			Surfaces: []sharedConsumerCoverageSurface{
				{Type: reflect.TypeOf((*DirectoryClient)(nil)), Methods: []string{"Resolve", "ListDevices", "ListAgents", "ListAbilities"}},
				{Type: reflect.TypeOf((*IdentityClient)(nil)), Methods: []string{"ProjectDescriptorRef", "BuildResourceRef"}},
			},
		},
		{
			Consumer: "backend_hub",
			Profile:  "receipt",
			Surfaces: []sharedConsumerCoverageSurface{
				{Type: reflect.TypeOf((*ReceiptClient)(nil)), Methods: []string{"Fetch", "Project", "Verify", "CausalRef", "ListHistory", "GetHistory", "GetTrace"}},
			},
		},
		{
			Consumer: "backend_hub",
			Profile:  "events",
			Surfaces: []sharedConsumerCoverageSurface{
				{Type: reflect.TypeOf((*EventClient)(nil)), Methods: []string{"SubscribeDirectory", "SubscribeInvocations", "ListDeviceEvents", "ProjectDropReport"}},
			},
		},
		{
			Consumer: "backend_hub",
			Profile:  "admin_gateway",
			Surfaces: []sharedConsumerCoverageSurface{
				{Type: reflect.TypeOf((*AdminClient)(nil)), Methods: []string{"GatewayStatus", "ListAgents", "ListDeviceSessions", "JoinHub", "LeaveHub"}},
			},
		},
		{
			Consumer: "backend_hub",
			Profile:  "surface",
			Surfaces: []sharedConsumerCoverageSurface{
				{Type: reflect.TypeOf((*SurfaceClient)(nil)), Methods: []string{"ListPages", "CreatePage", "DeletePage", "SurfaceManifest", "PublicPageRef", "SurfaceHealth"}},
			},
		},
		{
			Consumer: "backend_hub",
			Profile:  "compatibility",
			Surfaces: []sharedConsumerCoverageSurface{
				{Type: reflect.TypeOf((*CompatibilityClient)(nil)), Methods: []string{"ListModels", "ChatCompletions", "StreamChatCompletions", "UploadFile", "RetrieveFile", "DeleteFile"}},
			},
		},
		{
			Consumer: "backend_hub",
			Profile:  "publication",
			Surfaces: []sharedConsumerCoverageSurface{
				{Type: reflect.TypeOf((*PublicationClient)(nil)), Methods: []string{"ListAbilities", "ShowAbility", "BuildDeployInvocation"}},
			},
		},
		{
			Consumer: "backend_hub",
			Profile:  "wrappers",
			Surfaces: []sharedConsumerCoverageSurface{
				{Type: reflect.TypeOf((*WrapperClient)(nil)), Methods: []string{"TransferFile", "StartTerminalSession", "StartRemoteDesktopSession", "StartBrowserSession", "StartMediaSession"}},
			},
		},
		{
			Consumer: "easyremote",
			Profile:  "runtime_core",
			Surfaces: []sharedConsumerCoverageSurface{
				{Type: reflect.TypeOf((*DaemonControl)(nil)), Methods: []string{"Start", "Attach", "ConnectLocal"}},
				{Type: reflect.TypeOf((*RuntimeClient)(nil)), Methods: []string{"Prepare", "SubmitSigned", "Invoke", "InvokeStream", "OpenBidi"}},
			},
		},
		{
			Consumer: "easyremote",
			Profile:  "directory_identity",
			Surfaces: []sharedConsumerCoverageSurface{
				{Type: reflect.TypeOf((*DirectoryClient)(nil)), Methods: []string{"Resolve", "ListAbilities"}},
				{Type: reflect.TypeOf((*IdentityClient)(nil)), Methods: []string{"BuildResourceRef", "Signer", "RegisterSigningKey", "ListSigningKeys"}},
			},
		},
		{
			Consumer: "easyremote",
			Profile:  "publication",
			Surfaces: []sharedConsumerCoverageSurface{
				{Type: reflect.TypeOf((*PublicationClient)(nil)), Methods: []string{"BuildLocalResourceRef", "DeployAbility", "ListAbilities", "ShowAbility", "EnableAbilityImpl", "DisableAbilityImpl"}},
			},
		},
		{
			Consumer: "easyremote",
			Profile:  "host_binding",
			Surfaces: []sharedConsumerCoverageSurface{
				{Type: reflect.TypeOf((*HostBindingClient)(nil)), Methods: []string{"BuildHostStreamBinding", "DecodeRequest", "EncodeItem", "EncodeError", "EncodeTerminal", "FoldOutputHash"}},
			},
		},
		{
			Consumer: "easyremote",
			Profile:  "mission",
			Surfaces: []sharedConsumerCoverageSurface{
				{Type: reflect.TypeOf((*MissionClient)(nil)), Methods: []string{"BuildRunEALInvocation", "RunEAL", "RunFile", "Track", "Cancel", "Events", "TailEvents"}},
			},
		},
		{
			Consumer: "easyremote",
			Profile:  "admin_gateway",
			Surfaces: []sharedConsumerCoverageSurface{
				{Type: reflect.TypeOf((*AdminClient)(nil)), Methods: []string{"GatewayStatus", "ListAgents", "AgentStart", "AgentRefresh"}},
			},
		},
		{
			Consumer: "cli",
			Profile:  "runtime_core",
			Surfaces: []sharedConsumerCoverageSurface{
				{Type: reflect.TypeOf((*DaemonControl)(nil)), Methods: []string{"Discover", "Start", "Attach"}},
				{Type: reflect.TypeOf((*RuntimeClient)(nil)), Methods: []string{"Invoke", "InvokeStream", "OpenBidi"}},
			},
		},
		{
			Consumer: "cli",
			Profile:  "directory_identity",
			Surfaces: []sharedConsumerCoverageSurface{
				{Type: reflect.TypeOf((*DirectoryClient)(nil)), Methods: []string{"Resolve", "ListDevices", "ListAgents", "ListAbilities"}},
			},
		},
		{
			Consumer: "cli",
			Profile:  "publication",
			Surfaces: []sharedConsumerCoverageSurface{
				{Type: reflect.TypeOf((*PublicationClient)(nil)), Methods: []string{"ValidatePackage", "DeployAbility", "InstallPlugin", "UnpublishAbility"}},
			},
		},
		{
			Consumer: "cli",
			Profile:  "host_binding",
			Surfaces: []sharedConsumerCoverageSurface{
				{Type: reflect.TypeOf((*HostBindingClient)(nil)), Methods: []string{"BuildHostStreamBinding", "FoldOutputHash"}},
			},
		},
		{
			Consumer: "cli",
			Profile:  "mission",
			Surfaces: []sharedConsumerCoverageSurface{
				{Type: reflect.TypeOf((*MissionClient)(nil)), Methods: []string{"RunEAL", "RunFile", "Track", "Cancel"}},
			},
		},
		{
			Consumer: "cli",
			Profile:  "admin_gateway",
			Surfaces: []sharedConsumerCoverageSurface{
				{Type: reflect.TypeOf((*AdminClient)(nil)), Methods: []string{"GatewayStatus", "JoinHub", "LeaveHub", "ListAgents"}},
			},
		},
		{
			Consumer: "cli",
			Profile:  "wrappers",
			Surfaces: []sharedConsumerCoverageSurface{
				{Type: reflect.TypeOf((*WrapperClient)(nil)), Methods: []string{"TransferFile", "StartTerminalSession"}},
			},
		},
		{
			Consumer: "desktop_gui",
			Profile:  "runtime_core",
			Surfaces: []sharedConsumerCoverageSurface{
				{Type: reflect.TypeOf((*DaemonControl)(nil)), Methods: []string{"Start", "Attach", "ConnectLocal"}},
				{Type: reflect.TypeOf((*RuntimeClient)(nil)), Methods: []string{"Invoke", "InvokeStream", "OpenBidi"}},
				{Type: reflect.TypeOf((*HealthClient)(nil)), Methods: []string{"RuntimeHealth", "Diagnostics"}},
			},
		},
		{
			Consumer: "desktop_gui",
			Profile:  "directory_identity",
			Surfaces: []sharedConsumerCoverageSurface{
				{Type: reflect.TypeOf((*DirectoryClient)(nil)), Methods: []string{"ListDevices", "ListAgents", "ListAbilities", "Resolve"}},
			},
		},
		{
			Consumer: "desktop_gui",
			Profile:  "wrappers",
			Surfaces: []sharedConsumerCoverageSurface{
				{Type: reflect.TypeOf((*WrapperClient)(nil)), Methods: []string{"StartTerminalSession", "StartRemoteDesktopSession"}},
			},
		},
		{
			Consumer: "third_party_host_app",
			Profile:  "runtime_core",
			Surfaces: []sharedConsumerCoverageSurface{
				{Type: reflect.TypeOf((*RuntimeClient)(nil)), Methods: []string{"Invoke", "Prepare", "SubmitSigned"}},
			},
		},
		{
			Consumer: "third_party_host_app",
			Profile:  "directory_identity",
			Surfaces: []sharedConsumerCoverageSurface{
				{Type: reflect.TypeOf((*DirectoryClient)(nil)), Methods: []string{"Resolve", "ListAbilities"}},
				{Type: reflect.TypeOf((*IdentityClient)(nil)), Methods: []string{"BuildResourceRef", "ProjectDescriptorRef"}},
			},
		},
		{
			Consumer: "third_party_host_app",
			Profile:  "publication",
			Surfaces: []sharedConsumerCoverageSurface{
				{Type: reflect.TypeOf((*PublicationClient)(nil)), Methods: []string{"BuildLocalResourceRef", "ValidatePackage", "DeployAbility"}},
			},
		},
		{
			Consumer: "third_party_host_app",
			Profile:  "host_binding",
			Surfaces: []sharedConsumerCoverageSurface{
				{Type: reflect.TypeOf((*HostBindingClient)(nil)), Methods: []string{"BuildHostStreamBinding", "DecodeRequest", "EncodeTerminal", "FoldOutputHash"}},
			},
		},
	}

	missing, duplicates := auditSharedConsumerCoverage(requirements)
	if len(missing) > 0 {
		t.Fatalf("consumer coverage has missing public SDK methods:\n%s", strings.Join(missing, "\n"))
	}
	if len(duplicates) > 0 {
		t.Fatalf("consumer coverage has duplicate consumer/profile rows:\n%s", strings.Join(duplicates, "\n"))
	}
}

type sharedConsumerCoverageRequirement struct {
	Consumer string
	Profile  string
	Surfaces []sharedConsumerCoverageSurface
}

type sharedConsumerCoverageSurface struct {
	Type    reflect.Type
	Methods []string
}

func auditSharedConsumerCoverage(requirements []sharedConsumerCoverageRequirement) ([]string, []string) {
	var missing []string
	seen := make(map[string]struct{})
	var duplicates []string
	for _, requirement := range requirements {
		key := requirement.Consumer + "/" + requirement.Profile
		if _, ok := seen[key]; ok {
			duplicates = append(duplicates, key)
		}
		seen[key] = struct{}{}
		if len(requirement.Surfaces) == 0 {
			missing = append(missing, key+" has no public SDK surface")
			continue
		}
		for _, surface := range requirement.Surfaces {
			if surface.Type == nil || surface.Type.Kind() != reflect.Pointer {
				missing = append(missing, key+" has invalid SDK surface type")
				continue
			}
			for _, method := range surface.Methods {
				if _, ok := surface.Type.MethodByName(method); !ok {
					missing = append(missing, fmt.Sprintf("%s missing %s.%s", key, surface.Type.Elem().Name(), method))
				}
			}
		}
	}
	sort.Strings(missing)
	sort.Strings(duplicates)
	return missing, duplicates
}

func TestGoMEMCExecutesSharedNoCoreBloatConformanceCase(t *testing.T) {
	root := repositoryRoot(t)
	noBloatCase := sharedCase(t, root, "memc-no-core-bloat.yaml")
	requireCaseID(t, noBloatCase, "memc/no_core_bloat")
	requireCaseAction(t, noBloatCase, "inspect_runtime_core_surface")
	for _, expectation := range []string{
		"publication_package_building: false",
		"python_decorators: false",
		"backend_dtos: false",
		"cli_command_text: false",
		"one_method_per_ability_required_api: false",
	} {
		requireCaseExpectation(t, noBloatCase, expectation)
	}
	for _, scope := range []string{
		"lifecycle",
		"invocation",
		"signing_material",
		"unary_stream_bidi",
		"health_error",
		"process_safe_client_behavior",
	} {
		requireCaseLiteral(t, noBloatCase, "- "+scope)
	}

	forbiddenTokens := []string{
		"Publication",
		"DeployAbility",
		"ValidatePackage",
		"InstallPlugin",
		"HostBinding",
		"Mission",
		"Admin",
		"Gateway",
		"Surface",
		"Compatibility",
		"Wrapper",
		"Backend",
		"CLICommand",
		"Decorator",
		"InvokeAbility",
		"CallAbility",
	}
	for _, token := range forbiddenTokens {
		requireCaseLiteral(t, noBloatCase, "- "+token)
	}

	surfaces := []sharedNoCoreBloatSurface{
		{Name: "Client", Type: reflect.TypeOf((*Client)(nil))},
		{Name: "DaemonControl", Type: reflect.TypeOf((*DaemonControl)(nil))},
		{Name: "DaemonHandle", Type: reflect.TypeOf((*DaemonHandle)(nil))},
		{Name: "RuntimeClient", Type: reflect.TypeOf((*RuntimeClient)(nil))},
		{Name: "HealthClient", Type: reflect.TypeOf((*HealthClient)(nil))},
		{Name: "InvocationBuilder", Type: reflect.TypeOf((*InvocationBuilder)(nil))},
		{Name: "StreamHandle", Type: reflect.TypeOf((*StreamHandle)(nil))},
		{Name: "BidiSession", Type: reflect.TypeOf((*BidiSession)(nil))},
		{Name: "PreparedInvocation", Type: reflect.TypeOf((*PreparedInvocation)(nil))},
		{Name: "SignedInvocation", Type: reflect.TypeOf((*SignedInvocation)(nil))},
	}
	coreFiles := []string{
		"sdk/go/client.go",
		"sdk/go/runtime.go",
		"sdk/go/daemon.go",
		"sdk/go/health.go",
		"sdk/go/invocation.go",
		"sdk/go/signing.go",
		"sdk/go/stream.go",
		"sdk/go/bidi.go",
		"sdk/go/errors.go",
	}
	violations := auditSharedNoCoreBloat(t, root, surfaces, coreFiles, forbiddenTokens)
	if len(violations) > 0 {
		t.Fatalf("Runtime Core exposes profile/product bloat:\n%s", strings.Join(violations, "\n"))
	}
}

type sharedNoCoreBloatSurface struct {
	Name string
	Type reflect.Type
}

func auditSharedNoCoreBloat(t *testing.T, root string, surfaces []sharedNoCoreBloatSurface, coreFiles []string, forbiddenTokens []string) []string {
	t.Helper()
	var violations []string
	for _, surface := range surfaces {
		for i := 0; i < surface.Type.NumMethod(); i++ {
			method := surface.Type.Method(i)
			for _, token := range forbiddenTokens {
				if strings.Contains(method.Name, token) {
					violations = append(violations, fmt.Sprintf("%s.%s contains forbidden token %s", surface.Name, method.Name, token))
				}
			}
		}
	}
	for _, file := range coreFiles {
		raw, err := os.ReadFile(filepath.Join(root, file))
		if err != nil {
			t.Fatalf("read core source %s: %v", file, err)
		}
		body := string(raw)
		for _, token := range forbiddenTokens {
			if strings.Contains(body, token) {
				violations = append(violations, fmt.Sprintf("%s contains forbidden token %s", file, token))
			}
		}
	}
	sort.Strings(violations)
	return violations
}

type sharedMissionTransport struct {
	t                      *testing.T
	expectedRunRequest     []byte
	expectedRunFileRequest []byte
	expectedTrackRequest   []byte
	expectedCancelRequest  []byte
	expectedEventsRequest  []byte
	runInvocationJSON      []byte
	trackInvocationJSON    []byte
	cancelInvocationJSON   []byte
	statusJSON             []byte
	eventsJSON             []byte
}

func (t *sharedMissionTransport) BuildRunEALInvocation(_ context.Context, requestJSON []byte) ([]byte, error) {
	assertJSONEquivalent(t.t, requestJSON, t.expectedRunRequest)
	return t.runInvocationJSON, nil
}

func (t *sharedMissionTransport) BuildRunFileInvocation(_ context.Context, requestJSON []byte) ([]byte, error) {
	assertJSONEquivalent(t.t, requestJSON, t.expectedRunFileRequest)
	return t.runInvocationJSON, nil
}

func (t *sharedMissionTransport) BuildTrackInvocation(_ context.Context, requestJSON []byte) ([]byte, error) {
	assertJSONEquivalent(t.t, requestJSON, t.expectedTrackRequest)
	return t.trackInvocationJSON, nil
}

func (t *sharedMissionTransport) BuildCancelInvocation(_ context.Context, requestJSON []byte) ([]byte, error) {
	assertJSONEquivalent(t.t, requestJSON, t.expectedCancelRequest)
	return t.cancelInvocationJSON, nil
}

func (t *sharedMissionTransport) RunEAL(context.Context, []byte) ([]byte, error) {
	return nil, &SDKError{Code: ErrNotImplemented, Stage: "test", Retry: RetryNever}
}

func (t *sharedMissionTransport) RunFile(context.Context, []byte) ([]byte, error) {
	return nil, &SDKError{Code: ErrNotImplemented, Stage: "test", Retry: RetryNever}
}

func (t *sharedMissionTransport) Track(_ context.Context, requestJSON []byte) ([]byte, error) {
	assertJSONEquivalent(t.t, requestJSON, t.expectedTrackRequest)
	return t.statusJSON, nil
}

func (t *sharedMissionTransport) Cancel(_ context.Context, requestJSON []byte) ([]byte, error) {
	assertJSONEquivalent(t.t, requestJSON, t.expectedCancelRequest)
	return t.statusJSON, nil
}

func (t *sharedMissionTransport) Events(_ context.Context, requestJSON []byte) ([]byte, error) {
	assertJSONEquivalent(t.t, requestJSON, t.expectedEventsRequest)
	return t.eventsJSON, nil
}

func (t *sharedMissionTransport) Close(context.Context) error {
	return nil
}

type sharedAdminGatewayTransport struct {
	t                                  *testing.T
	expectedAgentListRequest           []byte
	expectedAgentStartRequest          []byte
	expectedAgentStopRequest           []byte
	expectedAgentRefreshRequest        []byte
	expectedSessionListRequest         []byte
	expectedPairingPreflightRequest    []byte
	expectedPairingCreateRequest       []byte
	expectedPairingValidateRequest     []byte
	expectedDeviceSessionCreateRequest []byte
	expectedDeviceSessionDeleteRequest []byte
	expectedGatewayStatusRequest       []byte
	agentListInvocationJSON            []byte
	agentStartInvocationJSON           []byte
	agentStopInvocationJSON            []byte
	agentRefreshInvocationJSON         []byte
	sessionListInvocationJSON          []byte
	gatewayStatusJSON                  []byte
	agentRecordsJSON                   []byte
	agentLifecycleResultJSON           []byte
	pairingPreflightJSON               []byte
	pairingTokenJSON                   []byte
	deviceCredentialJSON               []byte
	deviceSessionJSON                  []byte
	deviceSessionPageJSON              []byte
	deviceSessionDeleteResultJSON      []byte
}

func (t *sharedAdminGatewayTransport) BuildAgentListInvocation(_ context.Context, requestJSON []byte) ([]byte, error) {
	assertJSONEquivalent(t.t, requestJSON, t.expectedAgentListRequest)
	return t.agentListInvocationJSON, nil
}

func (t *sharedAdminGatewayTransport) BuildAgentStartInvocation(_ context.Context, requestJSON []byte) ([]byte, error) {
	assertJSONEquivalent(t.t, requestJSON, t.expectedAgentStartRequest)
	return t.agentStartInvocationJSON, nil
}

func (t *sharedAdminGatewayTransport) BuildAgentStopInvocation(_ context.Context, requestJSON []byte) ([]byte, error) {
	assertJSONEquivalent(t.t, requestJSON, t.expectedAgentStopRequest)
	return t.agentStopInvocationJSON, nil
}

func (t *sharedAdminGatewayTransport) BuildAgentRefreshInvocation(_ context.Context, requestJSON []byte) ([]byte, error) {
	assertJSONEquivalent(t.t, requestJSON, t.expectedAgentRefreshRequest)
	return t.agentRefreshInvocationJSON, nil
}

func (t *sharedAdminGatewayTransport) BuildSessionListInvocation(_ context.Context, requestJSON []byte) ([]byte, error) {
	assertJSONEquivalent(t.t, requestJSON, t.expectedSessionListRequest)
	return t.sessionListInvocationJSON, nil
}

func (t *sharedAdminGatewayTransport) BuildRevokeDeviceInvocation(context.Context, []byte) ([]byte, error) {
	return nil, &SDKError{Code: ErrNotImplemented, Stage: "test", Retry: RetryNever}
}

func (t *sharedAdminGatewayTransport) GatewayStatus(_ context.Context, requestJSON []byte) ([]byte, error) {
	assertJSONEquivalent(t.t, requestJSON, t.expectedGatewayStatusRequest)
	return t.gatewayStatusJSON, nil
}

func (t *sharedAdminGatewayTransport) ListAgents(_ context.Context, requestJSON []byte) ([]byte, error) {
	assertJSONEquivalent(t.t, requestJSON, t.expectedAgentListRequest)
	return t.agentRecordsJSON, nil
}

func (t *sharedAdminGatewayTransport) AgentStart(_ context.Context, requestJSON []byte) ([]byte, error) {
	assertJSONEquivalent(t.t, requestJSON, t.expectedAgentStartRequest)
	return t.agentLifecycleResultJSON, nil
}

func (t *sharedAdminGatewayTransport) AgentStop(_ context.Context, requestJSON []byte) ([]byte, error) {
	assertJSONEquivalent(t.t, requestJSON, t.expectedAgentStopRequest)
	return t.agentLifecycleResultJSON, nil
}

func (t *sharedAdminGatewayTransport) AgentRefresh(_ context.Context, requestJSON []byte) ([]byte, error) {
	assertJSONEquivalent(t.t, requestJSON, t.expectedAgentRefreshRequest)
	return t.agentLifecycleResultJSON, nil
}

func (t *sharedAdminGatewayTransport) ListDeviceSessions(_ context.Context, requestJSON []byte) ([]byte, error) {
	assertJSONEquivalent(t.t, requestJSON, t.expectedSessionListRequest)
	return t.deviceSessionPageJSON, nil
}

func (t *sharedAdminGatewayTransport) JoinHub(context.Context, []byte) ([]byte, error) {
	return nil, &SDKError{Code: ErrNotImplemented, Stage: "test", Retry: RetryNever}
}

func (t *sharedAdminGatewayTransport) LeaveHub(context.Context, []byte) ([]byte, error) {
	return nil, &SDKError{Code: ErrNotImplemented, Stage: "test", Retry: RetryNever}
}

func (t *sharedAdminGatewayTransport) PairingPreflight(_ context.Context, requestJSON []byte) ([]byte, error) {
	assertJSONEquivalent(t.t, requestJSON, t.expectedPairingPreflightRequest)
	return t.pairingPreflightJSON, nil
}

func (t *sharedAdminGatewayTransport) ValidatePairing(_ context.Context, requestJSON []byte) ([]byte, error) {
	assertJSONEquivalent(t.t, requestJSON, t.expectedPairingValidateRequest)
	return t.deviceCredentialJSON, nil
}

func (t *sharedAdminGatewayTransport) VerifyDeviceCredential(context.Context, []byte) ([]byte, error) {
	return nil, &SDKError{Code: ErrNotImplemented, Stage: "test", Retry: RetryNever}
}

func (t *sharedAdminGatewayTransport) CreatePairing(_ context.Context, requestJSON []byte) ([]byte, error) {
	assertJSONEquivalent(t.t, requestJSON, t.expectedPairingCreateRequest)
	return t.pairingTokenJSON, nil
}

func (t *sharedAdminGatewayTransport) RevokeDevice(context.Context, []byte) ([]byte, error) {
	return nil, &SDKError{Code: ErrNotImplemented, Stage: "test", Retry: RetryNever}
}

func (t *sharedAdminGatewayTransport) CreateDeviceSession(_ context.Context, requestJSON []byte) ([]byte, error) {
	assertJSONEquivalent(t.t, requestJSON, t.expectedDeviceSessionCreateRequest)
	return t.deviceSessionJSON, nil
}

func (t *sharedAdminGatewayTransport) DeleteDeviceSession(_ context.Context, requestJSON []byte) ([]byte, error) {
	assertJSONEquivalent(t.t, requestJSON, t.expectedDeviceSessionDeleteRequest)
	return t.deviceSessionDeleteResultJSON, nil
}

type sharedEventsTransport struct {
	t                                     *testing.T
	expectedDirectorySubscriptionRequest  []byte
	expectedDeviceSubscriptionRequest     []byte
	expectedSessionSubscriptionRequest    []byte
	expectedInvocationSubscriptionRequest []byte
	expectedDeviceEventListRequest        []byte
	expectedDirectoryProjectionInput      []byte
	expectedDropReportInput               []byte
	expectedTerminalInput                 []byte
	directorySubscriptionInvocationJSON   []byte
	deviceSubscriptionInvocationJSON      []byte
	sessionSubscriptionInvocationJSON     []byte
	invocationSubscriptionInvocationJSON  []byte
	deviceEventPageJSON                   []byte
	directoryEventJSON                    []byte
	dropReportJSON                        []byte
	terminalJSON                          []byte
}

func (t *sharedEventsTransport) BuildDirectorySubscriptionInvocation(_ context.Context, requestJSON []byte) ([]byte, error) {
	assertJSONEquivalent(t.t, requestJSON, t.expectedDirectorySubscriptionRequest)
	return t.directorySubscriptionInvocationJSON, nil
}

func (t *sharedEventsTransport) BuildDeviceSubscriptionInvocation(_ context.Context, requestJSON []byte) ([]byte, error) {
	assertJSONEquivalent(t.t, requestJSON, t.expectedDeviceSubscriptionRequest)
	return t.deviceSubscriptionInvocationJSON, nil
}

func (t *sharedEventsTransport) BuildSessionSubscriptionInvocation(_ context.Context, requestJSON []byte) ([]byte, error) {
	assertJSONEquivalent(t.t, requestJSON, t.expectedSessionSubscriptionRequest)
	return t.sessionSubscriptionInvocationJSON, nil
}

func (t *sharedEventsTransport) BuildInvocationSubscriptionInvocation(_ context.Context, requestJSON []byte) ([]byte, error) {
	assertJSONEquivalent(t.t, requestJSON, t.expectedInvocationSubscriptionRequest)
	return t.invocationSubscriptionInvocationJSON, nil
}

func (t *sharedEventsTransport) SubscribeDirectory(context.Context, []byte) ([]byte, error) {
	return nil, &SDKError{Code: ErrNotImplemented, Stage: "test", Retry: RetryNever}
}

func (t *sharedEventsTransport) SubscribeDevices(context.Context, []byte) ([]byte, error) {
	return nil, &SDKError{Code: ErrNotImplemented, Stage: "test", Retry: RetryNever}
}

func (t *sharedEventsTransport) SubscribeSessions(context.Context, []byte) ([]byte, error) {
	return nil, &SDKError{Code: ErrNotImplemented, Stage: "test", Retry: RetryNever}
}

func (t *sharedEventsTransport) SubscribeInvocations(context.Context, []byte) ([]byte, error) {
	return nil, &SDKError{Code: ErrNotImplemented, Stage: "test", Retry: RetryNever}
}

func (t *sharedEventsTransport) ListDeviceEvents(_ context.Context, requestJSON []byte) ([]byte, error) {
	assertJSONEquivalent(t.t, requestJSON, t.expectedDeviceEventListRequest)
	return t.deviceEventPageJSON, nil
}

func (t *sharedEventsTransport) ProjectDirectoryEvent(_ context.Context, eventJSON []byte) ([]byte, error) {
	assertJSONEquivalent(t.t, eventJSON, t.expectedDirectoryProjectionInput)
	return t.directoryEventJSON, nil
}

func (t *sharedEventsTransport) ProjectDropReport(_ context.Context, dropJSON []byte) ([]byte, error) {
	assertJSONEquivalent(t.t, dropJSON, t.expectedDropReportInput)
	return t.dropReportJSON, nil
}

func (t *sharedEventsTransport) ProjectTerminal(_ context.Context, terminalJSON []byte) ([]byte, error) {
	assertJSONEquivalent(t.t, terminalJSON, t.expectedTerminalInput)
	return t.terminalJSON, nil
}

type sharedSurfaceTransport struct {
	t                       *testing.T
	expectedListRequest     []byte
	expectedCreateRequest   []byte
	expectedDeleteRequest   []byte
	expectedManifestRequest []byte
	expectedHealthRequest   []byte
	listInvocationJSON      []byte
	createInvocationJSON    []byte
	deleteInvocationJSON    []byte
	manifestInvocationJSON  []byte
	healthInvocationJSON    []byte
	pagePageJSON            []byte
	manifestJSON            []byte
	healthJSON              []byte
}

func (t *sharedSurfaceTransport) BuildListPagesInvocation(_ context.Context, requestJSON []byte) ([]byte, error) {
	assertJSONEquivalent(t.t, requestJSON, t.expectedListRequest)
	return t.listInvocationJSON, nil
}

func (t *sharedSurfaceTransport) BuildCreatePageInvocation(_ context.Context, requestJSON []byte) ([]byte, error) {
	assertJSONEquivalent(t.t, requestJSON, t.expectedCreateRequest)
	return t.createInvocationJSON, nil
}

func (t *sharedSurfaceTransport) BuildDeletePageInvocation(_ context.Context, requestJSON []byte) ([]byte, error) {
	assertJSONEquivalent(t.t, requestJSON, t.expectedDeleteRequest)
	return t.deleteInvocationJSON, nil
}

func (t *sharedSurfaceTransport) BuildManifestInvocation(_ context.Context, requestJSON []byte) ([]byte, error) {
	assertJSONEquivalent(t.t, requestJSON, t.expectedManifestRequest)
	return t.manifestInvocationJSON, nil
}

func (t *sharedSurfaceTransport) BuildHealthInvocation(_ context.Context, requestJSON []byte) ([]byte, error) {
	assertJSONEquivalent(t.t, requestJSON, t.expectedHealthRequest)
	return t.healthInvocationJSON, nil
}

func (t *sharedSurfaceTransport) ListPages(_ context.Context, requestJSON []byte) ([]byte, error) {
	assertJSONEquivalent(t.t, requestJSON, t.expectedListRequest)
	return t.pagePageJSON, nil
}

func (t *sharedSurfaceTransport) CreatePage(context.Context, []byte) ([]byte, error) {
	return nil, &SDKError{Code: ErrNotImplemented, Stage: "test", Retry: RetryNever}
}

func (t *sharedSurfaceTransport) DeletePage(context.Context, []byte) ([]byte, error) {
	return nil, &SDKError{Code: ErrNotImplemented, Stage: "test", Retry: RetryNever}
}

func (t *sharedSurfaceTransport) SurfaceManifest(_ context.Context, requestJSON []byte) ([]byte, error) {
	assertJSONEquivalent(t.t, requestJSON, t.expectedManifestRequest)
	return t.manifestJSON, nil
}

func (t *sharedSurfaceTransport) PublicPageRef(context.Context, []byte) ([]byte, error) {
	return nil, &SDKError{Code: ErrNotImplemented, Stage: "test", Retry: RetryNever}
}

func (t *sharedSurfaceTransport) SurfaceHealth(_ context.Context, requestJSON []byte) ([]byte, error) {
	assertJSONEquivalent(t.t, requestJSON, t.expectedHealthRequest)
	return t.healthJSON, nil
}

type sharedCompatibilityTransport struct {
	t                     *testing.T
	expectedListRequest   []byte
	expectedChatRequest   []byte
	expectedStreamRequest []byte
	listInvocationJSON    []byte
	chatInvocationJSON    []byte
	streamInvocationJSON  []byte
	modelPageJSON         []byte
	chatCompletionJSON    []byte
	chatStreamJSON        []byte
}

func (t *sharedCompatibilityTransport) BuildListModelsInvocation(_ context.Context, requestJSON []byte) ([]byte, error) {
	assertJSONEquivalent(t.t, requestJSON, t.expectedListRequest)
	return t.listInvocationJSON, nil
}

func (t *sharedCompatibilityTransport) BuildChatCompletionInvocation(_ context.Context, requestJSON []byte) ([]byte, error) {
	assertJSONEquivalent(t.t, requestJSON, t.expectedChatRequest)
	return t.chatInvocationJSON, nil
}

func (t *sharedCompatibilityTransport) BuildStreamChatCompletionInvocation(_ context.Context, requestJSON []byte) ([]byte, error) {
	expectedStreamRequest := sharedCompatibilityStreamRequestJSON(t.t, t.expectedStreamRequest)
	assertJSONEquivalent(t.t, requestJSON, expectedStreamRequest)
	return t.streamInvocationJSON, nil
}

func (t *sharedCompatibilityTransport) BuildFileUploadInvocation(context.Context, []byte) ([]byte, error) {
	return nil, &SDKError{Code: ErrNotImplemented, Stage: "test", Retry: RetryNever}
}

func (t *sharedCompatibilityTransport) BuildFileRetrieveInvocation(context.Context, []byte) ([]byte, error) {
	return nil, &SDKError{Code: ErrNotImplemented, Stage: "test", Retry: RetryNever}
}

func (t *sharedCompatibilityTransport) BuildFileDeleteInvocation(context.Context, []byte) ([]byte, error) {
	return nil, &SDKError{Code: ErrNotImplemented, Stage: "test", Retry: RetryNever}
}

func (t *sharedCompatibilityTransport) ListModels(_ context.Context, requestJSON []byte) ([]byte, error) {
	assertJSONEquivalent(t.t, requestJSON, t.expectedListRequest)
	return t.modelPageJSON, nil
}

func (t *sharedCompatibilityTransport) CreateChatCompletion(_ context.Context, requestJSON []byte) ([]byte, error) {
	assertJSONEquivalent(t.t, requestJSON, t.expectedChatRequest)
	return t.chatCompletionJSON, nil
}

func (t *sharedCompatibilityTransport) StreamChatCompletion(_ context.Context, requestJSON []byte) ([]byte, error) {
	expectedStreamRequest := sharedCompatibilityStreamRequestJSON(t.t, t.expectedStreamRequest)
	assertJSONEquivalent(t.t, requestJSON, expectedStreamRequest)
	return t.chatStreamJSON, nil
}

func (t *sharedCompatibilityTransport) UploadFile(context.Context, []byte) ([]byte, error) {
	return nil, &SDKError{Code: ErrNotImplemented, Stage: "test", Retry: RetryNever}
}

func (t *sharedCompatibilityTransport) RetrieveFile(context.Context, []byte) ([]byte, error) {
	return nil, &SDKError{Code: ErrNotImplemented, Stage: "test", Retry: RetryNever}
}

func (t *sharedCompatibilityTransport) DeleteFile(context.Context, []byte) ([]byte, error) {
	return nil, &SDKError{Code: ErrNotImplemented, Stage: "test", Retry: RetryNever}
}

type sharedBidiLifecycleTransport struct {
	recvFrames [][]byte
	closed     bool
}

func (t *sharedBidiLifecycleTransport) Send(context.Context, []byte) ([]byte, error) {
	return nil, &SDKError{Code: ErrNotImplemented, Stage: "test", Retry: RetryNever}
}

func (t *sharedBidiLifecycleTransport) Recv(context.Context) ([]byte, error) {
	if len(t.recvFrames) == 0 {
		return nil, invalidRuntimePayload("no shared bidi lifecycle frame", nil)
	}
	frame := t.recvFrames[0]
	t.recvFrames = t.recvFrames[1:]
	return frame, nil
}

func (t *sharedBidiLifecycleTransport) CloseSend(context.Context) ([]byte, error) {
	return []byte(`{"session_id":"bidi-lifecycle-1","state":"HalfClosedLocal","terminal":false}`), nil
}

func (t *sharedBidiLifecycleTransport) Close(context.Context) error {
	t.closed = true
	return nil
}

func (t *sharedBidiLifecycleTransport) Cancel(context.Context, string) ([]byte, error) {
	return nil, &SDKError{Code: ErrNotImplemented, Stage: "test", Retry: RetryNever}
}

type sharedPublicationTransport struct {
	t                       *testing.T
	expectedResourceRequest []byte
	expectedValidateRequest []byte
	expectedDeployRequest   []byte
	resourceJSON            []byte
	validationJSON          []byte
	deployInvocationJSON    []byte
	unpublishInvocationJSON []byte
}

func (t *sharedPublicationTransport) BuildResourceRef(_ context.Context, requestJSON []byte) ([]byte, error) {
	assertJSONEquivalent(t.t, requestJSON, t.expectedResourceRequest)
	return t.resourceJSON, nil
}

func (t *sharedPublicationTransport) ValidatePackage(_ context.Context, requestJSON []byte) ([]byte, error) {
	assertJSONEquivalent(t.t, requestJSON, t.expectedValidateRequest)
	return t.validationJSON, nil
}

func (t *sharedPublicationTransport) DeployAbility(context.Context, []byte) ([]byte, error) {
	return nil, &SDKError{Code: ErrNotImplemented, Stage: "test", Retry: RetryNever}
}

func (t *sharedPublicationTransport) BuildDeployInvocation(_ context.Context, requestJSON []byte) ([]byte, error) {
	assertJSONEquivalent(t.t, requestJSON, t.expectedDeployRequest)
	return t.deployInvocationJSON, nil
}

func (t *sharedPublicationTransport) InstallPlugin(context.Context, []byte) ([]byte, error) {
	return nil, &SDKError{Code: ErrNotImplemented, Stage: "test", Retry: RetryNever}
}

func (t *sharedPublicationTransport) ListAbilities(context.Context, []byte) ([]byte, error) {
	return nil, &SDKError{Code: ErrNotImplemented, Stage: "test", Retry: RetryNever}
}

func (t *sharedPublicationTransport) ShowAbility(context.Context, []byte) ([]byte, error) {
	return nil, &SDKError{Code: ErrNotImplemented, Stage: "test", Retry: RetryNever}
}

func (t *sharedPublicationTransport) EnableAbilityImpl(context.Context, []byte) ([]byte, error) {
	return nil, &SDKError{Code: ErrNotImplemented, Stage: "test", Retry: RetryNever}
}

func (t *sharedPublicationTransport) DisableAbilityImpl(context.Context, []byte) ([]byte, error) {
	return nil, &SDKError{Code: ErrNotImplemented, Stage: "test", Retry: RetryNever}
}

func (t *sharedPublicationTransport) BuildUnpublishInvocation(_ context.Context, requestJSON []byte) ([]byte, error) {
	var request UnpublishAbilityRequest
	if err := json.Unmarshal(requestJSON, &request); err != nil {
		t.t.Fatalf("decode unpublish request: %v", err)
	}
	if request.AbilityURA != "easynet:///r/example/ability/device.dev-a.er.weather" ||
		request.CallerURA != "easynet:///r/example/agent/alice.sdk" {
		t.t.Fatalf("unexpected unpublish request: %#v", request)
	}
	return t.unpublishInvocationJSON, nil
}

func (t *sharedPublicationTransport) UnpublishAbility(context.Context, []byte) ([]byte, error) {
	return nil, &SDKError{Code: ErrNotImplemented, Stage: "test", Retry: RetryNever}
}

func (t *sharedPublicationTransport) Close(context.Context) error {
	return nil
}

type sharedDirectoryTransport struct {
	t                           *testing.T
	expectedDevicesRequest      []byte
	expectedAgentsRequest       []byte
	expectedAbilityRequest      []byte
	expectedResolveRequest      []byte
	expectedSubscriptionRequest []byte
	devicesJSON                 []byte
	peerDevicesJSON             []byte
	agentsJSON                  []byte
	abilitiesJSON               []byte
	resolveJSON                 []byte
	subscriptionInvocationJSON  []byte
	subscriptionJSON            []byte
}

func (t *sharedDirectoryTransport) BuildDirectorySubscriptionInvocation(_ context.Context, requestJSON []byte) ([]byte, error) {
	assertJSONEquivalent(t.t, requestJSON, t.expectedSubscriptionRequest)
	return t.subscriptionInvocationJSON, nil
}

func (t *sharedDirectoryTransport) Resolve(_ context.Context, requestJSON []byte) ([]byte, error) {
	assertJSONEquivalent(t.t, requestJSON, t.expectedResolveRequest)
	return t.resolveJSON, nil
}

func (t *sharedDirectoryTransport) ListDevices(_ context.Context, requestJSON []byte) ([]byte, error) {
	assertJSONEquivalent(t.t, requestJSON, t.expectedDevicesRequest)
	return t.devicesJSON, nil
}

func (t *sharedDirectoryTransport) ListPeerUserDevices(context.Context, []byte) ([]byte, error) {
	return nil, &SDKError{Code: ErrNotImplemented, Stage: "test", Retry: RetryNever}
}

func (t *sharedDirectoryTransport) ListAgents(_ context.Context, requestJSON []byte) ([]byte, error) {
	assertJSONEquivalent(t.t, requestJSON, t.expectedAgentsRequest)
	return t.agentsJSON, nil
}

func (t *sharedDirectoryTransport) ListAbilities(_ context.Context, requestJSON []byte) ([]byte, error) {
	assertJSONEquivalent(t.t, requestJSON, t.expectedAbilityRequest)
	return t.abilitiesJSON, nil
}

func (t *sharedDirectoryTransport) SubscribeDirectory(_ context.Context, requestJSON []byte) ([]byte, error) {
	assertJSONEquivalent(t.t, requestJSON, t.expectedSubscriptionRequest)
	return t.subscriptionJSON, nil
}

func (t *sharedDirectoryTransport) Close(context.Context) error {
	return nil
}

type sharedIdentityTransport struct {
	t                                 *testing.T
	expectedBuildURARequest           []byte
	expectedProjectIdentityRequest    []byte
	expectedDescriptorProjectionInput string
	expectedBuildDescriptorRefRequest []byte
	descriptorJSON                    []byte
	abilityJSON                       []byte
}

func (t *sharedIdentityTransport) ProjectDescriptorRef(_ context.Context, requestJSON []byte) ([]byte, error) {
	var request DescriptorRefRequest
	if err := json.Unmarshal(requestJSON, &request); err != nil {
		t.t.Fatalf("decode descriptor projection request: %v", err)
	}
	if request.DescriptorRef != t.expectedDescriptorProjectionInput {
		t.t.Fatalf("unexpected descriptor projection request: %#v", request)
	}
	return t.descriptorJSON, nil
}

func (t *sharedIdentityTransport) BuildDescriptorRef(_ context.Context, requestJSON []byte) ([]byte, error) {
	assertJSONEquivalent(t.t, requestJSON, t.expectedBuildDescriptorRefRequest)
	return t.descriptorJSON, nil
}

func (t *sharedIdentityTransport) ProjectIdentity(_ context.Context, requestJSON []byte) ([]byte, error) {
	assertJSONEquivalent(t.t, requestJSON, t.expectedProjectIdentityRequest)
	return t.abilityJSON, nil
}

func (t *sharedIdentityTransport) BuildURA(_ context.Context, requestJSON []byte) ([]byte, error) {
	assertJSONEquivalent(t.t, requestJSON, t.expectedBuildURARequest)
	return t.abilityJSON, nil
}

func (t *sharedIdentityTransport) BuildResourceRef(context.Context, []byte) ([]byte, error) {
	return nil, &SDKError{Code: ErrNotImplemented, Stage: "test", Retry: RetryNever}
}

func (t *sharedIdentityTransport) RegisterSigningKey(context.Context, []byte) ([]byte, error) {
	return nil, &SDKError{Code: ErrNotImplemented, Stage: "test", Retry: RetryNever}
}

func (t *sharedIdentityTransport) ListSigningKeys(context.Context, []byte) ([]byte, error) {
	return nil, &SDKError{Code: ErrNotImplemented, Stage: "test", Retry: RetryNever}
}

func (t *sharedIdentityTransport) RevokeSigningKey(context.Context, []byte) ([]byte, error) {
	return nil, &SDKError{Code: ErrNotImplemented, Stage: "test", Retry: RetryNever}
}

func (t *sharedIdentityTransport) Signer(context.Context, []byte) ([]byte, error) {
	return nil, &SDKError{Code: ErrNotImplemented, Stage: "test", Retry: RetryNever}
}

func (t *sharedIdentityTransport) Close(context.Context) error {
	return nil
}

type sharedHostBindingTransport struct {
	bindingJSON  []byte
	requestJSON  []byte
	itemJSON     []byte
	terminalJSON []byte
	hashJSON     []byte
	foldCalls    int
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
	t.foldCalls++
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

func sharedLocalResourceRefRequest(t *testing.T, root string) LocalResourceRefRequest {
	t.Helper()
	var request LocalResourceRefRequest
	if err := json.Unmarshal(sharedFixture(t, root, "local-resource-ref-request.v4.json"), &request); err != nil {
		t.Fatalf("decode shared local resource-ref request: %v", err)
	}
	return request
}

func sharedAbilityPackageManifest(t *testing.T, root string) AbilityPackageManifest {
	t.Helper()
	var manifest AbilityPackageManifest
	if err := json.Unmarshal(sharedFixture(t, root, "ability-package-manifest.v4.json"), &manifest); err != nil {
		t.Fatalf("decode shared ability package manifest: %v", err)
	}
	return manifest
}

func sharedPublicationValidatePackageRequest(t *testing.T, root string) []byte {
	t.Helper()
	manifest := map[string]any{}
	if err := json.Unmarshal(sharedFixture(t, root, "ability-package-manifest.v4.json"), &manifest); err != nil {
		t.Fatalf("decode shared publication manifest request: %v", err)
	}
	raw, err := json.Marshal(map[string]any{"manifest": manifest})
	if err != nil {
		t.Fatalf("encode shared publication validation request: %v", err)
	}
	return raw
}

func sharedAbilityDeployRequest(t *testing.T, root string) AbilityDeployRequest {
	t.Helper()
	var request AbilityDeployRequest
	if err := json.Unmarshal(sharedFixture(t, root, "ability-deploy-request.v4.json"), &request); err != nil {
		t.Fatalf("decode shared ability deploy request: %v", err)
	}
	return request
}

func sharedUnpublishAbilityRequest(t *testing.T, root string) UnpublishAbilityRequest {
	t.Helper()
	deploy := sharedAbilityDeployRequest(t, root)
	return UnpublishAbilityRequest{
		CallerURA:         deploy.CallerURA,
		CalleeURA:         deploy.CalleeURA,
		SubjectURA:        deploy.SubjectURA,
		DescriptorVersion: deploy.DescriptorVersion,
		NonceBase64:       deploy.NonceBase64,
		CausalContext:     deploy.CausalContext,
		AbilityURA:        "easynet:///r/example/ability/device.dev-a.er.weather",
	}
}

func sharedMissionCarrierBase(t *testing.T, root string, fixture string) MissionCarrierBase {
	t.Helper()
	var base MissionCarrierBase
	if err := json.Unmarshal(sharedFixture(t, root, fixture), &base); err != nil {
		t.Fatalf("decode shared mission carrier base fixture %s: %v", fixture, err)
	}
	return base
}

func sharedMissionRunRequest(t *testing.T, root string) MissionRunRequest {
	t.Helper()
	var request MissionRunRequest
	if err := json.Unmarshal(sharedFixture(t, root, "mission-run-request.v4.json"), &request); err != nil {
		t.Fatalf("decode shared mission run request: %v", err)
	}
	return request
}

func sharedMissionRunFileRequest(t *testing.T, root string) MissionRunFileRequest {
	t.Helper()
	var request MissionRunFileRequest
	if err := json.Unmarshal(sharedFixture(t, root, "mission-run-file-request.v4.json"), &request); err != nil {
		t.Fatalf("decode shared mission run-file request: %v", err)
	}
	return request
}

func sharedMissionTrackRequest(t *testing.T, root string) MissionTrackRequest {
	t.Helper()
	var request MissionTrackRequest
	if err := json.Unmarshal(sharedFixture(t, root, "mission-track-request.v4.json"), &request); err != nil {
		t.Fatalf("decode shared mission track request: %v", err)
	}
	return request
}

func sharedMissionCancelRequest(t *testing.T, root string) MissionCancelRequest {
	t.Helper()
	var request MissionCancelRequest
	if err := json.Unmarshal(sharedFixture(t, root, "mission-cancel-request.v4.json"), &request); err != nil {
		t.Fatalf("decode shared mission cancel request: %v", err)
	}
	return request
}

func sharedMissionEventsRequest(t *testing.T, root string) MissionEventListRequest {
	t.Helper()
	var request MissionEventListRequest
	if err := json.Unmarshal(sharedFixture(t, root, "mission-events-request.v4.json"), &request); err != nil {
		t.Fatalf("decode shared mission events request: %v", err)
	}
	return request
}

func sharedMissionStatusWithoutParentAnchor(t *testing.T, root string) []byte {
	t.Helper()
	status := map[string]any{}
	if err := json.Unmarshal(sharedFixture(t, root, "mission-status.v4.json"), &status); err != nil {
		t.Fatalf("decode shared mission status fixture: %v", err)
	}
	status["parent_receipt_ura"] = nil
	raw, err := json.Marshal(status)
	if err != nil {
		t.Fatalf("encode unanchored mission status: %v", err)
	}
	return raw
}

func sharedAdminAgentListRequest(t *testing.T, root string) AdminAgentListRequest {
	t.Helper()
	var request AdminAgentListRequest
	if err := json.Unmarshal(sharedFixture(t, root, "admin-agent-list-request.v4.json"), &request); err != nil {
		t.Fatalf("decode shared admin agent-list request: %v", err)
	}
	return request
}

func sharedAdminAgentStartRequest(t *testing.T, root string) AdminAgentStartRequest {
	t.Helper()
	var request AdminAgentStartRequest
	if err := json.Unmarshal(sharedFixture(t, root, "admin-agent-start-request.v4.json"), &request); err != nil {
		t.Fatalf("decode shared admin agent-start request: %v", err)
	}
	return request
}

func sharedAdminAgentStopRequest(t *testing.T, root string) AdminAgentStopRequest {
	t.Helper()
	var request AdminAgentStopRequest
	if err := json.Unmarshal(sharedFixture(t, root, "admin-agent-stop-request.v4.json"), &request); err != nil {
		t.Fatalf("decode shared admin agent-stop request: %v", err)
	}
	return request
}

func sharedAdminAgentRefreshRequest(t *testing.T, root string) AdminAgentRefreshRequest {
	t.Helper()
	var request AdminAgentRefreshRequest
	if err := json.Unmarshal(sharedFixture(t, root, "admin-agent-refresh-request.v4.json"), &request); err != nil {
		t.Fatalf("decode shared admin agent-refresh request: %v", err)
	}
	return request
}

func sharedAdminSessionListRequest(t *testing.T, root string) AdminSessionListRequest {
	t.Helper()
	var request AdminSessionListRequest
	if err := json.Unmarshal(sharedFixture(t, root, "admin-session-list-request.v4.json"), &request); err != nil {
		t.Fatalf("decode shared admin session-list request: %v", err)
	}
	return request
}

func sharedAdminPairingPreflightRequest(t *testing.T, root string) PairingPreflightRequest {
	t.Helper()
	var request PairingPreflightRequest
	if err := json.Unmarshal(sharedFixture(t, root, "admin-pairing-preflight-request.v4.json"), &request); err != nil {
		t.Fatalf("decode shared admin pairing-preflight request: %v", err)
	}
	return request
}

func sharedAdminPairingCreateRequest(t *testing.T, root string) CreatePairingRequest {
	t.Helper()
	var request CreatePairingRequest
	if err := json.Unmarshal(sharedFixture(t, root, "admin-pairing-create-request.v4.json"), &request); err != nil {
		t.Fatalf("decode shared admin pairing-create request: %v", err)
	}
	return request
}

func sharedAdminPairingValidateRequest(t *testing.T, root string) ValidatePairingRequest {
	t.Helper()
	var request ValidatePairingRequest
	if err := json.Unmarshal(sharedFixture(t, root, "admin-pairing-validate-request.v4.json"), &request); err != nil {
		t.Fatalf("decode shared admin pairing-validate request: %v", err)
	}
	return request
}

func sharedAdminDeviceSessionCreateRequest(t *testing.T, root string) CreateDeviceSessionRequest {
	t.Helper()
	var request CreateDeviceSessionRequest
	if err := json.Unmarshal(sharedFixture(t, root, "admin-device-session-create-request.v4.json"), &request); err != nil {
		t.Fatalf("decode shared admin device-session-create request: %v", err)
	}
	return request
}

func sharedAdminDeviceSessionDeleteRequest(t *testing.T, root string) DeleteDeviceSessionRequest {
	t.Helper()
	var request DeleteDeviceSessionRequest
	if err := json.Unmarshal(sharedFixture(t, root, "admin-device-session-delete-request.v4.json"), &request); err != nil {
		t.Fatalf("decode shared admin device-session-delete request: %v", err)
	}
	return request
}

func sharedControlOnlyGatewayStatus(t *testing.T, root string) []byte {
	t.Helper()
	status := map[string]any{}
	if err := json.Unmarshal(sharedFixture(t, root, "gateway-status.v4.json"), &status); err != nil {
		t.Fatalf("decode shared gateway status fixture: %v", err)
	}
	status["ready"] = false
	status["state"] = "degraded"
	status["runtime_ready"] = false
	status["directory_ready"] = false
	metadata, ok := status["metadata"].(map[string]any)
	if !ok {
		t.Fatalf("gateway status metadata is not an object")
	}
	metadata["lifecycle_state"] = "control_only"
	raw, err := json.Marshal(status)
	if err != nil {
		t.Fatalf("encode control-only gateway status: %v", err)
	}
	return raw
}

func sharedEventsDirectorySubscriptionRequest(t *testing.T, root string) EventsDirectorySubscriptionRequest {
	t.Helper()
	var request EventsDirectorySubscriptionRequest
	if err := json.Unmarshal(sharedFixture(t, root, "events-directory-subscription-request.v4.json"), &request); err != nil {
		t.Fatalf("decode shared events directory subscription request: %v", err)
	}
	return request
}

func sharedEventsDirectorySubscriptionRequestJSON(t *testing.T, root string) []byte {
	t.Helper()
	raw, err := marshalEventsSubscriptionRequest(sharedEventsDirectorySubscriptionRequest(t, root), EventStreamDirectory)
	if err != nil {
		t.Fatalf("marshal shared events directory subscription request: %v", err)
	}
	return raw
}

func sharedIdentityAbilityProjectionJSON(t *testing.T, root string) []byte {
	t.Helper()
	var descriptor IdentityProjection
	if err := json.Unmarshal(sharedFixture(t, root, "identity.descriptor-ref.v4.json"), &descriptor); err != nil {
		t.Fatalf("decode shared descriptor projection: %v", err)
	}
	abilityName, _ := descriptor.Components["ability_name"].(string)
	if abilityName == "" {
		abilityName = "observe.health"
	}
	projection := map[string]any{
		"kind":       "ability",
		"valid":      true,
		"ura":        descriptor.AbilityURA,
		"realm":      "example",
		"display_id": "device.dev-a.observe.health",
		"profile":    descriptor.Profile,
		"components": map[string]any{
			"owner_ura":    descriptor.Components["owner_ura"],
			"ability_name": abilityName,
		},
		"metadata": descriptor.Metadata,
	}
	raw, err := json.Marshal(projection)
	if err != nil {
		t.Fatalf("encode shared ability projection: %v", err)
	}
	return raw
}

func sharedEventsSessionSubscriptionRequest(t *testing.T, root string) EventsSessionSubscriptionRequest {
	t.Helper()
	var request EventsSessionSubscriptionRequest
	if err := json.Unmarshal(sharedFixture(t, root, "events-session-subscription-request.v4.json"), &request); err != nil {
		t.Fatalf("decode shared events session subscription request: %v", err)
	}
	return request
}

func sharedEventsSessionSubscriptionRequestJSON(t *testing.T, root string) []byte {
	t.Helper()
	raw, err := marshalEventsSubscriptionRequest(sharedEventsSessionSubscriptionRequest(t, root), EventStreamSession)
	if err != nil {
		t.Fatalf("marshal shared events session subscription request: %v", err)
	}
	return raw
}

func sharedEventsDeviceSubscriptionRequest(t *testing.T, root string) EventsDeviceSubscriptionRequest {
	t.Helper()
	var request EventsDeviceSubscriptionRequest
	if err := json.Unmarshal(sharedFixture(t, root, "events-device-subscription-request.v4.json"), &request); err != nil {
		t.Fatalf("decode shared events device subscription request: %v", err)
	}
	return request
}

func sharedEventsDeviceSubscriptionRequestJSON(t *testing.T, root string) []byte {
	t.Helper()
	raw, err := marshalEventsSubscriptionRequest(sharedEventsDeviceSubscriptionRequest(t, root), EventStreamDevice)
	if err != nil {
		t.Fatalf("marshal shared events device subscription request: %v", err)
	}
	return raw
}

func sharedEventsInvocationSubscriptionRequest(t *testing.T, root string) EventsInvocationSubscriptionRequest {
	t.Helper()
	var request EventsInvocationSubscriptionRequest
	if err := json.Unmarshal(sharedFixture(t, root, "events-invocation-subscription-request.v4.json"), &request); err != nil {
		t.Fatalf("decode shared events invocation subscription request: %v", err)
	}
	return request
}

func sharedEventsInvocationSubscriptionRequestJSON(t *testing.T, root string) []byte {
	t.Helper()
	raw, err := marshalEventsSubscriptionRequest(sharedEventsInvocationSubscriptionRequest(t, root), EventStreamInvocation)
	if err != nil {
		t.Fatalf("marshal shared events invocation subscription request: %v", err)
	}
	return raw
}

func sharedEventsDeviceEventListRequest(t *testing.T, root string) EventsDeviceEventListRequest {
	t.Helper()
	var request EventsDeviceEventListRequest
	if err := json.Unmarshal(sharedFixture(t, root, "events-device-event-list-request.v4.json"), &request); err != nil {
		t.Fatalf("decode shared events device event list request: %v", err)
	}
	return request
}

func sharedEventsDeviceEventListRequestJSON(t *testing.T, root string) []byte {
	t.Helper()
	raw, err := marshalEventsDeviceEventListRequest(sharedEventsDeviceEventListRequest(t, root))
	if err != nil {
		t.Fatalf("marshal shared events device event list request: %v", err)
	}
	return raw
}

func sharedEventsProjectionInput(t *testing.T, root string) EventProjectionInput {
	t.Helper()
	frame := sharedEventsFrameMap(t, root, "event.directory.v4.json")
	cursor := sharedEventsCursorFromFrame(t, frame)
	return EventProjectionInput{
		Cursor:      cursor,
		Event:       frame["payload"].(map[string]any),
		EventID:     frame["event_id"].(string),
		ResumeToken: frame["resume_token"].(string),
		TenantRef:   frame["tenant_ref"],
	}
}

func sharedEventsProjectionInputJSON(t *testing.T, root string) []byte {
	t.Helper()
	raw, err := json.Marshal(sharedEventsProjectionInput(t, root))
	if err != nil {
		t.Fatalf("marshal shared events projection input: %v", err)
	}
	return raw
}

func sharedEventsDropReportInput(t *testing.T, root string) EventDropReportInput {
	t.Helper()
	frame := sharedEventsFrameMap(t, root, "event.directory-drop-report.v4.json")
	cursor := sharedEventsCursorFromFrame(t, frame)
	return EventDropReportInput{
		Cursor:           cursor,
		OccurredUnixMS:   int64(frame["occurred_unix_ms"].(float64)),
		DroppedCount:     int(frame["dropped_count"].(float64)),
		ReconnectAfterMS: sharedEventsOptionalInt(frame["reconnect_after_ms"]),
		Reason:           frame["metadata"].(map[string]any)["reason"].(string),
		EventID:          frame["event_id"].(string),
		ResumeToken:      frame["resume_token"].(string),
		TenantRef:        frame["tenant_ref"],
	}
}

func sharedEventsDropReportInputJSON(t *testing.T, root string) []byte {
	t.Helper()
	raw, err := json.Marshal(sharedEventsDropReportInput(t, root))
	if err != nil {
		t.Fatalf("marshal shared events drop report input: %v", err)
	}
	return raw
}

func sharedEventsTerminalInput(t *testing.T, root string) EventTerminalInput {
	t.Helper()
	frame := sharedEventsFrameMap(t, root, "event.directory-terminal.v4.json")
	cursor := sharedEventsCursorFromFrame(t, frame)
	return EventTerminalInput{
		Cursor:           cursor,
		OccurredUnixMS:   int64(frame["occurred_unix_ms"].(float64)),
		ReconnectAfterMS: sharedEventsOptionalInt(frame["reconnect_after_ms"]),
		Reason:           frame["metadata"].(map[string]any)["reason"].(string),
		EventID:          frame["event_id"].(string),
		ResumeToken:      frame["resume_token"].(string),
		TenantRef:        frame["tenant_ref"],
	}
}

func sharedEventsTerminalInputJSON(t *testing.T, root string) []byte {
	t.Helper()
	raw, err := json.Marshal(sharedEventsTerminalInput(t, root))
	if err != nil {
		t.Fatalf("marshal shared events terminal input: %v", err)
	}
	return raw
}

func sharedEventsFrameWithoutCursorToken(t *testing.T, root string, fixture string) []byte {
	t.Helper()
	frame := sharedEventsFrameMap(t, root, fixture)
	cursor := frame["cursor"].(map[string]any)
	delete(cursor, "token")
	raw, err := json.Marshal(frame)
	if err != nil {
		t.Fatalf("encode event frame without cursor token: %v", err)
	}
	return raw
}

func sharedEventsDropReportWithoutDroppedCount(t *testing.T, root string) []byte {
	t.Helper()
	frame := sharedEventsFrameMap(t, root, "event.directory-drop-report.v4.json")
	frame["dropped_count"] = float64(0)
	raw, err := json.Marshal(frame)
	if err != nil {
		t.Fatalf("encode drop report without dropped count: %v", err)
	}
	return raw
}

func sharedEventsTerminalWithoutTerminalFlag(t *testing.T, root string) []byte {
	t.Helper()
	frame := sharedEventsFrameMap(t, root, "event.directory-terminal.v4.json")
	frame["terminal"] = false
	raw, err := json.Marshal(frame)
	if err != nil {
		t.Fatalf("encode terminal frame without terminal flag: %v", err)
	}
	return raw
}

func sharedEventsFrameMap(t *testing.T, root string, fixture string) map[string]any {
	t.Helper()
	frame := map[string]any{}
	if err := json.Unmarshal(sharedFixture(t, root, fixture), &frame); err != nil {
		t.Fatalf("decode shared events frame fixture %s: %v", fixture, err)
	}
	return frame
}

func sharedEventsCursorFromFrame(t *testing.T, frame map[string]any) EventCursor {
	t.Helper()
	cursor := frame["cursor"].(map[string]any)
	return EventCursor{
		Stream:   cursor["stream"].(string),
		Sequence: uint64(cursor["sequence"].(float64)),
	}
}

func sharedEventsOptionalInt(value any) *int {
	if value == nil {
		return nil
	}
	converted := int(value.(float64))
	return &converted
}

func sharedSurfaceListPagesRequest(t *testing.T, root string) SurfaceListPagesRequest {
	t.Helper()
	var request SurfaceListPagesRequest
	if err := json.Unmarshal(sharedFixture(t, root, "surface-list-pages-request.v4.json"), &request); err != nil {
		t.Fatalf("decode shared surface list-pages request: %v", err)
	}
	return request
}

func sharedSurfaceCreatePageRequest(t *testing.T, root string) SurfaceCreatePageRequest {
	t.Helper()
	var request SurfaceCreatePageRequest
	if err := json.Unmarshal(sharedFixture(t, root, "surface-create-page-request.v4.json"), &request); err != nil {
		t.Fatalf("decode shared surface create-page request: %v", err)
	}
	return request
}

func sharedSurfaceDeletePageRequest(t *testing.T, root string) SurfaceDeletePageRequest {
	t.Helper()
	var request SurfaceDeletePageRequest
	if err := json.Unmarshal(sharedFixture(t, root, "surface-delete-page-request.v4.json"), &request); err != nil {
		t.Fatalf("decode shared surface delete-page request: %v", err)
	}
	return request
}

func sharedSurfaceManifestRequest(t *testing.T, root string) SurfaceManifestRequest {
	t.Helper()
	var request SurfaceManifestRequest
	if err := json.Unmarshal(sharedFixture(t, root, "surface-manifest-request.v4.json"), &request); err != nil {
		t.Fatalf("decode shared surface manifest request: %v", err)
	}
	return request
}

func sharedSurfaceHealthRequest(t *testing.T, root string) SurfaceHealthRequest {
	t.Helper()
	var request SurfaceHealthRequest
	if err := json.Unmarshal(sharedFixture(t, root, "surface-health-request.v4.json"), &request); err != nil {
		t.Fatalf("decode shared surface health request: %v", err)
	}
	return request
}

func sharedSurfacePagePageWithOversizedLimit(t *testing.T, root string) []byte {
	t.Helper()
	page := map[string]any{}
	if err := json.Unmarshal(sharedFixture(t, root, "surface-page-page.v4.json"), &page); err != nil {
		t.Fatalf("decode shared surface page page: %v", err)
	}
	page["limit"] = float64(MaxSurfacePageSize + 1)
	raw, err := json.Marshal(page)
	if err != nil {
		t.Fatalf("encode oversized surface page page: %v", err)
	}
	return raw
}

func sharedSurfaceManifestWithoutEntrypoint(t *testing.T, root string) []byte {
	t.Helper()
	manifest := map[string]any{}
	if err := json.Unmarshal(sharedFixture(t, root, "surface-manifest.v4.json"), &manifest); err != nil {
		t.Fatalf("decode shared surface manifest: %v", err)
	}
	delete(manifest, "entrypoint")
	raw, err := json.Marshal(manifest)
	if err != nil {
		t.Fatalf("encode surface manifest without entrypoint: %v", err)
	}
	return raw
}

func sharedCompatibilityListModelsRequest(t *testing.T, root string) CompatibilityListModelsRequest {
	t.Helper()
	var request CompatibilityListModelsRequest
	if err := json.Unmarshal(sharedFixture(t, root, "compatibility-list-models-request.v4.json"), &request); err != nil {
		t.Fatalf("decode shared compatibility list-models request: %v", err)
	}
	return request
}

func sharedCompatibilityChatCompletionRequest(t *testing.T, root string) CompatibilityChatCompletionRequest {
	t.Helper()
	var request CompatibilityChatCompletionRequest
	if err := json.Unmarshal(sharedFixture(t, root, "compatibility-chat-completion-request.v4.json"), &request); err != nil {
		t.Fatalf("decode shared compatibility chat-completion request: %v", err)
	}
	return request
}

func sharedCompatibilityStreamChatCompletionRequest(t *testing.T, root string) CompatibilityStreamChatCompletionRequest {
	t.Helper()
	var request CompatibilityStreamChatCompletionRequest
	if err := json.Unmarshal(sharedFixture(t, root, "compatibility-stream-chat-completion-request.v4.json"), &request); err != nil {
		t.Fatalf("decode shared compatibility stream-chat-completion request: %v", err)
	}
	return request
}

func sharedCompatibilityFileUploadRequest(t *testing.T, root string) CompatibilityFileUploadRequest {
	t.Helper()
	var request CompatibilityFileUploadRequest
	if err := json.Unmarshal(sharedFixture(t, root, "compatibility-file-upload-request.v4.json"), &request); err != nil {
		t.Fatalf("decode shared compatibility file-upload request: %v", err)
	}
	return request
}

func sharedCompatibilityFileRequest(t *testing.T, root string) CompatibilityFileRequest {
	t.Helper()
	var request CompatibilityFileRequest
	if err := json.Unmarshal(sharedFixture(t, root, "compatibility-file-request.v4.json"), &request); err != nil {
		t.Fatalf("decode shared compatibility file request: %v", err)
	}
	return request
}

func sharedCompatibilityFileDeleteRequest(t *testing.T, root string) CompatibilityFileDeleteRequest {
	t.Helper()
	var request CompatibilityFileDeleteRequest
	if err := json.Unmarshal(sharedFixture(t, root, "compatibility-file-delete-request.v4.json"), &request); err != nil {
		t.Fatalf("decode shared compatibility file-delete request: %v", err)
	}
	return request
}

func sharedCompatibilityStreamRequestJSON(t *testing.T, requestJSON []byte) []byte {
	t.Helper()
	request := map[string]any{}
	if err := json.Unmarshal(requestJSON, &request); err != nil {
		t.Fatalf("decode shared compatibility stream request: %v", err)
	}
	chatRequest := request["request"].(map[string]any)
	chatRequest["stream"] = true
	raw, err := json.Marshal(request)
	if err != nil {
		t.Fatalf("encode shared compatibility stream request: %v", err)
	}
	return raw
}

func sharedFeatureDiscoveryJSON(t *testing.T, abiVersion uint32) []byte {
	t.Helper()
	featureDiscovery := map[string]any{}
	if err := json.Unmarshal(sharedFixture(t, repositoryRoot(t), "feature-discovery.v4.json"), &featureDiscovery); err != nil {
		t.Fatalf("decode shared feature discovery fixture: %v", err)
	}
	featureDiscovery["abi_version"] = abiVersion
	raw, err := json.Marshal(featureDiscovery)
	if err != nil {
		t.Fatalf("encode shared feature discovery: %v", err)
	}
	return raw
}

func sharedInvocationDraft(t *testing.T, root string) InvocationDraft {
	t.Helper()
	draft, err := NewInvocationDraftFromJSON(sharedFixture(t, root, "invocation.complete.v4.json"))
	if err != nil {
		t.Fatalf("decode shared invocation fixture: %v", err)
	}
	return draft
}

func sharedInvocationBuilder(t *testing.T, root string) *InvocationBuilder {
	t.Helper()
	draft := sharedInvocationDraft(t, root)
	builder := NewInvocationBuilder().
		WithCallerURA(draft.CallerURA()).
		WithCalleeURA(draft.CalleeURA()).
		WithDescriptorRef(draft.DescriptorRef()).
		WithSubjectURA(draft.SubjectURA()).
		WithNonceBase64(draft.NonceBase64()).
		WithCausalContext(draft.CausalContext()).
		WithContentType(draft.ContentType()).
		WithMetadata(draft.Metadata())
	if draft.HasJSONArgs() {
		builder.WithJSONArgs(draft.JSONArgs())
	} else {
		builder.WithArgumentsBase64(draft.ArgumentsBase64())
	}
	if signature := draft.CallerSignature(); signature != nil {
		builder.WithCallerSignature(*signature)
	}
	return builder
}

func sharedInvocationSignature() InvocationSignature {
	return InvocationSignature{
		Algorithm:       "ed25519",
		SignatureBase64: "c2lnbmF0dXJl",
		KeyIDHint:       "caller-key",
	}
}

func sharedControlOnlyHealthJSON(t *testing.T, root string) []byte {
	t.Helper()
	health := map[string]any{}
	if err := json.Unmarshal(sharedFixture(t, root, "health.ready.v4.json"), &health); err != nil {
		t.Fatalf("decode shared health fixture: %v", err)
	}
	health["invocation_ready"] = false
	health["runtime_ready"] = false
	health["diagnostics"] = []any{"invocation endpoint unavailable"}
	raw, err := json.Marshal(health)
	if err != nil {
		t.Fatalf("encode shared control-only health: %v", err)
	}
	return raw
}

func sharedDeviceQuery(t *testing.T, root string) DeviceQuery {
	t.Helper()
	return DeviceQuery{DirectoryQueryBase: sharedDirectoryQueryBase(t, root, "directory-list-devices-request.v4.json", 0)}
}

func sharedAgentQuery(t *testing.T, root string) AgentQuery {
	t.Helper()
	return AgentQuery{DirectoryQueryBase: sharedDirectoryQueryBase(t, root, "directory-list-agents-request.v4.json", 0)}
}

func sharedAbilityQuery(t *testing.T, root string) AbilityQuery {
	t.Helper()
	var query AbilityQuery
	if err := json.Unmarshal(sharedFixture(t, root, "directory-list-abilities-request.v4.json"), &query); err != nil {
		t.Fatalf("decode shared ability query fixture: %v", err)
	}
	return query
}

func sharedResolveQuery(t *testing.T, root string) ResolveQuery {
	t.Helper()
	var query ResolveQuery
	if err := json.Unmarshal(sharedFixture(t, root, "directory-resolve-request.v4.json"), &query); err != nil {
		t.Fatalf("decode shared resolve query fixture: %v", err)
	}
	return query
}

func sharedDirectorySubscriptionRequest(t *testing.T, root string) DirectorySubscriptionRequest {
	t.Helper()
	var request DirectorySubscriptionRequest
	if err := json.Unmarshal(sharedFixture(t, root, "directory-subscription-request.v4.json"), &request); err != nil {
		t.Fatalf("decode shared directory subscription request fixture: %v", err)
	}
	return request
}

func sharedDirectoryQueryBase(t *testing.T, root string, fixture string, overrideLimit int) DirectoryQueryBase {
	t.Helper()
	var query DirectoryQueryBase
	if err := json.Unmarshal(sharedFixture(t, root, fixture), &query); err != nil {
		t.Fatalf("decode shared directory query fixture %s: %v", fixture, err)
	}
	if overrideLimit != 0 {
		query.Limit = overrideLimit
	}
	return query
}

func assertJSONEquivalent(t *testing.T, actual []byte, expected []byte) {
	t.Helper()
	var actualValue any
	if err := json.Unmarshal(actual, &actualValue); err != nil {
		t.Fatalf("decode actual JSON: %v", err)
	}
	var expectedValue any
	if err := json.Unmarshal(expected, &expectedValue); err != nil {
		t.Fatalf("decode expected JSON: %v", err)
	}
	if !reflect.DeepEqual(actualValue, expectedValue) {
		t.Fatalf("JSON mismatch\nactual: %s\nexpected: %s", actual, expected)
	}
}
