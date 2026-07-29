package easynet

import (
	"strings"
	"testing"
)

func TestDecodeTransportErrorJSONDecodesFixtureShape(t *testing.T) {
	err, decodeErr := decodeRuntimeErrorJSON([]byte(`{
		"code": "INVALID_ARGUMENT",
		"stage": "prepare",
		"message": "missing caller_ura",
		"retry": "never",
		"source": "sdk",
		"invocation_id": null,
		"receipt_ura": null,
		"details": {}
	}`))
	if decodeErr != nil {
		t.Fatalf("decodeRuntimeErrorJSON: %v", decodeErr)
	}
	if err == nil {
		t.Fatalf("decodeRuntimeErrorJSON returned nil")
	}
	if err.Code != ErrInvalidArgument {
		t.Fatalf("code = %s, want %s", err.Code, ErrInvalidArgument)
	}
	if err.Retryable {
		t.Fatalf("Retryable = true, want false")
	}
	if err.Source != "sdk" {
		t.Fatalf("source = %q, want sdk", err.Source)
	}
}

func TestDecodeTransportErrorJSONPreservesRuntimeRefsAndRetryability(t *testing.T) {
	err, decodeErr := decodeRuntimeErrorJSON([]byte(`{
		"code": "TIMEOUT",
		"stage": "transport",
		"message": "deadline elapsed",
		"retry": "safe",
		"source": "c_abi",
		"invocation_id": "inv-1",
		"receipt_ura": "easynet:///r/example/resource/agent.alice.sdk/invocation/opaque/receipt",
		"details": {"abi_symbol": "ERR_TIMEOUT"}
	}`))
	if decodeErr != nil {
		t.Fatalf("decodeRuntimeErrorJSON: %v", decodeErr)
	}
	if err.Code != ErrTimeout {
		t.Fatalf("code = %s, want %s", err.Code, ErrTimeout)
	}
	if !err.Retryable {
		t.Fatalf("Retryable = false, want true")
	}
	if err.InvocationID != "inv-1" {
		t.Fatalf("invocation id = %q, want inv-1", err.InvocationID)
	}
	if err.ReceiptURA != "easynet:///r/example/resource/agent.alice.sdk/invocation/opaque/receipt" {
		t.Fatalf("receipt URA = %q", err.ReceiptURA)
	}
	if err.Details["abi_symbol"] != "ERR_TIMEOUT" {
		t.Fatalf("details not preserved: %#v", err.Details)
	}
	if err.Class() != ErrorClassTimeout {
		t.Fatalf("class = %s, want %s", err.Class(), ErrorClassTimeout)
	}
}

func TestDecodeTransportErrorJSONCanonicalizesCallerSignerCustodyDetail(t *testing.T) {
	err, decodeErr := decodeRuntimeErrorJSON([]byte(
		"{\n" +
			`"code":"CALLER_SIGNER_UNAVAILABLE",` + "\n" +
			`"stage":"caller_identity",` + "\n" +
			`"message":"easynet_runtime_resolve_descriptor_ref: remote invocation requires a caller signer for ` +
			"`easynet:///r/localhost/user/alice`" +
			`; load or provision that identity in the local key service: self-identity: keyring rejected request: kind=not_found, msg=keyring entry not found: easynet:///r/localhost/user/alice",` + "\n" +
			`"retry":"never",` + "\n" +
			`"source":"c_abi",` + "\n" +
			`"invocation_id":"inv-1",` + "\n" +
			`"receipt_ura":null,` + "\n" +
			`"details":{"abi_symbol":"ERR_PERMISSION_DENIED"}` + "\n" +
			"}",
	))
	if decodeErr != nil {
		t.Fatalf("decodeRuntimeErrorJSON: %v", decodeErr)
	}
	if err == nil {
		t.Fatalf("decodeRuntimeErrorJSON returned nil")
	}
	if err.Code != ErrCallerSignerUnavailable {
		t.Fatalf("code = %s, want %s", err.Code, ErrCallerSignerUnavailable)
	}
	if err.Message != "CALLER_SIGNER_UNAVAILABLE: remote invocation requires a caller signer for `easynet:///r/localhost/user/alice`; load or provision that identity in the local key service" {
		t.Fatalf("message = %q", err.Message)
	}
	for _, leaked := range []string{"keyring entry not found", "keyring rejected request", "self-identity:"} {
		if strings.Contains(err.Message, leaked) {
			t.Fatalf("caller signer message leaked %q: %s", leaked, err.Message)
		}
	}
	if err.Stage != "caller_identity" || err.Source != "c_abi" || err.InvocationID != "inv-1" {
		t.Fatalf("structured error facts changed: %#v", err)
	}
	if err.Details["abi_symbol"] != "ERR_PERMISSION_DENIED" {
		t.Fatalf("details not preserved: %#v", err.Details)
	}
}

