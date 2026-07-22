package easynet

import (
	"context"
	"crypto/sha256"
	"encoding/hex"
	"encoding/json"
	"errors"
	"fmt"
	"strings"
	"time"
)

type RuntimeSessionState string

const (
	RuntimeSessionIntent     RuntimeSessionState = "Intent"
	RuntimeSessionPrepared   RuntimeSessionState = "Prepared"
	RuntimeSessionAuthorized RuntimeSessionState = "Authorized"
	RuntimeSessionSigned     RuntimeSessionState = "Signed"
	RuntimeSessionSubmitted  RuntimeSessionState = "Submitted"
	RuntimeSessionTerminal   RuntimeSessionState = "Terminal"
)

type PrincipalRef struct {
	URA string `json:"ura"`
}

type CallerIdentityRef struct {
	Principal PrincipalRef `json:"principal"`
}

type ActingPrincipalRef struct {
	Principal PrincipalRef `json:"principal"`
}

type RuntimeTargetRef struct {
	URA string `json:"ura"`
}

type AbilityRef struct {
	Name string `json:"name"`
}

type IntentSubjectRef struct {
	URA            string `json:"ura"`
	DerivationRule string `json:"derivation_rule,omitempty"`
}

type InvocationIntent struct {
	CallerIdentity  CallerIdentityRef  `json:"caller_identity"`
	ActingPrincipal ActingPrincipalRef `json:"acting_principal"`
	Target          RuntimeTargetRef   `json:"target"`
	Ability         AbilityRef         `json:"ability"`
	Subject         IntentSubjectRef   `json:"subject"`
	CallMode        string             `json:"call_mode"`
	Arguments       any                `json:"arguments"`
	ContentType     string             `json:"content_type,omitempty"`
	DeadlineUnixMS  int64              `json:"deadline_unix_ms"`
	IdempotencyKey  string             `json:"idempotency_key"`
	CausalContext   map[string]any     `json:"causal_context"`
	Metadata        map[string]any     `json:"metadata,omitempty"`
}

type DescriptorResolutionState string

const (
	DescriptorResolved        DescriptorResolutionState = "Resolved"
	DescriptorNotFound        DescriptorResolutionState = "NotFound"
	DescriptorOwnerOffline    DescriptorResolutionState = "OwnerOffline"
	DescriptorModeUnsupported DescriptorResolutionState = "ModeUnsupported"
	DescriptorStale           DescriptorResolutionState = "Stale"
	DescriptorUnavailable     DescriptorResolutionState = "Unavailable"
)

type DescriptorResolutionRequest struct {
	CallerIdentity  CallerIdentityRef  `json:"caller_identity"`
	ActingPrincipal ActingPrincipalRef `json:"acting_principal"`
	Target          RuntimeTargetRef   `json:"target"`
	Ability         AbilityRef         `json:"ability"`
	Subject         IntentSubjectRef   `json:"subject"`
	CallMode        string             `json:"call_mode"`
	DeadlineUnixMS  int64              `json:"deadline_unix_ms"`
	IdempotencyKey  string             `json:"idempotency_key"`
	CausalContext   map[string]any     `json:"causal_context"`
}

type DescriptorResolution struct {
	State                 DescriptorResolutionState `json:"state"`
	DescriptorRef         string                    `json:"descriptor_ref,omitempty"`
	DescriptorFingerprint string                    `json:"descriptor_fingerprint,omitempty"`
	OwnerPrincipal        PrincipalRef              `json:"owner_principal,omitempty"`
	Reason                string                    `json:"reason,omitempty"`
}

type PreparedInvocationState struct {
	Intent                 InvocationIntent `json:"intent"`
	Draft                  InvocationDraft  `json:"draft"`
	DescriptorRef          string           `json:"descriptor_ref"`
	DescriptorFingerprint  string           `json:"descriptor_fingerprint"`
	OwnerPrincipal         PrincipalRef     `json:"owner_principal"`
	PreparationFingerprint string           `json:"preparation_fingerprint"`
}

type AuthorityArtifact struct {
	Authority   RuntimeInvocationAuthority `json:"-"`
	Fingerprint string                     `json:"fingerprint,omitempty"`
	Subject     IntentSubjectRef           `json:"subject"`
	Owner       PrincipalRef               `json:"owner_principal,omitempty"`
	Admission   map[string]any             `json:"admission_facts,omitempty"`
}

type AuthorizedInvocation struct {
	Prepared PreparedInvocationState `json:"prepared"`
	Draft    InvocationDraft         `json:"draft"`
	Artifact AuthorityArtifact       `json:"artifact"`
}

type SignedInvocationState struct {
	Authorized AuthorizedInvocation `json:"authorized"`
	Prepared   PreparedInvocation   `json:"prepared"`
	Signed     SignedInvocation     `json:"signed"`
	SignerID   string               `json:"signer_id"`
}

type SubmittedInvocation struct {
	Signed SignedInvocationState `json:"signed"`
	Handle InvocationHandle      `json:"handle"`
}

type TerminalReceipt struct {
	Submitted SubmittedInvocation `json:"submitted"`
	Result    InvocationResult    `json:"result"`
	Receipt   RuntimeReceipt      `json:"receipt"`
}

type RuntimeProvider interface {
	PrepareForSigning(context.Context, InvocationDraft, PrepareOptions) (PreparedInvocation, SigningMaterial, error)
	SubmitSigned(context.Context, SignedInvocation) (InvocationHandle, error)
	AwaitTerminal(context.Context, InvocationHandle) (InvocationResult, error)
	OpenStream(context.Context, SignedInvocation) (*StreamHandle, error)
	OpenBidi(context.Context, SignedInvocation, []BidiStreamDescriptor) (*BidiSession, error)
	Cancel(context.Context, InvocationHandle, string) (InvocationCancel, error)
	Events(context.Context, InvocationHandle) (InvocationHandle, error)
	Diagnostics(context.Context) (map[string]any, error)
}

