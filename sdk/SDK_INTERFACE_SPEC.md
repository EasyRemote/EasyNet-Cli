# Canonical Runtime SDK Interface

This document is the implementation contract for every language binding in
this repository. It is intentionally product-neutral. EasyNet, EasyRemote and
future applications consume this SDK; their ability names, request DTOs,
directory views, lifecycle workflows and UI projections do not become SDK
types.

The current architecture is indexed by
[`docs/ARCHITECTURE_STATE.md`](../docs/ARCHITECTURE_STATE.md). The normative
requirements are in
[`docs/spec/daemon-sdk-requirements-v1.md`](../docs/spec/daemon-sdk-requirements-v1.md).

## Ownership

| Owner | Canonical responsibility |
| --- | --- |
| Axon | URA grammar and typed builders, descriptor-reference grammar, Invocation canonical bytes, admission, transport call modes, receipt cryptography |
| daemon | process lifecycle, catalog assembly, governed ability execution, routing, persistence and transport providers |
| runtime SDK | product-neutral lifecycle, Addressing, identity/signing, PrincipalLifecycle, Directory resolution, complete Invocation, receipt/causal facts, runtime events, administration, health and typed errors |
| products | concrete workflows and arguments, product directory/read-model views, publication UX, Mission/EAL ergonomics, hosted-agent workflows, pages, OpenAI compatibility, host bindings and UI/application state |

The SDK may project an Axon-owned value, but it must not independently parse,
canonicalize or sign that value. A product may invoke a governed ability
through `RuntimeClient`; it must not add a one-method-per-ability API to the
runtime SDK.

## Object graph

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

`Addressing` is always available from an open native runtime handle. It is not
an Identity or Directory profile and does not depend on a daemon product
ability. A constructed handle is valid or construction fails; methods never
return `nil`/`None` to represent a missing required provider.

## Public runtime concepts

| Concept | Responsibility | Prohibited responsibility |
| --- | --- | --- |
| `Addressing` | Delegate URA, AbilityDescriptorRef and descriptor-bound subject build/parse to Axon | local grammar, string-prefix inference, product labels |
| `RuntimeIdentityClient` | Project runtime identities and owner-bound sign-only capabilities | private-key export, account records, role aliases |
| `ManagedSigningClient` | Create/list/rotate/revoke subject-bound managed keys through key-service | vault access, user enrollment policy |
| `PrincipalClient` | Operate the admitted PrincipalLifecycle and public-key bindings | passwords, OAuth sessions, private-key custody |
| `DirectoryClient` | Resolve and subscribe to canonical URA/ability reachability facts | product dashboard rows, UI aggregation |
| `AbilityDescriptorProjection` | Project the governed descriptor aggregate | deployment configuration, handler binding, product catalog rows |
| `AuthorityClient` | Project or materialize canonical delegation/session-authority metadata | authorization policy, local signing grammar, product account state |
| `RuntimeClient` | Execute a complete Invocation through one runtime provider | product ability methods, caller reconstruction, service location |
| `InvocationBuilder` | Validate and freeze one complete seven-tuple | routing, default caller identity, product DTO validation |
| `PreparedInvocation` | Hold canonical bytes and signing material | execution |
| `SignedInvocation` | Hold submit-ready caller- or daemon-signed material | mutation after signing |
| `InvocationHandle` | Observe, cancel and close a submitted Invocation | product job lifecycle |
| `InvocationResult` | Project terminal output, error and receipt facts | product response normalization |
| `ReceiptClient` | Project verifiable receipt/causal facts and continuation references | product ledger pages, invented receipt identities |
| `RuntimeEventClient` | Consume bounded runtime event streams with typed cursors | product event buses and UI events |
| `RuntimeAdminClient` | Control product-neutral daemon/runtime lifecycle and readiness | hosted-agent product workflow, account administration |
| `StreamHandle` | Ordered server-stream state and bounded buffering | product event buses |
| `BidiSession` | Bidirectional frame lifecycle and terminal state | terminal/browser/media product sessions |
| `HealthClient` | Runtime readiness and diagnostics | product route health |

Generic receipt facts, verification inputs and causal continuation references
are runtime SDK concepts. Product ledger pages, display aggregation and
invented receipt resource names remain downstream product concerns.

