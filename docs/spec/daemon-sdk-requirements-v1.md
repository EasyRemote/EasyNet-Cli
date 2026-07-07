# Daemon SDK Requirements v1

Status: active design spec.

Audience: EasyNet-Cli, EasyNet backend, EasyRemote, and language binding
maintainers.

Purpose: define the public SDK surface owned by EasyNet-Cli for controlling and
calling `easynet-daemon`. This is a Daemon SDK, not a command-line SDK. The
`easynet` executable is one consumer of this SDK; it is not the SDK boundary.

Normative language: `MUST`, `MUST NOT`, `SHOULD`, and `MAY` are intentional.
Any implementation that violates a `MUST` is not compatible with this spec.

## 1. Review Corrections Integrated

This revision repairs the root causes that made prior SDK planning
non-convergent:

1. **Duplicate architecture source of truth**: language support, OOP model,
   state machines, and project structure were appended after an already complete
   API spec. They are now first-class sections in one normative document.
2. **Implementation/facade confusion**: Go, Python, Node, Java, and Swift are
   language facades. The native Rust daemon SDK core plus C ABI projection are
   the semantic implementation. Facades may own ergonomics, not daemon or Axon
   semantics.
3. **Weak object model**: lifecycle, runtime client, invocation construction,
   prepared signing material, streams, bidi sessions, directory, identity,
   receipts, publication, host binding, mission, admin, events, surface,
   compatibility, and health are now explicit SDK objects with ownership and
   terminal operations.
4. **Underspecified state machines**: daemon lifecycle, client connection,
   invocation, stream, bidi, directory subscription, and explicit aggregate
   fan-out now have terminal-state rules.
5. **Unbounded listing risk**: ordinary device/agent/ability listing is a
   read-model facade. Per-agent/per-ability fan-out is forbidden in ordinary
   list methods and allowed only through named aggregate abilities with bounds.
6. **Project structure drift**: required roots, ownership per directory, and
   structural migration rules now define the target shape without relying on
   examples or gallery code as hidden behavior.
7. **Interface coverage ambiguity**: target capability matrices are now
   separated from the current repository snapshot. ABI v3 is identified as a
   complete Invocation-only dispatch ABI, not a complete multi-language SDK ABI.
8. **EasyRemote extraction gap**: SDK completeness is now judged by whether
   EasyRemote can delete its low-level ABI loader, transport session, Invocation
   codec, receipt projection, daemon lifecycle wrapper, host-stream wire/hash
   helpers, gateway/agent admin transport, and daemon system ability facades.
   The SDK is split into Core, Directory/Identity, Publication, Host Binding,
   Mission, Admin + Gateway, Events, Surface, Compatibility, and Convenience
   Wrapper profiles so EasyRemote keeps only Python product ergonomics and Hub
   routes keep only backend product responsibilities.
9. **Hub coverage gap**: EasyNet backend is now treated as the Hub acceptance
   test. Backend-cutover-ready requires route-family coverage for health,
   identity, pairing/admin, catalog, sessions, invoke/stream/bidi, events,
   file/context upload, terminal/remote desktop/browser/media, pages/surfaces,
   skills, OpenAI compatibility, receipts/metrics, and federation/peer hubs.

## 2. Decision

EasyNet product clients MUST depend on EasyNet-Cli SDK surfaces, not directly on
EasyNet-Axon SDKs or raw `axon.v1` generated protocol types.

The target dependency graph is:

```text
EasyNet backend / EasyRemote / GUI / host app / easynet CLI
  -> EasyNet-Cli Daemon SDK
  -> local easynet-daemon
  -> daemon-owned Axon adapter
  -> Axon SDK, proto, LocalRuntime, admission, receipts
```

The forbidden product graph is:

```text
EasyNet backend / EasyRemote
  -> EasyNet-Axon SDK or generated axon.v1 proto
  -> raw axon-runtime
```

This does not mean the SDK hides the words Invocation, URA, AbilityDescriptor,
or Receipt. Those are public EasyNet/Axon semantic terms. It means consumers do
not import Axon packages, do not construct Axon proto messages, and do not start
or dial raw `axon-runtime` for product paths.

Product deployments use `easynet-daemon`. Raw `axon-runtime` remains valid for
Axon protocol reference runtimes, Axon SDK tests, protocol examples, and
third-party minimal runtimes, but not for EasyNet product paths.

## 3. Ownership

| Layer | Owns | Must not own |
| --- | --- | --- |
| Axon | Invocation wire shape, canonical bytes, URA profile, admission, receipt verification, stream/bidi protocol invariants | EasyNet daemon lifecycle, plugins, keyring forwarding, EAL/Mission product policy, backend DB/product UX |
| EasyNet-Cli daemon | Device/Hub process lifecycle, local sockets, identity, keyring, plugin/MCP/EAL/Mission execution, local and remote dispatch, session state, daemon system abilities | Browser auth, product database, OAuth/JWT, frontend DTOs |
| EasyNet-Cli SDK core | Daemon lifecycle/client object model, language-neutral DTOs, C ABI handle model, daemon transport wrappers, typed errors, conformance fixtures | New Axon protocol semantics, backend product state, EasyRemote decorators |
| Language SDK facades | Idiomatic builders, async adapters, packaging, generated DTO bindings, retry/error mapping | Canonical Invocation algorithms, receipt verification algorithms, daemon policy, Axon admission semantics |
| EasyNet backend | Browser/product HTTP API, user/device dashboards, DB projections, Join Token UX | Ability execution, daemon session routing, Axon protocol rules |
| EasyRemote | Python developer facade for turning functions/classes/pipelines into abilities | Low-level daemon transport, canonical signing, receipt verification, daemon process ABI |

## 4. Consumers and Release Tiers

The SDK MUST support these first-class consumers:

1. EasyNet backend, primarily through a Go SDK.
2. EasyRemote, through the Python Daemon SDK. That Python SDK MAY use the C ABI
   internally; EasyRemote production code MUST NOT bind `libeasynet_cli`
   directly.
3. The `easynet` command-line executable.
4. Desktop or GUI products that need daemon lifecycle and invocation.
5. Third-party host apps that expose local resources as EasyNet abilities.

The EasyNet backend MUST NOT depend on EasyRemote. EasyRemote is a product
facade above the daemon SDK, not a runtime substrate for the backend.

| Tier | Language target | Primary consumer | Required at | Notes |
| --- | --- | --- | --- | --- |
| P0 | Rust | native SDK core, daemon internals, native tests, FFI implementation | SDK core freeze | Source-of-truth implementation and conformance runner host. |
| P0 | C ABI | Python/Node/Swift/JVM bridge option, native embedding | SDK core freeze | Stable ABI surface in `include/easynet_cli.h`; no Axon symbols exposed. |
| P0 | Go | EasyNet backend / Hub | backend cutover | Preferred backend facade; uses daemon local transport and/or C ABI helpers without importing Axon or owning Axon semantics. |
| P0 | Python | EasyRemote and local automation | EasyRemote cutover | Must support Pythonic OOP and async-friendly wrappers without making EasyRemote depend on backend. |
| P1 | Node / TypeScript | desktop tools, extension hosts, local dev tooling | post-backend cutover | Must provide typed declarations and promise/async iterator APIs. |
| P1 | Java / Kotlin JVM | enterprise and Android-adjacent integrations | post-backend cutover | Java API is normative; Kotlin gets idiomatic helpers over it. |
| P1 | Swift | macOS/iOS-adjacent clients | post-backend cutover | Swift Package Manager distribution; async/await first. |
| P2 | React/browser facade | frontend product SDK only | optional | Not a daemon SDK. Browser code cannot call local daemon UDS; any React facade talks to EasyNet backend. |

Rust, C ABI, Go, and Python are the first cutover languages because they cover
daemon implementation, backend, and EasyRemote. Node, Java, and Swift must be
designed at the same semantic level even if they ship later; the P0 design MUST
NOT paint them into a corner.

## 5. Public SDK Surfaces and Single Implementation Rule

The SDK family SHOULD have these public surfaces:

| Surface | Role | Stability target |
| --- | --- | --- |
| Native Rust SDK, `easynet_cli::{daemon,runtime,invocation}` | The single semantic implementation owned by EasyNet-Cli: daemon lifecycle, daemon client object model, EasyNet policy wrappers, and Axon delegation boundary | Semver crate API |
| C ABI, `libeasynet_cli` | Stable ABI projection of the native Rust SDK for language bindings | ABI versioned, header-checked |
| Go SDK, `easynet.run/cli/sdk/go/...` | Primary EasyNet backend facade over daemon SDK semantics | Semver module API |
| Python SDK, package name TBD | Primary EasyRemote low-level facade over daemon SDK semantics | Semver package API |
| Future bindings | Node, Swift, Java, etc. | Generated or hand-written facade over the same schema and Rust/C ABI core |

All surfaces MUST share the same logical data model and JSON schemas for
Invocation, authority metadata, daemon status, receipt summaries, stream events,
directory events, and typed errors.

There is one daemon SDK semantic implementation, not one implementation per
language.

1. The native Rust SDK is the source-of-truth implementation for EasyNet-Cli
   daemon lifecycle, local socket discovery, daemon readiness, EasyNet policy
   wrappers, language-neutral DTOs, and FFI handle ownership.
2. `libeasynet_cli` projects that Rust implementation as a stable C ABI.
3. Go, Python, Node/TypeScript, Java, Swift, and future SDKs are facades or
   wrappers. They may own idiomatic builders, async adapters, packaging,
   generated type bindings, retry classification, and error mapping, but they
   MUST NOT fork daemon SDK semantics.
4. If a non-Rust SDK talks directly to the daemon local transport for deployment
   reasons, that transport code is still a facade. It may serialize requests,
   manage connection lifecycle, and map errors, but it MUST NOT reimplement
   canonical Invocation signing material, receipt verification, Axon admission
   semantics, or stream/bidi terminal-state rules.
5. Semantics already implemented by EasyNet-Axon MUST be delegated to Axon from
   the Rust daemon SDK core or daemon runtime adapter. EasyNet-Cli MUST NOT
   create a second Axon SDK inside CLI language bindings.
6. The daemon SDK is a facade/projection engineering layer over the current
   EasyNet-Cli daemon and EasyNet-Axon implementation. When Axon already
   exposes a parser, builder, canonicalizer, verifier, state machine, or fixture
   for URA, DescriptorRef, Invocation, receipt, stream, bidi, or federation
   semantics, SDK work MUST call, wrap, generate from, or validate against that
   implementation. It MUST NOT introduce a parallel grammar, hash input,
   canonical JSON order, receipt URA shape, terminal-state taxonomy, or helper
   fixture because doing so would become a second protocol source.

### 5.1 SDK Profiles

The SDK is one semantic implementation with multiple stability profiles. A
language package may ship only the profiles it has implemented and passed
conformance for, but it MUST NOT call itself a complete Daemon SDK until the P0
profiles below are present.

| Profile | Required for | Owns | Must not own |
| --- | --- | --- | --- |
| Runtime Core | all product callers | ABI/version/feature discovery, daemon lifecycle, runtime health, complete Invocation builder, prepare/sign/submit, unary, stream, bidi, invocation handle/events, typed errors | product decorators, Python host process, backend HTTP/DB state, protocol algorithms not delegated to Axon |
| Receipt | backend, EasyRemote, audit/provenance tools | receipt fetch/projection/verification entry points, receipt summary DTOs, parent receipt to causal ref, opaque receipt URA projection while RFC-007 is unresolved | summary-only verification claims, fabricated receipt URAs, backend-local receipt parsing |
| Directory + Identity | backend and EasyRemote | URA and DescriptorRef builders/parsers exposed through SDK DTOs by delegating to Axon helpers, local identity, signer/key helpers, paginated directory read model, resolve, subscribe | backend JWT/OAuth, browser sessions, facade-side live fan-out, hand-built URA strings, hand-built `@` descriptor refs |
| Publication | EasyRemote, CLI, local host apps | ability package/resource references, AbilityDescriptor/AbilityImpl publication DTOs, deploy/install/list/show/enable/disable/unpublish, host-stream binding contract, plugin/skill install as implementation-resource management | Python function introspection, decorator semantics, host process runtime, one-method-per-ability protocol |
| Host Binding | EasyRemote and local host apps | host-stream binding DTOs, request envelope schema, item/error/terminal frame codec, output-hash helper, host readiness and cleanup contract | Python function execution, decorator semantics, thread model, product host process lifecycle |
| Mission | EasyRemote Pipeline, CLI, automation tools | `mission.run`, `mission.track`, `mission.cancel`, EAL source/file submission DTOs, mission status/error projection | Python Pipeline DSL, planner/scheduler/retry policy, redefining Invocation or child receipt semantics |
| Admin + Gateway | EasyRemote Server, CLI, onboarding/admin tools | daemon mode config/status DTOs, hub/device admin, agent lifecycle, gateway/public-listener readiness, join/leave/pairing/trust helpers when daemon exposes them | certificate authority policy, backend auth/session UX, product onboarding copy |
| Events | EasyNet backend/Hub, GUI monitors, CLI monitors | directory/device/session/invocation event subscription DTOs, cursors, reconnect hints, bounded drop reporting, SSE-friendly projections | backend subscriber registry, browser auth, frontend notification UX, product-local daemon stream loops |
| Surface | EasyNet backend/Hub public pages and embedded surfaces | page/surface manifest DTOs, page create/list/delete carriers, public page refs, surface health/status over governed daemon abilities | HTML rendering, frontend routing, CDN policy, browser sessions, product content management UX |
| Compatibility | EasyNet backend/Hub compatibility endpoints | OpenAI-compatible model/chat/files adapters and typed request/result DTOs over governed abilities | product API keys, billing/rate-limit policy, browser HTTP routes, pretending OpenAI schemas are daemon protocol |
| Convenience Wrappers | optional P1/P2 | file transfer, terminal, remote desktop, voice/media helpers over Invocation or bidi | becoming the only access path for the underlying governed abilities |

The split is a dependency rule:

```text
EasyRemote
  -> Python Daemon SDK Runtime Core
  -> Python Daemon SDK Directory + Identity
  -> Python Daemon SDK Receipt
  -> Python Daemon SDK Publication
  -> Python Daemon SDK Host Binding
  -> Python Daemon SDK Mission
  -> Python Daemon SDK Admin + Gateway, when `Server`, gateway, or agent admin
     facades are shipped
```

EasyRemote may add product ergonomics above those profiles. It MUST NOT be
forced to own the profile implementation itself. If EasyRemote still needs to
maintain a ctypes ABI loader, raw handle session, Invocation JSON codec,
receipt summary model, URA string builders, host-stream frame/hash codec,
gateway/agent admin system-ability transport, or daemon system ability transport
facade, the Python Daemon SDK is not complete for EasyRemote cutover.

The EasyNet backend/Hub split is the same kind of dependency rule:

```text
EasyNet backend / Hub
  -> Go Daemon SDK Runtime Core
  -> Go Daemon SDK Directory + Identity
  -> Go Daemon SDK Receipt
  -> Go Daemon SDK Events
  -> Go Daemon SDK Admin + Gateway
  -> Go Daemon SDK Surface
  -> Go Daemon SDK Compatibility
  -> Go Daemon SDK Convenience Wrappers for file, terminal, remote desktop,
     browser/session, voice/media, and similar governed abilities
  -> Go Daemon SDK Publication only for implementation-resource management
     routes such as plugin/skill install/list/show/upgrade/remove
```

The backend may keep browser/product HTTP routes, JWT/OAuth, account/device
database projections, rate limits, page rendering, and UI DTOs. It MUST NOT keep
raw daemon gRPC clients, generated `axon.v1` protocol types, hand-written
stream/bidi state machines, directory subscription loops, OpenAI compatibility
ability shims, page/surface ability shims, terminal/file/desktop ability shims,
or pairing/session daemon carriers once the corresponding Go SDK profiles are
cutover-ready.

### 5.2 MEMC Design Rule

This spec uses MEMC as the API review lens:

```text
Minimal core.
Exclusive ownership.
Maximal functional coverage through profiles.
Consistent semantics across languages and consumers.
```

MEMC is stricter than "the methods exist". A surface passes MEMC only when it is
small at the stable core, has one owner, covers the product workflows without
forcing facades to rebuild daemon substrate, and uses the same terms across
Axon, daemon, SDK, backend, and EasyRemote.

| MEMC axis | Required property | Rejection signal |
| --- | --- | --- |
| Minimal core | Runtime Core contains only lifecycle, Invocation, stream/bidi, health/error, receipts, and process-safe client behavior | Core grows one-method-per-ability APIs, product decorators, backend DTOs, Python host runtime, or UI concerns |
| Exclusive ownership | Each operation belongs to exactly one profile or one higher product facade | Same ability list/deploy/mission/status logic appears in Runtime Core, Publication, EasyRemote, backend, and CLI separately |
| Maximal coverage | Profiles cover backend cutover, EasyRemote cutover, local host apps, CLI, desktop, and future bindings | A product facade still needs raw daemon transport, Invocation JSON, URA strings, receipt placeholders, host-stream codecs, admin/gateway carriers, event loops, surface/page carriers, compatibility shims, wrappers, or daemon system ability carriers |
| Consistent semantics | Every language exposes the same DTOs, terminal states, error codes, receipt refs, and capability flags | Go/Python/C ABI disagree on signing material, terminal state, authority metadata, stream terminal event, or descriptor binding |

### 5.3 Profile Ownership Matrix

Every public method MUST be placed by this matrix before it is accepted into the
SDK. If a method appears plausible in more than one row, the design is not yet
clean enough.

| Operation class | Sole SDK/profile owner | Product facade role | Forbidden duplicate |
| --- | --- | --- | --- |
| ABI/library loading, feature/version discovery, raw handle lifetime | Runtime Core | none | EasyRemote `ctypes`, backend cgo/dlopen, CLI-only runtime wrappers |
| Daemon lifecycle and runtime readiness | Runtime Core | product aliases may re-export | backend subprocess calls, EasyRemote daemon wrapper with raw handles |
| Invocation tuple construction, prepare, signing material, signed submit | Runtime Core delegating Axon canonical logic | ergonomic builders over SDK DTOs | product-local canonical JSON, hidden subject/nonce/causal defaults |
| Unary, stream, bidi, cancellation, terminal events | Runtime Core | typed result-first wrappers | control.sock product calls, facade-owned stream state machines |
| Receipt summary, full receipt fetch, causal refs, verification entry points | Receipt delegating Axon verification | display, user-facing provenance UX | summary-only verification claims, fabricated receipt URAs |
| Directory resolve/list/subscribe pages | Directory + Identity | filtering/presentation only | facade-side live fan-out, backend DB as canonical directory |
| URA builders, local identity, signer/key helpers | Directory + Identity | product owner handles and stubs | hand-built URA strings, backend JWT/OAuth leakage |
| DescriptorRef construction and ability URA extraction | Directory + Identity delegating Axon `canonical_ability_descriptor_ref` and `ability_ura_from_descriptor_ref` | ergonomic typed wrappers only | facade string concatenation with `@`, nested ability paths, or alternate descriptor-ref grammar |
| Ability package/resource refs, deploy/list/show/unpublish, host-stream binding DTOs | Publication | Python package generation, decorators, host runtime | product-owned daemon ability deploy/list carriers |
| Host-stream request envelopes, stream item/error/terminal codecs, output hash | Host Binding | language runtime invokes user code and maps arguments | facade-owned canonical JSON/hash rules or daemon terminal semantics |
| Mission run/track/cancel/events | Mission | Pipeline DSL and EAL generation | product-owned mission transport or child Invocation semantics |
| File, terminal, remote desktop, media helpers | Convenience Wrappers | UX-specific wrappers | treating helper as the only callable path or bypassing Invocation |
| Hub join/leave, device admin, pairing/trust, device session lifecycle, agent lifecycle, gateway status | Admin + Gateway | onboarding UX, TLS certificate provisioning, token copy, account-device binding | hidden process/runtime bootstrap in normal `call()` |
| Directory/device/session/invocation events | Events | HTTP/SSE/WebSocket fanout, browser authorization, product notification filtering | backend-owned daemon subscribe loops, raw stream terminal handling, polling as the only live path |
| Page/surface manifest and public page references | Surface | HTML rendering, browser route mounting, auth gating, CDN/cache policy | backend-owned page daemon system-ability transport or static-only product fork |
| OpenAI-compatible chat/models/files routing | Compatibility | HTTP auth, product API-key policy, quota/rate limits, response presentation | backend-owned raw ability+args adapters or compatibility schema treated as canonical daemon protocol |

### 5.4 Consumer Coverage Matrix

The SDK is incomplete until each first-class consumer can be expressed as a
composition of profiles, not by importing lower layers directly.

