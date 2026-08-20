package easynet

import (
	"context"
	"encoding/base64"
	"encoding/json"
	"errors"
	"strings"
	"testing"
)

func TestAuthorizedRuntimeSessionRejectsAuthoritySubjectMismatchBeforeDispatch(t *testing.T) {
	session := newAuthorizedRuntimeSessionFixture(t)
	session.authorization.authority = sessionAuthorityFixture(t, map[string]any{
		"session_owner_user_id":      "bob",
		"subject_ura":                "easynet:///r/example/resource/user.bob/session/session-1",
		"scopes":                     []string{"invocation.history.list"},
		"allowed_followup_abilities": []string{"invocation.history.list"},
	})

	_, err := session.sdk.Invoke().Submit(context.Background(), canonicalSessionIntentFixture(), PrepareOptions{})
	if err == nil {
		t.Fatalf("expected authority subject mismatch")
	}
	if !IsCode(err, ErrAuthoritySubjectMismatch) {
		t.Fatalf("error = %v", err)
	}
	if session.runtime.prepareCalls != 0 || session.runtime.submitCalls != 0 {
		t.Fatalf("remote path attempted after mismatch: prepare=%d submit=%d", session.runtime.prepareCalls, session.runtime.submitCalls)
	}
}

func TestAuthorizedRuntimeSessionRejectsPathSubstringOwnerSubjectBeforeDispatch(t *testing.T) {
	session := newAuthorizedRuntimeSessionFixture(t)
	intent := canonicalSessionIntentFixture()
	intent.Subject = IntentSubjectRef{
		URA:            "easynet:///r/example/resource/device.dev-a/archive/resource/user.alice/session/session-1",
		DerivationRule: "fixture",
	}

	_, err := session.sdk.Invoke().Submit(context.Background(), intent, PrepareOptions{})
	if err == nil {
		t.Fatalf("expected authority subject mismatch")
	}
	if !IsCode(err, ErrAuthoritySubjectMismatch) {
		t.Fatalf("error = %v", err)
	}
	if session.runtime.prepareCalls != 0 || session.runtime.submitCalls != 0 {
		t.Fatalf("remote path attempted after path-substring subject: prepare=%d submit=%d", session.runtime.prepareCalls, session.runtime.submitCalls)
	}
}

func TestAuthorizedRuntimeSessionRejectsRetiredInvocationHistorySubjectExactAuthorityBeforeDispatch(t *testing.T) {
	session := newAuthorizedRuntimeSessionFixture(t)
	retiredSubject := "easynet:///r/example/resource/user.alice/session/invocation_history"
	session.authorization.authority = SessionAuthority{
		IssuerURA:                "easynet:///r/example/agent/backend",
		SessionID:                "invocation_history",
		SessionOwnerUserID:       "alice",
		SessionOwnerURA:          "easynet:///r/example/user/alice",
		CreatorPrincipalID:       "easynet:///r/example/agent/backend",
		CreatorPrincipalURA:      "easynet:///r/example/agent/backend",
		CalleeURA:                "easynet:///r/example/agent/device.dev-a.runtime-governance",
		SubjectURA:               retiredSubject,
		Audience:                 "easynet:///r/example/agent/device.dev-a.runtime-governance",
		Scopes:                   []string{"observe.health"},
		AllowedActions:           []string{"invoke"},
		AllowedFollowupAbilities: []string{"observe.health"},
		IssuedAtMS:               1000,
		ExpiresAtMS:              2000,
		Signature:                []byte("signature"),
	}
	intent := canonicalSessionIntentFixture()
	intent.Subject = IntentSubjectRef{
		URA:            retiredSubject,
		DerivationRule: "fixture",
	}

	_, err := session.sdk.Invoke().Submit(context.Background(), intent, PrepareOptions{})
	if err == nil {
		t.Fatalf("expected retired subject authority mismatch")
	}
	if !IsCode(err, ErrAuthoritySubjectMismatch) {
		t.Fatalf("error = %v", err)
	}
	if session.runtime.prepareCalls != 0 || session.runtime.submitCalls != 0 {
		t.Fatalf("remote path attempted after retired subject: prepare=%d submit=%d", session.runtime.prepareCalls, session.runtime.submitCalls)
	}
}

func TestAuthorizedRuntimeSessionRejectsMissingCallerIdentityBeforeDescriptor(t *testing.T) {
	session := newAuthorizedRuntimeSessionFixture(t)
	session.identity.caller = CallerIdentityRef{}
	intent := canonicalSessionIntentFixture()
	intent.CallerIdentity = CallerIdentityRef{}

	_, err := session.sdk.Abilities().Resolve(context.Background(), intent)
	if err == nil {
		t.Fatalf("expected missing caller identity")
	}
	if !IsCode(err, ErrCallerIdentityUnavailable) {
		t.Fatalf("error = %v", err)
	}
	if session.descriptor.calls != 0 {
		t.Fatalf("descriptor provider called after missing caller identity")
	}
}

