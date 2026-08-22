# Project Structure v1

Status: Active

Last reviewed: 2026-07-03

Scope: final EasyNet-Cli repository layout, Rust module ownership, daemon
semantic boundaries, SDK root ownership, descriptor/schema placement, tooling
and packaging roots, implementation order, complexity contracts, fan-out rules,
and final acceptance gates.

Non-goal: this spec does not change Ability wire names, Invocation semantics,
Receipt semantics, URA syntax, Axon admission semantics, descriptor schemas,
plugin runtime behavior, or backend product APIs.

## Normative Statement

This is a final implementation spec. It defines the structure EasyNet-Cli must
be implemented as. It does not define a transition architecture.

Any directory not present in the Final Repository Layout is either:

1. implementation debt to be moved into the final tree;
2. generated/build output that must be ignored; or
3. a future change that requires a separate spec update.

The final tree is the product contract. Current checkout shape, compatibility
wrappers, and historical migration folders are not architecture.

## Final Repository Layout

```text
EasyNet-Cli/
├─ Cargo.toml
├─ Cargo.lock
├─ README.md
├─ build.rs
├─ include/
│  ├─ easynet_cli.h
│  ├─ easynet_cli.exports.v7
│  └─ easynet_cli.exports.v8
├─ src/
│  ├─ lib.rs
│  ├─ bin/
│  │  ├─ easynet.rs
│  │  ├─ easynet-daemon.rs
│  │  ├─ easynet-keyring.rs
│  │  ├─ gen-ability-tomls.rs
│  │  ├─ real-user-smoke.rs
│  │  └─ verify-voice-contract.rs
│  ├─ core/
│  │  ├─ agent/
│  │  ├─ identity/
│  │  ├─ ura/
│  │  └─ domain/
│  ├─ daemon/
│  │  ├─ boot/
│  │  ├─ control/
│  │  ├─ invocation/
│  │  │  ├─ admission/
│  │  │  ├─ routing/
│  │  │  ├─ dispatch/
│  │  │  ├─ receipts/
│  │  │  ├─ streams/
│  │  │  └─ bidi/
│  │  ├─ ability/
│  │  │  ├─ names/
│  │  │  ├─ descriptors/
│  │  │  ├─ authority/
│  │  │  ├─ impl_bindings/
│  │  │  ├─ catalog/
│  │  │  ├─ wire/
│  │  │  └─ builtins/
│  │  │     ├─ agents/
│  │  │     ├─ device_control/
│  │  │     ├─ resources/
│  │  │     ├─ automation/
│  │  │     ├─ integrations/
│  │  │     └─ governance/
│  │  ├─ execution/
│  │  │  ├─ pty/
│  │  │  ├─ mcp/
│  │  │  ├─ mission/
│  │  │  ├─ schedule/
│  │  │  ├─ loop_instance/
│  │  │  ├─ permission/
│  │  │  └─ session/
│  │  ├─ resources/
│  │  │  ├─ skills/
│  │  │  ├─ pages/
│  │  │  ├─ context/
│  │  │  ├─ files/
│  │  │  └─ media/
│  │  ├─ identity/
│  │  ├─ trust/
│  │  ├─ keyring/
│  │  ├─ federation/
│  │  ├─ plugins/
│  │  ├─ persistence/
│  │  ├─ axon_bridge/
│  │  └─ telemetry/
│  ├─ cli/
│  │  ├─ commands/
│  │  ├─ presentation/
│  │  ├─ daemon_client/
│  │  └─ mcp/
│  ├─ ffi/
│  │  ├─ daemon/
│  │  ├─ client/
│  │  ├─ invocation/
│  │  ├─ errors/
│  │  ├─ features/
│  │  └─ strings/
│  ├─ eal/
│  │  ├─ parser/
│  │  ├─ interpreter/
│  │  ├─ runtime/
│  │  └─ diagnostics/
│  └─ support/
│     ├─ async_bridge/
│     ├─ shellguard/
│     └─ platform/
├─ sdk/
│  ├─ go/
│  ├─ python/
│  ├─ node/
│  ├─ java/
│  ├─ swift/
│  ├─ rust/
│  ├─ schemas/
│  └─ conformance/
│     ├─ cases/
│     ├─ fixtures/
│     └─ runner/
├─ ability-descriptors/
│  └─ system/
│     ├─ agents/
│     ├─ federation/
│     ├─ device_control/
│     ├─ resources/
│     ├─ automation/
│     ├─ integrations/
│     └─ governance/
├─ provider_routes/
│  ├─ easynet-access-control-routes.v1.json
│  ├─ easynet-principal-lifecycle-routes.v1.json
│  ├─ easynet-receipt-routes.v1.json
│  ├─ easynet-runtime-admin-routes.v1.json
│  └─ generate_*.py / route_generator.py
├─ schemas/
│  ├─ descriptor/
│  ├─ receipt/
│  ├─ control_plane.proto
│  └─ common.proto
├─ plugins/
├─ skills/
├─ examples/
├─ gallery/
├─ docs/
├─ tests/
│  ├─ e2e/
│  ├─ conformance/
│  ├─ fixtures/
│  ├─ scripts/
│  └─ support/
├─ tools/
│  ├─ benches/
│  ├─ scripts/
│  └─ sdk-conformance-runner/
│     └─ src/
├─ packaging/
│  ├─ docker/
│  └─ release/
├─ pr/
└─ .github/
   └─ workflows/
```

