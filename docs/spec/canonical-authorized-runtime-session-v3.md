# Canonical Authorized Runtime Session V3

Status: Normative for SDK/runtime/product cutover work after Canonical Runtime
Convergence V2.

This specification defines the canonical authorized runtime session model for
Axon SDK implementations and all downstream products. It is a contract for
architecture convergence, not a feature request list. EasyNet, EasyRemote,
plugins, sidecars, and future products consume this runtime model. They do not
define it.

The immediate production failure motivating this version is an invocation
history request rejected with `AUTHORITY_SUBJECT_MISMATCH`. The observed request
used a session subject owned by the all-zero user while the envelope subject was
the target device. That is not a product smoke-test problem. It is an SDK and
runtime session-model defect: caller identity, acting principal, subject,
target, and authority session were assembled by separate layers and could drift.

This document upgrades the SDK contract so product code cannot accidentally
construct that invalid shape.

## 1. Goal

The SDK must expose one product-neutral authorized runtime session model across
Rust, C ABI, Go, Python, Node, Java, and Swift.

The model must make these facts explicit before any invocation is submitted:

- who is calling;
- what principal is acting;
- what target is being invoked;
- what ability is requested;
- what subject the authority decision is about;
- what descriptor and proof facts bind the call; and
- what signer or delegated key custody is used.

Products may expose ergonomic APIs, but they must delegate to this model. They
must not reconstruct authorization, descriptor resolution, route selection,
receipt interpretation, or lifecycle state in product code.

## 2. Non-Goals

This specification does not add EasyNet-specific SDK abstractions.

This specification does not add EasyRemote-specific SDK abstractions.

This specification does not preserve legacy runtime paths for compatibility
unless they are edge adapters that enter the canonical authorized session path
before dispatch.

This specification does not allow product code to default missing caller,
subject, target, descriptor, signer, or authority fields.

## 3. Core Ownership

Axon owns the canonical runtime contract:

- principal and target reference types;
- invocation intent and prepared invocation state;
- authority session materialization;
- descriptor-bound proof construction;
- signer custody interface;
- runtime submission state machine;
- terminal receipt model;
- stream and bidi lifecycle model;
- cross-language conformance vectors; and
- public SDK capability matrix.

Products own product policy:

- deciding which product operation maps to which ability;
- selecting the caller identity from product authentication context;
- selecting the target and subject from product state;
- deciding whether a product feature is visible or enabled;
- rendering product receipt history; and
- handling product-specific user experience after terminal runtime outcomes.

Products do not own canonical runtime semantics.

## 4. Required Domain Model

Every SDK implementation must expose semantically equivalent domain types. Names
may follow language convention, but the public meaning must remain identical.

`PrincipalRef`

Identifies an authenticated principal. It must never silently collapse to an
anonymous, all-zero, daemon, device, or authority principal.

`CallerIdentityRef`

Identifies the caller whose signer or delegated signing authority is used for
the invocation. A caller identity can be a user, service, device, or system
principal. It must be explicit.

`ActingPrincipalRef`

Identifies the principal acting inside the request. It may equal the caller, or
it may be a delegated actor authorized by policy. Delegation must be represented
by an authorization artifact, not by changing the caller after preparation.

`RuntimeTargetRef`

Identifies the target runtime object. A device, service, ability host, local
runtime, or remote runtime can be a target. The target is not the caller and is
not automatically the subject.

`AbilityRef`

Identifies the requested ability. The ability reference must resolve through
the canonical descriptor provider before submission.

`SubjectRef`

Identifies the object the authority decision is about. A subject must be
declared by the caller or derived by a named policy before authorization. The
derivation rule must be inspectable in conformance output.

`InvocationIntent`

The product-level request before descriptor resolution and authority
materialization. It contains caller, acting principal, target, ability, subject,
 call mode, arguments, deadline, idempotency key, and causal context.

`PreparedInvocation`

The descriptor-resolved request. It contains the complete invocation tuple,
resolved descriptor, canonical capability facts, and a stable preparation
fingerprint.

`AuthorizedInvocation`

The prepared request plus authority session, authorization artifact, signer
binding, and admission facts required for dispatch.

`SignedInvocation`