func TestAuthorizedRuntimeSessionRejectsMissingCallerSignerBeforeSubmit(t *testing.T) {
	session := newAuthorizedRuntimeSessionFixture(t)
	session.signer.err = &SDKError{
		Code:      ErrCallerSignerUnavailable,
		Stage:     "sign",
		Retry:     RetryNever,
		Retryable: false,
		Message:   "no caller signer",
	}

	_, err := session.sdk.Invoke().Submit(context.Background(), canonicalSessionIntentFixture(), PrepareOptions{})
	if err == nil {
		t.Fatalf("expected missing signer")
	}
	if !IsCode(err, ErrCallerSignerUnavailable) {
		t.Fatalf("error = %v", err)
	}
	if session.runtime.prepareCalls != 1 || session.runtime.submitCalls != 0 {
		t.Fatalf("unexpected runtime calls: prepare=%d submit=%d", session.runtime.prepareCalls, session.runtime.submitCalls)
	}
}

func TestAuthorizedRuntimeSessionPrepareUsesDescriptorOwnerCalleeNotDeviceTarget(t *testing.T) {
	session := newAuthorizedRuntimeSessionFixture(t)
	intent := canonicalSessionIntentFixture()

	prepared, err := session.sdk.Prepare(context.Background(), intent)
	if err != nil {
		t.Fatalf("Prepare: %v", err)
	}
	if prepared.Intent.Target.URA != "easynet:///r/example/device/dev-a" {
		t.Fatalf("intent target = %q", prepared.Intent.Target.URA)
	}
	if prepared.Draft.CalleeURA() != "easynet:///r/example/agent/device.dev-a.runtime-governance" {
		t.Fatalf("draft callee = %q, want descriptor owner callee", prepared.Draft.CalleeURA())
	}
	if prepared.Draft.CalleeURA() == prepared.Intent.Target.URA {
		t.Fatalf("draft callee must not collapse to execution target")
	}
}

func TestAuthorizedRuntimeDescriptorResolutionRequiresDescriptorVocabulary(t *testing.T) {
	canonical := descriptorResolutionFromError(&SDKError{
		Code:      ErrDescriptorNotFound,
		Stage:     "descriptor",
		Retry:     RetryNever,
		Retryable: false,
		Message:   "descriptor missing",
	})
	if canonical.State != DescriptorNotFound {
		t.Fatalf("DESCRIPTOR_NOT_FOUND state = %s, want %s", canonical.State, DescriptorNotFound)
	}

	for _, tc := range []struct {
		name string
		code ErrorCode
	}{
		{name: "legacy ability not found", code: ErrAbilityNotFound},
		{name: "generic not found", code: ErrNotFound},
	} {
		t.Run(tc.name, func(t *testing.T) {
			resolution := descriptorResolutionFromError(&SDKError{
				Code:      tc.code,
				Stage:     "descriptor",
				Retry:     RetryNever,
				Retryable: false,
				Message:   "legacy provider not found",
			})
			if resolution.State != DescriptorUnavailable {
				t.Fatalf("%s state = %s, want %s", tc.code, resolution.State, DescriptorUnavailable)
			}
		})
	}
}

func TestAuthorizedRuntimeDescriptorResolutionRequiresTypedOwnerOffline(t *testing.T) {
	typed := descriptorResolutionFromError(&SDKError{
		Code:      ErrDescriptorOwnerOffline,
		Stage:     "descriptor",
		Retry:     RetryNever,
		Retryable: false,
		Message:   "owner is not online",
	})
	if typed.State != DescriptorOwnerOffline {
		t.Fatalf("DESCRIPTOR_OWNER_OFFLINE state = %s, want %s", typed.State, DescriptorOwnerOffline)
	}

	generic := descriptorResolutionFromError(&SDKError{
		Code:      ErrProviderUnavailable,
		Stage:     "descriptor",
		Retry:     RetryNever,
		Retryable: false,
		Message:   "owner is offline",
	})
	if generic.State != DescriptorUnavailable {
		t.Fatalf("generic offline text state = %s, want %s", generic.State, DescriptorUnavailable)
	}
}

func TestAuthorizedRuntimeSessionHistoryRejectsAuthoritySubjectMismatchBeforeReceiptProvider(t *testing.T) {
	session := newAuthorizedRuntimeSessionFixture(t)
	subject, err := RuntimeStateReadSubjectURA("example", "alice")
	if err != nil {
		t.Fatalf("runtime-state read subject: %v", err)
	}
	request := ReceiptListRequest{
		Call: RuntimeCallContext{
			CallerURA:     "easynet:///r/example/agent/backend",
			CalleeURA:     "easynet:///r/example/agent/device.dev-a.runtime-governance",
			SubjectURA:    subject,
			NonceBase64:   "AQIDBAUGBwgJCgsMDQ4PEA==",
			CausalContext: map[string]any{"form": "none"},
			Authority: sessionAuthorityFixture(t, map[string]any{
				"session_owner_user_id":      "bob",
				"subject_ura":                "easynet:///r/example/resource/user.bob/session/session-1",
				"scopes":                     []string{"invocation.history.list"},
				"allowed_followup_abilities": []string{"invocation.history.list"},
			}),
		},
		Limit: 10,
	}

	_, err = session.sdk.History().List(context.Background(), request)
	if err == nil {
		t.Fatalf("expected authority subject mismatch")
	}
	if !IsCode(err, ErrAuthoritySubjectMismatch) {
		t.Fatalf("error = %v", err)
	}
	if session.receipts.listCalls != 0 {
		t.Fatalf("receipt provider called after mismatch: %d", session.receipts.listCalls)
	}
}