| Consumer | Required profiles | Must not import/call | Completion evidence |
| --- | --- | --- | --- |
| EasyNet backend / Hub | Runtime Core, Directory + Identity, Receipt, Events, Admin + Gateway, Surface, Compatibility, selected Publication, Convenience Wrappers | Axon SDK/proto, C ABI, direct daemon sockets/gRPC, EasyRemote, CLI subprocesses, backend-local daemon stream/bidi loops | Backend SDK-only import-ban plus hub route-family coverage, health/invoke/list/events/session/page/OpenAI/wrapper smokes pass |
| EasyRemote | Runtime Core, Directory + Identity, Publication, Host Binding, Mission, Admin + Gateway for `Server` and agent admin | `ctypes`, `libeasynet_cli`, raw handles, Invocation JSON codec, receipt placeholders, host-stream wire/hash rules, generic daemon system ability carriers | EasyRemote extraction tests prove deleted or shimmed low-level modules |
| `easynet` CLI | Runtime Core, Directory + Identity, Publication, Host Binding when hosting, Mission, Admin + Gateway, wrappers as needed | duplicate command-only daemon clients | CLI commands call SDK clients or daemon abilities through SDK DTOs |
| Desktop / GUI host apps | Runtime Core, Directory + Identity, optional wrappers | Axon proto/gRPC, control-frame product calls | GUI can start/attach, invoke, stream/bidi, list directory, show typed health |
| Third-party local host apps | Runtime Core, Directory + Identity, Publication, Host Binding | plugin-private protocol forks, one-method-per-ability ABI, host-stream codec forks | Host app can publish AbilityImpl and invoke through generic SDK paths |
| Future Node/JVM/Swift bindings | Same profile set as their release tier | reimplemented canonical bytes, receipt verification, daemon policy | Shared conformance fixtures pass for shipped profiles |

### 5.5 Semantic Alignment Matrix

Use these terms consistently. A facade may rename for ergonomics only if its
docs map back to the SDK term and the underlying DTO remains unchanged.

| Product-facing term | SDK term | Daemon/Axon semantic owner | Notes |
| --- | --- | --- | --- |
| local function | implementation resource | EasyRemote product facade | Not network-addressable by itself. |
| capability | `AbilityDescriptor` plus one or more `AbilityImpl` bindings | daemon control plane, Axon descriptor semantics | Callable/discoverable/composable only after publication. |
| registered function | `AbilityDeployRequest` producing `AbilityDeployResult` | Publication profile | Python introspection remains in EasyRemote. |
| ComputeNode | device daemon plus host-stream AbilityImpls | EasyRemote + daemon | Device identity comes from daemon/keyring. |
| warm host socket | `HostStreamBinding` | Host Binding profile + daemon executor | User-code execution remains in EasyRemote. |
| host stream frame | `HostStreamFrame` / `HostStreamTerminal` | Host Binding profile + daemon executor | Item, error, terminal, and output hash semantics are shared fixtures. |
| owner handle | owner URA plus ergonomic call target | Directory + Identity | Must use SDK URA builders and directory refs. |
| call / execute | Invocation over Runtime Core | Axon Invocation semantics via daemon | Result-first wrapper must preserve receipt. |
| prepared call | `PreparedInvocation` plus `SigningMaterial` | Runtime Core delegating Axon canonical bytes | Must expose seven-tuple before submit. |
| stream | `StreamHandle` events and terminal receipt | Runtime Core / Axon stream invariants | EOF, terminal receipt, timeout, cancel are distinct. |
| session | `BidiSession` | Runtime Core / Axon bidi invariants | Frame0 generated from complete Invocation. |
| live update / SSE event | `EventStream<T>` plus `EventCursor` | Events profile + daemon directory/event stream | Backend SSE is a product projection, not the daemon event protocol. |
| device session | `DeviceSession` | Admin + Gateway plus Runtime Core stream/bidi | Browser session ids are product state and do not replace daemon session refs. |
| page / surface | `PageRecord`, `SurfaceManifest`, `PublicPageRef` | Surface profile + daemon governed abilities | HTTP URLs are product routes, not daemon identity. |
| OpenAI-compatible route | `Compatibility*` DTOs lowering to Invocation/File/Directory DTOs | Compatibility profile | External compatibility schema is an adapter, not the daemon protocol. |
| Pipeline | EAL/Mission source | EasyRemote DSL + Mission profile | Each mission step creates child Invocations. |
| Context.call | child Invocation with parent causal ref | Runtime Core receipt refs + EasyRemote context UX | Disabled unless causal ref is verifiable. |
| receipt | `Receipt` / `ReceiptSummary` / opaque receipt URA until RFC-007 lands | Axon receipt semantics projected by SDK | Summary continuity is not cryptographic verification; facades do not construct receipt URAs. |

### 5.6 Source Alignment Ledger

Every SDK interface family MUST trace back to the normative semantic owner
below. This ledger is the semantic-alignment guard for future edits: if a new
SDK method cannot point to one of these owners, it is either product facade
ergonomics, an implementation detail, or an underspecified design.

| Concept | Normative source | SDK implication | Nonconformance signal |
| --- | --- | --- | --- |
| Invocation seven-tuple | Axon Invocation Axiom | Runtime Core exposes caller, callee, ability/descriptor, subject, nonce, causal context, and args before prepare/submit. | Hidden subject, silent empty causal context, facade-generated canonical JSON, or one-method-per-ability dispatch. |
| URA identity/addressing | Axon `core/ura-rs` grammar and EasyNet ontology | Directory + Identity exposes URA builders/parsers and typed refs by delegating to Axon helpers; product facades do not hand-build URA strings. | Backend JWT/OAuth, socket paths, node ids, package names, or examples that fail `parse_ura` leak into public Invocation identity fields. |
| DescriptorRef canonicalization | Axon `canonical_ability_descriptor_ref` and `ability_ura_from_descriptor_ref` helpers | SDK facades obtain canonical descriptor refs and ability URAs from Axon/daemon helpers; language bindings do not define a second `ability_ura@version` grammar. | Facades concatenate strings with `@`, accept nested `/device/.../ability/...` shapes, or disagree on malformed descriptor refs. |
| AbilityDescriptor / AbilityImpl / AuthorityBinding | EasyNet ontology | Publication separates governed descriptor, executable binding, resource refs, authority, and enablement state. | "Capability" is treated as a local function, plugin, skill, or deploy path without descriptor/version/authority binding. |
| Receipt and causal continuity | Axon receipt semantics and RFC-006 state-object rules | Receipt APIs distinguish summary projection, full fetch, causal refs, and cryptographic verification. | Summary-only continuity is advertised as verification, or `Context.call` fabricates parent receipt refs. |
| Stream and bidi terminal semantics | Axon stream/bidi obligations | Runtime Core owns terminal events, close/cancel distinction, backpressure, and receipt projection. | Product facade treats EOF, cancel, timeout, transport failure, and terminal receipt as the same state. |
| Mission/EAL composition | EasyNet ontology | Mission profile submits/observes daemon-owned EAL; each child ability call remains an Invocation. | Pipeline DSL owns daemon transport, child receipt semantics, or a second Invocation construction path. |
| Host-stream execution binding | EasyNet-Cli daemon execution policy | Host Binding owns envelope/frame/hash/terminal codecs while EasyRemote owns Python user-code execution. | Warm host code defines canonical frame JSON, output hash, or terminal mapping independently. |
| Gateway/admin lifecycle | EasyNet-Cli daemon policy | Admin + Gateway owns daemon mode/status DTOs, agent lifecycle carriers, and gateway readiness. | Product facade shells out, calls system abilities directly, or hides daemon bootstrap inside normal `call()`. |
| Hub event propagation | EasyNet-Cli daemon directory/event stream plus backend product fanout boundary | Events profile owns subscription DTOs, cursor semantics, reconnect hints, terminal states, and drop reporting; backend owns HTTP/SSE/WebSocket authorization and fanout. | Backend imports daemon stream clients, decodes raw daemon event frames, or treats polling as sufficient live event coverage. |
| Hub page/surface delivery | EasyNet-Cli daemon surface/page abilities plus backend rendering boundary | Surface profile owns page/surface manifest carriers and daemon dispatch; backend owns public HTTP rendering, auth gates, and cache policy. | Backend invokes page system abilities manually or stores page runtime state as a daemon substitute. |
| Hub compatibility APIs | EasyNet-Cli governed abilities plus backend product HTTP compatibility layer | Compatibility profile owns typed adapters from OpenAI-style DTOs to Invocation/File/Directory SDK DTOs; backend owns API keys, quotas, and HTTP response shaping. | Backend maintains raw ability+args compatibility shims or exposes OpenAI DTOs as daemon protocol. |

### 5.7 MEMC Completion Gates

Completeness is a claim with levels. A package MUST use the narrowest truthful
claim until the evidence below exists in the current tree and CI.

| Claim | Required evidence | Explicitly not enough |
| --- | --- | --- |
| Runtime Core available | Lifecycle, health, complete Invocation, prepare/sign/submit, unary/stream/bidi, invocation handle terminal observation, typed errors, and Runtime Core conformance pass for that language. | Daemon start/stop plus unary invoke. |
| Profile available | The profile's public methods, schemas, fixtures, and conformance cases pass for that language. | A product facade can call one daemon system ability manually. |
| Language stable | Runtime Core plus every declared shipped profile passes shared conformance; public API exposes no Axon/proto/raw daemon internals. | Placeholder package, README, or one happy-path smoke. |
| Complete Daemon SDK | Runtime Core, Health, Errors, schemas, conformance, lifecycle rules, and every profile required by each declared first-class consumer are complete in the P0 language(s) for that consumer: Go for EasyNet Hub/backend, Python for EasyRemote, Rust/C ABI for core projection. | ABI v3 Invocation dispatch alone, one language's implementation, or forcing every language to ship profiles its declared consumer does not use. |
| EasyRemote-cutover-ready | EasyRemote can remove or shim raw FFI, raw sessions, Invocation JSON codec, receipt placeholders, URA string builders, host-stream codecs, publication/mission/admin transports, and daemon lifecycle wrappers. | EasyRemote can still run through its own `_transport` layer. |
| Backend-cutover-ready | Backend imports only the public Go SDK runtime boundary, passes SDK-only import bans, and every Hub route family maps to Runtime, Directory, Receipt, Events, Admin, Surface, Compatibility, Publication, or wrapper clients. | Backend can reach the daemon through Axon, C ABI, direct sockets, control frames, subprocesses, EasyRemote, raw daemon event loops, raw page/surface ability shims, raw OpenAI ability shims, or hand-written stream/bidi adapters. |

MEMC review rejects any edit that expands Runtime Core with product-specific
helpers when a profile or facade owns the behavior, or that leaves a first-class
consumer dependent on a lower layer after the supposed SDK surface exists.

## 6. Non-goals

The Daemon SDK MUST NOT expose:

1. `StartAxonRuntime`, `start_runtime`, or any raw `axon-runtime` lifecycle.
2. Raw `pb.InvokeRequest`, `pb.InvokeResponse`, `pb.InvokeBidiUp`, or other
   generated `axon.v1` types at product-facing boundaries.
3. Direct imports of `easynet.run/axon/*` in product SDK APIs.
4. `InvokeAbility(name, args)` or any public ability+args-only call surface.
5. A shell transport such as `RunCLICommand("easynet ...")`.
6. Product invocation over `control.sock` JSON frames.
7. Backend auth/JWT/OAuth/database helpers.
8. Frontend DTOs or HTTP route helpers.
9. One method per system ability as the only calling model.
10. SDK-side per-target fan-out for governed ability calls.

Convenience helpers MAY exist, but every helper MUST eventually produce an
inspectable complete Invocation before dispatch.

## 7. Core Invariants

### 7.1 Complete Invocation

Every dispatch API MUST preserve the complete Invocation tuple:

```text
caller, callee, ability, subject, nonce, causal_context, args -> receipt
```

`ability` is represented by a descriptor-bound reference:

```text
DescriptorRef = ability_ura@descriptor_version
```

`DescriptorRef` is not a string-concatenation rule owned by this SDK. SDK
facades MUST obtain or validate it through Axon/daemon helpers equivalent to
`canonical_ability_descriptor_ref`, and MUST derive the ability URA through
`ability_ura_from_descriptor_ref` rather than splitting by hand. The SDK may
expose typed wrappers, but the grammar and parse errors remain Axon-owned.

SDK builders MAY fill defaults such as a fresh nonce or root causal context, but
the filled values MUST be visible before dispatch. Hidden `subject`, hidden
`nonce`, or silently empty causal context at a public boundary is a correctness
bug.

### 7.2 Identity Split

The SDK MUST keep these separate:

| Term | Meaning |
| --- | --- |
| URA | Routable logical identity/address for agents, devices, abilities, resources, sessions, receipts |
| Endpoint | Transport locator such as UDS path or TCP endpoint |
| Process | Local OS process, e.g. `easynet-daemon` |
| AbilityDescriptor | Versioned governed interface with schema, policy, observability, and receipt semantics |
| AbilityImpl | Versioned executable binding that can satisfy one descriptor version |
| Plugin | Implementation resource that may be used by an AbilityImpl |

`callee` is the logical execution identity that advertises or is resolved as the
owner of the selected `AbilityDescriptor`. The stable target is an Agent URA:
`DeviceAgent advertises AbilityDescriptor`. Device- and hub-owned callee URAs
are daemon/Axon compatibility for current owner-resolution paths and MUST be
treated as explicitly versioned capability, not silently frozen as the only
public SDK contract. `callee` is never the UDS path, TCP endpoint, daemon
process, node id string, plugin instance, or backend DB row.

Caller/callee/subject kind rules:

| Invocation field | Stable allowed kinds | Compatibility allowed kinds | Rule |
| --- | --- | --- | --- |
| `caller` | `agent` | daemon-authorized service agent only | Caller carries accountability and signing authority. |
| `callee` | `agent` | `device` and `hub` while owner-resolution compatibility is enabled | SDK MUST surface capability/version flags for non-agent callees. |
| `subject` | `resource`, `device`, `agent`, `ability`, `hub`, `user`, session/state refs when Axon grammar supports them | opaque typed refs returned by daemon | Subject is the acted-on entity; it is not a substitute for callee. |

### 7.3 Transport Split

`control.sock` is only for boot/status/discovery. Product calls MUST use the
daemon Invocation endpoint (`daemon.sock` locally, TCP+TLS for remote
device/Hub traffic). SDKs MAY use `control.json` to discover the current
Invocation endpoint, but MUST NOT dispatch product ability calls over control
frames.

### 7.4 Axon Dependency Containment

Only the native Rust daemon SDK core and daemon runtime adapter MAY depend on
Axon SDKs and generated protocol types internally. Language SDK facades,
including the Go SDK consumed by EasyNet backend, MUST NOT import Axon or own an
Axon adapter. Public product APIs MUST wrap Axon-owned semantics in EasyNet-Cli
owned DTOs and typed errors.

### 7.5 Descriptor and Receipt Binding

`AbilityDescriptor` MUST carry descriptor version and schema hash. `AbilityImpl`
MUST carry implementation hash and runtime environment. Invocation/Receipt
projections exposed by the SDK MUST preserve enough fields for callers to bind:

1. descriptor version,
2. schema hash,
3. implementation hash when daemon returns it,
4. runtime environment when daemon returns it,
5. authority proof,
6. input hash,
7. output hash,
8. parent receipts,
9. terminal state,
10. signature or verifiable receipt reference.

The SDK does not create these protocol truths. It preserves and projects them.

## 8. Target Project Structure

The current repository already contains Rust daemon code and a C header. The
target structure makes the SDK boundary explicit and follows Axon's separation
between core, SDK, packaging, documents, and examples.

### 8.1 Required Roots

```text
EasyNet-Cli/
  README.md
  PROJECT_STRUCTURE.md
  Cargo.toml
  include/
    easynet_cli.h
  src/
    daemon/
      lifecycle/
      health/
      runtime_client/
      directory/
      identity/
      transport/
    ffi/
      abi_version.rs
      handles.rs
      runtime.rs
      invocation.rs
      errors.rs
    protocol/
      canonical_json.rs
      invocation_json.rs
      receipt_json.rs
      error_json.rs
      health_json.rs
      event_json.rs
  sdk/
    README.md
    SDK_INTERFACE_SPEC.md
    SDK_PARITY.md
    CONFORMANCE_SUITE.md
    schemas/
      invocation.schema.json
      receipt.schema.json
      error.schema.json
      health.schema.json
      events.schema.json
    conformance/
      cases/
      fixtures/
      runner/
    rust/
    c/
    go/
    python/
    node/
    java/
    swift/
  docs/
    spec/
      daemon-sdk-requirements-v1.md
  packaging/
    release/
    sdk-pack/
  examples/
  gallery/
```

If the existing physical layout differs, migration MUST be staged. The spec
defines the final ownership model; it does not require a single large move.

### 8.2 Ownership by Directory

| Path | Owns | Must not own |
| --- | --- | --- |
| `src/daemon/` | daemon process lifecycle, UDS endpoints, local runtime orchestration, native SDK core objects | language-specific SDK ergonomics |
| `src/protocol/` | Axon-derived JSON/schema projection and typed daemon protocol DTOs | product helpers, EasyRemote decorators, or second canonical Invocation grammar |
| `src/ffi/` | C ABI handle model and error conversion | daemon business policy |
| `include/easynet_cli.h` | stable public ABI declarations | generated Axon structs |
| `sdk/schemas/` | JSON schema projections for language parity, derived from Axon canonical shapes and daemon DTOs | implementation-only Rust structs or schema-as-canonical protocol truth |
| `sdk/conformance/` | golden fixtures generated or validated against Axon helpers and cross-language behavior tests | examples that silently define behavior |
| `sdk/rust/` | Native Rust SDK public surface if separated from current crate modules | duplicated Axon or daemon semantics |
| `sdk/c/` | C examples, ABI docs, generated binding metadata | duplicated C implementation |
| `sdk/go/` | Go facade package used by EasyNet backend | Axon imports in public package surface or duplicated Axon semantics |
| `sdk/python/` | Python facade package consumed by EasyRemote | EasyRemote-specific decorators as core SDK or duplicated Axon semantics |
| `sdk/node/` | Node/TypeScript facade package | browser-only assumptions or duplicated Axon semantics |
| `sdk/java/` | Java/JVM facade package | Android UI concerns or duplicated Axon semantics |
| `sdk/swift/` | Swift facade package | platform UI concerns |
| `packaging/` | release bundles, wheels, npm/maven/spm/crate metadata | SDK semantics |
| `examples/` | runnable examples | protocol orchestration source of truth |
| `gallery/` | demos and showcases | SDK tests or normative behavior |

### 8.3 Structural Migration Rules

1. Physical moves use `git mv` so history remains traceable.
2. Do not mix large structural moves with unrelated logic changes.
3. Every moved public artifact must update docs, tests, CI, release scripts, and
   generated references in the same stage.
4. There must be one source of truth per artifact class:
   - Invocation schema projection:
     `sdk/schemas/invocation.schema.json`, generated or validated from Axon
     canonical Invocation shape; it is not an independent protocol source.
   - C ABI declaration: `include/easynet_cli.h`.
   - Rust daemon transport: `src/daemon/transport/`.
   - Native daemon SDK semantics: Rust crate `easynet_cli`.
   - Language wrappers: `sdk/<language>/`.
   - Golden behavior: `sdk/conformance/cases/`, generated or checked against
     Axon helpers where URA, DescriptorRef, canonical bytes, receipt, stream, or
     bidi semantics are involved.
5. `examples/` and `gallery/` can import SDK packages; SDK packages must never
   import examples or gallery code.
6. Backend cutover must depend on `sdk/go`, not `src/daemon` internals.
7. EasyRemote cutover must depend on `sdk/python`, not backend code or direct C
   ABI calls. `sdk/python` may use C ABI internally as an implementation detail.

## 9. OOP Object Model

The SDK is object-oriented at the public boundary even when a target language
has a functional or procedural implementation style. C ABI represents objects as
opaque handles; all other languages expose classes/structs/interfaces with the
same lifecycle.

### 9.1 Object Graph

```text
SdkEnvironment
  -> DaemonHandle
       -> RuntimeClient
            -> InvocationBuilder
                 -> InvocationDraft
                      -> PreparedInvocation
                           -> SignedInvocation
                                -> InvocationHandle
                                     -> InvocationResult
            -> StreamHandle
            -> BidiSession
       -> DirectoryClient
       -> IdentityClient
       -> ReceiptClient
       -> PublicationClient
       -> HostBindingClient
       -> MissionClient
       -> AdminClient
       -> EventClient
       -> SurfaceClient
       -> CompatibilityClient
       -> HealthClient
```

No object in this graph may expose raw Axon client/proto/runtime types.

### 9.1.1 Layered Runtime Structure

The object graph is layered. `SdkEnvironment` is the process-level SDK root; it
is not part of a single Invocation's proof chain. Invocation objects start at
`InvocationBuilder` and become proof-bearing only after prepare/sign/submit.

