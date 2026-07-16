# Canonical Runtime Convergence V2 - Architecture

## Layering

Axon owns canonical invocation, descriptor-bound proof, admission, lifecycle,
and receipt semantics. EasyNet-Cli owns daemon policy, local resources,
providers, and product execution surfaces. Backend code submits complete
invocations and does not become a proof or receipt authority.

## Current Slice: Descriptor Governed Schema Projection

Owner: EasyNet-Cli daemon ability control plane.

The governed schema hash projection is a daemon descriptor fact. It is not an
Axon protocol primitive and it is not a public SDK compatibility surface.

The projection must keep all hash inputs explicit:

- input schema;
- output receipt schema;
- access policy;
- hints;
- receipt semantics;
- admission action;
- description;
- source; and
- metadata.

Those fields now move as one semantic projection object instead of as a loose
parameter list. This keeps the descriptor hash boundary cohesive while
preserving the exact JSON projection used by existing hash computation.

## Current Slice: Mission Terminal Transition Facts

Owner: EasyNet-Cli daemon Mission/EAL orchestration.

Mission/EAL remains a daemon-owned composite `AbilityImpl` strategy, not an
Axon invocation ontology. Its persisted run lifecycle still needs a real state
machine: `running` may transition to exactly one terminal state, and terminal
states are immutable.

The terminal transition now separates:

- run context: mission name, source file, trace id, start timestamp, duration,
  and parent invocation context;
- completion facts: total/completed/failed step counts and ability graph
  traces; and
- failure facts: total step count and error text.

This keeps the mission lifecycle transition explicit without turning Mission
state into a second invocation/proof model.

## Current Slice: Kernel Default Lifecycle Construction

Owner: EasyNet-Cli daemon boot/runtime kernel.

`Kernel` is the daemon-local execution entry point that owns admission,
permission gating, LocalRuntime dispatch, and receipt projection for internal
kernel calls. The default kernel lifecycle is the fresh-subservice,
allow-all-broker variant already exposed as `Kernel::new()`.

`Default` now delegates to `Kernel::new()` so generic lifecycle construction
uses the same domain constructor instead of leaving `new()` as an isolated
custom path. The subscriber-broker constructor remains explicit because it is a
daemon boot policy choice, not the object default.

## Current Slice: Bidi Event Payload Ownership

Owner: EasyNet-Cli daemon invocation stream/bidi dispatch.

Pending stream and bidi carrier events are bounded lifecycle signals: admission,
data chunks, and exactly one terminal result. Large protobuf receipt/result
payloads should not inflate every queued event variant because these events
flow through bounded channels and session drain loops.

The large admission, terminal, and local-bidi down-frame payloads now use boxed
ownership at the event boundary. This preserves the event state machine while
keeping queue element size bounded by pointer-sized large variants.

## Current Slice: Session Escalation Reply Ownership

Owner: EasyNet-Cli daemon reverse session escalation.

Session escalation correlates one device-originated request with exactly one
hub reply. Canonical product replies carry full `InvokeResponse` proof material,
while daemon-control replies carry a smaller control outcome. The enum now boxes
the canonical response so the correlation table and oneshot channel do not make
every reply slot the size of the largest proof-carrying variant.

The session outbox ready-hook list is named as `SessionReadyHook` and
`SessionReadyHooks`; the outbox still owns hook execution after a live sender is
published, but the public lifecycle field no longer exposes nested collection
mechanics as its type identity.

## Current Slice: Dispatch Result Projection

Owner: EasyNet-Cli daemon Axon bridge and local session dispatch.

Axon finalization and carrier-v1 session results project the same canonical
result facts: call identity, terminal state, completion payload, failure text,
and admission/terminal receipts. Those facts must be explicit at the projection
boundary so a result literal cannot silently inherit default receipt or failure
fields.

The bridge now expresses completion payload selection as an explicit terminal
state branch, and session dispatch result literals enumerate every carrier
field without no-op default tails. This is a local projection cleanup only; it
does not change the canonical result model or introduce a compatibility path.

## Current Slice: Resolver Ingress Tuple Source

Owner: EasyNet-Cli daemon invocation routing.

Target resolution is the first daemon policy boundary that can distinguish a
daemon-local system call from a public ingress call. Those sources cannot share
an `Option<subject>` shape because absence means different things: system calls
may use a named descriptor-derived subject policy, while public ingress must
already carry the signed subject and causal context.

`InvocationPlanIngress` now models that source as a closed enum:

- `DaemonSystem` maps to the explicit daemon-system subject and root causal
  derivation policy.
- `PublicIngress` requires an inspectable subject and causal context recovered
  from signed ingress material before target resolution.

The resolver validates public-ingress subjects as URAs and refuses to interpret
an invalid or absent public subject as daemon-system derivation. This narrows
RF-8 at the resolver boundary; direct `InvocationTarget` construction sites
remain to migrate before the fork is closed.

## Current Slice: Invocation Target Construction Boundary

Owner: EasyNet-Cli daemon invocation routing.

