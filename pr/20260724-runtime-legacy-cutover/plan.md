# Runtime Legacy Cutover Plan

## Goal restatement

Continue converging EasyNet-Cli toward the canonical runtime model by removing
legacy/compat layers that preserve pre-SPEC behavior. The change must improve
architecture rather than add feature surface.

## Scope for this iteration

- Inspect current runtime, SDK facade, descriptor, admission, and catalog paths.
- Select one root legacy/compat layer whose removal improves canonical runtime
  convergence without changing public product interfaces.
- Refactor from the owned lower layer upward.
- Add or update deterministic tests that prove the removed path cannot re-enter.

## Non-goals

- Do not add EasyNet/EasyRemote-specific SDK abstractions.
- Do not add fallback paths for old storage, old descriptor inference, or old
  route discovery.
- Do not loosen daemon admission to hide product ingress defects.

## Invariants

1. Runtime invocation tuples remain explicit: caller, callee, subject, ability,
   action, descriptor, and signer authority must be bound before dispatch.
2. Descriptor resolution must not synthesize a route from product naming or hidden
   discovery probes.
3. Session authority may admit only the canonical subject family encoded in its
   authority metadata.
4. Legacy data shape migration is not a runtime authority path. If old data is
   unsupported, loaders must fail closed or start from an empty canonical store.
5. Tests must prove the obsolete path is absent, not merely unused by one caller.

## Boundary proof

- Core runtime owns admission, descriptor resolution, signer binding, and receipt
  finalization.
- CLI/product surfaces may construct user intent but must not repair canonical
  tuple state through defaults.
- SDKs expose generic runtime concepts only; product lifecycle and product naming
  stay outside the SDK.

## Verification plan

- Run targeted Rust tests for the touched module.
- Run formatting checks for touched Rust code.
- Run architecture/SPEC gates when the change reaches a stable boundary.
- Re-check repository searches for the removed legacy pattern.

## Iteration 1 decision log

- `principal_lifecycle` was inspected first because it had legacy-store test
  fixtures. The production loader is already canonical and fail-closed via
  `deny_unknown_fields`; changing it would only rename tests, not remove a
  runtime compatibility path.
- The selected cutover target is daemon admission naming and receipt proof
  provenance. Runtime admission still used `ProductAdmission*`,
  `product_policy`, and the proof issuer string
  `easynet-cli.product_admission.v1`. That branded the canonical admission
  proof as an EasyNet product policy even though the object coordinates generic
  runtime admission around descriptor-bound envelopes, quota reservation, and
  authority proof facts.
- The refactor renames the state machine to `RuntimeAdmission*` /
  `DaemonRuntimeAdmission*`, updates call sites, and changes the proof issuer to
  `runtime.admission.v1`.
- Targeted verification exposed a non-hermetic plugin host test: the test
  catalog used `AxonAbilityCatalog::new_with_runtime`, which derives test device
  authority from local credentials. The helper now injects an explicit test
  device URA so runtime-admission tests do not depend on developer machine
  pairing state.

## Iteration 2 candidate policy

- Continue scanning for production legacy/compat paths after the runtime
  admission terminology cutover.
- Prefer paths that:
  1. preserve product-specific naming inside runtime core,
  2. retain fallback route/authority/defaulting behavior,
  3. weaken receipt/admission/descriptor proof provenance,
  4. make tests depend on local developer state.
- Reject candidates that are only negative tests, generated compatibility
  comments, or product-boundary features intentionally outside canonical SDK.

## Iteration 2 decision log

- codegraph and targeted searches found a production descriptor compatibility
  path in `AbilityDescriptor::with_hints`: advisory
  `hints.streaming_only` / `hints.bidi_only` still selected canonical
  `call_mode`.
- This violates the canonical runtime model because routing, descriptor hashing,
  and descriptor refs are governed by `call_mode`; presentation hints must not
  become a second transport authority.
- The cutover target is therefore the descriptor construction state machine:
  `call_mode` remains explicit and synchronizes presentation hints outward,
  while `with_hints` only records advisory UI facts and rejects contradictory
  transport hints at the wire boundary.
- Verification must prove that hints cannot change call mode or descriptor refs,
  and that wire payloads with contradictory hints fail closed instead of being
  interpreted through a legacy transport selector.

## Iteration 3 candidate policy

- Focus on SDK receipt/proof-fact parity because public product flows rely on
  SDK-side validation before and after daemon submission.
- Prefer removal of language-local canonicalizers that can accept weaker receipt
  or authority facts than the canonical runtime model.
- The first audit target is Java receipt/proof-fact handling. If Java accepts a
  receipt or authority shape that Go/Python/Rust reject, that is a cross-language
  compatibility fork, not a product feature.
- Verification must include the touched language tests, API inventory gates when
  SDK public surface changes, and the canonical runtime convergence gate.

## Iteration 3 decision log

- The Java receipt proof-fact constructor already calls
  `RuntimeReceiptProofFacts.validate(raw)`, but codegraph exposed a separate
  authority subject predicate inside `InvocationAuthorityBindingValidator`.
- That predicate parsed the invocation subject by substring search for
  `/resource/`, which can admit path-shaped strings without proving a canonical
  URA resource owner projection. Go and Python already route this through
  structured URA parsing.
- The selected cutover is to move Java session-authority subject admission into
  `AuthoritySupport` and remove the validator-local parser. The helper now
  admits exact subject equality or canonical resource URAs owned by the session
  user / that user's agent, and rejects path-substring impostors.
- This keeps the public Java API unchanged while converging Java onto the same
  SDK authority model as Go/Python.

## Iteration 4 candidate policy

- Focus on public ingress tuple construction and route facades.
- Prefer removal of CLI/product-facing code that still:
  1. accepts partial invocation tuples and repairs them through defaults,
  2. preserves non-canonical "not wired" / feature-disabled invoke paths,
  3. translates legacy target fields into descriptor-bound invocation state,
  4. keeps product-specific route discovery outside LocalRuntime.
- The first audit target is `src/cli/commands/invoke.rs` plus
  `src/cli/commands/invocation_tuple.rs`, because these are direct public
  ingress surfaces for product smoke tests and operator reproduction.
- Verification must prove the public CLI can only construct descriptor-bound
  canonical tuples, and axon-pb disabled builds fail with canonical unsupported
  errors rather than legacy invoke wording.

## Iteration 4 decision log

- codegraph and targeted search found a production-facing compatibility remnant
  in `ability invoke`: the axon-pb-disabled branch and its tests still described
  the public remote path as a legacy/not-wired message.
- `ability invoke`, `ability stream`, and `ability bidi` also duplicated their
  remote-transport disabled messages. That duplication makes it easy for one
  public ingress surface to drift back into product-specific or legacy wording.
- The refactor adds a shared public-ingress helper,
  `remote_invocation_transport_unsupported`, in `invocation_tuple.rs` and routes
  all three CLI surfaces through it.
- The invoke test now asserts that retired `not wired` / `legacy` wording is not
  accepted, and the SPEC v2 gate rejects reintroducing the old text or bypassing
  the shared helper.

## Iteration 5 candidate policy

- Focus on feature/cfg ownership. A canonical read model must not disappear just
  because one transport carrier is disabled.
- `cargo check --no-default-features --lib` currently fails because
  `invocation_history.rs` imports `dispatch::attempt_audit`, while
  `attempt_audit` is entirely behind `feature = "axon-pb"`.
- The root abstraction problem is that the attempt audit ledger mixes two
  responsibilities:
  1. transport-specific protobuf request projection, and
  2. transport-independent JSONL attempt read/write records consumed by
     invocation history.
- The intended cutover is to make the ledger record/store available independent
  of axon-pb and cfg-gate only protobuf request adapters.
- Verification must include `cargo check --no-default-features --lib` so this
  does not regress back into a hidden feature compatibility fork.

## Iteration 5 decision log

- codegraph confirmed `attempt_audit` is consumed by both daemon transport and
  the `invocation.history.list` governance read model.
- The selected cutover keeps the attempt ledger module always available and
  gates only protobuf/tonic adapters behind `axon-pb`.
- `InvocationAttemptHandle::reject_diagnostic` is now the transport-neutral
  terminal-state API; `reject_status` is only a tonic adapter.
- The default real invocation history test was also migrated off the retired
  provider filter field `agent_ura`. Runtime history providers accept canonical
  `callee_ura` / `subject_ura`; CLI-only `--agent` lowering must not enter the
  provider schema.
- SPEC v2 now includes an explicit feature-boundary guard to prevent the ledger
  module from being transport-gated again.

## Iteration 6 candidate policy

- Focus on SDK provider-output projection. A product-neutral SDK must not
  silently turn malformed provider directory rows into empty records.
- codegraph located the shared Directory projection surfaces in Go and Python.
  Both already reject malformed container types, but record projection still
  accepts a mapping without canonical `kind` or any canonical URA fact by
  projecting empty strings.
- The root abstraction problem is not legacy alias promotion; that was already
  blocked. The remaining compatibility behavior is weaker: malformed provider
  rows survive as empty canonical records and defer failure to product code.
- The intended cutover is to make Directory record projection fallible in both
  SDKs and require canonical record identity facts at the SDK boundary.
- Verification must cover Go/Python directory tests and the canonical runtime
  convergence gate so both SDK implementations remain aligned.

## Iteration 6 decision log

- The selected cutover was SDK Directory record projection, not product device
  rendering. Products may still decide how to display directory rows, but SDK
  provider output must either carry canonical record facts or fail closed.
- Go now uses an internal `projectDirectoryRecord` strict projector inside
  `ProjectDirectoryResolution`. The exported `ProjectDirectoryRecord` signature
  remains unchanged for public API compatibility, but it is no longer the
  provider-output authority path.
- Python's private `_project_record` now requires `kind` and at least one
  canonical URA fact (`ura`, `owner_ura`, `ability_ura`, or `route_ura`).
- Both SDKs now reject alias-only rows such as `{type, canonical_name}` at the
  provider boundary instead of producing empty canonical records.
- SPEC v2 now checks the strict record projector, canonical record fact guards,
  and Go/Python tests for missing record identity facts.
- Verification passed:
  - `go test ./... -run Directory` in `sdk/go`
  - `PYTHONPATH=sdk/python:../EasyNet-Axon/sdk/python python3 -m pytest -q sdk/python/tests/test_directory.py`
  - `tools/scripts/check-canonical-runtime-convergence-v2.sh`
  - `tools/scripts/check-architecture-convergence.sh`
  - `git diff --check`

## Iteration 7 candidate policy

- Focus on carrier-v1 destination dispatch. codegraph found production-facing
  `not wired` branches in `local_session_dispatcher.rs` for LocalRuntime,
  admission graph, trust sync, and bidi ability publication.
- The root abstraction problem is not the wording alone. A canonical dispatcher
  should express missing runtime assembly as one typed precondition, not as
  scattered compatibility-era strings that imply an optional second path.
- The intended cutover is to centralize LocalRuntime/admission/trust precondition
  projection and make failures say the canonical assembly is unavailable or the
  requested ability is not published for the negotiated carrier.
- Verification must cover local session dispatcher tests and SPEC v2 must reject
  reintroducing production `not wired` wording in this file.

## Iteration 7 decision log

- `LocalAxonSessionDispatcher` now uses one `require_local_runtime` helper and
  one canonical assembly diagnostic for carrier-v1 RPC and stream dispatch.
- Device trust sync absence now reports a missing canonical runtime assembly
  component instead of an optional wiring hole.
- Remote bidi missing ability publication now reports a carrier-v1 publication
  failure; it no longer describes a product route as "not wired".
- Destination runtime admission absence now reports a required canonical
  admission graph, not a compatibility-era optional graph.
- SPEC v2 now rejects production `not wired` wording and manual LocalRuntime
  optional branches in `local_session_dispatcher.rs`.
- Verification passed:
  - `cargo test -q local_session_dispatcher::tests::carrier_v1`
  - `cargo fmt --check`
  - `tools/scripts/check-canonical-runtime-convergence-v2.sh`
  - `tools/scripts/check-architecture-convergence.sh`
  - `git diff --check`

## Iteration 8 candidate policy

- Focus on daemon transport dispatchers. codegraph found the same optional
  runtime assembly branch still present in unary, server-stream, and bidi
  dispatchers through direct `RuntimePlane::local_runtime()` option handling.
- The root abstraction problem is that `RuntimePlane` owns the daemon runtime
  assembly state but exposes only an optional getter to production dispatchers.
  That lets each transport invent its own "not wired" failure and preserves the
  idea that LocalRuntime is an optional side path.
- The intended cutover is to add fallible `RuntimePlane` requirement helpers for
  LocalRuntime and runtime admission, then migrate unary/stream/bidi dispatchers
  through that single boundary.
- Session-control hub realm absence is also a canonical session precondition,
  not a wiring compatibility state; production diagnostics should say the
  carrier lacks session realm context.