func TestAuthorizedRuntimeSessionHistoryRejectsAllZeroSubjectBeforeReceiptProvider(t *testing.T) {
	session := newAuthorizedRuntimeSessionFixture(t)
	request := ReceiptListRequest{
		Call: RuntimeCallContext{
			CallerURA:     "easynet:///r/example/agent/backend",
			CalleeURA:     "easynet:///r/example/agent/device.dev-a.runtime-governance",
			SubjectURA:    "easynet:///r/example/resource/user.00000000-0000-0000-0000-000000000000/session/invocation_history",
			NonceBase64:   "AQIDBAUGBwgJCgsMDQ4PEA==",
			CausalContext: map[string]any{"form": "none"},
			Authority: sessionAuthorityFixture(t, map[string]any{
				"scopes":                     []string{"invocation.history.list"},
				"allowed_followup_abilities": []string{"invocation.history.list"},
			}),
		},
		Limit: 10,
	}

	_, err := session.sdk.History().List(context.Background(), request)
	if err == nil || !strings.Contains(err.Error(), "subject_ura must not be all-zero") {
		t.Fatalf("history list error = %v, want all-zero subject rejection", err)
	}
	if session.receipts.listCalls != 0 {
		t.Fatalf("receipt provider called after all-zero subject: %d", session.receipts.listCalls)
	}
}

func TestAuthorizedRuntimeSessionHistoryRejectsRetiredSessionSubjectBeforeReceiptProvider(t *testing.T) {
	session := newAuthorizedRuntimeSessionFixture(t)
	request := ReceiptListRequest{
		Call: RuntimeCallContext{
			CallerURA:     "easynet:///r/example/agent/backend",
			CalleeURA:     "easynet:///r/example/agent/device.dev-a.runtime-governance",
			SubjectURA:    "easynet:///r/example/resource/user.alice/session/invocation_history",
			NonceBase64:   "AQIDBAUGBwgJCgsMDQ4PEA==",
			CausalContext: map[string]any{"form": "none"},
			Authority: sessionAuthorityFixture(t, map[string]any{
				"scopes":                     []string{"invocation.history.list"},
				"allowed_followup_abilities": []string{"invocation.history.list"},
			}),
		},
		Limit: 10,
	}

	_, err := session.sdk.History().List(context.Background(), request)
	if err == nil || !strings.Contains(err.Error(), "runtime-state read subject") {
		t.Fatalf("history list error = %v, want runtime-state read subject rejection", err)
	}
	if !IsCode(err, ErrInvalidInvocation) {
		t.Fatalf("error = %v", err)
	}
	if session.receipts.listCalls != 0 {
		t.Fatalf("receipt provider called after retired session subject: %d", session.receipts.listCalls)
	}
}

func TestAuthorizedRuntimeSessionHistoryAllowsUserOwnedResourceSubjectBeforeReceiptProvider(t *testing.T) {
	session := newAuthorizedRuntimeSessionFixture(t)
	subject, err := RuntimeStateReadSubjectURA("example", "alice")
	if err != nil {
		t.Fatalf("runtime-state read subject: %v", err)
	}
	request := ReceiptListRequest{
		Call: RuntimeCallContext{
			CallerURA:     "easynet:///r/example/agent/backend",
			CalleeURA:     "easynet:///r/example/agent/device.dev-a.runtime-governance",
			SubjectURA:    subject,
			NonceBase64:   "AQIDBAUGBwgJCgsMDQ4PEA==",
			CausalContext: map[string]any{"form": "none"},
			Authority: sessionAuthorityFixture(t, map[string]any{
				"scopes":                     []string{"invocation.history.list"},
				"allowed_followup_abilities": []string{"invocation.history.list"},
			}),
		},
		Limit: 10,
	}

	_, err = session.sdk.History().List(context.Background(), request)
	if err != nil {
		t.Fatalf("history list: %v", err)
	}
	if session.receipts.listCalls != 1 {
		t.Fatalf("receipt provider calls = %d, want 1", session.receipts.listCalls)
	}
}

func TestAuthorizedRuntimeSessionHistoryUsesReceiptProviderAuthorityScope(t *testing.T) {
	session := newAuthorizedRuntimeSessionFixture(t)
	session.receipts.historyListScope = "invocation.history.list"
	subject, err := RuntimeStateReadSubjectURA("example", "alice")
	if err != nil {
		t.Fatalf("runtime-state read subject: %v", err)
	}
	request := ReceiptListRequest{
		Call: RuntimeCallContext{
			CallerURA:     "easynet:///r/example/agent/backend",
			CalleeURA:     "easynet:///r/example/agent/device.dev-a.runtime-governance",
			SubjectURA:    subject,
			NonceBase64:   "AQIDBAUGBwgJCgsMDQ4PEA==",
			CausalContext: map[string]any{"form": "none"},
			Authority: sessionAuthorityFixture(t, map[string]any{
				"scopes":                     []string{"invocation.history.list"},
				"allowed_followup_abilities": []string{"invocation.history.list"},
			}),
		},
		Limit: 10,
	}

	_, err = session.sdk.History().List(context.Background(), request)
	if err != nil {
		t.Fatalf("history list: %v", err)
	}
	if session.receipts.listCalls != 1 {
		t.Fatalf("receipt provider calls = %d, want 1", session.receipts.listCalls)
	}
}

