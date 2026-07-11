# Canonical Runtime SDK Requirements

Status: current normative specification
ABI line: v5
Applies to: Rust implementation, C ABI, Go, Python, Node, Java and Swift

This file keeps its historical path so existing tooling has one stable
reference. Its contents replace the former product-profile SDK design.

## 1. Purpose

The SDK defines one canonical runtime model for governed ability invocation.
It is not an EasyNet product SDK and it is not an EasyRemote product SDK.
Products consume the runtime model and own their concrete workflows.

The design goals are:

- one complete Invocation representation across every transport;
- one Axon-owned addressing and canonicalization authority;
- one product-neutral object graph in every language;
- explicit provider and lifecycle state;
- generic C ABI stability without domain symbol growth;
- observable, fail-closed behavior when required state is unavailable.

## 2. Layer ownership

### 2.1 Axon

Axon owns all facts that independent implementations must agree on:

- URA grammar, typed builders and parsers;
- AbilityDescriptorRef grammar;
- canonical Invocation bytes and signature material;
- caller, callee, subject, nonce and causal-context semantics;
- transport call modes;
- admission and receipt cryptography.

The runtime SDK delegates these operations to Axon. It does not maintain a
second grammar or signer.

### 2.2 Daemon

The daemon owns process lifecycle, provider wiring, catalog assembly,
AbilityDescriptor/AuthorityBinding/AbilityImplBinding state, governed dispatch,
routing, persistence and local execution. A language binding may project these
facts but may not redefine them.

### 2.3 Runtime SDK

The SDK owns language-safe projections and lifecycle around:

- environment and daemon/runtime handles;
- Addressing;
- AbilityDescriptor and authority metadata projections;
- complete Invocation construction, prepare, sign, submit and result state;
- unary, server-stream and bidi transport;
- typed errors, health and diagnostics;
- opaque receipt facts required for causal continuation.

### 2.4 Products

Products own all behavior whose meaning exists only for a concrete use case,
including hosted-agent administration, directory/read models, publication,
Mission/EAL ergonomics, pages, OpenAI-compatible routes, host-stream codecs,
file transfer, terminal/browser/media sessions, product event streams, account
and pairing state. These layers invoke generic runtime abilities and define
their own DTOs in their own repositories.

## 3. Required object model

The normative object graph is:

```text
SdkEnvironment
  -> NativeRuntimeHandle
       -> RuntimeClient
       -> HealthClient
       -> Addressing
  -> DaemonControl
       -> DaemonHandle
            -> RuntimeClient
            -> HealthClient
            -> Addressing

RuntimeClient
  -> InvocationBuilder
       -> InvocationDraft
            -> PreparedInvocation
                 -> SignedInvocation
                      -> InvocationHandle
                           -> InvocationResult
  -> StreamHandle
  -> BidiSession
```

REQ-OBJ-1: A public factory either returns a fully valid object or a typed
error. It must not construct an object whose required provider is `nil`/`None`.

REQ-OBJ-2: Provider selection happens at construction. An operation must not
search a registry, load a different library or fall back to another transport.

REQ-OBJ-3: Closing an owner closes only resources it explicitly owns. Borrowed
providers remain usable according to their own lifecycle.

REQ-OBJ-4: No product profile bundle or service locator is part of the public
graph.

## 4. Complete Invocation

One Invocation contains:

```text
(caller_ura,
 callee_ura,
 descriptor_ref,
 subject_ura,
 nonce_base64,
 causal_context,
 args | arguments_base64)
```

REQ-INV-1: All seven semantic slots are required before prepare or invoke.

REQ-INV-2: Exactly one argument representation is present.

REQ-INV-3: `descriptor_ref` is built/projected by Addressing and carries the
descriptor version. Callers do not append `@version` themselves.

REQ-INV-4: Runtime dispatch, local dispatch, stream/bidi open and federation
relay preserve caller, callee, descriptor reference, subject, nonce, causal
context, arguments and admitted metadata byte-for-byte or by a proven canonical
projection.

REQ-INV-5: No adapter substitutes a system caller, generates a replacement
nonce, drops causal context or reconstructs a signed business Invocation.