- Verification must cover focused daemon route/transport tests, rustfmt, SPEC
  v2, architecture convergence, and whitespace checks.

## Iteration 8 decision log

- Added `RuntimePlane::require_local_runtime` as the single production boundary
  for daemon transport handlers that require the canonical daemon runtime
  assembly.
- Migrated unary exact routes, unary local abilities, server-stream exact
  routes, server-stream local abilities, bidi exact routes, and bidi local
  abilities away from direct `local_runtime()` option handling.
- Updated runtime admission graph absence to report a canonical daemon runtime
  assembly requirement instead of an optional wiring hole.
- Updated session-control hub realm absence to report missing canonical session
  realm context.
- Updated principal enrollment proof handling to require a canonical
  PrincipalLifecycle provider instead of saying the provider is "not wired".
- SPEC v2 now checks the RuntimePlane helper, rejects production optional
  runtime wiring wording in unary/stream/bidi dispatchers, and prevents direct
  transport-level Option-to-Status conversion for LocalRuntime.
- Verification passed:
  - `cargo test -q invoke_stream_dispatches_registered_local_stream_ability`
  - `cargo test -q invoke_stream_dispatches_remote_selected_route_over_presence_session`
  - `cargo test -q exact_bidi_route_family_registers_hub_owned_session_open`
  - `cargo test -q local_bidi_down_stream_preserves_supplied_initial_frame_before_handler_frames`
  - `cargo fmt --check`
  - `tools/scripts/check-canonical-runtime-convergence-v2.sh`
  - `tools/scripts/check-architecture-convergence.sh`
  - `git diff --check`

## Iteration 9 candidate policy

- Focus on the daemon Kernel runtime boundary. codegraph found `Kernel::invoke`
  and `KernelApi::prepare_local_system_rpc` still reading the `OnceLock`
  directly and reporting `kernel LocalRuntime is not wired`.
- The root abstraction problem is the same as the transport dispatchers: the
  Kernel owns a lifecycle precondition but exposes it as ad hoc optional runtime
  lookup at each entry point.
- The intended cutover is to add one `Kernel::require_local_runtime` boundary
  and route every Kernel local-runtime entry through it.
- The failure should describe a missing canonical daemon kernel runtime assembly,
  not a compatibility wiring hole.
- Verification must cover Kernel invoke failure, KernelApi prepare path through
  compile/tests, rustfmt, SPEC v2, architecture convergence, and whitespace
  checks.

## Iteration 9 decision log

- Added `Kernel::require_local_runtime` as the single daemon-kernel boundary for
  LocalRuntime availability.
- Migrated `Kernel::invoke` and `KernelApi::prepare_local_system_rpc` away from
  direct `OnceLock` LocalRuntime lookup.
- Missing runtime now reports a canonical daemon kernel runtime assembly
  precondition instead of a wiring compatibility hole.
- SPEC v2 now checks that Kernel runtime lookup remains centralized and that
  both Kernel entry points route through `require_local_runtime`.
- Verification passed:
  - `cargo test -q kernel_invoke_without_runtime_returns_error_without_receipt`
  - `cargo test -q prepare_local_system_rpc`
  - `cargo test -q invoke_rejects_non_device_session_projection_without_admitting_row`
  - `cargo fmt --check`
  - `tools/scripts/check-canonical-runtime-convergence-v2.sh`
  - `tools/scripts/check-architecture-convergence.sh`
  - `git diff --check`

## Iteration 10 candidate policy

- Focus on `device_control/ability_management/ops.rs`. codegraph found
  production `federation_not_wired` wording and direct registrar `not wired`
  diagnostics in node/ability mutation handlers.
- The root abstraction problem is target lifecycle ambiguity: the handlers
  accept a `node_id` but branch procedurally between local device mutation and
  remote device mutation without an explicit target state.
- Current SPEC-aligned capability matrix treats remote device ability mutation
  as Unsupported unless a provider-backed federation mutation route exists.
  The handler must fail closed as an unsupported canonical capability state,
  not as a removed surface that will be re-wired later.
- Device ability registrar absence is a daemon runtime assembly precondition,
  not a compatibility wiring hole.
- The intended cutover is to introduce a local/remote target classifier,
  migrate remove/deploy/uninstall handlers through it, centralize registrar
  availability, and reject reintroduced federation-not-wired wording in SPEC v2.

## Iteration 10 decision log

- Added `DeviceOperationTarget` as the explicit local/remote target state for
  device ability mutation handlers.
- Removed the daemon ability-management `federation_not_wired` compatibility
  helper and retired "re-wired/follow-up" implementation vocabulary from the
  production path.
- Remote node remove/deploy/uninstall now fail closed as
  `capability_state=unsupported` under the canonical runtime capability matrix.
- Added `require_device_registrar` so device ability registrar absence is a
  daemon runtime assembly precondition rather than a wiring compatibility hole.
- Updated ability deploy test fixtures to include strict manifest
  `schema_version` when the test is targeting later manifest/registrar logic.
- SPEC v2 now checks the target state, Unsupported remote mutation diagnostics,
  centralized registrar lookup, and regression tests for all three remote
  mutation surfaces.
- Verification passed:
  - `cargo test -q remove_node_remote_target_is_unsupported_capability_state`
  - `cargo test -q deploy_ability_remote_target_is_unsupported_before_bundle_materialization`
  - `cargo test -q uninstall_ability_remote_target_is_unsupported_capability_state`
  - `cargo test -q deploy_ability_parses_canonical_manifest_then_requires_registrar`
  - `cargo test -q uninstall_ability_requires_canonical_registrar`
  - `cargo test -q deploy_ability_bundle_parser_strips_namespace_from_canonical_manifest_bytes`
  - `cargo test -q deploy_ability_wired_transaction_completes_inside_current_thread_runtime`
  - `cargo fmt --check`
  - `tools/scripts/check-canonical-runtime-convergence-v2.sh`
  - `tools/scripts/check-architecture-convergence.sh`
  - `git diff --check`

## Iteration 11 candidate policy

- Focus on CLI/support federation feature-off paths. codegraph found
  `support::platform::local_invoke::federation_not_wired_error` with six CLI
  callers and diagnostics that describe missing `axon-pb` build wiring.
- The root abstraction problem is capability-state leakage: a CLI action that
  requires federation reach should fail as an Unsupported canonical runtime
  capability state when the federation transport provider is unavailable, not
  as a production-build feature wiring problem.
- The intended cutover is to replace the support-layer helper with a
  product-neutral `federation_capability_unsupported_error`, migrate every
  feature-off caller to it, and gate against reintroducing `not wired`,
  `production builds always do`, or `federation_not_wired_error`.
- This preserves public CLI shape while removing the compatibility-era
  implementation narrative from the error boundary.

## Iteration 11 decision log

- Replaced `federation_not_wired_error` with
  `federation_capability_unsupported_error` in the support-layer federation
  feature-off boundary.
- Migrated every feature-off CLI federation caller (`devices`, `device` group,
  `reset`, `join`, and remote system ability forwarding) to the new canonical
  Unsupported helper.
- Feature-off federation diagnostics now report
  `capability_state=unsupported` because the federation transport provider is
  unavailable under the current runtime capability matrix.
- Removed production-build feature wiring vocabulary from the user-facing
  boundary and replaced a `follow-up trust auto-wire` implementation note with
  trust synchronization terminology.
- SPEC v2 now rejects the retired helper name, `axon-pb` feature wiring
  diagnostic, `production builds always do`, and related retired vocabulary
  across the support helper plus all six CLI caller files.
- Verification passed:
  - `cargo check --no-default-features --lib -q`
  - `cargo test -q non_canonical_node_ura_returns_actionable_error`
  - `cargo fmt --check`
  - `tools/scripts/check-canonical-runtime-convergence-v2.sh`
  - `tools/scripts/check-architecture-convergence.sh`
  - `git diff --check`

## Iteration 12 candidate policy

- Focus on shared device directory projection. `device show` already fails
  closed when a node read model returns numeric legacy enum state, but
  `support::platform::node::node_state_str` still translated numeric protobuf
  values for device list/status renderers.
- The root abstraction problem is split authority over directory row shape:
  one product path requires canonical string state while the shared helper still
  accepts a legacy wire enum and silently turns it into a display state.
- The intended cutover is to make the shared node state projection string-only.
  Numeric, object, boolean, missing, or otherwise malformed state projects to
  `UNKNOWN` and never influences online filtering.
- Verification must prove numeric `state` no longer maps to `HEALTHY`, `JOINING`
  or other canonical states, and SPEC v2 must reject reintroducing numeric enum
  projection in the shared helper.

## Iteration 12 decision log

- Removed numeric protobuf enum projection from
  `support::platform::node::node_state_str`; the helper now treats only string
  directory read-model states as canonical.
- Numeric, missing, or malformed state now projects to `UNKNOWN` and cannot make
  `node::is_online` return true unless an explicit `online: true` fact is
  present.
- Added focused tests proving `state: 3` no longer becomes `HEALTHY` and that
  canonical string `HEALTHY` still drives online projection.
- Added SPEC v2 coverage plus a negative self-test fixture that fails if the
  retired numeric projector returns.
- This closes the split authority where `device show` rejected legacy numeric
  state but device list/status still accepted it through the shared helper.
- Verification passed:
  - `cargo test -q support::platform::node::tests`
  - `cargo fmt --check`
  - `tools/scripts/check-canonical-runtime-convergence-v2.sh`
  - `tools/scripts/check-architecture-convergence.sh`
  - `git diff --check`

## Iteration 13 candidate policy

- Focus on remote failure classification. Product-visible errors showed
  descriptor resolution and history reads surfacing caller signer/key-custody
  failures under a legacy `ABILITY_NOT_FOUND` envelope.
- The root abstraction problem is semantic downgrading at the carrier boundary:
  a caller signer readiness failure is an admission/identity precondition, not
  ability absence. If the remote carrier reports the wrong outer code, the
  forwarding daemon must preserve the stronger semantic class from detail
  rather than returning NotFound.
- The intended cutover is to classify caller signer/key custody detail before
  the NotFound branch in `remote_failure.rs`, matching the existing explicit
  `CALLER_SIGNER_UNAVAILABLE` classification.
- Verification must prove `ABILITY_NOT_FOUND` with signer-readiness detail
  becomes `PermissionDenied`, while real descriptor-missing failures remain
  `NotFound` and route-negative remains `Unavailable`.

## Iteration 13 decision log

- Added a `remote_failure.rs` signer-readiness classifier that runs before the
  NotFound branch and recognizes `CALLER_SIGNER_UNAVAILABLE`, `requires a caller
  signer`, `self-identity`, and `keyring entry not found` detail.
- Preserved existing behavior for true descriptor-missing failures
  (`ABILITY_NOT_FOUND` -> `NotFound`) and route-negative owner-offline failures
  (`ROUTE_NEGATIVE` -> `Unavailable`).
- Added a focused regression test proving legacy `ABILITY_NOT_FOUND` outer codes
  cannot downgrade caller signer/key-custody failures to ability absence.
- Extended SPEC v2 remote-failure coverage with helper/order checks and a
  negative self-test fixture that models the retired downgrade.
- Verification passed:
  - `cargo test -q remote_failure::tests`
  - `cargo fmt --check`
  - `tools/scripts/check-canonical-runtime-convergence-v2.sh`
  - `tools/scripts/check-canonical-runtime-convergence-v2.sh --self-test`
  - `tools/scripts/check-architecture-convergence.sh`
  - `git diff --check`

## Iteration 14 candidate policy

- Focus on Mission/Agent trace artifact ownership. codegraph still finds a
  production `agent send --trace <file>` argument, a dispatch
  `extra_trace_path` field, and a legacy prompt-sidecar mirror that writes
  `<trace>.prompt.txt` outside the canonical mission run directory.
- The root abstraction problem is a second artifact sink beside the canonical
  Mission run directory and invocation ledger. Trace consumption has already
  converged on `invocation watch --trace <trace_id>` / `invocation.trace.get`;
  keeping a filesystem trace path on `agent send` preserves a legacy export
  lifecycle that is not tied to admission, receipts, or run metadata.
- The intended cutover is to remove the unused CLI `agent send --trace` flag,
  remove `extra_trace_path` from the dispatch request model and function
  signatures, remove the silent prompt sidecar write, and remove the reserved
  `MissionRunOpts.trace_path` field.
- This intentionally fails closed for the retired flag instead of accepting and
  ignoring it. The public trace story is the canonical run/ledger read path:
  users reference the printed mission run dir or `invocation watch --trace`.
- Verification must prove no production source can pass an external trace path
  into Mission/Agent dispatch, no prompt sidecar write exists, and SPEC v2
  rejects reintroducing the retired fields/flag.

## Iteration 14 decision log

- Removed the retired `agent send --trace <file>` argument from the CLI send
  surface. This makes the old path fail closed instead of accepting a flag that
  no longer owned the canonical trace lifecycle.