`InvocationTarget` is the daemon-local target value that all local ability
adapters eventually submit to the shared registry and Axon `LocalRuntime`.
Direct struct literals duplicated the same policy tuple at many adapters:
local scope, daemon-system subject derivation, root causal context, and empty
transport metadata. That made it too easy for public ingress and system calls
to look structurally identical.

The target type now owns named constructors for the common states:

- `local_daemon_system` for descriptor-derived daemon-system calls;
- `local_daemon_system_with_subject` for daemon-system calls with an explicit
  acted-on subject; and
- `local_explicit_tuple` for callers that already hold explicit subject and
  causal-context facts.

The first production migration covers agent discover/invoke and edge
integration adapters for MCP, A2A, and OpenAI compatibility. Remaining direct
`InvocationTarget` literals are still inventory for RF-8/RF-7 migration.

## Current Slice: Plugin Host Target Test Boundary

Owner: EasyNet-Cli daemon plugin host.

Plugin host tests exercise declarative plugin execution through the same local
registry target shape that production plugin dispatch uses. Those tests should
not preserve hand-assembled target literals because they become copyable
examples of the obsolete assembly style.

The plugin host tests now construct daemon-system and explicit-subject local
targets through `InvocationTarget` constructors. This keeps plugin execution
fixtures aligned with the canonical routing target boundary while preserving
the tested plugin load, hot-register, hot-unregister, and rejection behavior.

## Current Slice: Resource and Governance Target Boundary

Owner: EasyNet-Cli daemon resource and governance adapters.

Ability-backed page APIs and governance health dispatch both route through the
daemon local registry. They should not hand-assemble local routing targets
because the adapter's responsibility is selecting the product ability and
subject, not restating routing target state.

The pages ability adapter now uses the explicit-subject daemon-system target
constructor when forwarding a page API request to a selected ability. The
governance health test fixture uses the daemon-system constructor for its local
smoke dispatch. This continues the RF-8/RF-7 migration without claiming that
all local target construction has moved.

## Current Slice: Media Subject Target Fixtures

Owner: EasyNet-Cli daemon media resource adapters.

Media resource handlers are important subject-boundary tests because they
reject missing subjects, subject-in-args fallback, wrong resource type, corrupt
resource tables, and unknown resource URAs. Their tests should therefore
exercise subject policy through the same routing target constructors as other
daemon adapters rather than retaining local target literals.

The mic.subscribe and screen.snapshot/screen.subscribe fixtures now construct
local daemon-system targets through `InvocationTarget` constructors for both
explicit resource subjects and derived-system missing-subject cases. Camera
fixtures remain in the direct-literal inventory for a separate slice.

## Current Slice: Camera Subject Target Fixtures

Owner: EasyNet-Cli daemon camera media adapter.

Camera media tests cover the broadest media subject surface: snapshot,
subscribe, recording start/stop, duplicate recording rejection, missing
subject, unknown subject, wrong resource type, and subject-in-args rejection.
Those fixtures must use the same routing target construction boundary as the
mic and screen tests so media subject policy is exercised consistently.

The camera fixtures now construct explicit resource-subject targets and
missing-subject daemon-system targets through `InvocationTarget` constructors.
This closes the media fixture portion of the direct target construction
inventory while leaving non-media target literals for later RF-8/RF-7 slices.

## Current Slice: LocalRuntime Subject Derivation Ownership

Owner: EasyNet-Cli daemon invocation routing.

The target resolver already distinguishes public ingress from daemon-system
calls, but `local_runtime_invoker` still carried a second subject derivation
policy for descriptor defaults. That duplicated the RF-8 decision point: the
same missing subject could be interpreted by the target domain and again by
the LocalRuntime adapter.

`InvocationTarget` now owns resolution of its subject binding against the
selected callee. Explicit subjects are validated as URAs, daemon-system calls
resolve through the named descriptor-default policy, and hub-owned abilities
use the Ability URA as subject because Hub identities are not valid Axon
subjects. The LocalRuntime adapter consumes the resolved subject and causal
context; it no longer defines a separate fallback subject state machine.

## Current Slice: Mission Catalog Gateway Target Boundary

Owner: EasyNet-Cli daemon Mission/EAL test gateway.

Production Mission child dispatch already enters Axon through the admitted
parent `AbilityContext`, `ChildInvocationRequest`, descriptor-bound target,
parent subject, and runtime-managed causal chain. The cfg-test catalog gateway
is not the production proof path, but it is the Mission/EAL test port used by
orchestration tests; it should not preserve a hand-written local target
literal that copies routing policy.

The catalog gateway now uses `InvocationTarget::local_daemon_system` for its
daemon-system test dispatch. This keeps Mission/EAL test adapters aligned with
the canonical target construction boundary while preserving the production
child-invocation architecture.

## Current Slice: Ability Dispatch Target Fixture Boundary

Owner: EasyNet-Cli daemon ability registry dispatch tests.