The authorized request plus descriptor-bound proof and signer evidence.

`SubmittedInvocation`

The signed request after runtime acceptance. It exposes a queryable invocation
handle and lifecycle state.

`TerminalReceipt`

The only successful or failed terminal representation of an invocation. It must
include authority facts, descriptor facts, signer facts, runtime facts, causal
facts, and final status.

## 5. Canonical State Machine

All language SDKs must implement the same state machine.

```text
Intent
  -> Prepared
  -> Authorized
  -> Signed
  -> Submitted
  -> Terminal
```

Invalid transitions must be impossible through typed high-level APIs and must
return stable errors through low-level APIs.

`Intent`

The caller has expressed a product-neutral runtime request. No descriptor,
authority session, proof, or receipt exists.

`Prepared`

The SDK has resolved the target ability descriptor and frozen the complete
tuple. Caller, acting principal, target, ability, subject, call mode, deadline,
idempotency key, and causal context can no longer be mutated.

`Authorized`

The SDK has materialized an authority session for the frozen tuple. The
authority session subject must admit the invocation subject. A mismatch is a
pre-dispatch error.

`Signed`

The SDK has obtained a signer or delegated signing result for the caller
identity and has produced descriptor-bound proof. No process-local signer may be
created as fallback.

`Submitted`

The runtime has accepted the signed invocation. Cancellation, timeout,
streaming, bidi, retry, and history behavior are now managed by the runtime
state machine.

`Terminal`

The runtime has produced exactly one terminal receipt. Terminal receipts are
idempotently queryable and replay-safe.

## 6. Authority Binding Rules

Authority binding is tuple-bound. It is not a detached permission check.

An authority session must bind:

- caller identity;
- acting principal;
- target;
- ability;
- subject;
- call mode;
- descriptor fingerprint;
- deadline policy;
- causal context; and
- idempotency key.

The runtime must reject before dispatch if any of these fields diverge between
the prepared invocation, authority artifact, signed proof, and submitted
envelope.

The all-zero principal is not a valid default. It may only appear in explicit
test vectors that declare it as a negative fixture.

## 7. Signer Custody

The SDK must not generate hidden signing material.

Allowed signer sources:

- caller key material already provisioned in the local key service;
- delegated signing artifact issued by an authority provider;
- system signer for an explicit system principal; and
- test signer fixture enabled only in test configuration.

Disallowed signer sources:

- automatic default signer creation;
- process-local cached fallback signer;
- target-as-caller substitution;
- authority-as-caller substitution;
- anonymous caller substitution; and
- product-specific signer shortcuts inside canonical SDK packages.

Missing signer material must produce a stable `CALLER_SIGNER_UNAVAILABLE`
error before descriptor-bound remote invocation is attempted.

## 8. Descriptor Resolution

Descriptor resolution is part of preparation. It is not a product route lookup.

The descriptor provider must return one of these states:

- `Resolved`: descriptor exists and is usable for the requested call mode;
- `NotFound`: descriptor does not exist;
- `OwnerOffline`: descriptor owner is not reachable;
- `ModeUnsupported`: descriptor exists but does not support the call mode;
- `Stale`: descriptor exists but failed freshness or version policy; or
- `Unavailable`: provider could not answer within bounded runtime policy.

Product code may render these states, but it must not reinterpret them as
receipt states or authorization states.

## 9. Runtime Session API

Each language SDK must expose a high-level `AuthorizedRuntimeSession` equivalent.

The session constructor must require:

- runtime provider;
- descriptor provider;
- authorization provider;
- signer provider;
- receipt provider;
- caller identity source; and
- clock/idempotency source.

The high-level API must provide typed operation groups over generic runtime
concepts. Product-specific names are not allowed in canonical packages.

Required operation groups:

- `abilities`: list and resolve ability descriptors;
- `invoke`: unary invocation;
- `streams`: server stream lifecycle;
- `bidi`: bidirectional session lifecycle;
- `receipts`: receipt query and verification;
- `history`: invocation history query through canonical receipt provider;
- `cancellation`: cancellation request and acknowledgement; and
- `diagnostics`: read-only runtime diagnostics.