type DescriptorProvider interface {
	ResolveDescriptor(context.Context, DescriptorResolutionRequest) (DescriptorResolution, error)
}

type AuthorizationProvider interface {
	AuthorizeInvocation(context.Context, PreparedInvocationState) (AuthorityArtifact, error)
}

type SignerProvider interface {
	CallerSigner(context.Context, AuthorizedInvocation, SigningMaterial) (Signer, error)
}

type IdentityProvider interface {
	CallerIdentity(context.Context) (CallerIdentityRef, error)
}

type ClockIdempotencySource interface {
	NowUnixMS() int64
	NewIdempotencyKey() (string, error)
	NewNonceBase64() (string, error)
}

type AuthorizedRuntimeSessionDeps struct {
	Runtime       RuntimeProvider
	Descriptor    DescriptorProvider
	Authorization AuthorizationProvider
	Signer        SignerProvider
	Receipts      ReceiptProvider
	Identity      IdentityProvider
	Clock         ClockIdempotencySource
}

type AuthorizedRuntimeSession struct {
	runtime       RuntimeProvider
	descriptor    DescriptorProvider
	authorization AuthorizationProvider
	signer        SignerProvider
	receipts      ReceiptProvider
	identity      IdentityProvider
	clock         ClockIdempotencySource
	abilities     SessionAbilityOperations
	invoke        SessionInvokeOperations
	streams       SessionStreamOperations
	bidi          SessionBidiOperations
	receiptOps    SessionReceiptOperations
	history       SessionHistoryOperations
	cancellation  SessionCancellationOperations
	diagnostics   SessionDiagnosticsOperations
}

func NewAuthorizedRuntimeSession(deps AuthorizedRuntimeSessionDeps) (*AuthorizedRuntimeSession, error) {
	if deps.Runtime == nil {
		return nil, v3SessionError(ErrProviderUnavailable, "runtime", "runtime provider is required", nil, nil)
	}
	if deps.Descriptor == nil {
		return nil, v3SessionError(ErrProviderUnavailable, "descriptor", "descriptor provider is required", nil, nil)
	}
	if deps.Authorization == nil {
		return nil, v3SessionError(ErrProviderUnavailable, "authorization", "authorization provider is required", nil, nil)
	}
	if deps.Signer == nil {
		return nil, v3SessionError(ErrCallerSignerUnavailable, "sign", "signer provider is required", nil, nil)
	}
	if deps.Receipts == nil {
		return nil, v3SessionError(ErrProviderUnavailable, "receipt", "receipt provider is required", nil, nil)
	}
	if deps.Identity == nil {
		return nil, v3SessionError(ErrCallerIdentityUnavailable, "identity", "identity provider is required", nil, nil)
	}
	if deps.Clock == nil {
		return nil, v3SessionError(ErrProviderUnavailable, "clock", "clock/idempotency source is required", nil, nil)
	}
	session := &AuthorizedRuntimeSession{
		runtime:       deps.Runtime,
		descriptor:    deps.Descriptor,
		authorization: deps.Authorization,
		signer:        deps.Signer,
		receipts:      deps.Receipts,
		identity:      deps.Identity,
		clock:         deps.Clock,
	}
	session.abilities = SessionAbilityOperations{session: session}
	session.invoke = SessionInvokeOperations{session: session}
	session.streams = SessionStreamOperations{session: session}
	session.bidi = SessionBidiOperations{session: session}
	session.receiptOps = SessionReceiptOperations{session: session}
	session.history = SessionHistoryOperations{session: session}
	session.cancellation = SessionCancellationOperations{session: session}
	session.diagnostics = SessionDiagnosticsOperations{session: session}
	return session, nil
}

func (s *AuthorizedRuntimeSession) Abilities() *SessionAbilityOperations {
	return &s.abilities
}

func (s *AuthorizedRuntimeSession) Invoke() *SessionInvokeOperations {
	return &s.invoke
}

func (s *AuthorizedRuntimeSession) Streams() *SessionStreamOperations {
	return &s.streams
}

func (s *AuthorizedRuntimeSession) Bidi() *SessionBidiOperations {
	return &s.bidi
}

func (s *AuthorizedRuntimeSession) Receipts() *SessionReceiptOperations {
	return &s.receiptOps
}

func (s *AuthorizedRuntimeSession) History() *SessionHistoryOperations {
	return &s.history
}

func (s *AuthorizedRuntimeSession) Cancellation() *SessionCancellationOperations {
	return &s.cancellation
}

func (s *AuthorizedRuntimeSession) Diagnostics() *SessionDiagnosticsOperations {
	return &s.diagnostics
}

func (s *AuthorizedRuntimeSession) Prepare(ctx context.Context, intent InvocationIntent) (PreparedInvocationState, error) {
	intent, err := s.normalizedIntent(ctx, intent)
	if err != nil {
		return PreparedInvocationState{}, err
	}
	resolution, err := s.descriptor.ResolveDescriptor(ctx, descriptorRequestFromIntent(intent))
	if err != nil {
		return PreparedInvocationState{}, err
	}
	if err := validateDescriptorResolution(resolution); err != nil {
		return PreparedInvocationState{}, err
	}
	nonce, err := s.clock.NewNonceBase64()
	if err != nil {
		return PreparedInvocationState{}, v3SessionError(ErrProviderUnavailable, "prepare", "nonce source failed", sessionIntentDetails(intent), err)
	}
	metadata := runtimeSessionIntentMetadata(intent, resolution)
	draft, err := NewInvocationBuilder().
		WithCallerURA(intent.CallerIdentity.Principal.URA).
		WithCalleeURA(intent.Target.URA).
		WithSubjectURA(intent.Subject.URA).
		WithDescriptorRef(resolution.DescriptorRef).
		WithNonceBase64(nonce).
		WithCausalContext(intent.CausalContext).
		WithJSONArgs(intent.Arguments).
		WithContentType(runtimeSessionContentType(intent)).
		WithMetadata(metadata).
		Build()
	if err != nil {
		return PreparedInvocationState{}, err
	}
	prepared := PreparedInvocationState{
		Intent:                intent,
		Draft:                 draft,
		DescriptorRef:         resolution.DescriptorRef,
		DescriptorFingerprint: resolution.DescriptorFingerprint,
		OwnerPrincipal:        resolution.OwnerPrincipal,
	}
	prepared.PreparationFingerprint = preparationFingerprint(prepared)
	return prepared, nil
}

