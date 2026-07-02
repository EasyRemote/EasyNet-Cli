# Project Structure v1

Status: Active

Last reviewed: 2026-07-02

Scope: EasyNet-Cli repository layout, Rust module ownership, daemon-owned
ability source-code grouping, stable ability-name placement, descriptor contract
placement, migration order, complexity contracts, facade/fan-out rules, and
review gates for structural moves.

Non-goal: This spec does not change Ability wire names, Invocation semantics,
Receipt semantics, URA syntax, Axon admission semantics, plugin runtime
behavior, descriptor schemas, or backend product APIs.

## Normative Sources

This spec is a structure and migration contract. When it discusses ontology or
runtime semantics, these sources are higher priority:

1. `docs/easynet_ontology.tex` for Ability versus Skill, Agent as public
   methods/private fields, and `agent send` as default-ability sugar.
2. `docs/spec/daemon-sdk-requirements-v1.md` for daemon SDK object model,
   directory/listing complexity, facade rules, and aggregate fan-out state
   machines.
3. `src/daemon/ability/mod.rs`, `src/daemon/ability/dispatch.rs`,
   `src/daemon/ability/wire/mod.rs`, and `src/daemon/axon_bridge/` for the current
   daemon-side split among AbilityDescriptor, AuthorityBinding, AbilityImpl,
   dispatch compatibility, wire-profile lookup, and Axon glue.
4. `src/daemon/control/` for local boot/status IPC, `src/daemon/invocation/`
   for daemon-owned Invocation transport/admission/session dispatch and state,
   `src/daemon/federation/` for daemon-owned outbound peer-hub dialing,
   cross-realm directory projection, peer-map reload state, federation
   discovery read boundaries, and federation read models,
   `src/daemon/trust/` for trust-anchor state and Axon key-resolution adapters,
   `src/daemon/identity/` and `src/daemon/keyring/` for host identity signing
   and vault state, `src/daemon/ability/` for daemon ability support services,
   and `src/daemon/context/` for daemon-owned context capture services.

## Review Findings Grouped By Root Cause

The previous draft was useful as a directory sketch, but it was not yet an
industrial migration spec. The issues below are grouped by root cause so one
fix closes each pattern instead of repeating the same symptom per file.

### 1. Directory Shape Was Treated As Semantics

The old draft made `src/runtime/abilities/` and `src/runtime/ability_runtime/`
look like semantic owners. That is wrong for the current codebase. The semantic
owners already exist:

- `daemon/ability/` owns the daemon-local control-plane model:
  AbilityDescriptor, AuthorityBinding, AbilityImpl, and their registries.
- `daemon/ability/dispatch.rs` is a compatibility facade over the catalog and
  handler registration path, not a new protocol layer.
- `daemon/ability/wire/mod.rs` owns daemon wire-profile lookup for bidi/session
  bridges.
- `daemon/axon_bridge/` is glue to Axon SDK types and must not grow EasyNet
  product policy.

Fix: this spec now treats product handler grouping as source organization only.
It must not create a parallel runtime ontology or duplicate Axon/EasyNet control
plane semantics.

### 2. The Proposed `runtime/abilities/` Collided With Agent Ability Specs

An earlier checkout had `src/runtime/abilities.rs`, which enumerated abilities
published by locally registered agents, especially the default `chat` surface.
Creating a directory named `src/runtime/abilities/` would have collided with
that module name and blurred two concepts:

- agent-published ability specs, now in `runtime/agent_ability_specs.rs`;
- daemon-owned/system ability handler implementations, now in
  `daemon/ability/builtins/`.

Fix: the migration product-handler directory is `daemon/ability/builtins/`,
not `runtime/abilities/`. Clean Final uses `src/daemon/ability/builtins/`.
The old `runtime/abilities.rs` compatibility re-export has been retired; new
production code must import `runtime/agent_ability_specs.rs` directly.

### 3. Registry, Catalog, Descriptor, And Handler Ownership Were Collapsed

The previous draft used `ability_registry` for catalog construction while the
current code already has `daemon/ability/control_plane.rs` for control-plane
registration. Reusing the word `registry` for both would make call sites harder
to audit.

Fix: descriptor/table generation and system ability catalog assembly now live
under `src/daemon/ability/catalog/`, matching the Clean Final daemon ability
layout. The word `registry` remains reserved for control-plane registries
unless a module is explicitly named as a compatibility adapter.

### 4. Stable Names Were Not Separated From Private Constants

Moving all constants would be overreach. Some constants are public Ability wire
names; others are private error reasons, environment variable names, profile
versions, internal stream IDs, test fixtures, or plugin-local implementation
details.

Fix: `daemon/ability/names/` is only for stable public Ability wire names and
stable metadata keys used across modules. Private constants remain with their
owning implementation module.

### 5. Complexity And Fan-out Were Missing

A project-structure spec that moves `agent.list`, `meta.list_abilities`,
discovery, catalog projection, and SDK/facade surfaces must define complexity.
Otherwise a harmless-looking move can hide `O(devices * agents * abilities)`
network fan-out behind a list call.

Fix: this spec now pins list APIs to read models and requires named aggregate
abilities for explicit fleet-wide fan-out. Ordinary facade list methods must be
`O(page_size + filter_cost)`, not live per-agent/per-device loops.

### 6. Facade Boundaries Were Under-specified

CLI, FFI, language SDKs, and backend-facing helpers must not become hidden
dispatch engines. An ergonomic surface is allowed to marshal requests, expose
builders, and render CLI/UI output. It must not own daemon policy, fan-out,
admission, receipt binding, descriptor versioning, or Axon canonicalization.

Fix: this spec now defines ergonomic-surface import and behavior rules. These
surfaces call daemon directory/read-model APIs or one named aggregate ability;
they do not loop over targets and invoke governed abilities directly.

### 7. Migration Was Not Grounded In The Current Checkout

The old draft listed moves from `runtime/agents/`, but did not acknowledge the
current adjacent modules such as `daemon/ability/`,
`daemon/ability/dispatch.rs`, `daemon/ability/wire/mod.rs`,
`runtime/agent_ability_specs.rs`, daemon semantic
directories, and plugin packages.

Fix: every migration phase now has a caller-inventory gate before code moves,
and high-risk moves are sequenced after name extraction, path compatibility, and
baseline snapshots.

### 8. Compatibility Was Open-ended

"Preserve old public module paths through re-exports" is necessary, but without
a retirement rule it creates permanent duplicate APIs.

Fix: compatibility re-exports are phase-scoped. A phase must state whether a
compat path is permanent, deprecated, or removed by a named cleanup phase. New
production code must not import through compatibility paths after the move.

### 9. Descriptor Contract Movement Lacked A Root Abstraction Gate

Moving `abilities/system/` to `ability-descriptors/system/` while code
still concatenates string paths would create packaging drift and runtime misses.

Fix: descriptor contract files must move only after a single descriptor-root
helper lands and every descriptor caller uses it.

### 10. Plugin And Skill Boundaries Needed Stronger Language

Skills are resources, not protocol-callable entities. Plugins are extension
packages that may contribute AbilityImpl bindings; they are not core runtime
modules merely because their handlers are loaded in-process.

Fix: `skills/` and `plugins/` remain ecosystem-facing top-level directories.
Plugin package code keeps package-local constants unless a constant becomes a
stable cross-module core contract.

## Global And Migration Invariants

1. Ability dispatch remains flat: `ability_name -> handler`. No runtime registry
   tree is introduced by product-module grouping.
2. Ability wire names remain byte-identical.
3. Product-module grouping is source organization only. It does not define
   Invocation, Receipt, URA, descriptor version, or Axon admission semantics.
4. Axon owns canonical Invocation/Receipt wire semantics. EasyNet-Cli owns
   daemon product/device policy and local execution.
5. `daemon/ability/` remains the owner of daemon-local AbilityDescriptor,
   AuthorityBinding, and AbilityImpl registration.
6. `daemon/ability/builtins/` owns daemon-owned ability handlers only.
7. `runtime/agents/` was the temporary compatibility facade during migration;
   it is now retired and must not return.
8. `runtime/agent_ability_specs.rs` owns locally registered agent ability specs;
   the old `runtime::abilities` compatibility path is retired.
9. Skills remain implementation/resource packages. They are callable only when
   an explicit AbilityDescriptor exposes a wrapper.
