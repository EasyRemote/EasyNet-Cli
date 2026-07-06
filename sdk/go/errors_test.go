package easynet

import "testing"

func TestDecodeDaemonErrorJSONDecodesFixtureShape(t *testing.T) {
	err, decodeErr := DecodeDaemonErrorJSON([]byte(`{
		"code": "InvalidArgument",
		"stage": "prepare",
		"message": "missing caller_ura",
		"retry": "never",
		"source": "sdk",
		"invocation_id": null,
		"receipt_ura": null,
		"details": {}
	}`))
	if decodeErr != nil {
		t.Fatalf("DecodeDaemonErrorJSON: %v", decodeErr)
	}
	if err == nil {
		t.Fatalf("DecodeDaemonErrorJSON returned nil")
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

func TestDecodeDaemonErrorJSONPreservesRuntimeRefsAndRetryability(t *testing.T) {
	err, decodeErr := DecodeDaemonErrorJSON([]byte(`{
		"code": "TIMEOUT",
		"stage": "transport",
		"message": "deadline elapsed",
		"retry": "safe",
		"source": "c_abi",
		"invocation_id": "inv-1",
		"receipt_ura": "easynet:///r/example/receipt/opaque",
		"details": {"abi_symbol": "ERR_TIMEOUT"}
	}`))
	if decodeErr != nil {
		t.Fatalf("DecodeDaemonErrorJSON: %v", decodeErr)
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
	if err.ReceiptURA != "easynet:///r/example/receipt/opaque" {
		t.Fatalf("receipt URA = %q", err.ReceiptURA)
	}
	if err.Details["abi_symbol"] != "ERR_TIMEOUT" {
		t.Fatalf("details not preserved: %#v", err.Details)
	}
	if err.Class() != ErrorClassTimeout {
		t.Fatalf("class = %s, want %s", err.Class(), ErrorClassTimeout)
	}
}

func TestNormalizeErrorCodeCanonicalizesLegacyWireAliases(t *testing.T) {
	cases := map[string]ErrorCode{
		"DAEMON_DOWN":          ErrDaemonOffline,
		"VERSION_INCOMPATIBLE": ErrVersionMismatch,
		"ABILITY_FAILED":       ErrAdmissionDenied,
		"NOT_FOUND":            ErrAbilityNotFound,
		"PROTOCOL":             ErrProtocolMismatch,
		"TRANSPORT":            ErrRouteUnavailable,
		"DAEMON_OFFLINE":       ErrDaemonOffline,
		"VERSION_MISMATCH":     ErrVersionMismatch,
		"ADMISSION_DENIED":     ErrAdmissionDenied,
		"ABILITY_NOT_FOUND":    ErrAbilityNotFound,
		"PROTOCOL_MISMATCH":    ErrProtocolMismatch,
		"ROUTE_UNAVAILABLE":    ErrRouteUnavailable,
	}
	for input, want := range cases {
		if got := NormalizeErrorCode(input); got != want {
			t.Fatalf("NormalizeErrorCode(%q) = %s, want %s", input, got, want)
		}
	}
}

func TestErrorClassForCodeProjectsStableClasses(t *testing.T) {
	cases := map[ErrorCode]ErrorClass{
		ErrInvalidArgument:  ErrorClassValidation,
		ErrInvalidHandle:    ErrorClassHandle,
		ErrNotInitialized:   ErrorClassLifecycle,
		ErrDaemonOffline:    ErrorClassAvailability,
		ErrPermissionDenied: ErrorClassPermission,
		ErrAdmissionDenied:  ErrorClassAdmission,
		ErrAbilityNotFound:  ErrorClassRouting,
		ErrTimeout:          ErrorClassTimeout,
		ErrCancelled:        ErrorClassCancellation,
		ErrProtocolMismatch: ErrorClassProtocol,
		ErrVersionMismatch:  ErrorClassVersion,
		ErrControlOnly:      ErrorClassControl,
		ErrNotImplemented:   ErrorClassUnsupported,
		ErrGeneric:          ErrorClassGeneric,
	}
	for code, want := range cases {
		if got := ErrorClassForCode(code); got != want {
			t.Fatalf("ErrorClassForCode(%s) = %s, want %s", code, got, want)
		}
	}
}

func TestIsCodeMatchesCanonicalizedLegacyRequests(t *testing.T) {
	err := &SDKError{Code: ErrRouteUnavailable}
	if !IsCode(err, ErrTransport) {
		t.Fatalf("IsCode did not match legacy transport request")
	}
	if !IsCode(err, ErrRouteUnavailable) {
		t.Fatalf("IsCode did not match canonical route-unavailable request")
	}
}

func TestDecodeDaemonErrorJSONRejectsInvalidRetryHint(t *testing.T) {
	_, err := DecodeDaemonErrorJSON([]byte(`{
		"code": "TIMEOUT",
		"stage": "transport",
		"message": "deadline elapsed",
		"retry": "maybe",
		"details": {}
	}`))
	if err == nil {
		t.Fatalf("DecodeDaemonErrorJSON succeeded, want invalid argument")
	}
	if !IsCode(err, ErrInvalidArgument) {
		t.Fatalf("error code = %v, want %s", err, ErrInvalidArgument)
	}
}

func TestDecodeDaemonErrorJSONNullIsNoError(t *testing.T) {
	err, decodeErr := DecodeDaemonErrorJSON([]byte(`null`))
	if decodeErr != nil {
		t.Fatalf("DecodeDaemonErrorJSON: %v", decodeErr)
	}
	if err != nil {
		t.Fatalf("err = %#v, want nil", err)
	}
}

func TestProfileErrorDetailsAddsStableProfileRefs(t *testing.T) {
	details := profileErrorDetails("publication", map[string]any{
		"reason": "resource_ref_namespace_reserved",
	})

	if details["profile"] != "publication" {
		t.Fatalf("profile detail = %#v, want publication", details["profile"])
	}
	if details["source_ref"] != "go_sdk.profile.publication" {
		t.Fatalf("source_ref detail = %#v", details["source_ref"])
	}
	if details["reason"] != "resource_ref_namespace_reserved" {
		t.Fatalf("reason detail not preserved: %#v", details)
	}
}

func TestProfileErrorDetailsPreservesCallerRefs(t *testing.T) {
	details := profileErrorDetails("mission", map[string]any{
		"profile":    "custom",
		"source_ref": "custom.source",
		"operation":  "run_file",
	})

	if details["profile"] != "custom" {
		t.Fatalf("profile detail overwritten: %#v", details)
	}
	if details["source_ref"] != "custom.source" {
		t.Fatalf("source_ref detail overwritten: %#v", details)
	}
	if details["operation"] != "run_file" {
		t.Fatalf("operation detail not preserved: %#v", details)
	}
}

func TestSDKErrorProfileAndSourceRefAccessors(t *testing.T) {
	err := &SDKError{
		Code: ErrInvalidArgument,
		Details: profileErrorDetails("publication", map[string]any{
			"reason": "resource_ref_namespace_reserved",
		}),
	}

	if err.Profile() != "publication" {
		t.Fatalf("profile = %q", err.Profile())
	}
	if err.SourceRef() != "go_sdk.profile.publication" {
		t.Fatalf("source ref = %q", err.SourceRef())
	}
	if err.Class() != ErrorClassValidation {
		t.Fatalf("class = %s", err.Class())
	}
	if ProfileSourceRef(" publication ") != "go_sdk.profile.publication" {
		t.Fatalf("profile source helper mismatch")
	}
}