- Removed `extra_trace_path` from `AgentDispatchRequest`,
  `send_to_agent_with_depth`, and every Mission dispatch call site. The dispatch
  request model now only carries runtime inputs that are consumed by the
  canonical Mission/Agent execution path.
- Deleted the legacy prompt-sidecar writer that silently wrote
  `<trace>.prompt.txt` next to a caller-supplied file. Prompt/run artifacts stay
  in the agent run directory, while trace consumption remains the canonical
  mission run directory / invocation ledger read path.
- Removed the unused reserved `MissionRunOpts.trace_path` field and its `None`
  initializers from both mission ability and EAL executor callers.
- Added a SPEC v2 gate,
  `check_mission_agent_trace_sink_cutover_contract`, plus a negative self-test
  fixture that fails if `agent send --trace`, `extra_trace_path`,
  `with_extension("prompt.txt")`, or `MissionRunOpts.trace_path` return.
- Verification passed:
  - `cargo check -q`
  - `cargo test -q daemon::execution::mission::dispatch::tests`
  - `cargo fmt --check`
  - `tools/scripts/check-canonical-runtime-convergence-v2.sh`
  - `tools/scripts/check-canonical-runtime-convergence-v2.sh --self-test`
  - `tools/scripts/check-architecture-convergence.sh`
  - `git diff --check`

## Iteration 15 candidate policy

- Focus on installed-skill directory ownership. codegraph/rg found that Mission
  workspace seeding writes Codex project skills to `.agents/skills`, while
  `skill.publish`, `skill.list`, and `easynet skill install/upgrade/remove`
  still use `<agent-root>/skills` for Codex agents.
- The root abstraction problem is duplicated directory projection for one
  runtime capability. A skill package is an Agent-runtime resource; its managed
  directory must be derived once from the Agent runtime layout, not independently
  by workspace boot, publish, list, and install code.
- The intended cutover is to move managed skill directory selection into the
  daemon skill store, make Claude Code map to `.claude/skills`, Codex/Codex App
  Server map to `.agents/skills`, and keep only External runtimes on the
  generic `<root>/skills` path.
- This removes the legacy Codex audit-only `<root>/skills` path from managed
  runtime skills. The existing global pools remain runtime-specific
  (`~/.claude/skills`, `~/.agents/skills`) and are not a fallback for managed
  per-agent installs.
- Verification must prove publish/list/install/upgrade/remove all consume the
  shared directory helper and that SPEC v2 rejects reintroducing Codex managed
  skills under `<root>/skills`.

## Iteration 15 decision log

- Added `daemon::resources::skills::store::managed_skill_dir_for` as the single
  managed skill directory projection for registered Agent runtime workspaces:
  Claude Code → `.claude/skills`, Codex/Codex App Server → `.agents/skills`,
  External → `<root>/skills`.
- Refactored `skill.install`, `skill.upgrade`, and `skill.remove` to resolve
  an `AgentRegisteredWorkspace` once and derive the managed skill directory
  through the shared helper instead of hardcoding `<agent-root>/skills`.
- Refactored `skill.publish`, `skill.unpublish`, `skill.tree`,
  `skill.read_file`, and `skill.write_file` to use the shared helper and
  removed the legacy Claude `<root>/skills/<name>` candidate fallback.
- Refactored `skill.list` to consume the shared helper instead of maintaining
  its own `managed_skill_dir_for_layout` mapping.
- Added focused tests proving Codex managed skills now land/read from
  `.agents/skills` and never from the retired root-level managed directory.
- Updated architecture/SPEC gates to enforce the new workspace+layout helper
  boundary and added a SPEC v2 negative fixture for the retired Codex
  `<root>/skills` projection.
- Verification passed:
  - `cargo check -q`
  - `cargo test -q daemon::resources::skills::store::tests`
  - `cargo test -q daemon::ability::builtins::resources::skills::list::tests`
  - `cargo test -q daemon::ability::builtins::resources::skills::publish::tests`
  - `cargo test -q daemon::ability::builtins::resources::skills::install::tests`
  - `cargo fmt --check`
  - `tools/scripts/check-canonical-runtime-convergence-v2.sh`
  - `tools/scripts/check-canonical-runtime-convergence-v2.sh --self-test`
  - `tools/scripts/check-architecture-convergence.sh`
  - `git diff --check`

## Iteration 16 candidate policy

- Focus on files resource store persistence authority. codegraph found
  `files_store::handle_list` as the canonical list handler; rg found that
  `files.put` and `files.list` currently call `state::ensure_root(root).ok()`,
  while `files.list` also wraps `read_dir(root)` in `if let Ok(...)` and
  flattens directory entry errors.
- The root abstraction problem is a hidden persistence fallback inside a
  resource handler. A missing/unusable files store root is an operational
  failure of the resource authority, not proof that the resource inventory is
  empty and not a condition that should let writes continue into a partially
  resolved path.
- The intended cutover is fail-closed files store admission at the storage
  boundary: root creation, directory enumeration, and directory-entry reads
  return typed handler errors. The handler may still ignore non-blob filenames,
  but it must not suppress persistence errors.
- The naming cleanup is to replace the misleading
  `ensure_metadata_compatible` helper with
  `ensure_existing_blob_metadata_matches`, because the rule enforces immutable
  producer metadata for an existing content-addressed blob rather than a
  compatibility layer.
- Verification must prove `files.put` and `files.list` reject a non-directory
  store root, and SPEC v2 must reject reintroducing the silent
  `ensure_root(...).ok()`, `if let Ok(read_dir)`, or `flatten()` fallback in the
  files store handler.

## Iteration 16 decision log

- Refactored `files.put` to fail closed when the files store root cannot be
  created or is not a directory, instead of suppressing the root authority error
  and continuing toward blob path resolution.
- Refactored `files.list` to fail closed on store root creation, directory
  enumeration, and directory-entry read errors. Non-blob filenames remain
  ignored by schema, but persistence failures are no longer converted into an
  empty inventory.
- Renamed `ensure_metadata_compatible` to
  `ensure_existing_blob_metadata_matches`, making the helper describe the
  immutable metadata invariant instead of implying a compatibility layer.
- Added focused tests proving `files.put` and `files.list` reject a
  non-directory store root.
- Added SPEC v2 gate `check_files_store_persistence_cutover_contract` and a
  negative self-test fixture that fails if silent `ensure_root(...).ok()`,
  `if let Ok(read_dir)`, `flatten()`, or the retired helper name return.
- Verification passed:
  - `cargo test -q daemon::ability::builtins::resources::files_store::handlers::tests`
  - `cargo check -q`
  - `cargo fmt --check`
  - `tools/scripts/check-canonical-runtime-convergence-v2.sh`
  - `tools/scripts/check-canonical-runtime-convergence-v2.sh --self-test`
  - `tools/scripts/check-architecture-convergence.sh`
  - `git diff --check`

## Iteration 17 candidate policy

- Focus on Mission dispatch audit authority. codegraph/rg found
  `send_to_agent_with_depth_and_progress` still converts
  `RunDir::create(root)` failure into `None` and continues driver execution
  under the explicit `fallback = "no_per_run_persistence"` event.
- The same dispatch path also treats failed Timeline `admitted` and terminal
  emits as warnings and labels the run directory as authoritative
  (`run_dir_write_is_authoritative`, `run_dir_meta_is_authoritative`).
- The root abstraction problem is a second audit authority path in the local
  Agent/Mission dispatch lifecycle. A runtime call that cannot establish its
  run directory and Timeline admission record is not a valid invocation; it is
  an infrastructure failure before runtime execution.
- The intended cutover is:
  - run directory creation is required before adapter invocation;
  - prompt persistence is required before adapter invocation;
  - Timeline `admitted` is required before adapter invocation;
  - Timeline terminal emission is required before returning a successful
    dispatch response;
  - `AgentResponse.run_dir` remains optional on the public DTO for source
    compatibility, but successful local dispatch always projects `Some(path)`.
- Verification must prove a blocked `runs` store fails before the bogus test
  adapter command is spawned, and SPEC v2 must reject the retired fallback
  vocabulary and optional `run_dir` branch in the dispatch implementation.

## Iteration 17 decision log

- Removed Mission/Agent dispatch degraded mode where `RunDir::create(root)`
  failure produced `None` and the adapter still ran without per-run
  persistence.
- Made prompt persistence a pre-invocation requirement. A dispatch whose run
  directory cannot faithfully record the prompt fails before spawning the
  runtime adapter.
- Made Timeline `admitted` emission a pre-invocation requirement and Timeline
  terminal emission a response requirement. The run directory no longer acts as
  an alternate authority when Timeline writes fail.
- Preserved the public `AgentResponse.run_dir: Option<PathBuf>` shape while
  making successful local dispatch project `Some(path)`.
- Updated `RunDir::write_prompt` documentation to match the new dispatcher
  contract.
- Added focused dispatch coverage proving a blocked `runs/` store fails before
  the bogus test adapter command can spawn.
- Added SPEC v2 gate `check_mission_dispatch_audit_authority_contract` and a
  negative self-test fixture that fails if `no_per_run_persistence`,
  `run_dir_write_is_authoritative`, `run_dir_meta_is_authoritative`, optional
  run-dir branching, or timeline warning fallbacks return.
- Verification passed:
  - `cargo test -q daemon::execution::mission::dispatch::tests`
  - `cargo check -q`
  - `cargo fmt --check`
  - `tools/scripts/check-canonical-runtime-convergence-v2.sh`
  - `tools/scripts/check-canonical-runtime-convergence-v2.sh --self-test`
  - `tools/scripts/check-architecture-convergence.sh`
  - `git diff --check`

## Iteration 18 candidate policy

- Focus on Mission dispatch terminal artifact persistence. codegraph confirmed
  `RunDir::write_response` and `RunDir::write_meta` are owned by the Mission
  dispatch path; rg found that both failures are still warning-only
  (`response_write_failed`, `meta_write_failed`) after Iteration 17 made run
  directory creation, prompt persistence, and Timeline writes required.
- The root abstraction problem is a half-audited terminal state. A dispatch can
  currently run the adapter and emit a terminal Timeline event even when
  `response.md` or `meta.json` cannot be written, leaving operators with an
  incomplete run directory while public response semantics still look
  successful.
- The intended cutover is to make terminal artifacts part of the dispatch
  terminal decision:
  - successful runtime output requires `response.md` persistence;
  - every runtime outcome requires `meta.json` persistence;
  - terminal artifact persistence failure converts the dispatch terminal event
    to `failed` before any terminal event is emitted;
  - the returned result is not successful when terminal artifacts are
    incomplete;
  - runtime errors and artifact errors are combined instead of one silently
    shadowing the other.
- Verification must prove a response artifact write failure turns an otherwise
  successful adapter output into a failed terminal artifact outcome, and SPEC v2
  must reject reintroducing warning-only response/meta write fallbacks.

## Iteration 18 decision log

- Refactored Mission dispatch terminal artifact handling into
  `persist_terminal_run_artifacts`, keeping response/meta persistence decisions
  out of the main adapter invocation flow.
- Removed warning-only `response_write_failed` and `meta_write_failed` handling
  from the dispatch terminal path. A missing `response.md` or `meta.json`
  projection now participates in the terminal decision.
- Added `terminal_dispatch_error_message` so runtime errors and terminal
  artifact persistence errors are combined rather than one silently shadowing
  the other.
- Changed successful adapter output with incomplete terminal artifacts into a
  failed terminal Timeline event before any terminal event is emitted.
- Added focused coverage proving response artifact write failure turns an
  otherwise successful adapter output into a failed terminal artifact outcome
  and records the artifact failure in `meta.json`.
- Strengthened SPEC v2 `check_mission_dispatch_audit_authority_contract` to
  reject warning-only response/meta write fallbacks and require the terminal
  artifact helper/test.
- Verification passed:
  - `cargo test -q daemon::execution::mission::dispatch::tests`
  - `cargo check -q`
  - `cargo fmt --check`
  - `tools/scripts/check-canonical-runtime-convergence-v2.sh`
  - `tools/scripts/check-canonical-runtime-convergence-v2.sh --self-test`
  - `tools/scripts/check-architecture-convergence.sh`
  - `git diff --check`

## Iteration 19 candidate policy

- Focus on EAL Mission orchestration persistence authority. rg found
  `MissionRunStore::create` warning-only initial meta writes, `MissionRunner`
  best-effort `source.eal` / `ir.json` writes, warning-only `trace.json`
  writes, warning-only terminal `meta.json` writes, and
  `record_terminal` rebuilding a running aggregate when stored meta cannot be
  loaded.
- The root abstraction problem is the same half-audit state that was removed
  from local Agent dispatch, now at the Mission run lifecycle layer. A Mission
  run directory is the canonical lifecycle projection for source, compiled IR,
  trace, terminal status, and heartbeat liveness; it must not silently execute
  or finalize when those projections are missing or unreadable.
