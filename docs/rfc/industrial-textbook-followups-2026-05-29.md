# Industrial-Textbook Follow-up PRs (2026-05-29)

**Status:** Plan — scope sketches for follow-up PRs. **Author:** Silan Hu. **Trigger:** the 2026-05-29 cross-axis industrial-standard review of the in-progress unstaged diff (129 files, +6.2k/-8.3k).

This document records the structural debts identified in that review whose **execution** is too large to land alongside the current PR without making the diff itself an unreviewable mega-change. Each section is a self-contained follow-up PR scope: trigger, motivation, proposed shape, scope boundary (what's in, what's out), and ship criteria.

The in-PR fixes (kernel-boundary CI, dead-code prune, ProcessSingleton diagnostic mirror, async/sync bridge dedup, typed not-found classification, test-seam isolation, local_invoke collapse, `legacy self alias.*` grep-anchored TODOs, RuntimeHandlerSet drain extraction, async list_abilities propagation, classified try_send helper) are NOT repeated here — those are in the current PR's diff.

---

## PR-A — Split `daemon_invocation_service.rs` (10 208 lines → module dir)

**Motivation.** Single file, single impl block, 17 fields on the struct, 15+ `with_*` builders, three `impl` segments spanning thousands of lines each. Holds five disjoint concerns: federation wrapper dispatch, self-targeted ability arms, pubkey lifecycle, bidi-stream machinery, invoke_remote correlation + ledger record building. A reviewer cannot hold the impl in mind; new arms accrete to whichever block was nearest in `git blame`.

**Proposed shape.** Convert `src/services/invocation_transport/daemon_invocation_service.rs` into a module directory:

```
src/services/invocation_transport/daemon_invocation_service/
├── mod.rs                  # Invocation trait impl (~300 lines: routes by function_name)
├── state.rs                # DaemonInvocationService struct + Builder
├── federation_arms.rs      # dispatch_federation_* helpers
├── pubkey_arms.rs          # register_*/revoke_*/list_* pubkey
├── session_accept.rs       # dispatch_self_session_accept + downstream wiring
├── invoke_remote.rs        # runtime.invoke_remote initiator + correlation
├── bidi_streams.rs         # SessionDownStream / LocalBidiDownStream / frame mappers
├── ledger_record.rs        # InvocationLedger record building (~free fns at the file tail)
└── tests/
    ├── mod.rs              # shared test fixtures (make_service, TEST_DAEMON_URI, etc.)
    ├── federation_arms.rs
    ├── self_session_accept.rs
    └── ...                 # one test file per arm module
```

Each sibling module takes a borrowed slice of `Service` fields rather than the whole struct so the dependency direction is explicit at the type level — e.g. `federation_arms::dispatch_local_presence_forward_invoke(state: &FederationView, ...)`.

