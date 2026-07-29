# Canonical Runtime Hardening And Redundancy Retirement V1

Status: Normative follow-up to `canonical-runtime-convergence-v2.md`.

This specification does not change the architecture direction defined by
Canonical Runtime Convergence V2. It closes the remaining engineering risk
after the primary convergence gates are green: duplicated runtime authority,
large mixed-responsibility modules, language-specific SDK validation drift, and
product paths that are not yet proven by mutation tests.

## 1. Goal

Move the current implementation from gate-conformant convergence to stable
operational convergence.

The target state is:

- one descriptor resolution authority;
- one descriptor-bound invocation admission model;
- one provider-managed signing custody rule set across all language SDKs;
- one canonical receipt proof-fact verifier contract;
- typed route and admission decisions instead of message-string business
  classification;
- product tests proving that a user operation produces one invocation, one
  admission checkpoint, and one terminal signed receipt chain; and
- clear module ownership that prevents FFI, SDK facade, product UI, or plugin
  helper code from becoming a second runtime model.

## 2. Non-Goals

This specification does not authorize:

- EasyNet-specific or EasyRemote-specific SDK concepts;
- product lifecycle inside the canonical SDK;
- product directory, route, or receipt models inside the canonical SDK;
- compatibility layers that preserve retired runtime architecture;
- fallback paths whose only purpose is preserving old behavior;
- public API deletion without the existing canonical public API process; or
- any semantic identity terminology other than URA.

## 3. Required Invariants

Every implementation change under this specification must preserve these
invariants.

1. Every invocation reaches a deterministic terminal state or an explicitly
   typed pre-dispatch rejection.
2. Admission and terminal checkpoints are monotonic, ordered, and verifiable.
3. Descriptor-bound dispatch without a registered descriptor proof fails
   closed.
4. Provider-managed signing without custody facts fails closed.
5. Route negative outcomes are typed and do not require substring inspection.
6. FFI performs ABI and DTO translation only. It does not own runtime business
   decisions.
7. Language SDKs expose syntax-specific facades over one shared runtime
   capability matrix.
8. Product UI consumes verified runtime projections. It does not reinterpret
   canonical receipt state.
9. Tests may retain retired strings only as negative fixtures proving rejection.

## 4. Workstream A: Current Evidence Closure

### Problem

The primary convergence gate can pass while SDK runner reports remain stale or
the checkout contains uncommitted cross-language validation updates.

### Required Changes

- Complete provider-managed signer policy validation in Go, Python, Node, Java,
  and Rust-facing runtime paths.
- Add negative tests for missing `signer_id`, missing `policy_ref`, mismatched
  signer identity, and malformed provider-managed policy facts.
- Refresh canonical public API and runner evidence after implementation.
- Keep report evidence source references pinned to the actual implementation
  that enforces the case.

### Acceptance Gates

```bash
cargo fmt --check
tools/scripts/check-sdk-canonical-public-api.sh
tools/scripts/check-canonical-runtime-convergence-v2.sh
tools/scripts/check-architecture-convergence.sh
tools/scripts/check-sdk-conformance-reports.sh
```

## 5. Workstream B: Descriptor Resolution Authority

### Problem

The FFI invocation layer still contains descriptor catalog construction and
descriptor lookup logic. That gives the ABI bridge enough business knowledge to
diverge from the daemon runtime authority.

### Target Architecture

```text
SDK or FFI caller
  -> RuntimeDescriptorResolutionClient
  -> RuntimeDescriptorProvider
  -> LocalRuntime, route authority, and descriptor catalog owner
```

### Required Changes

- Introduce a runtime-owned descriptor resolution provider.
- Keep pure descriptor reference normalization in the descriptor reference
  module.
- Move descriptor catalog selection, owner matching, and call-mode lookup out
  of FFI.
- Make FFI parse the request DTO, call the provider, and map typed results to
  ABI JSON.
- Delete FFI-local catalog construction and catalog row resolution helpers after
  callers migrate.

### Required Result Types

Descriptor resolution must return a typed result:

- `Resolved`
- `NotFound`
- `OwnerOffline`
- `OwnerMismatch`
- `CallModeUnsupported`
- `RuntimeOwnerUnavailable`
- `InvalidRequest`