- The intended cutover is:
  - initial running `meta.json` persistence is required for run creation;
  - source and IR persistence are required before execution;
  - successful interpreter reports require `trace.json` persistence before
    terminal meta is written;
  - terminal meta persistence is required on both completion and failure paths;
  - terminal recording loads the stored aggregate fail-closed instead of
    reconstructing a running projection.
- Verification must prove terminal recording fails when `meta.json` is missing
  and SPEC v2 must reject the retired best-effort source/IR/trace/meta
  vocabulary and aggregate reconstruction fallback.

## Iteration 19 decision log

- Made initial Mission running `meta.json` persistence required during
  `MissionRunStore::create`; a run without persisted lifecycle state is no
  longer considered created.
- Refactored MissionRunner source/IR persistence to fail closed before
  interpreter execution. If source or IR persistence fails after run creation,
  the runner records a failed terminal meta before returning the error.
- Refactored successful interpreter reports so `trace.json` persistence is
  required before terminal completion is recorded. Trace persistence failure
  becomes a failed Mission terminal outcome.
- Changed `MissionRunDir::record_terminal` to return
  `anyhow::Result<MissionRunMeta>` and to require loading the stored aggregate.
  The retired reconstruction fallback from `transition.running_projection()` was
  removed.
- Removed warning-only terminal meta writes from both completion and failure
  paths. Terminal meta persistence now participates in the function result.
- Replaced stale "best-effort" Mission context wording with deterministic run
  directory projection wording.
- Added focused coverage proving terminal recording fails when stored
  `meta.json` is missing.
- Added SPEC v2 gate
  `check_mission_orchestration_persistence_authority_contract` plus a negative
  self-test fixture rejecting best-effort source/IR/trace/meta writes and the
  aggregate reconstruction fallback.
- Verification passed:
  - `cargo test -q daemon::execution::mission::orchestration::tests`
  - `cargo check -q`
  - `cargo fmt --check`
  - `tools/scripts/check-canonical-runtime-convergence-v2.sh`
  - `tools/scripts/check-canonical-runtime-convergence-v2.sh --self-test`
  - `tools/scripts/check-architecture-convergence.sh`
  - `git diff --check`

## Iteration 20 candidate policy

- Focus on Pages restore route authority. The publish/unpublish paths already
  fail closed on snapshot persistence, but daemon boot restore still calls
  `register_restored_project_abilities` after `restore_published_projects` and
  only logs `restore_project_abilities_failed` when a restored project cannot
  hot-register its fetch/API abilities.
- The root abstraction problem is a split restored-state authority: the
  persisted Pages project map can say a project exists while the daemon-hosted
  LocalRuntime catalog lacks the corresponding project ability routes. This is
  the same class of route/catalog inconsistency that surfaces as "route not
  visible" product failures.
- The intended cutover is:
  - `pages::register` becomes fallible and participates in registry assembly;
  - restore snapshot parse/cleanup failures propagate through registry build;
  - restored project ability registration failures abort assembly instead of
    warning and leaving a partial route catalog;
  - management ability registration remains unchanged; only restored dynamic
    project route replay becomes fail-closed.
- Verification must prove the boot path no longer contains
  `restore_project_abilities_failed` warning-only fallback and SPEC v2 rejects
  reintroducing non-fallible Pages registration.

## Iteration 20 decision log

- Made `pages::register` fallible so Pages restored-state authority participates
  in daemon ability registry assembly instead of being a side-effect-only boot
  hook.
- Changed persisted Pages project restore failures to propagate with
  `restore published Pages projects` context. A daemon no longer starts with an
  unknown persisted Pages map state.
- Changed restored project route replay to return
  `anyhow::Result<usize>`. Each restored project ability registration now
  fails closed with `register restored Pages project {user}/{project_id}`
  context instead of logging `restore_project_abilities_failed` and leaving a
  partial catalog.
- Updated registry assembly to propagate `pages::register` with
  `register Pages reference system` context.
- Updated external Pages unit fixture callers to acknowledge the fallible
  registration contract explicitly.
- Added SPEC v2 gate `check_pages_restore_route_authority_contract` plus a
  negative self-test fixture rejecting warning-only restored route replay and
  non-fallible Pages registration.
- Verification passed:
  - `cargo test -q daemon::ability::builtins::resources::pages`
  - `cargo test -q daemon::ability::catalog::assembly_tests::pages_management_is_user_owned_and_runs_on_the_declared_pages_agent`
  - `cargo check -q`
  - `cargo fmt --check`
  - `tools/scripts/check-canonical-runtime-convergence-v2.sh --self-test`
  - `tools/scripts/check-canonical-runtime-convergence-v2.sh`
  - `tools/scripts/check-architecture-convergence.sh`
  - `git diff --check`

## Iteration 21 candidate policy

- Focus on join-time runtime authority wiring. codegraph/rg found that
  `run_join_stages` still classified `daemon-config`, `federated-peers`, and
  `realm-trust` as skipped best-effort stages even though those files feed the
  daemon's runtime route, descriptor, and admission read models.
- The root abstraction problem is a product lifecycle split: join can persist
  credentials and then continue after local runtime authority wiring fails,
  leaving the later daemon start/invoke path to surface unrelated
  `AUTHORITY_DENIED`, descriptor, or route visibility errors.
- The intended cutover is:
  - join treats daemon-config, federated-peers, and realm-trust wiring as
    required stages;
  - stage failure is rendered as failed and returned immediately;
  - no local hub-mode daemon-config remains a valid no-op, but write/reload
    failure after a config exists is not hidden;
  - SIGHUP reload after authority file updates is a required runtime refresh
    when a pidfile exists, not a warning-only compatibility side effect.
- Verification must prove the retired join best-effort vocabulary cannot return
  around required authority wiring and that existing federation wire unit tests
  still pass.

## Iteration 21 decision log

- Refactored join-time local runtime authority wiring into
  `run_required_join_stage`, which renders required local transitions as failed
  and returns immediately instead of continuing with skipped authority state.
- Made `daemon-config`, `federated-peers`, and `realm-trust` required join
  stages. The join flow no longer persists credentials and then hides local
  runtime wiring failures that later surface as descriptor, route, or admission
  errors.
- Renamed the SIGHUP helper from `sighup_running_daemon_best_effort` to
  `reload_running_daemon_after_join`. Missing pidfile remains a no-op because
  there is no running read model to refresh; stale/unreachable pidfiles now
  fail the authority wiring transition.
- Removed warning-only "activate on next daemon restart" branches after
  daemon-config and realm-trust updates. Runtime refresh is now part of the
  join authority transition when a running daemon is present.
- Added SPEC v2 gate `check_join_authority_wiring_required_contract` plus a
  negative self-test fixture rejecting required-stage `stage_skipped` and
  SIGHUP best-effort vocabulary.
- Verification passed:
  - `cargo test -q cli::commands::federation_wire`
  - `cargo test -q cli::commands::join`
  - `cargo check -q`
  - `cargo fmt --check`
  - `tools/scripts/check-canonical-runtime-convergence-v2.sh --self-test`
  - `tools/scripts/check-canonical-runtime-convergence-v2.sh`
  - `tools/scripts/check-architecture-convergence.sh`
  - `git diff --check`

## Iteration 22 candidate policy

- Focus on remote caller-signer failure projection. codegraph/rg found that
  `remote_failure.rs` still classified legacy typed details containing
  `self-identity:` / `keyring entry not found` and the unit test required those
  keyring details to remain in the returned gRPC message.
- The root abstraction problem is an error-boundary leak: remote descriptor
  resolve / invoke should expose the canonical runtime state
  `CALLER_SIGNER_UNAVAILABLE`, not the daemon key-service storage failure that
  happened to prove signer custody is absent.
- The intended cutover is:
  - keep recognizing old raw details for classification so no security
    downgrade occurs;
  - sanitize caller-signer-unavailable details before constructing transport
    status;
  - preserve caller URA when it is present because it identifies the missing
    canonical runtime principal;
  - reject any future production path that emits `self-identity:` or keyring
    storage vocabulary through remote failure status.
- Verification must prove the remote failure status maps to
  `PermissionDenied`, contains `CALLER_SIGNER_UNAVAILABLE`, and omits keyring
  implementation details.

## Iteration 22 decision log

- Added `canonical_remote_failure_detail` as the single projection boundary
  before remote failure details become gRPC transport status messages.
- Added caller-signer-unavailable canonicalization that preserves the missing
  caller URA when present, while replacing `self-identity:` / keyring storage
  detail with `CALLER_SIGNER_UNAVAILABLE`.
- Kept legacy raw-detail recognition in `is_caller_signer_unavailable_message`
  so old upstream failure text is still classified as `PermissionDenied` rather
  than downgraded to `NotFound`.
- Updated the remote failure test to require the canonical code and to reject
  `keyring entry not found`, `keyring rejected request`, and `self-identity:`
  in the returned status message.
- Added SPEC v2 gate
  `check_remote_failure_caller_signer_projection_contract` plus a negative
  self-test fixture rejecting unsanitized caller-signer failure projection.
- Verification passed:
  - `cargo test -q daemon::invocation::dispatch::remote_failure`
  - `cargo check -q`
  - `cargo fmt --check`
  - `tools/scripts/check-canonical-runtime-convergence-v2.sh --self-test`
  - `tools/scripts/check-canonical-runtime-convergence-v2.sh`
  - `tools/scripts/check-architecture-convergence.sh`
  - `git diff --check`

## Iteration 23 candidate policy

- Focus on pluginexec sidecar invocation tuple naming. codegraph found the daemon
  sidecar frame and all language provider helpers still expose `caller`,
  `callee`, `subject`, and `ability` even though the canonical runtime model and
  SDK public APIs use `caller_ura`, `callee_ura`, `subject_ura`, and
  `ability_ura`.
- The root abstraction problem is a provider sidecar schema that teaches plugin
  authors a non-canonical tuple vocabulary at the exact boundary where product
  code first handles daemon-admitted invocations.
- The intended cutover is:
  - daemon sidecar frames serialize canonical `*_ura` tuple fields;
  - provider helpers in Go, Python, Node, Java, and Rust expose matching
    canonical field/property names;
  - old sidecar identity aliases are rejected by strict frame decoding rather
    than accepted as compatibility fields;
  - SPEC v2 rejects reintroducing bare `caller/callee/subject/ability` sidecar
    tuple fields in production provider helpers.
- Verification must prove daemon sidecar tests and provider helper tests pass,
  and SPEC v2 must reject legacy sidecar tuple vocabulary.

## Iteration 23 decision log

- codegraph confirmed `SidecarInvocationEnvelope` is the daemon-owned sidecar
  frame consumed by RPC, stream, and bidi sidecar hosts, and that Go, Python,
  Rust, Java, and Node provider helpers are the public plugin-author projection
  of the same frame.
- The cutover renamed the daemon frame fields from `caller`, `callee`,
  `ability`, and `subject` to `caller_ura`, `callee_ura`, `ability_ura`, and
  `subject_ura`.
- The daemon projection now uses the already-admitted `EnvelopeContext` for
  caller/callee/subject and the selected sidecar ability URA parameter for
  `ability_ura`; this removes the prior unused ability parameter and prevents
  provider-local ability reinterpretation.
- The MCP exec invocation observation context was migrated to the same
  canonical `*_ura` tuple vocabulary so sidecar-like provider execution does not
  retain a second tuple schema.
- Go, Python, Rust, Java, and Node provider helpers now expose canonical tuple
  names. Language facades keep idiomatic casing where appropriate
  (`CallerURA` / `callerURA`) while the sidecar wire schema remains
  snake-case `*_ura`.
- Provider helpers now reject retired tuple aliases even when canonical fields
  are present, preventing a hidden compatibility path where old
  `caller/callee/ability/subject` keys are silently ignored.
- SPEC v2 `check_plugin_sidecar_helper_matrix_contract` now requires canonical
  daemon/provider tuple fields and alias rejection, and includes a negative
  fixture for legacy sidecar tuple vocabulary.
- Verification passed:
  - `bash -n tools/scripts/check-canonical-runtime-convergence-v2.sh`
  - `cargo fmt --check`
  - `cargo test -q daemon::plugins::sidecar`
  - `cargo test -q --manifest-path sdk/rust/provider/easynet/pluginexec/Cargo.toml`
  - `(cd sdk/go && go test ./provider/easynet/pluginexec)`
  - `PYTHONPATH=/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Cli/sdk/python:/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Axon/sdk/python python3 -m unittest sdk/python/tests/test_plugin_exec.py`
  - `node --test sdk/node/test/pluginexec.test.mjs`
  - `(cd sdk/java && mvn -q -Dtest=run.runtime.sdk.provider.easynet.pluginexec.SidecarRuntimeTest test)`
  - an equivalent positive/negative tuple audit for the new SPEC v2 rule.