func (s *AuthorizedRuntimeSession) Authorize(ctx context.Context, prepared PreparedInvocationState) (AuthorizedInvocation, error) {
	if prepared.PreparationFingerprint == "" {
		return AuthorizedInvocation{}, v3SessionError(ErrInvalidInvocation, "authorize", "prepared invocation state is required", nil, nil)
	}
	artifact, err := s.authorization.AuthorizeInvocation(ctx, prepared)
	if err != nil {
		return AuthorizedInvocation{}, err
	}
	if artifact.Authority == nil {
		return AuthorizedInvocation{}, v3SessionError(ErrAuthorityDenied, "authorize", "authority artifact is required", sessionPreparedDetails(prepared), nil)
	}
	if artifact.Subject.URA == "" {
		artifact.Subject = prepared.Intent.Subject
	}
	if artifact.Owner.URA == "" {
		artifact.Owner = prepared.OwnerPrincipal
	}
	if err := validateAuthorizedRuntimeBinding(artifact, prepared); err != nil {
		return AuthorizedInvocation{}, err
	}
	projection, err := artifact.Authority.Metadata()
	if err != nil {
		return AuthorizedInvocation{}, err
	}
	metadata, err := projection.MergeInto(prepared.Draft.Metadata())
	if err != nil {
		return AuthorizedInvocation{}, err
	}
	draft, err := rebuildDraftWithMetadata(prepared.Draft, metadata)
	if err != nil {
		return AuthorizedInvocation{}, err
	}
	return AuthorizedInvocation{
		Prepared: prepared,
		Draft:    draft,
		Artifact: artifact,
	}, nil
}

func (s *AuthorizedRuntimeSession) Sign(ctx context.Context, authorized AuthorizedInvocation, opts PrepareOptions) (SignedInvocationState, error) {
	if authorized.Artifact.Authority == nil {
		return SignedInvocationState{}, v3SessionError(ErrAuthorityDenied, "sign", "authorized invocation is required", nil, nil)
	}
	prepared, material, err := s.runtime.PrepareForSigning(ctx, authorized.Draft, opts)
	if err != nil {
		return SignedInvocationState{}, err
	}
	signer, err := s.signer.CallerSigner(ctx, authorized, material)
	if err != nil {
		if IsCode(err, ErrCallerSignerUnavailable) {
			return SignedInvocationState{}, err
		}
		return SignedInvocationState{}, v3SessionError(ErrCallerSignerUnavailable, "sign", "caller signer unavailable", sessionAuthorizedDetails(authorized), err)
	}
	if strings.TrimSpace(signer.Handle().OwnerURA) != strings.TrimSpace(authorized.Prepared.Intent.CallerIdentity.Principal.URA) {
		return SignedInvocationState{}, v3SessionError(ErrCallerSignerUnavailable, "sign", "signer owner does not match caller identity", sessionAuthorizedDetails(authorized), nil)
	}
	signed, err := signer.Sign(prepared)
	if err != nil {
		return SignedInvocationState{}, err
	}
	return SignedInvocationState{
		Authorized: authorized,
		Prepared:   prepared,
		Signed:     signed,
		SignerID:   signed.SignerID(),
	}, nil
}

func (s *AuthorizedRuntimeSession) Submit(ctx context.Context, signed SignedInvocationState) (SubmittedInvocation, error) {
	if !signed.Signed.SubmitReady() {
		return SubmittedInvocation{}, v3SessionError(ErrInvalidInvocation, "submit", "signed invocation is not submit-ready", nil, nil)
	}
	handle, err := s.runtime.SubmitSigned(ctx, signed.Signed)
	if err != nil {
		return SubmittedInvocation{}, err
	}
	return SubmittedInvocation{Signed: signed, Handle: handle}, nil
}

func (s *AuthorizedRuntimeSession) AwaitTerminal(ctx context.Context, submitted SubmittedInvocation) (TerminalReceipt, error) {
	result, err := s.runtime.AwaitTerminal(ctx, submitted.Handle)
	if err != nil {
		return TerminalReceipt{}, err
	}
	receipt := result.TerminalReceiptSummary()
	if receipt == nil {
		return TerminalReceipt{}, v3SessionError(ErrTerminalReceiptUnavailable, "terminal", "terminal receipt is required", nil, nil)
	}
	if err := receipt.ValidateProofFacts(); err != nil {
		return TerminalReceipt{}, v3SessionError(ErrReceiptProofFactsMissing, "receipt", "terminal receipt proof facts are missing", nil, err)
	}
	return TerminalReceipt{
		Submitted: submitted,
		Result:    result,
		Receipt:   *receipt,
	}, nil
}

func (s *AuthorizedRuntimeSession) normalizedIntent(ctx context.Context, intent InvocationIntent) (InvocationIntent, error) {
	if ctx == nil {
		return InvocationIntent{}, invalidRuntimeClient("context is required")
	}
	if strings.TrimSpace(intent.CallerIdentity.Principal.URA) == "" {
		caller, err := s.identity.CallerIdentity(ctx)
		if err != nil {
			return InvocationIntent{}, v3SessionError(ErrCallerIdentityUnavailable, "identity", "caller identity unavailable", nil, err)
		}
		intent.CallerIdentity = caller
	}
	if err := validateInvocationIntent(intent); err != nil {
		return InvocationIntent{}, err
	}
	return intent, nil
}

