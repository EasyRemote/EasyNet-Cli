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
- runtime identity, public-key projection and sign-only capabilities;
- managed-signing key lifecycle through the daemon key-service;
- product-neutral principal enrollment, key binding and authorization grants;
- product-neutral Directory resolution and subscription;
- AbilityDescriptor and authority metadata projections;
- complete Invocation construction, prepare, sign, submit and result state;
- unary, server-stream and bidi transport;
- typed errors, health and diagnostics;
- receipt facts, causal continuation and runtime event cursors;
- product-neutral runtime administration.

### 2.4 Products

Products own all behavior whose meaning exists only for a concrete use case,
including hosted-agent workflows, product directory/read-model projections,
publication UX, Mission/EAL ergonomics, pages, OpenAI-compatible routes,
host-stream codecs, file transfer, terminal/browser/media sessions, product
event projections, account databases and HTTP pairing UX. These layers invoke
generic runtime capabilities and define their own DTOs in their own
repositories. A product-specific DTO does not move its underlying identity,
Directory, receipt, event or principal lifecycle out of the runtime SDK.

## 3. Required object model

The normative object graph is:

```text
SdkEnvironment
  -> NativeRuntimeHandle
       -> RuntimeClient
       -> HealthClient
       -> Addressing
       -> RuntimeIdentityClient
       -> ManagedSigningClient
       -> PrincipalClient
       -> DirectoryClient
       -> ReceiptClient
       -> RuntimeEventClient
       -> RuntimeAdminClient
  -> DaemonControl
       -> DaemonHandle
            -> RuntimeClient
            -> HealthClient
            -> Addressing
            -> RuntimeIdentityClient
            -> ManagedSigningClient
            -> PrincipalClient
            -> DirectoryClient
            -> ReceiptClient
            -> RuntimeEventClient
            -> RuntimeAdminClient

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
graph. Each canonical capability has one explicit provider-backed client; a
bundle cannot hide provider selection or merge unrelated product lifecycles.

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

REQ-MATRIX-4: Product workflows are not SDK capabilities. Identity, managed
signing, principal lifecycle, Directory resolution, receipt/causal facts,
runtime events and runtime administration are SDK capabilities when their
semantics are shared across independent consumers. Product-local DTOs and
workflows may cite these generic capabilities as dependencies.

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

REQ-PROD-1: EasyNet backend owns PostgreSQL account state, HTTP/API contracts,
hosted-agent workflows, product Directory views, product event projections,
Surface, Compatibility, Publication and wrapper DTOs. It consumes the generic
Go SDK identity, principal, Directory, receipt, event, access-control and
runtime-administration clients. It does not own their canonical lifecycle or
wire lowering.

REQ-PROD-2: EasyRemote owns Mission plans/status, hosted-agent workflow,
publication catalog, gateway UX, host-stream codec and local receipt
presentation. It consumes the generic Python SDK configuration, identity,
Directory, receipt and event clients rather than copying their runtime model.

REQ-PROD-3: Product repositories do not copy Axon URA grammar, canonical bytes,
Invocation transport or admission/receipt verification.

REQ-PROD-4: After migration, the runtime SDK deletes only product-specific
modules, exports, schemas, cases and documentation whose canonical capability
has already been extracted and whose consumers have migrated. A module cannot
be deleted merely because it historically used the word `profile` or contains
both generic and product DTOs; generic runtime contracts are extracted first.

REQ-PROD-5: Public Go/Python interfaces remain source-compatible unless this
specification explicitly declares a breaking major-version cutover. A retained
public facade delegates to the single canonical provider and cannot preserve a
second parser, transport, signer, state machine or persistence model.

## 14. Standalone Hub principal lifecycle

EasyNet-Cli must be able to run a complete runtime Hub without the EasyNet
backend. The backend is an optional PostgreSQL, HTTP and account-experience
layer; it is not the owner of Hub process lifecycle, runtime identity, private
keys, principal admission or receipt truth.

### 14.1 Current implementation baseline

The implementation at the time of this specification update has a complete
multi-key custody substrate but not a complete backend-free multi-user
lifecycle.

| Capability | Current state |
| --- | --- |
| One Hub manages multiple runtime owners | implemented |
| One User URA binds multiple public keys | implemented |
| Key create, list, public projection, rotate, revoke and expiry | implemented |
| Private keys are held only by the daemon key-service | implemented |
| Multi-user signature verification and admission | implemented |
| Create the first user without Backend | provider enforces explicit bootstrap and bind-first-key proof continuity; CLI/E2E workflow incomplete |
| Login, authentication and recovery without Backend | recovery policy proof is provider-enforced; login/recovery CLI flow and E2E incomplete |
| A user adds a second device/key without Backend | daemon provider supports key add/rotate/revoke state with active-key, grant and recovery proof enforcement; CLI flow and E2E incomplete |
| Multi-user administration and permission governance without Backend | partial capabilities; no standalone Hub closure |

The current substrate facts are:

- `RealmTrustAnchor` stores one User URA with multiple public-key bindings, so
  one user can use distinct non-exportable keys on a laptop, phone or other
  device;
- `identity.register_pubkey`, `identity.list_user_pubkeys` and
  `identity.revoke_user_pubkey` already mutate/read the daemon trust aggregate;
- the daemon key-service supports managed-key create, list, public projection,
  sign, rotate, revoke, expiry and immutable subject binding;
- every key can bind to a User URA while private material remains outside the
  backend, SDK consumers and EasyRemote;
- cross-realm user-binding tokens and replay protection exist; and
- `principal.lifecycle.*` now has an initial daemon-owned durable provider
  that records principal state, key bindings, recovery policy and grants while
  projecting active/revoked public-key facts through the existing
  `RuntimeTrust` aggregate; and
- provider-side PrincipalLifecycle proof enforcement now validates
  active-key references against active key bindings, grant references against
  durable authorization grants, recovery references against the configured
  recovery policy, and bind-first-key continuity against the create-time
  bootstrap/enrollment proof; and
- durable PrincipalLifecycle enrollment authority now issues, revokes and
  consumes one-time `EnrollmentCapability` records inside the same aggregate.
  Additional principal creation no longer accepts a bare `proof.kind =
  enrollment`; it must reference an active, unexpired, unrevoked and
  unconsumed capability scoped to the target Principal URA.

This substrate is not a user lifecycle. The current
`easynet auth signing-key register` flow derives a User URA from credentials
containing `user_id`/`username`, while pure URA `federation.join` establishes
Device membership and does not naturally create or bind a user. Local
loopback administration can manually register multiple users and keys, and
`principal.lifecycle.*` can now commit initial lifecycle facts. An ordinary
user still cannot yet complete the full invitation/enrollment,
additional-device binding, authentication or recovery product flow without the
remaining CLI proof and E2E work.

### 14.2 Canonical state machine

The daemon and SDK must expose one product-neutral `PrincipalLifecycle`:

```text
CreateUser
  -> BindFirstKey
  -> Active
  -> AddKey
  -> RotateKey | RevokeKey
  -> Recover
  -> Suspend
  -> Active | Delete