REQ-INV-6: `PreparedInvocation` is not executable. Only an Invocation draft or
a valid `SignedInvocation` can enter execution/submit.

REQ-INV-7: Terminal state is monotonic. Success, failure and cancellation are
mutually exclusive.

## 5. Addressing

REQ-ADR-1: The SDK has one public `Addressing` seam. Identity signing-key
lifecycle and product Directory are not Addressing.

REQ-ADR-2: Go and Python delegate URA and descriptor-reference behavior to the
same Axon grammar and cover the same accepted/rejected vectors.

REQ-ADR-3: Builders validate structural segments. Values containing an extra
path segment or an invalid owner form fail closed.

REQ-ADR-4: `Addressing` is always available from an open native runtime handle.

REQ-ADR-5: Product display labels, route paths and catalog projections do not
enter the Addressing interface.

REQ-ADR-6: URA is the only semantic identity term. External HTTP or transport
locator types retain the spelling required by their defining standards; that
spelling must never name an EasyNet/Axon identity field.

## 6. Ability aggregate projection

REQ-ABL-1: `AbilityDescriptor` is the sole governed-interface aggregate.

REQ-ABL-2: `AuthorityBinding` is the sole advertise/invoke authority aggregate.

REQ-ABL-3: `AbilityImplBinding` is the sole execution binding aggregate.

REQ-ABL-4: `AbilityManifest` is a daemon import/persistence DTO. It is
normalized exactly once and is not exposed as a parallel SDK domain model.

REQ-ABL-5: Schema, visibility, call mode, receipt semantics and authority are
projected from the aggregate that owns them. A catalog or SDK helper cannot
infer them from ability-name prefixes.

## 7. Transport modes and transition semantics

REQ-MODE-1: Unary, server-stream and bidi are the only transport modes and map
one-to-one to Axon.

REQ-MODE-2: The daemon may define an idiomatic projection enum only at the
boundary where it is converted to Axon. Routing, plugins and conformance code
reuse that type instead of declaring parallel enums.

REQ-MODE-3: Transition/receipt semantics are descriptor state-machine facts,
not a transport mode and not an alias for unary RPC.

## 8. Lifecycle state machines

Stateful objects expose explicit monotonic state and typed invalid-transition
errors.

```text
builder:   Building -> Frozen
prepared:  Prepared -> Closed
signed:    Signed -> Submitted -> Closed
handle:    Pending -> Terminal | Cancelled -> Closed
stream:    Opening -> Open -> Terminal | Cancelled | Failed -> Closed
bidi:      Opening -> Open -> SendClosed -> Terminal | Cancelled | Failed -> Closed
daemon:    Discovered -> Starting -> ControlReady -> InvocationReady -> Running
           Running -> Stopping -> Stopped
```

REQ-LIFE-1: Runtime/catalog/provider not ready is an error state, never a
successful no-op.

REQ-LIFE-2: A multi-stage operation either commits every required local stage or
rolls back completed stages in reverse order. If an external best-effort stage
fails, its result is explicit and observable.

REQ-LIFE-3: Corrupt durable state is not converted to an empty registry,
default object or restart suggestion.

REQ-LIFE-4: Compatibility fields that only report a hidden failure after a
successful return are prohibited.

## 9. Typed errors

Every language exposes the same error classes and retry semantics. At minimum:

- invalid argument/state/handle;
- version incompatible;
- daemon/runtime unavailable;
- permission/admission/authority denied;
- not found/route unavailable;
- cancelled/timeout;
- protocol/transport;
- ability failed;
- not implemented;
- internal/generic.

Errors carry stable code, stage and retry hint. Consumers branch on those
fields, never message text.

## 10. Capability-state matrix

Each Go/Python capability has exactly one state:

| State | Meaning |
| --- | --- |
| unsupported | no shipped public capability |
| seam | public interface and state model exist, but no shipped provider |
| provider-backed | an explicit provider is implemented and tested |
| cutover-ready | first-class consumers use it and lower-layer/product duplication is deleted |

