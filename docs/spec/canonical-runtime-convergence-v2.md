# Canonical Runtime Convergence V2

Status: Normative for new cross-repository convergence work.

Implementation status: convergence is in progress. Section 12 records the
live audit state and must not be treated as closure evidence.

This specification consolidates the architecture-convergence objective and the
2026-07-17 EasyNet-Cli/EasyNet-Axon code-graph audit. It is the clean target
for changes that touch Axon protocol/runtime/SDK, the EasyNet-Cli daemon, or
their language facades.

This document does not authorize unsequenced deletion of a public API. It does
prohibit retaining a second canonical implementation after callers have moved.
Public compatibility is delivered by versioned edge adapters only, not by
legacy paths inside the canonical domain.

## 1. Ownership Decision

The SDK is the canonical runtime model. It is not an EasyNet SDK, an
EasyRemote SDK, a daemon SDK, or a provider of product lifecycle policy.

Axon owns generic protocol truth:

- the complete Invocation tuple, URA parsing and canonicalization;
- descriptor binding, signature verification, replay protection, admission,
  receipt proof facts, terminal closure, and stream/bidi semantics; and
- the language-neutral lifecycle contract and conformance vectors.

EasyNet-Cli owns product and device policy:

- daemon lifecycle, local key-service custody, device and Hub policy;
- plugin, MCP, EAL/Mission, scheduling, pages, media, local resources, and
  product-specific integrations; and
- route locality, provided it dispatches through Axon's public
  descriptor-bound runtime entry point.

The backend owns product API, browser state, and database projections. It
submits complete Invocations to the daemon and never becomes a second runtime,
proof, or receipt authority.

## 2. Invocation and Key Custody

Every public invocation has exactly these inspectable fields before dispatch:

```text
invoke(caller, callee, ability, subject, nonce, causal_context, args) -> receipt
```

`subject` and `causal_context` are not optional semantic fields. An adapter may
offer ergonomic builders, but it must either require values or expose its
explicit derivation policy before dispatch. It must never silently substitute
the callee, descriptor, or an empty causal context for a public caller.

Internal daemon calls are allowed only through a named `SystemInvocationIssuer`:

1. It declares all seven fields.
2. It obtains a signature from the daemon `KeyService` for `_system.local`.
3. It enters the same descriptor-bound admission path as external calls.
4. It records its system authority in the terminal receipt.

An SDK, facade, LocalRuntime convenience constructor, or test helper must not
generate and cache signing material as an authority fallback. Test-only keys
must be explicit test fixtures behind test configuration.

## 3. Root-Fork Inventory

Each row is a single convergence slice. A slice ends only after callers migrate
and the replaced path is deleted.

The final column records the audit input captured on 2026-07-17. It is retained
to explain why each slice existed; it is not a statement about the accepted
checkout. Section 12 is the current implementation status.

| ID | Severity | Root fork | Clean target | Audit evidence captured 2026-07-17 |
| --- | --- | --- | --- | --- |
| RF-1 | P0 | Product features in SDK | Generic runtime primitives only; product features move to providers or daemon plugins | `audio`, `mcp`, `tool_adapter`, `presets/remote_control`, `presets/ability_dispatch`, and product federation presets remain in non-Rust SDK surfaces. |
| RF-2 | P0 | Mission state in Axon core | Mission/EAL is a daemon-owned composite `AbilityImpl`; every remote step is a child Invocation | `core/proto/axon/v1/mission.proto`, `MissionState`, and runtime mission state remain in Axon. |
| RF-3 | P0 | Two proof/admission models | Descriptor-bound proof is the one canonical model | Rust and Python still export plain canonical bytes, `verify_signature`, and `run_admission`. |
| RF-4 | P0 | Language lifecycle divergence | One lifecycle capability matrix and one transition-vector suite | Rust LocalRuntime has deadline ownership, cancel permits, and child-deadline propagation that are not uniformly modelled by Go, Python, Node, Java, and Swift. |
| RF-5 | P0 | Process-local signing fallback | Explicit caller signer or daemon KeyService only | `default_auth_for_subject` in Axon client SDK creates/caches signer material. |
| RF-6 | P1 | Receipts permit absent proof facts | Canonical receipt construction requires complete proof facts | Java `ReceiptBody` compatibility constructors default authority/proof facts. |
| RF-7 | P1 | Daemon bypasses runtime ownership | All ability routes enter `LocalRuntime` through descriptor-bound requests | Exact routes and loopback adapters still have direct route/envelope assembly risk. |
| RF-8 | P1 | Invocation tuple defaults in CLI | Defaults are prohibited at public ingress | `local_runtime_invoker.rs` derives descriptor subject and empty causal context; ingress scope must be narrowed or callers made explicit. |
| RF-9 | P2 | Terminology and schema-source drift | URA-only active vocabulary; one generated proto source path | A retired alternate address term remains in historical/normative text; copied proto inputs must be mechanically derived and checked. |