```

Each transition is an admitted Invocation, is authorized by an explicit
enrollment/recovery/grant proof, commits durable principal state atomically and
emits a verifiable receipt. A failed transition leaves the prior state and key
bindings unchanged.

The aggregate is:

```text
Principal URA
  -> enrollment authority
  -> public-key bindings
  -> key rotation/revocation state
  -> recovery policy
  -> authorization grants
```

REQ-PRINCIPAL-1: A Hub-mode `easynet-daemon` supports this lifecycle without a
backend process, backend database or backend-issued runtime key.

REQ-PRINCIPAL-2: The User URA is stable across key additions, rotations,
revocations, device changes and recovery. Identity is not a key.

REQ-PRINCIPAL-3: Active user-key uniqueness is `(user_ura, public_key)`.
Revocation is durable and terminal for that binding; another active key for
the same User URA remains valid.

REQ-PRINCIPAL-4: `PrincipalLifecycle` stores public bindings, lifecycle facts,
proof references and grants. Private keys, seeds, vault material and master
keys remain exclusively inside the daemon key-service.

REQ-PRINCIPAL-5: First-user bootstrap is an explicit local-administrator or
one-time enrollment-capability transition. Adding a key requires an active-key
signature, an authorized administrator grant or a recovery proof. A bare
public key or unauthenticated HTTP session is insufficient.

REQ-PRINCIPAL-6: Recovery is a distinct state machine with replay-protected,
single-use proof material and an explicit policy. Recovery cannot silently
replace all active keys or bypass revocation history.

REQ-PRINCIPAL-7: Pure URA `federation.join` creates Device membership. It binds
a user only when the request carries an admitted principal-enrollment proof;
it never invents a User URA or trusts product account fields implicitly.

REQ-PRINCIPAL-8: Go and Python expose the same typed principal operations and
state projections. Product code does not construct the transition Invocation,
proof layout or key-binding mutation by hand.

REQ-PRINCIPAL-9: EasyNet backend maps PostgreSQL/OAuth/Passkey account results
onto this same Principal URA and lifecycle through the Go SDK. It cannot create
a second trust store, private-key inventory, admission path or recovery truth.

REQ-PRINCIPAL-10: Without Backend, CLI local administration, invitation
capabilities and signed enrollment/recovery proofs operate the same lifecycle.
Backend-present and backend-free deployments produce equivalent principal,
key-binding, admission and receipt facts.

### 14.3 Standalone Hub acceptance

Completion requires one backend-free end-to-end test that:

1. starts one Hub-mode daemon and its single key-service;
2. bootstraps the first administrator through an explicit one-time authority;
3. enrolls two distinct User URAs;
4. binds at least two keys to each user;
5. admits signed Invocations from every active key;
6. revokes one key and proves the sibling key remains admitted while the
   revoked key is rejected;
7. exercises rotation, recovery, suspend/reactivate and delete terminality;
8. restarts the Hub and proves principal state, active bindings, revocation
   history, grants and receipt continuity persist;
9. joins a Device through a Hub URA without an HTTP pairing dependency; and
10. proves no private-key material enters the SDK consumer, EasyRemote or an
    optional backend process.

A second end-to-end test starts the optional EasyNet backend and proves its
account flow maps into the same daemon PrincipalLifecycle without spawning a
second daemon/key-service or writing a parallel trust source.

### 14.4 Delivery status at this specification update

This target is not currently cutover-ready. On 2026-07-11 the restored baseline
was re-audited after the interrupted work: Go SDK tests, Python SDK tests,
EasyNet backend tests and EasyRemote tests all passed. There is therefore no
current Go compilation conflict to repair. The remaining defect is
architectural and functional: PrincipalLifecycle has a provider-backed
Go/Python SDK facade and a daemon durable provider. Active-key, grant,
recovery, admission-state and enrollment-capability proof enforcement have
landed in the daemon provider. The product-neutral `easynet principal`
operator facade now covers the provider-backed lifecycle transition surface
through daemon `principal.lifecycle.*` abilities, including create,
bind-first-key, add-key, rotate-key, revoke-key, configure-recovery, recover,
suspend, reactivate, delete, issue/revoke enrollment, issue/revoke grant and
get. It is still not standalone-Hub cutover-ready until the no-Backend URA
join/user-lifecycle flow and the two E2E gates pass.
Directory
is still a seam; receipt/history now has a symmetric bounded seam but no
stable cursor or downstream cutover; runtime events and runtime administration
now have symmetric provider-backed Go/Python facades; access control now has
symmetric provider-backed Go/Python SDK facades over daemon
`authority.binding.*` abilities, while Backend product role mapping and
standalone-Hub governance cutover remain incomplete; and the backend still
owns product-local runtime-profile lowering.
Backend-free multi-user closure remains partial as described above. Passing
baseline tests must not be reported as standalone-Hub delivery evidence until
sections 14.2 and 14.3 and the cross-language parity gates pass.

## 15. Required conformance evidence

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
10. zero compiler warnings in the production Rust library;
11. backend-free PrincipalLifecycle acceptance from section 14.3; and
12. backend-present mapping to the same principal/key/admission truth.

## 16. Architecture prohibitions

The following fail review and CI:

- parallel AbilityDescriptor/manifest domain aggregates;
- more than one daemon transport `CallMode` definition;
- a runtime adapter that manufactures caller identity;
- product-specific C ABI exports;
- product profile bundles or service locators in the runtime SDK;
- deleting a generic runtime capability before its canonical provider and
  migrated consumers are proven;
- parallel principal, user-key, recovery or trust stores in Backend/products;
- local URA/descriptor grammar in a product or binding;
- legacy identity-field spelling instead of URA;
- load-error-to-empty/default behavior;
- boot-window no-op success or restart-as-repair;
- dead compatibility modules, aliases, old schemas or historical source-of-truth
  documents.

## 17. Migration plan and remaining work

Migration follows dependency direction. Destructive deletion is last.

| Order | Repository | Required action | Completion effect |
| --- | --- | --- | --- |
| 1 | EasyNet-Cli | Isolate the interrupted SDK restoration delta and return Go SDK to a compiling, attributable baseline without overwriting unrelated worktree changes | recovery work is reviewable and reversible |
| 2 | EasyNet-Cli | Inventory published Go/Python modules, exports and current consumers; classify every capability as generic runtime or product workflow | no module is deleted by name/category guess |
| 3 | EasyNet-Cli | Converge Addressing, identity/signing, managed signing, PrincipalLifecycle, Directory, receipt/causal, events, administration and access-control on one explicit provider per capability | necessary canonical SDK model is restored without parallel implementations |
| 4 | EasyNet-Cli | Add identical Go/Python capability rows and executable evidence | language parity reflects architecture rather than documentation |
| 5 | EasyNet backend | Replace duplicated runtimeprofile wire lowering with generic Go SDK clients while retaining PostgreSQL/HTTP/product DTO ownership | backend becomes a consumer, not a runtime contributor |
| 6 | EasyNet backend | Start/attach Hub mode only through daemon Go SDK lifecycle and map account authentication to PrincipalLifecycle | no backend-owned daemon/key-service/authentication root |
| 7 | EasyRemote | Consume Python SDK environment, identity, Directory, receipt and events; retain Remote product workflows locally | no copied daemon discovery, FFI, Axon wire or canonical model |
| 8 | EasyNet-Cli | Complete standalone PrincipalLifecycle and URA-only join acceptance | Hub operates correctly without Backend |
| 9 | all three | Delete only proven-obsolete product modules, duplicate DTO/wire code and invalid gates after consumers migrate | architecture converges without capability loss |
| 10 | all three | Run public-API compatibility, Go/Python parity, Rust, Backend, EasyRemote and cross-repository E2E suites; commit by feature | cutover-ready evidence is complete |

The interrupted restoration conflict described in section 14.4 has been
resolved. Public API inventory, the symmetric capability matrix, generic
PrincipalLifecycle seams, canonical Invocation lowering, the first Directory
provider migration, the bounded Receipt/causal/history/trace seam, and
provider-backed runtime Events/Admin and AccessControl facades have landed.
They are intermediate convergence evidence, not completion of downstream
product cutover or the standalone-Hub PrincipalLifecycle closure.

The current remaining work is:

- add a stable Receipt history cursor/anchor provider and cut over every
  downstream Receipt consumer before promoting the bounded seam;
- migrate Backend access-control role/account mapping onto the generic Go SDK
  AccessControl facade and delete remaining product-local runtime lowering;
- finish migrating Backend off duplicated `internal/runtimeprofile` lowering,
  including receipt, event, administration and principal lifecycle paths;
- migrate EasyRemote to canonical typed Python SDK configuration, identity,
  Directory, receipt and event capabilities;
- prove the completed `easynet principal` lifecycle facade in a backend-free
  Hub flow with first-user bootstrap, additional-key authorization, key
  rotation/revocation, recovery, suspension/reactivation/deletion and grants;
- prove URA-only `federation.join` plus Principal enrollment without Backend
  HTTP;
- delete obsolete product modules, duplicate wire/DTO code and legacy gates
  only after their consumers have migrated;
- run the full Rust default/`axon-pb`, Go, Python, Backend and EasyRemote
  regression suites; and
- run the two standalone/backend-present E2E scenarios from section 14.3.

## 18. Source of truth

`docs/ARCHITECTURE_STATE.md` is the current architecture index. This file is
the normative SDK contract. `sdk/SDK_INTERFACE_SPEC.md` is the concise public
object-graph contract. The machine capability state is
`sdk/conformance/sdk-parity-matrix.json`. No other document may claim a
different current SDK object graph.