10. Plugins remain extension packages. A plugin contributes descriptors,
    implementation bindings, sidecars, or runtime state; it does not become a
    core source module by directory move.
11. The retired `services/` catch-all must not return; daemon state belongs in
    explicit daemon semantic directories.
12. Ordinary list APIs are read-model queries. Distributed fan-out is explicit,
    bounded, and hosted by daemon/hub aggregate abilities.

## Current And Migration Layer Ownership

This table describes the current checkout and migration-compatible staging
names. Clean Final ownership is defined later under `src/daemon/`,
`src/cli/`, `src/ffi/`, `sdk/`, and top-level contract/tooling directories.

| Layer | Owns | Must not own |
| --- | --- | --- |
| `core/` | zero-dependency domain/value types | filesystem walking, daemon policy, transport |
| `daemon/ability/{descriptors,authority,impl_bindings,control_plane}` | AbilityDescriptor, AuthorityBinding, AbilityImpl, control-plane registration | product handler bodies, plugin process management |
| `daemon/ability/dispatch.rs` | compatibility catalog facade and handler registration bridge | new protocol semantics, product policy branching |
| `daemon/ability/wire/mod.rs` | local ability-to-wire-profile projection | plugin package loading, transport sessions |
| `daemon/ability/catalog/` | built-in catalog assembly, catalog metadata, profile descriptors, descriptor TOML rendering | handler implementation bodies |
| `daemon/ability/builtins/` | daemon-owned/system ability handlers grouped by product module | transport admission, receipt canonicalization, persistent service state |
| `runtime/executors/` | reusable execution engines used by handlers or manifest-bound abilities | public ability registration |
| `daemon/execution/` | stateful runtime services such as PTY, schedule, loop, permission, session, MCP execution state | descriptor identity, facade rendering |
| `daemon/resources/` | resource models and shared resource projection helpers | product ability handler bodies |
| `daemon/plugins/` | plugin manifest parsing, install/activation, sidecar protocol, daemon contribution registration | core Ability ontology |
| `daemon/axon_bridge/` | Axon SDK glue and wire/type adapters | EasyNet product policy, plugin lifecycle decisions |
| `daemon/federation/read_model/` | federation resolver/catalog read models such as advertised agents, owner ability projections, and hub-published abilities | Axon protocol authority, transport dialing, product handler bodies |
| `daemon/ability/health.rs` | daemon-owned ability support services such as manifest ability health monitoring | handler implementation bodies, plugin process ownership |
| `daemon/ability/names/` | stable public Ability wire-name constants and stable public metadata keys | private reason strings, test fixtures, env vars |
| `daemon/context/` | daemon-owned background loops for the local Context surface | context persistence format, product ability handler bodies |
| `daemon/` | Rust daemon SDK facade, local boot/status IPC under `daemon/control`, daemon-owned Invocation transport under `daemon/invocation`, and daemon semantic state directories | Axon protocol semantics, one method per ability, generic service catch-all ownership |
| `cli/` | command-line facade, clap command tree, presentation, and CLI-only ergonomic adapters | daemon policy, hidden fan-out loops, Axon canonical algorithms |
| `facade/` | retired legacy compatibility namespace | all new ownership; use `cli/`, `ffi/`, `daemon/`, or future `sdk/` |
| `ffi/` | C ABI projection over daemon/client surfaces | one method per ability as the stable model |
| `registry/` | local agent registry/config projection | runtime ability descriptor ownership |
| `persistence/` | storage formats and file-backed stores | network dispatch decisions |
| `eal/` | Mission/EAL parsing and orchestration language | Invocation tuple construction rules |

## How To Read This Spec

This spec has two architecture targets:

1. **Clean Final** is the desired end state. It keeps the current Rust package
   model, adds future `sdk/` facades, removes `runtime/` and `services/`
   catch-alls from daemon source, and treats AbilityDescriptor TOMLs as product
   contracts.
2. **Migration-Compatible** is a staging layout for the current checkout. It
   exists only to avoid breaking every import in one change.

If Clean Final and Migration-Compatible sections appear to conflict, Clean
Final wins for product direction and Migration-Compatible wins only for
sequencing an incremental PR. A review must reject any change that presents a
Migration-Compatible name as permanent product architecture.

## Clean Final Repository Layout

This is the expected optimal end state for the EasyNet-Cli repository. It keeps
the current Rust package model and cleans the source ownership first. A Cargo
workspace split is not part of this target by default; introduce workspace
crates only after a separate decision proves a real compile-time, publication,
or external-reuse boundary.

```text
EasyNet-Cli/
├─ Cargo.toml
├─ Cargo.lock
├─ README.md
├─ src/
│  ├─ lib.rs
│  ├─ bin/
│  ├─ core/
│  ├─ daemon/
│  ├─ cli/
│  ├─ ffi/
│  ├─ eal/
│  └─ support/
├─ sdk/
│  ├─ rust/
│  ├─ go/
│  ├─ python/
│  ├─ node/
│  ├─ java/
│  └─ swift/
├─ ability-descriptors/
│  └─ system/
├─ schemas/
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
├─ benches/
├─ tools/
├─ packaging/
│  ├─ docker/
│  └─ release/
├─ platforms/
│  ├─ macos/
│  └─ windows/
└─ .github/
```

Ownership rules:

- `src/bin/` contains thin process entry points only: `easynet`,
  `easynet-daemon`, keyring, smoke binaries, and maintainer generators. Bins
  parse process-level flags and delegate immediately.
- `src/core/` owns zero-dependency EasyNet domain/value types only.
  It must not know daemon sockets, plugin loading, filesystem layouts, or Axon
  runtime boot.
- `src/daemon/` owns the product/device daemon: identity, local
  resources, plugin host, Mission/EAL orchestration, invocation routing,
  service state, Hub/device mode, and daemon-owned AbilityImpl execution.
- `src/cli/` owns the command-line facade and presentation. It calls
  daemon APIs; it does not own daemon policy or fan-out.
- `src/ffi/` owns the Rust/C ABI projection for daemon control and generic
  Invocation submission. It must not expose one method per ability as the
  stable model.
- `src/eal/` owns Mission/EAL parsing and execution-language semantics. It does
  not own Invocation tuple construction or receipt binding.
- `src/support/` owns low-level helpers with no product policy.
- `sdk/` is reserved for future language facades over daemon/libeasynet-cli
  semantics. SDKs may expose idiomatic object models, builders, packaging,
  generated DTOs, and local daemon transport helpers. SDKs must not own daemon
  policy, Axon canonical algorithms, hidden fan-out loops, or a stable API
  model based only on one method per ability.
- `sdk/rust/` is the primary idiomatic Rust daemon SDK facade. It is not the
  daemon implementation module and not the C ABI layer. It may wrap stable
  daemon client types, lifecycle handles, Invocation builders, directory pages,
  stream/bidi handles, and typed errors.
- `tests/support/` owns reusable integration fixtures, fake stores, daemon
  harnesses, and conformance helpers.
- `tools/` owns repeatable repo tasks that should not ship in the product
  library: descriptor generation wrappers, conformance snapshots, packaging
  checks, and migration audits.
- `ability-descriptors/` contains governed AbilityDescriptor TOMLs. They are
  product contracts, not generic static assets and not Rust source.
- `schemas/` contains machine-readable schemas used by descriptor generation,
  receipt validation, FFI headers, packaging, and conformance checks.
- `plugins/` and `skills/` remain ecosystem-facing packages.
- `packaging/` contains release and container packaging. Platform app/tray/
  launcher source stays under `platforms/`.
- `.github/workflows/` remains at repository root because GitHub requires it.

## Clean Final Daemon Source Layout

```text
src/daemon/
├─ boot/
├─ control/
├─ invocation/
│  ├─ admission/
│  ├─ routing/
│  ├─ local_runtime_adapter.rs
│  ├─ dispatch/
│  ├─ receipts/
│  ├─ streams/
│  └─ bidi/
├─ ability/
│  ├─ names/
│  ├─ descriptors/
│  ├─ authority/
│  ├─ impl_bindings/
│  ├─ catalog/
│  ├─ wire/
│  └─ builtins/
│     ├─ agents/
│     ├─ device_control/
│     ├─ resources/
│     ├─ automation/
│     ├─ integrations/
│     └─ governance/
├─ execution/
│  ├─ pty/
│  ├─ mcp/
│  ├─ mission/
│  ├─ schedule/
│  ├─ loop_instance/
│  ├─ permission/
│  └─ session/
├─ resources/
│  ├─ skills/
│  ├─ pages/
│  ├─ context/
│  ├─ files/
│  ├─ media/
│  └─ remote_desktop/
├─ identity/
├─ trust/
├─ keyring/
├─ federation/
├─ plugins/
├─ persistence/
├─ axon_bridge/
└─ telemetry/
```