type SessionAbilityOperations struct {
	session *AuthorizedRuntimeSession
}

func (o *SessionAbilityOperations) Resolve(ctx context.Context, intent InvocationIntent) (PreparedInvocationState, error) {
	return o.session.Prepare(ctx, intent)
}

type SessionInvokeOperations struct {
	session *AuthorizedRuntimeSession
}

func (o *SessionInvokeOperations) Submit(ctx context.Context, intent InvocationIntent, opts PrepareOptions) (SubmittedInvocation, error) {
	prepared, err := o.session.Prepare(ctx, intent)
	if err != nil {
		return SubmittedInvocation{}, err
	}
	authorized, err := o.session.Authorize(ctx, prepared)
	if err != nil {
		return SubmittedInvocation{}, err
	}
	signed, err := o.session.Sign(ctx, authorized, opts)
	if err != nil {
		return SubmittedInvocation{}, err
	}
	return o.session.Submit(ctx, signed)
}

func (o *SessionInvokeOperations) Run(ctx context.Context, intent InvocationIntent, opts PrepareOptions) (TerminalReceipt, error) {
	submitted, err := o.Submit(ctx, intent, opts)
	if err != nil {
		return TerminalReceipt{}, err
	}
	return o.session.AwaitTerminal(ctx, submitted)
}

type SessionStreamOperations struct {
	session *AuthorizedRuntimeSession
}

func (o *SessionStreamOperations) Open(ctx context.Context, intent InvocationIntent, opts PrepareOptions) (*StreamHandle, error) {
	signed, err := o.session.signIntent(ctx, intent, opts)
	if err != nil {
		return nil, err
	}
	return o.session.runtime.OpenStream(ctx, signed.Signed)
}

type SessionBidiOperations struct {
	session *AuthorizedRuntimeSession
}

func (o *SessionBidiOperations) Open(ctx context.Context, intent InvocationIntent, opts PrepareOptions, streams []BidiStreamDescriptor) (*BidiSession, error) {
	signed, err := o.session.signIntent(ctx, intent, opts)
	if err != nil {
		return nil, err
	}
	return o.session.runtime.OpenBidi(ctx, signed.Signed, streams)
}

type SessionReceiptOperations struct {
	session *AuthorizedRuntimeSession
}

func (o *SessionReceiptOperations) Get(ctx context.Context, request ReceiptGetRequest) (ReceiptGetResult, error) {
	return o.session.receipts.Get(ctx, request)
}

func (o *SessionReceiptOperations) Trace(ctx context.Context, request ReceiptTraceRequest) (ReceiptTraceResult, error) {
	return o.session.receipts.Trace(ctx, request)
}

type SessionHistoryOperations struct {
	session *AuthorizedRuntimeSession
}

func (o *SessionHistoryOperations) List(ctx context.Context, request ReceiptListRequest) (ReceiptHistoryPage, error) {
	if err := validateSessionHistoryRequest(request); err != nil {
		return ReceiptHistoryPage{}, err
	}
	return o.session.receipts.List(ctx, request)
}

type SessionCancellationOperations struct {
	session *AuthorizedRuntimeSession
}

func (o *SessionCancellationOperations) Cancel(ctx context.Context, submitted SubmittedInvocation, reason string) (InvocationCancel, error) {
	return o.session.runtime.Cancel(ctx, submitted.Handle, reason)
}

func (o *SessionCancellationOperations) Events(ctx context.Context, submitted SubmittedInvocation) (InvocationHandle, error) {
	return o.session.runtime.Events(ctx, submitted.Handle)
}

type SessionDiagnosticsOperations struct {
	session *AuthorizedRuntimeSession
}

func (o *SessionDiagnosticsOperations) Read(ctx context.Context) (map[string]any, error) {
	return o.session.runtime.Diagnostics(ctx)
}

func (s *AuthorizedRuntimeSession) signIntent(ctx context.Context, intent InvocationIntent, opts PrepareOptions) (SignedInvocationState, error) {
	prepared, err := s.Prepare(ctx, intent)
	if err != nil {
		return SignedInvocationState{}, err
	}
	authorized, err := s.Authorize(ctx, prepared)
	if err != nil {
		return SignedInvocationState{}, err
	}
	return s.Sign(ctx, authorized, opts)
}

type RuntimeClientSessionRuntimeProvider struct {
	client *RuntimeClient
}

func NewRuntimeClientSessionRuntimeProvider(client *RuntimeClient) RuntimeClientSessionRuntimeProvider {
	return RuntimeClientSessionRuntimeProvider{client: client}
}

func (p RuntimeClientSessionRuntimeProvider) SubmitSigned(ctx context.Context, signed SignedInvocation) (InvocationHandle, error) {
	return p.client.SubmitSigned(ctx, signed)
}

func (p RuntimeClientSessionRuntimeProvider) PrepareForSigning(ctx context.Context, draft InvocationDraft, opts PrepareOptions) (PreparedInvocation, SigningMaterial, error) {
	return p.client.Prepare(ctx, draft, opts)
}

func (p RuntimeClientSessionRuntimeProvider) AwaitTerminal(ctx context.Context, handle InvocationHandle) (InvocationResult, error) {
	return p.client.Await(ctx, handle)
}

func (p RuntimeClientSessionRuntimeProvider) OpenStream(ctx context.Context, signed SignedInvocation) (*StreamHandle, error) {
	_ = ctx
	_ = signed
	return nil, v3SessionError(
		ErrProviderUnavailable,
		"stream",
		"runtime client adapter does not expose signed stream submission",
		nil,
		nil,
	)
}

func (p RuntimeClientSessionRuntimeProvider) OpenBidi(ctx context.Context, signed SignedInvocation, streams []BidiStreamDescriptor) (*BidiSession, error) {
	_ = ctx
	_ = signed
	_ = streams
	return nil, v3SessionError(
		ErrProviderUnavailable,
		"bidi",
		"runtime client adapter does not expose signed bidi submission",
		nil,
		nil,
	)
}