| Layer | Public object(s) | Owns | Must not own |
| --- | --- | --- | --- |
| Product facade | EasyNet backend, EasyRemote, CLI, GUI | HTTP/auth/DB/UI projection, product ergonomics, examples, browser route shapes | daemon transport, Axon canonical bytes, receipt verification, stream/bidi terminal semantics |
| Language SDK facade | Go, Python, Node, Java, Swift packages | idiomatic APIs, async adapters, generated DTOs, error mapping, packaging | protocol semantics, hidden invocation defaults, backend product state |
| SDK process root | `SdkEnvironment` | SDK library initialization, feature/version checks, default path resolution, global resource cleanup | invocation tuple state, daemon ownership, signer authority, request replay policy |
| Daemon lifecycle | `DaemonHandle` | discover/start/attach/status/endpoints/stop/detach for `easynet-daemon` | normal product ability calls, Axon runtime lifecycle, backend process liveness |
| Runtime connection | `RuntimeClient` | authenticated daemon Invocation endpoint connection, retry classification, request dispatch | canonical Invocation algorithms, product HTTP routes, daemon process ownership |
| Invocation construction | `InvocationBuilder`, `InvocationDraft` | complete seven-tuple construction and inspection | canonical signing material, signatures, runtime terminal state |
| Signing boundary | `PreparedInvocation`, `SigningMaterial`, `SignedInvocation` | canonical bytes projection, signer binding, submit-ready signed envelope | mutating the seven-tuple after prepare, re-signing in backend/facade code |
| Runtime observation | `InvocationHandle`, `StreamHandle`, `BidiSession`, `InvocationResult` | submitted invocation observation, cancellation, terminal state, receipt/output projection | fabricating receipts, merging cancel/timeout/failure states |
| Profile clients | Directory, Identity, Receipt, Publication, Host Binding, Mission, Admin, Events, Surface, Compatibility, Wrappers, Health | typed daemon profile DTOs over the same runtime boundary | product-specific HTTP/DB DTOs or one-method-per-ability protocol forks |

### 9.1.2 Invocation Object Lifecycle Map

`PreparedInvocation` and `SignedInvocation` are intentionally separate. They are
not two names for "ready to run". `PreparedInvocation` is canonical signing
material; `SignedInvocation` is the submit-ready envelope.

| Object | Lifecycle role | Submit-ready | Signature-bearing | Mutation allowed | Semantic rule |
| --- | --- | --- | --- | --- | --- |
| `SdkEnvironment` | SDK process root | no | no | feature/path config before runtime use only | Never appears in a single Invocation proof chain. |
| `InvocationBuilder` | mutable construction helper | no | no | yes | May fill ergonomic defaults only when the final seven-tuple remains inspectable. |
| `InvocationDraft` | immutable complete seven-tuple snapshot | no | no | no | Missing caller, callee, descriptor, subject, nonce, causal context, or args is invalid. |
| `PreparedInvocation` | immutable canonical signing material | no | no | no | Contains canonical bytes, expiry, descriptor binding, request id, and tuple snapshot. It is not executable. |
| `SigningMaterial` | signer-facing bytes and metadata | no | no | no | Stable across language bindings for the same draft. |
| `SignedInvocation` | immutable caller-signed submit envelope | yes | yes | no | Preserves caller or daemon-authorized local signer material; SDK/backend must not re-sign it. |
| `InvocationHandle` | submitted invocation observer | already submitted | already submitted | cancel/close only | Owns event cursor, cancellation, and await semantics. |
| `InvocationResult` | terminal projection | terminal | receipt-dependent | no | Projects output/error/receipt/terminal status without claiming verification unless receipt data supports it. |
| `CausalRef` | child-call causal input | no | receipt-derived | no | Built from terminal receipts through `ReceiptClient`, never guessed by a facade. |

Normative flow:

```text
SdkEnvironment
  -> DaemonHandle
    -> RuntimeClient
      -> InvocationBuilder
        -> InvocationDraft
          -> PreparedInvocation + SigningMaterial
            -> SignedInvocation
              -> InvocationHandle
                -> InvocationResult
                  -> ReceiptClient.causal_ref(...)
                    -> child InvocationBuilder
```

Stream and bidi are submission variants of the same signing flow:

```text
InvocationDraft -> PreparedInvocation -> SignedInvocation
  -> open_stream -> StreamHandle -> terminal receipt
```

```text
InvocationDraft -> PreparedInvocation -> SignedInvocation
  -> open_bidi(frame0) -> BidiSession -> terminal receipt
```

Local daemon signing does not remove the signing boundary. It is represented as:

```text
PreparedInvocation -> SignedInvocation(signer = local-daemon, policy = local_signing_allowed) -> Submitted
```

It MUST NOT be represented as a direct `PreparedInvocation -> Submitted` public
state, because that collapses canonical material and submit-ready envelope into
one object and makes caller accountability ambiguous.

### 9.2 Core Objects

| Object | Responsibility | Owns resources | Terminal operation |
| --- | --- | --- | --- |
| `SdkEnvironment` | SDK initialization, version checks, default paths, feature flags | global SDK config only; no per-invocation state | `close` / drop |
| `DaemonHandle` | discover, start, attach, stop, inspect daemon process | optional child process plus socket metadata | `stop`, `detach`, drop |
| `RuntimeClient` | authenticated connection to daemon invocation endpoint | UDS/TCP connection pool, retry policy, request ids | `close` |
| `InvocationBuilder` | construct a complete seven-tuple draft | mutable draft only | `build`, `prepare`, `invoke` |
| `InvocationDraft` | inspectable immutable seven-tuple before canonical prepare | tuple snapshot | `prepare`, `invoke` |
| `PreparedInvocation` | immutable canonical signing material, not an executable call | canonical bytes, expiry, descriptor binding, request id, tuple snapshot | `sign`, local-daemon sign policy |
| `SignedInvocation` | immutable submit-ready signed envelope | signature, signer id, signer policy, prepared tuple | `submit` |
| `InvocationHandle` | observe submitted invocation | invocation id, event cursor | `await_result`, `cancel`, `events` |
| `InvocationResult` | terminal result projection | receipt, output, terminal status | none |
| `StreamHandle` | receive server-stream events with terminal state | stream cursor and backpressure state | `close`, `cancel` |
| `BidiSession` | send/receive bidirectional frames | send queue, receive cursor, frame0 metadata | `close_send`, `close`, `cancel` |
| `DirectoryClient` | device/agent/ability catalog read model | subscription cursors | `close` |
| `IdentityClient` | local caller identity and signing key access | key handles, policy metadata | `close` |
| `ReceiptClient` | fetch/project/verify receipts and build causal refs | receipt cursors/cache if any | `close` |
| `PublicationClient` | publish AbilityDescriptor/AbilityImpl resources and host-stream bindings | publication transaction handles | `close` |
| `HostBindingClient` | encode/decode host-stream request, item, error, terminal, and output-hash semantics | optional host-binding codec state | `close` |
| `MissionClient` | submit, observe, and cancel daemon-owned Mission/EAL runs | mission event cursors | `close` |
| `AdminClient` | daemon admin, gateway readiness, agent lifecycle, and join/leave helpers | admin event cursors if any | `close` |
| `EventClient` | subscribe to directory, device, session, and invocation events with cursor/reconnect/drop semantics | event subscriptions and cursors | `close` |
| `SurfaceClient` | list/create/delete page records, fetch surface manifests, and construct public page refs over daemon-governed surfaces | surface request cursors if any | `close` |
| `CompatibilityClient` | project compatibility API requests such as OpenAI models/chat/files onto governed daemon abilities and SDK file/directory DTOs | compatibility stream/file handles if any | `close` |
| `HealthClient` | readiness and diagnostics | none or short-lived connection | none |
| `DaemonError` | typed error and retry classification | code, message, source, retry hint | none |

### 9.3 Required Interfaces

The exact syntax is language-specific, but every target must represent these
interfaces.

```text
interface DaemonHandle {
  status() -> DaemonStatus
  endpoints() -> Endpoints
  runtime() -> RuntimeClient
  directory() -> DirectoryClient
  identity() -> IdentityClient
  receipts() -> ReceiptClient
  publication() -> PublicationClient
  host_binding() -> HostBindingClient
  missions() -> MissionClient
  admin() -> AdminClient
  events() -> EventClient
  surfaces() -> SurfaceClient
  compatibility() -> CompatibilityClient
  health() -> HealthClient
  stop(options) -> StopResult
  detach() -> void
}

interface RuntimeClient {
  new_invocation() -> InvocationBuilder
  invoke(invocation) -> InvocationResult
  prepare(invocation) -> PreparedInvocation
  submit_signed(signed) -> InvocationHandle
  open_stream(invocation) -> StreamHandle
  open_bidi(invocation, frame0) -> BidiSession
  close() -> void
}

interface InvocationBuilder {
  caller(ura) -> InvocationBuilder
  callee(ura) -> InvocationBuilder
  descriptor(ref) -> InvocationBuilder
  subject(ura_or_subject) -> InvocationBuilder
  nonce(bytes_or_string) -> InvocationBuilder
  causal_context(context) -> InvocationBuilder
  args(json_or_bytes) -> InvocationBuilder
  inspect() -> InvocationDraft
  prepare() -> PreparedInvocation
  invoke() -> InvocationResult
}

interface InvocationDraft {
  inspect_tuple() -> InvocationTuple
  prepare() -> PreparedInvocation
  invoke() -> InvocationResult
}

interface PreparedInvocation {
  tuple() -> InvocationTuple
  signing_material() -> SigningMaterial
  expiry() -> Timestamp
  descriptor_binding() -> DescriptorBinding
  sign(signer) -> SignedInvocation
  close() -> void
}

interface SignedInvocation {
  prepared() -> PreparedInvocation
  signer_id() -> SignerID
  signature() -> Signature
  submit() -> InvocationHandle
  close() -> void
}

interface InvocationHandle {
  events(options) -> EventStream<InvocationEvent>
  cancel(reason) -> CancellationResult
  await_result(options) -> InvocationResult
  close() -> void
}

interface InvocationResult {
  terminal_state() -> InvocationTerminalState
  output() -> OutputRef
  error() -> DaemonError
  receipt() -> ReceiptSummary
}

interface DirectoryClient {
  resolve(ref_or_query) -> ResolvedRef
  list_devices(query) -> Page<DeviceRecord>
  list_agents(query) -> Page<AgentRecord>
  list_abilities(query) -> Page<AbilityRecord>
  subscribe(query) -> DirectoryEventStream
  close() -> void
}

interface IdentityClient {
  local_identity(options) -> LocalIdentity
  build_user_ura(request) -> URA
  build_device_ura(request) -> URA
  build_agent_ura(request) -> URA
  build_resource_ref(request) -> ResourceRef
  register_signing_key(request) -> SigningKeyRecord
  list_signing_keys(query) -> Page<SigningKeyRecord>
  revoke_signing_key(id) -> void
  signer(request) -> Signer
  close() -> void
}

interface ReceiptClient {
  fetch(ref) -> Receipt
  project(ref_or_receipt) -> ReceiptSummary
  verify(receipt_or_ref) -> VerificationResult
  causal_ref(receipt_or_ref) -> CausalRef
  close() -> void
}

interface PublicationClient {
  build_resource_ref(request) -> ResourceRef
  validate_package(path_or_manifest, options) -> PackageValidation
  deploy_ability(request) -> AbilityDeployResult
  list_abilities(query) -> Page<PublishedAbility>
  show_ability(ref) -> PublishedAbility
  enable_impl(id) -> void
  disable_impl(id) -> void
  unpublish(ref) -> void
  close() -> void
}

interface HostBindingClient {
  build_host_stream_binding(request) -> HostStreamBinding
  decode_request(envelope) -> HostStreamRequest
  encode_item(seq, value) -> HostStreamFrame
  encode_error(error) -> HostStreamFrame
  encode_terminal(summary) -> HostStreamFrame
  fold_output_hash(state, seq, value) -> HostStreamHashState
  close() -> void
}

interface MissionClient {
  run_eal(request) -> MissionRun
  run_file(path, options) -> MissionRun
  track(id) -> MissionStatus
  cancel(id) -> MissionCancelResult
  events(id, options) -> MissionEventStream
  close() -> void
}

interface AdminClient {
  gateway_status(options) -> GatewayStatus
  agent_start(request) -> AgentStartResult
  list_agents(query) -> Page<AgentRecord>
  refresh_agent(request) -> AgentRefreshResult
  join_hub(request) -> JoinResult
  leave_hub(request) -> LeaveResult
  pairing_preflight(request) -> PairingPreflight
  validate_pairing(request) -> DeviceCredential
  verify_device_credential(request) -> VerificationResult
  create_pairing(request) -> PairingToken
  revoke_device(request) -> DeviceAdminResult
  create_device_session(request) -> DeviceSession
  list_device_sessions(query) -> Page<DeviceSession>
  delete_device_session(id) -> DeviceAdminResult
  close() -> void
}

interface EventClient {
  subscribe_directory(query) -> EventStream<DirectoryEvent>
  subscribe_devices(query) -> EventStream<DeviceEvent>
  subscribe_sessions(query) -> EventStream<SessionEvent>
  subscribe_invocations(query) -> EventStream<InvocationEvent>
  list_device_events(query) -> Page<DeviceEvent>
  close() -> void
}

interface SurfaceClient {
  list_pages(query) -> Page<PageRecord>
  create_page(request) -> PageRecord
  delete_page(id) -> SurfaceMutationResult
  surface_manifest(request) -> SurfaceManifest
  public_page_ref(request) -> PublicPageRef
  close() -> void
}

interface CompatibilityClient {
  list_models(request) -> CompatibilityModelPage
  chat_completions(request) -> CompatibilityCompletion
  stream_chat_completions(request) -> StreamHandle
  upload_file(request) -> CompatibilityFile
  get_file(request) -> CompatibilityFile
  delete_file(request) -> CompatibilityDeleteResult
  close() -> void
}

interface HealthClient {
  readiness(options) -> RuntimeHealth
  diagnostics(options) -> DiagnosticsReport
  close() -> void
}
```

Convenience helpers are allowed only if they lower to this object graph:

```text
client.device("edge-01").ability("fs.read").subject(file_ura).invoke(args)
```

The helper above is valid only if the SDK can expose the resulting full
Invocation draft before dispatch.

### 9.4 Lifetime Rules

1. `SdkEnvironment` owns SDK process-level initialization only. It MUST NOT
   store per-invocation tuple fields, signer material, or submitted invocation
   state.
2. `DaemonHandle` may own a process or attach to an existing process.
3. `detach` releases the local handle without stopping an existing daemon.
4. `stop` is idempotent and may only stop a process the handle is authorized to
   control.
5. `RuntimeClient` does not imply process ownership.
6. Closing a `RuntimeClient` never stops the daemon.
7. `InvocationBuilder` is mutable and not thread-safe unless a language marks it
   as such.
8. `InvocationDraft`, `PreparedInvocation`, and `SignedInvocation` are immutable
   and safe to share where the language supports immutable sharing.
9. `PreparedInvocation` owns signing material only. It has no public submit
   operation; local daemon signing must produce a `SignedInvocation`.
10. `SignedInvocation` is the only submit-ready pre-runtime Invocation object.
11. `InvocationHandle.cancel` sends cancellation to daemon/runtime; dropping the
   local handle does not imply cancellation unless explicitly documented.
12. `StreamHandle` and `BidiSession` must expose explicit close/cancel semantics.
13. All object methods that touch the daemon must surface typed `DaemonError`,
    not string-only failures.

### 9.5 Language-specific Shape

| Concept | Rust | C ABI | Go | Python | Node/TS | Java | Swift |
| --- | --- | --- | --- | --- | --- | --- | --- |
| daemon handle | `DaemonHandle` struct | `easynet_daemon_t*` | `*DaemonHandle` | `DaemonHandle` | `DaemonHandle` | `DaemonHandle` | `DaemonHandle` |
| runtime client | `RuntimeClient` | `easynet_runtime_t*` | `*runtime.Client` | `RuntimeClient` | `RuntimeClient` | `RuntimeClient` | `RuntimeClient` |
| builder | owned builder | `easynet_invocation_builder_t*` | pointer receiver builder | mutable object | chainable class | builder class | value or class builder |
| async stream | `Stream<Item=Event>` | polling/callback handle | channel/iterator | async iterator | `AsyncIterable` | `Flow.Publisher` or iterator | `AsyncSequence` |
| close | `Drop` + explicit close | explicit destroy | `Close()` | context manager + `close` | `close()` | `close()` / `AutoCloseable` | `close()` / `deinit` |
| cancellation | cancellation token | cancel function | `context.Context` | task cancellation + explicit cancel | `AbortSignal` + explicit cancel | `CompletableFuture.cancel` + explicit cancel | task cancellation + explicit cancel |

The C ABI is not exempt from the object model; it implements the same objects as
opaque handles.

### 9.6 Naming Conventions

All languages expose the same objects and state transitions with idiomatic names:

| Target | Naming rule | Example |
| --- | --- | --- |
| Rust | `snake_case` methods, `PascalCase` types | `RuntimeClient::connect_local`, `InvocationBuilder::with_subject` |
| C ABI | `easynet_*` symbol prefix, explicit handles | `easynet_runtime_connect_local`, `easynet_invocation_builder_set_subject` |
| Go | `PascalCase` exported names | `runtime.ConnectLocal`, `InvocationBuilder.WithSubject` |
| Python | `snake_case` methods, `PascalCase` classes | `RuntimeClient.connect_local`, `InvocationBuilder.with_subject` |
| Node / TypeScript | `camelCase` methods, `PascalCase` classes | `RuntimeClient.connectLocal`, `builder.withSubject` |
| Java / Kotlin | `camelCase` methods, `PascalCase` classes | `RuntimeClient.connectLocal`, `InvocationBuilder.withSubject` |
| Swift | `camelCase` methods, `PascalCase` types | `RuntimeClient.connectLocal`, `InvocationBuilder.withSubject` |

Semantic divergence hidden behind naming is a conformance failure. For example,
`submit_signed`, `SubmitSigned`, `submitSigned`, and
`easynet_invocation_submit_signed` must all preserve the same pre-signed caller
material, return the same receipt shape, and emit the same terminal state.

### 9.7 Interface Completeness Contract

An SDK object is not complete merely because one transport path can exercise one
happy-path operation. An interface is complete only when all of the following are
true:

1. The object has a public construction or discovery path.
2. Every mutating or I/O operation has a typed success result and typed failure
   result.
3. Every state machine in section 10 has public observation for terminal states.
4. Every owned resource has an explicit close, detach, stop, cancel, or destroy
   operation.
5. Every JSON DTO accepted by one language binding has a schema and conformance
   fixture shared with the other bindings.
6. Every convenience helper lowers to an inspectable complete Invocation draft
   before dispatch.
7. No public product-facing type exposes raw Axon protobuf, raw daemon gRPC, raw
   daemon socket frames, Rust pointers, or backend product DTOs.

The stable interface baseline is:

| Object family | Required public operations | Completeness condition |
| --- | --- | --- |
| `SdkEnvironment` / ABI root | version, feature discovery, default path resolution, init, shutdown, last error, string/free helpers | Bindings can reject incompatible library or daemon versions before opening runtime traffic. |
| `DaemonHandle` | discover, start, attach, status, endpoints, open runtime client, stop, detach | Start/attach succeeds only when the Invocation endpoint is ready; detach and stop are distinct. |
| `RuntimeClient` | connect local/remote when allowed, health, new invocation builder, invoke, prepare, submit signed, open stream, open bidi, close | Runtime traffic never uses control frames and exposes typed readiness and terminal failures. |
| `InvocationBuilder` / `InvocationDraft` | set caller, callee, descriptor ref, subject, nonce, causal context, args, metadata, authority, timeout, idempotency; inspect, build, prepare, invoke | Missing seven-tuple fields are rejected before canonical prepare or submit. |
| `PreparedInvocation` / `SignedInvocation` | expose canonical signing material, expiry, descriptor binding, sign, local-daemon signing policy, submit signed, destroy/free | `PreparedInvocation` is not submit-ready; `SignedInvocation` preserves caller or daemon-authorized signer material and never re-signs with a facade or backend key. |
| `InvocationHandle` / `InvocationResult` | await result, cancel, event stream, terminal receipt/output/error projection | Terminal state is monotonic and observable even when transport fails after submission. |
| `StreamHandle` | receive/poll or callback, terminal frame, cancel, close, bounded backpressure reporting | EOF, terminal receipt, timeout, cancellation, and transport failure are distinguishable. |
| `BidiSession` | open with frame0, send, receive/callback, close send, close, cancel, terminal receipt/event | Frame0 is generated from the complete Invocation and close-send is distinct from cancel. |
| `DirectoryClient` | resolve, list devices, list agents, list abilities, subscribe, close | Lists are paginated read-model queries and do not perform default governed fan-out. |
| `IdentityClient` | local identity lookup, URA builders, user key register/list/revoke, signer construction, close | Keyring policy remains daemon-owned; backend JWT/OAuth concepts do not leak in. |
| `ReceiptClient` | fetch receipt, project receipt summary, verify when full body is available, construct causal refs from parent receipts | Facades do not fabricate receipt URAs or pretend summary-only data proves cryptographic validity. |
| `PublicationClient` | resource refs, package validation, deploy/list/show/enable/disable/unpublish AbilityImpls, host-stream binding DTOs | Product facades do not own generic daemon system ability carriers. |
| `HostBindingClient` | host-stream binding, request envelope decode, item/error/terminal frame encode, output-hash fold | Product hosts do not own canonical host-stream wire, terminal, or hash semantics. |
| `MissionClient` | run EAL/source/file, track, cancel, events, terminal mission status | Product DSLs compile to EAL but do not own daemon Mission transport or child Invocation semantics. |
| `AdminClient` | gateway status, agent start/list/refresh, join/leave helpers, device admin DTOs | Product facades do not own daemon admin carriers or hide process bootstrap in normal calls. |
| `EventClient` | directory/device/session/invocation subscribe, historical device events, cursor/reconnect/drop reporting, close | Backend/GUI fanout uses SDK event cursors instead of raw daemon stream loops. |
| `SurfaceClient` | list/create/delete pages, fetch surface manifests, build public page refs, close | Backend renders and authenticates pages but does not own daemon surface ability carriers. |
| `CompatibilityClient` | list models, chat completions, streaming completions, compatibility file upload/get/delete adapters, close | Backend owns HTTP/API-key product policy but not compatibility-to-Invocation shims. |
| `HealthClient` | daemon/runtime readiness, endpoint readiness, trust/directory/runtime flags, version mismatch reporting | API liveness and runtime readiness are separate typed values. |
| `DaemonError` | stable code, stage, message, retry hint, receipt/invocation reference, source mapping | Consumers never parse human error strings for control flow. |