func TestAuthorizedRuntimeSessionHistoryRejectsProviderWithoutAuthorityScope(t *testing.T) {
	session := newAuthorizedRuntimeSessionFixtureWithReceipts(t, &sessionReceiptProviderWithoutScope{})
	subject, err := RuntimeStateReadSubjectURA("example", "alice")
	if err != nil {
		t.Fatalf("runtime-state read subject: %v", err)
	}
	request := ReceiptListRequest{
		Call: RuntimeCallContext{
			CallerURA:     "easynet:///r/example/agent/backend",
			CalleeURA:     "easynet:///r/example/agent/device.dev-a.runtime-governance",
			SubjectURA:    subject,
			NonceBase64:   "AQIDBAUGBwgJCgsMDQ4PEA==",
			CausalContext: map[string]any{"form": "none"},
			Authority: sessionAuthorityFixture(t, map[string]any{
				"scopes":                     []string{"invocation.history.list"},
				"allowed_followup_abilities": []string{"invocation.history.list"},
			}),
		},
		Limit: 10,
	}

	_, err = session.sdk.History().List(context.Background(), request)
	if err == nil {
		t.Fatalf("expected missing history authority scope")
	}
	if !IsCode(err, ErrProviderUnavailable) || !strings.Contains(err.Error(), "receipt provider does not expose receipt history authority scope") {
		t.Fatalf("error = %v", err)
	}
}

func TestRuntimeStateReadSubjectURABuildsUserOwnedResourceSubject(t *testing.T) {
	subject, err := RuntimeStateReadSubjectURA("example", "alice")
	if err != nil {
		t.Fatalf("RuntimeStateReadSubjectURA error = %v", err)
	}
	if subject != "easynet:///r/example/resource/user.alice/runtime-state/read" {
		t.Fatalf("subject = %q", subject)
	}
}

func TestRuntimeGovernanceReadSubjectURAProjectsUserBusinessSubject(t *testing.T) {
	subject, err := RuntimeGovernanceReadSubjectURA(
		"easynet:///r/example/user/alice",
		"easynet:///r/example/agent/device.dev-a.runtime-governance",
	)
	if err != nil {
		t.Fatalf("RuntimeGovernanceReadSubjectURA error = %v", err)
	}
	if subject != "easynet:///r/example/resource/user.alice/runtime-state/read" {
		t.Fatalf("subject = %q", subject)
	}
}

func TestRuntimeGovernanceReadSubjectURAAdmitsMatchingRuntimeOwner(t *testing.T) {
	subject, err := RuntimeGovernanceReadSubjectURA(
		"easynet:///r/example/agent/device.dev-a.runtime-governance",
		"easynet:///r/example/agent/device.dev-a.runtime-governance",
	)
	if err != nil {
		t.Fatalf("RuntimeGovernanceReadSubjectURA error = %v", err)
	}
	if subject != "easynet:///r/example/device/dev-a" {
		t.Fatalf("subject = %q", subject)
	}
}

func TestRuntimeStateReadSubjectURARejectsAllZeroUserBeforeDeviceFallback(t *testing.T) {
	_, err := RuntimeStateReadSubjectURA("example", "00000000-0000-0000-0000-000000000000")
	if err == nil || !strings.Contains(err.Error(), "user_id must not be all-zero") {
		t.Fatalf("RuntimeStateReadSubjectURA all-zero error = %v", err)
	}
}

func TestRuntimeStateReadSubjectPredicateRejectsAllZeroOwner(t *testing.T) {
	subject := "easynet:///r/example/resource/user.00000000-0000-0000-0000-000000000000/runtime-state/read"
	if isRuntimeStateReadSubjectURA(subject) {
		t.Fatalf("all-zero runtime-state read subject was accepted: %s", subject)
	}
}

func TestAuthorizedRuntimeSessionHistoryRejectsPathSubstringOwnerSubjectBeforeReceiptProvider(t *testing.T) {
	session := newAuthorizedRuntimeSessionFixture(t)
	request := ReceiptListRequest{
		Call: RuntimeCallContext{
			CallerURA:     "easynet:///r/example/agent/backend",
			CalleeURA:     "easynet:///r/example/agent/device.dev-a.runtime-governance",
			SubjectURA:    "easynet:///r/example/resource/device.dev-a/archive/resource/user.alice/session/session-1",
			NonceBase64:   "AQIDBAUGBwgJCgsMDQ4PEA==",
			CausalContext: map[string]any{"form": "none"},
			Authority: sessionAuthorityFixture(t, map[string]any{
				"scopes":                     []string{"invocation.history.list"},
				"allowed_followup_abilities": []string{"invocation.history.list"},
			}),
		},
		Limit: 10,
	}

	_, err := session.sdk.History().List(context.Background(), request)
	if err == nil {
		t.Fatalf("expected runtime-state subject rejection")
	}
	if !IsCode(err, ErrInvalidInvocation) {
		t.Fatalf("error = %v", err)
	}
	if session.receipts.listCalls != 0 {
		t.Fatalf("receipt provider called after path-substring subject: %d", session.receipts.listCalls)
	}
}