Clean-final rules:

1. No top-level `runtime/` catch-all inside the daemon source tree.
2. No top-level `services/` catch-all inside the daemon source tree.
3. No `system_abilities/` name in the clean final daemon source tree; daemon
   built-ins live under `ability/builtins/` because their semantic owner is
   Ability execution.
4. `ability/descriptors`, `ability/authority`, and `ability/impl_bindings`
   remain separate. A handler body must not become the descriptor owner.
5. `invocation/` owns admission, routing, dispatch, receipts, streams, and bidi
   session semantics, including the daemon-local adapter that translates
   resolved `InvocationTarget`s into Axon `LocalRuntime` unary, stream, and
   bidi calls. It does not own product handler bodies.
6. `execution/` owns long-lived stateful managers. Built-in handlers use typed
   handles into `execution/`, not process-global mutable state.
7. `axon_bridge/` is glue to Axon SDK/runtime types only. It must not grow
   EasyNet product policy.
8. `control/` is boot/status/discovery only. Product ability calls go through
   Invocation.

## Migration-Compatible Intermediate Repository Layout

This layout is not the optimal end state. It exists only to migrate the current
single-crate repository without breaking all call sites in one change.

```text
EasyNet-Cli/
├─ Cargo.toml
├─ Cargo.lock
├─ README.md
├─ src/
│  ├─ core/
│  ├─ daemon/
│  ├─ cli/
│  ├─ runtime/
│  ├─ persistence/
│  ├─ registry/
│  ├─ eal/
│  ├─ ffi/
│  ├─ plugins/
│  └─ support/
├─ ability-descriptors/
│  └─ system/
├─ schemas/
├─ plugins/
├─ skills/
├─ examples/
├─ gallery/
├─ engineering/
│  ├─ docker/
│  ├─ scripts/
│  ├─ tests/
│  ├─ benches/
│  └─ ci/
├─ platforms/
│  ├─ macos/
│  └─ windows/
├─ docs/
├─ tests/
├─ benches/
├─ scripts/
└─ .github/
```

Root compatibility entries:

- `tests/` remains for Cargo integration-test discovery. It may contain thin
  wrappers over `engineering/tests/`, but must not be removed until `cargo test`
  behavior is proven. Shell guard bodies live under `engineering/tests/scripts/`;
  `tests/scripts/*.sh` is only the contributor- and Cargo-facing wrapper layer.
- `benches/` remains for Cargo benchmark discovery. It may contain thin
  wrappers over `engineering/benches/`.
- `scripts/` may remain as compatibility wrappers while CI/docs migrate to
  `engineering/scripts/`.
- `.github/workflows/` remains because GitHub Actions requires this root path.
- `abilities/system/` is retired after the descriptor-root abstraction lands.
  The current governed descriptor contract root is `ability-descriptors/system/`.

## Migration-Compatible Runtime And Daemon Layout

This is the single-package staging shape for the current checkout. It is not
the clean final daemon source layout.

```text
src/runtime/
├─ agent_ability_specs.rs
├─ executors/
└─ keyring/
```

```text
src/daemon/
├─ ability/
│  ├─ authority/
│  ├─ descriptors/
│  │  ├─ mod.rs
│  │  └─ surface.rs
│  ├─ dispatch.rs
│  ├─ impl_bindings/
│  ├─ control_plane.rs
│  ├─ control_plane_error.rs
│  ├─ catalog/
│  │  ├─ mod.rs
│  │  ├─ build.rs
│  │  ├─ catalog_metadata.rs
│  │  ├─ profiles/
│  │  ├─ ability_toml.rs
│  │  └─ system_manifest.rs
│  ├─ builtins/
│  │  ├─ mod.rs
│  │  ├─ agents/
│  │  ├─ device_control/
│  │  ├─ resources/
│  │  ├─ automation/
│  │  ├─ integrations/
│  │  └─ governance/
│  ├─ health.rs
│  ├─ conformance.rs
│  ├─ wire/
│  └─ names/
├─ identity/
│  ├─ local_invocation.rs
│  └─ self_identity.rs
├─ kernel/
│  ├─ mod.rs
│  └─ api.rs
├─ invocation/
│  ├─ target.rs
│  ├─ runtime_record.rs
│  ├─ receipt_subscriber.rs
│  └─ local_runtime_invoker.rs
├─ federation/
│  ├─ advertise.rs
│  ├─ publish.rs
│  ├─ init/
│  │  ├─ mod.rs
│  │  ├─ outcome.rs
│  │  ├─ probe.rs
│  │  └─ resolver_seed.rs
│  ├─ client/
│  │  └─ ability_contract.rs
│  ├─ read_model/
│  │  ├─ ability_catalog.rs
│  │  ├─ advertised_agents.rs
│  │  ├─ hub_published_abilities.rs
│  │  └─ owner_projection.rs
│  ├─ directory.rs
│  ├─ directory_reader.rs
│  ├─ gateway.rs
│  ├─ gateway_api.rs
│  ├─ peers.rs
│  └─ resolver.rs
├─ hub/
│  ├─ mod.rs
│  ├─ pages_listener.rs
│  └─ pages_serve_ability.rs
└─ axon_bridge/
```

Notes:

- `runtime/ability_runtime/` must not be introduced. It duplicates the existing
  `daemon/ability/`, `daemon/ability/dispatch.rs`,
  `daemon/ability/wire/mod.rs`, and `daemon/axon_bridge/` ownership split.
- `daemon/ability/builtins/` is used instead of `runtime/abilities/` to avoid
  reintroducing the retired agent-ability-specs module name and to state that
  these are daemon-owned/system handlers.
- `runtime/agents/` is retired after handlers move. New handlers must land in
  `daemon/ability/builtins/`.
- `runtime/abilities.rs` has become `runtime/agent_ability_specs.rs`; the
  `runtime::abilities` compatibility export is retired and must not return.
- `daemon/invocation/local_runtime_invoker.rs` owns the daemon-local Axon
  `LocalRuntime` adapter. The retired `runtime/local_runtime_invoker.rs`
  path and `runtime::local_runtime_invoker` import must not return.
- `daemon/invocation/target.rs` owns stage-1 target resolution and the
  `InvocationTarget` value objects consumed by daemon dispatch. The retired
  `runtime/invocation_target.rs` path and `runtime::invocation_target`
  import must not return.
- `daemon/invocation/runtime_record.rs` owns the daemon-local
  `RuntimeInvocation` adapter record and receipt projection used by
  daemon-internal Kernel calls. The retired `runtime/invocation.rs` path and
  `runtime::invocation` import must not return.
- `daemon/invocation/receipt_subscriber.rs` owns the receipt-observer
  extension surface. The retired `runtime/receipt_subscriber.rs` path and
  `runtime::receipt_subscriber` import must not return.
- `daemon/kernel/` owns the daemon execution kernel and its syscall-style API.
  The retired `runtime/kernel.rs`, `runtime/kernel_api.rs`,
  `runtime::kernel`, and `runtime::kernel_api` paths must not return.
- `daemon/federation/gateway.rs` and `daemon/federation/gateway_api.rs` own
  the daemon kernel's federation lifecycle/discovery adapter. The retired
  `runtime/gateway.rs`, `runtime/gateway_api.rs`, `runtime::gateway`, and
  `runtime::gateway_api` paths must not return.
- `daemon/federation/read_model/owner_projection.rs` owns the owner-keyed
  ability projection read model used by federation advertise/resolve and
  session heartbeat refresh. The retired `runtime/owner_projection.rs` path
  and `runtime::owner_projection` import must not return.
- `daemon/federation/client/ability_contract.rs` owns typed
  argument/response DTOs for hub-profile `federation.*` abilities. The retired
  `runtime/federation_client.rs` path and `runtime::federation_client` import
  must not return.
- `daemon/federation/advertise.rs` owns typed federation advertise, heartbeat,
  resolve, resolve_key, revoke, and forward_invoke ability calls. The retired
  `runtime/advertise.rs` path and `runtime::advertise` import must not return.