`AxonAbilityCatalog` is the daemon registry boundary that forwards local RPC,
stream, and bidi calls into the attached Axon `LocalRuntime`. Its tests cover
the core envelope-aware behavior: explicit subjects, derived system subjects,
remote-route rejection, stream/RPC mode separation, and bidi lifecycle guards.
Those tests should exercise routing target construction through the target
value object instead of restating local or remote scope, system subject policy,
root causal context, and empty metadata literals.

`InvocationTarget` now also owns the remote daemon-system constructor, backed
by the same internal scoped binding constructor as local targets. Ability
dispatch fixtures now use named constructors for local RPC, stream, bidi, and
remote guard targets. This removes a high-visibility set of obsolete tuple
assembly examples from the registry tests while leaving protobuf transport
targets and other remaining inventory for separate RF-8/RF-7 slices.

## Current Slice: LocalRuntime Invoker Target Fixture Boundary

Owner: EasyNet-Cli daemon Axon bridge tests.

`local_runtime_invoker` is the daemon adapter that lowers routing targets into
descriptor-bound Axon `LocalRuntime` requests. Its production path now consumes
resolved target tuple facts, but the module tests still used a hand-written
`InvocationTarget` fixture that repeated local scope, daemon-system subject
policy, root causal context, and empty metadata.

The test helper now constructs explicit-subject and daemon-system targets
through the `InvocationTarget` constructors. This keeps the LocalRuntime
adapter tests focused on envelope lowering and finalized result projection
rather than preserving a second example of target assembly.

## Current Slice: Builtins Smoke Target Fixture Boundary

Owner: EasyNet-Cli daemon built-in ability test fixtures.

`real_invoke_tests` and catalog assembly tests are broad smoke coverage for
daemon built-in abilities and registry assembly. Their local invocation helpers
are copied into many tests, so a hand-written `InvocationTarget` literal there
keeps the obsolete tuple assembly style highly visible.

The shared smoke helper and catalog assembly loop now use
`InvocationTarget::local_daemon_system`. Per-test subject and metadata
overrides still attach through the target value object's builder methods, but
the default system/root tuple policy is no longer repeated in these fixtures.

## Current Slice: CLI Agent Command Target Fixture Boundary

Owner: EasyNet-Cli CLI agent command tests.

The CLI agent command fixture builds an in-process daemon ability catalog and
invokes agent-management abilities through the same local routing target shape
used by the daemon. The fixture is not a public daemon invocation surface, but
it is a command-layer adapter test; keeping a hand-written local
`InvocationTarget` literal there preserves a second target assembly idiom at
the CLI/daemon boundary.

The fixture now calls `InvocationTarget::local_daemon_system` when it dispatches
ordinary agent command abilities. The special envelope-aware branch still uses
its explicit `EnvelopeContext` helper because that path tests envelope handler
behavior, not routing target construction.

## Current Slice: Protobuf Transport Target Projection Boundary

Owner: EasyNet-Cli daemon Invocation protobuf wire facade.

Bidi `EnvelopeOpen.target` is a protobuf transport selector, not the canonical
Invocation tuple owner. The signed envelope still owns caller, callee, subject,
nonce, causal context, and descriptor-bound proof material. However, SDK bidi
frame construction, `session.open`, the local daemon gRPC bidi adapter, and
crate-internal service test helpers were each hand-building the same
`InvocationTarget { ability_name, ..Default::default() }` projection. That
kept transport frame assembly duplicated at the exact boundary where the daemon
should have one wire-shape construction point.

`invocation_wire::wire_invocation_target` now owns that protobuf selector
projection and rejects empty selectors before frame construction. Production
callers pass either the descriptor ref or route-local ability name required by
their ingress contract, but they no longer construct the proto target literal
themselves. External integration tests may still hand-build raw protobuf input
when they intentionally model an outside client; that is fixture input, not an
internal construction path.

## Current Slice: RF-5 Rust Public Surface Signer Fallback Removal

Owner: EasyNet-Axon Rust SDK runtime-admin public surface and EasyNet-Cli SDK
conformance model.

The canonical SDK capability matrix must not count process-local signer
fallback helpers as evidence for a generic runtime capability, and the canonical
Rust Axon SDK must not expose process-local generated auth as public runtime
administration.

The Axon Rust SDK no longer exports `GeneratedSubjectAuth`,
`generate_subject_auth`, `generate_private_agent_auth`, or
`generate_private_hub_auth` from `invocation::runtime_admin`. The remaining
runtime-admin subject helpers are pure identifier helpers; they do not mint
secret material or define a local signing authority.

The EasyNet-Cli public-surface policy now classifies default/generated subject
auth, generated private agent/hub auth, process-local signer, and private-key
authenticator symbols as RF-5 non-canonical signer fallback defects. The
regenerated manifest and parity matrix contain no generated auth symbols. If
any SDK reintroduces this class of helper, the V2 gate fails before the symbol
can be counted as canonical capability evidence.

This slice removes the Rust public fallback root and closes its conformance
evidence path. Full RF-5 remains open until all SDK languages converge on the
same explicit signer-handle/daemon KeyService authority model and the remaining
plain proof helper cutover is complete.