func (p RuntimeClientSessionRuntimeProvider) Cancel(ctx context.Context, handle InvocationHandle, reason string) (InvocationCancel, error) {
	return p.client.Cancel(ctx, handle, reason)
}

func (p RuntimeClientSessionRuntimeProvider) Events(ctx context.Context, handle InvocationHandle) (InvocationHandle, error) {
	return p.client.Events(ctx, handle)
}

func (p RuntimeClientSessionRuntimeProvider) Diagnostics(ctx context.Context) (map[string]any, error) {
	_ = ctx
	return map[string]any{"runtime_provider": "runtime_client"}, nil
}

type RuntimeClientDescriptorProvider struct {
	client *RuntimeClient
}

func NewRuntimeClientDescriptorProvider(client *RuntimeClient) RuntimeClientDescriptorProvider {
	return RuntimeClientDescriptorProvider{client: client}
}

func (p RuntimeClientDescriptorProvider) ResolveDescriptor(ctx context.Context, request DescriptorResolutionRequest) (DescriptorResolution, error) {
	ref, err := p.client.ResolveDescriptorRef(ctx, RuntimeDescriptorRefRequest{
		CalleeURA:  request.Target.URA,
		Ability:    request.Ability.Name,
		CallMode:   request.CallMode,
		CallerURA:  request.CallerIdentity.Principal.URA,
		SubjectURA: request.Subject.URA,
	})
	if err != nil {
		return descriptorResolutionFromError(err), nil
	}
	return DescriptorResolution{
		State:                 DescriptorResolved,
		DescriptorRef:         ref,
		DescriptorFingerprint: descriptorFingerprint(ref),
	}, nil
}

type StaticCallerIdentityProvider struct {
	Caller CallerIdentityRef
}

func (p StaticCallerIdentityProvider) CallerIdentity(ctx context.Context) (CallerIdentityRef, error) {
	_ = ctx
	if strings.TrimSpace(p.Caller.Principal.URA) == "" {
		return CallerIdentityRef{}, v3SessionError(ErrCallerIdentityUnavailable, "identity", "caller identity unavailable", nil, nil)
	}
	return p.Caller, nil
}

type SystemClockIdempotencySource struct{}

func (SystemClockIdempotencySource) NowUnixMS() int64 {
	return time.Now().UnixMilli()
}

func (SystemClockIdempotencySource) NewIdempotencyKey() (string, error) {
	nonce, err := NewInvocationNonceBase64()
	if err != nil {
		return "", err
	}
	return "idem-" + strings.TrimRight(strings.NewReplacer("+", "-", "/", "_").Replace(nonce), "="), nil
}

func (SystemClockIdempotencySource) NewNonceBase64() (string, error) {
	return NewInvocationNonceBase64()
}

func validateInvocationIntent(intent InvocationIntent) error {
	if err := validatePrincipalRef(intent.CallerIdentity.Principal, "caller identity"); err != nil {
		return err
	}
	if err := validatePrincipalRef(intent.ActingPrincipal.Principal, "acting principal"); err != nil {
		return err
	}
	if strings.TrimSpace(intent.Target.URA) == "" || containsAllZeroPrincipal(intent.Target.URA) {
		return v3SessionError(ErrInvalidInvocation, "intent", "target URA is required", sessionIntentDetails(intent), nil)
	}
	if strings.TrimSpace(intent.Ability.Name) == "" {
		return v3SessionError(ErrInvalidInvocation, "intent", "ability is required", sessionIntentDetails(intent), nil)
	}
	if strings.TrimSpace(intent.Subject.URA) == "" || containsAllZeroPrincipal(intent.Subject.URA) {
		return v3SessionError(ErrInvalidInvocation, "intent", "subject URA is required", sessionIntentDetails(intent), nil)
	}
	if strings.TrimSpace(intent.CallMode) == "" {
		return v3SessionError(ErrInvalidInvocation, "intent", "call mode is required", sessionIntentDetails(intent), nil)
	}
	if intent.DeadlineUnixMS <= 0 {
		return v3SessionError(ErrInvalidInvocation, "intent", "deadline_unix_ms is required", sessionIntentDetails(intent), nil)
	}
	if strings.TrimSpace(intent.IdempotencyKey) == "" {
		return v3SessionError(ErrInvalidInvocation, "intent", "idempotency key is required", sessionIntentDetails(intent), nil)
	}
	if intent.CausalContext == nil {
		return v3SessionError(ErrInvalidInvocation, "intent", "causal context is required", sessionIntentDetails(intent), nil)
	}
	return nil
}

func validatePrincipalRef(ref PrincipalRef, label string) error {
	if strings.TrimSpace(ref.URA) == "" {
		return v3SessionError(ErrCallerIdentityUnavailable, "intent", label+" URA is required", nil, nil)
	}
	if containsAllZeroPrincipal(ref.URA) {
		return v3SessionError(ErrCallerIdentityUnavailable, "intent", label+" must not be all-zero", map[string]any{"principal_ura": ref.URA}, nil)
	}
	return nil
}