func TestDecodeTransportErrorJSONAcceptsTypedDescriptorOwnerOffline(t *testing.T) {
	err, decodeErr := decodeRuntimeErrorJSON([]byte(`{
		"code": "DESCRIPTOR_OWNER_OFFLINE",
		"stage": "runtime",
		"message": "DESCRIPTOR_OWNER_OFFLINE: descriptor owner is not online",
		"retry": "safe",
		"source": "c_abi",
		"invocation_id": null,
		"receipt_ura": null,
		"details": {}
	}`))
	if decodeErr != nil {
		t.Fatalf("decodeRuntimeErrorJSON: %v", decodeErr)
	}
	if err == nil {
		t.Fatalf("decodeRuntimeErrorJSON returned nil")
	}
	if err.Code != ErrDescriptorOwnerOffline {
		t.Fatalf("code = %s, want %s", err.Code, ErrDescriptorOwnerOffline)
	}
	if err.Message != "DESCRIPTOR_OWNER_OFFLINE: descriptor owner is not online" {
		t.Fatalf("message = %q", err.Message)
	}
	if err.Class() != ErrorClassRouting {
		t.Fatalf("class = %s, want %s", err.Class(), ErrorClassRouting)
	}
}

func TestDecodeTransportErrorJSONDoesNotInferOwnerOfflineFromRouteText(t *testing.T) {
	err, decodeErr := decodeRuntimeErrorJSON([]byte(`{
		"code": "ABILITY_NOT_FOUND",
		"stage": "runtime",
		"message": "invocation.history.list failed: ROUTE_NEGATIVE: namespace.resolve negative for ` + "`" + `easynet:///r/localhost/ability/device.dev-a.invocation.history.list` + "`" + `: NEGATIVE_REASON_NXDOMAIN: owner is not online",
		"retry": "never",
		"source": "c_abi",
		"invocation_id": null,
		"receipt_ura": null,
		"details": {}
	}`))
	if decodeErr != nil {
		t.Fatalf("decodeRuntimeErrorJSON: %v", decodeErr)
	}
	if err == nil {
		t.Fatalf("decodeRuntimeErrorJSON returned nil")
	}
	if err.Code != ErrAbilityNotFound {
		t.Fatalf("code = %s, want %s", err.Code, ErrAbilityNotFound)
	}
	if err.Message == "DESCRIPTOR_OWNER_OFFLINE: descriptor owner is not online" {
		t.Fatalf("message was rewritten from diagnostic text: %q", err.Message)
	}
}