## Current Slice: RF-3 Public Plain Proof Helper Removal

Owner: EasyNet-Axon Rust/Python invocation public surface and EasyNet-Cli SDK
conformance gates.

The descriptor-bound envelope is the only canonical admission/proof boundary.
The plain encoder and plain admission helpers remain useful only as internal
test fixtures for historical vector stability; they must not be public SDK
entry points because they sign or admit an envelope without binding an
`AbilityDescriptorRef` and derived `EntityRef`.

The Axon Rust SDK no longer exports the plain helper group from
`invocation::*`, and the underlying Rust helpers are crate-internal
`#[cfg(test)]` functions instead of rustdoc-visible public API:
`canonical_invocation_bytes`, `sign_invocation`,
`verify_invocation_signature`, `verify_phase`, `verify_signature`, and
`run_admission`. Runtime-admin resolver tests were migrated to
`DescriptorBoundEnvelope`, `sign_descriptor_bound_invocation`, and
`verify_descriptor_bound_invocation_signature` so test fixtures no longer
teach the obsolete proof boundary.

The Axon Python invocation package root no longer exports the same plain helper
group. It instead exposes descriptor-bound admission replacements:
`run_descriptor_bound_admission` and `verify_descriptor_bound_signature`,
alongside the existing descriptor-bound canonical bytes and signature helpers.

The EasyNet-Cli V2 conformance gate now rejects plain proof helpers anywhere in
the public manifest, including `non_canonical` quarantine. This changes RF-3
from "legacy public export is documented" to "legacy public export is a gate
failure." Full RF-3 remains open until all language packages and old
vectors/examples are audited against the same descriptor-bound-only public
contract.

## Current Slice: RF-3 Python Submodule Plain Proof Hardening

Owner: EasyNet-Axon Python invocation implementation and EasyNet-Cli V2 source
gate.

Removing package-root exports was not sufficient for Python because SDK users
can still import non-underscore functions from submodules. The plain proof
helpers in `easynet_axon.invocation.axiom` and
`easynet_axon.invocation.admission` therefore remained discoverable as normal
Python module API even though the canonical proof boundary is descriptor-bound.

The Python plain helper group is now private by name:
`_canonical_invocation_bytes`, `_sign_invocation`,
`_verify_invocation_signature`, `_verify_signature`, and `_run_admission`.
Runtime admission tests no longer import the plain helpers. Historical axiom
vector tests and cross-language bundle producer tests use the private fixtures
explicitly, which documents that they are vector fixtures rather than public
SDK proof APIs.

The V2 convergence script now performs a direct Axon source scan for public
Rust and Python plain proof/admission helpers. This closes the gap where the
manifest proved the EasyNet-Cli facade surface was clean but could not detect
an Axon Python submodule exposing the obsolete proof boundary.

## Current Slice: RF-6 Java LocalRuntime Receipt Proof Facts

Owner: EasyNet-Axon Java LocalRuntime receipt binding and EasyNet-Cli V2
conformance gate.

Java receipt constructors already reject omitted authority and proof facts, but
the production `LocalRuntime` binding path still created receipts with
`ReceiptProofFacts.empty()`. That preserved the exact RF-6 defect after the
constructor cleanup: the runtime could produce canonical receipts whose proof
facts did not identify the descriptor version, subject, authority proof,
runtime environment, input hash, output hash, or causal parents.

The Java LocalRuntime now constructs receipt proof facts at the admission
binding boundary. Signed descriptor-bound calls derive proof facts from the
descriptor-bound envelope and caller authority. Plain `invokeAsync` calls use a
separate system-local proof identity, `system-local.invoke.v1`, so internal
SystemAgent receipts remain auditable without pretending to be external
descriptor-bound calls.

`InvocationReceipt.AxiomBinding.withPayloadDigest` now carries per-event proof
facts forward by replacing the immutable proof-fact output hash with the event
payload hash. This keeps output facts attached to the receipt event that is
actually emitted instead of leaving the binding with an admission-time empty
output fact.

This slice removes the Java LocalRuntime empty proof-fact production path and
adds an EasyNet-Cli V2 source gate for that exact regression. It does not close
RF-6 globally: Java still needs full descriptor proof-binding metadata parity
with Rust, and the remaining language examples/tests/constructors must be
audited before receipt proof-fact convergence is complete.

## Current Slice: RF-6 Python LocalRuntime Receipt Proof Facts

Owner: EasyNet-Axon Python LocalRuntime receipt binding and EasyNet-Cli V2
conformance gate.

The Python LocalRuntime had the same RF-6 production defect as Java: signed
descriptor-bound invocations and system-local `invoke_async` both created
`AxiomBinding` values with default `ReceiptProofFacts()`. Because
`_InvocationCore.emit` refreshed only `payload_digest`, every emitted receipt
could still carry an admission-time empty proof-fact block even when the
receipt payload digest changed per event.