Complete Invocation dispatch in the current C ABI is necessary but not
sufficient. `easynet_invocation_invoke`, `easynet_invocation_stream_open`, and
`easynet_invocation_bidi_open` prove that product calls can cross the daemon
Invocation endpoint with the seven-tuple visible, but they do not by themselves
complete the SDK object model. The SDK is incomplete until prepare/sign/submit,
receipt fetch/projection, directory, identity, publication, host binding,
mission, admin/gateway, events, surface, compatibility, health, typed error
JSON, lifecycle attach/detach, and shared conformance fixtures exist at the
same semantic level.

### 9.8 Minimum C ABI Family Coverage

The C ABI projects the same object model through opaque handles and JSON DTOs.
It MUST eventually cover these function families before P0 bindings are marked
stable:

| Family | Required C ABI coverage | Current ABI v3 status |
| --- | --- | --- |
| ABI root | `easynet_abi_version`, feature discovery, `easynet_last_error`, `easynet_string_free` | Version, last error, and string free exist; feature discovery is missing. |
| SDK/client environment | initialize from control descriptor or endpoint, shutdown, close client | `easynet_init` and `easynet_shutdown` exist. |
| Daemon lifecycle | start, attach, discover, status, endpoints, open client, stop, detach | start/stop/status/invocation endpoint/open client exist; explicit attach, discover, all-endpoints, and detach are missing. |
| Runtime client | connect/open, health, invoke, prepare, submit signed, stream, bidi, close | open client, unary invoke, stream, bidi, and shutdown/close exist; health, prepare, and submit signed are missing. |
| Invocation builder | create builder, set seven tuple fields, set metadata/authority/timeout/idempotency, inspect draft, build/free | Missing as a typed handle API; callers currently pass whole Invocation JSON. |
| Prepared/signed invocation | prepare draft, return signing material, attach caller signature, submit signed, free prepared/signed handles | Missing. |
| Invocation handle/result | await result, cancel, events, result/receipt JSON, destroy | Unary is synchronous receipt JSON only; async handle/events are missing. |
| Stream | open, receive/callback, terminal event schema, cancel, close | open/callback/cancel exist; close and schema-fixture-backed terminal event contract are incomplete. |
| Bidi | open, send, receive/callback, close send, close, cancel, terminal event schema | open/send/close/cancel exist; explicit close-send and schema-fixture-backed terminal event contract are incomplete. |
| Directory | resolve, paginated list devices/agents/abilities, subscribe, close/free | Missing. |
| Identity | local identity, URA builders, signing key register/list/revoke, signer helpers | Missing. |
| Receipt | fetch/project receipt, full receipt verification entry point, parent receipt to causal ref, receipt chain continuity | Missing; current unary result returns summaries only. |
| Publication | resource refs, package validation, deploy/list/show/enable/disable/unpublish, host-stream binding DTOs | Missing as SDK ABI; callers invoke system abilities manually today. |
| Host Binding | host-stream request envelope, frame/error/terminal codec, output hash, readiness/cleanup DTOs | Missing as SDK ABI; EasyRemote owns host-stream wire helpers today. |
| Mission | run/track/cancel/events, mission status, child receipt refs, output refs | Missing as SDK ABI; callers invoke system abilities manually today. |
| Admin + Gateway | gateway status, hub/device join/leave, agent start/list/refresh, admin DTOs | Missing as SDK ABI; callers invoke system abilities manually today. |
| Events | directory/device/session/invocation subscribe, event cursor, reconnect hint, dropped-count reporting, close/free | Missing as SDK ABI; backend owns daemon event stream loops today. |
| Surface | page list/create/delete, surface manifest, public page ref, surface status | Missing as SDK ABI; backend would otherwise invoke page/surface abilities manually. |
| Compatibility | model list, chat completion, streaming completion, compatibility file create/get/delete, typed adapter errors | Missing as SDK ABI; backend would otherwise own raw OpenAI-to-ability shims. |
| Health/diagnostics | runtime health JSON, readiness flags, version mismatch, trust/directory/runtime readiness | Daemon status exists; full typed runtime health is missing. |
| Errors | stable integer codes plus typed error JSON projection | Integer codes and last-error string exist; typed error JSON projection is missing. |

ABI v3 should not be renamed to "complete SDK ABI" until every P0 family above
has either a stable symbol or an explicit documented non-goal with a replacement
path. Until then, ABI v3 is the complete Invocation-only dispatch ABI.

## 10. Normative State Machines

The SDK must model daemon lifecycle separately from invocation execution. A
healthy API process with a dead daemon is not a healthy runtime.

### 10.1 Daemon Lifecycle State Machine

```text
Unknown
  -> Discovered
  -> Starting
  -> ControlReady
  -> InvocationReady
  -> Running
  -> Stopping
  -> Stopped
```

Failure/degraded states:

```text
ConfigInvalid
PermissionDenied
VersionMismatch
ControlOnly
InvocationDown
StartFailed
CrashLoop
```

Transitions:

| From | Event | To | Rule |
| --- | --- | --- | --- |
| `Unknown` | `discover_ok` | `Discovered` | SDK found install paths and expected sockets. |
| `Unknown` | `config_invalid` | `ConfigInvalid` | Required config is missing or invalid. |
| `Discovered` | `start_requested` | `Starting` | SDK may spawn daemon only if policy allows. |
| `Discovered` | `attach_running` | `ControlReady` | Existing daemon control socket is reachable. |
| `Starting` | `control_socket_ready` | `ControlReady` | Control channel is alive but product calls are not ready. |
| `ControlReady` | `invocation_socket_ready` | `InvocationReady` | Invocation endpoint is reachable and version-compatible. |
| `InvocationReady` | `readiness_ok` | `Running` | Directory, identity, and runtime readiness checks passed. |
| `ControlReady` | `invocation_socket_missing` | `ControlOnly` | `/health` may pass liveness, but ability calls must fail readiness. |
| `InvocationReady` | `version_mismatch` | `VersionMismatch` | SDK and daemon protocol versions are incompatible. |
| `Running` | `stop_requested` | `Stopping` | Stop is idempotent. |
| `Stopping` | `process_exited` | `Stopped` | Handle can be dropped safely. |
| any non-terminal | `permission_denied` | `PermissionDenied` | Socket or key material is inaccessible. |
| any non-terminal | `process_crashed` | `CrashLoop` | Retry policy may restart or report failure. |

Invariants:

1. `RuntimeClient` may be created only from `InvocationReady` or `Running`.
2. `ControlReady` is not sufficient for product ability calls.
3. `ControlOnly` MUST be reported as degraded readiness, not success.
4. `stop` is idempotent.
5. `detach` never sends a stop signal.
6. Version mismatch is terminal for that client instance.

### 10.2 Client Connection State Machine

```text
Idle -> Resolving -> Connecting -> Ready -> Closed
                         |          |
                         v          v
                      Failed     Degraded -> Reconnecting -> Ready
```

Transitions:

| From | Event | To | Rule |
| --- | --- | --- | --- |
| `Idle` | `connect` | `Resolving` | Resolve socket path, protocol version, and auth material. |
| `Resolving` | `resolved` | `Connecting` | Begin daemon transport connection. |
| `Connecting` | `handshake_ok` | `Ready` | SDK can send invocation traffic. |
| `Connecting` | `handshake_failed` | `Failed` | Error includes retry hint. |
| `Ready` | `transport_lost` | `Degraded` | In-flight calls receive typed transport errors or resume if allowed. |
| `Degraded` | `retry_allowed` | `Reconnecting` | Reconnect policy starts. |
| `Reconnecting` | `handshake_ok` | `Ready` | New calls may continue. |
| any | `close` | `Closed` | Explicit close wins over reconnect. |

Invariants:

1. New invocation submissions require `Ready`.
2. Reconnect is a transport concern; it must not silently replay a
   non-idempotent invocation unless the invocation carries an idempotency key
   and replay policy.
3. `Closed` is terminal.
4. `Failed` may be retried only by constructing or reopening a client.

### 10.3 Invocation Draft, Signing, and Submission State Machine

This SDK state machine wraps, but does not redefine, Axon's invocation state
machine. Axon runtime states remain:

```text
ACCEPTED -> ADMITTED -> DISPATCHED -> RUNNING -> COMPLETED
                                               -> FAILED
                                               -> TIMED_OUT
                                               -> CANCELLED
```

Daemon SDK adds pre-runtime states:

```text
EmptyBuilder -> DraftMutable -> DraftImmutable -> Prepared -> Signed
  -> Submitted -> Accepted -> Admitted -> Dispatched -> Running -> Completed
                                                        -> Failed
                                                        -> TimedOut
                                                        -> Cancelled
```

Transitions:

| From | Event | To | Rule |
| --- | --- | --- | --- |
| `EmptyBuilder` | `set_first_field` | `DraftMutable` | Builder has started tuple construction. |
| `DraftMutable` | `inspect` or `build` | `DraftImmutable` | All required seven-tuple fields must be present or the transition fails. |
| `DraftImmutable` | `prepare` | `Prepared` | Daemon/Rust SDK core delegates canonical material construction to Axon-owned semantics. |
| `DraftImmutable` | `invoke` | `Submitted` | Convenience path internally performs prepare, sign, and submit according to policy; tracing must reveal those internal states. |
| `Prepared` | `sign` | `Signed` | Caller signature covers canonical bytes. |
| `Prepared` | `local_daemon_sign` | `Signed` | Only if daemon policy explicitly permits local signing; signer id and policy proof are recorded. |
| `Signed` | `submit` | `Submitted` | Caller signature is preserved. |
| `Submitted` | `accepted` | `Accepted` | Daemon accepted request envelope. |
| `Accepted` | `admitted` | `Admitted` | Runtime admission succeeded. |
| `Admitted` | `dispatched` | `Dispatched` | Callee dispatch selected. |
| `Dispatched` | `running` | `Running` | Ability execution started. |
| `Running` | `completed` | `Completed` | Terminal success with receipt. |
| non-terminal | `failed` | `Failed` | Terminal failure with typed error and receipt if available. |
| non-terminal | `timed_out` | `TimedOut` | Terminal timeout. |
| non-terminal | `cancelled` | `Cancelled` | Terminal cancellation. |

Object-state mapping:

| State | Public object | Meaning |
| --- | --- | --- |
| `EmptyBuilder`, `DraftMutable` | `InvocationBuilder` | Mutable construction only. |
| `DraftImmutable` | `InvocationDraft` | Complete seven-tuple snapshot, still unsigned and not canonical signing material. |
| `Prepared` | `PreparedInvocation` plus `SigningMaterial` | Immutable canonical signing material; not executable and not submit-ready. |
| `Signed` | `SignedInvocation` | Immutable submit-ready envelope carrying caller or daemon-authorized signer proof. |
| `Submitted` through terminal states | `InvocationHandle`, then `InvocationResult` | Runtime observation and terminal projection. |

Invariants:

1. `DraftImmutable` must expose caller, callee, ability descriptor, subject,
   nonce, causal context, and args before prepare/submit.
2. `PreparedInvocation` is immutable and has no public submit operation.
3. `SignedInvocation` is immutable and is the only submit-ready pre-runtime
   object.
4. `submit_signed` must not re-sign or mutate caller signature material.
5. Local daemon signing must be represented as `Prepared -> Signed`, not as
   `Prepared -> Submitted`.
6. Terminal states are monotonic.
7. Receipt chain verification must refer to terminal state and canonical
   invocation material.
8. SDK convenience methods may skip user-visible intermediate objects only if
   tracing/debug APIs can reveal the same states.

### 10.4 Stream State Machine

```text
Opening -> Open -> TerminalFrameSeen -> Draining -> Closed
               |                         |
               v                         v
            Failed                    Cancelled
```

Transitions:

| From | Event | To | Rule |
| --- | --- | --- | --- |
| `Opening` | `stream_ready` | `Open` | First stream metadata accepted. |
| `Open` | `data_frame` | `Open` | Preserve daemon event order. |
| `Open` | `terminal_frame` | `TerminalFrameSeen` | No new data frames may be accepted after this point. |
| `TerminalFrameSeen` | `drain_complete` | `Draining` | Client drains buffered frames. |
| `Draining` | `close` | `Closed` | Terminal receipt available. |
| non-terminal | `cancel` | `Cancelled` | Cancel request sent to daemon/runtime. |
| non-terminal | `transport_error` | `Failed` | Error includes whether resume is possible. |

Invariants:

1. Stream events are ordered per invocation.
2. Terminal frame appears at most once.
3. `Closed`, `Cancelled`, and `Failed` are terminal.
4. Backpressure behavior must be documented per language.
5. Stream queues must be bounded by named constants.

### 10.5 Bidirectional Session State Machine

```text
Created -> Opening -> Open -> HalfClosedLocal -> Terminal -> Closed
                         |          |
                         v          v
                  HalfClosedRemote  Cancelled
                         |
                         v
                      Terminal
```

Transitions:

| From | Event | To | Rule |
| --- | --- | --- | --- |
| `Created` | `send_frame0` | `Opening` | Frame0 contains invocation/session metadata. |
| `Opening` | `session_accepted` | `Open` | Daemon accepted bidi session. |
| `Open` | `send_frame` | `Open` | Frame ordering is preserved. |
| `Open` | `close_send` | `HalfClosedLocal` | Client will not send more frames. |
| `Open` | `remote_close_send` | `HalfClosedRemote` | Remote will not send more frames. |
| `HalfClosedLocal` | `remote_terminal` | `Terminal` | Remote completes. |
| `HalfClosedRemote` | `close_send` | `Terminal` | Both sides closed send path. |
| non-terminal | `cancel` | `Cancelled` | Terminal local cancellation. |
| `Terminal` | `close` | `Closed` | All receipts/events drained. |

Invariants:

1. Frame0 is required and sent exactly once.
2. Local close-send is not the same as cancel.
3. Session terminal receipt must be observable before final close where daemon
   provides it.
4. Dropping a local object must not silently pretend remote terminal success.
5. Send and receive queues must be bounded by named constants.

### 10.6 Directory Subscription State Machine

```text
Opening -> CatchingUp -> Live -> Resuming -> Live
                         |             |
                         v             v
                       Closed        Failed
```

Transitions:

| From | Event | To | Rule |
| --- | --- | --- | --- |
| `Opening` | `snapshot_start` | `CatchingUp` | Initial catalog snapshot begins. |
| `CatchingUp` | `snapshot_complete` | `Live` | Live delta stream starts after snapshot cursor. |
| `Live` | `transport_lost` | `Resuming` | SDK attempts cursor-based resume. |
| `Resuming` | `resume_ok` | `Live` | No duplicate events after resume cursor. |
| `Resuming` | `resume_failed` | `Failed` | Caller must reopen full subscription. |
| any non-terminal | `close` | `Closed` | Caller intentionally closed subscription. |

Invariants:

1. Snapshot events precede live delta events.
2. Resume must not duplicate a committed event id.
3. Directory event types must be stable across languages.
4. Backend read models must be able to rebuild from snapshot plus deltas.

### 10.7 Aggregate Fan-out State Machine

Ordinary SDK list methods do not fan out. When a caller explicitly needs
fleet-wide aggregation, the SDK invokes a named aggregate ability hosted by the
daemon or hub.

```text
Planned -> Dispatching -> Collecting -> Completed
                              |             |
                              v             v
                           Partial       Failed
                              |
                              v
                           TimedOut
```

Transitions:

| From | Event | To | Rule |
| --- | --- | --- | --- |
| `Planned` | `invoke_aggregate_ability` | `Dispatching` | One parent Invocation enters daemon/hub aggregate ability. |
| `Dispatching` | `child_invocations_started` | `Collecting` | Child Invocations carry parent receipt/causal context. |
| `Collecting` | `all_children_terminal` | `Completed` | Result includes child receipt refs. |
| `Collecting` | `some_children_terminal_some_failed` | `Partial` | Partial result includes per-target errors. |
| `Collecting` | `deadline_elapsed` | `TimedOut` | Timeout result includes completed child receipt refs. |
| non-terminal | `aggregate_failed` | `Failed` | Parent receipt records aggregate failure. |

Invariants:

1. Fan-out concurrency is bounded by an explicit daemon-side limit.
2. Fan-out deadline is bounded by the parent Invocation timeout.
3. Partial results are explicit and typed.
4. Each child call is an Invocation with its own receipt.
5. SDK language facades do not run their own governed fan-out loops.

## 11. Daemon Lifecycle API

Lifecycle APIs manage or attach to a local `easynet-daemon` process. They do not
perform ability dispatch.

Required Go shape:

```go
type Mode string

const (
    ModeDevice Mode = "device"
    ModeHub    Mode = "hub"
    ModeBoth   Mode = "both"
)

type StartConfig struct {
    Mode        Mode
    Realm       string
    DeviceID    string // local daemon/device config label, not Invocation callee
    HomeDir     string
    DaemonBin   string
    LogPath     string
    Detached    bool
    Env         map[string]string

    UDSPath     string
    ListenTCP   string
    TLSCertPath string
    TLSKeyPath  string
    HubEndpoint string
    TrustPath   string
}

type Endpoints struct {
    ControlEndpoint    string
    InvocationEndpoint string
    PublicEndpoint     string
}

type Handle interface {
    Status(ctx context.Context) (DaemonStatus, error)
    Endpoints() Endpoints
    OpenClient(ctx context.Context, opts runtime.ConnectOptions) (*runtime.Client, error)
    Stop(ctx context.Context) error
    Detach() error
}

func Start(ctx context.Context, cfg StartConfig) (Handle, error)
func Attach(ctx context.Context, opts AttachOptions) (Handle, error)
func Discover(ctx context.Context, opts DiscoverOptions) (Endpoints, error)
func ConnectLocal(ctx context.Context, opts runtime.ConnectOptions) (*runtime.Client, error)
```

Requirements:

1. `Start` MUST return only after both control and Invocation endpoints are
   accepting connections.
2. `Attach` MUST fail closed when a control endpoint is alive but the Invocation
   endpoint is down.
3. `Discover` MUST prefer the daemon-advertised `invocation_endpoint` over
   hard-coded `~/.easynet/daemon.sock`.
4. `ModeDevice` MUST NOT accept a public TCP listener.
5. Hub or both mode with `ListenTCP` MUST require TLS material or an explicit
   daemon-side provisioning path.
6. `Detach` MUST NOT stop the daemon.
7. `Stop` MUST be idempotent and authority-checked.

## 12. Runtime Client API

Runtime APIs submit complete Invocations to the daemon.

Required Go shape:

```go
type Client struct{}

type ConnectOptions struct {
    Endpoint        string
    ControlPath     string
    DialTimeout     time.Duration
    InvokeTimeout   time.Duration
    MaxMessageBytes int
    Signer          identity.Signer
    Reconnect       bool
}

func Connect(ctx context.Context, opts ConnectOptions) (*Client, error)
func (c *Client) Health(ctx context.Context) (RuntimeHealth, error)
func (c *Client) NewInvocation() *invocation.Builder
func (c *Client) Invoke(ctx context.Context, inv invocation.Invocation) (invocation.Result, error)
func (c *Client) Prepare(ctx context.Context, draft invocation.Draft, opts invocation.PrepareOptions) (invocation.PreparedInvocation, invocation.SigningMaterial, error)
func (c *Client) SubmitSigned(ctx context.Context, signed invocation.SignedInvocation) (invocation.Handle, error)
func (c *Client) InvokeStream(ctx context.Context, inv invocation.Invocation) (session.Stream, error)
func (c *Client) OpenBidi(ctx context.Context, inv invocation.Invocation, streams []session.StreamDescriptor) (session.Bidi, error)
func (c *Client) Close() error
```