- Full `tools/scripts/check-canonical-runtime-convergence-v2.sh` and
  `--self-test` were attempted in this environment but were killed with exit
  `137` before emitting a SPEC failure. This is not counted as passed; the next
  iteration must still re-run the full gate in an environment that can complete
  it.

## Iteration 24 candidate policy

- Focus on namespace proxy resolver ingress. codegraph/rg found that
  `NamespaceProxyResolveRequest` still defaulted core resolver tuple fields
  (`query_name`, `qtype`, `caller_ura`, `subject_ura`, and `realm_hint`) to empty
  strings.
- The root abstraction problem is public ingress tuple defaulting: a malformed
  product request can pass JSON decoding and then fail later as empty namespace
  answers, peer fanout errors, or route-negative state. That preserves a
  compatibility surface instead of making resolver tuple construction explicit.
- The intended cutover is:
  - keep `peer_hub_urls` optional because an empty peer set is a real
    non-dispatchable answer state;
  - keep `ability_name` optional because it is an owner-local filter;
  - require `query_name`, `qtype`, `caller_ura`, `subject_ura`, and `realm_hint`
    at request decode/validation;
  - validate caller/subject as canonical URAs before proxy fanout;
  - update the daemon Invocation descriptor schema so product clients generate
    the real proxy request shape, not the older `target_ura` / `peers` shape.

## Iteration 24 decision log

- Removed `serde(default)` from the namespace proxy resolver tuple fields:
  `query_name`, `qtype`, `caller_ura`, `subject_ura`, and `realm_hint`.
- Added explicit dispatcher validation for non-empty `query_name` and
  `realm_hint`, canonical `ResolveType`, non-unspecified qtype, and canonical
  URA parsing for `caller_ura` and `subject_ura`.
- Split the daemon Invocation descriptor schema for `namespace.resolve` and
  `namespace.proxy_resolve`. The proxy schema now exposes
  `peer_hub_urls`, `query_name`, `qtype`, `caller_ura`, `subject_ura`,
  `realm_hint`, and optional `ability_name`, with the resolver tuple fields
  required.
- Added daemon unary tests proving missing tuple fields and non-canonical
  caller/subject URAs fail before proxy routing.
- Added catalog tests proving the proxy descriptor schema requires the explicit
  resolver tuple and no longer advertises retired `target_ura` / `peers` fields.
- Added SPEC v2 gate
  `check_namespace_proxy_resolve_exact_tuple_ingress_contract` with a negative
  self-test fixture rejecting reintroduced tuple defaulting.
- Verification passed:
  - `bash -n tools/scripts/check-canonical-runtime-convergence-v2.sh`
  - `cargo fmt --check`
  - `cargo test -q daemon::ability::catalog::daemon_invocation_contracts`
  - `cargo test -q namespace_proxy_resolve`
  - `cargo check -q`
  - `tools/scripts/check-architecture-convergence.sh`
  - `tools/scripts/check-canonical-runtime-convergence-v2.sh`
  - `git diff --check`

## Iteration 25 candidate policy

- Focus on federation user-device listing descriptor drift. codegraph/rg found
  that the actual runtime tuples are:
  - `federation.list_user_devices`: peer-hub internal `{ realm }`;
  - `federation.proxy_list_user_devices`: daemon proxy `{ realm, peer_hub_urls }`.
- The descriptor contract still advertised a shared older product projection
  with `user_ura`, optional `realm`, and `peers`. That lets product clients build
  requests from the wrong catalogue shape even though the dispatcher already
  rejects unknown fields.
- The root abstraction problem is descriptor ownership drift: the canonical
  ability catalogue was describing an old product directory model rather than
  the exact daemon Invocation tuple accepted by the runtime route.
- The intended cutover is:
  - split peer and proxy list-user-devices descriptor schemas;
  - require `realm` in both schemas;
  - expose only `peer_hub_urls` as the proxy fanout selector;
  - remove retired `user_ura`, `peers`, and `tenant_id` vocabulary from the
    active request/schema contract;
  - make SPEC v2 reject future schema/request drift.

## Iteration 25 decision log

- Split `daemon_invocation_contracts::input_schema_for` so
  `federation.list_user_devices` and `federation.proxy_list_user_devices` no
  longer share one descriptor schema.
- `federation.list_user_devices` now publishes the exact peer-hub tuple:
  required `realm`, closed schema, no product user filter fields.
- `federation.proxy_list_user_devices` now publishes the exact daemon proxy
  tuple: required `realm`, optional `peer_hub_urls`, closed schema, no retired
  `peers` alias.
- Tightened regression tests:
  - catalog schema tests verify both schemas match dispatcher tuples;
  - dispatcher tests verify proxy requests missing `realm` fail instead of
    becoming empty successful directory results;
  - request DTO tests verify retired product directory fields are rejected.
- Added SPEC v2 gate
  `check_federation_list_user_devices_exact_tuple_ingress_contract` covering
  DTOs, dispatcher guards, descriptor schemas, and regression test presence.
- Verification passed:
  - `cargo fmt --check`
  - `cargo test -q daemon::ability::catalog::daemon_invocation_contracts`
  - `cargo test -q list_user_devices_requests_reject_retired_product_directory_fields`
  - `cargo test -q invoke_federation_proxy_list_user_devices_rejects_missing_required_realm`
  - `tools/scripts/check-canonical-runtime-convergence-v2.sh`
  - `tools/scripts/check-architecture-convergence.sh`
  - `git diff --check`

## Iteration 26 candidate policy

- Focus on federation directory stream vocabulary. codegraph/rg confirmed the
  active route is `federation.subscribe_directory_v2`; the retired
  `federation.subscribe_directory` v1 descriptor and DTOs are already gated
  out.
- Active source comments still used bare `subscribe_directory` and one stale
  comment claimed a legacy v1 directory-stream projection still existed. That
  does not change runtime behavior, but it preserves the wrong architecture
  story in the files that own the presence/read-model FSM.
- The root abstraction problem is boundary documentation drift: active runtime
  code was still naming a retired stream lifecycle, making future patches more
  likely to reintroduce a second directory stream model.
- The intended cutover is:
  - active source may refer to `subscribe_directory_v2` only;
  - no bare `subscribe_directory` v1 lifecycle vocabulary remains in runtime
    source, tests, or active federation descriptors;
  - SPEC v2 rejects the retired bare stream name, not only the fully-qualified
    `federation.subscribe_directory` ability.

## Iteration 26 decision log

- Replaced active source comments in federation directory, presence, dispatch
  deps, daemon invocation service, and federation wrappers so they name
  `subscribe_directory_v2` explicitly.
- Removed the stale comment claiming a legacy v1 directory-stream projection
  remained active in `federation_wrappers`.
- Tightened SPEC v2
  `check_retired_federation_directory_v1_stream_contract` to reject bare
  `subscribe_directory` when it is not suffixed with `_v2`.
- Verification passed:
  - `rg --pcre2 -n '\\bsubscribe_directory\\b(?!_v2)' src tests ability-descriptors/system/federation -S`
  - `cargo fmt --check`
  - `bash -n tools/scripts/check-canonical-runtime-convergence-v2.sh`
  - `tools/scripts/check-canonical-runtime-convergence-v2.sh`
  - `tools/scripts/check-architecture-convergence.sh`
  - `git diff --check`

## Iteration 27 candidate policy

- Focus on `federation.advertise_agent` acknowledgement and receipt shape.
  codegraph found `AdvertiseAgentResponse` has a narrow impact radius in the
  daemon wrapper/test path, while `AdvertiseAgentReceipt` is owned by the
  federation client parser. The old field `replaced_prior` was explicitly
  documented as an always-false wire-compat field in the advertise response.
- The root abstraction problem is lifecycle vocabulary leaking into an ACK
  contract that does not own lifecycle replacement. Real replacement semantics
  belong to agent lifecycle and directory stream events; an advertise ACK should
  only acknowledge admission/commit success.
- The intended cutover is:
  - keep directory and hosted-agent lifecycle `replaced_prior` semantics where
    they describe real state transitions;
  - remove `replaced_prior` from `AdvertiseAgentResponse`;
  - remove `replaced_prior` from `AdvertiseAgentReceipt`;
  - make the receipt parser reject retired `replaced_prior` instead of silently
    accepting it;
  - make SPEC v2 reject reintroduced advertise response/receipt compat fields.

## Iteration 27 decision log

- Removed the always-false `replaced_prior` field from
  `AdvertiseAgentResponse` and from the daemon dispatcher test expectation.
- Removed `replaced_prior` from `AdvertiseAgentReceipt` and renamed/extended
  the retired-field negative test so receipt parsing rejects the old field.
- Preserved real replacement state in agent lifecycle and directory event code;
  those paths continue to model actual prior-row/replacement transitions rather
  than advertise ACK compatibility.
- Tightened SPEC v2 `check_advertise_agent_ingress_contract` so it now covers
  both daemon response and federation client receipt shapes, and includes a
  negative self-test fixture for reintroduced `replaced_prior`.
- Verification passed:
  - `/Users/macbook.silan.tech/.local/bin/codegraph sync .`
  - `/Users/macbook.silan.tech/.local/bin/codegraph impact AdvertiseAgentResponse`
  - `/Users/macbook.silan.tech/.local/bin/codegraph impact AdvertiseAgentReceipt`
  - `cargo fmt --check`
  - `cargo test -q advertise_agent`
  - `bash -n tools/scripts/check-canonical-runtime-convergence-v2.sh`
  - `tools/scripts/check-canonical-runtime-convergence-v2.sh --self-test`
  - `tools/scripts/check-canonical-runtime-convergence-v2.sh`
  - `tools/scripts/check-architecture-convergence.sh`
  - `git diff --check`

## Iteration 28 candidate policy

- Focus on daemon credentials identity projection. codegraph/rg found
  `StoredDeviceIdentity` still deserialized old `agent_ura` credentials and
  accepted them when they matched the canonical `realm + node_id` device URA.
- The root abstraction problem is credentials read-model compatibility leaking
  into daemon identity selection. `agent_ura` was no longer a fallback, but it
  still acted as an accepted checksum field, keeping a retired identity fact
  alive in the boot path.
- The intended cutover is:
  - derive daemon device caller URA only from canonical `realm` and `node_id`;
  - reject credentials carrying retired `agent_ura` at parse time, just like
    retired `tenant_id`;
  - tolerate unrelated modern credentials fields so the projection remains a
    narrow read model, not a full file-schema owner;
  - make SPEC v2 reject any return of `agent_ura` checksum/fallback parsing.

## Iteration 28 decision log

- Replaced `StoredDeviceIdentity.agent_ura: Option<String>` with a typed
  `RejectedAgentUra` sentinel field renamed to `agent_ura`.
- Removed `agent_ura` comparison from
  `canonical_caller_ura_from_stored_identity`; the canonical device identity
  now derives only from `realm` and `node_id`.
- Replaced compatibility tests that accepted/mismatched `agent_ura` with a
  parse-level rejection test for credentials carrying retired `agent_ura`.
- Added SPEC v2 `check_daemon_credentials_identity_contract` plus a negative
  self-test fixture for the old checksum/compatibility implementation.
- Verification passed:
  - `/Users/macbook.silan.tech/.local/bin/codegraph explore daemon_identity_from_stored agent_ura tenant_id`
  - `cargo fmt --check`
  - `cargo test -q daemon_identity`
  - `bash -n tools/scripts/check-canonical-runtime-convergence-v2.sh`
  - `tools/scripts/check-canonical-runtime-convergence-v2.sh --self-test`
  - `tools/scripts/check-canonical-runtime-convergence-v2.sh`
  - `tools/scripts/check-architecture-convergence.sh`
  - `git diff --check`

## Iteration 29 candidate policy

- Focus on cutover-ready descriptor ingress. Once a runtime ability is marked
  `cutover_ready`, its public schema must describe only canonical runtime facts;
  product pairing or old bootstrap carriers must stay outside the ability tuple.
- codegraph and targeted descriptor inspection found `federation.join` still
  publishing `pairing_secret` as an optional input with explicit "legacy join
  flows" wording, while the production `JoinArgs` DTO already emits only
  `realm`, `membership_ura`, `public_key_hex`, and optional
  `principal_enrollment`.
- The root abstraction problem is schema authority drift: products and SDKs read
  the descriptor/catalog as the public runtime contract, so a legacy descriptor
  field remains active even when the Rust client no longer sends it.
- The selected cutover is to remove `pairing_secret` from the canonical
  descriptor and add SPEC v2 coverage that keeps descriptor, generated catalog
  schema, and client DTO aligned.

## Iteration 29 decision log

- Product pairing tokens remain valid only in the CLI preflight path that
  reserves/validates a pairing session before runtime join. They are not a
  canonical `federation.join` ability argument.