The Python LocalRuntime now constructs receipt proof facts at the binding
boundary through `_LocalReceiptProofFacts`. Signed invocations derive facts
from the descriptor-bound envelope, caller authority, subject, causal parent
receipts, and runtime admission hook. System-local calls use the separate
`system-local.invoke.v1` proof identity so infrastructure-originated local
calls are auditable without being mislabelled as external descriptor-bound
signed calls.

`ReceiptProofFacts.with_output_hash` and `_InvocationCore.emit` now keep each
receipt's proof output hash aligned with the event payload hash. This mirrors
the Java correction and prevents terminal receipts from combining a refreshed
payload digest with stale empty proof output facts.

This slice removes the Python LocalRuntime empty proof-fact production path and
adds a V2 source gate for that regression. RF-6 remains open for Go, Node,
remaining examples/tests, and full descriptor proof-binding parity.

## Current Slice: RF-6 Go LocalRuntime Receipt Proof Facts

Owner: EasyNet-Axon Go LocalRuntime receipt binding and EasyNet-Cli V2
conformance gate.

The Go LocalRuntime carried the same production RF-6 defect: descriptor-bound
signed invocations and system-local `InvokeAsync` calls built `AxiomBinding`
values with `EmptyReceiptProofFacts()`. The receipt constructor was already
able to carry complete facts, but the runtime binding boundary still admitted
empty descriptor, subject, authority, runtime, input, output, and parent
receipt facts.

The Go LocalRuntime now constructs receipt proof facts where the runtime
creates the admitted `AxiomBinding`. Signed invocations derive proof facts
from the descriptor-bound envelope and caller authority. System-local
`InvokeAsync` uses the separate `system-local.invoke.v1` proof identity, so
internal runtime-originated calls remain auditable without being represented
as externally signed descriptor-bound requests.

Full Go invocation-package verification exposed a second Go/Rust fork: Go
accepted `ability_ura@version` descriptor refs while the Rust verifier requires
`ability_ura@version#descriptor_hash!admission_action`. The Go invocation
parser now uses the Rust canonical descriptor-ref shape, canonicalizes the
descriptor hash casing, validates the admission action, and signs
cross-language bundle fixtures with descriptor-bound bytes.

`ReceiptProofFacts.WithOutputHash` and `InvocationCore.emit` now refresh the
proof output hash with each emitted event payload digest. This matches the
Java and Python RF-6 shape: admission owns descriptor/input/authority facts,
while event emission owns output facts.

This slice removes the Go LocalRuntime empty proof-fact production path and
adds a V2 gate/self-test for that exact regression. RF-6 remains open for
Node, remaining examples/tests, and full descriptor proof-binding parity.

## Current Slice: RF-4 Go Runtime Lifecycle Facade

Owner: EasyNet-Axon Go LocalRuntime lifecycle/control surface.

The Go LocalRuntime already had an internal lifecycle state machine for
generation-checked cancellation, bounded message inbox delivery, event
sequencing, parent-child cancellation propagation, and receipt-chain audit
inspection. The public runtime facade, however, only exposed those controls
through `InvocationHandle`. Industrial lifecycle vectors also exercise
runtime-level control by invocation id, so the missing public methods left Go
with a facade gap even though the internal state machine existed.

The Go LocalRuntime now exposes `CoreOf`, `SendMessage`, and `Cancel` as
runtime-level methods. `SendMessage` and `Cancel` do not create a second
control path: they resolve the current generation token and delegate into the
same `sendWithControl` and `cancelWithControl` paths used by handles. That
keeps ABA protection, idempotent cancel intent latching, bounded inbox
delivery, cleanup-before-terminal-receipt ordering, and child cancellation
propagation under one lifecycle owner.

`CoreOf` is explicitly inspection-oriented. Runtime mutation still goes
through `LocalRuntime` or `InvocationHandle` control methods, while
`InvocationCore` provides snapshot/current-state evidence for audit and
industrial lifecycle vectors.

This slice removes the Go lifecycle facade gap that prevented the Go
industrial audit/cancel/message vectors from compiling. RF-4 remains open for
the shared machine-readable transition vectors and cross-language provider
status cutover.

## Current Slice: RF-6 Node LocalRuntime Receipt Proof Facts

Owner: EasyNet-Axon Node LocalRuntime receipt binding and EasyNet-Cli V2
conformance gate.

The Node LocalRuntime had the same RF-6 production defect previously removed
from Java, Python, and Go: descriptor-bound signed invocations and
system-local `invokeAsync` calls built `AxiomBinding` values with
`EMPTY_RECEIPT_PROOF_FACTS`. Because `InvocationCore.emit` refreshed only the
event payload digest, terminal receipts could still carry an admission-time
empty proof-fact block even when the emitted payload hash changed.

The Node LocalRuntime now constructs receipt proof facts at the admitted
binding boundary. Signed descriptor-bound calls derive subject, descriptor
version, authority proof, input hash, parent receipts, and runtime environment
from the admitted envelope and caller authority. System-local calls use the
separate `system-local.invoke.v1` proof identity so infrastructure-originated
local calls remain auditable without being represented as external signed
descriptor-bound calls.