- `daemon/federation/publish.rs` owns daemon federation publish orchestration:
  local-agent bootstrap, self-identity bootstrap calls, runtime-local tool
  registration, advertise batching, descriptor publication, and revoke. The
  retired `runtime/publish.rs` path and `runtime::publish` import must not
  return.
- `daemon/federation/init/` owns the daemon federation initialization state
  machine, operator-facing status probe, typed terminal outcomes, and resolver
  seed loader. The retired `runtime/federation_init/` path and
  `runtime::federation_init` import must not return.
- `daemon/federation/resolver.rs` owns realm-suffix to federation posture
  resolution and canonical device-URA helper logic. The retired
  `runtime/resolver/` path and `runtime::resolver` import must not return.
- `daemon/hub/` owns in-daemon Hub-side HTTP adapter surfaces such as the
  Pages reference listener and `pages.serve` transport adapter. The retired
  `runtime/hub/` path and `runtime::hub` import must not return.

## CLI Boundary

`src/cli/` owns the command-line user surface during migration and in Clean
Final. It contains clap command definitions, command-group routing,
presentation helpers, and CLI-only ergonomic adapters.

`src/facade/`, `src/facade/cli/`, and `crate::facade::cli` are retired. Active
production code, tests, scripts, and docs for current behavior must use
`src/cli/` or `crate::cli`.

Rules:

1. New CLI modules land under `src/cli/`.
2. New production imports must use `crate::cli` or `easynet_cli::cli`.
3. MCP CLI edges live under `src/cli/`; MCP descriptor/tool projection and
   dispatch into daemon Invocation live under
   `daemon::ability::catalog::profiles::mcp`.
4. No file may be added under `src/facade/`.
5. Current scripts and boundary guards must refer to `src/cli/...` paths.

## Daemon SDK Root Boundary

`src/daemon/mod.rs` is the public Rust daemon SDK root. It exposes lifecycle
and Invocation-client types such as `DaemonStartConfig`, `DaemonHandle`,
`DaemonClient`, and `DaemonInvocation`.

`src/daemon.rs` is retired. The daemon SDK is directory-shaped so future daemon
SDK modules can stay under `src/daemon/` without reintroducing a root
catch-all file.

Rules:

1. New daemon SDK modules land under `src/daemon/`.
2. `src/daemon/mod.rs` may re-export SDK types but must not become daemon
   service implementation.
3. Direct construction of `DaemonInvocation` remains restricted to
   `src/daemon/invocation/request.rs`; callers use
   `DaemonInvocation::builder(...)`.
4. Active code and current documentation must not refer to `src/daemon.rs`.

## Ability Name Constants

`daemon/ability/names/` is the canonical home for stable public Ability wire
names used by more than one module.

Allowed content:

- public Ability wire names, for example `terminal.attach`, `fs.transfer`, and
  `meta.list_abilities`;
- public descriptor names;
- stream/bidi sentinel ability names;
- stable public metadata keys;
- stable resource kind names when shared across modules.

Disallowed content:

- handler-local error strings and reason codes;
- environment variable names;
- profile-version strings unless they are a public cross-module contract;
- private temporary file or directory names;
- test fixture names;
- JSON field names unless the field is a stable wire contract;
- plugin-private ability names that are not referenced by core modules.

Example:

```rust
pub mod device_control {
    pub const TERMINAL_ATTACH: &str = "terminal.attach";
    pub const FILE_TRANSFER: &str = "fs.transfer";
    pub const PROCESS_EXEC: &str = "process.exec";
}

pub mod resources {
    pub const SKILL_TREE: &str = "skill.tree";
}
```

Compatibility alias example:

```rust
pub const ABILITY_TREE: &str = crate::daemon::ability::names::resources::SKILL_TREE;
```

Rules:

1. New cross-module code imports from `crate::daemon::ability::names`.
2. Handler modules may re-export old names only as phase-scoped compatibility.
3. A moved name must have an inventory of all importers before the move.
4. A compatibility alias must point to the new constant. Duplicating the string
   literal in both places is forbidden.

## System Ability Handler Boundaries

`daemon/ability/builtins/` owns daemon-owned/system ability handlers.

Target grouping:

```text
src/daemon/ability/builtins/
├─ agents/
│  ├─ chat.rs
│  ├─ chat_history.rs
│  ├─ discover.rs
│  ├─ invoke.rs
│  ├─ lifecycle.rs
│  └─ list.rs
├─ device_control/
│  ├─ files.rs
│  ├─ file_edit.rs
│  ├─ file_transfer.rs
│  ├─ process.rs
│  ├─ shell.rs
│  ├─ http.rs
│  ├─ browser.rs
│  └─ terminal/
├─ resources/
│  ├─ skills/
│  ├─ pages/
│  ├─ context/
│  ├─ files_store/
│  ├─ media/
│  └─ remote_desktop/
├─ automation/
│  ├─ mission.rs
│  ├─ think.rs
│  ├─ schedule.rs
│  ├─ loop.rs
│  ├─ discuss.rs
│  └─ orchestration.rs
├─ integrations/
│  ├─ mcp/
│  ├─ a2a/
│  ├─ federation_probe.rs
│  ├─ openai_compat.rs
│  └─ plugins.rs
└─ governance/
   ├─ consent.rs
   ├─ keyring.rs
   ├─ invocation_history.rs
   ├─ health.rs
   ├─ network_health.rs
   └─ admin_status.rs
```

Allowed content:

- input/output DTOs owned by the handler contract;
- thin validation and handler entry functions;
- registration functions that mount the handler into the catalog;
- handler-local tests.

Disallowed content:

- long-lived service state;
- transport session loops;
- Axon canonicalization;
- receipt/admission policy;
- plugin load planning;
- descriptor root discovery;
- cross-device fan-out loops hidden behind list handlers.

OOP/encapsulation rules:

1. Stateful managers live under `daemon/execution/`, explicit `daemon/`
   semantic directories, or `persistence/`, not inside handler modules.
2. Stateful structs must have private fields and constructor-based dependency
   injection. No public fields on state-holding structs.
3. Handler modules receive dependencies through registry/catalog build services
   or explicit handles; they must not reach through process-global state unless
   the existing subsystem has no injectable handle and the migration phase names
   that as debt.
4. Public fallible boundaries use typed errors. `anyhow` is allowed at process
   or CLI boundaries, not as the semantic error type of a reusable module.
5. Three repeated handler patterns justify a shared helper; two do not.

## System Ability Catalog

`daemon/ability/catalog/` owns the built-in catalog projection.

Allowed content:

- catalog build functions;
- catalog metadata;
- profile descriptors;
- descriptor TOML renderer;
- system manifest helpers;
- descriptor drift tests.

Disallowed content:

- handler implementation bodies;
- transport/session dispatch;
- Axon Invocation tuple construction;
- plugin sidecar process ownership.

The catalog may depend on `daemon/ability/builtins/*` registration functions.
Handlers may not depend back on `daemon/ability/catalog/` except through narrow
types needed for registration.

## Executors And Stateful Execution

`runtime/executors/` owns reusable execution engines used by handlers or
manifest-bound AbilityImpl bindings:

- shell executor;
- EAL executor;
- HTTP executor;
- MCP executor;
- host-stream executor;
- template renderer.

Executors are implementation machinery. They must not register public abilities
directly.

`daemon/execution/` owns long-lived runtime services and stateful managers:

- PTY service;
- schedule service;
- session service;
- loop instances;
- permission service;
- discuss service;
- MCP client execution state.

Handlers may call these services through explicit handles. Services must not
import handler modules to discover names; they use `daemon/ability/names/` or
typed service contracts.

## Product Module Classification

### Agents

User-visible agent operations:

- chat with agent;
- discover agent abilities;
- invoke ability;
- manage agents;
- list agents.

This group must not mean "agent owns Ability" in the formal ontology.
`DeviceAgent advertises AbilityDescriptor`; accountability belongs to
`device_ura` or an explicit AuthorityBinding.

### Device Control

Host-control operations:

- filesystem access;
- file editing and transfer;
- terminal/PTY;
- process execution;
- shell execution;
- browser session control;
- HTTP request.

Device-control handlers must not make locality decisions such as
`target_node == self.node_id`. Locality belongs in resolver/dispatch layers.

### Resources

User/device resources managed or exposed by EasyNet:

- skills;
- pages;
- context captures, clipboard, folders, and favorites;
- files store;
- media devices;
- remote desktop resource integration.