func TestAuthorizedRuntimeSessionHistoryAllowsSessionAuthorityWithExactDeviceSubjectFilter(t *testing.T) {
	session := newAuthorizedRuntimeSessionFixture(t)
	subject, err := RuntimeStateReadSubjectURA("example", "alice")
	if err != nil {
		t.Fatalf("runtime-state read subject: %v", err)
	}
	request := ReceiptListRequest{
		Call: RuntimeCallContext{
			CallerURA:     "easynet:///r/example/agent/backend",
			CalleeURA:     "easynet:///r/example/agent/device.dev-a.runtime-governance",
			SubjectURA:    subject,
			NonceBase64:   "AQIDBAUGBwgJCgsMDQ4PEA==",
			CausalContext: map[string]any{"form": "none"},
			Authority: sessionAuthorityFixture(t, map[string]any{
				"scopes":                     []string{"invocation.history.list"},
				"allowed_followup_abilities": []string{"invocation.history.list"},
			}),
		},
		Filter: ReceiptFilter{
			SubjectURAs: []string{"easynet:///r/example/agent/device.dev-a.runtime-governance"},
		},
		Limit: 10,
	}

	_, err = session.sdk.History().List(context.Background(), request)
	if err != nil {
		t.Fatalf("history list: %v", err)
	}
	if session.receipts.listCalls != 1 {
		t.Fatalf("receipt provider calls = %d, want 1", session.receipts.listCalls)
	}
}

func TestRuntimeClientSessionRuntimeProviderOpensSignedStreamAndBidi(t *testing.T) {
	var streamSigned map[string]any
	var bidiSigned map[string]any
	client, err := NewRuntimeClient(RuntimeTransportFunc{
		OpenStreamFunc: func(ctx context.Context, signedJSON []byte) (StreamTransport, []byte, error) {
			if err := json.Unmarshal(signedJSON, &streamSigned); err != nil {
				t.Fatalf("stream signed JSON: %v", err)
			}
			return StreamTransportFunc{
				RecvFunc: func(context.Context) ([]byte, error) {
					return nil, errors.New("unused")
				},
			}, []byte(`{"stream_id":"provider-stream-1","state":"Open","max_buffered_events":4}`), nil
		},
		OpenBidiFunc: func(ctx context.Context, signedJSON []byte, streamsJSON []byte) (BidiTransport, []byte, error) {
			if err := json.Unmarshal(signedJSON, &bidiSigned); err != nil {
				t.Fatalf("bidi signed JSON: %v", err)
			}
			return &memoryBidiTransport{}, []byte(`{"session_id":"provider-bidi-1","state":"Open","max_buffered_frames":4}`), nil
		},
	})
	if err != nil {
		t.Fatalf("NewRuntimeClient: %v", err)
	}
	provider := NewRuntimeClientSessionRuntimeProvider(client)
	signed := signedForRuntimeTest(t)

	stream, err := provider.OpenStream(context.Background(), signed)
	if err != nil {
		t.Fatalf("OpenStream: %v", err)
	}
	if stream.StreamID() != "provider-stream-1" {
		t.Fatalf("stream id = %q", stream.StreamID())
	}
	if callerSignatureKeyID(streamSigned) != "caller-key" {
		t.Fatalf("stream signed envelope not forwarded: %#v", streamSigned)
	}

	session, err := provider.OpenBidi(context.Background(), signed, []BidiStreamDescriptor{{StreamID: 1}})
	if err != nil {
		t.Fatalf("OpenBidi: %v", err)
	}
	if session.SessionID() != "provider-bidi-1" {
		t.Fatalf("bidi session id = %q", session.SessionID())
	}
	if callerSignatureKeyID(bidiSigned) != "caller-key" {
		t.Fatalf("bidi signed envelope not forwarded: %#v", bidiSigned)
	}
}

func callerSignatureKeyID(invocation map[string]any) string {
	signature, _ := invocation["caller_signature"].(map[string]any)
	keyID, _ := signature["key_id_hint"].(string)
	return keyID
}

func TestRuntimeClientSessionRuntimeProviderRejectsNilClientBeforeDereference(t *testing.T) {
	provider := NewRuntimeClientSessionRuntimeProvider(nil)
	ctx := context.Background()

	if _, _, err := provider.PrepareForSigning(ctx, InvocationDraft{}, PrepareOptions{}); !IsCode(err, ErrProviderUnavailable) {
		t.Fatalf("PrepareForSigning error = %v, want %s", err, ErrProviderUnavailable)
	}
	if _, err := provider.SubmitSigned(ctx, SignedInvocation{}); !IsCode(err, ErrProviderUnavailable) {
		t.Fatalf("SubmitSigned error = %v, want %s", err, ErrProviderUnavailable)
	}
	if _, err := provider.AwaitTerminal(ctx, InvocationHandle{}); !IsCode(err, ErrProviderUnavailable) {
		t.Fatalf("AwaitTerminal error = %v, want %s", err, ErrProviderUnavailable)
	}
	if _, err := provider.Cancel(ctx, InvocationHandle{}, "test"); !IsCode(err, ErrProviderUnavailable) {
		t.Fatalf("Cancel error = %v, want %s", err, ErrProviderUnavailable)
	}
	if _, err := provider.Events(ctx, InvocationHandle{}); !IsCode(err, ErrProviderUnavailable) {
		t.Fatalf("Events error = %v, want %s", err, ErrProviderUnavailable)
	}
	if _, err := provider.Diagnostics(ctx); !IsCode(err, ErrProviderUnavailable) {
		t.Fatalf("Diagnostics error = %v, want %s", err, ErrProviderUnavailable)
	}
	if _, err := provider.OpenStream(ctx, SignedInvocation{}); !IsCode(err, ErrProviderUnavailable) {
		t.Fatalf("OpenStream error = %v, want %s", err, ErrProviderUnavailable)
	}
	if _, err := provider.OpenBidi(ctx, SignedInvocation{}, nil); !IsCode(err, ErrProviderUnavailable) {
		t.Fatalf("OpenBidi error = %v, want %s", err, ErrProviderUnavailable)
	}
}

