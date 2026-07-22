package easynet

import (
	"context"
	"encoding/base64"
	"encoding/json"
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

func TestAuthorizedRuntimeSessionHistoryRejectsAuthoritySubjectMismatchBeforeReceiptProvider(t *testing.T) {
	session := newAuthorizedRuntimeSessionFixture(t)
	request := ReceiptListRequest{
		Call: RuntimeCallContext{
			CallerURA:     "easynet:///r/example/agent/backend",
			CalleeURA:     "easynet:///r/example/device/dev-a",
			SubjectURA:    "easynet:///r/example/device/dev-a",
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
		t.Fatalf("expected authority subject mismatch")
	}
	if !IsCode(err, ErrAuthoritySubjectMismatch) {
		t.Fatalf("error = %v", err)
	}
	if session.receipts.listCalls != 0 {
		t.Fatalf("receipt provider called after mismatch: %d", session.receipts.listCalls)
	}
}

func TestAuthorizedRuntimeSessionHistoryRejectsOwnerEquivalentSubjectExpansionBeforeReceiptProvider(t *testing.T) {
	session := newAuthorizedRuntimeSessionFixture(t)
	request := ReceiptListRequest{
		Call: RuntimeCallContext{
			CallerURA:     "easynet:///r/example/agent/backend",
			CalleeURA:     "easynet:///r/example/device/dev-a",
			SubjectURA:    "easynet:///r/example/resource/user.alice/session/session-2",
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
		t.Fatalf("expected authority subject mismatch")
	}
	if !IsCode(err, ErrAuthoritySubjectMismatch) {
		t.Fatalf("error = %v", err)
	}
	if session.receipts.listCalls != 0 {
		t.Fatalf("receipt provider called after owner-equivalent subject expansion: %d", session.receipts.listCalls)
	}
}

func TestAuthorizedRuntimeSessionHistoryRejectsFilterSubjectExpansionBeforeReceiptProvider(t *testing.T) {
	session := newAuthorizedRuntimeSessionFixture(t)
	request := ReceiptListRequest{
		Call: RuntimeCallContext{
			CallerURA:     "easynet:///r/example/agent/backend",
			CalleeURA:     "easynet:///r/example/device/dev-a",
			SubjectURA:    "easynet:///r/example/resource/user.alice/session/session-1",
			NonceBase64:   "AQIDBAUGBwgJCgsMDQ4PEA==",
			CausalContext: map[string]any{"form": "none"},
			Authority: sessionAuthorityFixture(t, map[string]any{
				"scopes":                     []string{"invocation.history.list"},
				"allowed_followup_abilities": []string{"invocation.history.list"},
			}),
		},
		Filter: ReceiptFilter{
			SubjectURAs: []string{"easynet:///r/example/device/dev-a"},
		},
		Limit: 10,
	}

	_, err := session.sdk.History().List(context.Background(), request)
	if err == nil {
		t.Fatalf("expected filter subject mismatch")
	}
	if !IsCode(err, ErrAuthoritySubjectMismatch) {
		t.Fatalf("error = %v", err)
	}
	if session.receipts.listCalls != 0 {
		t.Fatalf("receipt provider called after filter mismatch: %d", session.receipts.listCalls)
	}
}

func TestRuntimeClientSessionRuntimeProviderRejectsUnsignedStreamDowngrade(t *testing.T) {
	provider := NewRuntimeClientSessionRuntimeProvider(&RuntimeClient{})

	if _, err := provider.OpenStream(context.Background(), SignedInvocation{}); !IsCode(err, ErrProviderUnavailable) {
		t.Fatalf("OpenStream error = %v", err)
	}
	if _, err := provider.OpenBidi(context.Background(), SignedInvocation{}, nil); !IsCode(err, ErrProviderUnavailable) {
		t.Fatalf("OpenBidi error = %v", err)
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
	runtime := &sessionRuntimeProviderFixture{}
	descriptor := &sessionDescriptorProviderFixture{}
	authorization := &sessionAuthorizationProviderFixture{
		authority: sessionAuthorityFixture(t, map[string]any{
			"scopes":                     []string{"invocation.history.list"},
			"allowed_followup_abilities": []string{"invocation.history.list"},
		}),
	}
	signer := &sessionSignerProviderFixture{}
	receipts := &sessionReceiptProviderFixture{}
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
		receipts:      receipts,
	}
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
		"prepared_id": "prepared-1",
		"tuple":       json.RawMessage(rawDraft),
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
		DescriptorRef:         "easynet:///r/example/ability/invocation.history.list@1.0.0",
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
	listCalls int
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
