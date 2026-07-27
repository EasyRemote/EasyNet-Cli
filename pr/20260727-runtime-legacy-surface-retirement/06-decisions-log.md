# Decisions Log

## Self-target catalogue reads

Decision: treat `--node <local device URA>` as an explicit `LocalRuntime` catalogue read state instead of sending it through canonical remote invocation.

Reason: a local target is not a remote authority problem. Remote invocation requires caller signer and owner authority facts; using it for the local catalogue recreates a second route for the same runtime owner and surfaces product-facing `AUTHORITY_DENIED`/descriptor errors even though the local daemon owns the descriptor catalogue.

Boundary: peer device catalogue reads still use canonical remote invocation. No product-directory fallback or node-id repair was introduced.

## FFI descriptor source naming

Decision: keep local runtime-owner descriptor resolution sourced from `runtime_local_descriptor_catalog`, and update the stale test expectation that still called it `runtime_receipt_provider`.

Reason: descriptor catalogue ownership and receipt history ownership are separate provider states. Calling local catalogue rows receipt-provider rows collapses the owner boundary and weakens SPEC conformance.

## Rejected Device-owned local read issuer

Decision: do not commit the `LocalRuntimeDeviceReadIssuer` prototype.

Reason: the current SPEC v2 gate explicitly treats `discover.rs`, `doctor.rs`, `groups/device.rs`, `status.rs`, `invocation_watch.rs`, the agent state gateway, and `llm-api` model catalogue discovery as runtime-state read paths that must enter through `LocalRuntimeStateReadIssuer`. The prototype would have changed that contract rather than converging to it.

Boundary: this does not bless subject-owner conflation elsewhere. It only records that this repository's current authoritative gate defines those CLI paths as user runtime-state reads. Future changes must first update the SPEC/gate contract, not silently diverge from it.

## Node receipt-history governance subject parity

Decision: align Node SDK receipt-history admission with the Go/Python canonical runtime model by accepting two explicit subject states:

- user-owned runtime-state read subject;
- exact callee runtime-owner subject for Device/Authority governance reads.

Reason: product history views sometimes need a device-owned ledger query. Go and Python already admit this when the subject equals the callee runtime owner and authority is bound to that exact tuple. Node only admitted user runtime-state subjects, forcing product code toward placeholder user sessions or divergent provider behavior.

Boundary: this is not a fallback. Non-callee runtime-owner subjects still fail before provider dispatch, all-zero principals remain rejected, and session-authority subject rules remain strict. Device-owned history requires authority that actually admits the device subject, e.g. exact delegation authority.

## Swift receipt canonicalizer fail-closed parity

Decision: make Swift `RuntimeReceipt.canonicalReceiptType` throw on unknown canonical lifecycle states instead of returning an empty string.

Reason: receipt validation is proof-fact validation, not presentation formatting. Returning an empty string leaves a permissive internal helper that can be reused incorrectly even though the current constructor first canonicalizes `state`. Go, Java, Python, and Node either operate on already validated lifecycle state or fail explicitly; Swift should not preserve a silent empty receipt-type sentinel.

Boundary: public receipt behavior remains compatible for valid receipts. Invalid canonical lifecycle states now fail with an explicit `INVALID_ARGUMENT` validation error before any proof-fact path can treat an empty receipt type as data.

## Java receipt canonicalizer fail-closed parity

Decision: make Java `RuntimeReceipt.canonicalReceiptType` throw on unknown canonical lifecycle states instead of returning an empty string.

Reason: Java had the same internal fail-open sentinel as Swift. Even if current construction validates `state` before binding `receipt_type`, proof-fact validation helpers should not encode unknown lifecycle states as data. Receipt type derivation must be total only over known lifecycle states and explicit-failing otherwise.

Boundary: no new Java public API was introduced. The regression test reaches the private helper by reflection only to lock the internal invariant; valid receipt behavior and public interfaces remain unchanged.

## Device directory user-binding state machine

Decision: model `device list` directory reads as three explicit states:

- bound user credentials: read `federation.discover` through the user-scoped directory path;
- unbound federation-native credentials: fail closed at the CLI boundary because no user-scoped directory principal exists;
- local Authority daemon: read the operator/audit directory path.

Reason: a clean Hub-URA join can intentionally produce a federation-native device credential without a user binding. Treating that as a missing legacy `user_id` sent the product path into an unauthorized operator/audit invocation from a Device daemon and surfaced daemon-internal `AUTHORITY_DENIED`/`LOCAL_BOOTSTRAP_OWNER_UNAVAILABLE`. The runtime state itself is valid; the unsupported capability is the user-scoped product directory read.

Boundary: this does not add a compatibility fallback, does not synthesize a user id, and does not allow a Device daemon to use the Authority operator/audit directory. A user-facing product device directory still requires either a bound User principal or an Authority daemon.

## Hosted-Agent target projection fail-closed cutover