func TestRuntimeClientDescriptorProviderRejectsNilClientBeforeDereference(t *testing.T) {
	provider := NewRuntimeClientDescriptorProvider(nil)
	resolution, err := provider.ResolveDescriptor(context.Background(), DescriptorResolutionRequest{})
	if err != nil {
		t.Fatalf("ResolveDescriptor returned transport error = %v", err)
	}
	if resolution.State != DescriptorUnavailable {
		t.Fatalf("nil-client descriptor state = %s, want %s", resolution.State, DescriptorUnavailable)
	}
	if !strings.Contains(resolution.Reason, string(ErrProviderUnavailable)) {
		t.Fatalf("nil-client descriptor reason = %q, want provider unavailable", resolution.Reason)
	}
}

func TestRuntimeClientDescriptorProviderUsesAbilityDescriptorProviderForCatalogueAbilityURA(t *testing.T) {
	var seen RuntimeDescriptorRefRequest
	transport := RuntimeTransportFunc{
		ResolveDescriptorRefFunc: func(_ context.Context, requestJSON []byte) ([]byte, error) {
			if err := json.Unmarshal(requestJSON, &seen); err != nil {
				return nil, err
			}
			return []byte(`{"descriptor_ref":"easynet:///r/example/ability/system-agent.dev-a.runtime-introspection.meta.list_resources@1.0.0#aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa!read"}`), nil
		},
	}
	runtime, err := NewRuntimeClient(transport)
	if err != nil {
		t.Fatalf("NewRuntimeClient: %v", err)
	}
	provider := NewRuntimeClientDescriptorProvider(runtime)

	resolution, err := provider.ResolveDescriptor(context.Background(), DescriptorResolutionRequest{
		CallerIdentity: CallerIdentityRef{Principal: PrincipalRef{URA: "easynet:///r/example/user/alice"}},
		Target:         RuntimeTargetRef{URA: "easynet:///r/example/agent/device.dev-a.runtime-introspection"},
		Ability:        AbilityRef{Name: "easynet:///r/example/ability/system-agent.dev-a.runtime-introspection.meta.list_resources"},
		Subject:        IntentSubjectRef{URA: "easynet:///r/example/user/alice"},
		CallMode:       "rpc",
	})
	if err != nil {
		t.Fatalf("ResolveDescriptor: %v", err)
	}
	if resolution.State != DescriptorResolved {
		t.Fatalf("descriptor state = %s, want %s (%s)", resolution.State, DescriptorResolved, resolution.Reason)
	}
	if seen.Provider != runtimeAbilityDescriptorProvider {
		t.Fatalf("descriptor provider = %q, want %q", seen.Provider, runtimeAbilityDescriptorProvider)
	}
	if seen.SubjectURA != "easynet:///r/example/resource/user.alice/runtime-state/read" {
		t.Fatalf("descriptor subject_ura = %q, want runtime governance read subject", seen.SubjectURA)
	}
}

func TestRuntimeClientDescriptorProviderUsesGovernanceSubjectForReceiptHistoryProvider(t *testing.T) {
	var seen RuntimeDescriptorRefRequest
	transport := RuntimeTransportFunc{
		ResolveDescriptorRefFunc: func(_ context.Context, requestJSON []byte) ([]byte, error) {
			if err := json.Unmarshal(requestJSON, &seen); err != nil {
				return nil, err
			}
			return []byte(`{"descriptor_ref":"easynet:///r/example/ability/system-agent.dev-a.runtime-governance.invocation.history.list@1.0.0#bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb!read"}`), nil
		},
	}
	runtime, err := NewRuntimeClient(transport)
	if err != nil {
		t.Fatalf("NewRuntimeClient: %v", err)
	}
	provider := NewRuntimeClientDescriptorProvider(runtime)

	resolution, err := provider.ResolveDescriptor(context.Background(), DescriptorResolutionRequest{
		CallerIdentity: CallerIdentityRef{Principal: PrincipalRef{URA: "easynet:///r/example/user/alice"}},
		Target:         RuntimeTargetRef{URA: "easynet:///r/example/agent/device.dev-a.runtime-governance"},
		Ability:        AbilityRef{Name: "invocation.history.list"},
		Subject:        IntentSubjectRef{URA: "easynet:///r/example/user/alice"},
		CallMode:       "rpc",
	})
	if err != nil {
		t.Fatalf("ResolveDescriptor: %v", err)
	}
	if resolution.State != DescriptorResolved {
		t.Fatalf("descriptor state = %s, want %s (%s)", resolution.State, DescriptorResolved, resolution.Reason)
	}
	if seen.Provider != runtimeReceiptHistoryProvider {
		t.Fatalf("descriptor provider = %q, want %q", seen.Provider, runtimeReceiptHistoryProvider)
	}
	if seen.SubjectURA != "easynet:///r/example/resource/user.alice/runtime-state/read" {
		t.Fatalf("descriptor subject_ura = %q, want runtime governance read subject", seen.SubjectURA)
	}
}