Skill belongs here because skill is an implementation/resource package, not a
protocol-level Ability object.

### Automation

Composition and scheduled work:

- mission;
- think;
- schedule;
- loop;
- discuss;
- orchestration.

Composite AbilityImpl behavior that calls other governed abilities must create
child Invocations. Mission/EAL may orchestrate abilities; it does not redefine
Invocation construction or receipt binding.

### Integrations

External protocol adapters and plugin integration:

- MCP;
- A2A;
- OpenAI compatibility;
- plugin lifecycle ability wrappers.

Integration modules adapt external protocols. They must not duplicate Axon
protocol semantics or become alternate product dispatch paths.

### Governance

Safety, audit, and operational governance:

- consent;
- keyring ability wrapper;
- invocation history;
- health;
- admin status.

The keyring implementation remains under `runtime/keyring/`. The governance
handler is a wrapper over that implementation.

## Descriptor Contracts

Descriptor TOML files are governed product contracts, not Rust source and not
generic static assets.

Final location:

```text
ability-descriptors/system/
├─ agents/
├─ device_control/
├─ resources/
├─ automation/
├─ integrations/
└─ governance/
```

Current governed system descriptors live under `ability-descriptors/system/`.
The retired `abilities/system/` path must not be hard-coded by production code,
tests, scripts, build logic, or packaging checks.

Required abstraction:

```rust
system_ability_descriptor_root() -> PathBuf
system_ability_descriptor_path(ability_name: &str) -> PathBuf
iter_system_ability_descriptor_paths() -> impl Iterator<Item = PathBuf>
```

Rules:

1. Callers must not concatenate `"abilities/system"` directly.
2. Callers must not assume descriptors are flat files under the root. The helper
   owns the map from Ability name to grouped asset path.
3. The helper owns dev, test, packaging, and installed-layout resolution.
4. Descriptor-generation tests must prove rendered descriptor output is
   byte-identical before and after an asset move.
5. Packaging checks must prove the final contract path is included in release
   artifacts before `abilities/system/` is retired.

## Complexity Contract

Project structure must protect algorithmic behavior. Ordinary list/discovery
methods are read-model queries, not distributed live fan-out.

| Surface | Required data source | Complexity target | Forbidden default implementation |
| --- | --- | --- | --- |
| `agent.list` | local hosted-agent registry or hub directory projection | `O(page_size + filter_cost)` | invoking each agent |
| `node.list` | daemon/hub directory read model indexed by realm/user/device | `O(page_size + filter_cost)` | dialing every device |
| `meta.list_abilities` | ability catalog projection keyed by owner/descriptor/filter | `O(page_size + filter_cost)` | calling `meta.list_abilities` on every agent |
| `meta.list_resources` | local resource projection or indexed resource snapshot | `O(page_size + filter_cost)` | probing every resource backend live |
| `skill.list` | local managed skill index or bounded local scan | `O(page_size + filter_cost)` | network-wide skill discovery |
| `skill.tree` | one selected skill root | `O(nodes_returned)` with max depth/page limits | scanning all skills by default |
| `plugin.status` | plugin runtime/load-plan snapshot | `O(plugin_count)` local only | starting plugin sidecars for status |
| `mcp.client.list` | configured MCP client snapshot or explicit refresh | `O(page_size + filter_cost)` for snapshot reads | unbounded live tool listing across servers |
| `namespace.resolve` / `federation.resolve` | indexed directory/namespace lookup | `O(log n + result_count)` or documented equivalent | scanning all devices for exact URA lookup |
| directory `Subscribe` | snapshot cursor plus delta log | `O(snapshot_page + delta_count)` per page | replaying full catalog for each delta |

`filter_cost` must be bounded by indexed predicates or by a documented local
in-memory scan. It must not hide network I/O.

Every public list surface must define:

1. cursor pagination;
2. `DefaultPageSize`;
3. `MaxPageSize`;
4. deterministic sort order;
5. behavior when the read model is stale or unavailable;
6. typed error for invalid cursor and oversized page;
7. no implicit network fan-out.

## Fan-out And Facade Rules

Ordinary facade methods do not fan out governed ability calls across devices,
agents, or abilities.

Rules:

1. `cli`, language SDK facades, FFI convenience layers, and backend
   adapters must not implement per-target governed fan-out loops.
2. Fleet-wide aggregation belongs in a named daemon/hub aggregate ability, for
   example `aggregate.list_abilities_catalog`, not in a facade method named
   like an ordinary list call.
3. Aggregate abilities create one parent Invocation and child Invocations for
   governed child calls.
4. Aggregate abilities expose max concurrency, deadline, page size,
   partial-result semantics, and per-target typed errors.
5. Aggregate results include completed child receipt refs when child calls
   reach terminal states.
6. A facade helper may call one aggregate ability. From the facade perspective
   that is still one parent Invocation.
7. Facade helper names must reveal aggregation, for example
   `AggregateAbilities`, not `ListAbilities`.
8. A default `ListAbilities` call returns the catalog projection already known
   to daemon/hub. It does not trigger live remote discovery.

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

State rules:

| From | Event | To | Rule |
| --- | --- | --- | --- |
| `Planned` | `invoke_aggregate_ability` | `Dispatching` | One parent Invocation enters daemon/hub aggregate ability. |
| `Dispatching` | `child_invocations_started` | `Collecting` | Child Invocations carry parent receipt/causal context. |
| `Collecting` | `all_children_terminal` | `Completed` | Result includes child receipt refs. |
| `Collecting` | `some_children_terminal_some_failed` | `Partial` | Partial result includes per-target typed errors. |
| `Collecting` | `deadline_elapsed` | `TimedOut` | Timeout result includes completed child receipt refs. |
| non-terminal | `aggregate_failed` | `Failed` | Parent receipt records aggregate failure. |

Invariants:

1. Fan-out concurrency is bounded by an explicit daemon-side limit.
2. Fan-out deadline is bounded by the parent Invocation timeout.
3. Partial results are explicit and typed.
4. Every governed child call is an Invocation with its own receipt.
5. SDK language facades do not run governed fan-out loops.

## Migration Risk Classification

### Pre-move Inventory Gate

Before any move, produce an inventory for the affected symbol/path:

```text
rg -n "runtime::agents::<module>|<ABILITY_CONST>|<descriptor path>|<handler type>" src tests docs
```

The inventory must classify each caller as one of:

- handler registration;
- catalog/descriptor generation;
- transport/session/wire profile;
- CLI/MCP/FFI;
- plugin package;
- service/trust/admission;
- test/conformance;
- docs/scripts;
- legacy compatibility only.

No move is allowed until caller classes are known.

Historical coverage rule: before `src/runtime/agents/` was retired, every file
or directory under it had to belong to one risk bucket below before structural
migration started. The current checkout must not contain `src/runtime/agents/`;
the inventory below remains as the audit record for where those files moved.
When a directory path is listed, every descendant file was covered by that
bucket unless a descendant file was explicitly called out in another bucket.

### Low-Risk Moves

These can move with compatibility re-exports and local tests:

```text
src/runtime/agents/ping.rs
  -> src/daemon/ability/builtins/governance/health.rs

src/runtime/agents/admin_status_ability.rs
  -> src/daemon/ability/builtins/governance/admin_status.rs

src/runtime/agents/network_health_ability.rs
  -> src/daemon/ability/builtins/governance/network_health.rs

src/runtime/agents/agent_list_ability.rs
  -> src/daemon/ability/builtins/agents/list.rs

src/runtime/agents/a2a_bridge_ability.rs
  -> src/daemon/ability/builtins/integrations/a2a/bridge.rs

src/runtime/agents/a2a_client_ability.rs
  -> src/daemon/ability/builtins/integrations/a2a/client.rs

src/runtime/agents/mcp_client_ability.rs
  -> src/daemon/ability/builtins/integrations/mcp/client.rs

src/runtime/agents/federation_probe.rs
  -> src/daemon/ability/builtins/integrations/federation_probe.rs
```

Catalog-only moves:

```text
src/runtime/agents/ability_toml.rs
  -> src/daemon/ability/catalog/ability_toml.rs

src/runtime/agents/system_ability_manifest.rs
  -> src/daemon/ability/catalog/system_manifest.rs
```

Low-risk repository moves:

```text
docker/   -> engineering/docker/
macos/    -> platforms/macos/
windows/  -> platforms/windows/
```