`receiptProofFactsWithOutputHash` and `InvocationCore.emit` now refresh the
proof output hash with each emitted event payload digest. This matches the
Java/Python/Go ownership model: admission owns descriptor/input/authority
facts, while event emission owns output facts.

This slice removes the Node LocalRuntime empty proof-fact production path and
adds a V2 gate/self-test for that exact regression. RF-6 remains open for
remaining examples/tests and full descriptor proof-binding parity.

## Current Slice: RF-5 Rust Local-Fast Signer Feature Removal

Owner: EasyNet-Axon Rust SDK signer boundary and EasyNet-Cli V2 conformance
gate.

The Rust SDK still exposed a public `local-fast-probes` Cargo feature that
made process-local receipt and invocation signing helpers available outside
crate tests. That feature was a convenience probe, but architecturally it
kept a public path where consumers could build receipt authority and
invocation signing state without an explicit provider or downstream
KeyService boundary.

The Rust SDK now removes the `local-fast-probes` feature entirely. Local-fast
constructors and process-local signer helpers are restricted to `cfg(test)`
crate internals, while integration tests and examples construct explicit
test/example signing providers at their own boundary. This preserves test
ergonomics without publishing a canonical fallback authority model.

The `receipt_closure` example now demonstrates the intended provider-backed
shape by passing an explicit receipt signing authority provider into
`LocalRuntime`. Integration fixtures use `descriptor_bound_support` test
providers rather than SDK public fallback helpers. The EasyNet-Cli V2 gate now
rejects reintroducing the feature, public feature cfg, or external
example/test consumption of the fallback helper group.

EasyNet-Cli no longer requests a downstream `local-fast-probes` feature from
Axon. The maintainer `real-user-smoke` binary now owns an explicit local smoke
receipt provider, and the Pages integration test owns a bounded Pages test
provider. Both callers construct providers directly at their own test/probe
boundary instead of depending on SDK-published fallback constructors.
The V2 gate now checks both sides of this boundary: Axon must not publish the
feature/helper consumption path, and EasyNet-Cli must not request or consume
it downstream.

This slice removes one Rust public fallback signer seam. RF-5 remains open for
cross-language signer-handle parity and daemon KeyService authority cutover.

## Current Slice: RF-5 Runtime Client Subject Auth Generator Removal

Owner: EasyNet-Axon runtime client SDK signing boundary and EasyNet-Cli V2
conformance gate.

The Axon runtime client SDK still exposed `AxonClient::generate_subject_auth`,
which created process-local Ed25519 secret material and returned it as
`EasyNetUserAuth`. Even though authenticated calls already fail closed without
`AbilityCallOptions::auth`, this helper made the SDK a signing-material
generator instead of a consumer of host-managed authority.

The generator is removed. `EasyNetUserAuth` remains as an explicit
host-supplied DTO because current authenticated call paths still need a
concrete signing input while signer-handle parity is unfinished. Tests now use
a local `host_auth_fixture` with fixed material so fixture ownership is
visible and cannot be mistaken for SDK authority generation.

The V2 gate now performs a source-level RF-5 scan across Axon runtime client
SDK, canonical SDK packages, and runtime source so process-local fallback
helpers such as `generate_subject_auth`, `default_auth_for_subject`, generated
private agent/hub auth, `ProcessLocalSigner`, and `PrivateKeyAuthenticator`
cannot re-enter outside tests.

This slice removes another public process-local signer fallback. RF-5 remains
open until host auth DTOs converge to signer handles or daemon KeyService
authority across language facades.

## Current Slice: RF-3 Go Public Plain Proof Helper Removal

Owner: EasyNet-Axon Go invocation facade and EasyNet-Cli V2 conformance gate.

The Go invocation package still exported the plain proof helper group:
`CanonicalInvocationBytes`, `SignInvocation`, `VerifyInvocationSignature`,
`VerifySignature`, and `RunAdmission`. These helpers sign or verify the legacy
plain envelope bytes and therefore expose the second proof model that RF-3 is
removing.

The Go package now keeps those helpers as package-private fixture functions.
Existing Go package tests can still use them for historical plain vector
stability, but downstream callers cannot import them as SDK proof APIs. The
descriptor-bound public proof path remains exported through
`CanonicalDescriptorBoundInvocationBytes`, `SignDescriptorBoundInvocation`,
`VerifyDescriptorBoundInvocationSignature`, `VerifyDescriptorBoundSignature`,
and `RunDescriptorBoundAdmission`.

`sdk/API_MAPPING.md` now documents the descriptor-bound public names instead
of the legacy plain proof names. The V2 source gate now rejects the Go
capitalized plain helper group in both the Go invocation package and the SDK
API mapping document.

This slice closes the Go public plain proof helper surface. RF-3 remains open
for remaining language package/vector/example audit and any other public
plain proof surfaces.

## Current Slice: RF-3 Node Public Plain Proof Helper Removal