Requirements:

1. `Client` MUST hide gRPC, UDS, generated protocol types, reconnect state, and
   frame-zero construction.
2. `Invoke` MUST return an SDK `invocation.Result`, not raw proto.
3. `InvokeStream` MUST expose SDK `session.Stream` events and terminal state,
   not raw `InvokeStreamChunk`.
4. `OpenBidi` MUST send a correct frame0 before returning the session.
5. Reconnecting clients MUST invalidate only transport-broken connections, not
   application-level invocation failures.
6. Reconnect MUST NOT replay non-idempotent invocations unless replay policy and
   idempotency key are explicit in the Invocation.

## 13. Invocation API

The Invocation package is the SDK's load-bearing contract.

Required Go shape:

```go
type URA string
type DescriptorRef string // canonical ability_ura@descriptor_version obtained from Axon helper

type Invocation struct {
    CallerURA     URA
    CalleeURA     URA
    DescriptorRef DescriptorRef
    SubjectURA    URA
    Nonce         [16]byte
    Causal        CausalContext
    Args          Payload
    ContentType   string

    Metadata      map[string]string
    Authority     AuthorityProof
    CallerSig     *CallerSignature
    Timeout       time.Duration
    Idempotency   *IdempotencyPolicy
}

type Payload struct {
    JSON        any
    Raw         []byte
    ContentType string
}

type Builder struct{}

func New(caller, callee URA, descriptor DescriptorRef, subject URA) *Builder
func (b *Builder) JSON(v any) *Builder
func (b *Builder) Bytes(contentType string, raw []byte) *Builder
func (b *Builder) Nonce(n [16]byte) *Builder
func (b *Builder) FreshNonce() *Builder
func (b *Builder) CausalNone(reason string) *Builder
func (b *Builder) CausalScalar(ref ReceiptRef) *Builder
func (b *Builder) CausalVector(refs []ReceiptRef) *Builder
func (b *Builder) CausalMerkle(root [32]byte, proofURA URA) *Builder
func (b *Builder) Authority(a AuthorityProof) *Builder
func (b *Builder) CallerSignature(sig CallerSignature) *Builder
func (b *Builder) Timeout(d time.Duration) *Builder
func (b *Builder) Idempotency(policy IdempotencyPolicy) *Builder
func (b *Builder) Metadata(k, v string) *Builder
func (b *Builder) Inspect() Draft
func (b *Builder) Build() (Invocation, error)
```

Requirements:

1. `Build` MUST reject empty caller, callee, descriptor, subject, args content
   type, invalid nonce length, and ambiguous payload forms.
2. `DescriptorRef` MUST bind ability identity and descriptor version and MUST
   be obtained or validated through Axon `canonical_ability_descriptor_ref`
   equivalent behavior.
3. `Payload` MUST support JSON and raw bytes without re-encoding raw payloads.
4. SDKs MUST NOT publish a public `InvokeAbility(name, args)` API that bypasses
   caller/callee/subject/nonce/causal visibility.
5. `Inspect` MUST show the full draft tuple and defaults that have already been
   filled.

## 14. Prepare, Sign, Submit

This API supports browser/user signing flows and backend-mediated signed submit.

Required Go shape:

```go
type Draft struct {
    CallerURA     URA
    CalleeURA     URA
    DescriptorRef DescriptorRef
    SubjectURA    URA
    Args          Payload
    Causal        CausalContext
    Metadata      map[string]string
    Timeout       time.Duration
    Idempotency   *IdempotencyPolicy
}

type PrepareOptions struct {
    ExpiresIn          time.Duration
    SignerID           string
    PolicyRef          string
    LocalDaemonSigning bool
}

type PreparedInvocation struct {
    Invocation       Invocation
    RequestID        string
    DescriptorRef    DescriptorRef
    DescriptorHash   string
    SchemaHash       string
    CanonicalHashHex string
    ExpiresAt        time.Time
}

type SigningMaterial struct {
    CanonicalBytes []byte
    ArgsDigestHex  string
    NonceBase64    string
    SignedFields   []string
    SignerPolicy   SignerPolicy
}

type SignerPolicy struct {
    Mode        string // caller_signing | local_daemon_signing
    SignerID    string
    PolicyRef   string
    ExpiresAt   time.Time
}

type SignedInvocation struct {
    Prepared  PreparedInvocation
    Signature CallerSignature
    SignerID  string
    Policy    SignerPolicy
}

func (c *Client) Prepare(ctx context.Context, draft invocation.Draft, opts invocation.PrepareOptions) (invocation.PreparedInvocation, invocation.SigningMaterial, error)
func (c *Client) SignPrepared(ctx context.Context, prepared invocation.PreparedInvocation, signer identity.Signer) (invocation.SignedInvocation, error)
func (c *Client) SubmitSigned(ctx context.Context, signed invocation.SignedInvocation) (invocation.Handle, error)
```

Requirements:

1. Canonical bytes MUST be obtained through the daemon-owned Axon adapter or an
   Axon helper path exposed to the CLI SDK core; product code and language
   facades MUST NOT implement canonicalization.
2. `SigningMaterial` MUST be stable across Go, Python, Rust, and C ABI bindings
   for the same Invocation.
3. `PreparedInvocation` is canonical signing material only. It MUST NOT expose a
   public submit method and MUST NOT be accepted by submit APIs.
4. Local daemon signing, when policy allows it, MUST produce a
   `SignedInvocation` with signer id and policy proof before submission.
5. `SubmitSigned` MUST preserve the caller's signature and public key material;
   it MUST NOT replace a user signature with the backend's hub key.
6. Prepared invocations MUST expire or carry enough metadata for callers to
   reject stale browser signatures.
7. `DescriptorHash`, `SchemaHash`, and `CanonicalHashHex` MUST be stable enough
   for callers to audit which descriptor version and canonical bytes were
   signed.

## 15. Authority API

The SDK MUST expose authority metadata as typed values.

Required Go shape:

```go
type Signer interface {
    PublicKey() []byte
    Sign(ctx context.Context, msg []byte) ([]byte, error)
}

type SessionAuthority struct {
    IssuerURA   invocation.URA
    SubjectURA  invocation.URA
    AudienceURA invocation.URA
    Scopes      []string
    ExpiresAt   time.Time
    Signature   []byte
}

type DelegationProof struct {
    IssuerURA   invocation.URA
    SubjectURA  invocation.URA
    AudienceURA invocation.URA
    Scopes      []string
    ExpiresAt   time.Time
    Signature   []byte
}

func NewSessionAuthority(ctx context.Context, signer Signer, req SessionAuthorityRequest) (SessionAuthority, error)
func NewDelegationProof(ctx context.Context, signer Signer, req DelegationRequest) (DelegationProof, error)
```

Requirements:

1. `SessionAuthority` and `DelegationProof` MUST be mutually exclusive on one
   Invocation.
2. Authority metadata MUST bind issuer, subject, audience, scopes, expiry, and
   signature.
3. Builder APIs MUST reject ambiguous authority.
4. AuthorityBinding must govern both advertisement and invocation permission.

## 16. Directory, Catalog, and Listing API

Directory APIs project daemon read models. They may invoke daemon abilities such
as `namespace.resolve`, `namespace.proxy_resolve`, `federation.resolve`, or
`federation.list_user_devices` internally, but product consumers do not need to
know those carrier names.

Required Go shape:

```go
type Client struct { Runtime *runtime.Client }

func (d *Client) Resolve(ctx context.Context, q ResolveQuery) (ResolveAnswer, error)
func (d *Client) ListDevices(ctx context.Context, q DeviceQuery) (Page[Device], error)
func (d *Client) ListAgents(ctx context.Context, q AgentQuery) (Page[Agent], error)
func (d *Client) ListAbilities(ctx context.Context, q AbilityQuery) (Page[AbilityDescriptor], error)
func (d *Client) Subscribe(ctx context.Context, f DirectoryFilter) (DirectoryStream, error)
```

Requirements:

1. Directory results MUST expose URAs for owners, devices, agents, and
   abilities.
2. Directory errors MUST use typed SDK errors, not human string parsing.
3. Event subscriptions MUST expose SDK event DTOs, not raw federation proto.
4. Every list method MUST support cursor pagination.
5. Every list method MUST define `DefaultPageSize` and `MaxPageSize` as named
   constants in each implementation.
6. A list method returning all rows without pagination is forbidden at public
   SDK boundaries.

### 16.1 Complexity Contract

Ordinary list methods are read-model queries, not distributed fan-out.

| Method | Required data source | Complexity target | Forbidden implementation |
| --- | --- | --- | --- |
| `ListDevices` | daemon/hub directory read model indexed by realm/user/device | `O(page_size + filter_cost)` | dialing every device |
| `ListAgents` | local hosted-agent registry or hub directory projection | `O(page_size + filter_cost)` | calling each agent |
| `ListAbilities` | ability catalog projection keyed by owner/descriptor/filter | `O(page_size + filter_cost)` | calling `meta.list_abilities` on every agent by default |
| `Resolve` | indexed directory/namespace lookup | `O(log n + result_count)` or documented equivalent | scanning all devices for exact URA lookup |
| `Subscribe` | snapshot cursor plus delta log | `O(snapshot_page + delta_count)` per page | replaying full catalog for each delta |

`filter_cost` must be bounded by indexed predicates or by a documented scan over
a local in-memory snapshot. It must not hide network fan-out.

### 16.2 Fan-out and Facade Rules

1. Language SDK facades MUST NOT fan out governed ability calls across devices,
   agents, or abilities on their own.
2. Fleet-wide aggregation belongs in a named daemon/hub aggregate ability, such
   as `aggregate.list_abilities_catalog`.
3. Aggregate abilities MUST create child Invocations for each governed child
   call.
4. Aggregate abilities MUST expose max concurrency, deadline, page size,
   partial-result semantics, and per-target error summaries.
5. SDK facade methods MAY call one aggregate ability. That is still one parent
   Invocation from the SDK's point of view.
6. Facade helpers MUST be named to reveal aggregation, for example
   `AggregateAbilities`, not `ListAbilities`.
7. A default `ListAbilities` call MUST return the catalog projection already
   known to the daemon/hub, not trigger live remote discovery.

## 17. Identity API

The SDK MUST expose identity helpers needed by daemon clients and product
backends while keeping keyring policy daemon-owned.

Required capabilities:

1. Load or initialize local Hub identity for a realm.
2. Load device identity from daemon/keyring context.
3. Construct URAs through SDK-owned builders.
4. Register, list, and revoke user signing keys through daemon identity
   abilities.
5. Provide `Signer` implementations for local Ed25519 identities.

The SDK MUST NOT expose backend JWT/OAuth/session concepts.

## 18. Stream, Bidi, and Session API

SDK sessions hide raw daemon frame details.

Required Go shape:

```go
type Stream interface {
    Recv(ctx context.Context) (StreamEvent, error)
    Close() error
    Cancel(ctx context.Context) error
}

type Bidi interface {
    Send(ctx context.Context, frame UpFrame) error
    Recv(ctx context.Context) (DownFrame, error)
    CloseSend() error
    Close() error
    Cancel(ctx context.Context) error
}
```

Requirements:

1. Streams MUST make terminal state explicit.
2. Bidi sessions MUST hide frame0 construction and sequence numbering.
3. EOF, terminal receipt, cleanup receipt, timeout, and transport cancellation
   MUST be distinguishable.
4. Long-lived streams MUST support caller cancellation through context and an
   explicit close/cancel method.
5. Stream and bidi queues MUST be bounded by named constants.

## 19. Convenience Wrappers

These wrappers are convenience APIs over complete Invocation, stream, and bidi
sessions. They MUST NOT be the only way to reach those abilities. A Hub may use
them for HTTP/WebSocket bridges, but the wrapper remains SDK-owned and the
browser route remains backend-owned.

Required capabilities:

```go
func OpenFileTransfer(ctx context.Context, c *runtime.Client, req FileTransferRequest) (FileTransfer, error)
func OpenTerminal(ctx context.Context, c *runtime.Client, req TerminalRequest) (TerminalSession, error)
func OpenRemoteDesktop(ctx context.Context, c *runtime.Client, req RemoteDesktopRequest) (RemoteDesktopSession, error)
func OpenBrowserSession(ctx context.Context, c *runtime.Client, req BrowserSessionRequest) (BrowserSession, error)
func OpenMediaSession(ctx context.Context, c *runtime.Client, req MediaSessionRequest) (MediaSession, error)
```

Requirements:

1. Wrappers MUST still produce a complete Invocation internally.
2. Wrappers MUST surface admission failure, routing failure, terminal failure,
   timeout, and client cancellation as typed errors.
3. File wrappers MUST cover normal file transfer and context-file upload
   carriers without making the backend parse raw daemon receipts.
4. Terminal, remote desktop, browser, and media wrappers MUST preserve stream
   and bidi terminal-state distinctions from section 18.
5. Backend HTTP/WS bridges MAY consume these wrappers, but wrappers MUST NOT
   import backend packages.

## 20. Publication, Host Binding, Mission, Admin, and Hub Profiles

These profiles primarily serve CLI, desktop apps, local host apps, and
EasyRemote, plus the EasyNet backend/Hub. They are part of a complete Daemon SDK
because product facades must not be forced to reimplement daemon system ability
carriers, host-stream wire contracts, daemon admin transport, Hub event streams,
surface/page carriers, or compatibility adapters.

### 20.1 Publication Profile

The Publication profile is the SDK surface for turning an implementation
resource into a daemon-governed AbilityImpl bound to an AbilityDescriptor. It is
not Python function introspection, not decorators, and not the host process
runtime; those remain EasyRemote product concerns.

Required Go shape:

```go
type PublicationClient struct { Runtime *runtime.Client }

func (p *PublicationClient) BuildLocalResourceRef(ctx context.Context, req LocalResourceRefRequest) (ResourceRef, error)
func (p *PublicationClient) ValidatePackage(ctx context.Context, path string, opts ValidatePackageOptions) (PackageValidation, error)
func (p *PublicationClient) DeployAbility(ctx context.Context, req AbilityDeployRequest) (AbilityDeployResult, error)
func (p *PublicationClient) InstallPlugin(ctx context.Context, source string, opts InstallOptions) (PluginInstallResult, error)
func (p *PublicationClient) ListAbilities(ctx context.Context, q PublishedAbilityQuery) (Page[PublishedAbility], error)
func (p *PublicationClient) ShowAbility(ctx context.Context, ref DescriptorRef) (PublishedAbility, error)
func (p *PublicationClient) EnableAbilityImpl(ctx context.Context, id AbilityImplID) error
func (p *PublicationClient) DisableAbilityImpl(ctx context.Context, id AbilityImplID) error
func (p *PublicationClient) UnpublishAbility(ctx context.Context, ref DescriptorRef) error
```

Required DTOs:

```text
LocalResourceRefRequest
ResourceRef
AbilityPackageManifest
AbilityDeployRequest
AbilityDeployResult
PublishedAbility
AbilityImplID
HostStreamBinding
PluginInstallResult
PackageValidation
```

Requirements:

1. Publication APIs MUST submit complete Invocations to daemon-owned system
   abilities such as `ability.deploy`, `ability.publish`, `ability.unpublish`,
   plugin install, or equivalent descriptor-bound replacements.
2. `ResourceRef` MUST be SDK/daemon-authored, not hand-built by product
   facades. Local filesystem refs must bind owner/device URA, path, capability,
   TTL, and revision.
3. `AbilityDescriptor` fields returned by publication APIs MUST include
   descriptor version, schema hash, owner URA, execution mode, policy, and
   observability metadata.
4. `AbilityImpl` fields returned by publication APIs MUST include implementation
   id, implementation hash, runtime environment, package/source reference, and
   enabled/disabled state.
5. Host-stream publication MUST be represented as a generic binding contract:
   socket endpoint, frame schema, lifecycle, cleanup, timeout, and readiness.
   The SDK MUST NOT own Python `ComputeNode`, `@node.register`, decorators,
   function signature introspection, or the warm Python host process.
6. Plugin, skill, and host-process management belongs to
   implementation-resource management, not protocol ability identity.
7. Publication list/show methods MUST use paginated daemon read models or named
   aggregate abilities. They MUST NOT scan local package directories or fan out
   to every device by default.

### 20.2 Host Binding Profile

The Host Binding profile is the SDK surface for the daemon-to-host execution
binding used by EasyRemote-style warm host processes. It is not a Python
function runner and not a decorator API. It owns the shared wire contract that
lets a product host serve one AbilityImpl without redefining stream, terminal,
or hash semantics.

Required Go shape:

```go
type HostBindingClient struct { Runtime *runtime.Client }

func (h *HostBindingClient) BuildHostStreamBinding(ctx context.Context, req HostStreamBindingRequest) (HostStreamBinding, error)
func (h *HostBindingClient) DecodeRequest(ctx context.Context, envelope HostStreamEnvelope) (HostStreamRequest, error)
func (h *HostBindingClient) EncodeItem(ctx context.Context, seq uint64, value any) (HostStreamFrame, error)
func (h *HostBindingClient) EncodeError(ctx context.Context, err error) (HostStreamFrame, error)
func (h *HostBindingClient) EncodeTerminal(ctx context.Context, summary HostStreamTerminalSummary) (HostStreamFrame, error)
func (h *HostBindingClient) FoldOutputHash(ctx context.Context, state HostStreamHashState, seq uint64, value any) (HostStreamHashState, error)
```

Required DTOs:

```text
HostStreamBindingRequest
HostStreamBinding
HostStreamEnvelope
HostStreamRequest
HostStreamFrame
HostStreamTerminalSummary
HostStreamHashState
```

Requirements:

1. The profile MUST define item, error, and terminal frame schemas shared by
   every language.
2. Output-hash folding MUST be schema-backed and fixture-tested so a host and
   daemon compute the same digest for the same stream.
3. Host readiness, cleanup, timeout, and endpoint ownership MUST be represented
   as typed DTOs.
4. Product hosts MAY own function/class introspection, dependency loading,
   threads, process warmth, and argument binding.
5. Product hosts MUST NOT own the canonical host-stream frame semantics or
   terminal-state mapping.

### 20.3 Mission Profile

The Mission profile is the SDK surface for submitting and observing daemon-owned
Mission/EAL execution. Python `Pipeline` and other product DSLs may compile to
EAL, but they must call this profile rather than owning a daemon transport
facade.

Required Go shape:

```go
type MissionClient struct { Runtime *runtime.Client }

func (m *MissionClient) RunEAL(ctx context.Context, req MissionRunRequest) (MissionRun, error)
func (m *MissionClient) RunFile(ctx context.Context, path string, opts MissionRunOptions) (MissionRun, error)
func (m *MissionClient) Track(ctx context.Context, id MissionID) (MissionStatus, error)
func (m *MissionClient) Cancel(ctx context.Context, id MissionID) (MissionCancelResult, error)
func (m *MissionClient) Events(ctx context.Context, id MissionID, opts MissionEventOptions) (MissionEventStream, error)
```

Requirements:

1. Mission/EAL helpers MUST create child Invocations for ability calls rather
   than redefining Invocation semantics.
2. Mission status MUST expose parent invocation id, parent receipt URA when
   available, child receipt refs, terminal state, partial failures, cancellation
   state, and output artifact refs.
3. Product DSLs such as EasyRemote `Pipeline` may own syntax and compilation,
   but the SDK owns the daemon submission, typed status, typed errors, and event
   stream.
4. Composite ability orchestration belongs in Mission/EAL or daemon-owned
   aggregate abilities unless the flow is trivial, latency-critical, or
   session-oriented.

### 20.4 Admin + Gateway Profile

Hub join/leave, gateway status, agent lifecycle, and device administration are
daemon product lifecycle helpers. They are optional for a narrow caller-only
SDK, but required for full EasyRemote repository cutover because EasyRemote
ships `Server` and `AgentControl` facades.

```go
type AdminClient struct { Runtime *runtime.Client }

func (a *AdminClient) GatewayStatus(ctx context.Context, req GatewayStatusRequest) (GatewayStatus, error)
func (a *AdminClient) AgentStart(ctx context.Context, req AgentStartRequest) (AgentStartResult, error)
func (a *AdminClient) ListAgents(ctx context.Context, q AgentQuery) (Page[AgentRecord], error)
func (a *AdminClient) RefreshAgent(ctx context.Context, req AgentRefreshRequest) (AgentRefreshResult, error)
func (a *AdminClient) JoinHub(ctx context.Context, req JoinRequest) (JoinResult, error)
func (a *AdminClient) LeaveHub(ctx context.Context, req LeaveRequest) (LeaveResult, error)
func (a *AdminClient) PairingPreflight(ctx context.Context, req PairingPreflightRequest) (PairingPreflight, error)
func (a *AdminClient) ValidatePairing(ctx context.Context, req ValidatePairingRequest) (DeviceCredential, error)
func (a *AdminClient) VerifyDeviceCredential(ctx context.Context, req VerifyDeviceCredentialRequest) (VerificationResult, error)
func (a *AdminClient) CreatePairing(ctx context.Context, req CreatePairingRequest) (PairingToken, error)
func (a *AdminClient) RevokeDevice(ctx context.Context, req RevokeDeviceRequest) (DeviceAdminResult, error)
func (a *AdminClient) CreateDeviceSession(ctx context.Context, req CreateDeviceSessionRequest) (DeviceSession, error)
func (a *AdminClient) ListDeviceSessions(ctx context.Context, q DeviceSessionQuery) (Page[DeviceSession], error)
func (a *AdminClient) DeleteDeviceSession(ctx context.Context, req DeleteDeviceSessionRequest) (DeviceAdminResult, error)
```