## 4. Product Extraction

The following mappings are mandatory. A rename alone is insufficient; the
owner, lifecycle, and tests move with the capability.

| Existing feature family | Canonical replacement | Owner after migration |
| --- | --- | --- |
| audio/voice SDK APIs | generic `BidiSession` and typed content/session contracts; product voice call aggregate | Axon generic session; EasyNet-Cli voice provider |
| MCP SDK APIs | external tool-provider binding represented by an AbilityDescriptor | EasyNet-Cli plugin/provider |
| tool_adapter | generic `AbilityImpl` or provider binding | Axon contract; downstream implementation |
| remote_control / ability_dispatch presets | product capability packages | EasyNet-Cli or downstream product |
| product-flavoured federation presets | generic forwarding Invocation or product policy | Axon forwarding primitive; downstream policy |
| Mission proto/state | child Invocation graph plus daemon Mission/EAL state object | EasyNet-Cli daemon |

An Axon SDK package, namespace, or public type must not use EasyNet or
EasyRemote names unless it is an explicitly downstream adapter package. Package
renaming is a compatibility migration: publish the neutral canonical name,
migrate consumers, and then remove the product-named canonical export.

## 5. Proof and Receipt Cutover

`DescriptorBoundInvocationRequest` (or its successor) is the sole public
runtime admission input. It owns canonical bytes, descriptor resolution,
signature verification, nonce/replay checks, authority binding, and the proof
facts used by the receipt.

The following are transitional defects, not alternative APIs:

- plain `canonical_invocation_bytes` signing or verification;
- public plain `verify_signature` and `run_admission`;
- receipt constructors that synthesize empty authority/proof facts; and
- process-local default signer/authenticator generation.

Migration order for this fork:

1. Add descriptor-bound equivalents for every legitimate current caller.
2. Port cross-language conformance vectors and test fixtures.
3. Make the legacy surface non-public and forbid new call sites.
4. Migrate remaining callers, examples, and benchmarks.
5. Delete legacy exports, implementation, old vectors, and compatibility
   constructors in one root-fork completion change.

If a released binding needs an old method temporarily, it is an edge adapter
which constructs a complete descriptor-bound request. It may not call plain
admission, create a signer, or mint receipt facts. Its release-removal version
and a zero-new-caller gate are required before publication.

## 6. Lifecycle Parity

Language facades may differ in syntax, but they may not define independent
runtime state machines. Axon owns a versioned machine-readable capability
matrix and transition vectors. Every facade consumes the same contract.

Required lifecycle actions are `start`, `dispatch`, `stream_open`,
`bidi_open`, `child_dispatch`, `cancel`, `deadline`, `terminal_receipt`, and
`restart_recover`. For each action, the matrix declares:

- allowed source states and exactly one terminal or next state;
- deadline owner and child-deadline propagation rule;
- cancellation authority, acknowledgement, and idempotent replay result;
- bounded queue/concurrency limits and cleanup responsibility; and
- receipt/event observability.

The capability state of each language is exactly one of:

- `Unsupported`: no public contract;
- `Seam`: public type exists but has no provider-backed behavior;
- `ProviderBacked`: operation delegates to the shared provider but does not
  yet meet all transition vectors; or
- `CutoverReady`: all vectors, recovery cases, and public error contracts pass.

No language may be labelled `CutoverReady` from API-shape parity alone.

## 7. Daemon Adapter Boundary

All daemon ability paths, including unary, server stream, bidi, local loopback,
and exact routes, must follow this shape:

```text
product request
  -> daemon policy and complete tuple input
  -> Axon InvocationDraft/descriptor-bound builder
  -> DescriptorBoundInvocationRequest
  -> LocalRuntime
  -> terminal receipt/event
```

The daemon owns classification and policy, not canonical wire assembly. Axon
must provide the builder that encodes the canonical envelope. CLI code supplies
the complete typed fields and no longer constructs `caller`, `callee`,
`subject`, nonce, causal context, or proof bytes as an ad hoc envelope literal.

Direct response synthesis is allowed only for daemon boot, health, status, and
diagnostics. It is forbidden for an ability invocation, stream, bidi session,
admission outcome, or receipt. Route handlers must not bypass `LocalRuntime`.

## 8. Mission/EAL Boundary

Mission/EAL is an implementation strategy for a daemon-owned composite
`AbilityImpl`, not Axon's invocation ontology. A Mission/EAL step that calls
another ability creates a complete child Invocation with an explicit causal
parent and its own receipt.

The Axon clean target contains no `mission.proto`, `MissionState`, mission
runtime state module, MissionControl service, or public Mission SDK facade.
The corresponding state machine, scheduling, retries, timelines, and product
visibility move to EasyNet-Cli. Axon preserves only generic child-invocation,
causal-chain, cancellation, deadline, and receipt primitives.