func TestParseErrorCodeAcceptsOnlyCanonicalSchemaValues(t *testing.T) {
	cases := map[string]ErrorCode{
		"RUNTIME_OFFLINE":      ErrRuntimeOffline,
		"VERSION_MISMATCH":     ErrVersionMismatch,
		"VERSION_INCOMPATIBLE": ErrVersionIncompatible,
		"ADMISSION_DENIED":     ErrAdmissionDenied,
		"HTTP_AUTH_DENIED":     ErrHTTPAuthDenied,
		"SIGNATURE_DENIED":     ErrSignatureDenied,
		"POLICY_DENIED":        ErrPolicyDenied,
		"AUTHORITY_DENIED":     ErrAuthorityDenied,
		"ABILITY_NOT_FOUND":    ErrAbilityNotFound,
		"PROTOCOL_MISMATCH":    ErrProtocolMismatch,
		"ROUTE_UNAVAILABLE":    ErrRouteUnavailable,
		"EXECUTION_FAILED":     ErrExecutionFailed,
		"TRANSPORT":            ErrTransport,
	}
	for input, want := range cases {
		got, err := ParseErrorCode(input)
		if err != nil {
			t.Fatalf("ParseErrorCode(%q): %v", input, err)
		}
		if got != want {
			t.Fatalf("ParseErrorCode(%q) = %s, want %s", input, got, want)
		}
	}
}

func TestDecodeTransportErrorJSONRejectsLegacyCodeAliases(t *testing.T) {
	for _, code := range []string{"InvalidArgument", "DaemonDown", "DAEMON_DOWN", "DAEMON_OFFLINE", "VersionIncompatible"} {
		_, err := decodeRuntimeErrorJSON([]byte(`{
			"code": "` + code + `",
			"stage": "runtime",
			"message": "legacy code",
			"retry": "never",
			"details": {}
		}`))
		if err == nil {
			t.Fatalf("decodeRuntimeErrorJSON accepted legacy code %q", code)
		}
		if !IsCode(err, ErrInvalidArgument) {
			t.Fatalf("legacy code %q error = %v, want %s", code, err, ErrInvalidArgument)
		}
	}
}

func TestRuntimeFailureCodePreservesDomainCodesAndRejectsLegacyAliases(t *testing.T) {
	cases := map[string]ErrorCode{
		"":                                ErrProtocolMismatch,
		"   ":                             ErrProtocolMismatch,
		"TRANSPORT":                       ErrTransport,
		"AXON_MEMBERSHIP_REQUIRED":        ErrorCode("AXON_MEMBERSHIP_REQUIRED"),
		"TARGET_NOT_IN_PRESENCE_REGISTRY": ErrorCode("TARGET_NOT_IN_PRESENCE_REGISTRY"),
		"InvalidArgument":                 ErrProtocolMismatch,
		"DAEMON_DOWN":                     ErrProtocolMismatch,
		"DAEMON_OFFLINE":                  ErrProtocolMismatch,
	}
	for input, want := range cases {
		if got := runtimeFailureCode(input); got != want {
			t.Fatalf("runtimeFailureCode(%q) = %s, want %s", input, got, want)
		}
	}
}

func TestErrorClassForCodeProjectsStableClasses(t *testing.T) {
	cases := map[ErrorCode]ErrorClass{
		ErrInvalidArgument:     ErrorClassValidation,
		ErrInvalidHandle:       ErrorClassHandle,
		ErrNotInitialized:      ErrorClassLifecycle,
		ErrRuntimeOffline:      ErrorClassAvailability,
		ErrTransport:           ErrorClassAvailability,
		ErrPermissionDenied:    ErrorClassPermission,
		ErrHTTPAuthDenied:      ErrorClassPermission,
		ErrAdmissionDenied:     ErrorClassAdmission,
		ErrSignatureDenied:     ErrorClassAdmission,
		ErrPolicyDenied:        ErrorClassAdmission,
		ErrAuthorityDenied:     ErrorClassAdmission,
		ErrExecutionFailed:     ErrorClassAdmission,
		ErrAbilityFailed:       ErrorClassAdmission,
		ErrAbilityNotFound:     ErrorClassRouting,
		ErrNotFound:            ErrorClassRouting,
		ErrTimeout:             ErrorClassTimeout,
		ErrCancelled:           ErrorClassCancellation,
		ErrProtocolMismatch:    ErrorClassProtocol,
		ErrProtocol:            ErrorClassProtocol,
		ErrVersionMismatch:     ErrorClassVersion,
		ErrVersionIncompatible: ErrorClassVersion,
		ErrControlOnly:         ErrorClassControl,
		ErrNotImplemented:      ErrorClassUnsupported,
		ErrGeneric:             ErrorClassGeneric,
	}
	for code, want := range cases {
		if got := ErrorClassForCode(code); got != want {
			t.Fatalf("ErrorClassForCode(%s) = %s, want %s", code, got, want)
		}
	}
}