Required DTOs:

```text
GatewayStatusRequest
GatewayStatus
AgentStartRequest
AgentStartResult
AgentQuery
AgentRecord
PairingPreflightRequest
PairingPreflight
ValidatePairingRequest
DeviceCredential
VerifyDeviceCredentialRequest
CreatePairingRequest
PairingToken
DeviceSession
DeviceSessionQuery
DeviceAdminResult
```

Requirements:

1. Admin operations MUST use daemon-owned system abilities or lifecycle APIs
   through SDK DTOs, not product-local transport code.
2. `gateway_status` MUST distinguish daemon process liveness, public listener
   readiness, TLS/trust readiness, and directory/runtime readiness.
3. Agent lifecycle DTOs MUST model daemon-owned agent records. They MUST NOT
   become EasyRemote-specific Python objects.
4. Pairing, credential verification, and device session lifecycle DTOs MUST
   model daemon/Hub trust state and device runtime sessions without importing
   backend account tables or browser session DTOs.
5. Certificate provisioning, ACME/self-signed policy, pairing guidance text, and
   onboarding UX MAY remain product facade concerns as long as they call SDK
   admin and lifecycle APIs.

### 20.5 Events Profile

The Events profile is the SDK surface for daemon-originated live state changes
that a Hub needs to expose through SSE, WebSocket, polling fallback, or internal
backend projections. It owns daemon event subscription semantics; it does not
own the backend subscriber registry or browser authorization.

Required Go shape:

```go
type EventClient struct { Runtime *runtime.Client }

func (e *EventClient) SubscribeDirectory(ctx context.Context, q DirectoryEventQuery) (EventStream[DirectoryEvent], error)
func (e *EventClient) SubscribeDevices(ctx context.Context, q DeviceEventQuery) (EventStream[DeviceEvent], error)
func (e *EventClient) SubscribeSessions(ctx context.Context, q SessionEventQuery) (EventStream[SessionEvent], error)
func (e *EventClient) SubscribeInvocations(ctx context.Context, q InvocationEventQuery) (EventStream[InvocationEvent], error)
func (e *EventClient) ListDeviceEvents(ctx context.Context, q DeviceEventQuery) (Page[DeviceEvent], error)
```

Required DTOs:

```text
EventStream
EventCursor
EventResumeToken
DirectoryEvent
DeviceEvent
SessionEvent
InvocationEvent
EventDropReport
```

Requirements:

1. Event streams MUST define snapshot, live delta, resume cursor, terminal,
   reconnect hint, and dropped-event reporting semantics.
2. Events MUST preserve tenant/realm/device/agent references as SDK URA or typed
   ref DTOs, not backend row ids as canonical identity.
3. Backend SSE/WS fanout MAY filter and authorize events, but it MUST consume
   SDK `EventClient` streams after cutover.
4. Polling MAY remain as fallback, but it is not complete live Hub coverage by
   itself.

### 20.6 Surface Profile

The Surface profile is the SDK surface for daemon-governed pages and embedded
surfaces such as public page manifests. It lets the backend render and route
pages without owning page daemon transports or treating static product data as
runtime truth.

Required Go shape:

```go
type SurfaceClient struct { Runtime *runtime.Client }

func (s *SurfaceClient) ListPages(ctx context.Context, q PageQuery) (Page[PageRecord], error)
func (s *SurfaceClient) CreatePage(ctx context.Context, req CreatePageRequest) (PageRecord, error)
func (s *SurfaceClient) DeletePage(ctx context.Context, req DeletePageRequest) (SurfaceMutationResult, error)
func (s *SurfaceClient) SurfaceManifest(ctx context.Context, req SurfaceManifestRequest) (SurfaceManifest, error)
func (s *SurfaceClient) PublicPageRef(ctx context.Context, req PublicPageRefRequest) (PublicPageRef, error)
```

Required DTOs:

```text
PageRecord
PageQuery
CreatePageRequest
DeletePageRequest
SurfaceManifest
SurfaceManifestRequest
PublicPageRef
SurfaceMutationResult
SurfaceHealth
```

Requirements:

1. Page and surface methods MUST lower to complete Invocations or daemon-owned
   read models and preserve descriptor, owner, realm, version, and health.
2. Public page refs MUST be typed refs. Backends may turn them into HTTP URLs,
   but HTTP routes are not daemon identity.
3. Backend rendering, browser authentication, cache/CDN policy, and frontend
   component state remain product concerns.
4. Backend code MUST NOT invoke page/surface daemon system abilities manually
   once `SurfaceClient` is cutover-ready.

### 20.7 Compatibility Profile

The Compatibility profile is the SDK surface for product APIs that intentionally
imitate external protocols, such as OpenAI-compatible chat, model, and file
endpoints. The profile maps those requests to governed daemon abilities and SDK
file/directory/runtime DTOs without making the external protocol the daemon
protocol.

Required Go shape:

```go
type CompatibilityClient struct { Runtime *runtime.Client }

func (c *CompatibilityClient) ListModels(ctx context.Context, req CompatibilityModelRequest) (CompatibilityModelPage, error)
func (c *CompatibilityClient) ChatCompletions(ctx context.Context, req CompatibilityChatRequest) (CompatibilityCompletion, error)
func (c *CompatibilityClient) StreamChatCompletions(ctx context.Context, req CompatibilityChatRequest) (StreamHandle, error)
func (c *CompatibilityClient) UploadFile(ctx context.Context, req CompatibilityFileUploadRequest) (CompatibilityFile, error)
func (c *CompatibilityClient) GetFile(ctx context.Context, req CompatibilityFileRequest) (CompatibilityFile, error)
func (c *CompatibilityClient) DeleteFile(ctx context.Context, req CompatibilityFileRequest) (CompatibilityDeleteResult, error)
```

Required DTOs:

```text
CompatibilityModelRequest
CompatibilityModelPage
CompatibilityChatRequest
CompatibilityCompletion
CompatibilityFileUploadRequest
CompatibilityFileRequest
CompatibilityFile
CompatibilityDeleteResult
CompatibilityAdapterError
```

Requirements:

1. Compatibility methods MUST lower to complete Invocations, file wrappers, or
   directory reads and expose terminal receipts/errors when available.
2. External DTOs MAY be projected for product HTTP compatibility, but SDK DTOs
   remain the internal boundary to the daemon.
3. Backend API keys, billing, rate limits, CORS, and HTTP response formatting
   remain backend-owned.
4. Backend code MUST NOT maintain raw ability+args OpenAI compatibility shims
   after `CompatibilityClient` is cutover-ready.

## 21. Health and Diagnostics API

SDK health MUST separate product process liveness from runtime readiness.

Required shape:

```go
type RuntimeHealth struct {
    DaemonVersion       string
    SDKVersion          string
    Mode                daemon.Mode
    Realm               string
    PID                 int
    ControlEndpoint     string
    InvocationEndpoint  string
    PublicEndpoint      string
    ControlReady        bool
    InvocationReady     bool
    PublicListenerReady bool
    TrustReady          bool
    DirectoryReady      bool
    RuntimeReady        bool
    LastError           *RuntimeError
}
```

Requirements:

1. Local health MUST check the Invocation endpoint, not only control discovery.
2. Hub health SHOULD check public TCP+TLS listener readiness when configured.
3. Health MUST expose typed readiness flags so product backends can distinguish
   "API alive but runtime unavailable" from successful runtime readiness.
4. `RuntimeReady=false` with `ControlReady=true` MUST be possible and visible.

## 22. Error Model

All bindings MUST map daemon, transport, admission, timeout, cancellation, and
protocol failures into typed errors.

Required Go shape:

```go
type ErrorCode string

const (
    ErrDaemonOffline     ErrorCode = "DAEMON_OFFLINE"
    ErrPermissionDenied  ErrorCode = "PERMISSION_DENIED"
    ErrAdmissionDenied   ErrorCode = "ADMISSION_DENIED"
    ErrAbilityNotFound   ErrorCode = "ABILITY_NOT_FOUND"
    ErrRouteUnavailable  ErrorCode = "ROUTE_UNAVAILABLE"
    ErrTimeout           ErrorCode = "TIMEOUT"
    ErrCancelled         ErrorCode = "CANCELLED"
    ErrInvalidInvocation ErrorCode = "INVALID_INVOCATION"
    ErrProtocolMismatch  ErrorCode = "PROTOCOL_MISMATCH"
    ErrVersionMismatch   ErrorCode = "VERSION_MISMATCH"
    ErrControlOnly       ErrorCode = "CONTROL_ONLY"
)

type RuntimeError struct {
    Code       ErrorCode
    Stage      string
    Message    string
    Retryable  bool
    ReceiptURA string
    Cause      error
}
```

Requirements:

1. Product consumers MUST NOT parse human-readable daemon error strings.
2. Errors SHOULD preserve receipt URA or invocation id when the daemon provides
   one.
3. Retriability MUST be explicit; do not infer it from string fragments.
4. Error stages MUST align with the state machines in section 10.
5. Until `docs/rfc/AXON-RFC-007-receipt-ura-builder-agenda-2026-06-12.md`
   is resolved and the Axon/daemon receipt URA builder lands, SDK `ReceiptURA`
   fields are opaque strings returned by daemon/Axon paths. SDK facades MUST NOT
   construct receipt URAs or treat any local pattern as canonical.

## 23. Wire JSON Schemas

The C ABI, Python SDK, and any binding that does not use generated Go/Rust types
MUST accept the same Invocation JSON shape:

This section describes the SDK Invocation JSON v4 projection. It is derived
from Axon canonical Invocation/URA/DescriptorRef helpers and MUST be generated
or validated against those helpers. ABI v3 `ability`-name JSON is retired:
SDK and FFI adapters MUST reject it instead of canonicalizing it as a
compatibility input. Product consumers must supply the v4 projection directly
or obtain a descriptor ref from the identity/addressing helpers before building
an invocation.

```json
{
  "caller_ura": "easynet:///r/example/agent/alice.sdk",
  "callee_ura": "easynet:///r/example/device/dev-a",
  "descriptor_ref": "easynet:///r/example/ability/device.dev-a.observe.health@1.0.0",
  "subject_ura": "easynet:///r/example/device/dev-a",
  "nonce_base64": "AQIDBAUGBwgJCgsMDQ4PEA==",
  "causal_context": {"form": "none"},
  "args": {},
  "content_type": "application/json",
  "metadata": {},
  "caller_signature": {
    "algorithm": "ed25519",
    "signature_base64": "...",
    "signer_public_key_base64": "..."
  }
}
```

For non-JSON payloads, callers pass `arguments_base64` and `content_type`
instead of `args`.

The SDK MUST reject:

1. Both `args` and `arguments_base64`.
2. Neither `args` nor `arguments_base64`.
3. Empty tuple fields.
4. Any URA field that fails Axon `parse_ura` for its declared role.
5. Missing descriptor version in descriptor-bound calls.
6. Any descriptor ref rejected by Axon `canonical_ability_descriptor_ref`.
7. Non-16-byte nonces.
8. Ambiguous authority metadata.
9. Unknown state-machine terminal names.
10. Page requests above `MaxPageSize`.

Normative URA/DescriptorRef example source:

1. Agent URAs follow Axon `core/ura-rs` grammar:
   `easynet:///r/<realm>/agent/<user-id>.<agent-id>`.
2. Device-owned ability URAs follow the top-level ability role:
   `easynet:///r/<realm>/ability/device.<device-id>.<namespace>.<ability-id>`.
3. Descriptor refs are canonicalized by Axon helpers equivalent to
   `canonical_ability_descriptor_ref`; SDK facades MUST NOT hand-build the
   `@descriptor_version` string.

## 24. Transport Strategy by Language

The public API is stable even when transport implementation changes.

| Target | Default transport | Allowed fallback | Forbidden exposure |
| --- | --- | --- | --- |
| Rust | native daemon SDK core plus daemon invocation UDS client | direct in-process test adapter | public Axon client/proto types in stable product API |
| C ABI | opaque handles over the Rust SDK core | none | raw Rust/Axon pointers or generated proto structs |
| Go | thin daemon local-transport facade over generated schemas | C ABI for canonical/signing helper calls if needed | `easynet.run/axon/*` imports in public packages or reimplemented Axon algorithms |
| Python | C ABI bridge over Rust SDK core | native daemon transport only for request marshalling | CLI text scraping or reimplemented Axon algorithms |
| Node / TS | N-API/C ABI bridge or thin daemon transport facade | child-process CLI only for dev diagnostics | shelling out for normal invocation or reimplemented Axon algorithms |
| Java / JVM | JNI/C ABI bridge or thin daemon transport facade | none for normal invocation | generated Axon protobufs in public API or reimplemented Axon algorithms |
| Swift | Swift package over C ABI or thin daemon transport facade | none for normal invocation | Axon symbols in public API or reimplemented Axon algorithms |

Command-line binaries are consumers of the SDK. The SDK MUST NOT be a wrapper
around terminal text output.

For this table, "thin daemon transport facade" means request marshalling,
connection lifecycle, retry classification, and idiomatic async adaptation. It
does not include canonical Invocation material generation, Axon receipt-chain
verification, admission semantics, or Axon stream/bidi state machines. Those
semantics are delegated to Axon through the native Rust daemon SDK core or the
daemon runtime adapter.

## 25. Language Capability Parity

Capability parity is measured by behavior, not by identical function names.
In the table below, `yes` means the target language surface must support the
capability before that language is marked stable; it does not mean the current
repository already implements it.

| Module | Rust | C ABI | Go | Python | Node/TS | Java/JVM | Swift | Requirement |
| --- | --- | --- | --- | --- | --- | --- | --- | --- |
| ABI/version discovery | yes | yes | yes | yes | yes | yes | yes | P0 for Rust/C/Go/Python; P1 for others |
| Daemon lifecycle attach/start/stop | yes | yes | yes | yes | yes | yes | yes | P0 for Rust/C/Go/Python |
| Runtime connect over UDS | yes | yes | yes | yes | yes | yes | yes | P0 for Rust/C/Go/Python |
| Complete Invocation builder | yes | yes | yes | yes | yes | yes | yes | P0 all designed, staged release |
| Prepare/sign/submit | yes | yes | yes | yes | yes | yes | yes | P0 for Rust/C/Go/Python |
| Unary invoke | yes | yes | yes | yes | yes | yes | yes | P0 for Rust/C/Go/Python |
| Server stream | yes | yes | yes | yes | yes | yes | yes | P1 if not needed for first backend cutover |
| Bidi session | yes | yes | yes | yes | yes | yes | yes | P1 |
| Directory/catalog | yes | yes | yes | yes | yes | yes | yes | P0 for backend read models |
| Identity/signer | yes | yes | yes | yes | yes | yes | yes | P0 prepare/sign parity |
| Receipt profile | yes | yes | yes | yes | yes | yes | yes | P0 for backend and EasyRemote causal/provenance flows |
| Health/diagnostics | yes | yes | yes | yes | yes | yes | yes | P0 backend readiness |
| File transfer wrapper | yes | yes | yes | yes | yes | yes | yes | P1 |
| Terminal wrapper | yes | yes | yes | yes | yes | yes | yes | P1 |
| Remote desktop wrapper | yes | yes | yes | yes | yes | yes | yes | P2 |
| Publication profile | yes | yes | yes | yes | yes | yes | yes | P0 for EasyRemote cutover; P1 for others |
| Host Binding profile | yes | yes | yes | yes | yes | yes | yes | P0 for EasyRemote/local host app cutover; P1 for others |
| Mission profile | yes | yes | yes | yes | yes | yes | yes | P0 for EasyRemote Pipeline cutover; P1 for others |
| Admin + Gateway profile | yes | yes | yes | yes | yes | yes | yes | P0 for full EasyRemote cutover; optional for caller-only consumers |
| Events profile | yes | yes | yes | yes | yes | yes | yes | P0 for EasyNet backend/Hub cutover; P1 for monitors |
| Surface profile | yes | yes | yes | yes | yes | yes | yes | P0 for EasyNet backend/Hub page/surface routes |
| Compatibility profile | yes | yes | yes | yes | yes | yes | yes | P0 for EasyNet backend/Hub compatibility endpoints |
| Conformance runner | yes | yes | yes | yes | yes | yes | yes | Required before language marked stable |

The matrix is aspirational for P1/P2 languages but normative for shape. A
language may ship later; it must not ship with a different concept model.

### 25.1 Language Interface Projection Rules

Each language may use idiomatic names and async primitives, but it MUST project
the same object graph, profile clients, lifecycle ownership, terminal states,
and error taxonomy.

| Target | Required projection | Async/stream idiom | Stability proof |
| --- | --- | --- | --- |
| Rust | Native `SdkEnvironment`, `DaemonHandle`, `RuntimeClient`, profile clients, DTOs, and typed `DaemonError`; no public Axon/proto types. | `Future`, `Stream`, explicit close/cancel where `Drop` is insufficient. | Rust conformance plus C ABI parity fixtures for canonical material and terminal states. |
| C ABI | Opaque handles for every SDK object family and JSON DTOs matching `sdk/schemas`. | Poll/callback handles with explicit close/cancel/free functions. | Header/export checks, ABI version/feature discovery, and schema-backed conformance. |
| Go | Public SDK packages wrapping daemon runtime boundary with `context.Context`, typed structs, and profile clients. | Channels/iterators or callback adapters; context cancellation distinct from SDK cancel. | Backend SDK-only import ban plus Go conformance for Runtime Core and shipped profiles. |
| Python | Product-facing package that EasyRemote imports; profile clients and DTOs are public, raw FFI is private implementation. | Context managers and async iterators where exposed; sync wrappers may delegate to async or threads. | EasyRemote extraction bans plus Python conformance for Runtime Core, Directory + Identity, and every shipped profile. |
| Node / TypeScript | Typed package with `DaemonHandle`, `RuntimeClient`, builders, profile clients, and generated schema types. | `Promise`, `AsyncIterable`, `AbortSignal`, and explicit close/cancel. | Type tests, runtime conformance fixtures, and no normal-path CLI subprocess calls. |
| Java / JVM | `AutoCloseable` clients/builders, DTO classes, profile clients, stable error hierarchy. | `CompletableFuture`, iterator, or reactive publisher; cancellation remains observable. | JNI/transport conformance and no generated Axon protobufs in public API. |
| Swift | Swift Package with async/await clients, value DTOs, profile clients, and stable error enum. | `async` functions and `AsyncSequence`; explicit close/cancel for daemon resources. | Swift conformance and no Axon symbols in public package API. |

Method names may differ by naming convention, but the following families MUST be
present under equivalent profile clients before a language can claim complete
SDK support: environment/version, daemon lifecycle, runtime invocation,
prepare/sign/submit, stream/bidi, directory, identity, receipt, publication,
host binding, mission, admin/gateway, events, surface, compatibility,
convenience wrappers, health, typed errors, and conformance runner integration.

### 25.2 Current Repository Coverage Snapshot

This subsection is descriptive. It records the current repository shape so the
requirements above are not mistaken for completed implementation.