**Scope boundary.**
- IN: file move only. Method bodies and signatures stay identical. Tests stay attached to the arm they exercise.
- OUT: any behaviour change. No new methods, no signature refactors, no Builder restructure (that's PR-B). One commit per arm-module move, each ~150–500 line patch.

**Ship criteria.**
1. `git mv` history preserved (sibling files retain blame).
2. `cargo check --features axon-pb --tests` clean before + after each commit.
3. Production module's public surface (`pub struct DaemonInvocationService`, `pub fn new`, all `pub fn with_*`, all `impl Invocation for ...` arms) unchanged.
4. Each test file's `#[test]` count matches the pre-split state.
5. After landing: max single-file LoC under `services/invocation_transport/daemon_invocation_service/` is ≤ 2 000.

**Sequencing risk.** PR-B (Builder::build()) reshapes the `Service` struct; PR-A should land FIRST so the per-arm files exist when the struct contracts. Landing PR-B first creates a wave of merge conflicts when PR-A moves arms whose Builder signatures just changed.

---

## PR-B — `DaemonInvocationService::Builder::build()` with grouped capability structs

**Motivation.** Today `DaemonInvocationService::new(...)` returns a half-configured value that 15 `with_*` setters mutate in any order. The struct holds 17 fields, most `Option<Arc<...>>`. Each dispatch arm runs `Option::ok_or_else(Status::failed_precondition("daemon was constructed without ..."))`. "What is a legal DaemonInvocationService?" is undecidable from the type — every legality check is scattered across 15+ runtime callsites.

**Proposed shape.** Four cohesive grouping structs, one `Builder` whose `build()` enforces invariants once at boot:

```rust
pub struct FederationConfig {
    pub federation_client: Option<Arc<dyn FederationClient>>,
    pub federated_peers: Option<Arc<FederatedPeersCell>>,
    pub federated_directory: Option<Arc<FederatedDirectoryCell>>,
    pub allow_directory_auto_route: bool,
    pub federated_bindings: Option<Arc<FederatedBindingsStore>>,
}

pub struct TrustWriteContext {
    pub register_pubkey: Option<Arc<RegisterPubkeyService>>,
    pub session_realm: Option<String>,
    pub hub_signing_seed: Option<[u8; 32]>,
}

pub struct LedgerWiring {
    pub invocation_ledger: Option<Arc<InvocationLedger>>,
    pub subscribe_v2_heartbeat: NonZeroU64,  // was a raw u64 + runtime assert
}

pub struct RuntimeBindings {
    pub local_runtime: Option<Arc<LocalRuntime>>,
    pub session_escalation: Option<Arc<SessionEscalation>>,
    pub pending: Option<Arc<PendingDispatchMap>>,
    pub pending_stream: Option<Arc<PendingStreamMap>>,
}

pub struct DaemonInvocationServiceBuilder { /* …four optionals + presence + admission… */ }

impl DaemonInvocationServiceBuilder {
    pub fn build(self) -> Result<DaemonInvocationService, ConfigError> {
        // Invariant 1: session_escalation requires local_runtime
        if self.runtime.session_escalation.is_some() && self.runtime.local_runtime.is_none() {
            return Err(ConfigError::SessionEscalationWithoutRuntime);
        }
        // Invariant 2: every federation_* requires federation_client
        // ...
        Ok(DaemonInvocationService { /* … */ })
    }
}
```

Boot callsites read as four lines instead of fifteen:

```rust
let svc = DaemonInvocationServiceBuilder::new(presence, admission)
    .with_federation(federation_cfg)
    .with_trust_writes(trust_ctx)
    .with_ledger(ledger_wiring)
    .with_runtime(runtime_bindings)
    .build()?;
```

**Scope boundary.**
- IN: Builder restructure, invariant validation moves from per-arm `failed_precondition` to one `build()`-time check.
- OUT: any arm-method changes. Arms still read `self.federation.federated_peers.as_ref()` etc — the field paths change shape, not the body.

**Ship criteria.**
1. Every `with_*` call in the daemon's boot path is gone (replaced by group struct construction).
2. No dispatch arm in `services/invocation_transport/daemon_invocation_service/` issues `Status::failed_precondition("daemon was constructed without ...")` — the equivalent rejections live in `Builder::build()`.
3. `ConfigError` enum has one variant per validated invariant; each variant carries a human-actionable message naming the missing capability and the boot step that would supply it.
4. `cargo check --features axon-pb --tests` + boundary script + every boot integration test passes unchanged.

**Sequencing.** Lands AFTER PR-A so the per-arm files already exist with the new field-access shape.

---

## PR-C — Split `agent_lifecycle_ability::start_agent_handler` into two abilities + typed args

**Motivation.** `agent.start` is a 230-line closure that simultaneously: validates 9 stringly-typed args, creates an `agent.toml` on disk, projects a workspace via `workspace::ensure_from_directory`, runs live `LocalRuntime` registration through a `OnceLock`-injected `HotAgentRegistrar`, and persists a registry row. Five concerns, one ability, no isolation. The current PR shrank the CLI side by 1 488 lines but only because the equivalent fat function was reborn in the runtime layer.

**Proposed shape.** Split into two cohesive abilities the CLI orchestrates:

1. **`device.agent.register`** — pure registry write; idempotent; takes a typed `RegisterAgentArgs`; returns `RegisterAgentResponse { agent_ura, replaced_prior }`. Knows nothing about the filesystem or LocalRuntime.

2. **`device.agent.materialize_directory`** — pure fs side; takes `MaterializeArgs { agent_name, parent_dir, agent_type, model, ... }`; returns `MaterializeResponse { root_path, created_directory, updated_spec, workspace_projected }`. Knows nothing about the registry.

CLI's `easynet agent add` becomes two sequential invokes the operator can read in plain terms. Each ability has a single concern, single-purpose unit tests, no tempdir dependency for the registry test, no registry-dep for the fs test.

Typed args throughout:

```rust
#[derive(Deserialize)]
struct RegisterAgentArgs {
    name: String,
    #[serde(default)]
    agent_type: Option<AgentType>,
    #[serde(default)]
    model: Option<String>,
    #[serde(default)]
    entry: Option<AgentEntry>,    // canonical writer path per current docstring
}

#[derive(Serialize)]
struct RegisterAgentResponse {
    agent_ura: String,
    replaced_prior: bool,
}
```

Handler becomes `let args: RegisterAgentArgs = serde_json::from_value(args)?;` — one line replaces the 9 `args.get("…").and_then(Value::as_str).unwrap_or(...)` chains.

**Scope boundary.**
- IN: split `start_agent_handler`; add typed args/response structs; CLI's `easynet agent add` orchestrates two invokes.
- IN: same split for `stop_agent_handler` if its body has the same 5-concern shape (audit confirms first).
- OUT: typed I/O sweep across other agent-lifecycle abilities (that's PR-D, the typed-IO sweep). This PR only does the split + types for the two `device.agent.{start,stop}` paths.

**Ship criteria.**
1. `device.agent.register` exists with `additionalProperties = false` schema; handler is `<= 60 lines`.
2. `device.agent.materialize_directory` exists with the same schema strictness.
3. CLI `easynet agent add` issues two invokes; failure of the second (materialize) does NOT leave a half-registered row — the orchestration uses a saga shape (register → materialize → on materialize-fail, unregister).
4. `agent.start` is removed; CLI surfaces and any third-party that called it use the new name (deprecation aliasing optional during transition; current PR's start ability is internal so direct rename is safer).
5. No `Value::get(...).and_then(...)` chain remains in either handler. All arg parsing through `serde_json::from_value::<TypedArgs>(args)?`.

---

## PR-D — Typed ability I/O sweep (`[output_schema]` + typed wrappers)

**Motivation.** Three of the modified `.ability.toml` files (`agent.start`, `agent.stop`, `a2a.client.send_task`) declare input shape but NO `[output_schema]`. Handlers return `json!({...})` with 8-11 stringly-typed keys; CLI surfaces consume with `daemon_response.get("…").and_then(.as_bool).unwrap_or(false)`. A daemon-side rename of any field is silently swallowed at every consumer until a test catches it three commits later.

**Proposed shape.** Three coordinated sub-tasks:

### D-1 Output schema TOML blocks

Add `[output_schema]` to every ability TOML where a CLI consumer reads named fields. Block shape mirrors `[input_schema]`. The codegen pass that produces TOMLs at boot also generates an `OutputShape` struct via `serde_derive` so the handler returns a typed value and the wire shape is pinned by the schema.

### D-2 Typed handler returns

`AxonAbilityCatalog::register_rpc_*` grows a variant accepting `Fn(Args) -> Result<Resp>` where `Args: DeserializeOwned` and `Resp: Serialize`. The current `Fn(Value) -> Result<Value>` shape stays as a fallback for hand-rolled handlers but new abilities use the typed variant. Boot lint warns on registrations using the untyped shape without an `#[allow(untyped_ability)]` annotation.

### D-3 Typed client invocation

`LocalDaemonAbilityClient::invoke` becomes generic:

```rust
pub fn invoke<R: DeserializeOwned>(
    &self,
    fn_name: &str,
    payload: impl Serialize,
) -> anyhow::Result<R>
```

CLI subcommands consume named structs end-to-end. The `.with_context("decode foo response")` boilerplate at every callsite disappears (one decode error path, in the client).

### D-4 Versioned envelope wrapper

The `{result, fulfilled_by}` envelope (e.g. `invoke.rs::unwrap_envelope`) becomes a versioned `ResultEnvelope<R>` struct with `Serialize + Deserialize`. The wire shape is documented at one place; sniffing for the two keys at multiple call sites disappears.

**Scope boundary.**
- IN: every ability TOML modified in the current PR gets `[output_schema]`. Every CLI subcommand that consumes a typed shape uses the typed client.
- IN: codegen update so future TOMLs require an output schema (lint or boot panic).
- OUT: abilities NOT touched in the current PR. (A second sweep can cover the rest if telemetry shows untyped responses still landing.)

**Ship criteria.**
1. Every TOML touched in the current PR has both `[input_schema]` and `[output_schema]`.
2. CLI invocation surfaces use typed `invoke::<Response>(…)` calls; no `.get(...).and_then(.as_bool).unwrap_or(...)` remains in the touched subcommands.
3. `a2a.client.send_task` schema declares `args: object` (currently free-form) with the SDK-side note that the inner shape is the remote skill's responsibility.
4. Schema mismatch (`additionalProperties = false` + a handler returning an extra key) is a CI failure, not a silent drift.

---

## PR-E — MCP namespace + RFC doc alignment

**Motivation.** The current PR flattens generated MCP ability names to `mcp_<server>_<tool>` (commit `efb4e15`). Underscores in the first segment violate the `OwnerKind` invariant the same PR introduced (`{device, hub, <agent>, <user>}` — `mcp_` matches none). Several RFC tables also still write `mcp.bridge.*` without the `device.` prefix even though the actual abilities are `device.mcp.bridge.*`. Two doc/code drifts.

**Proposed shape.**

### E-1 Pick the canonical namespace

Two options:
- **A: `device.mcp.<server>.<tool>`** — globally addressable; namespace is "any MCP tool any MCP server my daemon hosts." Wins for cross-host discovery, loses for per-agent scoping.
- **B: `<agent>.mcp_<server>_<tool>`** — per-agent scoped; underscores stay (the per-agent shape is currently advertised as `mcp_<server>_<tool>` under the agent's name). Wins for per-agent visibility, loses for cross-host enumeration.

Decision criterion: how does an LLM discover MCP tools? Today the discover ability returns names under the agent's prefix, which argues for B. Verify by walking the discover output produced by a hot-added agent in test fixtures.

### E-2 Apply the choice

Sweep `runtime/agents/mcp_*.rs` and `cli/agent.rs:2061-2092` to emit the canonical shape. Add the post-Stage-4 lint envisioned in `docs/open-questions/deprecate-self-alias-in-ability-names.md` §"Stage 4 ship criterion 6" so any new `register_rpc(name, ...)` whose first segment isn't in `{device, hub, <agent-id>, <user-id>}` fails the build.

### E-3 RFC doc sweep

Search-and-replace in `docs/rfc/AXON-RFC-001-discovery-planes.md`, `…-edge-adapter-bidirectionality.md`, `…-ability-layers.md`, `docs/spec/owner-truth-table/ability-owner-truth-table.tex`, `docs/rfc/AXON-RFC-001-c-m13-ontology-audit.md`: change `mcp.bridge.*` → `device.mcp.bridge.*` and `mcp.client.*` → `device.mcp.client.*` (or equivalent per E-1's choice).

**Scope boundary.**
- IN: pick + apply the canonical name shape. Update RFC tables.
- OUT: any breaking wire change with the hub (MCP names are device-local, no wire pinning).

**Ship criteria.**
1. `agents/mod.rs:2576-2584` lint passes the new shape unchanged (lint already covers `legacy self alias.*`; extending to "owner-kind first segment" closes the same hole for MCP).
2. RFC tables consistent with actual ability registration (grep the registration sites, compare).
3. `easynet ability list` output renders the chosen shape; truth-table spec updated.

---

## PR-F — Split `runtime/skill_store.rs` (1 059 lines → three concerns)

**Motivation.** The file is a single flat module trying to be three things: a metadata store (`InstallRecord`, `SkillSource`, read paths), an installer (download / extract / hash / register / upgrade / remove), and filesystem plumbing (bespoke `TempDirGuard`, tree-hash walker). 20+ public functions, no struct collecting per-skill state. Adding a new install variant means editing the middle of the file and hoping the surrounding functions stay coherent.

**Proposed shape.**

```
src/runtime/skill_store/
├── mod.rs            # re-exports the public surface; doc + invariants
├── metadata.rs       # InstallRecord / SkillSource types + list / read
├── installer.rs      # install / upgrade / remove + the saga that
│                     # rolls back partial installs
└── fsutil.rs         # tree hashing — but `TempDirGuard` is gone:
                      # replaced by `tempfile::TempDir` (already a
                      # crate dependency).
```

**Scope boundary.**
- IN: file split, replace `TempDirGuard` with `tempfile::TempDir`, restructure tree-hash to take `&Path` not `String`.
- OUT: changing the on-disk layout (`~/.easynet/skills/...`) or adding new skill-source kinds.

**Ship criteria.**
1. `cargo check --features axon-pb --tests` clean before + after each commit.
2. `src/runtime/skill_store/*.rs` each ≤ 500 LoC.
3. `TempDirGuard` removed; no reinvention of a tempfile in `src/runtime/`.
4. Existing skill install / upgrade / remove e2e tests pass unchanged.

---

## PR-G — Cross-cutting naming sweep (dispatch / local_ / refresh / ability namespace)

**Motivation.** The current diff lands four families of name drift that no PR-A–PR-F PR will reach individually:

- **`dispatch` is overloaded** across three layers — `runtime/ability_dispatch.rs` (handler registration / conversion), `services/control/runtime_dispatch.rs` (control-plane routing), `services/invocation_transport/daemon_invocation_service.rs` (gRPC fn_name arms). `git grep dispatch` returns three unrelated meanings.
- **`local_*` prefix** is split across four different scopes — `runtime/local_runtime_invoker.rs` (CLI JSON adapter), `services/invocation_transport/local_session_dispatcher.rs` (device session dispatcher, 1 512 LoC), `support/local_invoke.rs` (daemon-side fallback invoke), `support/local_daemon_grpc.rs` (socket-path resolver). Four meanings, one prefix.
- **`refresh` verb**, while now adequately documented in `agent.refresh`'s description (this PR), still has two reading paths an operator may confuse.
- **MCP namespace shape** open per PR-E (`mcp_<server>_<tool>` vs `<agent>.mcp_<server>_<tool>` vs `device.mcp.<server>.<tool>`); PR-E owns the policy choice, PR-G owns the codebase sweep when PR-E decides.

**Proposed shape.** A coordinated rename, one PR:

### G-1 `dispatch` family rename

- `runtime/ability_dispatch.rs::rpc_handler_to_ability_fn` and siblings → `into_axon_ability_*` (action-only nouns; the file-level doc names the conversion contract).
- `services/control/runtime_dispatch.rs` → keeps `dispatch_` (control-plane routing IS dispatch).
- `services/invocation_transport/daemon_invocation_service.rs` arms → rename `dispatch_*` arms to `route_*` arms (they branch on `function_name`; that's routing, not dispatch).
- Add a 3-line table in `docs/design/daemon-layers-v1.md` naming the three meanings + the layer that owns each.

### G-2 `local_*` prefix rename

- `services/invocation_transport/local_session_dispatcher.rs` → `session_dispatch.rs` (drop the `local_`; sessions are by definition device-side).
- `support/local_invoke.rs` → leave name (canonical CLI surface).
- `support/local_daemon_grpc.rs` → leave name (the prefix here means "talks to the local daemon over a UDS").
- `runtime/local_runtime_invoker.rs` → leave name (the prefix here means "drives the local Axon LocalRuntime"), but add a header section explaining the `local_*` semantic in each module so `git grep local_` returns interpretable hits.

### G-3 Apply PR-E's MCP namespace decision

Sweep `runtime/agents/mcp_*.rs` and `cli/agent.rs:2061-2092` to emit the canonical shape PR-E chose. Update the post-Stage-4 `OwnerKind`-first-segment lint to require the shape.

**Scope boundary.**
- IN: rename + a single round of `git grep` / `cargo check` verification per family.
- OUT: any behavioural change. Each rename ships as one `git mv` / `sd` commit per family with byte-equivalent semantics.

**Ship criteria.**
1. `git grep -E 'fn dispatch_|fn route_'` returns hits only at the layer that owns each verb.
2. `git grep local_runtime_invoker` returns the same set of files before + after (no inadvertent rename).
3. Post-Stage-4 lint extension active: any new `register_rpc(name, ...)` whose first segment is not in `{device, hub, <agent-id>, <user-id>, <chosen-MCP-shape>}` fails the build.

**Sequencing.** Lands AFTER PR-E (which picks the MCP shape). Otherwise standalone.

---

## PR-D update — `[output_schema]` requires renderer extension

**Discovery during 2026-05-29 follow-up implementation.** The on-disk ability TOMLs are **codegenerated** by `src/bin/gen-ability-tomls.rs` from `runtime/agents/ability_toml::render_ability_toml(name, description, input_schema)`. The renderer signature deliberately omits `output_schema` (a drift test compares its byte-exact output against on-disk). Adding `[output_schema]` blocks by hand to abilities/system/*.toml is a foot-gun: the next codegen run wipes them and the drift test rejects the wipe.

**PR-D therefore must extend:**

1. `SystemAbilityMetadata` (in `runtime/agents/mod.rs:1160`) — add `output_schema: Option<Value>`.
2. New per-ability function family: `output_schema_for(name) -> Option<Value>` matching the shape of `input_schema_for(name)`.
3. `render_ability_toml` — extended signature `(name, description, input_schema, output_schema: Option<&Value>)`. Renderer must learn `oneOf` (today's design doc explicitly excludes it); a future-compat `oneOf` clause is what response envelopes need.
4. Drift test — extend to require an output_schema for every ability whose handler returns a structured object (i.e. anything that isn't pure ack: bool).
5. The seven ability TOMLs the 2026-05-29 follow-up identified (`a2a.client.send_task`, `device.agent.{start,stop,refresh}`, `device.invocation.history.{list,path}`, `terminal.list`) get their `[output_schema]` blocks via this path, not by hand.

Until PR-D ships, the descriptions for those 7 abilities have been improved in-place through `description_for(name)` (lands in the current PR via codegen regenerate); the output_schema block is the PR-D deliverable.

---

## Sequencing summary

```
PR-A  daemon_invocation_service split
   ↓ (PR-A must land first — gives PR-B per-arm files to reshape)
PR-B  Builder::build() + grouped capability structs
   ↓ (parallel-safe with PR-C onwards; can land any order after PR-A)

PR-C  start/stop_agent split + typed args     (standalone)
PR-D  typed I/O sweep + render_ability_toml   (standalone; touches more files than PR-C)
       output_schema support
PR-E  MCP namespace + RFC alignment           (standalone; touches docs + register sites)
PR-F  skill_store split + TempDir replacement (standalone)
PR-G  cross-cutting naming sweep              (lands after PR-E for the MCP shape)
```

PR-A and PR-B together delete the largest source of structural debt in `services/invocation_transport/`. PR-C delivers the same payoff for `runtime/agents/agent_lifecycle_ability.rs`. PR-D + PR-E close the wire/contract polishing. PR-F and PR-G ship after the bigger structural moves so they don't conflict.

---

## Why these are NOT in the current PR

The current PR already deletes 8 278 lines and adds 6 251 across 129 files. Adding the five PR-A through PR-E scopes would push it past 25 000 net changed lines, none of which a single reviewer can hold in context. Splitting respects the same industrial-textbook standard the review applied: **a PR that cannot be reviewed is debt, not progress**.

The current PR's contribution toward each follow-up:
- PR-A: nothing concrete; the file is a candidate, this doc names it.
- PR-B: NonZeroU64 fix (P3-18 in the review) is a sliver of the builder restructure.
- PR-C: `start_agent_handler` was identified; no code change.
- PR-D: `_caller_envelope` was pruned (P0-4); `LocalDaemonAbilityClient` consolidation (P1-7) preps the typed-invoke generalization site. Discovery in the 2026-05-29 follow-up: `[output_schema]` blocks require a renderer extension (see PR-D update section above).
- PR-E: MCP naming open, RFC tables open.
- PR-F: skill_store identified as 1 059 LoC three-concerns file; no split yet.
- PR-G: dispatch/local_/refresh naming drift documented; the sweep itself defers until PR-E picks the MCP shape.

## What the 2026-05-29 follow-up PR DID land (smaller items not deferred)

These are the items the second-pass review identified that were small enough to land in the same PR without breaking the "reviewable" bound. Each is independent and grep-anchored:

- **Status-code policy docstring** at top of `services/invocation_transport/daemon_invocation_service.rs` — three explicit classes (internal / invalid_argument / failed_precondition) plus not_found / permission_denied / unimplemented, each with a one-paragraph definition naming the caller's expected response. Future arm authors have the policy in front of them.
- **`device.agent.{start,stop,refresh}` hard-Err on missing tokio runtime** when the hot registrar IS wired. The previous code returned a silent `runtime_not_ready` envelope that operators could mistake for the legitimate boot-window state. Now: registrar empty + no tokio = `runtime_not_ready` envelope + warn-class op_event; registrar wired + no tokio = anyhow::bail with the exact wiring step that's missing.
- **`block_on_runtime` wrapper deleted** in `runtime/local_runtime_invoker.rs`; three call sites now reach `support::async_bridge::run_blocking(..., NoRuntimeFallback::BuildCurrentThreadTokio)` directly. `block_on_runtime_sync` retained in `ability_dispatch.rs` (7 call sites, all under the same in-memory-only invariant, documented in the helper's docstring).
- **`stamp_bidi_down_sequence` helper extracted** in `daemon_invocation_service.rs`; the two byte-identical `stamp_sequence` methods on `LocalBidiDownStream` and `SessionDownStream` now both delegate. Future PR-A's per-arm split can move them anywhere without splitting the saturating_add semantic.
- **`late_bound_rpc_handler` op_event** in `runtime/ability_dispatch.rs::invoke_rpc_json`. The self-heal path (handler in catalogue but missing from LocalRuntime) is no longer silent — operators see when boot's sync_runtime_ability is incomplete.
- **`invoke_daemon_ability_required` helper** in `cli/agent.rs`. The four `invoke_daemon_agent_*_required` wrappers now delegate to one shared helper that owns the error-format policy. The wrappers stay as 1-line named entry points for `git grep`.
- **CLI `agent refresh --agent <name>`** new flag. Previously `easynet agent refresh` always rebuilt every row; now operators can target one row. Wired through the `agent.refresh` `name` field that was already in the input schema.
- **`daemon/axon_bridge/` moved from `services/`** — its imports went almost entirely to `runtime/*`; the `services/` placement was a false hierarchy. Project-structure v1 later made the daemon ownership explicit by placing the bridge under `src/daemon/axon_bridge/`.
- **`check-kernel-boundary.sh` extended with retired-tree rules** — current rules reject `runtime/agents/` and `runtime/axon_bridge/` returning after their daemon-domain moves. New daemon Invocation imports still go through the explicit runtime allowlist in the script.
- **Description improvements at the source** for four abilities (`a2a.client.send_task` args opacity, `agent.refresh` semantics, `invocation.history.path` singular, `terminal.list` namespace alias). Landed through `description_for(name)` so the next `gen-ability-tomls` run keeps them; the output_schema half waits for PR-D's renderer extension.

The follow-up PRs cite this document by file path in their commit message so the lineage is grep-able six months out.