## Global Invariants

1. No top-level `crates/` workspace split is part of this spec.
2. No top-level `engineering/`, `scripts/`, `demos/`, `runtime/`, or `services/`
   ownership root exists in the final tree.
3. `src/runtime/` is not a final source namespace.
4. `src/services/` is not a final source namespace.
5. `src/facade/` is not a final source namespace.
6. `sdk/rust/` is limited to provider/runtime SDK packages. Rust-facing daemon
   APIs remain in the main Rust package; `sdk/rust/` must not own product
   daemon lifecycle, route policy, or receipt authority.
7. Ability dispatch remains flat: `ability_name -> handler`. Product grouping
   is source organization, not a registry tree.
8. Ability wire names remain byte-identical unless a dedicated semantic-change
   spec says otherwise.
9. Axon owns canonical Invocation/Receipt wire semantics. EasyNet-Cli owns
   daemon product/device policy and local execution.
10. `AbilityDescriptor`, `AuthorityBinding`, `AbilityImpl`, and handler bodies
    remain separate concepts and separate module ownership.
11. Skills are implementation/resource packages, not protocol-callable objects.
12. Plugins are extension packages. A plugin may contribute descriptors,
    implementation bindings, sidecars, or runtime state; it is not a core
    daemon source module merely because it executes in process.
13. Ordinary list APIs are read-model queries. Distributed fan-out is explicit,
    bounded, and hosted by named daemon/hub aggregate abilities.

## Root Ownership

| Path | Owns | Must not own |
| --- | --- | --- |
| `Cargo.toml` | single Rust package manifest | workspace split without a new spec |
| `build.rs` | build-time generation/check hooks | product runtime policy |
| `include/` | exported ABI headers | FFI implementation or generated junk |
| `src/` | product Rust source | SDK language packages, docs, packaging |
| `sdk/` | language facades over daemon/libeasynet-cli semantics | daemon implementation, Axon canonical algorithms, hidden fan-out |
| `ability-descriptors/` | governed AbilityDescriptor TOMLs | generic assets or Rust source |
| `provider_routes/` | provider route manifests and generators consumed by Rust/Go/Python route constants | runtime state, generated bytecode, product workflows |
| `schemas/` | machine-readable contract schemas and protos | generated build output |
| `plugins/` | extension packages | core daemon modules |
| `skills/` | implementation-resource packages | protocol-callable Ability identity |
| `examples/` | source examples, demo fixtures, sample projects | conformance gates, release packaging |
| `gallery/` | showcase cases and authored assets | executable source ownership |
| `docs/` | human-readable architecture, specs, runbooks | generated build artifacts |
| `tests/` | Cargo integration tests, e2e, conformance, fixtures, test scripts/support | product modules |
| `tools/` | maintainer tools, benchmark entry points, descriptor generation wrappers, audits, repo checks | product runtime modules |
| `packaging/` | docker and release packaging | platform app source |
| `pr/` | checked-in delivery intent, invariants, and verification evidence | runtime source or generated output |
| `.github/workflows/` | GitHub Actions workflow entry points | non-GitHub CI implementation roots |

## Source Ownership

### `src/bin/`

