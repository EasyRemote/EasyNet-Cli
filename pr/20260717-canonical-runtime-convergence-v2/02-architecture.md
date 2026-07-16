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