Owner: EasyNet-Axon Node invocation facade and EasyNet-Cli V2 conformance
gate.

The Node SDK still exposed the plain proof helper group from its root and
invocation entry points: `canonicalInvocationBytes`, `signInvocation`,
`verifyInvocationSignature`, `verifySignature`, and `runAdmission`. Those
helpers signed or verified the legacy plain envelope bytes and therefore made
the Node facade another public entry into the second proof model.

The Node public surface now exports only the descriptor-bound proof/admission
helpers. Historical plain vector and admission tests use explicitly named
`legacyPlain*` internal fixture helpers, and generated declarations no longer
publish those fixture names. This keeps old byte-layout regression tests
available without presenting the plain path as the SDK runtime proof API.

The Node cross-language bundle producer now signs invocation JSON with
`signDescriptorBoundInvocation` and descriptor-ref ability names, so Rust
`easynet-verify` accepts the Node bundle through the descriptor-bound verifier
instead of rejecting it as missing descriptor binding. The V2 source gate now
rejects the old Node plain helper names in Node source, generated JS, and
generated declarations.

This slice closes the Node public plain proof helper surface. RF-3 remains
open for remaining language surfaces, package export audits, and vector/example
documentation cleanup.

## Current Slice: RF-3 Java Public Plain Proof Helper Removal

Owner: EasyNet-Axon Java invocation facade and EasyNet-Cli V2 conformance
gate.

The Java SDK still exposed the plain proof helper group as public static
methods on the invocation facade: `canonicalInvocationBytes`,
`signInvocation`, `verifyInvocationSignature`, `verifySignature`, and
`runAdmission`. Those methods signed or verified the legacy plain envelope and
therefore gave downstream Java callers a public route around the
descriptor-bound admission model.

The Java facade now keeps the plain helper group as package-private
`legacyPlain*` fixture methods. Existing same-package vector and admission
tests can still assert historical byte-layout stability, but downstream SDK
callers cannot consume the plain proof path as public API. The public Java
proof/admission surface remains descriptor-bound through
`canonicalDescriptorBoundInvocationBytes`, `signDescriptorBoundInvocation`,
`verifyDescriptorBoundInvocationSignature`, `verifyDescriptorBoundSignature`,
and `runDescriptorBoundAdmission`.

The Java cross-language bundle producer now signs invocation JSON with
`signDescriptorBoundInvocation` and descriptor-ref ability names, so Rust
`easynet-verify` validates the Java bundle through descriptor-bound signature
verification. The V2 gate now rejects Java production invocation classes that
reintroduce public static plain proof/admission helpers.

This slice closes the Java public plain proof helper surface. RF-3 remains open
for Swift and any remaining package/export/vector/example cleanup.

## Current Slice: RF-3 Swift Public Plain Proof Helper Removal

Owner: EasyNet-Axon Swift SDK invocation facade and EasyNet-Cli V2
conformance gate.

Swift public API must expose only descriptor-bound invocation proof and
admission helpers. Plain canonical bytes, plain invocation signing, plain
signature verification, and plain admission are not canonical SDK surfaces
because they omit descriptor binding from the proof boundary.

The Swift facade now keeps the plain helper group as internal `legacyPlain*`
fixture functions. Same-module `@testable` vector and admission tests can
still assert historical byte-layout stability, but downstream SDK callers
cannot consume the plain proof path as public API. The public Swift
proof/admission surface remains descriptor-bound through
`canonicalDescriptorBoundInvocationBytes`, `signDescriptorBoundInvocation`,
`verifyDescriptorBoundInvocationSignature`, `verifyDescriptorBoundSignature`,
and `runDescriptorBoundAdmission`.

The Swift cross-language bundle producer now signs invocation JSON with
`signDescriptorBoundInvocation` and descriptor-ref ability names. Public
examples and README snippets now demonstrate descriptor-bound signing. The V2
gate now rejects Swift production invocation source, README examples, and
runnable examples that reintroduce public plain proof/admission helpers or old
helper usage.

This slice closes the Swift public plain proof helper surface. It does not
complete RF-3 globally until the remaining public-surface manifest and
language/package audits are clean across all SDKs.

## Current Slice: RF-3 Go Legacy Plain Fixture Naming Hardening

Owner: EasyNet-Axon Go invocation package and EasyNet-Cli V2 conformance gate.

The Go public plain proof helper removal made the helpers package-private, but
the production invocation package still named the retired plain byte/sign/verify
and admission functions as if they were ordinary package internals:
`canonicalInvocationBytes`, `signInvocation`, `verifyInvocationSignature`,
`verifySignature`, and `runAdmission`. That left the wrong proof model
semantically normalized inside the canonical package even though it was no
longer exported.

The Go package now uses explicit `legacyPlain*` names for the retired fixture
path. Descriptor-bound helpers remain the public proof/admission boundary.
Historical Go tests can still cover old plain vector stability, but production
source can no longer reintroduce the retired helper names without failing the
V2 gate.