- `federation.join` now exposes a product-neutral runtime request:
  `realm`, `membership_ura`, `public_key_hex`, and optional
  `principal_enrollment`.
- SPEC v2 now rejects any reintroduction of `pairing_secret`, generic `token`,
  or "legacy join flows" wording in the cutover-ready join descriptor/catalog
  contract, and includes a negative self-test fixture.
- Verification passed:
  - `/Users/macbook.silan.tech/.local/bin/codegraph explore "federation.join token legacy join pairing secret descriptor"`
  - `cargo test -q join_args_does_not_emit_retired_pairing_secret`
  - `cargo fmt --check`
  - `bash -n tools/scripts/check-canonical-runtime-convergence-v2.sh`
  - `tools/scripts/check-canonical-runtime-convergence-v2.sh --self-test`
  - `tools/scripts/check-canonical-runtime-convergence-v2.sh`
  - `tools/scripts/check-architecture-convergence.sh`
  - `git diff --check`

## Iteration 30 candidate policy

- Focus on runtime-admin bootstrap ingress because product/device start depends
  on this path to establish caller-key custody before steady-state signed
  invocation.
- codegraph and targeted search found
  `runtime.bootstrap_self_identity` marked `cutover_ready` while its descriptor
  required `tenant_id` and described it as a compatibility field carrying realm.
  The handler also accepted an undocumented `display_name` field and lacked
  `deny_unknown_fields`.
- The root abstraction problem is a canonical runtime-admin ability whose public
  schema and private DTO still model realm as a tenant alias. That makes SDK and
  product callers infer an obsolete bootstrap shape from the canonical catalog.
- The selected cutover is to make bootstrap identity ingress strict and
  realm-named: descriptor, DTO, state naming, bootstrap alias construction, and
  dispatcher tests all use `realm`; retired `tenant_id` and `display_name`
  fields are rejected.

## Iteration 30 decision log

- `runtime.bootstrap_self_identity` now requires `realm`, `node_id`,
  `owner_id`, and `public_key_b64`.
- The Axon runtime-admin bridge uses `#[serde(deny_unknown_fields)]` for
  `BootstrapSelfIdentityArgs`, removes the undocumented `display_name` carrier,
  and renames `tenant_by_node` to `realm_by_node`.
- Runtime-admin bootstrap diagnostics now report
  `node_id_already_bootstrapped_for_realm` instead of tenant vocabulary.
- SPEC v2 now rejects reintroduced bootstrap `tenant_id`, `display_name`, and
  compatibility wording across descriptor, handler DTO, and dispatcher payloads,
  with a negative self-test fixture.
- Verification passed:
  - `/Users/macbook.silan.tech/.local/bin/codegraph explore "runtime.bootstrap_self_identity compatibility field realm descriptor bootstrap self identity"`
  - `cargo test -q bootstrap_args_reject_retired_tenant_id_alias_and_display_name`
  - `cargo test -q invoke_runtime_bootstrap_self_identity`
  - `cargo fmt --check`
  - `bash -n tools/scripts/check-canonical-runtime-convergence-v2.sh`
  - `tools/scripts/check-canonical-runtime-convergence-v2.sh --self-test`
  - `tools/scripts/check-canonical-runtime-convergence-v2.sh`
  - `tools/scripts/check-architecture-convergence.sh`
  - `git diff --check`

## Iteration 31 candidate policy

- Focus on provider sidecar helper exactness because plugin templates are the
  product-facing bridge into the canonical runtime model. A helper that silently
  ignores extra invocation fields becomes a compatibility layer even when the
  template calls the SDK/provider package instead of hand-writing JSON.
- codegraph found the same sidecar invocation abstraction implemented in
  Go/Python/Node/Java/Rust with retired tuple alias rejection, but without a
  shared fail-closed rule for unknown `invocation` fields.
- The root abstraction problem is schema authority drift at the provider helper
  boundary: the daemon sidecar frame is the canonical contract, while helper
  parsers were still open-ended maps/struct decoders.
- The selected cutover is to make the provider sidecar invocation parser exact
  across all supported helper languages. Retired aliases still produce retired
  field diagnostics; any other non-canonical invocation key now fails before the
  plugin handler runs.

## Iteration 31 decision log

- Canonical sidecar invocation fields are now exactly:
  `caller_ura`, `callee_ura`, `ability_ura`, `subject_ura`,
  `invocation_nonce`, `causal_context`, and `args`.
- Go, Python, Node, Java, and Rust provider helpers now reject unknown
  `invocation` fields before projecting handler-facing `SidecarInvocation`.
- Cross-language tests now include an unknown-field negative vector using
  `descriptor_ref` as a representative legacy/provider leak.
- SPEC v2 now gates both production helper tokens and test evidence for unknown
  sidecar invocation field rejection, and its self-test proves both retired
  alias and unknown-field regressions are caught.
- Verification passed:
  - `/Users/macbook.silan.tech/.local/bin/codegraph explore "SidecarInvocation rejectLegacyTupleAliases pluginexec invocation unknown fields"`
  - `cd sdk/go && go test ./provider/easynet/pluginexec`
  - `PYTHONPATH=/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Axon/sdk/python:/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Cli/sdk/python python3 sdk/python/tests/test_plugin_exec.py`
  - `node --test sdk/node/test/pluginexec.test.mjs`
  - `cargo test -q --manifest-path sdk/rust/provider/easynet/pluginexec/Cargo.toml`
  - `mvn -q -f sdk/java/pom.xml test -Dtest=run.runtime.sdk.provider.easynet.pluginexec.SidecarRuntimeTest`
  - `bash -n tools/scripts/check-canonical-runtime-convergence-v2.sh`
  - `tools/scripts/check-canonical-runtime-convergence-v2.sh --self-test`
  - `tools/scripts/check-canonical-runtime-convergence-v2.sh`
  - `tools/scripts/check-architecture-convergence.sh`
  - `cargo fmt --check`
  - `git diff --check`

## Iteration 38 candidate policy

- Converge the Python SDK to the same receipt proof-fact exactness now enforced
  in Java and Go. Python `RuntimeReceipt` kept the raw receipt object, but
  nested proof/binding structures were still accepted as wide mappings before
  canonical proof facts were built.
- codegraph confirmed the same split schema authority in Python receipt
  validation: required canonical facts were checked, while unknown legacy
  metadata in authority bindings, authority proofs, issuer refs, signatures,
  causal bindings, and receipt refs could still survive the raw JSON boundary.
- The root abstraction problem is identical across SDKs: proof-fact
  canonicalization cannot be the first authority boundary if raw proof objects
  are not exact.
- The selected cutover is to validate Python receipt proof-fact object shapes
  before constructing canonical Axon proof facts, preserving public SDK
  behavior while making the internal receipt model fail closed.

## Iteration 38 decision log

- Python `RuntimeReceipt.validate_proof_facts` now calls
  `_validate_runtime_receipt_raw_proof_shape` before canonical proof-fact
  construction.
- Python receipt validation now rejects unknown fields in authority bindings,
  authority proofs, issuer/entity refs, signatures, causal bindings, and receipt
  refs.
- `authority_proof.proof_payload_base64` is now explicit in Python receipt JSON:
  it may be empty for binding-hash proofs, but it may not be omitted.
- Added Python negative tests for legacy authority binding metadata, legacy
  authority proof metadata, missing proof payload fields, and legacy issuer
  profile metadata.
- SPEC v2 now gates Python receipt proof-fact exactness with a negative
  self-test that removes the exact-shape validator, and the session authority
  facade gate now expects exact-schema rejection for retired Python session
  fields.
- Rebuilt `sdk/conformance/canonical-public-api.json` and
  `sdk/conformance/sdk-parity-matrix.json` after Python SDK source changes.
- Verification passed:
  - `/Users/macbook.silan.tech/.local/bin/codegraph explore "Go Python runtime receipt authority_binding authority_proof proof facts unknown fields legacy metadata canonicalizer"`
  - `PYTHONPATH=/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Axon/sdk/python:/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Cli/sdk/python python3 -m pytest -q sdk/python/tests/test_runtime.py`
  - `PYTHONPATH=/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Axon/sdk/python:/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Cli/sdk/python python3 -m pytest -q sdk/python/tests`
  - `python3 sdk/conformance/rebuild_public_api_model.py --write`
  - `bash -n tools/scripts/check-canonical-runtime-convergence-v2.sh`
  - `tools/scripts/check-canonical-runtime-convergence-v2.sh --self-test`
  - `tools/scripts/check-canonical-runtime-convergence-v2.sh`
  - `tools/scripts/check-sdk-canonical-public-api.sh`
  - `tools/scripts/check-architecture-convergence.sh`
  - `cargo fmt --check`
  - `git diff --check`

## Iteration 41 candidate policy

- Continue RF-4/RF-8 convergence by ensuring signed invocation submission keeps
  the complete tuple at the public SDK-to-runtime boundary.
- codegraph showed Go, Node, Java, and Swift signed envelopes include
  `prepared.tuple`, but Python `SignedInvocation.to_json_dict()` only emitted
  prepared metadata and canonical bytes.
- The root abstraction problem is split tuple authority: if a signed
  submission omits the complete Invocation tuple, the downstream runtime or
  daemon transport must recover caller/callee/subject/descriptor facts from
  adjacent state, reintroducing the same class of subject/descriptor mismatch
  seen in product invocation history failures.
- The selected cutover is to make Python signed submission serialize the same
  canonical prepared tuple as the other language SDKs, without changing the
  public object model.

## Iteration 41 decision log

- Python `SignedInvocation.to_json_dict()` now includes
  `prepared.tuple.to_json_dict()` under the signed `prepared` envelope.
- Python runtime tests now assert that `RuntimeClient.submit_signed()` forwards
  signed envelopes with both caller and descriptor facts inside
  `prepared.tuple`.
- Added SPEC v2
  `check_python_sdk_signed_submission_complete_tuple_contract`, including a
  negative self-test fixture that rejects the retired Python signed submission
  shape without `prepared.tuple`.
- Rebuilt `sdk/conformance/canonical-public-api.json` and
  `sdk/conformance/sdk-parity-matrix.json` after the Python SDK signed
  envelope source change.
- Verification passed:
  - `/Users/macbook.silan.tech/.local/bin/codegraph explore "SignedInvocation to_json toObject prepared tuple canonical_hash descriptor_ref prepared_id cross-language SDK Go Python Node Java Swift"`
  - `PYTHONPATH=/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Cli/sdk/python:/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Axon/sdk/python python3 -m pytest sdk/python/tests/test_runtime.py sdk/python/tests/test_signing.py`
  - `python3 sdk/conformance/rebuild_public_api_model.py --write`
  - `bash -n tools/scripts/check-canonical-runtime-convergence-v2.sh`
  - `tools/scripts/check-canonical-runtime-convergence-v2.sh --self-test`
  - `tools/scripts/check-canonical-runtime-convergence-v2.sh`
  - `tools/scripts/check-sdk-canonical-public-api.sh`
  - `tools/scripts/check-architecture-convergence.sh`
  - `cargo fmt --check`
  - `git diff --check`

## Iteration 42 candidate policy

- Continue RF-5/RF-4 convergence by preserving provider-issued signer custody
  policy through Java prepare/sign/submit.
- codegraph showed Node and Python attach `signing_material.signer_policy` to
  `SignedInvocation`, while Java parsed no signer policy from
  `SigningMaterial` and always constructed signed invocations with
  `Map.of()`.
- The root abstraction problem is signer policy custody loss: a prepared
  invocation can carry provider-managed signing constraints, but the Java
  language facade erased those constraints before submit-signed transport.
- The selected cutover is to introduce a generic Java `SignerPolicy` value
  object, parse it at the signing-material boundary, select the policy signer
  when present, and serialize it on the signed invocation envelope.

## Iteration 42 decision log

- Added Java `SignerPolicy` as a product-neutral runtime signing policy value
  object with exact wire projection for `mode`, `signer_id`, `policy_ref`, and
  `expires_at_unix_ms`.
- Java `SigningMaterial` now parses and serializes optional
  `signer_policy`.
- Java `PreparedInvocation.signWithCallerSignature` now uses the prepared
  signer policy's signer id when present and passes the policy into
  `SignedInvocation`.
- Java `SignedInvocation` now normalizes constructor policy maps into typed
  `SignerPolicy` and serializes the policy object instead of retaining an
  untyped map.
- Java runtime seam tests now assert provider-managed signer id selection and
  signed-envelope `policy_ref` preservation.
- Added SPEC v2 `check_java_sdk_signer_policy_custody_contract`, including a
  negative self-test fixture that rejects the retired Java policy-drop shape.
- Rebuilt `sdk/conformance/canonical-public-api.json` and
  `sdk/conformance/sdk-parity-matrix.json` after the Java SDK public model
  gained `SignerPolicy` and policy accessors.