func validateDescriptorResolution(resolution DescriptorResolution) error {
	switch resolution.State {
	case DescriptorResolved:
		if strings.TrimSpace(resolution.DescriptorRef) == "" {
			return v3SessionError(ErrDescriptorNotFound, "descriptor", "resolved descriptor omitted descriptor_ref", nil, nil)
		}
		return nil
	case DescriptorNotFound:
		return v3SessionError(ErrDescriptorNotFound, "descriptor", "descriptor not found", map[string]any{"reason": resolution.Reason}, nil)
	case DescriptorOwnerOffline:
		return v3SessionError(ErrDescriptorOwnerOffline, "descriptor", "descriptor owner offline", map[string]any{"reason": resolution.Reason}, nil)
	case DescriptorModeUnsupported:
		return v3SessionError(ErrDescriptorModeUnsupported, "descriptor", "descriptor mode unsupported", map[string]any{"reason": resolution.Reason}, nil)
	case DescriptorStale:
		return v3SessionError(ErrDescriptorStale, "descriptor", "descriptor stale", map[string]any{"reason": resolution.Reason}, nil)
	case DescriptorUnavailable, "":
		return v3SessionError(ErrProviderUnavailable, "descriptor", "descriptor provider unavailable", map[string]any{"reason": resolution.Reason}, nil)
	default:
		return v3SessionError(ErrProviderUnavailable, "descriptor", "descriptor provider returned unknown state", map[string]any{"state": string(resolution.State)}, nil)
	}
}

func validateAuthorizedRuntimeBinding(artifact AuthorityArtifact, prepared PreparedInvocationState) error {
	details := sessionPreparedDetails(prepared)
	details["authority_session_subject"] = artifact.Subject.URA
	details["owner_principal"] = artifact.Owner.URA
	switch authority := artifact.Authority.(type) {
	case DelegationProof:
		return validateDelegationAuthorityForSession(authority, prepared, details)
	case *DelegationProof:
		if authority == nil {
			return v3SessionError(ErrAuthorityDenied, "authorize", "delegation authority is required", details, nil)
		}
		return validateDelegationAuthorityForSession(*authority, prepared, details)
	case SessionAuthority:
		return validateSessionAuthorityForSession(authority, prepared, details)
	case *SessionAuthority:
		if authority == nil {
			return v3SessionError(ErrAuthorityDenied, "authorize", "session authority is required", details, nil)
		}
		return validateSessionAuthorityForSession(*authority, prepared, details)
	default:
		return v3SessionError(ErrAuthorityDenied, "authorize", "unsupported authority artifact", details, nil)
	}
}

func validateDelegationAuthorityForSession(authority DelegationProof, prepared PreparedInvocationState, details map[string]any) error {
	intent := prepared.Intent
	details["authority_session_subject"] = authority.SubjectURA
	if strings.TrimSpace(authority.CallerURA) != strings.TrimSpace(intent.CallerIdentity.Principal.URA) {
		return v3SessionError(ErrAuthorityDenied, "authorize", "authority caller does not match caller identity", details, nil)
	}
	if strings.TrimSpace(authority.SubjectURA) != strings.TrimSpace(intent.Subject.URA) {
		return v3SessionError(ErrAuthoritySubjectMismatch, "authorize", "authority subject does not admit invocation subject", details, nil)
	}
	if !authority.MatchesAudience(intent.Target.URA) || !authority.MatchesScope(intent.Ability.Name) {
		return v3SessionError(ErrAuthorityDenied, "authorize", "authority does not admit target or ability", details, nil)
	}
	return nil
}

func validateSessionAuthorityForSession(authority SessionAuthority, prepared PreparedInvocationState, details map[string]any) error {
	intent := prepared.Intent
	details["authority_session_subject"] = authority.SubjectURA
	if strings.TrimSpace(authority.IssuerURA) != strings.TrimSpace(intent.CallerIdentity.Principal.URA) {
		return v3SessionError(ErrAuthorityDenied, "authorize", "authority issuer does not match caller identity", details, nil)
	}
	if strings.TrimSpace(authority.CalleeURA) != strings.TrimSpace(intent.Target.URA) {
		return v3SessionError(ErrAuthorityDenied, "authorize", "authority target does not match invocation target", details, nil)
	}
	if !runtimeSessionAuthorityAdmitsSubject(&authority, intent.Subject.URA) {
		return v3SessionError(ErrAuthoritySubjectMismatch, "authorize", "authority subject does not admit invocation subject", details, nil)
	}
	if !authority.MatchesAudience(intent.Target.URA) || !authority.MatchesScope(intent.Ability.Name) {
		return v3SessionError(ErrAuthorityDenied, "authorize", "authority does not admit target or ability", details, nil)
	}
	return nil
}

func descriptorRequestFromIntent(intent InvocationIntent) DescriptorResolutionRequest {
	return DescriptorResolutionRequest{
		CallerIdentity:  intent.CallerIdentity,
		ActingPrincipal: intent.ActingPrincipal,
		Target:          intent.Target,
		Ability:         intent.Ability,
		Subject:         intent.Subject,
		CallMode:        intent.CallMode,
		DeadlineUnixMS:  intent.DeadlineUnixMS,
		IdempotencyKey:  intent.IdempotencyKey,
		CausalContext:   copyMap(intent.CausalContext),
	}
}

func validateSessionHistoryRequest(request ReceiptListRequest) error {
	if err := validateSessionHistoryRuntimeCall(request.Call); err != nil {
		return err
	}
	return validateSessionHistoryFilterBinding(request.Call, request.Filter)
}

func validateSessionHistoryRuntimeCall(call RuntimeCallContext) error {
	if err := validateRuntimeCallContext(call); err != nil {
		return err
	}
	authority, err := runtimeCallAuthority(call)
	if err != nil {
		return err
	}
	if authority == nil {
		return v3SessionError(
			ErrAuthorityDenied,
			"history",
			"session history requires runtime authority bound to the receipt query tuple",
			runtimeCallDetails(call),
			nil,
		)
	}
	return validateSessionHistoryAuthorityBinding(authority, call)
}