An SDK may expose language-native builders, decorators, async iterators, futures,
or fluent APIs. They must all lower into the same state machine.

## 10. Provider Interfaces

Provider boundaries must be high cohesion and product-neutral.

`RuntimeProvider`

Owns runtime submission, cancellation, stream transport, bidi transport, and
terminal lifecycle observation.

`DescriptorProvider`

Owns descriptor resolution and descriptor freshness policy.

`AuthorizationProvider`

Owns authorization decision and artifact creation for the frozen tuple.

`SignerProvider`

Owns caller signer lookup, delegated signing, and system signer access.

`ReceiptProvider`

Owns receipt verification, history query, and proof-fact validation.

`IdentityProvider`

Owns caller identity discovery from host environment. It must return explicit
absence rather than synthetic identity.

No provider may depend on EasyNet or EasyRemote package names in canonical SDK
code.

## 11. Language Parity

The supported SDK language set is:

- Rust;
- C ABI;
- Go;
- Python;
- Node;
- Java; and
- Swift.

Each capability in each language must be in exactly one state:

- `Unsupported`: no public contract is exposed;
- `Seam`: public contract exists but has no production provider;
- `ProviderBacked`: provider path exists but not all conformance vectors pass;
- `CutoverReady`: all vectors, error contracts, lifecycle tests, and product
  cutover checks pass.

No language may be marked `CutoverReady` because its public API shape resembles
another language. The label requires behavioral proof.

The capability matrix must include:

- identity source;
- descriptor resolution;
- authorization artifact;
- signer lookup;
- descriptor-bound proof;
- unary invocation;
- server stream;
- bidi session;
- cancellation;
- timeout;
- retry/idempotency;
- terminal receipt;
- receipt history;
- remote owner offline handling;
- local runtime routing; and
- product edge adapter cutover.

## 12. Product Cutover

EasyNet product code must consume the SDK session model for:

- device ability listing;
- resource listing;
- invocation history;
- browser session open;
- media and voice bidi;
- plugin invocation;
- local runtime invocation;
- remote runtime invocation; and
- receipt display.

EasyRemote product code must consume the same SDK session model for:

- remote device discovery;
- remote ability invocation;
- Hub-mediated invocation;
- stream and bidi lifecycle;
- cancellation;
- timeout and retry;
- receipt history; and
- receipt rendering.

Products may keep product-specific facade methods, but those methods must be
edge adapters that create `InvocationIntent` and call `AuthorizedRuntimeSession`.
They must not own canonical route resolution, authority session construction,
signer fallback, receipt canonicalization, or lifecycle interpretation.

## 13. Plugin And Sidecar Model

Plugins are product extensions, not canonical SDK contributors.

A plugin template may use any language if it can produce a sidecar process or
runtime binding that implements the canonical provider contract. Python is not
special. Languages without first-class helper packages may still work through a
compiled sidecar, but product templates should prefer generated helper APIs from
the SDK when available.

Plugin creation templates must not ask authors to hand-write canonical runtime
JSON as the primary interface. Templates must use the SDK helper package for the
selected language where one exists. If no helper exists, the template must mark
that language as `Seam` or `ProviderBacked` according to the capability matrix
and generate a sidecar adapter that still submits through
`AuthorizedRuntimeSession`.

Plugin conflict prevention must be runtime-owned:

- descriptor names are scoped by package identity and ability reference;
- registration is idempotent;
- conflicting ability ownership produces a deterministic rejection;
- install, upgrade, remove, reinstall, and crash recovery have terminal
  receipts; and
- no plugin can shadow another plugin's descriptor without an explicit
  replacement policy.

## 14. Receipt Rules

A receipt is canonical only when it contains complete proof facts.

Mandatory receipt facts:

- invocation tuple fingerprint;
- descriptor fingerprint;
- caller identity;
- acting principal;
- target;
- subject;
- authority artifact fingerprint;
- signer fingerprint;
- admission decision;
- runtime acceptance facts;
- causal parent, if any;
- terminal state;
- terminal timestamp; and
- error code and reason, if terminal state is failure.

Receipt constructors in any language must reject missing proof facts. Existing
constructors that synthesize empty proof facts must be removed or changed into
test-only negative fixtures.