- Verification passed:
  - `/Users/macbook.silan.tech/.local/bin/codegraph explore "SignedInvocation signer_policy policy signWithCallerSignature signer_id prepared signingMaterial cross-language SDK Go Python Node Java Swift"`
  - `cd sdk/java && mvn -q test`
  - `python3 sdk/conformance/rebuild_public_api_model.py --write`
  - `bash -n tools/scripts/check-canonical-runtime-convergence-v2.sh`
  - `tools/scripts/check-canonical-runtime-convergence-v2.sh --self-test`
  - `tools/scripts/check-canonical-runtime-convergence-v2.sh`
  - `tools/scripts/check-sdk-canonical-public-api.sh`
  - `tools/scripts/check-architecture-convergence.sh`
  - `cargo fmt --check`
  - `git diff --check`

## Iteration 43 candidate policy

- Continue RF-5/RF-4 convergence by applying the same signer custody policy
  preservation to the Swift SDK.
- codegraph showed Swift still lacked `SignerPolicy`, did not parse
  `signing_material.signer_policy`, selected only signature key hints for
  `signerId`, and serialized signed submissions without a policy object.
- The root abstraction problem is the same custody gap fixed in Java:
  provider-managed signing policy is part of prepared canonical material and
  must survive language-facade sign/submit boundaries.
- The selected cutover is to add a generic Swift `SignerPolicy` value object,
  carry it through `SigningMaterial`, use the policy signer when present, and
  serialize it on `SignedInvocation`.

## Iteration 43 decision log

- Added Swift `SignerPolicy` with exact generic runtime fields:
  `mode`, `signerId`, `policyRef`, and `expiresAtUnixMS`.
- Swift `SigningMaterial` now parses and serializes optional
  `signer_policy`.
- Swift `PreparedInvocation.signWithCallerSignature` now selects
  `signingMaterial.signerPolicy.signerId` when present and carries the policy
  into `SignedInvocation`.
- Swift `SignedInvocation` now exposes optional `policy` and serializes it on
  the signed envelope when present.
- Swift runtime seam tests now assert provider-managed signer id selection and
  signed-envelope `policy_ref` preservation.
- Added SPEC v2 `check_swift_sdk_signer_policy_custody_contract`, including a
  negative self-test fixture that rejects the retired Swift policy-drop shape.
- Rebuilt `sdk/conformance/canonical-public-api.json` and
  `sdk/conformance/sdk-parity-matrix.json` after the Swift SDK public model
  gained `SignerPolicy` and policy accessors.
- Verification passed:
  - `/Users/macbook.silan.tech/.local/bin/codegraph explore "Swift SigningMaterial SignerPolicy signer_policy SignedInvocation policy signWithCallerSignature provider_managed_signing RuntimeSDK"`
  - `cd sdk/swift && swift test`
  - `python3 sdk/conformance/rebuild_public_api_model.py --write`
  - `bash -n tools/scripts/check-canonical-runtime-convergence-v2.sh`
  - `tools/scripts/check-canonical-runtime-convergence-v2.sh --self-test`
  - `tools/scripts/check-canonical-runtime-convergence-v2.sh`
  - `tools/scripts/check-sdk-canonical-public-api.sh`
  - `tools/scripts/check-architecture-convergence.sh`
  - `cargo fmt --check`
  - `git diff --check`

## Iteration 44 candidate policy

- Continue RF-7/RF-4 convergence on the product-visible failure mode where
  runtime-state reads can surface stale or mismatched caller/session subjects.
- codegraph and source search showed production no longer constructs the
  retired `/session/invocation_history` subject; remaining occurrences are
  negative fixtures. The still-live root abstraction problem is weaker:
  `LocalRuntimeStateReadIssuer` issued a user-owned read subject from the
  credentials file alone, without proving the active daemon attachment or the
  paired User caller signer that daemon Ready claimed.
- The selected cutover is to make runtime-state read subject issuance an
  explicit attachment state machine: credentials ownership, daemon Ready
  identity, Ready signer capability, and live signer custody must all agree
  before the support layer crosses the local Axon gRPC boundary.

## Iteration 44 decision log

- Replaced credentials-only read-subject issuance with
  `LocalRuntimeStateReadSubject::from_runtime_attachment_file`.
- Added a narrow `RuntimeStateReadSignerCustody` seam and production
  `KeyServiceRuntimeStateReadSignerCustody` implementation. The seam proves
  live caller signer custody before issuing a runtime-state read subject while
  keeping key-service details out of the product-facing error.
- Runtime-state reads now require `control.json` Ready discovery, a daemon
  runtime identity, the `paired_user_runtime_signer` capability flag, matching
  daemon/credential realm, matching daemon/credential node, and a live paired
  User signer.
- Added unit coverage for the ready path, missing Ready signer capability,
  stale daemon identity, missing live signer custody, and missing user id.
- Strengthened `check-runtime-state-read-subject-boundary.sh` and its
  self-test fixture so future regressions cannot return to credentials-only
  subject issuance.
- Verification passed:
  - `/Users/macbook.silan.tech/.local/bin/codegraph explore "00000000 session subject owned by does not admit envelope subject caller signer keyring entry not found self-identity invocation_history meta.list_abilities descriptor_ref not found owner is not online"`
  - `cargo test runtime_state_read_subject --features axon-pb`
  - `tools/scripts/check-runtime-state-read-subject-boundary.sh`
  - `tests/scripts/test_check_runtime_state_read_subject_boundary.sh`
  - `tools/scripts/check-canonical-runtime-convergence-v2.sh`
  - `tools/scripts/check-architecture-convergence.sh`
  - `cargo fmt --check`
  - `git diff --check`

## Iteration 45 candidate policy

- Continue product boundary cleanup after the runtime-state read issuer cutover.
  The next concrete legacy path found by codegraph was in the remote-desktop
  product contract: `RemoteDesktopTransportKind::WebRtc` still accepted
  `web_rtc` as a serde alias for the canonical `webrtc` wire value.
- The root abstraction problem is product-contract drift: once a product
  provider retains old spellings at a typed parse boundary, downstream UI and
  tests can keep sending retired vocabulary while the runtime model appears
  canonical.
- The selected cutover is to make the product contract exact: `webrtc` is the
  only accepted wire spelling, and `web_rtc` is retained only as a negative
  regression vector.

## Iteration 45 decision log

- Removed the `web_rtc` serde alias from
  `RemoteDesktopTransportKind::WebRtc`.
- Added remote-desktop contract tests proving canonical `webrtc` decodes and
  retired `web_rtc` fails during typed parse.
- Added `check-remote-desktop-contract-boundary.sh` plus a self-test fixture,
  and wired it into `tests/script_checks.rs`, so the product contract cannot
  reintroduce transport aliases silently.
- Verification passed:
  - `/Users/macbook.silan.tech/.local/bin/codegraph explore "remote-desktop contract serde alias web_rtc webrtc transport session contract compatibility"`
  - `cargo test remote_desktop::contract --features axon-pb`
  - `tools/scripts/check-remote-desktop-contract-boundary.sh`
  - `tests/scripts/test_check_remote_desktop_contract_boundary.sh`
  - `cargo test remote_desktop_contract_boundary_script_holds`
  - `tools/scripts/check-canonical-runtime-convergence-v2.sh`
  - `tools/scripts/check-architecture-convergence.sh`
  - `cargo fmt --check`
  - `git diff --check`

## Iteration 36 candidate policy

- Move the same proof-custody convergence into the Java SDK receipt validator.
  The Java SDK already required receipt proof facts and checked proof hashes,
  but several nested proof-fact objects still accepted unknown metadata.
- codegraph highlighted `RuntimeReceiptProofFacts` as the Java receipt
  canonicalizer boundary and showed it was responsible for validating
  `authority_binding`, `authority_proof`, issuer refs, signatures, causal
  bindings, and receipt refs.
- The root abstraction problem is mandatory proof facts inside open-ended
  object shapes: a product could carry retired metadata through Java receipt
  parsing without that metadata being part of a canonical proof fact.
- The selected cutover is to make Java receipt proof-fact objects exact at the
  SDK boundary instead of treating unknown fields as forward-compat metadata.

## Iteration 36 decision log

- Java `RuntimeReceiptProofFacts` now rejects unknown fields in authority
  bindings, authority proofs, agent/entity refs, signatures, causal bindings,
  and receipt refs.
- `authority_proof.proof_payload_base64` is now an explicit proof-fact field:
  it may be the empty string for binding-hash proofs, but it may not be omitted.
- Added negative Java seam tests for legacy authority binding metadata, legacy
  authority proof metadata, missing proof payload fields, and legacy issuer
  profile metadata.
- SPEC v2 now gates Java receipt proof-fact exactness and includes a negative
  self-test that removes the exact-shape validator.
- Verification passed:
  - `/Users/macbook.silan.tech/.local/bin/codegraph explore "AuthoritySupport Java authority metadata all zero principal subject binding proof fact canonicalizer unknown fields"`
  - `mvn -q -f sdk/java/pom.xml test -Dtest=run.runtime.sdk.RuntimeCoreSeamTest`
  - `mvn -q -f sdk/java/pom.xml test`
  - `bash -n tools/scripts/check-canonical-runtime-convergence-v2.sh`
  - `tools/scripts/check-canonical-runtime-convergence-v2.sh --self-test`
  - `tools/scripts/check-canonical-runtime-convergence-v2.sh`
  - `tools/scripts/check-architecture-convergence.sh`
  - `cargo fmt --check`
  - `git diff --check`

## Iteration 32 candidate policy

- Continue the sidecar helper convergence one level up: after Iteration 31 made
  the nested `invocation` object exact, the top-level request frame still had
  open-ended parsing in provider helpers.
- The daemon's authoritative `SidecarRequestFrame` already uses
  `deny_unknown_fields`; helper parsers must not be wider than the daemon
  contract because that creates a product-facing compatibility seam.
- The root abstraction problem is split protocol authority: daemon frame parsing
  was exact while helper frame parsing could silently accept extra request
  metadata such as `legacy_mode`, encouraging plugins to depend on non-canonical
  carriers.
- The selected cutover is to make the provider sidecar request frame exact
  across Go, Python, Node, Java, and Rust. For exec invoke helpers, only
  `type`, `call_id`, and `invocation` are accepted at the request-frame top
  level.

## Iteration 32 decision log

- Go, Python, Node, Java, and Rust provider helpers now reject unknown top-level
  sidecar request fields before reading `type`, `call_id`, or `invocation`.
- Cross-language negative tests now include `legacy_mode` at the request-frame
  top level and assert the plugin handler is not reached.
- SPEC v2 now gates top-level sidecar request exactness with production helper
  tokens, per-language tests, and a negative self-test that breaks the Python
  request-field guard.
- Verification passed:
  - `/Users/macbook.silan.tech/.local/bin/codegraph explore "compatibility fallback legacy unknown fields sidecar request frame invocation route descriptor signer authority"`
  - `cd sdk/go && go test ./provider/easynet/pluginexec`
  - `PYTHONPATH=/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Axon/sdk/python:/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Cli/sdk/python python3 sdk/python/tests/test_plugin_exec.py`
  - `node --test sdk/node/test/pluginexec.test.mjs`
  - `cargo test -q --manifest-path sdk/rust/provider/easynet/pluginexec/Cargo.toml`
  - `mvn -q -f sdk/java/pom.xml test -Dtest=run.runtime.sdk.provider.easynet.pluginexec.SidecarRuntimeTest`
  - `bash -n tools/scripts/check-canonical-runtime-convergence-v2.sh`
  - `tools/scripts/check-canonical-runtime-convergence-v2.sh --self-test`
  - `tools/scripts/check-canonical-runtime-convergence-v2.sh`
  - `tools/scripts/check-architecture-convergence.sh`
  - `cargo fmt --check`
  - `git diff --check`

## Iteration 33 candidate policy

- Move from provider helper exactness into admission proof exactness. Authority
  proof metadata is a signer/proof custody boundary, so accepting extra fields
  there is more dangerous than accepting presentation metadata: ignored fields
  are not part of the canonical signed material but can still travel with a
  proof envelope.
- codegraph highlighted `AuthorityProof` as a highly shared admission model
  consumed by access-control persistence, admission facade verification, and
  child invocation building.
- The root abstraction problem is that the proof struct had canonical material
  and route/session binding checks, but its serde ingress was still wider than
  the canonical proof model.
- The selected cutover is to make `AuthorityProof` fail closed on unknown
  fields and to gate that invariant together with the existing session/route
  proof facts.

## Iteration 33 decision log

- `AuthorityProof` now uses `#[serde(deny_unknown_fields)]`.
- Added a negative deserialization vector proving a noncanonical
  `legacy_scope` proof field is rejected before verification.
- SPEC v2 `check_authority_proof_session_fact_contract` now requires
  `deny_unknown_fields` and the unknown-field negative test, with self-test
  coverage through the existing authority-proof legacy fixture.