Generic Directory reachability is also an SDK concept. A concrete device
dashboard, federated user-device table or product catalog page remains a
product projection over those facts.

## Complete Invocation

Every draft contains exactly one value for each semantic slot:

1. `caller_ura`
2. `callee_ura`
3. `descriptor_ref` (therefore descriptor version)
4. `subject_ura`
5. `nonce_base64`
6. `causal_context`
7. exactly one of `args` or `arguments_base64`

Metadata, timeout and content-envelope facts may accompany the tuple but may
not replace any slot. The tuple is immutable after `prepare`. Runtime dispatch,
local execution, stream/bidi opening and cross-hub relay carry the same
Invocation; no adapter is allowed to reconstruct a business call with a system
caller, a new nonce or an empty causal context.

## Transport modes and transitions

Axon's unary, server-stream and bidi modes are the only transport taxonomy.
Language bindings may expose idiomatic names but must map one-to-one to that
taxonomy. State transition/receipt semantics are descriptor facts and are not a
fourth RPC mode.

## Lifecycle state machines

All stateful public objects have explicit monotonic state:

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

Invalid transitions return typed errors. Missing providers, corrupt discovery,
catalog load failures and runtime-not-ready states fail closed. A compatibility
success, empty registry, restart suggestion or implicit local fallback is not a
state transition.

## Provider rules

- Every capability has one explicit provider seam.
- Provider selection happens at construction; operations do not search for a
  transport or fall back to a second implementation.
- A provider may be `unsupported`, `seam`, `provider-backed` or
  `cutover-ready` as defined in `sdk/conformance/sdk-parity-matrix.json`.
- Go and Python describe the same capability set and state. Evidence is
  language-specific; architecture is not.
- Direct providers may use generated Axon types internally. Public SDK objects
  never expose them.
- Native providers use generic C ABI v5 only.

## C ABI v5

The stable C boundary contains only:

- ABI/version and typed-error discovery;
- daemon/runtime lifecycle;
- generic Invocation prepare, invoke, submit and handle operations;
- stream and bidi operations;
- health/diagnostics and canonical Addressing operations required by bindings;
- opaque memory/handle management.

There are no one-method-per-ability or product-domain C symbols. Identity,
PrincipalLifecycle, Directory, receipt, events and runtime administration use
generic handle/Invocation operations at the ABI boundary; language clients may
provide typed provider-backed projections. Mission, Publication, Surface,
OpenAI, HostBinding, Wrappers and other product helpers live downstream.

## Forbidden SDK surfaces

The following are architecture violations in any language binding:

- product profile bundles or service locators;
- `MissionClient`, product account/admin clients, `PublicationClient`,
  `SurfaceClient`, `CompatibilityClient`, `HostBindingClient`, product event
  clients, convenience wrapper clients or equivalents;
- product Directory rows or product event/receipt presentations inside the
  generic `DirectoryClient`, `RuntimeEventClient` or `ReceiptClient`;
- product ability literals such as `mission.run`, `agent.start`,
  `openai.chat_completions` or `pages.publish`;
- product-specific directory views, receipt-history pages, account, pairing,
  page, model, file-transfer, terminal or desktop-companion DTOs;
- a second PrincipalLifecycle, trust store, key inventory or recovery truth;
- a second URA/descriptor parser, Invocation envelope or call-mode enum;
- raw C ABI product symbols or a compatibility alias for removed symbols;
- optional-provider return values for capabilities declared always available;
- legacy identity spelling for a URA semantic value.

## Language parity

Names may be idiomatic, but the object graph, lifecycle, error classes,
capability state and semantic validation must match. Node, Java and Swift may
ship a subset; they must mark missing concepts unsupported instead of publishing
product seams or placeholder clients.

## Stability gates

A binding is stable only when:

- its public exports contain only the runtime concepts above;
- the complete Invocation and lifecycle conformance cases pass;
- all URA/descriptor operations delegate to Axon;
- its capability states match the Go/Python matrix;
- no product workflow module, product ability literal, domain C symbol,
  parallel principal/key model, legacy parser or fallback provider is
  reachable;
- downstream product tests prove their local typed facades lower to generic
  Invocation without owning canonical protocol logic.