Decision: self-target Agent URA locality must use a validated aggregate
projection. A malformed hosted-Agent identity in `local-agents.json` is not the
same state as "no hosted identity"; silently dropping it lets registry-only
matching continue and recreates a compatibility-style locality path.

Reason: `matches_self_target_ura()` is a route/admission boundary. If the hosted
identity projection is structurally invalid, the daemon cannot prove that an
Agent URA belongs to this runtime owner. The correct state is an unavailable
projection that fails closed, not a partial projection assembled from whichever
rows happened to parse.

Boundary: registered Agent names remain useful only when the aggregate
projection is valid and the credential `(realm,user)` matches the target tuple.
Malformed hosted identity data never causes route repair, alias matching, or a
registry-only self-target decision.

## Hosted-Agent route placement projection fail-closed cutover

Decision: route resolver hosted-Agent placement must use the same validated
hosted identity projection as self-target locality. Malformed hosted-Agent
identity rows must make placement state unavailable instead of being dropped
from a partial placement map.

Reason: placement projection decides whether an Agent-owned ability is locally
hosted or should route through remote presence/directory state. A partial map
built by silently filtering invalid identities lets one corrupted row disappear
while the resolver still treats the aggregate as available. That is another
compatibility-style repair path.

Boundary: an empty `host_device_agent_ura` remains a valid first-boot empty
placement state. Malformed hosted Agent URAs and non-Agent hosted identities are
invalid projection state and fail closed before route selection can use them.

## Registered-Agent registry projection fail-closed cutover

Decision: registered-Agent name and surface projections must parse every
registry key through the canonical `AgentId` model and return a typed projection
error on the first malformed key.

Reason: these projections feed admission self-target locality, Mission
traditional target conflict detection, and skill ownership discovery. Treating a
bad row as absent lets product code continue from a partial aggregate snapshot,
which is a compatibility repair path rather than a canonical runtime state.

Boundary: an empty registry remains valid. A malformed key in memory or on disk
is corrupt aggregate state and must make the dependent projection unavailable or
return an explicit product-boundary error; no alias matching, default tenant
repair, or row skipping is introduced.

## Owner projection cursor URA binding cutover

Decision: owner projection cursor persistence and publication integrity share
one canonical owner/host binding rule:

- Agent owner -> same-realm Device host;
- Device owner -> the same Device URA;
- Authority owner -> the same Authority URA.

Reason: owner projection cursors are durable lifecycle facts used by
republication, heartbeat refresh, and purge high-water fencing. Accepting
arbitrary strings such as `"z"`/`"host"` at the cursor layer lets malformed
state survive until a later authority boundary rejects it, which recreates a
second lifecycle authority and makes product failures harder to diagnose.

Boundary: missing cursor store remains the first-boot empty state. A present
cursor store with malformed or contradictory owner/host URAs is corrupt state
and fails closed without row skipping or host repair.

## Builtin plugin provider entrypoint binding cutover

Decision: validate a provider's compiled entrypoint against its manifest
entrypoint inside `PluginProviderRegistry::binding_from_provider`, before
creating a `BuiltinPluginBinding`.

Reason: the provider registry is the daemon-owned list of shipped native-static
and desktop-companion providers. If it only validates package id, the manifest
entrypoint remains a second identity fact checked later by package loading. That
split lets a mismatched provider/manifest pair exist as an apparently valid
builtin binding, weakening plugin package ownership and making plugin conflict
diagnosis depend on load order.

Boundary: this reuses the existing manifest-layer builtin entrypoint validator.
It does not add a new plugin-specific rule or compatibility alias; a mismatched
entrypoint is a corrupt provider binding and fails closed before projection.

## Desktop companion daemon lifecycle fail-closed audit

Decision: daemon-start companion reconcile remains non-fatal to runtime ready,
but every package-plan failure and desired-state read failure becomes a typed
`DesktopCompanionReconcileFailure`. Runtime-stop companion cleanup remains
best-effort, but a stop-on-runtime-stop companion that cannot be planned emits a
warning instead of disappearing from the stop stage.

Reason: desktop companions are product/session-adjacent plugins. They must not
be able to prevent the canonical invocation daemon from becoming ready, but
their lifecycle state is still durable product state. Silently `continue`-ing
when a manifest cannot be planned or the companion state store cannot be read
turns corrupt lifecycle state into an implicit "nothing to do" default, which is
a compatibility repair path.

Boundary: non-companion packages and disabled companions remain valid no-op
states. Malformed companion package plans and corrupt desired-state stores are
reported through existing warning/op-event/stage-warning paths; no new fallback,
migration, or legacy state translation is introduced.

## Go runtime-host detach provider fail-closed parity

Decision: `RuntimeLifecycleTransportFunc.Detach` must reject a missing
`DetachFunc` with the same neutral invalid-runtime-client diagnostic style as
discover/start/attach/status/open-runtime/stop.