Process entry points only. Binaries parse process-level flags, initialize
logging/runtime glue, and delegate immediately into `src/cli/`, `src/daemon/`,
or maintainer tool entry functions.

Allowed binaries:

- `easynet.rs`
- `easynet-daemon.rs`
- `easynet-keyring.rs`
- `gen-ability-tomls.rs`
- `real-user-smoke.rs`
- `verify-voice-contract.rs`

No daemon policy, ability dispatch, catalog construction, or reusable business
logic belongs in `src/bin/`.

### `src/core/`

Zero-dependency domain/value layer:

- `agent/`: core agent identity/spec value types.
- `identity/`: identity value objects and pure validation helpers.
- `ura/`: URA value objects, parsing wrappers, and route-independent helpers.
- `domain/`: shared domain primitives and newtypes.

`src/core/` must not know daemon sockets, plugin loading, filesystem layout,
Axon runtime boot, network dispatch, persistence, CLI rendering, or SDK
packaging.

### `src/daemon/`

Product/device daemon implementation. It owns identity, local resources,
plugin host, Mission/EAL orchestration, invocation routing, service state,
Hub/device mode, and daemon-owned AbilityImpl execution.

`src/daemon/boot/` owns daemon boot sequencing, mode selection, bootstrap
dependencies, listener startup, and shutdown handoff.

`src/daemon/control/` owns local boot/status/discovery IPC only. Product ability
calls go through Invocation, not control frames.

`src/daemon/invocation/` owns daemon Invocation admission, routing, dispatch,
receipt observation, unary/stream/bidi lifecycle, causal placement checks, and
daemon-local target resolution. It must preserve the complete Invocation tuple:
caller, callee, ability, subject, nonce, causal_context, args.

`src/daemon/ability/` owns daemon-local ability control-plane implementation:

- `names/`: stable public Ability wire-name constants and public metadata keys.
- `descriptors/`: descriptor surface/value helpers.
- `authority/`: advertise/invoke authority predicates and proofs.
- `impl_bindings/`: executable binding registration.
- `catalog/`: built-in catalog projection and descriptor rendering.
- `wire/`: local ability-to-wire-profile projection.
- `builtins/`: daemon-owned handler implementations grouped by product module.

`src/daemon/execution/` owns long-lived stateful managers used by handlers:
PTY, MCP, Mission/EAL execution, schedules, loop instances, permission state,
and sessions. Handlers call these services through explicit handles.

`src/daemon/resources/` owns daemon resource models and shared resource
projection helpers for skills, pages, context, files, media, and remote
desktop.

`src/daemon/identity/`, `trust/`, and `keyring/` own host identity, trust-anchor
state, key resolution, signing handles, and vault access.

`src/daemon/federation/` owns daemon federation posture, peer/hub adapters,
directory read models, publication, and forwarding wrappers. It does not own
Axon protocol semantics.

`src/daemon/plugins/` owns plugin manifest parsing, install/activation,
sidecar protocol, runtime contribution registration, and plugin runtime state.

`src/daemon/persistence/` owns daemon-local persistence stores and on-disk
schema coordination that are daemon-specific. Generic persistence helpers do
not get a separate root in final structure.

`src/daemon/axon_bridge/` is glue to Axon SDK/runtime types only. It must not
grow EasyNet product policy.

`src/daemon/telemetry/` owns daemon metrics, tracing adapters, operator logs,
and observability projection.

### `src/cli/`

Command-line facade and presentation:

- `commands/`: command tree and command handlers.
- `presentation/`: output rendering and formatting.
- `daemon_client/`: CLI-local daemon client adapters.
- `mcp/`: CLI front-door for MCP-facing command integration.

CLI code must not own daemon policy, hidden fan-out, Axon canonical algorithms,
descriptor versioning, receipt binding, or one-off transport semantics.

### `src/ffi/`

Rust/C ABI projection over daemon control and generic Invocation submission:

- `daemon/`: daemon lifecycle ABI.
- `client/`: client handle ABI.
- `invocation/`: complete Invocation submission/projection ABI.
- `errors/`: stable FFI error representation.
- `features/`: capability-neutral feature-discovery catalogue.
- `strings/`: allocation and string ownership ABI.