### Boundary Gate

Add a script that fails if FFI invocation code calls system registry builders,
descriptor catalog constructors, or local catalog row resolution helpers.

### Tests

- Local owner descriptor resolution succeeds through the provider.
- Realm descriptor resolution succeeds through the provider.
- Missing call mode fails before catalog lookup.
- Owner mismatch fails before route lookup.
- Local catalog miss does not probe remote state.
- Remote owner offline is not downgraded to ability absence.

## 6. Workstream C: Provider-Managed Signing Custody

### Problem

Provider-managed signing validation exists in multiple language SDKs and can
drift unless all implementations are pinned to the same conformance case.

### Required Rules

When `mode` is `provider_managed_signing`:

- `signer_id` is required and non-empty;
- `policy_ref` is required and non-empty;
- `policy_ref` is bound to owner URA, key identity, and public key material;
- process-local signer fallback is prohibited; and
- SDK helpers must not synthesize signing custody facts.

### Required Changes

- Add a single conformance case for provider-managed signer policy custody.
- Link Go, Python, Node, Java, Swift, Rust, and C ABI evidence to the same case
  where the language exposes the capability.
- Reject stale public API or runner evidence when signer policy source changes.
- Remove permissive null-object policy parsing for provider-managed mode.

### Tests

- Missing `signer_id` rejected.
- Missing `policy_ref` rejected.
- Wrong `policy_ref` rejected.
- Wrong signer identity rejected.
- Valid provider-managed signing policy accepted.
- Public SDK inventory contains no process-local signer fallback.

## 7. Workstream D: Receipt Proof-Fact Verifier

### Problem

Receipt decoding and projection exist across FFI, daemon local transport, and
language SDKs. The rule set must be one canonical verifier contract, not
language-specific interpretation.

### Target Architecture

```text
Raw receipt projection
  -> CanonicalReceiptVerifier
  -> VerifiedReceiptProjection
  -> SDK or product presentation
```

### Required Rules

- Terminal receipt is required for completed invocation results.
- Terminal state in the result must match terminal receipt state.
- Admission checkpoint must precede terminal checkpoint.
- Mandatory proof facts must be present.
- Retired receipt aliases and noncanonical fields are rejected.
- Product UI reads verified projections only.

### Required Changes

- Centralize the receipt proof-fact contract in conformance cases.
- Ensure each language SDK rejects missing proof facts.
- Ensure Java, Node, Go, Python, Swift, Rust, and C ABI evidence names the
  exact source enforcing the case.
- Remove or quarantine any raw receipt canonicalizer that can bypass proof-fact
  validation.

### Tests

- Missing authority proof rejected.
- Missing descriptor hash rejected.
- Missing schema hash rejected.
- Missing implementation hash rejected.
- Terminal state mismatch rejected.
- Terminal-before-admission rejected.
- Retired receipt aliases rejected.
- Invocation history displays only verified terminal receipt chains.

## 8. Workstream E: Admission Facade Decomposition

### Problem

Admission currently concentrates descriptor binding, trust, principal lifecycle,
access control, quota, authority proof, and error mapping in one large facade.
The facade is a valid runtime boundary, but the internal responsibilities are
too broad.

### Target Components

```text
AdmissionFacade
  -> DescriptorAdmissionPolicy
  -> PrincipalLifecycleGate
  -> AuthorityProofVerifier
  -> AccessControlAdmissionGate
  -> QuotaAdmissionGate
  -> AdmissionErrorMapper
```

### Required Rules

- `AdmissionFacade` orchestrates. It does not directly own all checks.
- Each gate returns a typed decision.
- Admission failure preserves target stage and target reason.
- No public governance read bypass is allowed.
- No second core admission authority is allowed.

### Delivery Order

1. Extract error mapping.
2. Extract descriptor admission policy.
3. Extract authority proof verifier.
4. Extract lifecycle gate.
5. Extract access and quota gates.
6. Remove obsolete helper paths.

### Tests

- Authority subject mismatch preserves typed reason.
- Missing caller signer fails at authority stage.
- Inactive principal rejected.
- Quota denial rejected.
- Descriptor-bound proof missing rejected.
- RPC, stream, bidi, and local runtime paths consume the same admission
  decision shape.