| Surface | Current coverage | Missing before stable SDK claim |
| --- | --- | --- |
| Rust daemon internals | Daemon process, ability dispatch, invocation routing, C ABI implementation, daemon Invocation service, and the source-of-truth runtime substrate exist in `src/daemon/`, `src/ffi/`, and the Rust crate. | A clearly separated public Rust daemon SDK facade with stable DTOs and package-level profile clients remains incomplete before Rust can claim language-stable public SDK support. |
| C ABI v4 | `include/easynet_cli.h` and `src/ffi/` expose ABI/version discovery, typed error JSON, daemon lifecycle/open-runtime, runtime health, unary invoke, stream/bidi callbacks, invocation builder handles, prepare/sign/submit-handle flows, authority materialization, and shipped profile carrier/projection entry points. | Live event streaming coverage, broader profile execution handles, and downstream binding cutover gates remain incomplete before complete C ABI SDK support can be claimed. |
| Go SDK | `sdk/go` is a provider-backed P0 facade for Runtime Core plus Directory + Identity, Receipt, Publication, Host Binding, Mission, Admin + Gateway, Events, Surface, Compatibility, Wrappers, typed errors, backend boundary gates, package metadata gates, product smoke gate wiring, and shared conformance runner integration. | Backend route-source cutover, publish/release stability evidence, lower-layer deletion proof, and per-route live smoke evidence remain incomplete before backend-cutover-ready or complete SDK support can be claimed. |
| Python SDK | `sdk/python/easynet_sdk` is a provider-backed P0 facade for Runtime Core plus Directory + Identity, Receipt, Publication, Host Binding, Mission, Admin + Gateway, Events, Surface, Compatibility, Wrappers, typed errors, EasyRemote boundary gates, package metadata gates, and shared conformance runner integration. | EasyRemote repository extraction, non-C ABI live daemon keyring transport, publish/release stability evidence, and lower-layer deletion proof remain incomplete before EasyRemote-cutover-ready or complete SDK support can be claimed. |
| Node / TypeScript SDK | `sdk/node` exposes a P1 seam package with package metadata gates for Runtime Core, Health, Authority, Directory + Identity, Receipt, Publication, Host Binding, Mission, Admin + Gateway, Events, Surface, Compatibility, and Wrappers with type declarations, Invocation builders, promise-based runtime calls, async iterable stream/bidi handles, bounded stream/bidi backpressure projection, `AbortSignal` cancellation, authority metadata guardrails, and profile DTO/client seams over injected transports. | Daemon/C ABI provider, publish/release stability evidence, and product cutover evidence remain incomplete before Node can claim provider-backed or stable SDK support. |
| Java / JVM SDK | `sdk/java` exposes a P1 Runtime Core seam as a Maven package for typed errors, feature discovery, complete Invocation draft construction, injected runtime transport, `CompletableFuture` async adapters, iterator-backed bounded stream/bidi retained-history state, Health DTO/client seams, Directory + Identity DTO/client seams, and directly exercised seam action-adapter reports over dependency-free Java classes. | JNI/C ABI or daemon transport adapter, receipt/profile APIs, full reactive publisher adapters, provider-backed transport evidence, directory subscription lifecycle, and product cutover evidence. |
| Swift SDK | `sdk/swift` exposes a P1 Runtime Core seam as a Swift Package Manager package for typed errors, feature discovery, complete Invocation draft construction, injected runtime transport, `AsyncSequence` stream/bidi retained-history state, Health DTO/client seams, Directory + Identity DTO/client seams, and directly exercised seam action-adapter reports over dependency-free Swift types. | C ABI or daemon transport adapter, receipt/profile APIs, provider-backed transport evidence, directory subscription lifecycle, and product cutover evidence. |
| SDK schemas | `sdk/schemas/` contains the shared public DTO schema set for Runtime Core, profile clients, stream/bidi terminal projections, and conformance fixtures. | Schema generation/validation must stay tied to Axon helpers and every new public DTO must add fixture bindings before profile-stable claims. |
| SDK conformance | `sdk/conformance/` contains shared cases, fixtures, fixture-schema bindings, parity matrix, backend route-family coverage data, Rust/C ABI/Go/Python action-adapter reports, Node/Java/Swift seam action-adapter reports, Java/Swift seam scaffold guards, and aggregate product smoke gate wiring. | Per-route live product smoke evidence, provider-backed reports for non-P0 daemon transports, and complete profile execution cases remain incomplete. |
| Interface docs | `sdk/README.md`, `sdk/SDK_INTERFACE_SPEC.md`, `sdk/SDK_PARITY.md`, and `sdk/CONFORMANCE_SUITE.md` exist as implementation-facing contracts. | These files must remain synchronized with the parity matrix, action-adapter reports, and backend/EasyRemote cutover gates as new profiles move from seam to provider-backed or cutover-ready. |

The immediate spec consequence is that no P0 language should be marked stable
from the current tree. Rust/C ABI have enough runtime dispatch to validate the
daemon Invocation path, but not enough complete interface coverage to satisfy
sections 9.7, 25, and 27.

## 26. SDK Interface Spec Files to Add

This document is the requirements source. The SDK work should add these
Axon-style files under `sdk/`:

| File | Purpose |
| --- | --- |
| `sdk/README.md` | Entry point for SDK users and maintainers. |
| `sdk/SDK_INTERFACE_SPEC.md` | Normative OOP API for every supported language. |
| `sdk/SDK_PARITY.md` | Feature and behavior parity matrix by language. |
| `sdk/CONFORMANCE_SUITE.md` | How to run golden fixtures against every SDK. |
| `sdk/schemas/*.schema.json` | Canonical public JSON shapes. |
| `sdk/conformance/cases/*.yaml` | Language-neutral behavior tests. |
| `sdk/conformance/fixtures/*` | Canonical invocation, receipt, event, and error fixtures. |

`SDK_INTERFACE_SPEC.md` MUST include:

1. Object model and method list for each core object.
2. Language naming mapping.
3. Sync/async behavior per method.
4. Ownership and lifetime rules.
5. Thread-safety rules.
6. Error taxonomy.
7. State machine references.
8. Minimum examples for unary, stream, bidi, directory, aggregate fan-out,
   publication, host binding, mission, admin/gateway, events, surface,
   compatibility, convenience wrappers, and health.
9. MEMC profile ownership matrix: every public method mapped to exactly one
   profile or declared as product-facade-only.
10. Consumer coverage matrix for backend, EasyRemote, CLI, desktop/GUI,
    third-party host apps, and future bindings.
11. Semantic alignment matrix mapping product-facing terms to SDK DTOs and
    daemon/Axon ownership.
12. Source alignment ledger mapping each interface family to Axon, daemon,
    EasyNet ontology, or product facade ownership.
13. Hub route-family coverage matrix mapping each EasyNet backend runtime route
    family to exactly one SDK profile/client and one retained backend product
    responsibility.
14. MEMC completion gates defining Runtime Core, profile, language-stable,
    complete SDK, EasyRemote-cutover-ready, and backend-cutover-ready claims.

`SDK_PARITY.md` MUST include:

1. Language support tier.
2. Feature support table.
3. Platform support table.
4. Known gaps and deadlines.
5. API stability level per package.
6. Packaging status per registry.

`CONFORMANCE_SUITE.md` MUST include:

1. Test fixture format.
2. Runner contract.
3. Required cases before a language can be marked stable.
4. CI commands for Rust, C ABI, Go, Python, Node, Java, and Swift.
5. Rules for adding a new language.

## 27. Conformance Requirements

Before backend cutover, the repo MUST provide language-neutral conformance cases
for the P0 languages. Before P1 language release, each P1 SDK must pass the same
case set.

Minimum conformance cases:

1. `version/abi_compatible`: SDK detects compatible daemon/ABI version.
2. `version/abi_incompatible`: SDK returns typed version mismatch.
3. `daemon/control_only`: control socket up but invocation socket down reports
   degraded runtime readiness.
4. `daemon/permission_denied`: socket permission failure maps to
   `PermissionDenied`.
5. `invocation/complete_tuple`: builder rejects missing caller, callee, ability,
   subject, nonce, causal context, or args.
6. `invocation/canonical_material`: all languages obtain identical canonical
   signing bytes for the same draft from the Axon/daemon canonical helper path;
   facades do not compute a second canonicalization.
7. `invocation/presigned_submit`: `submit_signed` preserves caller signature.
8. `invocation/prepared_not_submittable`: submit APIs reject
   `PreparedInvocation`; only `SignedInvocation` is submit-ready.
9. `invocation/local_daemon_signing_boundary`: local daemon signing produces a
   `SignedInvocation` with signer id and policy proof before submission.
10. `invocation/terminal_monotonicity`: terminal states cannot transition again.
11. `authority/mutual_exclusion`: local daemon signing and caller pre-signing
   rules are enforced.
12. `stream/order_terminal`: stream data frames are ordered and terminal appears
    once.
13. `stream/backpressure_bound`: stream queue overflow produces typed backpressure
    or cancellation behavior.
14. `bidi/frame0_required`: bidi session without frame0 is rejected.
15. `bidi/close_send_not_cancel`: local half-close is distinguishable from
    cancel.
16. `directory/snapshot_then_live`: directory subscription emits snapshot before
    live deltas.
17. `directory/list_pagination`: list APIs reject requests above max page size.
18. `directory/no_default_fanout`: ordinary list APIs do not perform per-agent
    live remote calls.
19. `aggregate/partial_result`: aggregate fan-out returns typed partial results
    and child receipt refs.
20. `error/retry_hint`: retryable and non-retryable errors are classified
    consistently.
21. `health/api_vs_runtime`: API liveness and daemon runtime readiness are
    distinct.
22. `backend/import_ban`: backend cannot import Axon public packages after
    cutover.
23. `backend/no_direct_daemon_transport`: backend production code cannot import
    generated `axon.v1` protobufs, direct daemon gRPC/socket clients, C ABI/FFI,
    control-frame dispatch, CLI subprocess runtime paths, or EasyRemote.
24. `backend/hub_route_family_coverage`: every EasyNet backend runtime route
    family maps to exactly one SDK profile/client and one retained backend
    product responsibility.
25. `backend/events_profile`: directory, device, session, invocation, SSE, and
    polling-fallback event flows consume SDK `EventClient` DTOs/cursors and do
    not own daemon subscribe loops.
26. `backend/admin_pairing_session_profile`: pairing preflight/validate,
    credential verification, device admin, gateway status, agent lifecycle, and
    device sessions consume SDK `AdminClient` and Directory/Identity DTOs.
27. `backend/surface_profile`: page create/list/delete, public pages, and
    surface manifests consume SDK `SurfaceClient`; backend owns rendering and
    browser auth only.
28. `backend/compatibility_profile`: OpenAI-compatible models/chat/files routes
    consume SDK `CompatibilityClient` or declared wrapper DTOs and do not own raw
    ability+args compatibility shims.
29. `backend/wrapper_profile`: file/context upload, terminal, remote desktop,
    browser session, and media/voice WebSocket bridges consume SDK wrappers and
    preserve stream/bidi terminal-state distinctions.
30. `backend/receipt_projection`: call/history/metrics/failure-location paths
    consume SDK receipt/error projections and do not parse raw receipt strings or
    protobuf fields for control flow.
31. `python/easyremote_no_raw_ffi`: EasyRemote production code does not import
    `ctypes`, call `dlopen`, or reference `easynet_*` ABI symbols directly.
32. `python/easyremote_no_invocation_codec`: EasyRemote does not encode daemon
    Invocation JSON; it consumes SDK `InvocationDraft`, `PreparedInvocation`,
    `SignedInvocation`, stream, bidi, and receipt DTOs.
33. `python/easyremote_publication_profile`: EasyRemote ability deploy/list/show
    flows use SDK Publication APIs and do not own generic daemon system ability
    carriers.
34. `python/easyremote_mission_profile`: EasyRemote `Pipeline` and direct
    Mission/EAL calls use SDK `MissionClient` for run/track/cancel/status.
35. `python/easyremote_context_causal`: EasyRemote server-side `Context.call`
    can attach parent receipt causal refs through SDK receipt APIs, or remains
    explicitly disabled when full receipt refs are unavailable.
36. `python/easyremote_host_binding_profile`: EasyRemote host serving code uses
    SDK `HostBindingClient` DTOs/codecs for request envelope, item/error/
    terminal frames, and output-hash folding.
37. `python/easyremote_admin_gateway_profile`: EasyRemote `Server`,
    `DaemonHandle`, and `AgentControl` use SDK Runtime Core and Admin + Gateway
    APIs for daemon lifecycle, gateway status, and agent lifecycle transport.
38. `python/easyremote_product_facade_only`: EasyRemote keeps `ComputeNode`,
    `@node.register`, `@remote`, schema introspection, `Pipeline` DSL, TLS
    provisioning policy, and examples as product facades over SDK profiles.
39. `memc/profile_exclusivity`: each public SDK operation belongs to one profile
    owner; duplicate placement fails review.
40. `memc/consumer_coverage`: every first-class consumer can implement its
    runtime needs through declared profiles without importing lower layers.
41. `memc/semantic_alignment`: terms such as capability, AbilityDescriptor,
    AbilityImpl, Invocation, Receipt, Pipeline, and Context.call map to the same
    SDK DTOs and daemon/Axon owners across docs and language bindings.
42. `memc/no_core_bloat`: Runtime Core contains no publication package building,
    Python decorators, backend DTOs, CLI command text, or one-method-per-ability
    helper as a required stable API.
43. `invocation/descriptor_ref_helper_delegation`: descriptor refs are obtained
    or validated through Axon helpers equivalent to
    `canonical_ability_descriptor_ref` and `ability_ura_from_descriptor_ref`;
    facades do not concatenate `ability_ura + "@" + version`.

Each case should have:

```yaml
id: invocation/complete_tuple
description: Builder rejects incomplete seven-tuple drafts.
given:
  draft:
    caller: "easynet:///r/acme/agent/alice.sdk"
    callee: "easynet:///r/acme/device/edge-01"
    descriptor_ref: "easynet:///r/acme/ability/device.edge-01.fs.read@1.0.0"
    subject_ura: null
    nonce_base64: "AQIDBAUGBwgJCgsMDQ4PEA=="
    causal_context: {"form": "none"}
    args: {"path": "/tmp/a.txt"}
expect:
  error_code: "INVALID_INVOCATION"
  missing_fields: ["subject_ura"]
```

## 28. EasyRemote Extraction Contract

EasyRemote MUST become a high-level consumer of the Python Daemon SDK. This is
not optional polish: EasyRemote is the acceptance test that the SDK supports
local host apps, ability publication, streaming, bidi, mission orchestration,
identity, receipts, and daemon lifecycle without forcing each product facade to
own its own runtime substrate.

The Python Daemon SDK is EasyRemote-cutover-ready only when EasyRemote can
delete or reduce these modules to compatibility shims:

| Current EasyRemote responsibility | SDK profile that must absorb it | Acceptance condition |
| --- | --- | --- |
| `easyremote/_transport/abi.py`: `libeasynet_cli` loading, ABI version checks, symbol prototypes, out-string ownership, last-error mapping | Runtime Core | EasyRemote imports a Python SDK package and never calls `ctypes`, `dlopen`, `easynet_*`, or `easynet_last_error` directly. |
| `easyremote/_transport/session.py`: handle lifetime, unary dispatch thread, stream callback queues, bidi channel, daemon process handle | Runtime Core | EasyRemote uses `RuntimeClient`, `StreamHandle`, `BidiSession`, and `DaemonControl`; raw handles and callback lifetime rules are hidden inside the SDK. |
| `easyremote/invocation.py`: seven-tuple DTOs, nonce generation, payload codec, causal encoding, descriptor_ref wire encoding, prepared invocation wrapper | Runtime Core + Axon delegation | EasyRemote receives SDK `InvocationDraft`, `Invocation`, `PreparedInvocation`, `SigningMaterial`, and `SignedInvocation`; it never serializes daemon Invocation JSON itself. |
| `easyremote/client.py` signing gap: `signing_path_pending` | Runtime Core + Identity | EasyRemote can request prepare/sign/submit through SDK signer helpers; caller signatures and public key material are preserved. |
| `easyremote/receipts.py`: receipt summary DTOs, invocation states, continuity checks, failed cryptographic verification | Receipt | SDK exposes receipt summary, full receipt fetch when available, opaque daemon/Axon-returned receipt refs, receipt-chain continuity, and verification entry points. |
| `easyremote/context.py`: child dispatch unavailable without parent receipt URA | Runtime Core + Receipt | SDK can turn a parent terminal receipt into a causal reference for child Invocations; EasyRemote `Context.call` no longer fabricates or guesses causal placement. |
| `easyremote/identity.py`: local credentials loading, device/hub/agent/resource URA rendering | Directory + Identity | SDK exposes local identity, URA builders, resource refs, signer construction, and key registration/list/revoke. EasyRemote does not hand-build URA strings. |
| `easyremote/daemon.py`: daemon start/status/endpoint/open-client/stop wrapper | Runtime Core | EasyRemote either re-exports SDK `DaemonControl`/`DaemonHandle` or keeps only a thin product alias. |
| `easyremote/control.py`: ability deploy/list/show over daemon system abilities | Publication + Directory | Generic daemon publication/catalog calls move to SDK. EasyRemote keeps only Python package generation and product naming. |
| `easyremote/control.py`: `AgentControl.add/list/refresh` over daemon system abilities | Admin + Gateway | Generic daemon agent lifecycle calls move to SDK `AdminClient`. EasyRemote may keep product aliases only. |
| `easyremote/mission.py`: `mission.run/track/cancel` transport facade | Mission | EasyRemote `Pipeline` compiles EAL and calls SDK `MissionClient`; daemon mission transport and status DTOs live in SDK. |

Full repository coverage must also classify the modules that are not pure
transport. A module may remain in EasyRemote only when its retained role is a
product facade rather than daemon substrate.

| EasyRemote module | SDK profile absorption | EasyRemote retained role |
| --- | --- | --- |
| `easyremote/client.py` | Runtime Core, Directory + Identity, Host Binding for stream/bidi DTO alignment | Result-first ergonomics, `RemoteFunction`, `RemoteOwner`, `@remote`, and Python signature binding. |
| `easyremote/_addressing.py` | Directory + Identity | Product selection policy names and short-name ergonomics only. Owner/ability URAs and directory facts come from SDK DTOs. |
| `easyremote/node.py` | Publication, Host Binding, Directory + Identity | `ComputeNode`, `@node.register`, package generation, and local Python host lifecycle. It must not own resource refs, host-stream frame semantics, or deploy/list carriers. |
| `easyremote/_host/server.py` | Host Binding | Running Python user functions and managing the warm host socket. Envelope decode, frame encoding, terminal mapping, and rolling output hash come from SDK fixtures/helpers. |
| `easyremote/schema.py` | product facade, feeding Publication DTOs | Python annotation/signature introspection and JSON schema extraction. Shared schema DTO names must map to SDK publication schemas. |
| `easyremote/pipeline.py` | Mission | Python DSL and EAL generation. Submission, tracking, cancellation, event stream, child receipt refs, and mission status are SDK-owned. |
| `easyremote/gateway.py` | Runtime Core, Admin + Gateway, Health | `Server` UX, TLS certificate provisioning policy, and pairing guidance. Daemon lifecycle, gateway readiness, and admin transport are SDK-owned. |
| `easyremote/errors.py` | Runtime Core typed errors | Product aliases or richer messages only. Stable error codes, retry hints, stages, and receipt/invocation refs are SDK-owned. |
| `easyremote/_codec.py`, `easyremote/_json.py` | SDK schemas, Runtime Core, Host Binding | Conversion of Python values to JSON-compatible payloads. Canonical protocol JSON, host-stream hash JSON, and shared DTO schemas are SDK-owned. |
| `easyremote/config.py`, `easyremote/_toml.py` | Admin + Gateway where config touches daemon lifecycle | Product defaults and local file parsing. Daemon mode/status/readiness DTOs are SDK-owned. |
| `easyremote/_cli.py` | SDK consumer | CLI argument parsing and user-facing text only. It must not bypass SDK clients for runtime behavior. |

The following MUST remain in EasyRemote:

1. `ComputeNode`.
2. `@node.register`.
3. `@remote`.
4. Python function/class signature introspection and JSON schema extraction.
5. Warm Python host process integration and host-stream implementation.
6. `Pipeline` Python DSL and EAL generation.
7. `Server` TLS provisioning policy, pairing guidance, and product UX.
8. Python product examples, gallery, docs, and positioning.
9. Product-level owner handles and ergonomic stubs, as thin wrappers over SDK
   URA builders and directory results.

EasyRemote MUST NOT keep:

1. A raw `ctypes` binding to `libeasynet_cli`.
2. A raw daemon handle/session implementation.
3. A daemon Invocation JSON encoder.
4. Its own daemon error-code taxonomy separate from SDK typed errors.
5. Its own receipt verification placeholder caused by missing SDK receipt
   fetch/projection APIs.
6. Host-stream envelope, frame, terminal, or output-hash semantics separate from
   SDK Host Binding fixtures/helpers.
7. Generic ability publication, mission, directory, identity, admin/gateway, or
   daemon lifecycle transport code.

EasyRemote MUST NOT be a dependency of EasyNet backend.

## 29. Backend Migration Requirements

EasyNet backend migration is complete only when:

1. Backend `go.mod` no longer requires `easynet.run/axon/sdk/go`.
2. Backend does not import `easynet.run/axon/*`.
3. Backend does not expose or depend on generated `axon.v1` proto types.
4. Backend runtime injection is named `Runtime` or `DaemonRuntime`, not `Axon`.
5. Backend ability invoke, directory, events, identity, receipts, pairing/trust,
   device sessions, file/context upload, terminal, remote desktop,
   browser/media sessions, pages/surfaces, and OpenAI compatibility paths all
   use the CLI Go SDK.
6. Backend tests use `fakeRuntimeClient`, not `fakeAxonClient`.
7. Backend health distinguishes API liveness from daemon runtime readiness.
8. Backend list endpoints use daemon SDK directory pages or named aggregate
   abilities; they do not manually fan out to device ability endpoints.
9. Backend route-family coverage maps every runtime route group to one SDK
   profile/client and one retained backend product responsibility.

### 29.1 Backend SDK-only Contract

After the Go Daemon SDK reaches cutover-ready status, EasyNet backend production
code is an SDK consumer only.

Allowed production dependency:

1. Public Go SDK packages under the chosen `sdk/go` module path, for example
   `easynet.run/cli/sdk/go/...`.

Allowed local backend shape:

1. Backend MAY define product-level interfaces such as `Runtime` or
   `DaemonRuntime` for dependency injection.
2. The only production implementation of those interfaces MUST wrap the public
   CLI Go SDK client.
3. Tests MAY use SDK-compatible fakes or mocks, but the fake shape must be named
   after the SDK/runtime boundary, not Axon.

Forbidden backend production dependencies and call paths:

1. Direct Axon SDK imports, generated `axon.v1` protobuf types, raw
   `axon-runtime` clients, or Axon runtime process lifecycle.
2. Direct C ABI / FFI calls, including `libeasynet_cli`, `include/easynet_cli.h`,
   `import "C"`, cgo wrappers, `dlopen`/`dlsym`, or any `easynet_*` ABI symbol.
3. Direct Rust crate internals, daemon internals, or paths under
   `src/daemon`, `src/ffi`, `src/services`, or `src/runtime`.
4. Direct daemon socket/protocol access from backend code, including hand-written
   `daemon.sock` UDS clients, direct TCP daemon protocol clients, direct gRPC
   clients to daemon invocation services, or generated daemon protobuf clients.
5. Product ability calls over `control.sock`, `control.json`, or control-frame
   JSON `Invoke`, `Subscribe`, or `OpenBidi`.
6. Shelling out to `easynet` or `easynet-daemon` commands for normal product
   runtime behavior.
7. EasyRemote, Python SDK packages, Node SDK packages, or any other language
   facade as backend runtime dependencies.
8. Backend-local reimplementation of Invocation canonicalization, signing
   material, admission, receipt verification, stream/bidi terminal semantics,
   directory pagination, or aggregate fan-out policy.

The Go SDK facade MAY use daemon local transport and MAY call C ABI helpers
inside the SDK implementation if that is the chosen implementation strategy.
That does not grant the backend permission to call FFI directly. From the
backend's perspective, the SDK package boundary is the only runtime boundary.

### 29.2 Hub Interface Coverage Matrix

The EasyNet backend is the acceptance test for Hub completeness. A Go SDK design
is not backend-cutover-ready until every route/handler family below is expressed
through the public SDK boundary.

| Backend route/handler family | SDK profile/client that must absorb daemon runtime behavior | Backend retained responsibility |
| --- | --- | --- |
| `/health`, daemon readiness, runtime liveness | `HealthClient`, `DaemonHandle`, `RuntimeClient` | API process liveness, deployment status text, product health response shape |
| User signing keys, local identity, prepare/submit handoff | `IdentityClient`, `RuntimeClient`, `PreparedInvocation`, `SignedInvocation` | Browser auth, user account ownership, JWT/OAuth session checks |
| Device pairing preflight/validate, credential verification, device revoke | `AdminClient`, `IdentityClient`, `DirectoryClient` | Pairing token issuance UX, account-device DB binding, rate limits |
| Device, agent, and ability catalog routes | `DirectoryClient`, `PublicationClient` read models | Product filtering, dashboard DTOs, cached DB projections |
| Device sessions and gateway/agent lifecycle | `AdminClient`, `RuntimeClient`, `BidiSession` where needed | Browser session authorization, WebSocket upgrade routing |
| Ability invoke, signed invoke, stream, signed bidi bridge | `RuntimeClient`, `InvocationBuilder`, `StreamHandle`, `BidiSession`, `ReceiptClient` | HTTP/WS route auth, request size limits, response presentation |
| Directory, device, session, invocation, and SSE events | `EventClient`, `DirectoryClient.subscribe` | Subscriber registry, tenant authorization, browser SSE/WS fanout |
| File transfer and context-file upload | Convenience file wrappers, `RuntimeClient`, `ReceiptClient` | Multipart parsing, browser auth, storage quota/policy |
| Terminal, remote desktop, browser session, voice/media bridges | Convenience wrappers over `BidiSession`/`StreamHandle` | WebSocket upgrade, per-user authorization, UI protocol adaptation |
| Page create/list/delete, public pages, surface manifests | `SurfaceClient`, `DirectoryClient`, `HealthClient` | Public HTTP rendering, frontend route mounting, auth/cache/CDN policy |
| Skill/plugin install/list/remove/upgrade/file tree | `PublicationClient` implementation-resource management, `DirectoryClient` | Product naming, upload UX, marketplace/account policy |
| OpenAI-compatible models/chat/files | `CompatibilityClient`, file wrappers, `RuntimeClient`, `DirectoryClient` | Product API keys, billing/quota/rate limits, HTTP compatibility response shape |
| Call history, receipts, failure location, metrics | `ReceiptClient`, `EventClient`, `HealthClient` | Product analytics aggregation, DB retention, dashboard presentation |
| Federation, peer hubs, remote devices | `AdminClient`, `DirectoryClient`, `RuntimeClient` remote connection DTOs | Peer trust policy, account-level federation settings, product UX |

Coverage rules:

1. Each row MUST have at least one SDK conformance or backend smoke that proves
   the backend route family no longer imports Axon, generated protobufs,
   `daemon_grpc`, raw sockets, C ABI, control frames, subprocesses, or
   EasyRemote.
2. The SDK profile owns daemon DTOs and terminal semantics. The backend owns
   browser/product DTOs and must translate at its HTTP boundary.
3. A route family may use multiple SDK clients, but it MUST NOT invent a
   backend-local daemon transport or Invocation codec because no single SDK
   method fits exactly.
4. If a new backend runtime route family is added, this matrix and the
   conformance suite must be updated before the route is considered covered.

## 30. Conformance and CI Gates

The SDK repo MUST provide these gates before backend cutover:

1. Golden conformance cases for Invocation JSON, URA/DescriptorRef validation,
   and Axon/daemon-obtained canonical signing material.
2. Rust, C ABI, Go, and Python parity tests for the same Invocation fixture.
3. ABI v3 header/export/version checks.
4. SDK URA naming gate: public SDK docs, C ABI headers, FFI surfaces, and
   Go/Python SDK facades must not reintroduce retired address-era identifiers
   or aliases.
5. Go SDK import ban: Go SDK packages must not import Axon. Axon dependency is
   contained in the native Rust daemon SDK core and daemon runtime adapter.
6. EasyNet backend SDK-only ban after cutover, using section 29.1 as the single
   normative forbidden-dependency list:
   - no `easynet.run/axon/*`;
   - no generated `axon.v1` protobuf imports;
   - no `libeasynet_cli`, `import "C"`, cgo FFI wrappers, `dlopen`, or
     `easynet_*` ABI symbol calls;
   - no direct daemon UDS/TCP/gRPC/protobuf clients outside the CLI Go SDK;
   - no `easynet`/`easynet-daemon` subprocess calls for product runtime paths;
   - no EasyRemote or non-Go language SDK runtime dependency.
7. Backend runtime allowlist: production runtime packages may depend only on the
   public CLI Go SDK module path plus backend-owned product interfaces/fakes.
8. Backend Hub route-family coverage gate: every row in section 29.2 has a
   route-family smoke or static check proving the route uses the assigned SDK
   profile/client.
9. Backend Events gate: SSE, directory events, device events, session events,
   and invocation events use SDK `EventClient` DTOs/cursors.
10. Backend Surface and Compatibility gates: page/public-surface routes use SDK
   `SurfaceClient`, and OpenAI-compatible models/chat/files routes use SDK
   `CompatibilityClient`.
11. Backend wrapper gate: file/context upload, terminal, remote desktop,
    browser session, and media/voice bridges use SDK wrappers or Runtime Core
    stream/bidi clients with shared terminal-state fixtures.
12. Backend Admin gate: pairing, credential verification, gateway status,
    device sessions, and agent lifecycle use SDK `AdminClient`,
    `DirectoryClient`, and `IdentityClient` DTOs.
13. `control.sock` product-call ban: no `Invoke`, `Subscribe`, or `OpenBidi`
    product dispatch over JSON control frames.
14. Facade fan-out ban: ordinary SDK list methods do not run per-target
    governed calls.
15. Live daemon smoke covering unary, stream, bidi, file transfer, and typed
    terminal failure.
16. Health smoke covering daemon down, UDS permission denied, public listener
    down, control-only, version mismatch, and trust not ready.
17. Python SDK extraction ban: EasyRemote production code must not import
    `ctypes`, open `libeasynet_cli`, call `easynet_*` ABI symbols, encode daemon
    Invocation JSON, own host-stream frame/hash semantics, call gateway/agent
    admin system abilities directly, or own raw daemon handle/session lifecycle
    after cutover.
18. Publication profile smoke: deploy an EasyRemote-generated host-stream
    package through SDK Publication APIs, list/show it through SDK directory or
    publication pages, then invoke it through SDK Runtime APIs.
19. Host Binding profile smoke: serve one EasyRemote-hosted unary function and
    one streaming function through SDK Host Binding DTOs/codecs, verifying item,
    error, terminal, and output-hash fixture parity with the daemon.
20. Mission profile smoke: run, track, cancel, and observe terminal status for
    an EAL mission through SDK Mission APIs, with child receipt refs when the
    daemon provides them.
21. Admin + Gateway profile smoke: start or attach a hub/device daemon, read
    gateway status, list/refresh/start an agent through SDK `AdminClient`, and
    verify product facades do not call daemon admin abilities directly.
22. Events/Surface/Compatibility profile smoke: subscribe to directory/device
    events, create/list/delete a page or fetch a surface manifest, and execute
    one OpenAI-compatible model/chat/files flow through SDK profile clients.
23. Receipt profile smoke: fetch/project a terminal receipt, expose a receipt
    URA usable as causal context, and distinguish summary-only continuity checks
    from full cryptographic verification.

## 31. Implementation Phases

The migration from the current architecture to the complete SDK architecture
must be staged by semantic boundary.

### 31.1 Phase A - SDK Spec and Scaffold

Deliverables:

1. Add `sdk/README.md`, `SDK_INTERFACE_SPEC.md`, `SDK_PARITY.md`, and
   `CONFORMANCE_SUITE.md`.
2. Add `sdk/schemas/` placeholders for Invocation, receipt, error, health,
   events, directory pages, publication, resource refs, host-stream binding and
   frame codecs, mission status, admin, gateway, agent records, surface/page,
   compatibility, file, terminal, remote desktop, browser session, and media
   session records.
3. Add `PROJECT_STRUCTURE.md` or update it if it already exists.
4. Add CI checks that spec files and schema fixtures are present.

Exit criteria:

1. Every supported language has an owner and tier.
2. Every SDK object has a documented lifecycle.
3. Every state machine is referenced by the interface spec.

### 31.2 Phase B - Rust Core and C ABI Freeze

Deliverables:

1. Split Rust daemon SDK facade from daemon process internals where needed.
2. Wire SDK Invocation JSON projections to Axon canonical Invocation and
   signing-material helpers; do not create a second canonicalization path.
3. Freeze C ABI handle names for daemon, runtime, invocation builder, prepared
   invocation, signed invocation, invocation handle, stream, bidi, directory,
   identity, health, receipt, publication, host binding, mission,
   admin/gateway, events, surface, compatibility, wrappers, and error.
4. Add ABI version and feature discovery.
5. Add receipt projection/fetch entry points sufficient for causal references
   and future full verification.
6. Wrap any unstable raw Axon proto returns behind SDK DTOs before marking the
   Rust API stable.

Exit criteria:

1. Rust and C ABI pass canonical fixture conformance.
2. C header exposes no Axon symbols.
3. CLI binary uses SDK objects, not duplicate command-only code.
4. Stable Rust daemon SDK public API exposes SDK DTOs, not raw `axon.v1` proto
   message types.
5. EasyRemote can no longer justify a local ctypes ABI loader or Invocation
   JSON codec for Runtime Core behavior.
6. Hub and device product paths both run through `easynet-daemon`; no public
   `DaemonHandle`, C ABI, or backend cutover may be frozen while Hub product
   runtime still starts or depends on raw `axon-runtime`.

### 31.3 Phase C - Directory and Aggregate Read Model

Deliverables:

1. Define `Page<T>`, cursor, filter, and directory event schemas.
2. Implement paginated `ListDevices`, `ListAgents`, and `ListAbilities`.
3. Define named aggregate abilities for fleet-wide rich catalog reads.
4. Add fan-out bounds, partial-result error taxonomy, and child receipt refs.
5. Define Publication profile DTOs and daemon carriers for resource refs,
   ability package validation, deploy/list/show/enable/disable/unpublish, and
   host-stream binding metadata.
6. Define Host Binding profile DTOs/codecs for request envelopes, item/error/
   terminal frames, output hashes, readiness, and cleanup.
7. Define Mission profile DTOs and daemon carriers for run/track/cancel/events.
8. Define Admin + Gateway profile DTOs for gateway status, agent lifecycle, and
   join/leave helpers.
9. Define Events profile DTOs for directory/device/session/invocation events,
   cursors, resume tokens, reconnect hints, and drop reports.
10. Define Surface profile DTOs for page records, surface manifests, public page
    refs, and surface health/status.
11. Define Compatibility profile DTOs for OpenAI-compatible model/chat/files
    adapters over SDK runtime, file, and directory DTOs.

Exit criteria:

1. Ordinary list methods satisfy `O(page_size + filter_cost)`.
2. Aggregate methods expose max concurrency and deadlines.
3. Facade fan-out ban passes CI/conformance.
4. Publication, Host Binding, Mission, Admin + Gateway, Events, Surface, and
   Compatibility profiles pass schema conformance before EasyRemote or backend
   cutover work starts.

### 31.4 Phase D - Go SDK for EasyNet Backend

Deliverables:

1. Add `sdk/go` package.
2. Implement `ConnectLocal`, `DaemonHandle`, `RuntimeClient`,
   `InvocationBuilder`, `Prepare`, `SubmitSigned`, `Invoke`, `HealthClient`,
   `DirectoryClient`, `IdentityClient`, `ReceiptClient`, `EventClient`,
   `AdminClient`, `SurfaceClient`, `CompatibilityClient`, selected
   `PublicationClient`, convenience wrappers, and typed errors.
3. Add backend fake runtime client and import-ban test.
4. Cut backend runtime adapter from Axon SDK to CLI Go SDK.
5. Ensure Go owns only facade ergonomics, generated DTOs, transport marshalling,
   retries, and typed error mapping; canonical material, descriptor refs,
   admission, receipt verification, and stream/bidi state semantics must be
   obtained from or delegated to Axon/daemon helper paths.

Exit criteria:

1. EasyNet backend has no public Axon imports.
2. EasyNet backend has no FFI/C ABI, raw daemon socket/protocol, control-frame,
   CLI subprocess, EasyRemote, or non-Go language SDK runtime dependency.
3. Backend production runtime adapters wrap only the public CLI Go SDK client.
4. Backend `/health` distinguishes API liveness from daemon runtime readiness.
5. Backend ability calls fail clearly when daemon invocation socket is absent.
6. Backend list APIs use directory pages or aggregate abilities, not SDK-side
   fan-out loops.
7. Backend section 29.2 route-family coverage gates pass for health, identity,
   pairing/admin, catalog, sessions, invoke/stream/bidi, events, file/context
   upload, terminal/remote desktop/browser/media, pages/surfaces, skills,
   OpenAI compatibility, receipts/metrics, and federation/peer hubs.
8. Backend Hub runtime path uses colocated `easynet-daemon`; backend cutover is
   blocked if production Hub runtime still starts or talks to raw
   `axon-runtime`.

### 31.5 Phase E - Python SDK and EasyRemote Extraction

Deliverables:

1. Add `sdk/python` package.
2. Implement Python Runtime Core, Directory + Identity, Receipt, Publication, Host
   Binding, Mission, and Admin + Gateway profiles.
3. Move generic daemon client, ABI loading, raw handle sessions, Invocation
   codec, URA parsing/building for daemon calls, signing material, receipt
   projection/fetch, typed errors, publication transport, host-stream wire/hash
   helpers, mission transport, and gateway/agent admin transport out of
   EasyRemote.
4. Keep EasyRemote-specific decorators, `ComputeNode`, `@remote`, Python schema
   extraction, warm host process, `Pipeline`, `Server` TLS provisioning policy,
   and gallery code inside EasyRemote.

Exit criteria:

1. EasyRemote imports EasyNet Daemon SDK rather than owning daemon transport,
   raw ABI loading, Invocation JSON encoding, receipt projection, publication
   carriers, host-stream codecs, mission carriers, or admin/gateway carriers.
2. Python conformance cases pass for Runtime Core, Directory + Identity,
   Receipt, Publication, Host Binding, Mission, and Admin + Gateway profiles.
3. EasyRemote `Context.call` either works through SDK parent receipt causal refs
   or remains explicitly disabled by a typed SDK capability flag; it must not
   fabricate causal refs.
4. EasyRemote remains a consumer, not a dependency of backend.

### 31.6 Phase F - Node, Java, and Swift SDKs

Deliverables:

1. Add package skeletons and generated schema types.
2. Implement daemon lifecycle, runtime client, Invocation builder, invoke,
   health, directory, and errors.
3. Add async stream and bidi APIs using each language's idioms.

Exit criteria:

1. Each language passes shared conformance cases before being marked stable.
2. Packaging metadata exists for npm, Maven, and Swift Package Manager.
3. No package exposes Axon symbols.

### 31.7 Phase G - Product and CI Enforcement

Deliverables:

1. Backend import-ban enforcement for Axon.
2. Product-call ban over `control.sock`.
3. Hub route-family coverage gates from section 29.2.
4. Runtime readiness smoke tests for daemon down, invocation socket down,
   permission denied, version mismatch, control-only, and trust-not-ready.
5. Release packaging for daemon plus P0 SDKs.

Exit criteria:

1. Production Hub deploy documents `easynet-api.service` and
   `easynet-daemon.service` as separate processes.
2. Ability invocation requires daemon runtime readiness.
3. Hub route-family coverage checks pass.
4. SDK conformance runs in CI.

## 32. MVP Scope

MVP order:

1. Rust daemon SDK core and C ABI: lifecycle, runtime connection, complete
   Invocation, prepare/sign/submit, typed errors, health, receipt projection,
   and causal receipt refs.
2. Directory + Identity page/query model, URA builders, signer helpers, and
   no-default-fan-out conformance.
3. Publication, Host Binding, Mission, Admin + Gateway, Events, Surface, and
   Compatibility profile schemas plus Rust/C ABI carriers.
4. Go SDK facade: `ConnectLocal`, `Invocation`, `Prepare`, `SubmitSigned`,
   `Invoke`, `Health`, `Directory`, `Identity`, `Receipt`, `Events`, `Admin`,
   `Surface`, `Compatibility`, and selected wrappers for Hub route families.
5. Python SDK extraction for EasyRemote across Runtime Core, Directory +
   Identity, Receipt, Publication, Host Binding, Mission, and Admin + Gateway
   profiles.
6. Reconnecting client, stream, and bidi abstractions.
7. File, terminal, remote desktop, browser session, media, and context-upload
   wrappers.
8. Backend cutover, Hub route-family coverage, and SDK-only enforcement.

The minimum stable Runtime Core is complete Invocation plus signing material,
daemon client, typed health/error projection, receipt projection/fetch hooks,
and causal receipt refs. Daemon lifecycle alone is not sufficient. The complete
SDK claim additionally requires Directory + Identity, Publication, Host Binding,
Mission, Admin + Gateway, Events, Surface, Compatibility, and wrapper profiles
for the P0 languages that claim full EasyRemote or EasyNet Hub repository
cutover.

## 33. Open Questions

1. Exact public module path for the Go SDK (`easynet.run/cli/sdk/go` vs
   `easynet.run/easynet-cli/sdk/go`).
2. Exact Python package name (`easynet-runtime`, `easynet-cli-sdk`, or another
   name).
3. Whether the Go facade should use daemon local transport only or link
   `libeasynet_cli` for selected local helpers. Either path must delegate
   canonical/signing/receipt semantics to the Rust daemon SDK core or daemon.
4. Exact split between SDK Gateway/Admin DTOs and product-owned TLS
   provisioning/onboarding policy. Certificate authority policy remains outside
   the protocol/runtime core unless a future daemon ability governs it.
5. Full receipt fetch and receipt-chain verification API shape once receipt URA
   builders and fetch paths are finalized. Until
   `docs/rfc/AXON-RFC-007-receipt-ura-builder-agenda-2026-06-12.md` is
   resolved, receipt URA values remain opaque daemon/Axon-returned strings.
6. Exact default values for `DefaultPageSize`, `MaxPageSize`,
   `MaxFanoutConcurrency`, and aggregate deadline caps. These must be named
   constants with engineering justification before implementation.

## 34. Final Abstraction Statement

The correct public abstraction is:

```text
Daemon SDK = typed, object-oriented, multi-language client for the local
easynet-daemon runtime boundary.
```

The CLI command binary is one consumer of this SDK. EasyNet backend is another.
EasyRemote is another. The SDK is not command-output scraping, not direct Axon
exposure, and not a hidden fan-out engine in language facade code.