Reason: detach is part of the runtime-host lifecycle authority seam. The Python
SDK already models `detach` as a required transport operation, while the Go
function adapter silently returned success when a provider omitted it. That made
`ConnectLocalRuntimeHost` able to open a runtime client and report success
without proving that the lifecycle handle was detached at the provider boundary.

Boundary: a handle may still make repeated local detach calls idempotent after a
successful provider detach. `RuntimeTransportFunc.Close` remains an optional
resource cleanup no-op because it is not the runtime-host lifecycle detach
authority. No product-specific daemon or EasyNet vocabulary is added to the SDK.

## Session handler error-frame fact cutover

Decision: remote session handler error frames must deserialize through a shared
`HandlerErrorFrame` value object requiring non-empty `code` and `message` before
the daemon projects a `SessionFailure`.

Reason: file-transfer and JSON-frame bidi handlers previously accepted partial
error frames and filled missing failure facts from default strings. That made a
malformed handler protocol frame indistinguishable from an authored product
failure and weakened the terminal failure fact model that receipt/session
projections rely on.

Boundary: complete handler error frames still project to typed data until the
runtime terminal receipt closes the invocation lifecycle. Incomplete handler
error frames now fail the dispatch mapping as protocol/schema violations; the
daemon does not synthesize business failure codes or messages for them.

## Node runtime receipt provider cutover

Decision: Node SDK must expose a provider-backed `RuntimeReceiptProvider`
composed over a generic `RuntimeAbilityClient`, matching Go and Python. Receipt
history reads resolve descriptors through the receipt-history provider and
dispatch through the governance-read seam, not through public descriptor-bound
action ingress.

Reason: clean Hub/device testing shows remote catalog/resource public ingress is
healthy, while direct remote `invocation.history.list` is correctly rejected.
The remaining product failure mode is an SDK/product boundary gap: Node-facing
product code can still lack a canonical receipt provider and therefore rebuild a
history invocation by hand, producing descriptor/admission/session-subject
errors downstream.

Boundary: the generic public `InvocationBuilder` and public action path continue
to reject runtime governance read descriptors. The new provider path does not
add a compatibility alias; callers must provide explicit caller, callee,
subject, nonce, causal context, and authority facts before dispatch.

## Runtime response state projection fail-closed cutover

Decision: attempt audit response finalization now uses a dedicated
`RuntimeResponseStateProjection` value object. Known Axon lifecycle states map
to the existing attempt states; undecodable wire values become terminal
`RuntimeFailed` rows with `PROTOCOL_MISMATCH`, `protocol_decode`, and
non-retryable diagnostics.

Reason: an invalid `InvokeResponse.state` is a protocol/schema mismatch between
runtime and daemon, not a valid in-flight lifecycle state. Recording it as
`unknown` and falling through to `runtime_started` made product diagnostics and
history views believe an invocation was still live even though the daemon could
not decode the runtime's terminal fact.

Boundary: valid runtime responses preserve the existing public attempt-history
projection. The cutover adds no compatibility alias and no fallback state; it
only turns malformed protocol data into a deterministic terminal audit failure.

## Presence resolve-only slot cutover

Decision: `PresenceRegistry` now models two explicit slot states:
negotiated dispatch sessions and resolve-only visibility rows. A dispatch
session is admitted only with a canonical carrier contract and a 16-byte
claimant fingerprint. Device-mode self presence seeding uses the resolve-only
path and therefore cannot expose a dispatch sender.

Reason: device-mode self presence is a directory visibility fact used by local
backend resolve. It is not a real inbound `session.open` reverse channel.
Representing it as a no-op dispatch sender with an empty canonical contract made
presence look like one lifecycle when it actually contained two, and forced
dispatch code to defend against a fake session that could never complete.

Boundary: directory readers still see resolve-only presence through
`snapshot/contains/online_count`. Unary, stream, and bidi dispatch continue to
enter only through `lookup_dispatch_session` and therefore require a negotiated
session. No product compatibility dispatch path is preserved.

## FFI native session caller-authority cutover

Decision: FFI native invocation binding now resolves a `RuntimeSessionCallerAuthority`
from the attached daemon discovery and admits exactly two unsigned caller
classes: the runtime owner URA and the paired User URA proven by daemon Ready.
Signer loading for both classes goes through `load_runtime_caller_signer`.

Reason: Go/Python daemon paths already distinguish runtime-owner signers from
managed User signers. The FFI native path still called
`RuntimeSigningIdentity::load_default`, which is intentionally runtime-owner
only and therefore cannot load managed User callers. That left language-binding
traffic able to regress into `keyring entry not found` for User URAs even after
daemon boot had proven paired-user signer custody.

Boundary: explicit caller signatures still pass through without using daemon
custody. Unsigned foreign callers remain rejected before signing. This is not a
compatibility fallback; it is the same Ready-proven paired-user authority model
used by daemon runtime-state reads.
