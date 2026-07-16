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