- Verification passed:
  - `/Users/macbook.silan.tech/.local/bin/codegraph explore "AuthorityProof compatibility fallback legacy admission proof signer descriptor route subject unknown fields"`
  - `cargo test -q authority_proof_deserialization_rejects_unknown_fields`
  - `cargo test -q authority_proof`
  - `bash -n tools/scripts/check-canonical-runtime-convergence-v2.sh`
  - `tools/scripts/check-canonical-runtime-convergence-v2.sh --self-test`
  - `tools/scripts/check-canonical-runtime-convergence-v2.sh`
  - `tools/scripts/check-architecture-convergence.sh`
  - `cargo fmt --check`
  - `git diff --check`

## Iteration 34 candidate policy

- Continue closing the same admission/proof custody chain. After
  `AuthorityProof` became strict, the adjacent access-control models still had
  wider serde ingress: `PermissionGrant`, `PermissionRequest`, and
  `PermissionConstraints` accepted unknown fields, and grant/request identity
  facts defaulted when omitted.
- The root abstraction problem is that access-control replay and admission
  request storage could accept legacy or partial policy records and only reject
  them later through validation. For policy custody, schema authority should
  fail at parse time.
- The selected cutover is to make policy request/grant schemas exact and to
  require owner/principal identity facts explicitly.

## Iteration 34 decision log

- `PermissionGrant`, `PermissionRequest`, and `PermissionConstraints` now use
  `#[serde(deny_unknown_fields)]`.
- Removed serde defaults from `PermissionGrant.owner_user_id`,
  `PermissionGrant.principal_id`, `PermissionRequest.owner_user_id`, and
  `PermissionRequest.principal_id`.
- Added negative deserialization tests for unknown grant/request/constraint
  fields and missing owner/principal identity fields.
- SPEC v2 now includes `check_access_control_policy_schema_contract`, with a
  self-test fixture that preserves the retired defaults and missing
  deny-unknown attributes.
- Verification passed:
  - `/Users/macbook.silan.tech/.local/bin/codegraph explore "PermissionRequest PermissionGrant access-control legacy compatibility unknown fields authority proof admission persistence"`
  - `cargo test -q permission_grant_deserialization`
  - `cargo test -q permission_constraints_deserialization`
  - `cargo test -q permission_request_deserialization`
  - `cargo test -q access_control`
  - `bash -n tools/scripts/check-canonical-runtime-convergence-v2.sh`
  - `tools/scripts/check-canonical-runtime-convergence-v2.sh --self-test`
  - `tools/scripts/check-canonical-runtime-convergence-v2.sh`
  - `tools/scripts/check-architecture-convergence.sh`
  - `cargo fmt --check`
  - `git diff --check`

## Iteration 35 candidate policy

- Continue from policy payload exactness to access-control store envelope
  exactness. Manifest, journal, audit, and compaction checkpoint records are
  replay-facing authority state, so they must not accept legacy metadata outside
  the canonical policy schema.
- codegraph highlighted `AccessControlStoreManifest`, `GrantAuditRecord`,
  `CompactionCheckpoint`, and `JournalRecord` as shared persistence models
  around policy replay and audit custody.
- The root abstraction problem is strict policy payloads inside wider
  persistence envelopes: a legacy field could survive at the replay/audit layer
  even after the grant/request payload became exact.
- The selected cutover is to make access-control store manifest sections,
  journal records, audit records, compaction policy/result records, and
  checkpoint records fail closed on unknown fields.

## Iteration 35 decision log

- `AccessControlStoreManifest`, `PolicyStoreSection`,
  `CanonicalizationSection`, `PolicyStoreFiles`, `GrantAuditRecord`,
  `AuthorityBindingGrantResult`, `PermissionRequestResolutionResult`,
  `AccessControlCompactionPolicy`, `AccessControlCompactionResult`,
  `CompactionCheckpoint`, and `JournalRecord` now use
  `#[serde(deny_unknown_fields)]`.
- Added negative deserialization tests for legacy manifest metadata, legacy
  policy-store owner metadata, legacy journal sequencing metadata, legacy audit
  actor metadata, and legacy compaction retention metadata.
- SPEC v2 now includes `check_access_control_store_schema_contract`, with a
  negative self-test fixture that proves the gate fails when store envelopes are
  not exact.
- Verification passed:
  - `/Users/macbook.silan.tech/.local/bin/codegraph explore "AccessControlStoreManifest GrantAuditRecord CompactionCheckpoint JournalRecord legacy unknown fields access-control replay audit authority"`
  - `cargo test -q access_control`
  - `bash -n tools/scripts/check-canonical-runtime-convergence-v2.sh`
  - `tools/scripts/check-canonical-runtime-convergence-v2.sh --self-test`
  - `tools/scripts/check-canonical-runtime-convergence-v2.sh`
  - `tools/scripts/check-architecture-convergence.sh`
  - `cargo fmt --check`
  - `git diff --check`

## Iteration 37 candidate policy

- Converge the Go SDK to the same receipt proof-fact exactness introduced for
  Java in Iteration 36. The Go `RuntimeReceipt` projection kept the original
  raw JSON, but nested typed structs and map-based authority binding parsing
  could still ignore unknown proof metadata.
- codegraph highlighted Go/Python receipt proof-fact paths after the Java
  cutover. Go was selected first because its conformance attestation directly
  detects SDK source hash changes and the runtime receipt tests already cover
  proof facts.
- The root abstraction problem is split schema authority: typed receipt
  projection validated required canonical facts while raw nested proof objects
  remained wide enough to carry legacy metadata.
- The selected cutover is to validate Go receipt proof-fact object shapes from
  the raw JSON boundary before constructing canonical Axon proof facts.

## Iteration 37 decision log

- Go `RuntimeReceipt.ValidateProofFacts` now calls
  `validateRuntimeReceiptRawProofShape` before canonical proof-fact
  construction.
- Go receipt parsing now rejects unknown fields in authority bindings,
  authority proofs, issuer/agent/entity refs, signatures, causal bindings, and
  receipt refs.
- `authority_proof.proof_payload_base64` is now explicit in Go receipt JSON:
  it may be empty for binding-hash proofs, but it may not be omitted.
- Added Go negative tests for legacy authority binding metadata, legacy
  authority proof metadata, missing proof payload fields, and legacy issuer
  profile metadata.
- SPEC v2 now gates Go receipt proof-fact exactness with a negative self-test
  that removes the exact-shape validator, and the session authority facade gate
  now expects exact-schema rejection for retired Go session fields.
- Rebuilt `sdk/conformance/canonical-public-api.json` and
  `sdk/conformance/sdk-parity-matrix.json` after Go SDK source changes.
- Verification passed:
  - `/Users/macbook.silan.tech/.local/bin/codegraph explore "Go Python runtime receipt authority_binding authority_proof proof facts unknown fields legacy metadata canonicalizer"`
  - `cd sdk/go && go test -count=1 -run 'TestRuntimeReceipt' .`
  - `cd sdk/go && go test ./...`
  - `python3 sdk/conformance/rebuild_public_api_model.py --write`
  - `bash -n tools/scripts/check-canonical-runtime-convergence-v2.sh`
  - `tools/scripts/check-canonical-runtime-convergence-v2.sh --self-test`
  - `tools/scripts/check-canonical-runtime-convergence-v2.sh`
  - `tools/scripts/check-architecture-convergence.sh`
  - `cargo fmt --check`
  - `git diff --check`

## Iteration 39 candidate policy

- Continue RF-4/RF-3 convergence from the runtime-state read subject failure
  mode observed in product usage. Go, Python, and Node already exposed a
  canonical runtime-state subject helper and shared session-authority subject
  admission predicate, but Java and Swift were not aligned.
- codegraph and source inspection showed the immediate root abstraction problem
  in Swift: `InvocationBuilder.inspect()` validated only authority metadata
  shape, then returned an invocation draft without proving that a
  `DelegationProof` or `SessionAuthority` admitted the tuple's
  caller/callee/subject/ability.
- Java already had tuple-bound authority validation, but lacked the public
  runtime-state read subject constructor, which encourages product facades to
  hand-assemble `resource/user...` subjects.
- The selected cutover is to make Java/Swift consume the same canonical
  runtime-state subject model and to make Swift invocation authority binding
  fail closed at the draft boundary.

## Iteration 39 decision log

- Added Java `RuntimeSubjects.runtimeStateReadSubjectURA(realm, userID)` backed
  by `AuthoritySupport.runtimeStateReadSubjectURA`, with all-zero and canonical
  resource-subject guards.
- Added Swift `runtimeStateReadSubjectURA(realm:userID:)` using the same
  `runtime-state/read` Resource URA projection.
- Refactored Swift invocation authority validation into
  `validateInvocationAuthorityBinding(_:)` and
  `InvocationAuthorityBindingValidator`, mirroring the Java tuple-bound
  delegation/session checks.
- Swift `InvocationBuilder.inspect()` now constructs the tuple once, validates
  authority metadata against that tuple, and only then returns the draft.
- Added Swift negative tests for delegation subject mismatch and session
  subject path-substring/nested-resource regressions, plus Java/Swift
  runtime-state subject helper tests.
- SPEC v2 now gates Java/Swift runtime-state subject parity and Swift
  tuple-bound authority validation, each with negative self-test fixtures.
- Rebuilt `sdk/conformance/canonical-public-api.json` and
  `sdk/conformance/sdk-parity-matrix.json` after Java/Swift public surface
  changes.
- Verification passed:
  - `/Users/macbook.silan.tech/.local/bin/codegraph explore "compatibility aliases node_id device_id stream kind event content-type request_id prepared_id runtime SDK"`
  - `/Users/macbook.silan.tech/.local/bin/codegraph explore "RuntimeEnvironment device_id node_id serde alias stream kind event prepared_id request_id content_type alias legacy SDK Go Python CABI"`
  - `cd sdk/java && mvn -q test`
  - `cd sdk/swift && swift test`
  - `python3 sdk/conformance/rebuild_public_api_model.py --write`
  - `bash -n tools/scripts/check-canonical-runtime-convergence-v2.sh`
  - `tools/scripts/check-canonical-runtime-convergence-v2.sh --self-test`
  - `tools/scripts/check-canonical-runtime-convergence-v2.sh`
  - `tools/scripts/check-sdk-canonical-public-api.sh`
  - `tools/scripts/check-architecture-convergence.sh`
  - `git diff --check`

## Iteration 40 candidate policy

- Continue RF-3/RF-4 convergence by removing the last observed
  `PreparedInvocation` identity alias across language SDKs.
- codegraph and source inspection showed that Go/Python already required the
  provider-issued `prepared_id`, while Node, Java, and Swift still allowed
  `request_id` to stand in as a prepared identity. Swift also still allowed a
  missing top-level `descriptor_ref` to be reconstructed from signing material.
- The root abstraction problem is mixed identity ownership: `prepared_id` is
  the provider/native prepared handle, while `request_id` is only request
  correlation metadata. Treating them as substitutes preserves a hidden legacy
  path and weakens descriptor-bound proof custody.
- The selected cutover is to require explicit `prepared_id` and explicit
  top-level `descriptor_ref` at every prepared invocation decode/constructor
  boundary, with request IDs remaining observation/correlation metadata only.

## Iteration 40 decision log

- Node `PreparedInvocation` now rejects request-id-only payloads with
  `prepared_id is required`.
- Java `PreparedInvocation` now rejects request-id-only payloads and direct
  constructor calls instead of accepting `request_id` as a legacy prepared
  handle alias.
- Swift `PreparedInvocation` now rejects request-id-only payloads and direct
  constructor calls, and no longer backfills missing top-level
  `descriptor_ref` from signing material.
- Added Node/Java/Swift negative tests for request-id-only prepared invocation
  payloads, plus a Swift negative test for missing explicit top-level
  `descriptor_ref`.
- Extended SPEC v2 `check_sdk_prepared_descriptor_ref_required_contract` to
  cover Node/Java/Swift prepared identity semantics and Swift descriptor-ref
  explicitness.
- Added negative self-test fixtures proving the gate rejects descriptor-ref
  fallback and prepared-id alias regressions.
- Verification passed:
  - `/Users/macbook.silan.tech/.local/bin/codegraph explore "PreparedInvocation prepared_id request_id descriptor_ref fallback alias Java Swift Go Python SDK canonical runtime"`
  - `cd sdk/node && npm test`
  - `cd sdk/swift && swift test`
  - `cd sdk/java && mvn -q test`
  - `bash -n tools/scripts/check-canonical-runtime-convergence-v2.sh`
  - `tools/scripts/check-canonical-runtime-convergence-v2.sh --self-test`
  - `tools/scripts/check-canonical-runtime-convergence-v2.sh`
  - `tools/scripts/check-sdk-canonical-public-api.sh`
  - `tools/scripts/check-architecture-convergence.sh`
  - `cargo fmt --check`
  - `git diff --check`