## 9. URA and Schema Ownership

URA is the only routable identity/address term in active source, schemas,
tests, error messages, and normative specifications. Historical documents may
remain readable only when marked historical and excluded from active terminology
gates. New alternate address vocabulary is prohibited.

Protocol definitions have one editable canonical source. Any checked-in proto
copy is generated from that source through a deterministic script and verified
byte-for-byte in CI. A copied schema must never receive an independent manual
edit.

## 10. Delivery Order

Work proceeds by root fork, lower layers before upper layers:

1. RF-5 and RF-3: signer custody and descriptor-bound proof cutover.
2. RF-8 and RF-7: complete tuple ingress and LocalRuntime-only daemon routes.
3. RF-4: lifecycle contract, vectors, and language-facade parity.
4. RF-6: mandatory receipt proof facts and constructor removal.
5. RF-1: extract product SDK surface to downstream providers/packages.
6. RF-2: move Mission/EAL schema and lifecycle from Axon to EasyNet-Cli.
7. RF-9: URA terminology and generated-schema ownership closure.

Each slice must contain: the owner decision, explicit state machine where
lifecycle exists, caller inventory, migration, deletion list, automated gate,
and measured regression report. Parallel implementation is acceptable only for
independent forks; a change may not stack a second transitional path on an
unfinished fork.

## 11. Acceptance Gates

Completion requires all of the following, not a partial green build:

1. Product-neutral SDK surface scans reject listed product features in Axon
   canonical packages.
2. Axon core scans reject Mission schemas/state/services after the migration.
3. A public-surface manifest has no plain admission or fallback signer export.
4. Receipt constructors reject omitted authority and proof facts in every
   language.
5. All language facades pass identical lifecycle transition and recovery
   vectors against the same provider/runtime version.
6. CLI route inventory proves all ability routes enter `LocalRuntime` through
   descriptor-bound requests, including stream and bidi.
7. The complete invocation tuple is inspectable at every public SDK, FFI,
   daemon, and backend boundary.
8. URA terminology and proto-source derivation gates pass in both repositories.
9. Fixed-baseline benchmark results are published for unary/stream/bidi latency,
   allocation, cancellation cleanup, and bounded-concurrency behavior. No
   percentage performance claim is valid without those numbers.

## 12. Audit Status

No root fork is accepted as closed from a mechanical gate alone. Closure
requires caller migration, deletion of the replaced authority, a negative gate
that rejects reintroduction, and revision-pinned cross-language or
cross-repository evidence. The 2026-07-18 closure report was withdrawn after
its claims were found not to describe the accepted checkout.

| ID | Status | Live audit state |
| --- | --- | --- |
| RF-1 | Revalidation required | Axon product/protocol boundary and public-surface gates now reject product protocol, lifecycle, package, provider, and runtime configuration leakage in canonical SDK roots. Final closure still requires downstream owner evidence for extracted product packages/providers. |
| RF-2 | Revalidation required | Mission schema and runtime removal exists in the current Axon history, but final closure requires the post-RF-1/RF-3 revision and downstream child-invocation evidence. |
| RF-3 | Open | Plain `canonical_invocation_bytes`, `verify_signature`, and `run_admission` exports are rejected by gate, but descriptor-bound raw canonical/sign/verify helpers remain public across languages. These must move behind request/draft builders before RF-3 can close. |
| RF-4 | In progress | The shared lifecycle contract exists, but provider-backed Go/Python stream and bidi behavior and final revision-pinned multi-language evidence are still being converged. |
| RF-5 | Revalidation required | Process-local signer fallback removal exists in the current history; final custody evidence must be regenerated from the accepted source revisions. |
| RF-6 | Revalidation required | Java receipt construction now requires authority binding and complete proof facts, with six-language receipt-surface gates passing. Final closure remains blocked on the RF-3 request-owner cutover and revision-pinned acceptance bundle. |
| RF-7 | Revalidation required | CLI exact unary, stream, bidi, sidecar, and daemon route inventories enter descriptor-bound `LocalRuntime` through registered route adapters. Final live-daemon evidence and RF-8 public-ingress cutover are still pending. |
| RF-8 | Open | EasyNet-Cli loopback helpers, EasyRemote `FreshRoot` public conveniences, and backend root-origin factories still derive nonce, subject, or root causal placement at product ingress. These must be narrowed to explicit tuple origins or named daemon-system issuers. |
| RF-9 | Revalidation evidence refreshed | Axon revision `ef6f119001fd13baa2cec705c6a681c82e13ac89` removes active address vocabulary from canonical protocol inputs, adds a negative product/protocol boundary gate, synchronizes derived proto copies, and passes URA/schema-source gates. This is RF-9 evidence, not whole-SPEC closure. |

`docs/reviews/canonical-runtime-convergence-v2-closure-2026-07-18.md` is a
withdrawn historical closure attempt. It is not acceptance evidence.