The stable model is the exact generic daemon/Invocation boundary in
`include/easynet_cli.exports.v7`, plus the feature-detected raw-stream
extension declared in `include/easynet_cli.exports.v8`. Identity, Directory,
Receipt, Publication, Host Binding, Mission, Events, Admin/Gateway, Surface,
Compatibility, Wrappers, and companion control remain language-SDK provider
responsibilities and must not grow corresponding FFI directories or exports.

### Product projection ownership

Product-specific JSON projections live inside the daemon domain that owns their
semantics. Companion status projection belongs to `daemon/plugins/companion/`,
OpenAI file projection belongs to the OpenAI compatibility ability, host-stream
framing belongs to its mission executor, and signer policy binding belongs to
daemon identity. Generic C ABI callback projection remains internal to
`src/ffi/`. There is no cross-domain product protocol layer.

### `src/eal/`

Mission/EAL language implementation:

- `parser/`: parsing and AST construction.
- `interpreter/`: execution-language evaluation.
- `runtime/`: EAL runtime support that is not daemon policy.
- `diagnostics/`: parse/runtime diagnostics.

Mission/EAL may orchestrate abilities, but it does not define Invocation tuple
construction, receipt binding, or Axon canonicalization.

### `src/support/`

Low-level helpers without product policy:

- `async_bridge/`
- `shellguard/`
- `platform/`

Support modules must not become a dumping ground for daemon state, product
policy, or hidden service ownership.

## Ability Built-in Classification

`src/daemon/ability/builtins/` groups daemon-owned/system ability handlers by
product meaning:

| Group | Owns | Must not own |
| --- | --- | --- |
| `agents/` | chat, history, discovery, invoke, lifecycle, list | formal Ability ownership semantics |
| `device_control/` | filesystem, file edit/transfer, terminal, process, shell, browser, HTTP, session, device ability management | locality decisions, transport admission |
| `resources/` | skills, pages, context, files store, media, voice, remote desktop resources | skill as protocol identity |
| `automation/` | mission, think, schedule, loop, discuss, orchestration | Invocation axiom or receipt semantics |
| `integrations/` | MCP, A2A, OpenAI compatibility, plugin wrappers, federation probe | alternate Axon dispatch path |
| `governance/` | consent, keyring wrapper, invocation history, health, network health, admin status, meta governance | long-lived service state |

Handler modules may own input/output DTOs, validation, entry functions,
registration functions, and handler-local tests. Long-lived managers live in
`src/daemon/execution/`, `src/daemon/resources/`, explicit daemon semantic
directories, or `src/daemon/persistence/`.

## Descriptor And Schema Contracts

Descriptor TOMLs are governed product contracts:

```text
ability-descriptors/system/
├─ agents/
├─ device_control/
├─ resources/
├─ automation/
├─ integrations/
└─ governance/
```

Descriptor lookup rules:

1. Code uses descriptor-root/path/iterator helpers.
2. Code must not concatenate `"ability-descriptors/system"` with a flat file
   name.
3. Code must not assume descriptors are flat under the system root.
4. Per-ability descriptor paths are grouped by product owner, not by string
   prefix.
5. Descriptor generation must be deterministic and byte-comparable.

Schemas live in:

```text
schemas/
├─ descriptor/
├─ receipt/
├─ control_plane.proto
└─ common.proto
```

`schemas/` is the contract root for descriptor validation, receipt validation,
control-plane proto, common proto definitions, FFI generation, packaging, and
conformance checks.

## SDK Contract

Final SDK roots:

```text
sdk/
├─ go/
├─ python/
├─ node/
├─ java/
└─ swift/
```

SDKs are language facades over daemon/libeasynet-cli semantics. They may expose
idiomatic object models, builders, generated DTOs, lifecycle handles,
Invocation builders, directory pages, stream/bidi handles, and typed errors.

SDKs must not:

1. own daemon policy;
2. implement Axon canonical algorithms;
3. run hidden governed fan-out loops;
4. depend on handler-module paths;
5. make one method per Ability the stable API model;
6. start or own Axon protocol runtime as a product shortcut.

## Complexity Contract

Project structure must protect algorithmic behavior. Ordinary list/discovery
methods are read-model queries, not distributed live fan-out.