func validateSessionHistoryFilterBinding(call RuntimeCallContext, filter ReceiptFilter) error {
	callerURA := strings.TrimSpace(call.CallerURA)
	calleeURA := strings.TrimSpace(call.CalleeURA)
	details := runtimeCallDetails(call)
	if filterCaller := strings.TrimSpace(filter.CallerURA); filterCaller != "" && filterCaller != callerURA {
		details["filter_caller_ura"] = filter.CallerURA
		return v3SessionError(
			ErrAuthorityDenied,
			"history",
			"receipt filter caller_ura does not match receipt query caller_ura",
			details,
			nil,
		)
	}
	if filterCallee := strings.TrimSpace(filter.CalleeURA); filterCallee != "" && filterCallee != calleeURA {
		details["filter_callee_ura"] = filter.CalleeURA
		return v3SessionError(
			ErrAuthorityDenied,
			"history",
			"receipt filter callee_ura does not match receipt query callee_ura",
			details,
			nil,
		)
	}
	// Subject filters are receipt-query predicates, not the authority subject.
	// The session authority remains bound to call.SubjectURA above; the daemon
	// receives SubjectURAs only as exact ledger filters after admission.
	return nil
}

func runtimeCallAuthority(call RuntimeCallContext) (RuntimeInvocationAuthority, error) {
	metadata := cloneAbilityMetadata(call.Metadata)
	if err := validateAuthorityMetadata(metadata); err != nil {
		return nil, err
	}
	rawPresent := rawRuntimeAuthorityPresent(metadata)
	if call.Authority != nil {
		if rawPresent {
			return nil, invalidRuntimePayload(
				"runtime call authority must be supplied once as a typed authority or metadata, not both",
				nil,
			)
		}
		return call.Authority, nil
	}
	return runtimeAuthorityFromMetadata(metadata)
}

func validateSessionHistoryAuthorityBinding(
	authority RuntimeInvocationAuthority,
	call RuntimeCallContext,
) error {
	callerURA := strings.TrimSpace(call.CallerURA)
	calleeURA := strings.TrimSpace(call.CalleeURA)
	subjectURA := strings.TrimSpace(call.SubjectURA)
	details := runtimeCallDetails(call)
	switch typed := authority.(type) {
	case DelegationProof:
		return validateSessionHistoryDelegationBinding(&typed, callerURA, calleeURA, subjectURA, details)
	case *DelegationProof:
		return validateSessionHistoryDelegationBinding(typed, callerURA, calleeURA, subjectURA, details)
	case SessionAuthority:
		return validateSessionHistorySessionBinding(&typed, callerURA, calleeURA, subjectURA, details)
	case *SessionAuthority:
		return validateSessionHistorySessionBinding(typed, callerURA, calleeURA, subjectURA, details)
	default:
		return invalidRuntimePayload("runtime call authority has an unsupported canonical type", nil)
	}
}

func validateSessionHistoryDelegationBinding(
	proof *DelegationProof,
	callerURA string,
	calleeURA string,
	subjectURA string,
	details map[string]any,
) error {
	if proof == nil {
		return v3SessionError(ErrAuthorityDenied, "history", "delegation authority is required", details, nil)
	}
	if strings.TrimSpace(proof.CallerURA) != callerURA {
		return v3SessionError(ErrAuthorityDenied, "history", "delegation authority caller does not match receipt query caller_ura", details, nil)
	}
	if strings.TrimSpace(proof.SubjectURA) != subjectURA {
		return v3SessionError(ErrAuthoritySubjectMismatch, "history", "delegation authority subject does not match receipt query subject_ura", details, nil)
	}
	if !proof.MatchesAudience(calleeURA) {
		return v3SessionError(ErrAuthorityDenied, "history", "delegation authority audience does not admit receipt query callee_ura", details, nil)
	}
	if !proof.MatchesScope(receiptHistoryListAbility) {
		return v3SessionError(ErrAuthorityDenied, "history", "delegation authority scopes do not admit invocation.history.list", details, nil)
	}
	return nil
}

func validateSessionHistorySessionBinding(
	authority *SessionAuthority,
	callerURA string,
	calleeURA string,
	subjectURA string,
	details map[string]any,
) error {
	if authority == nil {
		return v3SessionError(ErrAuthorityDenied, "history", "session authority is required", details, nil)
	}
	details["authority_session_subject"] = authority.SubjectURA
	if strings.TrimSpace(authority.IssuerURA) != callerURA {
		return v3SessionError(ErrAuthorityDenied, "history", "session authority issuer does not match receipt query caller_ura", details, nil)
	}
	if strings.TrimSpace(authority.CalleeURA) != calleeURA {
		return v3SessionError(ErrAuthorityDenied, "history", "session authority callee does not match receipt query callee_ura", details, nil)
	}
	if !authority.MatchesAudience(calleeURA) {
		return v3SessionError(ErrAuthorityDenied, "history", "session authority audience does not admit receipt query callee_ura", details, nil)
	}
	if !runtimeSessionAuthorityAdmitsSubject(authority, subjectURA) {
		return v3SessionError(ErrAuthoritySubjectMismatch, "history", "session authority subject does not admit receipt query subject_ura", details, nil)
	}
	if !authority.MatchesScope(receiptHistoryListAbility) {
		return v3SessionError(ErrAuthorityDenied, "history", "session authority scopes do not admit invocation.history.list", details, nil)
	}
	return nil
}

func runtimeCallDetails(call RuntimeCallContext) map[string]any {
	return map[string]any{
		"caller_ura":  call.CallerURA,
		"callee_ura":  call.CalleeURA,
		"subject_ura": call.SubjectURA,
	}
}

func runtimeSessionIntentMetadata(intent InvocationIntent, resolution DescriptorResolution) map[string]any {
	metadata := copyMap(intent.Metadata)
	if metadata == nil {
		metadata = map[string]any{}
	}
	metadata["canonical_runtime_session"] = map[string]any{
		"state":                  string(RuntimeSessionPrepared),
		"caller_ura":             intent.CallerIdentity.Principal.URA,
		"acting_principal_ura":   intent.ActingPrincipal.Principal.URA,
		"target_ura":             intent.Target.URA,
		"ability":                intent.Ability.Name,
		"subject_ura":            intent.Subject.URA,
		"subject_derivation":     intent.Subject.DerivationRule,
		"call_mode":              intent.CallMode,
		"deadline_unix_ms":       intent.DeadlineUnixMS,
		"idempotency_key":        intent.IdempotencyKey,
		"descriptor_ref":         resolution.DescriptorRef,
		"descriptor_fingerprint": resolution.DescriptorFingerprint,
		"owner_principal_ura":    resolution.OwnerPrincipal.URA,
	}
	return metadata
}