`schemas/` stays at the repository root because Clean Final defines it as the
machine-readable contract root used by proto generation, receipt validation,
FFI headers, packaging, and conformance checks. It is not an engineering-tool
asset.

### Medium-Risk Moves

These require ability-name extraction first and domain-specific tests after each
move.

Device control:

```text
src/runtime/agents/fs_ability.rs
src/runtime/agents/fs_edit_ability.rs
src/runtime/agents/file_transfer_ability.rs
src/runtime/agents/pty_lifecycle_ability.rs
src/runtime/agents/pty_io_ability.rs
src/runtime/agents/pty_attach_ability.rs
src/runtime/agents/process_exec_ability.rs
src/runtime/agents/shell_run_ability.rs
src/runtime/agents/http_request_ability.rs
src/runtime/agents/browser_session_ability.rs
src/runtime/agents/session_ability.rs
```

Reason: transport, wire conversion, facade code, tests, and plugin code import
public constants or specific types.

Executors and templates:

```text
src/runtime/agents/shell_executor.rs
src/runtime/agents/eal_executor.rs
src/runtime/agents/http_executor.rs
src/runtime/agents/host_stream_executor.rs
src/runtime/agents/template.rs
```

Target:

```text
src/runtime/executors/
```

Reason: these modules are reusable implementation machinery shared by
manifest-bound abilities and handlers. They must move behind compatibility
re-exports before handler modules stop importing through `runtime::agents`.

Resources:

```text
src/runtime/agents/skill_ability.rs
src/runtime/agents/skill_install_ability.rs
src/runtime/agents/skill_publish_ability.rs
src/runtime/skill_store.rs
src/runtime/agents/context_ability.rs
src/runtime/agents/context_loaders/
src/runtime/agents/list_resources_ability.rs
src/runtime/agents/files/
src/runtime/agents/pages/
src/runtime/agents/pages_identity.rs
src/runtime/agents/media_abilities.rs
src/runtime/agents/media/
src/runtime/agents/voice_call_ability.rs
```

Reason: skill listing/tree/read/write, workspace paths, pages listener, media
resource projection, context stores, voice call state, plugin reuse, and global
pools are coupled.

Automation/orchestration:

```text
src/runtime/agents/discuss_ability.rs
src/runtime/agents/loop_ability.rs
src/runtime/agents/mission_ability.rs
src/runtime/agents/orchestration_ability.rs
src/runtime/agents/schedule_ability.rs
src/runtime/agents/think_ability.rs
```

Reason: these handlers are coupled to Mission/EAL, long-lived execution
services, child Invocation semantics, subscriptions, and persisted run state.

Integrations/governance:

```text
src/runtime/agents/api_key_ability.rs
src/runtime/agents/openai_compat_ability.rs
src/runtime/agents/permission_ability.rs
src/runtime/agents/invocation_history_ability.rs
src/runtime/keyring/abilities.rs
src/runtime/agents/plugin_lifecycle_ability.rs
src/runtime/agents/mcp_bridge_ability.rs
src/runtime/agents/teach_ability.rs
```

Reason: wrappers are correctly grouped, but implementation ownership and
handler ownership must remain separated. `teach_ability` also touches
ability/skill curation semantics and must move only after catalog and resource
boundaries are pinned.

### High-Risk Moves

Move last, or initially keep as re-export-only wrappers.

Catalog center:

```text
src/runtime/agents/registry_builder.rs
src/runtime/agents/catalog_metadata.rs
src/runtime/agents/profiles/
src/runtime/agents/meta_ability.rs
```

Reason: boot, publish, MCP profile projection, session prelude, descriptor
generation, and conformance baselines depend on these.

Agent interaction:

```text
src/runtime/agents/chat_ability.rs
src/runtime/agents/chat_history_ability.rs
src/runtime/agents/discover_ability.rs
src/runtime/agents/invoke_ability.rs
src/runtime/agents/agent_lifecycle_ability.rs
src/runtime/agent_ability_specs.rs
```

Reason: dynamic hosted-agent lifecycle, hot registration, discovery, default
chat ability specs, and invocation are tightly coupled.

Device ability management:

```text
src/runtime/agents/device_ability_registrar.rs
src/runtime/agents/device_ability_store.rs
src/runtime/agents/device_ops_ability.rs
src/runtime/agents/ability_publish_ability.rs
```

Reason: install/uninstall, manifest storage, control-plane registration, and
descriptor transactions are coupled. `ability_publish_ability` also crosses the
AbilityDescriptor/AbilityImpl/resource-curation boundary.

MCP reflection:

```text
src/runtime/agents/mcp_reflective_registry.rs
src/runtime/agents/mcp_executor.rs
```

Reason: reflection, EAL dispatch, plugin host, concurrency limits, environment
configuration, and tests depend on these paths and types.

Test-only modules:

```text
src/runtime/agents/assembly_tests.rs
src/runtime/agents/real_invoke_tests.rs
```

Reason: these move only after the production modules they exercise have moved.
Tests may be colocated under the new module or promoted to integration tests,
but they must continue to exercise registry lookup, handler invocation, service
interaction, and response shape.

### Tool Compatibility Moves

Do not move without compatibility wrappers:

```text
tests/    -> engineering/tests/
benches/  -> engineering/benches/
scripts/  -> engineering/scripts/
.github/  -> engineering/ci/
```

`tests/`, `benches/`, and `.github/workflows/` are tool-recognized root paths.

### Descriptor Contract Moves

Move only after descriptor-root abstraction lands:

```text
abilities/system/
  -> ability-descriptors/system/
```

Do not move descriptor contract files while code still hard-codes
`abilities/system`.

## Required Migration Order

0. Capture current baselines:
   - current `meta.list_abilities` names and call modes;
   - current descriptor generated output;
   - `rg` inventory for handler constants imported outside their modules;
   - `rg` inventory for hard-coded `abilities/system`.
1. Add `daemon/ability/names/`.
2. Move stable public wire constants into `daemon/ability/names/`.
3. Keep old handler constants as aliases to `daemon/ability/names/`.
4. Update transport, plugin, facade, and tests to import public names from
   `daemon::ability::names`.
5. Rename `runtime/abilities.rs` to `runtime/agent_ability_specs.rs`, then
   retire the `runtime::abilities` compatibility re-export after callers move.
6. Add `daemon/ability/builtins/` and `daemon/ability/catalog/`
   skeletons.
7. Freeze `runtime/agents/` as compatibility facade, move callers, then retire
   the compatibility module once production imports stop using it.
8. Move low-risk leaf handlers.
9. Move reusable executors/templates behind compatibility re-exports.
10. Move device-control handlers after transport imports use `ability_names`.
11. Move resources/skills/pages/context/media/voice after hub/plugin/test
    imports use stable paths.
12. Move automation, integrations, and governance handlers.
13. Move catalog center files into `daemon/ability/catalog/` after
    descriptor/catalog baselines are pinned.
14. Add descriptor-root and descriptor-path abstractions.
15. Move descriptor contract files.
16. Move command-line facade source from `src/facade/cli/` to `src/cli/`, then
    retire `src/facade/` and `crate::facade::cli` after callers move.
17. Triage root tool directories into `engineering/` with wrappers.
18. Remove deprecated compatibility imports in a dedicated cleanup phase.

Each phase must compile independently. A phase that moves files and changes
semantics at the same time is invalid.

## Verification Matrix

No phase passes by prose. Each migration PR must attach the evidence below in
its PR note or review artifact. If a listed command is not yet supported by the
repository, that phase must first add the missing deterministic check or mark
the phase blocked.