This slice removes a Go-internal RF-3 naming seam. It does not complete RF-3
globally because remaining package/export/vector/example audit and final
legacy deletion still require separate closure evidence.

## Current Slice: RF-3 Rust Legacy Plain Fixture Naming Hardening

Owner: EasyNet-Axon Rust invocation package and EasyNet-Cli V2 conformance
gate.

The Rust public plain proof helper removal restricted plain helpers to
`cfg(test)`, but the production invocation modules still carried retired names
such as `canonical_invocation_bytes`, `sign_invocation`,
`verify_invocation_signature`, `verify_signature`, `verify_phase`, and
`run_admission`. Even test-only helpers inside the canonical Rust package
should not read as the normal proof/admission model.

The Rust package now names the retired plain path as `legacy_plain_*`,
`sign_legacy_plain_invocation`, `verify_legacy_plain_invocation_signature`,
`verify_legacy_plain_signature`, `verify_phase_legacy_plain`, and
`run_legacy_plain_admission`. Descriptor-bound helpers remain the normal
runtime proof/admission boundary, and the common signature-bytes verifier keeps
a neutral name because it is shared by descriptor-bound verification.

The V2 gate now rejects retired Rust plain helper names anywhere in the Rust
invocation source. This removes another RF-3 semantic naming seam; it does not
complete RF-3 because final legacy deletion and remaining package/vector audit
still require closure evidence.

## Current Slice: RF-3 Python Legacy Plain Fixture Naming and Producer Hardening

Owner: EasyNet-Axon Python invocation package and EasyNet-Cli V2 conformance
gate.

The Python public plain proof helper hardening removed direct public exports,
but private invocation modules and vector tests still used retired names such
as `_canonical_invocation_bytes`, `_sign_invocation`,
`_verify_invocation_signature`, `_verify_signature`, and `_run_admission`.
Those names kept plain proof vocabulary normalized inside the canonical Python
package.

The Python package now names the retired private fixture path as
`_legacy_plain_invocation_bytes`, `_sign_legacy_plain_invocation`,
`_verify_legacy_plain_invocation_signature`,
`_verify_legacy_plain_signature`, and `_run_legacy_plain_admission`.
Descriptor-bound signing and admission remain the public proof boundary.

The Python cross-language bundle producer now signs invocation JSON with
`sign_descriptor_bound_invocation` over `DescriptorBoundEnvelope` and uses
descriptor-ref ability names. Receipt proof facts derive their subject ref and
descriptor version from the same descriptor-bound invocation fields. The V2
gate now rejects retired Python private plain helper names anywhere under the
Python SDK source.

This slice removes another RF-3 semantic naming seam and converts the Python
cross-language producer away from plain signatures. It does not complete RF-3
because final package/export/vector/example audit and legacy implementation
deletion still require closure evidence.

## Current Slice: RF-3 Node Production Legacy Plain Export Removal

Owner: EasyNet-Axon Node invocation package and EasyNet-Cli V2 conformance
gate.

The Node public plain proof helper removal renamed the retired path to
`legacyPlain*`, but the legacy encoder, signer, verifier, and admission
pipeline still lived in production `sdk/node/src/invocation` modules. Since
Node module exports are public within the SDK package source, this preserved a
second proof/admission model even though the root package surface pointed users
toward descriptor-bound helpers.

The Node production invocation source now contains only descriptor-bound
signing, verification, and admission. Historical plain vector coverage moved
to an explicit test/vector fixture under `sdk/node/scripts`, and Node admission
tests now exercise `runDescriptorBoundAdmission` with descriptor-ref ability
names. The V2 gate rejects `legacyPlain*` proof/admission names in Node
production invocation source while allowing the explicit fixture boundary.

This slice removes a production legacy implementation from the Node SDK. It
does not complete RF-3 because other language fixture boundaries, package
exports, examples, and final legacy deletion still require closure evidence.

## Current Slice: RF-3 Go Production Legacy Plain Implementation Removal

Owner: EasyNet-Axon Go invocation package and EasyNet-Cli V2 conformance gate.

The Go public plain proof helper removal and fixture renaming still left the
retired plain encoder, signer, verifier, and admission runner in production
`sdk/go/easynet/invocation` files under explicit `legacyPlain*` names. That
kept a second proof/admission implementation inside the canonical runtime
package even though the public facade had moved to descriptor-bound helpers.

The Go production invocation source now contains descriptor-bound signing,
verification, and admission only. Historical plain vector coverage is isolated
to `legacy_plain_fixtures_test.go`, so the old byte layout can still be
verified as a fixture without remaining in the SDK runtime build. Go admission
tests now construct `DescriptorBoundEnvelope` values and call
`RunDescriptorBoundAdmission` with descriptor-ref ability names.

The V2 gate now rejects Go `legacyPlain*` proof/admission names in non-test
production invocation source, with a self-test that reintroduces
`legacyPlainInvocationBytes` in a fake Go production package. This slice
removes the Go production legacy plain implementation. It does not complete
RF-3 globally because final package/export/vector/example audit and legacy
fixture closure still require separate evidence.