| Surface | Required data source | Complexity target | Forbidden default implementation |
| --- | --- | --- | --- |
| `agent.list` | local hosted-agent registry or hub directory projection | `O(page_size + filter_cost)` | invoking each agent |
| `node.list` | daemon/hub directory read model indexed by realm/user/device | `O(page_size + filter_cost)` | dialing every device |
| `meta.list_abilities` | ability catalog projection keyed by owner/descriptor/filter | `O(page_size + filter_cost)` | calling every agent |
| `meta.list_resources` | local resource projection or indexed resource snapshot | `O(page_size + filter_cost)` | probing every resource backend live |
| `skill.list` | managed skill index or bounded local scan | `O(page_size + filter_cost)` | network-wide skill discovery |
| `skill.tree` | one selected skill root | `O(nodes_returned)` with max depth/page limits | scanning all skills by default |
| `plugin.status` | plugin runtime/load-plan snapshot | `O(plugin_count)` local only | starting plugin sidecars for status |
| `mcp.client.list` | configured MCP client snapshot or explicit refresh | `O(page_size + filter_cost)` for snapshot reads | unbounded live listing across servers |
| `namespace.resolve` / `federation.resolve` | indexed namespace/directory lookup | `O(log n + result_count)` or documented equivalent | scanning all devices |
| directory `Subscribe` | snapshot cursor plus delta log | `O(snapshot_page + delta_count)` per page | replaying full catalog for each delta |

Every public list surface must define cursor pagination, `DefaultPageSize`,
`MaxPageSize`, deterministic sort order, stale-read-model behavior, typed
invalid-cursor errors, typed oversized-page errors, and no implicit network
fan-out.

## Fan-out And Facade Rules

Ordinary facade methods do not fan out governed ability calls across devices,
agents, or abilities.

Rules:

1. CLI, SDK, FFI, and backend adapters must not implement per-target governed
   fan-out loops.
2. Fleet-wide aggregation belongs in a named daemon/hub aggregate ability.
3. Aggregate abilities create one parent Invocation and child Invocations for
   governed child calls.
4. Aggregate abilities expose max concurrency, deadline, page size,
   partial-result semantics, and per-target typed errors.
5. Aggregate results include completed child receipt refs when child calls
   reach terminal states.
6. A facade helper may call one aggregate ability, but its name must reveal
   aggregation.
7. Default list calls return catalog/directory projections already known to the
   daemon or hub. They do not trigger live remote discovery.

Aggregate fan-out state machine:

```text
Planned -> Dispatching -> Collecting -> Completed
                              |             |
                              v             v
                           Partial       Failed
                              |
                              v
                           TimedOut
```

## Final Implementation Plan

This plan is an implementation order, not a transition architecture. Each phase
must leave the repository closer to the final tree and must not introduce a
new permanent root.

1. Freeze this spec as the only project-structure authority.
2. Add or update the final structure guard so it checks the Final Repository
   Layout exactly.
3. Create missing final roots: `tools/`, `packaging/docker/`,
   `packaging/release/`, SDK language roots, and final source subdirectories.
4. Move maintainer/audit/generator scripts into `tools/`.
5. Move docker/release harnesses into `packaging/{docker,release}/`.
6. Fold root `demos/` into `examples/`, `docs/`, or `tools/` by function.
7. Remove root `engineering/` and root `scripts/` as ownership roots.
8. Reshape `src/core/` into `agent/`, `identity/`, `ura/`, and `domain/`;
   daemon-imported ability manifests belong to the daemon catalogue boundary.
9. Reshape `src/daemon/` into the final semantic directories, especially
   `boot/`, `invocation/{admission,routing,dispatch,receipts,streams,bidi}`,
   `execution/{pty,mcp,mission,schedule,loop_instance,permission,session}`,
   and daemon-local `persistence/`.
10. Reshape `src/cli/`, `src/ffi/`, `src/eal/`, and `src/support/` into their
    final subdirectories.
11. Remove final-forbidden source roots: `src/runtime/`, `src/services/`, and
    `src/facade/`.
12. Prove descriptor and schema roots match this spec.
13. Run compile, descriptor, ability-name, fan-out, and final-structure gates.

No phase may combine structural moves with intentional Ability semantic
changes. If a semantic change is needed, write a separate spec first.

## Verification Matrix