Products must not canonicalize receipts. Products may verify, filter, display,
and query receipts through the SDK receipt provider.

## 15. Error Contract

The SDK must expose stable product-neutral errors:

- `CALLER_IDENTITY_UNAVAILABLE`;
- `CALLER_SIGNER_UNAVAILABLE`;
- `AUTHORITY_SUBJECT_MISMATCH`;
- `AUTHORITY_DENIED`;
- `DESCRIPTOR_NOT_FOUND`;
- `DESCRIPTOR_OWNER_OFFLINE`;
- `DESCRIPTOR_MODE_UNSUPPORTED`;
- `DESCRIPTOR_STALE`;
- `RUNTIME_ROUTE_UNAVAILABLE`;
- `INVOCATION_CANCELLED`;
- `INVOCATION_TIMEOUT`;
- `TERMINAL_RECEIPT_UNAVAILABLE`;
- `RECEIPT_PROOF_FACTS_MISSING`; and
- `PROVIDER_UNAVAILABLE`.

Each error must include structured context sufficient for product rendering and
debugging without parsing free-form messages.

For the motivating failure, the acceptable behavior is:

- the SDK rejects before dispatch if the authority session subject does not
  admit the envelope subject;
- the error is `AUTHORITY_SUBJECT_MISMATCH`;
- the error context includes caller, acting principal, target, ability, subject,
  authority-session subject, and owner principal;
- no remote route is attempted after mismatch; and
- no terminal success receipt is produced.

## 16. Lifecycle Contract

Unary, stream, and bidi invocation must share one lifecycle vocabulary.

Unary states:

```text
Created -> Prepared -> Authorized -> Signed -> Submitted -> Terminal
```

Server stream states:

```text
Created -> Prepared -> Authorized -> Signed -> Open -> Draining -> Terminal
```

Bidi states:

```text
Created -> Prepared -> Authorized -> Signed -> Opening -> Open -> HalfClosed -> Draining -> Terminal
```

Cancellation is a runtime event, not an out-of-band product mutation. It must
produce a queryable acknowledgement and terminal receipt behavior. Retry must be
idempotency-bound. Timeout must have a declared owner and must converge to one
terminal state.

## 17. Compatibility Policy

Public compatibility is allowed only at the edge.

An edge adapter may preserve an old product method signature if it immediately
constructs a complete `InvocationIntent` and enters `AuthorizedRuntimeSession`.
It must not:

- resolve descriptors independently;
- construct route references independently;
- default caller or subject;
- create signer material;
- bypass authority artifact creation;
- construct receipts;
- reinterpret terminal state; or
- call legacy admission paths.

Compatibility adapters must have removal criteria and zero-new-caller gates.

## 18. Required Deletions

After cutover, the following implementation classes are obsolete and must be
removed:

- product-named canonical SDK packages;
- product-specific lifecycle models inside canonical SDKs;
- product-specific directory models inside canonical SDKs;
- product-specific receipt models inside canonical SDKs;
- direct route assembly in product runtime handlers;
- plain proof/admission public APIs;
- receipt constructors that synthesize proof facts;
- signer fallback creation;
- target-as-subject defaulting at public ingress;
- all-zero user fallback identity;
- product receipt canonicalizers; and
- Hub-owned canonical lifecycle interpretation.

## 19. Conformance Gates

The V3 gate must check:

- no retired address vocabulary appears in active normative source;
- SDK public API inventory is product-neutral;
- provider interfaces are present across all supported languages;
- capability matrix is complete and synchronized;
- source attestation hashes match provider implementations;
- all high-level APIs lower into the canonical state machine;
- no product layer calls descriptor resolution directly except through an edge
  adapter that immediately enters the SDK session;
- no product layer constructs canonical receipts;
- no SDK path creates fallback signer material;
- no SDK path substitutes all-zero identity;
- authority-subject mismatch is rejected before remote dispatch;
- descriptor owner offline is distinct from descriptor not found;
- cancellation, timeout, stream, and bidi terminality are deterministic;
- plugin install, upgrade, remove, reinstall, and crash recovery are receipt
  backed; and
- product mutation tests prove one product operation submits once and produces
  one Axon-finalized signed receipt chain.

