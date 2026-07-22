# Canonical Runtime SDK Requirements

Status: current normative specification
ABI line: v5
Applies to: Rust implementation, C ABI, Go, Python, Node, Java and Swift

This file keeps its historical path so existing tooling has one stable
reference. Its contents replace the former product-profile SDK design.

## 1. Purpose

The SDK distribution defines one canonical runtime model for governed ability
invocation and ships an EasyNet provider ABI that binds that model to
`easynet-daemon`. The canonical model is provider-neutral; the provider ABI is
not. Neither layer owns EasyNet or EasyRemote product workflows. Products
consume the runtime model through an explicit provider and own their concrete
workflows.

The design goals are:

- one complete Invocation representation across every transport;
- one Axon-owned addressing and canonicalization authority;
- one canonical product-neutral object graph in every language;
- explicit provider and lifecycle state;
- bounded EasyNet provider ABI stability without domain symbol growth;
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

The reproducible cross-repository dependency contract is Axon commit
`896b35c1c403f23754604822a992d2fbbd14520c`. At that revision the Rust SDK is
`0.129.22` (the version locked by this repository's `Cargo.lock`) and the
Python SDK is `0.129.23` (the version required by `sdk/python/pyproject.toml`
and `sdk/python/uv.lock`). CI must check out that exact revision and fail if
its HEAD or either SDK version differs. A branch name such as `main` is not a
dependency version.

### 2.2 Daemon

The daemon owns process lifecycle, provider wiring, catalog assembly,
AbilityDescriptor/AuthorityBinding/AbilityImplBinding state, governed dispatch,
routing, persistence and local execution. A language binding may project these
facts but may not redefine them.

### 2.3 Canonical runtime SDK

The SDK owns language-safe projections and lifecycle around:

- environment and provider-independent runtime-host/runtime handles;
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

The canonical inventory is the `languages`, `members` and
`capability_inventory` surface in
`sdk/conformance/canonical-public-api.json`. A symbol exported by a language
package is not canonical merely because it is public.

### 2.4 EasyNet provider ABI

The EasyNet provider binds the canonical model to `easynet-daemon`. It owns:

- daemon discovery, start, attach, status, stop and endpoint projection;
- loading and calling the `easynet_*` C ABI v6 symbol set;
- EasyNet runtime-event and Directory route catalogs;
- adaptation from provider responses into canonical runtime objects.

Provider-specific routes and daemon terms must stay in explicit provider
sources or in source-compatibility facades classified by
`canonical-public-api.json#non_canonical`. They cannot enter provider-neutral
cores. The neutrality closure is recursive: every Go directory named
`*core` or `runtimeevents` under `sdk/go` (currently `directorycore`,
`internal/runtimeevents` and `runtimeevents`) and every Python source under
`sdk/python/easynet_sdk/core` are canonical runtime roots. The EasyNet
provider ABI is the separate lowering layer under `sdk/go/provider/easynet`
and `sdk/python/easynet_sdk/providers/easynet`, plus the generic C ABI v6. It
may contain daemon route names but may only return canonical runtime types.

### 2.5 Products

Products own all behavior whose meaning exists only for a concrete use case,
including hosted-agent workflows, product directory/read-model projections,
publication UX, Mission/EAL ergonomics, pages, OpenAI-compatible routes,
host-stream codecs, file transfer, terminal/browser/media sessions, product
event projections, account databases and HTTP pairing UX. These layers invoke
generic runtime capabilities and define their own DTOs in their own
repositories. A product-specific DTO does not move its underlying identity,
Directory, receipt, event or principal lifecycle out of the runtime SDK.

## 3. Required object model

The normative canonical object graph is:

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
  -> RuntimeHost
       -> RuntimeHandle
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
graph. Each canonical capability has one explicit provider interface; a bundle
cannot hide provider selection or merge unrelated product lifecycles. An
interface or facade remains a seam until live step-complete provider proof is
attached to the concept schema.

REQ-OBJ-5: `DaemonControl`, `DaemonHandle` and related `Daemon*` names are
EasyNet provider/source-compatibility exports, not a second canonical object
graph. Each must delegate to the corresponding `RuntimeHost`/`RuntimeHandle`
implementation and is classified under `non_canonical`.

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

The canonical capability set is language-independent. Every capability has
exactly one state for each of Rust, C ABI, Go, Python, Node, Java and Swift:

| State | Meaning |
| --- | --- |
| unsupported | no shipped public capability |
| seam | public interface and state model exist, but no shipped provider |
| provider-backed | an explicit provider is implemented and tested |
| cutover-ready | first-class consumers use it and lower-layer/product duplication is deleted |

REQ-MATRIX-1: `canonical-public-api.json` is the single concept schema. Its
complete Go/Python public export graph assigns every canonical symbol and
member to exactly one runtime capability or to a justified non-capability
schema category. A capability without an owned public export is invented and
invalid. The checked-in matrix is generated as the complete seven-language
Cartesian product of that derived capability universe. Missing, duplicate or
undeclared cells are invalid; unsupported is explicit, not skipped.

REQ-MATRIX-2: Each non-unsupported state cites runner-owned case selectors that
were collected and executed in the current gate invocation. Committed coverage
reports are not state evidence.

REQ-MATRIX-3: A type or placeholder method alone cannot justify
`provider-backed` or `cutover-ready`. Provider-backed evidence names the
production provider owner and implementation path, hashes that implementation,
and maps every normative case step to a runner-owned selector attested by the
same live invocation. Without that closed proof the state is seam or
unsupported.

REQ-MATRIX-4: Product workflows are not SDK capabilities. Identity, managed
signing, principal lifecycle, Directory resolution, receipt/causal facts,
runtime events and runtime administration are SDK capabilities when their
semantics are shared across independent consumers. Product-local DTOs and
workflows may cite these generic capabilities as dependencies.

The machine-readable source is `sdk/conformance/canonical-public-api.json`;
the generated, validated Cartesian product is
`sdk/conformance/sdk-parity-matrix.json`. Conformance and repository quality
gates are modeled separately and are not runtime capabilities.

At this update the closed public graph derives 31 runtime capabilities and a
31 x 7 = 217-cell matrix with no missing or duplicate cell. No cell is labeled
provider-backed or cutover-ready: the shipped facades remain seams until their
production provider source and every normative case step are jointly attested.
The omitted-frame0 bidi requirement is recorded as unproven for all seven
languages; compile-time arity, fake transports, a positive frame0 test, or a
Java `NullPointerException` cannot satisfy it.

`sdk/conformance/toolchains.json` pins every CI language/build toolchain and
Python gate tool. CI installs those exact versions and verifies them before
running conformance. The Axon sibling checkout additionally verifies the exact
commit and Rust/Python package versions described in section 2.1.

## 11. Runtime C ABI v6

The `runtime_*` C ABI is the major-versioned native runtime ABI packaged by `libeasynet_cli`. It is the lowest shared SDK seam for native providers. Invocation, stream and lifecycle operations are expressed through stable runtime operation families rather than one C symbol per product/domain ability.

Allowed symbol families:

- version/feature and typed-error discovery;
- environment, runtime host and runtime lifecycle;
- generic Invocation build/prepare/invoke/submit/handle;
- stream and bidi;
- runtime health/diagnostics and required Addressing projection;
- opaque handle and owned-buffer release.

REQ-ABI-1: Domain operations do not receive C symbols. Runtime host lifecycle symbols are allowed because this ABI owns generic runtime host control.

REQ-ABI-2: Removed v4 domain symbols have no aliases, weak exports, fallback
lookups or permanent dual track.

REQ-ABI-3: Go/Python native providers resolve only the v6 export list.

REQ-ABI-4: The header, export list, loader symbol table, release packaging and
ABI conformance test agree exactly.

REQ-ABI-5: Provider child resources created by the C ABI are bound to one live
client-session incarnation, not merely to the numeric `RuntimeHandle` value.
The internal lifecycle is `Active -> Closing -> Released`. Submit/open paths
must perform "session is Active + child resource insertion" as one lifecycle
transaction; shutdown must mark the session Closing before draining resources
for that exact binding.

REQ-ABI-6: v6 submitted-invocation handles are one-shot provider resources.
Unknown, stale, cross-session or post-free submitted handles return
`ERR_INVALID_HANDLE` for `await`, `cancel`, `events` and `free`. This is the
v5 public behavior; bindings must not preserve an idempotent-free compatibility
layer because it allows replay-compatible lifecycle authority.

REQ-ABI-7: Submitted-handle cancellation reports the cancel-request lifecycle,
not a fabricated target terminal state. The JSON object must include
`request_accepted`, `deduplicated`, `cancelled`, `state` and `terminal`.
`CancelRequested` is non-terminal unless a verified target terminal receipt is
observed later.

REQ-ABI-8: Unary result JSON exposes the verified terminal fact as
`terminal_receipt`. The retired `receipt` alias is not part of the v5 provider
ABI because it conflates operational result projection with receipt authority.

REQ-ABI-9: Stream and bidi terminal JSON may treat a frame as canonical
Invocation terminal only when a terminal receipt passes the inbound checkpoint
verifier. Transport EOF/status and unverified wire terminal flags are transport
events, not receipt-backed terminal authority.

## 12. Language bindings

REQ-LANG-1: Languages may use idiomatic names but preserve the object graph,
state transitions and validation semantics.

REQ-LANG-2: Raw generated Axon/protobuf/native handle types remain internal.

REQ-LANG-3: Node, Java and Swift publish only concepts they implement. Missing
concepts are unsupported, not placeholder product clients.

REQ-LANG-4: A language-specific convenience adapter may translate host objects
to `InvocationDraft`; it cannot own routing, addressing grammar or product
ability semantics.

REQ-LANG-5: Go/Python `Daemon*` and daemon-named function aliases required by
REQ-PROD-5 remain source-compatible exports until an explicit major-version
cutover. They are non-canonical provider ABI names, must be exhaustively listed
under `canonical-public-api.json#non_canonical`, and must not carry a second
transport, parser, state machine or fallback path.

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
| Create the first user without Backend | provider, CLI bootstrap facade and standalone-Hub TCP+TLS E2E implemented |
| Login, authentication and recovery without Backend | recovery policy proof, replay protection and CLI recovery facade are implemented; broader login/recovery UX packaging remains |
| A user adds a second device/key without Backend | add/rotate/revoke, device enrollment proof binding and live E2E coverage implemented |
| Multi-user administration and permission governance without Backend | provider and live wrong-action grant denial implemented; broader governance UX packaging remains |

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

This substrate now composes into the product-neutral runtime user lifecycle.
Pure URA `federation.join` still establishes Device membership and does not
implicitly create a User; a principal binding is admitted only when the join
carries a valid PrincipalLifecycle proof. The CLI and SDK facades now cover
bootstrap, invitation enrollment, additional keys, rotation, revocation,
recovery, suspension, reactivation, grants, deletion and inspection without
Backend account state. Product login screens and broader governance UX
packaging remain downstream product work, not missing canonical runtime state.

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

This target is accepted only when the current CI invocation produces the
required live conformance, SDK, backend and downstream results. Historical or
committed pass statements are not evidence of current health. PrincipalLifecycle
has Go/Python SDK facades and a daemon implementation, but remains `seam` in
the SDK matrix until production-provider identity and every normative step are
bound to live runner attestations.
Active-key, grant, recovery, admission-state and enrollment-capability proof
enforcement have landed in the daemon provider. The product-neutral
`easynet principal`
operator facade now covers the provider-backed lifecycle transition surface
through daemon `principal.lifecycle.*` abilities, including create,
bind-first-key, add-key, rotate-key, revoke-key, configure-recovery, recover,
suspend, reactivate, delete, issue/revoke enrollment, issue/revoke grant and
get. A provider-level backend-free scenario gate proves multi-user, multi-key
enrollment, rotation, revocation, recovery, lifecycle state changes and
persisted trust/lifecycle reload at the daemon aggregate boundary. A real
daemon gRPC descriptor-ref E2E now drives the same `principal.lifecycle.*`
surface through `DaemonInvocationService`, restarts the daemon, and verifies
persisted PrincipalLifecycle and trust-anchor public-key state without Backend,
HTTP account state or a second auth store. The `federation.join` contract now
has a product-neutral optional PrincipalLifecycle proof seam, and the Hub daemon
validates that proof before atomically binding the joined Device URA to the User
Principal in RuntimeTrust; a real daemon UDS E2E now proves that binding
persists through the same Backend-free PrincipalLifecycle test fixture. A real
two-HOME CLI binary E2E now starts a hub-mode `easynet-daemon` with its
TCP+TLS Invocation listener and joins it by Hub URA with `--hub-ca`, proving
backend-free `federation.join`, empty HTTP credential token, persisted
`join_receipt_hash`, pinned CA persistence, and in-band Hub key import through
`federation.resolve_key`. The same E2E now bootstraps a Hub-side administrator,
issues a product-neutral enrollment capability, joins a Device over the Hub URA
with `--principal-ura` plus `--principal-enrollment-id`, and verifies the Hub
RuntimeTrust owner binding from Device URA to User Principal URA. Broader
recovery/governance UX packaging remains downstream product work and does not
constitute a missing canonical runtime lifecycle. The
Backend-present live daemon-backed account-flow E2E now attaches the Backend
account signing-key product flow to the same `easynet-daemon`
PrincipalLifecycle provider through the Go SDK, proving that account input maps
to one Principal URA, daemon key-service public binding state and runtime
lifecycle projection without a Backend-owned daemon, key-service or trust store.
The downstream SDK consumer cutover and product key-custody gates now cover
Backend/EasyRemote Receipt/Directory/runtime consumer usage and reject product
private-key custody, raw daemon process spawning and raw FFI escape paths. The
same real two-HOME CLI binary E2E now extends the TCP+TLS Hub daemon path to the
multi-user lifecycle
scenario: Alice and Bob are enrolled through product-neutral capabilities,
both receive at least two public-key bindings, Alice exercises add-key,
rotate-key, revoke-key, recovery, suspend and reactivate, Bob is deleted
through an admin grant, the Hub daemon is restarted, and the test verifies
persisted PrincipalLifecycle state, grant state, RuntimeTrust revocation
projection and Device-to-Principal owner binding without Backend HTTP state.
Go and Python PrincipalLifecycle projection decoders now reject forbidden
custody fields recursively, matching the managed-signing public-projection
guard, and the real CLI TLS lifecycle E2E scans PrincipalLifecycle JSON output
for private-key custody fields. The same real CLI TLS lifecycle E2E now also
proves that replayed recovery proofs and deleted-principal recovery attempts are
rejected by the live Hub daemon and do not project replacement keys into
RuntimeTrust. The same live Hub E2E now rejects a wrong-action administrator
grant before deleting another principal and proves the target principal remains
active until a `principal.lifecycle.delete` grant is supplied. Backend-present
mapping to the same live daemon runtime is now covered by the Backend live
PrincipalLifecycle E2E; broader login/recovery flow packaging still remains
outside the section 14.3 canonical runtime acceptance boundary.
Directory now has daemon-backed resolution, stable listing cursors and explicit
subscription resume in symmetric Go/Python providers; receipt/history now has a
daemon-backed stable cursor provider and symmetric Go/Python cursor forwarding;
downstream Backend/EasyRemote Directory and Receipt consumer cutover is now
covered by `tools/scripts/check-downstream-sdk-consumer-cutover.sh`.
Runtime events and runtime administration have symmetric Go/Python facades;
access control has symmetric Go/Python SDK facades over daemon
`authority.binding.*` abilities, while Backend product role mapping and
standalone-Hub governance UX packaging remain downstream product work. These
facades are matrix seams, not provider-backed evidence by themselves.
Runtime Events now also have an explicit cross-repository adapter gate covering
the Go/Python SDK event facades, Backend SDK event subscription/open-stream
adapters and EasyRemote product event consumer behavior. This is adapter
evidence only by itself. `tools/scripts/runtime-events-live-daemon-e2e.sh`
now composes it with Go and Python live daemon smokes that read bounded
`RuntimeEventClient` pages from real `easynet-daemon` handle events over the C
ABI. Runtime Events remain `seam` in the SDK capability matrix until route
implementation and every case step share live selector attestations. Consumer
cutover is established only by its separately executed downstream gate, and
product event taxonomies remain downstream.
AbilityDescriptor projection has symmetric Go/Python facades over daemon
`meta.list_abilities`, but remains a seam until provider proof closes every
normative step. Descriptor schema, call mode,
hashes, visibility and hints remain daemon catalog facts rather than
SDK-inferred product facts.
Receipt summary causal-anchor projection now has symmetric Go/Python SDK
helpers, and EasyRemote child Context dispatch consumes the Python SDK
`ReceiptReference` instead of deciding receipt-anchor validity in product code.
Backend-free multi-user closure is now covered at the canonical runtime
boundary by the section 14.3 composite gate. Passing baseline tests alone are
still not standalone-Hub delivery evidence; the accepted evidence is the
combination of sections 14.2 and 14.3, SDK parity, downstream consumer cutover
and live daemon E2E gates.

## 15. Required conformance evidence

The release gate includes:

1. complete Invocation seven-tuple round trip through local runtime, stream,
   bidi and cross-hub relay;
2. caller, nonce, causal context and descriptor version preservation;
3. descriptor schema/authority/call-mode single-source projection;
4. Axon Addressing accepted/rejected vectors in Go and Python;
5. explicit lifecycle transition and rollback tests;
6. exact ABI v6 symbol/header/package checks;
7. public-export/import gates rejecting product SDK modules and parallel URA,
   Invocation or call-mode models;
8. downstream backend and EasyRemote tests proving product-local ownership;
9. project-structure and dead-code gates;
10. zero compiler warnings in the production Rust library;
11. backend-free PrincipalLifecycle acceptance from section 14.3;
12. backend-present mapping to the same principal/key/admission truth; and
13. SDK action-adapter coverage manifests with no committed status, SHA-256
    pinned evidence sources, and successful runner-owned per-case command
    executions.

The action-adapter JSON files are coverage manifests only. A committed
`status=passed`, selector or command is invalid evidence.
`sdk/conformance/runner/execution-manifest.json` is the runner-owned binding
from `(language, case_id)` to one exact test selector and its evidence source.
The runner hashes the case YAML and evidence, proves that the selector is
declared in the bound source, asks the language test tool to collect it, then
executes that exact collected test. A required case passes only when collection
returns exactly that selector and the emitted result contains non-empty
execution proofs. Each result binds the case SHA-256, evidence SHA-256,
selector, collected test, command, working directory, exit code and
command-output SHA-256, then hashes that complete tuple into
`attestation_sha256`. Empty, uncollected, unrelated or report-supplied
execution evidence is rejected.
The parity gate consumes only the seven JSON result files emitted by that same
runner invocation. It rejects schema-v2 coverage manifests, zero executions,
`skipped`/undeclared states and any non-unsupported matrix cell without a
passing selector-bound attestation.

## 16. Architecture prohibitions

The following fail review and CI:

- parallel AbilityDescriptor/manifest domain aggregates;
- more than one daemon transport `CallMode` definition;
- a runtime adapter that manufactures caller identity;
- product/domain ability-specific C ABI exports;
- product profile bundles or service locators in the runtime SDK;
- deleting a generic runtime capability before its canonical provider and
  migrated consumers are proven;
- parallel principal, user-key, recovery or trust stores in Backend/products;
- local URA/descriptor grammar in a product or binding;
- legacy identity-field spelling instead of URA;
- load-error-to-empty/default behavior;
- boot-window no-op success or restart-as-repair;
- unclassified or logic-bearing compatibility modules/aliases, old schemas or
  historical source-of-truth documents. REQ-LANG-5 aliases are permitted only
  while they remain exact delegates and explicitly classified non-canonical.

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
PrincipalLifecycle seams, canonical Invocation lowering, Directory resolution,
Receipt/causal/history/trace facades, AbilityDescriptor projection, and runtime
Events/Admin and AccessControl seams have landed. None is labeled
provider-backed by the SDK matrix without step-complete live provider proof.
Backend and EasyRemote
Directory/Receipt/runtime consumer cutover is now guarded by
`tools/scripts/check-downstream-sdk-consumer-cutover.sh`, and product key
custody/process/FFI escapes are guarded by
`tools/scripts/check-product-key-custody-boundary.sh`.
The SDK runtime environment projection now covers the local state root,
credentials path and paired runtime identity in both Go and Python, and
EasyRemote `LocalIdentity` consumes that projection instead of directly
parsing daemon credentials. Those projection symbols and `SdkEnvironment`
members are now tracked by the complete canonical public API inventory so
future refactors cannot silently drop the public runtime model surface.
They are accepted runtime convergence evidence. Broader standalone-Hub
recovery/governance UX packaging remains downstream product work.

The Backend-present evidence has now crossed the live runtime boundary: the
Backend has a tested ServiceContext SDK profile graph proving
PrincipalLifecycle, Receipt, Directory, Events, Admin and AccessControl clients
are all derived from one Go SDK native runtime provider and not from parallel
daemon/key-service/trust-store construction. The account signing-key product
flow has an in-process test through the real Go SDK PrincipalLifecycle adapter
proving `get -> create -> bind_first_key` lowering for a Backend account User
URA, a process-level HTTP E2E proving the browser-facing
`POST /api/v1/user/me/signing-keys` route feeds signed invocation admission
through the same projection, and a live daemon-backed Backend-present E2E
proving the same flow against an actual Hub-mode `easynet-daemon` via the Go
SDK C ABI daemon lifecycle and `principalprofile.NewClient`.

The current remaining work is:

- keep the runtime-events live daemon gate in the cutover suite. Runtime events
  now have both cross-repo adapter evidence and Go/Python live daemon
  `RuntimeEventClient` handle-event proof, so the remaining work is regression
  preservation rather than the missing cutover proof;
- keep the section 14.3 PrincipalLifecycle acceptance gate in the cutover
  suite. `tools/scripts/standalone-hub-principal-lifecycle-e2e.sh` now composes
  the backend-free standalone Hub TCP+TLS E2E with the Backend-present live
  daemon PrincipalLifecycle E2E, so the two required deployment-shape E2Es are
  one auditable regression gate;
- keep broader standalone-Hub login/recovery/governance UX packaging as
  downstream product work over the same PrincipalLifecycle model, not as a
  second authentication system;
- delete obsolete product modules, duplicate wire/DTO code and legacy gates
  only after their consumers have migrated;
- run the full Rust default/`axon-pb`, Go, Python, Backend and EasyRemote
  regression suites.

## 18. Source of truth

`docs/ARCHITECTURE_STATE.md` is the current architecture index. This file is
the normative SDK contract. `sdk/SDK_INTERFACE_SPEC.md` is the concise public
object-graph contract. The machine capability state is
`sdk/conformance/sdk-parity-matrix.json`. No other document may claim a
different current SDK object graph.