type authorizedRuntimeSessionFixture struct {
	sdk           *AuthorizedRuntimeSession
	runtime       *sessionRuntimeProviderFixture
	descriptor    *sessionDescriptorProviderFixture
	authorization *sessionAuthorizationProviderFixture
	signer        *sessionSignerProviderFixture
	identity      *sessionIdentityProviderFixture
	receipts      *sessionReceiptProviderFixture
}

func newAuthorizedRuntimeSessionFixture(t *testing.T) authorizedRuntimeSessionFixture {
	t.Helper()
	return newAuthorizedRuntimeSessionFixtureWithReceipts(t, &sessionReceiptProviderFixture{
		historyListScope: receiptHistoryListAbility,
	})
}

func newAuthorizedRuntimeSessionFixtureWithReceipts(t *testing.T, receipts ReceiptProvider) authorizedRuntimeSessionFixture {
	t.Helper()
	runtime := &sessionRuntimeProviderFixture{}
	descriptor := &sessionDescriptorProviderFixture{}
	authorization := &sessionAuthorizationProviderFixture{
		authority: sessionAuthorityFixture(t, map[string]any{
			"scopes":                     []string{"invocation.history.list"},
			"allowed_followup_abilities": []string{"invocation.history.list"},
		}),
	}
	signer := &sessionSignerProviderFixture{}
	identity := &sessionIdentityProviderFixture{
		caller: CallerIdentityRef{Principal: PrincipalRef{URA: "easynet:///r/example/agent/backend"}},
	}
	sdk, err := NewAuthorizedRuntimeSession(AuthorizedRuntimeSessionDeps{
		Runtime:       runtime,
		Descriptor:    descriptor,
		Authorization: authorization,
		Signer:        signer,
		Receipts:      receipts,
		Identity:      identity,
		Clock:         sessionClockFixture{},
	})
	if err != nil {
		t.Fatalf("NewAuthorizedRuntimeSession: %v", err)
	}
	return authorizedRuntimeSessionFixture{
		sdk:           sdk,
		runtime:       runtime,
		descriptor:    descriptor,
		authorization: authorization,
		signer:        signer,
		identity:      identity,
		receipts:      sessionReceiptFixture(receipts),
	}
}

func sessionReceiptFixture(provider ReceiptProvider) *sessionReceiptProviderFixture {
	if fixture, ok := provider.(*sessionReceiptProviderFixture); ok {
		return fixture
	}
	return nil
}

func canonicalSessionIntentFixture() InvocationIntent {
	return InvocationIntent{
		CallerIdentity:  CallerIdentityRef{Principal: PrincipalRef{URA: "easynet:///r/example/agent/backend"}},
		ActingPrincipal: ActingPrincipalRef{Principal: PrincipalRef{URA: "easynet:///r/example/agent/backend"}},
		Target:          RuntimeTargetRef{URA: "easynet:///r/example/device/dev-a"},
		Ability:         AbilityRef{Name: "invocation.history.list"},
		Subject:         IntentSubjectRef{URA: "easynet:///r/example/resource/user.alice/session/session-1", DerivationRule: "fixture"},
		CallMode:        "rpc",
		Arguments:       map[string]any{"limit": float64(10)},
		DeadlineUnixMS:  2000,
		IdempotencyKey:  "idem-1",
		CausalContext:   map[string]any{"form": "none"},
	}
}

func sessionAuthorityFixture(t *testing.T, override map[string]any) SessionAuthority {
	t.Helper()
	payload := sessionAuthorityPayloadFixture()
	for key, value := range override {
		payload[key] = value
	}
	authority, err := NewSessionAuthorityFromMetadata(authorityMetadataFixture(t, payload, []byte("session-signature")))
	if err != nil {
		t.Fatalf("NewSessionAuthorityFromMetadata: %v", err)
	}
	return authority
}

type sessionRuntimeProviderFixture struct {
	prepareCalls int
	submitCalls  int
}

func (p *sessionRuntimeProviderFixture) PrepareForSigning(ctx context.Context, draft InvocationDraft, opts PrepareOptions) (PreparedInvocation, SigningMaterial, error) {
	_ = ctx
	_ = opts
	p.prepareCalls++
	rawDraft, err := json.Marshal(draft)
	if err != nil {
		return PreparedInvocation{}, SigningMaterial{}, err
	}
	raw, err := json.Marshal(map[string]any{
		"prepared_id":        "prepared-1",
		"descriptor_ref":     draft.DescriptorRef(),
		"expires_at_unix_ms": float64(3000),
		"tuple":              json.RawMessage(rawDraft),
		"signing_material": map[string]any{
			"canonical_bytes_base64": base64.StdEncoding.EncodeToString([]byte("canonical")),
			"args_digest_hex":        "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
			"descriptor_ref":         draft.DescriptorRef(),
			"expires_at_unix_ms":     float64(3000),
			"signed_fields":          []string{"caller_ura"},
		},
	})
	if err != nil {
		return PreparedInvocation{}, SigningMaterial{}, err
	}
	prepared, err := NewPreparedInvocationFromJSON(raw)
	if err != nil {
		return PreparedInvocation{}, SigningMaterial{}, err
	}
	return prepared, prepared.SigningMaterial(), nil
}