## 20. Required Tests

SDK conformance tests:

- authority subject mismatch negative vector;
- missing caller identity negative vector;
- missing caller signer negative vector;
- owner offline descriptor vector;
- descriptor not found vector;
- stream cancellation vector;
- bidi timeout vector;
- retry idempotency vector;
- receipt proof facts mandatory vector; and
- provider source attestation vector.

Product tests:

- device ability listing uses SDK session;
- resource listing uses SDK session;
- invocation history uses SDK session;
- browser open session uses SDK session;
- media bidi uses SDK session;
- remote bidi uses SDK session;
- disconnect during stream reaches terminal state;
- cancel during bidi reaches terminal state;
- timeout during remote route reaches terminal state;
- plugin install/upgrade/remove/reinstall has one receipt chain; and
- crash recovery does not duplicate product operation submission.

Mutation tests:

- remove caller identity and verify pre-dispatch rejection;
- replace caller with all-zero identity and verify pre-dispatch rejection;
- replace subject with target and verify mismatch where policy does not admit
  it;
- remove descriptor proof facts and verify receipt rejection;
- make descriptor owner offline and verify `DESCRIPTOR_OWNER_OFFLINE`;
- duplicate idempotency key and verify replay-safe outcome; and
- kill sidecar mid-invocation and verify terminal receipt convergence.

## 21. Delivery Order

Implementation must proceed in this order:

1. Add canonical domain types and state machine in Rust core SDK.
2. Add provider interfaces and conformance vectors.
3. Add C ABI surface for the same model.
4. Implement Go, Python, Node, Java, and Swift bindings against the same
   vectors.
5. Cut EasyNet product ingress to `AuthorizedRuntimeSession`.
6. Cut EasyRemote product ingress to `AuthorizedRuntimeSession`.
7. Move plugin templates to SDK helper packages or sidecar adapters.
8. Remove obsolete product-specific SDK surfaces and legacy admission paths.
9. Add product mutation tests and Docker e2e gates.
10. Update source attestation and public API inventory.

Do not mark a layer complete until callers are migrated and obsolete code is
deleted.

## 22. Acceptance Definition

This SPEC is complete only when:

- all supported languages publish the same capability matrix;
- every `CutoverReady` capability passes the shared vectors;
- EasyNet no longer assembles authority sessions outside the SDK model;
- EasyRemote no longer interprets canonical receipt lifecycle;
- product history, device resources, browser session, media, remote bidi, and
  plugin paths submit through `AuthorizedRuntimeSession`;
- descriptor resolution, authorization, signing, submission, receipt, stream,
  and bidi lifecycle are owned by providers;
- all-zero identity fallback is impossible in production code;
- authority-subject mismatch is caught before remote dispatch;
- Java receipt canonicalization cannot bypass proof-fact validation;
- product mutation tests prove single submission and single signed terminal
  receipt chain;
- canonical-runtime-convergence V2 gates remain green; and
- V3 SDK/product gates are green.

## 23. Current Failure Acceptance Scenario

Given an authenticated user principal and a device target, when the product asks
for invocation history:

1. The product creates an `InvocationIntent` with explicit caller, acting
   principal, target device, `invocation.history.list`, subject, call mode,
   deadline, idempotency key, and causal context.
2. The SDK resolves the descriptor.
3. The SDK materializes an authority session for the same subject.
4. The SDK obtains signer material for the caller identity.
5. The SDK submits the signed descriptor-bound invocation.
6. The runtime returns a terminal receipt or a stable typed failure.

The invalid behavior observed before V3 is rejected:

- session subject owned by all-zero user;
- envelope subject set to device without an admitting authority artifact;
- remote route attempted after authority mismatch;
- descriptor resolution attempted without caller signer; and
- product UI interpreting descriptor or authority errors as canonical receipt
  state.

## 24. Architecture Bar

The implementation must converge on a single shared runtime model. Adding
patches to make current product flows pass while keeping separate authority,
descriptor, signer, receipt, or lifecycle paths is non-conformant.

The correct fix for the observed failure is not to special-case invocation
history. The correct fix is to make invalid session construction impossible in
the SDK and to cut product ingress over to that SDK model.