| Gate | Applies to | Required evidence | Fails when |
| --- | --- | --- | --- |
| Clean-final clarity | Spec edits | Line references showing `Clean Final` and `Migration-Compatible` sections remain separate | A migration-only name is presented as permanent architecture |
| Forbidden final names | Spec edits and source moves | Review of the Clean Final blocks proving no final `runtime/`, `services/`, `system_abilities/`, `ability_runtime/`, or `crates/` target | A forbidden name appears in Clean Final instead of review findings or migration sections |
| SDK boundary | Spec edits and SDK work | `sdk/rust/` exists before other SDKs in the target tree, plus review text stating SDKs are facades over daemon/libeasynet-cli semantics | SDK code owns daemon policy, Axon canonical algorithms, or hidden governed fan-out |
| Caller inventory | Every symbol/path move | `rg -n "runtime::agents::<module>|<ABILITY_CONST>|<descriptor path>|<handler type>" src tests docs` output classified by caller type | Any caller is unclassified, or transport/facade/plugin callers are moved without a replacement path |
| `runtime/agents` retirement | Runtime handler migration | `engineering/scripts/check-project-structure-v1.sh` plus `tests/scripts/test_check_project_structure_v1.sh` prove `src/runtime/agents` is absent and active code does not import `runtime::agents` | `runtime/agents` returns, or production code imports through it |
| Ability-name stability | Ability-name extraction and handler moves | Before/after snapshot of public ability names and call modes | Any Ability wire name or call mode changes outside a dedicated semantic-change spec |
| Catalog stability | Handler/catalog moves | Before/after `meta.list_abilities` snapshot sorted by ability name | Names disappear, duplicate, reorder nondeterministically, or change owner/call-mode metadata unintentionally |
| Descriptor drift | Descriptor generation and descriptor moves | `cargo run --bin gen-ability-tomls` followed by a clean descriptor diff, or an explicit intentional descriptor diff | Rendered descriptor bytes drift without a descriptor-format change |
| Descriptor path abstraction | Descriptor moves | `rg -n "abilities/system|ability-descriptors/system" src tests build.rs scripts` showing production code uses root/path/iterator helpers | Production code concatenates descriptor paths or assumes a flat directory |
| Migration-compatible root layout | Repository directory moves | `engineering/scripts/check-project-structure-v1.sh` proving root `schemas/`, `engineering/docker/`, `engineering/scripts/`, `engineering/tests/scripts/`, `engineering/benches/`, `platforms/macos/`, and `platforms/windows/` exist; retired root directories do not; root `scripts/`, `tests/scripts/`, and `benches/` are thin compatibility wrappers | Retired root `docker/`, `macos/`, `windows/`, or `engineering/schemas/` directories return; platform/tooling directories are missing; or root wrappers contain real logic |
| CLI/MCP facade retirement | CLI source moves and facade namespace retirement | `engineering/scripts/check-project-structure-v1.sh` proving `src/cli/mod.rs` exists, `src/facade` is absent, active code does not import through `facade::cli` or `facade::mcp`, and MCP front-door code stays in `src/cli` plus the runtime MCP profile | New CLI/MCP code lands under `src/facade/*`, or active code imports retired facade paths |
| Daemon SDK root ownership | Daemon SDK source moves | `engineering/scripts/check-project-structure-v1.sh` proving `src/daemon/mod.rs` exists, `src/daemon.rs` is retired, and active code does not reference the retired physical path | New daemon SDK root logic lands in `src/daemon.rs`, or docs/scripts keep treating it as the active SDK root |
| Daemon control ownership | Control-plane source moves | `engineering/scripts/check-project-structure-v1.sh` proving `src/daemon/control/` exists, `src/services/control/` is retired, and active code does not import through `services::control` | New local control-plane code lands under `src/services/control`, or active code imports the retired services control path |
| Daemon Invocation ownership | Invocation transport source moves | `engineering/scripts/check-project-structure-v1.sh` proving `src/daemon/invocation/`, `src/daemon/invocation/target.rs`, `src/daemon/invocation/runtime_record.rs`, `src/daemon/invocation/receipt_subscriber.rs`, and `src/daemon/invocation/local_runtime_invoker.rs` exist, `src/services/invocation_transport/`, `src/runtime/invocation_target.rs`, `src/runtime/invocation.rs`, `src/runtime/receipt_subscriber.rs`, and `src/runtime/local_runtime_invoker.rs` are retired, and active code does not import through `services::invocation_transport`, `runtime::invocation_target`, `runtime::invocation`, `runtime::receipt_subscriber`, or `runtime::local_runtime_invoker` | New Invocation transport, target-resolution, runtime-record, or receipt-observer logic lands under `src/services/invocation_transport`, `src/runtime/invocation_target.rs`, `src/runtime/invocation.rs`, `src/runtime/receipt_subscriber.rs`, or `src/runtime/local_runtime_invoker.rs`, or active code imports the retired services/runtime Invocation paths |
| Daemon kernel ownership | Kernel and KernelApi source moves | `engineering/scripts/check-project-structure-v1.sh` proving `src/daemon/kernel/mod.rs` and `src/daemon/kernel/api.rs` exist, `src/runtime/kernel.rs` and `src/runtime/kernel_api.rs` are retired, and active code does not import through `runtime::kernel` or `runtime::kernel_api` | New daemon kernel execution or syscall-boundary logic lands under `src/runtime/kernel*.rs`, or active code imports retired runtime kernel paths |
| Daemon federation gateway ownership | Gateway and GatewayApi source moves | `engineering/scripts/check-project-structure-v1.sh` proving `src/daemon/federation/gateway.rs` and `src/daemon/federation/gateway_api.rs` exist, `src/runtime/gateway.rs` and `src/runtime/gateway_api.rs` are retired, and active code does not import through `runtime::gateway` or `runtime::gateway_api` | New daemon federation lifecycle/discovery gateway logic lands under `src/runtime/gateway*.rs`, or active code imports retired runtime gateway paths |
| Daemon federation owner projection ownership | Owner projection read-model move | `engineering/scripts/check-project-structure-v1.sh` proving `src/daemon/federation/read_model/owner_projection.rs` exists, `src/runtime/owner_projection.rs` is retired, and active code does not import through `runtime::owner_projection` | New owner ability projection, lease refresh, or advertised callable summary logic lands under `src/runtime/owner_projection.rs`, or active code imports the retired runtime owner projection path |
| Daemon federation ability contract ownership | federation.* DTO source move | `engineering/scripts/check-project-structure-v1.sh` proving `src/daemon/federation/client/ability_contract.rs` exists, `src/runtime/federation_client.rs` is retired, and active code does not import through `runtime::federation_client` | New typed argument/response helpers for hub-profile `federation.*` abilities land under `src/runtime/federation_client.rs`, or active code imports the retired runtime federation client path |
| Daemon federation advertise ownership | federation.* advertise/heartbeat client move | `engineering/scripts/check-project-structure-v1.sh` proving `src/daemon/federation/advertise.rs` exists, `src/runtime/advertise.rs` is retired, and active code does not import through `runtime::advertise` | New federation advertise, heartbeat, resolve, revoke, resolve_key, or forward_invoke wrapper logic lands under `src/runtime/advertise.rs`, or active code imports the retired runtime advertise path |
| Daemon federation publish ownership | federation publish/registration orchestration move | `engineering/scripts/check-project-structure-v1.sh` proving `src/daemon/federation/publish.rs` exists, `src/runtime/publish.rs` is retired, and active code does not import through `runtime::publish` | New local-agent bootstrap, self-identity bootstrap caller, runtime-local registration, advertise batching, descriptor publication, or revoke orchestration lands under `src/runtime/publish.rs`, or active code imports the retired runtime publish path |
| Daemon federation init ownership | federation boot state machine move | `engineering/scripts/check-project-structure-v1.sh` proving `src/daemon/federation/init/{mod,outcome,probe,resolver_seed}.rs` exist, `src/runtime/federation_init/` is retired, and active code does not import through `runtime::federation_init` | New federation boot decision, operator-facing status probe, typed init outcome, or shard resolver seed loader lands under `src/runtime/federation_init/`, or active code imports the retired runtime federation-init path |
| Daemon federation resolver ownership | realm federation posture resolver move | `engineering/scripts/check-project-structure-v1.sh` proving `src/daemon/federation/resolver.rs` exists, `src/runtime/resolver/` is retired, and active code does not import through `runtime::resolver` | New realm-suffix admission-mode, hub endpoint, or canonical device-URA resolver logic lands under `src/runtime/resolver/`, or active code imports the retired runtime resolver path |
| Daemon hub ownership | Hub Pages listener and serve adapter move | `engineering/scripts/check-project-structure-v1.sh` proving `src/daemon/hub/{mod,pages_listener,pages_serve_ability}.rs` exist, `src/runtime/hub/` is retired, and active code does not import through `runtime::hub` | New in-daemon Hub listener, Pages HTTP adapter, or Hub-side transport adapter logic lands under `src/runtime/hub/`, or active code imports the retired runtime hub path |
| Daemon Invocation state ownership | Invocation presence, pending dispatch, replay, quota, and failure-state source moves | `engineering/scripts/check-project-structure-v1.sh` proving `src/daemon/invocation/state/{presence,pending_dispatch,nonce_replay,usage_quota,session_failure}.rs` exist; retired Invocation state files under `src/services/` do not; active code does not import through retired services Invocation-state paths | New daemon Invocation liveness, pending-dispatch, replay, quota, or failure-state code lands under `src/services`, or active code imports retired services state paths |
| Daemon federation ownership | Federation transport, directory, peer-map, discovery read-boundary, and read-model source moves | `engineering/scripts/check-project-structure-v1.sh` proving `src/daemon/federation/client/`, `src/daemon/federation/directory.rs`, `src/daemon/federation/directory_reader.rs`, `src/daemon/federation/peers.rs`, and `src/daemon/federation/read_model/{ability_catalog,advertised_agents,hub_published_abilities}.rs` exist; retired federation files under `src/services/` do not; active code does not import through retired `services::*` paths | New daemon federation transport, directory, peer-map, discovery-reader, or read-model code lands under `src/services`, or active code imports retired services paths |
| Daemon trust ownership | Trust-anchor state, hot-reload cell, and Axon key-resolver source moves | `engineering/scripts/check-project-structure-v1.sh` proving `src/daemon/trust/anchor.rs`, `src/daemon/trust/cell.rs`, and `src/daemon/trust/key_resolver.rs` exist; retired trust files under `src/services/` do not; active code does not import through retired `services::realm_trust_anchor`, `services::trust_anchor_cell`, or `services::trust_anchor_key_resolver` paths | New daemon trust state or key-resolution adapters land under `src/services`, or active code imports retired services trust paths |
| Daemon identity/keyring ownership | Host signing handle and keyring vault source moves | `engineering/scripts/check-project-structure-v1.sh` proving `src/daemon/identity/self_identity.rs` and `src/daemon/keyring/mod.rs` exist; retired identity/keyring files under `src/services/` do not; active code does not import through retired `services::self_identity` or `services::keyring` paths | New host identity signing or keyring vault code lands under `src/services`, or active code imports retired services identity/keyring paths |
| Daemon ability/context ownership | Ability-health monitor and context-capture background services | `engineering/scripts/check-project-structure-v1.sh` proving `src/daemon/ability/health.rs` and `src/daemon/context/clipboard_tracker.rs` exist; `src/services/` is absent; active code does not import through `services::*` | New ability support services or context capture loops land under `src/services`, or active code imports retired services paths |
| Facade fan-out ban | CLI, FFI, SDK, backend adapter work | Audit of facade code paths plus search for loops/concurrency over devices, agents, or abilities in facade layers | A default list/helper performs governed per-target fan-out |
| Aggregate fan-out contract | Aggregate ability work | State machine, max concurrency, deadline, page size, partial-result type, child receipt refs, and per-target typed errors | Aggregation is hidden behind ordinary list naming or lacks bounded/typed partial semantics |
| Compile gate | Every code phase | `cargo fmt --check` and a narrow compile command such as `cargo check --lib --features axon-pb` or a phase-specific stricter gate | Formatting fails, compilation fails, or the chosen compile gate does not cover touched modules |
| Compatibility retirement | Every phase that adds re-exports | List of compatibility exports with owner, reason, and cleanup phase | A compatibility path has no retirement plan or new production code imports it |