func (p *sessionRuntimeProviderFixture) SubmitSigned(ctx context.Context, signed SignedInvocation) (InvocationHandle, error) {
	_ = ctx
	_ = signed
	p.submitCalls++
	return InvocationHandle{}, nil
}

func (p *sessionRuntimeProviderFixture) AwaitTerminal(context.Context, InvocationHandle) (InvocationResult, error) {
	return InvocationResult{}, nil
}

func (p *sessionRuntimeProviderFixture) OpenStream(context.Context, SignedInvocation) (*StreamHandle, error) {
	return nil, nil
}

func (p *sessionRuntimeProviderFixture) OpenBidi(context.Context, SignedInvocation, []BidiStreamDescriptor) (*BidiSession, error) {
	return nil, nil
}

func (p *sessionRuntimeProviderFixture) Cancel(context.Context, InvocationHandle, string) (InvocationCancel, error) {
	return InvocationCancel{}, nil
}

func (p *sessionRuntimeProviderFixture) Events(context.Context, InvocationHandle) (InvocationHandle, error) {
	return InvocationHandle{}, nil
}

func (p *sessionRuntimeProviderFixture) Diagnostics(context.Context) (map[string]any, error) {
	return map[string]any{}, nil
}

type sessionDescriptorProviderFixture struct {
	calls int
}

func (p *sessionDescriptorProviderFixture) ResolveDescriptor(context.Context, DescriptorResolutionRequest) (DescriptorResolution, error) {
	p.calls++
	return DescriptorResolution{
		State:                 DescriptorResolved,
		DescriptorRef:         "easynet:///r/example/ability/system-agent.dev-a.runtime-governance.invocation.history.list@1.0.0#aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa!read",
		ResolvedCalleeURA:     "easynet:///r/example/agent/device.dev-a.runtime-governance",
		DescriptorFingerprint: "descriptor-fingerprint",
		OwnerPrincipal:        PrincipalRef{URA: "easynet:///r/example/user/alice"},
	}, nil
}

type sessionAuthorizationProviderFixture struct {
	authority SessionAuthority
}

func (p *sessionAuthorizationProviderFixture) AuthorizeInvocation(context.Context, PreparedInvocationState) (AuthorityArtifact, error) {
	return AuthorityArtifact{
		Authority:   p.authority,
		Fingerprint: "authority-fingerprint",
		Subject:     IntentSubjectRef{URA: p.authority.SubjectURA},
		Owner:       PrincipalRef{URA: "easynet:///r/example/user/alice"},
	}, nil
}

type sessionSignerProviderFixture struct {
	err error
}

func (p *sessionSignerProviderFixture) CallerSigner(context.Context, AuthorizedInvocation, SigningMaterial) (Signer, error) {
	if p.err != nil {
		return Signer{}, p.err
	}
	return Signer{}, &SDKError{Code: ErrCallerSignerUnavailable, Stage: "sign", Retry: RetryNever, Message: "fixture has no signer"}
}

type sessionIdentityProviderFixture struct {
	caller CallerIdentityRef
}

func (p *sessionIdentityProviderFixture) CallerIdentity(context.Context) (CallerIdentityRef, error) {
	if p.caller.Principal.URA == "" {
		return CallerIdentityRef{}, &SDKError{Code: ErrCallerIdentityUnavailable, Stage: "identity", Retry: RetryNever, Message: "missing"}
	}
	return p.caller, nil
}

type sessionClockFixture struct{}

func (sessionClockFixture) NowUnixMS() int64 { return 1000 }

func (sessionClockFixture) NewIdempotencyKey() (string, error) { return "idem-1", nil }

func (sessionClockFixture) NewNonceBase64() (string, error) {
	return "AQIDBAUGBwgJCgsMDQ4PEA==", nil
}

type sessionReceiptProviderFixture struct {
	listCalls        int
	historyListScope string
}

func (p *sessionReceiptProviderFixture) ReceiptHistoryListAuthorityScope() (string, error) {
	return p.historyListScope, nil
}

func (p *sessionReceiptProviderFixture) List(context.Context, ReceiptListRequest) (ReceiptHistoryPage, error) {
	p.listCalls++
	return ReceiptHistoryPage{}, nil
}

func (*sessionReceiptProviderFixture) Get(context.Context, ReceiptGetRequest) (ReceiptGetResult, error) {
	return ReceiptGetResult{}, nil
}

func (*sessionReceiptProviderFixture) Trace(context.Context, ReceiptTraceRequest) (ReceiptTraceResult, error) {
	return ReceiptTraceResult{}, nil
}

type sessionReceiptProviderWithoutScope struct{}

func (*sessionReceiptProviderWithoutScope) List(context.Context, ReceiptListRequest) (ReceiptHistoryPage, error) {
	return ReceiptHistoryPage{}, nil
}

func (*sessionReceiptProviderWithoutScope) Get(context.Context, ReceiptGetRequest) (ReceiptGetResult, error) {
	return ReceiptGetResult{}, nil
}

func (*sessionReceiptProviderWithoutScope) Trace(context.Context, ReceiptTraceRequest) (ReceiptTraceResult, error) {
	return ReceiptTraceResult{}, nil
}