REQ-MATRIX-1: The capability identifiers and semantics are identical for Go and
Python.

REQ-MATRIX-2: Each non-unsupported state cites executable evidence.

REQ-MATRIX-3: A type or placeholder method alone cannot justify
`provider-backed` or `cutover-ready`.

REQ-MATRIX-4: Product workflows are not SDK capabilities. Their downstream
tests may cite generic runtime capabilities as dependencies.

The machine-readable source is
`sdk/conformance/sdk-parity-matrix.json`.

## 11. C ABI v5

The C ABI is generic and major-versioned.

Allowed symbol families:

- version/feature and typed-error discovery;
- environment, daemon and runtime lifecycle;
- generic Invocation build/prepare/invoke/submit/handle;
- stream and bidi;
- runtime health/diagnostics and required Addressing projection;
- opaque handle and owned-buffer release.

REQ-ABI-1: Domain operations do not receive C symbols.

REQ-ABI-2: Removed v4 domain symbols have no aliases, weak exports, fallback
lookups or permanent dual track.

REQ-ABI-3: Go/Python native providers resolve only the v5 export list.

REQ-ABI-4: The header, export list, loader symbol table, release packaging and
ABI conformance test agree exactly.

## 12. Language bindings

REQ-LANG-1: Languages may use idiomatic names but preserve the object graph,
state transitions and validation semantics.

REQ-LANG-2: Raw generated Axon/protobuf/native handle types remain internal.

REQ-LANG-3: Node, Java and Swift publish only concepts they implement. Missing
concepts are unsupported, not placeholder product clients.

REQ-LANG-4: A language-specific convenience adapter may translate host objects
to `InvocationDraft`; it cannot own routing, addressing grammar or product
ability semantics.

## 13. Product extraction

REQ-PROD-1: EasyNet backend owns its Admin, Directory, Events, Surface,
Compatibility, Publication, AccessControl, signing-key and wrapper DTOs/ports.
They lower through generic Go runtime interfaces.

REQ-PROD-2: EasyRemote owns Mission plans/status, hosted-agent workflow,
publication catalog, gateway lifecycle, host-stream codec and local receipt
presentation. They lower through generic Python runtime interfaces.

REQ-PROD-3: Product repositories do not copy Axon URA grammar, canonical bytes,
Invocation transport or admission/receipt verification.

REQ-PROD-4: After migration, the runtime SDK deletes product modules, exports,
schemas, cases, documentation and compatibility aliases.

## 14. Required conformance evidence

The release gate includes:

1. complete Invocation seven-tuple round trip through local runtime, stream,
   bidi and cross-hub relay;
2. caller, nonce, causal context and descriptor version preservation;
3. descriptor schema/authority/call-mode single-source projection;
4. Axon Addressing accepted/rejected vectors in Go and Python;
5. explicit lifecycle transition and rollback tests;
6. exact ABI v5 symbol/header/package checks;
7. public-export/import gates rejecting product SDK modules and parallel URA,
   Invocation or call-mode models;
8. downstream backend and EasyRemote tests proving product-local ownership;
9. project-structure and dead-code gates;
10. zero compiler warnings in the production Rust library.

## 15. Architecture prohibitions

The following fail review and CI:

- parallel AbilityDescriptor/manifest domain aggregates;
- more than one daemon transport `CallMode` definition;
- a runtime adapter that manufactures caller identity;
- product-specific C ABI exports;
- product profile clients, bundles or service locators in the runtime SDK;
- local URA/descriptor grammar in a product or binding;
- legacy identity-field spelling instead of URA;
- load-error-to-empty/default behavior;
- boot-window no-op success or restart-as-repair;
- dead compatibility modules, aliases, old schemas or historical source-of-truth
  documents.

## 16. Source of truth

`docs/ARCHITECTURE_STATE.md` is the current architecture index. This file is
the normative SDK contract. `sdk/SDK_INTERFACE_SPEC.md` is the concise public
object-graph contract. The machine capability state is
`sdk/conformance/sdk-parity-matrix.json`. No other document may claim a
different current SDK object graph.