## 9. Workstream F: Typed Route State Machine

### Problem

Route resolution is a central authority, but negative route outcomes still risk
being handled as message strings by downstream code.

### Required State Machine

```text
RouteResolution
  -> Resolved(SelectedInvokeRoute)
  -> Negative(RouteNegative)

RouteNegative
  -> OwnerOffline
  -> AbilityNotFound
  -> DescriptorOwnerMismatch
  -> CallModeUnsupported
  -> NamespaceUnavailable
  -> AuthorityDenied
```

### Required Rules

- Route negative outcomes are typed at the source.
- Owner offline is not ability absence.
- Caller signer missing is not ability absence.
- Descriptor owner mismatch fails before route lookup.
- Product UI receives stable error classes and target reasons.
- LocalRuntime remains the only local finalization authority.

### Tests

- Owner offline maps to route unavailable.
- Ability absence maps to ability not found.
- Caller signer missing maps to identity readiness error.
- Public ingress requires a complete invocation tuple.
- Local exact route submits once.
- Invocation history observes one finalized signed receipt chain.

## 10. Workstream G: Product Mutation Tests

### Problem

Architecture gates prove boundaries. They do not fully prove user-facing product
paths such as device ability listing, browser session open, media bidi, and
invocation history.

### Required Product Proofs

- Device ability listing resolves `meta.list_abilities` through descriptor
  provider readiness.
- `invocation.history.list` uses a valid caller signer and subject.
- Browser open-session route is visible only after descriptor readiness.
- Media bidi declares exactly one transport and no transport fallback.
- Cancel, timeout, close-send, stream terminal, and bidi terminal paths produce
  deterministic terminal results.
- One product operation submits one canonical invocation and one finalized
  signed receipt chain.

### Tests

- Docker media bidi e2e.
- Device ability list e2e.
- Invocation history mutation test.
- Browser remote session smoke.
- Cancel, disconnect, timeout, retry e2e.

## 11. Workstream H: Module Ownership Cleanup

### Problem

Several modules are large enough to hide mixed responsibility and accidental
second authority paths.

### High-Risk Modules

- FFI invocation bridge
- ability dispatch registry
- agent lifecycle builtin
- admission facade
- route resolver
- Node runtime SDK facade
- Go runtime SDK facade
- Python runtime SDK facade

### Required Rules

- Split only when it removes a real responsibility mix.
- Do not create utility dumping grounds.
- New modules must have one semantic owner.
- Delete obsolete helpers immediately after migration.

### Preferred Ownership Boundaries

- ABI bridge
- descriptor resolution provider
- admission policy
- route decision
- receipt verification
- signing custody validation
- SDK facade
- product presentation

## 12. Delivery Order

1. Close current evidence and signer policy changes.
2. Cut descriptor resolution authority out of FFI.
3. Lock provider-managed signing custody across SDKs.
4. Lock receipt proof facts across SDKs.
5. Convert route negatives to typed state.
6. Decompose admission internals.
7. Add product mutation tests.
8. Clean large modules after authority paths have moved.

## 13. Completion Definition

The work is complete only when all of the following are true:

- FFI does not own descriptor catalog lookup.
- SDKs do not diverge on provider-managed signing policy.
- Receipt proof facts cannot be bypassed by a language-specific canonicalizer.
- Route negative business logic is typed.
- Product UI does not reinterpret receipt state.
- Product e2e proves descriptor readiness, signer readiness, route visibility,
  invocation history, media bidi terminality, and single receipt chain behavior.
- The primary convergence gate, architecture gate, SDK public API gate, SDK
  report gate, formatting, and targeted product e2e all pass.

## 14. Estimated Work

- Current evidence and signer policy closure: 0.5 to 1 day.
- Descriptor authority cutover: 1 to 2 days.
- Signing and receipt conformance closure: 1 to 2 days.
- Typed route state and product e2e: 2 to 4 days.
- Admission and module cleanup: 2 to 4 days.

Expected total: 5 to 10 engineering days. If product e2e exposes daemon
lifecycle or key-service provisioning defects, reserve up to 2 weeks.