| Gate | Required evidence | Fails when |
| --- | --- | --- |
| Final layout | Final guard proves all required roots exist and forbidden roots are absent | `engineering/`, root `scripts/`, root `demos/`, `crates/`, `src/runtime/`, `src/services/`, or `src/facade/` remains |
| Source layout | Guard or review proves every `src/` subtree matches Final Repository Layout | source remains in legacy or near-synonym buckets |
| SDK layout | `sdk/{go,python,node,java,swift}/` exist and no undeclared SDK root exists | SDK root owns daemon policy or hidden fan-out |
| Ability names | before/after public Ability-name snapshot | wire names drift unintentionally |
| Catalog stability | before/after sorted `meta.list_abilities` snapshot | names disappear, duplicate, or reorder nondeterministically |
| Descriptor drift | deterministic descriptor generation diff | descriptor bytes drift without a descriptor-format spec |
| Descriptor paths | search proves code uses descriptor-root/path/iterator helpers | code assumes flat descriptor files |
| Complexity | list surfaces prove pagination, sort, stale model, and page errors | list calls perform hidden live fan-out |
| Facade fan-out | audit of CLI/SDK/FFI/backend adapters | facade loops over governed targets |
| Compile | `cargo fmt --check` and a phase-appropriate compile gate | formatting or compile fails |
| Hygiene | tracked files contain no OS/editor artifacts | `.DS_Store`, `Thumbs.db`, or similar files are tracked |

## Acceptance Criteria

Final structure:

1. The repository tree matches Final Repository Layout.
2. `build.rs` exists at repository root.
3. `include/easynet_cli.h` exists.
4. `tools/` exists and owns maintainer/audit/generator tooling.
5. `packaging/docker/` and `packaging/release/` exist.
6. `sdk/go/`, `sdk/python/`, `sdk/node/`, `sdk/java/`, `sdk/swift/`, and
   `sdk/rust/` exist.
7. `sdk/rust/` contains only provider/runtime SDK packages; it does not own
   EasyNet product daemon behavior.
8. No top-level `engineering/`, `scripts/`, `demos/`, or `crates/` exists.
9. No `src/runtime/`, `src/services/`, or `src/facade/` exists.
10. `.github/workflows/` remains at repository root.

Behavior:

1. Existing public Ability names remain byte-identical.
2. Ability call modes remain unchanged.
3. `meta.list_abilities` returns the same ability names before and after
   structural moves.
4. Descriptor generation output remains byte-identical unless the phase is a
   descriptor-format change.
5. No structural move changes Invocation or Receipt semantics.

Boundary:

1. Transport and daemon surfaces import ability names from
   `daemon::ability::names` or typed service contracts, not handler modules.
2. Plugin code does not depend on core handler-module paths for public wire
   constants.
3. Descriptor lookup uses root/path/iterator helpers.
4. Skills are not treated as protocol-callable objects unless wrapped by an
   explicit AbilityDescriptor.
5. Axon protocol semantics are not duplicated in CLI, backend, FFI, SDK, or
   daemon product code.

Quality:

1. Stateful structs keep private fields and constructor-injected dependencies.
2. Public fallible module boundaries use typed errors.
3. Handler modules do not own long-lived services.
4. Reusable managers live under the correct daemon semantic directory.
5. No hidden per-target governed fan-out exists in facade layers.
6. Every final guard and compile gate passes.

## Review Checklist

1. Does the tree exactly match Final Repository Layout?
2. Did any final-forbidden root remain or reappear?
3. Does each module name describe semantic ownership instead of a convenient
   bucket?
4. Did any move collapse AbilityDescriptor, AuthorityBinding, AbilityImpl, and
   handler body ownership?
5. Are public wire constants centralized without moving private constants?
6. Are list methods paginated, sorted, bounded, and read-model backed?
7. Is any facade doing hidden governed fan-out?
8. Does every aggregate ability expose concurrency, deadline, partial-result,
   and child-receipt semantics?
9. Are descriptor roots resolved through one helper family?
10. Do plugin packages remain plugin packages?
11. Do skills remain implementation/resource packages?
12. Does any handler own long-lived state that belongs in `execution/`,
    `resources/`, explicit daemon semantic directories, or daemon
    `persistence/`?
13. Did any CLI/SDK/backend-facing code import Axon internals or duplicate Axon
    canonical algorithms?
14. Is there a caller inventory for every moved path or symbol?
15. Does the phase compile independently?
16. Does the final structure guard enforce this spec instead of current
    historical layout?