func TestIsCodeMatchesExactCanonicalRequests(t *testing.T) {
	err := &SDKError{Code: ErrRouteUnavailable}
	if IsCode(err, ErrTransport) {
		t.Fatalf("IsCode matched a different canonical code")
	}
	if !IsCode(err, ErrRouteUnavailable) {
		t.Fatalf("IsCode did not match canonical route-unavailable request")
	}
}

func TestDecodeTransportErrorJSONRejectsInvalidRetryHint(t *testing.T) {
	_, err := decodeRuntimeErrorJSON([]byte(`{
		"code": "TIMEOUT",
		"stage": "transport",
		"message": "deadline elapsed",
		"retry": "maybe",
		"details": {}
	}`))
	if err == nil {
		t.Fatalf("decodeRuntimeErrorJSON succeeded, want invalid argument")
	}
	if !IsCode(err, ErrInvalidArgument) {
		t.Fatalf("error code = %v, want %s", err, ErrInvalidArgument)
	}
}

func TestDecodeTransportErrorJSONNullIsNoError(t *testing.T) {
	err, decodeErr := decodeRuntimeErrorJSON([]byte(`null`))
	if decodeErr != nil {
		t.Fatalf("decodeRuntimeErrorJSON: %v", decodeErr)
	}
	if err != nil {
		t.Fatalf("err = %#v, want nil", err)
	}
}

func TestProfileErrorDetailsAddsStableProfileRefs(t *testing.T) {
	details := profileErrorDetails("directory", map[string]any{
		"reason": "canonical_projection_rejected",
	})

	if details["profile"] != "directory" {
		t.Fatalf("profile detail = %#v, want directory", details["profile"])
	}
	if details["source_ref"] != "go_sdk.profile.directory" {
		t.Fatalf("source_ref detail = %#v", details["source_ref"])
	}
	if details["reason"] != "canonical_projection_rejected" {
		t.Fatalf("reason detail not preserved: %#v", details)
	}
}

func TestProfileErrorDetailsPreservesCallerRefs(t *testing.T) {
	details := profileErrorDetails("authority", map[string]any{
		"profile":    "custom",
		"source_ref": "custom.source",
		"operation":  "mint_session_authority",
	})

	if details["profile"] != "custom" {
		t.Fatalf("profile detail overwritten: %#v", details)
	}
	if details["source_ref"] != "custom.source" {
		t.Fatalf("source_ref detail overwritten: %#v", details)
	}
	if details["operation"] != "mint_session_authority" {
		t.Fatalf("operation detail not preserved: %#v", details)
	}
}

func TestSDKErrorProfileAndSourceRefAccessors(t *testing.T) {
	err := &SDKError{
		Code: ErrInvalidArgument,
		Details: profileErrorDetails("directory", map[string]any{
			"reason": "canonical_projection_rejected",
		}),
	}

	if err.Profile() != "directory" {
		t.Fatalf("profile = %q", err.Profile())
	}
	if err.SourceRef() != "go_sdk.profile.directory" {
		t.Fatalf("source ref = %q", err.SourceRef())
	}
	if err.Class() != ErrorClassValidation {
		t.Fatalf("class = %s", err.Class())
	}
	if ProfileSourceRef(" directory ") != "go_sdk.profile.directory" {
		t.Fatalf("profile source helper mismatch")
	}
}