Minimum guard command for the Project Structure v1 migration boundary:

```sh
engineering/scripts/check-project-structure-v1.sh
tests/scripts/test_check_project_structure_v1.sh
```

Baseline artifacts must be deterministic:

1. sorted by stable key;
2. free of wall-clock timestamps, random ids, and host-specific absolute paths;
3. checked into the PR note or stored under ignored `pr/` review artifacts;
4. comparable by plain `diff`.

## Acceptance Criteria

Code and structure:

1. `cargo fmt --check` passes.
2. A narrow compile gate appropriate to the touched phase passes, for example
   `cargo check --lib` or the existing project feature gate used by the phase.
3. Old public module paths compile through re-exports until their named cleanup
   phase.
4. New production imports do not use compatibility paths.
5. `runtime/ability_runtime/` is not introduced.
6. `daemon/ability/builtins/` does not collide with or reintroduce the retired
   `runtime/abilities.rs` path.
7. `runtime/agents/` is absent after the retirement phase.
8. `runtime/local_runtime_invoker.rs` is absent; LocalRuntime invocation
   adapter code lives under `daemon/invocation/`.
9. `runtime/invocation_target.rs` is absent; Invocation target resolution
   code lives under `daemon/invocation/`.
10. `runtime/invocation.rs` and `runtime/receipt_subscriber.rs` are absent;
    daemon-local invocation records and receipt observer surfaces live under
    `daemon/invocation/`.
11. `runtime/kernel.rs` and `runtime/kernel_api.rs` are absent; daemon kernel
    code lives under `daemon/kernel/`.
12. `runtime/gateway.rs` and `runtime/gateway_api.rs` are absent; federation
    gateway code lives under `daemon/federation/`.
13. `runtime/owner_projection.rs` is absent; owner ability projection read
    model code lives under `daemon/federation/read_model/`.
14. `runtime/federation_client.rs` is absent; typed federation ability DTOs
    live under `daemon/federation/client/`.
15. `runtime/advertise.rs` is absent; typed federation advertise/heartbeat
    client code lives under `daemon/federation/`.
16. `runtime/publish.rs` is absent; federation publish/registration
    orchestration lives under `daemon/federation/`.
17. `runtime/federation_init/` is absent; federation boot state machine and
    status probe live under `daemon/federation/init/`.
18. `runtime/resolver/` is absent; realm federation posture resolution lives
    under `daemon/federation/resolver.rs`.
19. `runtime/hub/` is absent; in-daemon Hub Pages listener and serve adapter
    live under `daemon/hub/`.

Behavior:

20. Existing public Ability names remain byte-identical.
21. `meta.list_abilities` returns the same ability names before and after a
   structural move.
22. Ability call modes remain unchanged.
23. Descriptor generation output remains byte-identical unless the phase is
    explicitly a descriptor-format change.
24. No product-module source move changes Invocation or Receipt semantics.
25. No runtime registry tree is introduced.

Complexity/fan-out:

19. Ordinary list methods satisfy their documented complexity contracts.
20. Ordinary list/facade methods do not run per-agent/per-device governed
    fan-out loops.
21. Any aggregate fan-out is a named daemon/hub aggregate ability with bounded
    concurrency, deadline, partial result semantics, child receipt refs, and
    typed per-target errors.
22. Facade methods that invoke aggregate abilities are named as aggregate
    helpers.

Boundary:

23. Transport and daemon surfaces import ability names from
    `daemon::ability::names` or typed service contracts, not from handler
    modules.
24. Plugin code does not depend on core handler-module paths for public wire
    constants.
25. Descriptor generation does not hard-code `abilities/system`.
26. Descriptor lookup uses root, per-ability path, or iterator helpers and does
    not assume a flat descriptor directory.
27. Skills are not treated as protocol-callable objects unless wrapped by an
    explicit AbilityDescriptor.
28. Axon protocol semantics are not duplicated in CLI/backend/facade code.

## Review Checklist For The Next Audit

Use this list to decide whether the next review has no remaining issues.

1. Does each module name describe the semantic owner instead of a convenient
   file bucket?
2. Is there any new directory that duplicates an existing semantic module?
3. Did any move collapse AbilityDescriptor, AuthorityBinding, AbilityImpl, and
   handler body ownership?
4. Are public wire constants centralized without moving private implementation
   constants?
5. Are list methods paginated, sorted, bounded, and read-model backed?
6. Is any facade doing hidden per-target governed fan-out?
7. Does every aggregate ability expose concurrency, deadline, partial-result,
   and child-receipt semantics?
8. Are old module paths compatibility-only, with a retirement phase?
9. Can `meta.list_abilities` be compared before/after a move?
10. Are descriptor roots resolved through one helper?
11. Do plugin packages remain plugin packages?
12. Do skills remain implementation/resource packages?
13. Does any handler own long-lived state that belongs in `execution/`,
    explicit `daemon/` semantic directories, or `persistence/`?
14. Do stateful structs keep private fields and constructor-injected
    dependencies?
15. Did any CLI/SDK/backend-facing code import Axon internals or duplicate Axon
    canonical algorithms?
16. Is there a caller inventory for every moved path or symbol?
17. Does the phase compile independently?
18. Is any compatibility wrapper becoming permanent by accident?

If all answers are clean, the structure migration is acceptable for the next
review.