func rebuildDraftWithMetadata(draft InvocationDraft, metadata map[string]any) (InvocationDraft, error) {
	builder := NewInvocationBuilder().
		WithCallerURA(draft.CallerURA()).
		WithCalleeURA(draft.CalleeURA()).
		WithDescriptorRef(draft.DescriptorRef()).
		WithSubjectURA(draft.SubjectURA()).
		WithNonceBase64(draft.NonceBase64()).
		WithCausalContext(draft.CausalContext()).
		WithContentType(draft.ContentType()).
		WithMetadata(metadata)
	if draft.HasJSONArgs() {
		builder.WithJSONArgs(draft.JSONArgs())
	} else {
		builder.WithArgumentsBase64(draft.ArgumentsBase64())
	}
	return builder.Build()
}

func runtimeSessionContentType(intent InvocationIntent) string {
	if strings.TrimSpace(intent.ContentType) != "" {
		return intent.ContentType
	}
	return "application/json"
}

func preparationFingerprint(prepared PreparedInvocationState) string {
	payload := map[string]any{
		"caller":                 prepared.Intent.CallerIdentity.Principal.URA,
		"acting_principal":       prepared.Intent.ActingPrincipal.Principal.URA,
		"target":                 prepared.Intent.Target.URA,
		"ability":                prepared.Intent.Ability.Name,
		"subject":                prepared.Intent.Subject.URA,
		"call_mode":              prepared.Intent.CallMode,
		"deadline_unix_ms":       prepared.Intent.DeadlineUnixMS,
		"idempotency_key":        prepared.Intent.IdempotencyKey,
		"descriptor_ref":         prepared.DescriptorRef,
		"descriptor_fingerprint": prepared.DescriptorFingerprint,
		"causal_context":         prepared.Intent.CausalContext,
	}
	raw, _ := json.Marshal(payload)
	sum := sha256.Sum256(raw)
	return hex.EncodeToString(sum[:])
}

func descriptorFingerprint(ref string) string {
	sum := sha256.Sum256([]byte(strings.TrimSpace(ref)))
	return hex.EncodeToString(sum[:])
}

func descriptorResolutionFromError(err error) DescriptorResolution {
	var sdkErr *SDKError
	if !IsCode(err, ErrAbilityNotFound) && !IsCode(err, ErrNotFound) &&
		!IsCode(err, ErrRouteUnavailable) && !IsCode(err, ErrRuntimeRouteUnavailable) &&
		!IsCode(err, ErrDescriptorOwnerOffline) && !IsCode(err, ErrDescriptorModeUnsupported) &&
		!IsCode(err, ErrDescriptorStale) {
		if !strings.Contains(fmt.Sprint(err), "offline") {
			return DescriptorResolution{State: DescriptorUnavailable, Reason: fmt.Sprint(err)}
		}
	}
	if IsCode(err, ErrDescriptorOwnerOffline) || strings.Contains(strings.ToLower(fmt.Sprint(err)), "offline") {
		return DescriptorResolution{State: DescriptorOwnerOffline, Reason: fmt.Sprint(err)}
	}
	if IsCode(err, ErrRouteUnavailable) || IsCode(err, ErrRuntimeRouteUnavailable) {
		return DescriptorResolution{State: DescriptorUnavailable, Reason: fmt.Sprint(err)}
	}
	if IsCode(err, ErrAbilityNotFound) || IsCode(err, ErrNotFound) {
		return DescriptorResolution{State: DescriptorNotFound, Reason: fmt.Sprint(err)}
	}
	if errors.As(err, &sdkErr) && sdkErr.Code == ErrDescriptorModeUnsupported {
		return DescriptorResolution{State: DescriptorModeUnsupported, Reason: sdkErr.Message}
	}
	if errors.As(err, &sdkErr) && sdkErr.Code == ErrDescriptorStale {
		return DescriptorResolution{State: DescriptorStale, Reason: sdkErr.Message}
	}
	return DescriptorResolution{State: DescriptorUnavailable, Reason: fmt.Sprint(err)}
}

func sessionIntentDetails(intent InvocationIntent) map[string]any {
	return map[string]any{
		"caller":           intent.CallerIdentity.Principal.URA,
		"acting_principal": intent.ActingPrincipal.Principal.URA,
		"target":           intent.Target.URA,
		"ability":          intent.Ability.Name,
		"subject":          intent.Subject.URA,
		"call_mode":        intent.CallMode,
		"idempotency_key":  intent.IdempotencyKey,
	}
}

func sessionPreparedDetails(prepared PreparedInvocationState) map[string]any {
	details := sessionIntentDetails(prepared.Intent)
	details["descriptor_ref"] = prepared.DescriptorRef
	details["descriptor_fingerprint"] = prepared.DescriptorFingerprint
	details["preparation_fingerprint"] = prepared.PreparationFingerprint
	details["owner_principal"] = prepared.OwnerPrincipal.URA
	return details
}

func sessionAuthorizedDetails(authorized AuthorizedInvocation) map[string]any {
	details := sessionPreparedDetails(authorized.Prepared)
	details["authority_artifact_fingerprint"] = authorized.Artifact.Fingerprint
	details["authority_session_subject"] = authorized.Artifact.Subject.URA
	return details
}

func v3SessionError(code ErrorCode, stage string, message string, details map[string]any, cause error) error {
	if details == nil {
		details = map[string]any{}
	}
	return &SDKError{
		Code:      code,
		Stage:     stage,
		Retry:     RetryNever,
		Retryable: false,
		Message:   message,
		Details:   details,
		Cause:     cause,
	}
}
