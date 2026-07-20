# Codegraph evidence

Evidence will be appended after indexing and focused impact queries.

## 2026-07-20 Doctor agent projection fallback audit

- `codegraph query check_agents` identified the product health-check owner in
  `src/cli/commands/doctor.rs`.
- `rg` showed the active fallback inside `check_agents()`: on `agent.list`
  failure the command appended an unavailable warning, substituted
  `Vec::new()`, and then treated the result as an empty registry by probing
  default `claude-code` and `codex` local CLIs.
- The same function also used `filter_map(...agent_kind(row).ok())`, hiding
  invalid daemon runtime projections, while both `doctor` and
  `agent doctor` selected probes with a "claude-code else codex" branch. That
  made `external` and any future non-Codex runtime look like Codex CLI checks.
- The root abstraction problem was mixed authority: daemon `agent.list` is the
  registry projection authority, while local CLI probing is only a product
  diagnostic over declared runtime kind. An unavailable or corrupt registry
  projection must remain visible instead of being repaired into local CLI
  facts.
- After the change, `codegraph query LocalAgentCliProbe` shows the single CLI
  command-layer runtime-to-probe mapping; `codegraph query is_claude_code`
  returns no results. `check_agents()` returns immediately on unavailable
  `agent.list`, fails invalid runtime rows, maps `codex-app-server` explicitly
  to the Codex probe, and treats `external` as having no local CLI probe.

## 2026-07-20 Device show legacy state projection audit

- `rg` found an explicit compatibility path in
  `src/cli/commands/groups/device.rs`: `device show` accepted a numeric
  `node.describe.state` and translated the old Axon SDK enum values into
  string labels, then projected missing/unknown state as `UNKNOWN`.
- The root abstraction problem was schema drift hidden inside product display:
  `node.describe` is the canonical substrate inspection projection, so a
  non-string or missing `state` means the describe schema is invalid. Rendering
  a legacy enum label or `UNKNOWN` creates a second read-model authority and
  can hide why a substrate is not routeable.
- After the change, `codegraph query device_show_state` reports the new
  schema-bound state extractor. It accepts only string `state` and fails closed
  on missing or numeric legacy state before `device show` renders the node.

## 2026-07-20 LocalDevice resource subject migration audit

- `rg` found the active migration owner in
  `src/daemon/persistence/resources.rs`: existing local-device rows using the
  retired single-segment resource subject were rewritten by
  `canonical_resource_ura_for_existing(...)` once `owner_agent` became a device
  URA.
- The root abstraction problem was split authority for resource ownership:
  `resources.json` could persist a generic `resource/<id>` subject, while
  device-local media runtime paths expected
  `resource/device.<device-id>/streams/<kind>.<id>`. Rewriting the subject
  during upsert hid stale local state and preserved a compatibility lifecycle
  inside the resource store.
- `resource_bootstrap.rs` also retained a stale pruning detector for old
  bootstrap metadata (`capture_target` plus backend), allowing retired local
  discovery state to influence current resource publication.
- After the change, LocalDevice inserts require a same-realm device
  `owner_agent`, existing LocalDevice rows must already have the canonical
  device-stream resource subject, and bootstrap propagation fails explicitly on
  retired resource state instead of silently rewriting it.

## 2026-07-20 Recording artifact content-type fallback audit

- `codegraph query fallback` reported the production method
  `MediaRecordingKind::fallback_content_type` in
  `src/cli/commands/ability_record.rs`.
- `codegraph impact frame_content_type` showed a narrow production impact:
  `frame_content_type(...)` feeds `RecordingSink::write_artifact_frame(...)`,
  which feeds `RecordingSink::write_frames(...)`.
- The root abstraction problem was that CLI recording manifests were minting
  artifact facts from ability kind (`mic` → `audio/L16`, `camera` →
  `image/jpeg`) when neither the payload nor the stream frame carried
  `content_type`. That preserved a product-side evidence fallback instead of
  requiring the runtime stream to provide the artifact metadata it produced.
- After the change, `frame_content_type(...)` is fallible and recording fails
  closed if an artifact frame omits content type. `codegraph query
  fallback_content_type` returns no results.

## 2026-07-20 Remote-device local-realm target fallback audit

- `codegraph query fallback` reported the active tests
  `directory_hit_beats_local_realm_fallback` and
  `local_realm_fallback_is_used_when_directory_misses` in
  `src/support/platform/remote_device.rs`, pointing at the production resolver
  `resolve_target_device_ura_with_lookup(...)`.
- The production resolver accepted a bare node id, hid
  `federation.discover` failures as `None`, and on directory miss minted
  `easynet:///r/<local-realm>/device/<node>`. That created a second target
  authority adjacent to the federated directory and could turn unresolved
  namespace state into later `owner is not online` route failures.
- After the change, bare node ids resolve only through the daemon-backed
  federated directory; directory failures propagate, and directory misses fail
  closed with an instruction to refresh federation state or pass a canonical
  device URA.
- `codegraph query local_realm_fallback`,
  `codegraph query directory_hit_beats_local_realm_fallback`, and
  `codegraph query local_realm_fallback_is_used_when_directory_misses` return
  no results after sync.

## 2026-07-20 focused identity projection audit

- `codegraph impact RuntimeIdentityProjection` reported the affected production
  surface as Go/Python `runtime_environment` plus provider/test files.
- `codegraph callers NewRuntimeIdentityProjectionFromJSON` reported one Go
  production caller: `ReadRuntimeIdentityProjection`.
- After the change, `codegraph impact RuntimeIdentityProjection` includes the
  new Go EasyNet provider adapter, and `codegraph callers
  ReadDaemonRuntimeIdentityProjection` reports no production callers.
- `rg` confirmed `sdk/go/runtime_environment.go` no longer reads `device_id` or
  `node_id`; those product credential fields are accepted only by
  `sdk/go/provider/easynet/identity.go`.

## 2026-07-20 LedgerSink fallback audit

- `codegraph impact McpToolRouteTable` showed the active MCP tools/call
  surface routes through `target_for_tool()` in `mcp.bridge.call_tool` and
  `InvokeMcpProvider`; `codegraph callers target_for_tool` reported only those
  production call paths plus `canonical_for_tool`. The runtime route table
  already rejects canonical dotted MCP aliases, so the remaining bridge delta
  was stale compatibility language in the header.
- `codegraph impact ledger_route_ura` reported one production caller:
  `install_ledger_sink`, which wires the resolver into Axon's `LedgerSink`.
  That made `runtime_factory.rs` the actual daemon receipt-owner adapter for
  terminal ledger rows.
- `rg` found the active `_system` fallback in `ledger_invocation_ura()` and
  `ledger_route_ura()`: unowned records could become
  `resource/_system/authority.invocations/...`, and unowned routes could become
  `_system/ability/authority.system.<ability>`.
- After the change, `runtime_factory.rs` contains no direct
  `invocation_history_resource_ura(...)` fallback and no
  `hub_ability_ura("_system", ...)` route fallback; both resolver miss paths
  fail fast with explicit `LedgerSink cannot derive ... from binding` messages.

## 2026-07-20 Mission model authority audit

- `rg` found an active mission dispatch fallback where
  `resolve_model_with_overrides()` resolved
  `per-call override > agent.toml spec > entry.model`.
- The fallback was not an edge-adapter quarantine: it lived directly in
  `src/daemon/execution/mission/dispatch.rs`, the production path that writes
  mission `meta.json` and dispatches the runtime adapter.
- The root abstraction problem was dual authority for Agent runtime model:
  `agent.toml` is the AgentDirectory runtime configuration authority, while
  `AgentEntry` is registry placement/state. Reading `entry.model` during
  dispatch allowed stale registry state to override a missing spec value.
- After the change, `resolve_model_with_overrides()` accepts only
  `override_model` and `spec_model`; `entry.model` appears only in negative
  tests and explanatory comments proving it is ignored.

## 2026-07-20 Mission timeout authority audit

- `codegraph impact resolve_timeout` reported a narrow impact surface:
  production mission dispatch plus resolver tests.
- `rg` confirmed the active fallback:
  `resolve_timeout(spec_source.timeout_secs, entry.timeout_secs)` in
  `src/daemon/execution/mission/dispatch.rs`.
- The v1 registry migration already persists non-default row timeouts into
  `agent.toml` and intentionally omits default timeouts. Therefore dispatch
  reading `entry.timeout_secs` after opening `AgentDirectory` was a second
  runtime configuration authority.
- After the change, `resolve_timeout()` accepts only `spec_timeout_secs`; a
  missing spec value resolves through the canonical agent runtime default
  exposed by `agent_registry::default_timeout_for_new_rows()`.

## 2026-07-20 Trust-anchor user-key fallback audit

- `codegraph impact RealmTrustAnchor` reported the active trust surface around
  `src/daemon/trust/anchor.rs`, `RealmTrustAnchorKeyResolver`,
  `device_trust_sync`, federation key wrappers, bidi admission, and identity
  write policy.
- `rg` found an active user bucket fallback inside
  `RealmTrustAnchor::lookup()`: when `by_ura` missed, bare user URAs were
  resolved by selecting a deterministic key from `self.users`.
- The root abstraction problem was a mixed lookup method: singleton runtime
  principals (`backend`, `device`, `hub`) are keyed by URA, while users are
  keyed by `(user_ura, pubkey)` under DEC-EU multi-device. A bare user URA
  lookup created a second signer-selection authority and could hide missing
  presented-key proof.
- After the change, `RealmTrustAnchor::lookup()` reads only `by_ura`;
  user-role trust remains available through `lookup_user_by_pubkey()` and
  `lookup_user_all()`.

## 2026-07-20 Descriptor call-mode ingress fallback audit

- `codegraph query fallback` and targeted `rg` found active descriptor
  resolver defaults in the generic runtime/provider seam:
  `RuntimeClient.resolve_descriptor_ref(..., call_mode="rpc")` in Python,
  `RuntimeClient.ResolveDescriptorRef` defaulting blank Go `CallMode` to
  `"rpc"`, C ABI diagnostics fallback defaulting missing request/catalog
  `call_mode` to `"rpc"`, and Rust FFI descriptor resolution accepting a
  missing request `call_mode`.
- The root abstraction problem was tuple ingress defaultization. The ability
  tuple is complete only when its selector includes the intended call mode;
  letting provider seams infer RPC creates a second descriptor-selection
  authority and hides caller mistakes in stream/bidi paths.
- After the change, Go, Python, and Rust FFI generic descriptor resolvers
  require explicit `call_mode`. High-level convenience surfaces still pass
  `"rpc"`/`"stream"` explicitly before crossing the provider seam.

## 2026-07-20 Python key-service compatibility facade audit

- `codegraph node sdk/python/easynet_sdk/_key_service.py` reported a 38-line
  private module used only by `sdk/python/easynet_sdk/managed_signing.py` and
  `sdk/python/tests/test_managed_signing.py`.
- The module was not an owned runtime abstraction. Its header explicitly
  described `REQ-LANG-5 compatibility exports for the EasyNet key-service
  provider`, and every symbol was a re-export from
  `sdk/python/easynet_sdk/providers/easynet/key_service.py`.
- The root abstraction problem was a product-named compatibility facade inside
  the Python SDK root. `managed_signing` is the canonical public capability;
  the provider implementation belongs under `providers.easynet`, not behind a
  second private SDK facade with EasyNet key-service ownership language.
- After the change, `codegraph node sdk/python/easynet_sdk/managed_signing.py`
  shows the production import resolves directly to
  `.providers.easynet.key_service`. `rg` finds no `_key_service` imports in
  Python SDK production or tests; remaining `_key_service.py` mentions are only
  the SDK neutrality gate rule and its negative self-test fixture.

## 2026-07-20 Go daemon lifecycle compatibility facade audit

- `codegraph node sdk/go/daemon_compat.go` reported a 139-line Go SDK root
  module with 40 public symbols. Its header described `REQ-LANG-5
  compatibility types`, and its public surface exposed `Daemon*`,
  `NewDaemonControl`, top-level `Start`/`Attach`/`Discover`/`ConnectLocal`, and
  `runtimeLifecycleCompatibilityAdapter`.
- `codegraph impact RuntimeLifecycle` showed the adapter was not just dead
  API: `runtime_admin.go`, `environment.go`, native CABI helpers, live smoke
  tests, and provider lifecycle tests were still using source-compatible
  lifecycle names from the canonical root.
- The root abstraction problem was that Go's canonical SDK root still carried
  an EasyNet daemon lifecycle facade, while the provider-owned EasyNet process
  policy already exists under `sdk/go/provider/easynet/contract`. This made the
  SDK look like an EasyNet SDK instead of a runtime model with provider-owned
  start/discover requests.
- After the change, `codegraph query runtimeLifecycleCompatibilityAdapter` and
  `codegraph query NewDaemonControl` return no indexed results. `RuntimeAdmin`
  now depends directly on `RuntimeHostLifecycle`; native runtime start accepts
  a provider-owned `RuntimeHostStartRequest`; and `RuntimeHostDiscoverOptions`
  is the only provider-neutral concrete discover DTO in the Go SDK root.

## 2026-07-20 Local daemon-system subject fallback audit

- `codegraph node src/daemon/invocation/routing/target.rs` showed the canonical
  target resolver already models daemon-system subject derivation as an explicit
  state, not as public ingress absence.
- `rg` on `LocalDaemonLoopbackTuplePlan` showed the older support transport
  still had a separate `CallerDeclaredSubject` state and
  `targeted_root_with_declared_subject(...)` accepted `subject: Option<String>`.
- The root abstraction problem was not LocalRuntime itself; it was the product
  system facade boundary. Product commands could call a target-owned system
  ability with `None`, letting support transport choose `default_subject_ura`
  later. That preserved a hidden tuple repair path adjacent to the explicit
  `LocalSystemInvocationIssuer::root_context` model.
- The selected convergence path deletes the fallback state. Target-owned
  daemon-system root/stream calls now require a materialized `subject_ura`
  before transport entry; callers either pass their recovered envelope subject
  or deliberately pass `target.default_subject_ura()`.

## 2026-07-20 Agent workspaces layout migration audit

- `codegraph query migrate_legacy_agents_directory` identified the
  compatibility owner in `src/daemon/persistence/config.rs`.
- `codegraph impact migrate_legacy_agents_directory` showed the active impact
  was limited to the two process entrypoints, registry read/write, and
  migration tests.
- The root abstraction problem was that runtime startup and registry I/O still
  carried the retired `~/.easynet/workspaces` directory model. Even though
  runtime readers used `agents_root()`, the boot/read/write path still owned a
  second directory lifecycle and a registry root-prefix repair step.
- After the change, `agents_root()` is the only per-agent directory authority.
  Startup does not migrate old state, registry load/write does not rewrite
  legacy prefixes, and the architecture gate no longer allows
  `legacy_agents_root()` in production.

## 2026-07-20 Authority all-zero principal audit

## 2026-07-20 Chat session inventory fallback audit

- `codegraph impact rescan_dir` identified the active production impact as
  `src/daemon/persistence/chat_sessions.rs::list_sessions` and
  `src/daemon/ability/builtins/agents/chat_history.rs::list_handler`.
- `rg` confirmed the retired path: when `index.json` was missing, empty, or
  corrupt, `list_sessions()` scanned `*.jsonl` transcript files and assembled a
  synthetic session inventory.
- The root abstraction problem was split persistence authority. `index.json`
  is the canonical session inventory and pointer state (`latest`, `lifelong`,
  ordered descriptors), while JSONL files are transcript content. Scanning
  transcript files let stale local content recreate discovery facts and hide
  corrupted pointer state.
- After the change, `list_sessions()` is index-only, malformed index state
  fails closed, `write_turn_inner()` reads the index before appending JSONL, and
  `set_lifelong_session()` can only bind session ids already present in the
  canonical index.
- `/Users/macbook.silan.tech/.local/bin/codegraph query rescan_dir` and
  `/Users/macbook.silan.tech/.local/bin/codegraph query
  list_sessions_falls_back_to_dir_scan_when_index_missing` return no results

## 2026-07-20 Global skill directory-name identity fallback audit

- `codegraph query skill_dir_in_global_pool` identified the single production
  resolver for global skill pool directory lookup:
  `src/daemon/resources/skills/store.rs::skill_dir_in_global_pool`.
- `codegraph query global_skill_record_from_dir` identified the synthetic
  public row constructor used by global skill listing:
  `src/daemon/resources/skills/store.rs::global_skill_record_from_dir`.
- `rg` found the legacy fallback in both paths: lookup accepted either
  `parse_skill_md_name(SKILL.md)` or the directory name, and row construction
  used `unwrap_or_else(|| dir_name.to_string())` when frontmatter was absent.
- The root abstraction problem was split identity authority. Global skill pools
  are externally managed packages, but the daemon was allowing filesystem
  layout to become public skill identity. That preserved a compatibility path
  where old directory-only packages could be listed, read, and addressed
  without declaring a package name.
- After the change, global skill identity is metadata-only. `SKILL.md`
  frontmatter `name` is required for lookup and listing; directory names remain
  only `SkillSource.subpath` provenance.

## 2026-07-20 Runtime status empty-fleet fallback audit

- `codegraph query fetch_directory_entries` reported two CLI implementations:
  `src/cli/commands/devices.rs::fetch_directory_entries`, already returning
  `anyhow::Result<Vec<Value>>`, and
  `src/cli/commands/status.rs::fetch_directory_entries`, still returning
  `Vec<Value>`.
- `rg 'Fleet: cannot query|Vec::new' src/cli/commands/status.rs` identified
  the active compatibility behavior: `easynet status` printed an informational
  line and returned `Vec::new()` when user-scoped `federation.discover` failed.
- The root abstraction problem was split product directory semantics. `device
  list` treated directory failure as a failed product read, while `runtime
  status` projected the same signer/admission/namespace failure as a valid
  empty fleet. That hides exactly the class of caller-signer and owner-offline
  failures that product users need to see.
- After the change, status fleet reads use the same fail-closed shape as
  `device list`: directory failure returns an error with explicit context that
  status refuses to project the failure as an empty fleet.

## 2026-07-20 Device show target and ability fallback audit

- `rg 'same-realm fallback|Vec::new|meta.list_abilities' src/cli/commands/groups/device.rs`
  identified two product-facing compatibility paths in `easynet device show`.
- The first fallback was target synthesis: non-local bare device ids were
  passed into the remote describe path, whose downstream helper could interpret
  them relative to current product state. That preserved a second target
  authority next to canonical Device URAs.
- The second fallback was ability repair: when `node.describe` omitted
  `abilities`, the CLI queried local `meta.list_abilities`; if that failed, it
  returned `Vec::new()`. A remote/stale descriptor or signer failure could
  therefore render as "0 abilities hosted on this substrate."
- The root abstraction problem was mixed inspection authority. A device
  inspection result must be the inspected device's descriptor-bound response,
  not a composite assembled from local catalogue state.
- After the change, `device show` classifies targets through explicit states:
  local/self or canonical remote Device URA. Missing `abilities` fails closed
  and the local catalogue is no longer used as a repair authority.

## 2026-07-20 Device remove same-realm target fallback audit

- `codegraph query canonicalize_remove_target_ura` identified the product
  revocation target boundary in `src/cli/commands/groups/device.rs`.
- `rg 'device_ura\\(&local_tenant|local_tenant|pair this device first'` showed
  `run_remove` still accepted a bare target id and constructed
  `easynet:///r/<current-realm>/device/<id>` when local credentials were
  present.
- The root abstraction problem was the same target authority fork removed from
  `device show`: product CLI state was allowed to manufacture a remote
  canonical owner instead of requiring the operator to pass the canonical owner
  being revoked.
- After the change, remove target canonicalization is metadata-free and
  fail-closed: target input must parse as a canonical Device URA. A bare id
  matching the current node is rejected with the local `device reset` lifecycle
  guidance; other bare ids are never promoted to same-realm remote targets.
  after sync.

## 2026-07-20 MCP reflection mode fallback audit

- `rg` found `McpReflectionMode::from_env()` in
  `src/daemon/ability/builtins/integrations/mcp/reflective_registry.rs`
  converting an unrecognized `EASYNET_MCP_REFLECTION` value into `Lazy` after
  emitting a warning event.
- The production caller is daemon registry assembly in
  `src/daemon/ability/catalog/build.rs`, so a typo such as
  `EASYNET_MCP_REFLECTION=eagre` could still boot a daemon and advertise the
  lazy MCP reflection lifecycle instead of the operator-requested policy.
- The root abstraction problem was configuration authority repair: the daemon
  registry lifecycle had a declared state machine (`off`, `lazy`, `eager`) but
  the env adapter could synthesize `lazy` from invalid operator input.
- After the change, missing or empty env still resolves to the explicit default
  (`lazy`), while unknown values return `UnknownReflectionMode` and abort
  daemon registry assembly before runtime services are advertised.
- `/Users/macbook.silan.tech/.local/bin/codegraph query
  reflection_mode_unknown` returns no results after sync.

- `codegraph query SessionAuthority` showed the canonical authority facade is
  implemented across Go, Python, Node, Java, and Swift SDKs, with downstream
  impact through `RuntimeAbilityClient`, `AuthorizedRuntimeSession`, and
  language seam tests.
- The live product error showed a session authority owned by
  `00000000-0000-0000-0000-000000000000` reaching daemon admission and being
  rejected later as `AUTHORITY_SUBJECT_MISMATCH`. That is not a product
  special case: all-zero principal identifiers are invalid canonical authority
  facts and must not cross any SDK authority facade.
- The root abstraction problem was that some SDK validators rejected all-zero
  invocation tuple principals, but authority metadata could still materialize
  all-zero `session_owner_user_id` / owner-bearing URAs. This left a split
  model where tuple validation was canonical but authority validation could
  carry legacy placeholder ownership.
- After the change, all language authority facades reject all-zero authority
  participants before metadata projection or request minting. Go/Python share a
  small identity guard instead of duplicating the literal inside session logic.

## 2026-07-20 Paired User signer lifecycle audit

- The live product error showed `remote invocation requires a caller signer`
  for the paired User URA while daemon boot had already reported Ready. That
  means canonical SDK invocation was active, but the runtime identity lifecycle
  was not closed before publication.
- `codegraph impact load_runtime_caller_signer` showed user-as-caller signing
  is consumed by remote RPC, stream, bidi, CLI invoke, A2A client, and
  federation probe paths. Fixing one product button would leave the same
  missing signer on other invocation surfaces.
- `rg` found the hidden compatibility lifecycle in `src/cli/commands/start.rs`:
  after daemon Ready, the CLI called
  `reconcile_local_user_signing_identity(...)` and printed `user-signer`.
  Direct daemon start, attach-existing-daemon flows, and non-CLI product entry
  points therefore could publish Invocation readiness before the paired User
  signer existed.
- `codegraph impact ensure_user_runtime_signing_identity` after the change
  reports exactly two production consumers: daemon Invocation boot and the
  explicit CLI `auth signing-key register` facade. `start.rs` is no longer a
  lifecycle owner.
- The root abstraction problem was split lifecycle ownership: daemon boot owned
  Device/Hub/_system identities, while CLI start owned paired User signing as a
  post-ready repair. The fix moves key-service ensure and trust-anchor
  registration into daemon Invocation boot, before admission publication.

## 2026-07-20 Plugin realtime transport fallback audit

- `rg fallback_transport` found an active plugin realtime fallback model across
  `PluginRealtimeCapability`, activation readiness, CLI status labels,
  Remote Desktop plugin metadata, and the Docker media bidi E2E fixture.
- `codegraph impact PluginRealtimeCapability` showed the production blast
  radius is bounded to plugin manifest parsing, realtime activation planning,
  broker outcome projection, CLI plugin status, runtime manager contribution,
  and Remote Desktop plugin publication.
- The root abstraction problem was a second transport selection authority:
  plugin manifests declared `transport` and `fallback_transport`, and daemon
  readiness could silently select the fallback and report `FallbackReady`.
  That made WebRTC readiness look cutover-compatible even when WebRTC roles
  were not the executable path.
- After the change, plugin realtime capability has one canonical transport.
  Readiness either selects that declared transport or blocks; there is no
  `FallbackReady` state and no manifest `fallback_transport` field.

## 2026-07-20 PrincipalLifecycle admission convergence audit

- `codegraph explore resolve_federated_key_b64` showed a one-caller blast
  radius from `AdmissionFacade` to `unary_dispatcher`, which proved that fixing
  only `federation.resolve_key` would not cover Axon's LocalRuntime caller
  authentication.
- `codegraph explore PrincipalLifecycleReader` showed the reader was wired into
  admission state checks, but not into the key resolver used by LocalRuntime.
  The live browser HTTP E2E confirmed this: the first failure was
  `CALLER_KEY_NOT_FOUND:same_realm_local_miss` even after backend registration
  logged "PrincipalLifecycle".
- `rg` then identified the real shared authentication port:
  `FederatedKeyResolver` is installed into `CanonicalAdmissionKeyResolver`,
  which is used by Axon's LocalRuntime. Its same-realm local miss path only read
  the trust-anchor projection.
- The root abstraction problem was split authority for same-realm User keys:
  PrincipalLifecycle owned the durable principal/key lifecycle, while
  LocalRuntime authentication and product policy still treated the trust anchor
  projection as the only source of caller truth.
- After the change, `FederatedKeyResolver` owns a late-bound
  `PrincipalLifecycleReader` read model. Transport boot installs the reader into
  the same resolver instance already shared by LocalRuntime admission,
  `AdmissionFacade`, cross-hub routing, and SIGHUP reload. Same-realm User key
  resolution now checks trust-anchor projection first, then the durable
  PrincipalLifecycle aggregate, and only then enters federated resolution.
- The second live failure exposed a matching product-policy fork:
  `AdmissionFacade` resolved `TrustedAgentRole` from `RealmTrustAnchor::lookup`
  only. Product policy now uses the same canonical role model: trust-anchor hit,
  same-realm active PrincipalLifecycle User, then federated caller role.

## 2026-07-20 SDK conformance snapshot toolchain audit

- `check-sdk-conformance-reports.sh` reproduced the cutover evidence gap with
  `SDK_CONFORMANCE_LANGUAGES=go`: the gate wrote
  `source-attestation.json`, but no `go.json`, so
  `check-sdk-parity-matrix.sh` failed with `missing_live_results:go`.
- A retained manual source snapshot showed the only failed runner record was
  `sdk/seven_language_capability_matrix` /
  `TestConformanceSevenLanguageCapabilityMatrix`.
- `codegraph callers runRepositoryGate` confirmed the Go conformance gate
  surface is four repository-gate wrapper tests; `codegraph node
  TestConformanceSevenLanguageCapabilityMatrix` showed the failed selector
  calls `runRepositoryGate(t, "check-sdk-parity-matrix.sh", "--self-test")`.
- The full selector stderr proved the failure was not Go SDK runtime behavior:
  `check-sdk-parity-matrix.sh --self-test` could not find a Python interpreter
  with `pytest` inside the clean source snapshot.
- The root abstraction problem was ambient toolchain authority. The report gate
  treated Python as a language-slice-only dependency, while language-owned
  conformance selectors can execute repository gates whose validators require
  Python. That let ignored local state (`sdk/python/.venv`) decide whether live
  evidence exists.
- After the change, `check-sdk-conformance-reports.sh` resolves the Python
  toolchain with `pytest` once at the report-gate boundary before snapshot
  execution. The selected language no longer controls whether shared
  repository-gate dependencies are available.

## 2026-07-20 SDK conformance toolchain attestation audit

- Full-language `check-sdk-conformance-reports.sh` generated all seven live
  result files, but `check-sdk-parity-matrix.sh` failed on
  `toolchain_attestation_mismatch:node:ability/descriptor_projection`.
- The `node.json` live result recorded `toolchain_version = v24.14.0`, while
  `check-sdk-parity-matrix.sh` sourced `sdk/conformance/toolchain_path.sh`,
  prepended the contract Node path from `toolchains.json`, and computed
  expected Node attestation from `v22.16.0`.
- The root abstraction problem was split toolchain authority: report execution
  used ambient PATH, while parity validation used the canonical SDK toolchain
  resolver. This made live evidence non-replayable even though all Node
  selectors passed.
- After the change, `check-sdk-conformance-reports.sh` sources
  `toolchain_path.sh` and calls `resolve_sdk_toolchain_path` from the shared
  `prepare_report_gate_toolchains()` boundary before snapshot execution. Runner
  records and parity validation now attest the same toolchain contract.

## 2026-07-20 Remote desktop diagnostic transport projection audit

- `codegraph query legacy_label` showed three active methods in
  `plugins/remote-desktop/src/contract.rs`. Their callers projected current
  product session state, backend status, and transport names into public JSON;
  they were not legacy readers or migration tests.
- `rg` showed the same provider emitted `diagnostic_fallback`,
  "diagnostic fallback" user-facing messages, and
  `xcap_snapshot_fallback` in session/capability views. No downstream in-repo
  caller consumed those values, so retaining the fallback vocabulary only made
  the current product contract look like a compatibility path.
- `codegraph query diagnostic_fallback` and repository-wide `rg` confirmed the
  value was produced only by the remote desktop plugin itself. This made the
  change local to the provider boundary.
- The root abstraction problem was projection vocabulary drift: WebRTC,
  InvokeBidi, and preview stream are explicit product transport capabilities.
  They should be modeled as primary or diagnostic transports, not as a
  primary/fallback selection policy. Calling the current JSON projection
  `legacy_label()` also obscured the fact that this is the product-owned JSON
  vocabulary, not a legacy adapter.
- After the change, state/status/transport projection methods are named
  `json_name()`, diagnostic carriers are described as diagnostic transports,
  and the provider no longer emits fallback/legacy terminology for current
  remote desktop session views.

## 2026-07-20 Owner projection cursor pre-v2 migration audit

- `codegraph explore migrate_legacy_schema_unlocked` showed the migration path
  had a single production caller: `load_and_migrate_unlocked` in
  `src/daemon/persistence/owner_projections.rs`. The path was not a shared
  parser or public SDK contract; it existed only at the daemon persistence
  boundary for `~/.easynet/owner-projections.json`.
- `codegraph impact OwnerProjectionCursorFile` showed the real consumers are
  owner projection publication/read-model tests and admission owner-resolution
  tests. Those consumers use schema-v2 cursors with explicit generation and
  lifecycle facts.
- The root abstraction problem was hidden startup compatibility in a cursor
  store that claims to be dumb persistence. A schema-less file was silently
  promoted to v2 with `generation = 1` and `lifecycle = active`, which makes a
  missing lifecycle fact look canonical. That violates the current direction:
  old local state should be cleaned and republished, not normalized into the
  current authority path.
- After the change, the store has one reader, `load_unlocked`, and accepts only
  the current schema version. Missing `schema_version` fails closed with an
  explicit delete-and-republish diagnostic and does not rewrite the file.

## 2026-07-20 EAL agent registry bare-key fallback audit

- `codegraph impact validate_agent_target` showed the production impact surface
  is narrow: `validate_agent_target(...)` feeds `AgentAwareDispatcher::dispatch`
  and then `src/eal/interpreter/mod.rs::dispatch`.
- The active compatibility path lived directly in
  `src/eal/interpreter/dispatch.rs`: when the canonical registry key
  `AgentId::Display` (`default/claude`) missed, default-tenant agent targets
  retried the bare agent name (`claude`).
- The root abstraction problem was a split registry key authority. EAL surface
  shorthand (`claude`) is valid only before parsing; once parsed, runtime
  dispatch must resolve the canonical `tenant/name` identity. Accepting retired
  bare registry rows allowed old local state to affect mission dispatch after
  the registry model had converged.
- After the change, EAL dispatch uses exactly one registry selector:
  `registry.agents.get(&agent_id.to_string())`. Bare default-tenant registry
  rows are unsupported state and fail closed with `not_found` instead of being
  treated as an alias.

## 2026-07-20 Agent run-store workspace-dir helper audit

- `rg` found `workspace_dir(agent_name)` in
  `src/daemon/execution/mission/workspace.rs`, explicitly documented as a
  legacy path helper. Its production caller was
  `src/daemon/execution/mission/run_store.rs::RunDir::create`.
- The dispatch path already validates the registry row's `root_path`, opens
  `AgentDirectory`, verifies `agent.toml::name`, and projects that directory
  before creating a run directory. Calling `RunDir::create(agent_name)` after
  that rebuilt `agents_root()/agent_name`, creating a second directory
  authority next to the registry-owned root.
- The root abstraction problem was responsibility leakage: run persistence
  should persist under the already-validated runtime root, not resolve where an
  agent lives. The latter belongs to `AgentDirectory` and the registry
  aggregate.
- After the change, `RunDir::create` accepts `&Path` for the verified agent
  root, dispatch passes the `root_path` it already validated, and the legacy
  `workspace_dir(agent_name)` helper is deleted.

## 2026-07-20 Local daemon identity fallback audit

- `codegraph impact local_daemon_ura` showed six direct production call sites:
  discover device aggregate, teach descriptor mutation, federation probe,
  federation discover/revoke, and the local daemon gRPC default callee
  resolver.
- The active compatibility path lived in
  `src/daemon/identity/local_invocation.rs::local_daemon_ura`: if
  `control.json` did not publish a daemon identity, it returned
  `local_device_ura()`. That downstream function can derive from device
  credentials and ultimately from the unpaired `default/local` device URA.
- The root abstraction problem was split daemon identity authority. Daemon-local
  invocation targets must bind to the daemon identity published by the running
  process. A missing control-discovery identity is not permission to synthesize
  a device owner and continue constructing signed invocation envelopes.
- After the change, `local_daemon_ura()` returns `anyhow::Result<String>` and
  fails closed when control discovery lacks a daemon identity. Callers now
  propagate the error instead of silently falling through to device/default
  ownership.

## 2026-07-20 Files resource producer-fact and selector fallback audit

- `codegraph impact handle_put` showed a narrow production surface:
  `files_store::register`, local handler tests, and the OpenAI files
  round-trip real-invoke test. The MIME inference helper had no independent
  consumers; it existed only to let `files.put` synthesize a content type from
  `filename`.
- `codegraph impact mime_from_filename` confirmed the active fallback was
  local to `src/daemon/ability/builtins/resources/files_store/handlers.rs`.
  Removing it does not affect SDK builders or public invocation tuple code.
- `rg 'files\.get|handle_get\(|path.*sha256'` showed the retired `{path:
  "<sha256>"}` selector was implemented only inside `files_store::handle_get`.
  Pages still owns its own `{path}` API; OpenAI files retrieve already calls
  `files.get` with `sha256`.
- After the first producer-fact cut, review found `files.get` still sniffed
  bytes to synthesize `content_type`. That was the same abstraction defect
  shifted from write-time to read-time. The final design persists
  `<sha256>.metadata.json` with schema version, sha256, filename,
  content_type, and size.
- `codegraph impact read_metadata` showed the final metadata path is consumed
  by `handle_get`, `handle_list`, `ensure_metadata_compatible`, and
  `handle_put`, plus the same files_store tests and OpenAI files round-trip.
  The `tests/pages_unit.rs::u11_get_returns_detail` entry is a codegraph
  same-symbol-name collision; `rg` confirms Pages uses a separate module.
- The root abstraction problem was a mixed product/resource interface:
  `files.put` was accepting incomplete producer facts and `files.get` accepted
  a Pages-shaped path selector. That made the generic resource surface carry
  product compatibility rules instead of one content-addressed runtime model.
- After the change, `files.put` requires explicit `filename`, `bytes_b64`, and
  `content_type`; `files.get` accepts exactly one canonical selector
  (`sha256` or files resource `ura`); `files.get`/`files.list` read persisted
  producer metadata and fail closed on orphan/corrupt pre-metadata blobs;
  ability manifests declare those schemas instead of accepting arbitrary JSON
  objects.

## 2026-07-20 Federation discover user-scope cutover audit

- `codegraph impact invoke_federation_discover` showed eleven affected symbols:
  the transport helper, `device list`, `runtime status`, `doctor`,
  `daemon::federation::directory_reader`, `agent.discover` federation
  expansion, and `support::platform::remote_device`.
- `codegraph callers invoke_federation_discover` showed five direct callers
  before removal: product device list, product status, doctor, directory reader,
  and remote-device bare-node resolution. All five inherited an unfiltered
  cross-realm directory read by default.
- The root abstraction problem was a shared helper with the wrong semantic
  default: `invoke_federation_discover(None)` meant “full directory” while most
  consumers were product/user operations. That made operator visibility the
  accidental product privacy model.
- After the change, codegraph callers show:
  - `invoke_federation_discover_for_operator_audit` has one caller:
    `read_federated_directory_for_operator_audit`.
  - `read_federated_directory_for_operator_audit` has one caller:
    `easynet federation discover` when `--user-id` is absent.
  - product/status/doctor/remote-device paths call
    `read_federated_directory_for_current_user` or
    `read_federated_directory_for_user`.
- `rg 'invoke_federation_discover\(|invoke_federation_discover_filtered|read_federated_directory\(|read_federated_directory_filtered|backwards-compat with operator|Absent ⇒ unfiltered|None.*operator query path' src tests tools`
  has no matches after migration.

## 2026-07-20 AgentSpec implicit schema removal audit

- `codegraph query AgentSpec` identified the core durable agent spec boundary:
  `AgentSpec::new`, `AgentSpec::from_toml_str`, `AgentSpec::to_toml_string`,
  and the now-removed `effective_schema_version` helper.
- `codegraph impact AgentSpec` showed the production impact concentrated in
  agent registry migration, agent lifecycle materialisation, agent ability
  publish/materialise, mission `AgentDirectory`, and agent ability manifest
  discovery. That is the right ownership surface for an agent.toml schema
  cutover.
- The root abstraction problem was an implicit durable schema: `AgentSpec::new`
  produced TOML with no `schema_version`, while `from_toml_str` treated missing
  `schema_version` as current v1. That made old pre-stamp local files and new
  writer output indistinguishable.
- After the change, `AgentSpec::new` stamps `schema_version =
  CURRENT_SCHEMA_VERSION`, `to_toml_string` validates before serialising,
  `from_toml_str` rejects missing `schema_version`, and the
  `effective_schema_version()` fallback helper is removed.
- Full library testing exposed test-only handwritten `agent.toml` fixtures in
  agent chat/discover/invoke. Those fixtures were migrated to the canonical
  `AgentSpec` writer instead of restoring missing-schema read compatibility.

## 2026-07-20 Plugin status local projection fallback audit

- `codegraph query select_companion_status` and `rg` identified a product
  status fork in `src/cli/commands/groups/plugin.rs`: desktop companion status
  preferred local manager observation whenever daemon status differed, and
  `plugin list` projected an offline local load plan when daemon control was
  unavailable.
- `codegraph query offline_plugin_surface_report` showed the non-companion
  `plugin status` path also used local package-index/load-plan projection
  instead of daemon runtime status.
- The root abstraction problem was mixed authority: package index and desktop
  process manager are configuration/lifecycle inputs, while product-facing
  plugin runtime status must be reported by the daemon plugin control ability
  that sees descriptor publication, runtime registration, and invokability.
- After the change, `plugin list`, non-companion `plugin status`, and desktop
  companion `plugin status` all require daemon plugin control output. Missing
  daemon/control output fails closed with a daemon-required diagnostic. The
  local projection helper and daemon/local companion status selector are
  deleted.
- `codegraph query require_plugin_control_value` now shows the single
  fail-closed helper in the plugin command module; `codegraph query
  offline_plugin_surface_report` returns no results after migration.

## 2026-07-20 Ability catalogue fulfilled_by classifier audit

- `codegraph query fulfilled_by` and `rg` located the product catalogue
  projection in `src/cli/commands/abilities.rs::extract_columns`.
- The root abstraction problem was mixed classification authority: comments
  described `owner_ura` as the authoritative classifier, but the implementation
  let the legacy handler hint `fulfilled_by` override the displayed KIND when
  present. That allowed implementation metadata such as `mcp_proxy`, `shell`,
  or `agent_chat` to replace canonical owner-kind facts (`system`, `hub`,
  `agent`, `user`) in `easynet ability list`.
- After the change, `extract_columns` reads KIND only from `owner_ura`'s URA
  kind. `fulfilled_by` can remain in raw JSON output as handler metadata, but
  it no longer owns product grouping or classification.
- `codegraph query extract_columns_ignores_fulfilled_by_as_kind_classifier`
  reports the new focused regression test.

## 2026-07-20 Device list directory projection fallback audit

- `rg` found the active compatibility path in
  `src/cli/commands/devices.rs::project_directory_entry`: missing `node_id`
  and `agent_ura` were projected as empty strings, missing `status` defaulted
  to `active`, and unknown status rendered as `UNKNOWN`.
- The root abstraction problem was a product read-model repair inside the
  device-list projection. `federation.discover` is the directory authority for
  product device rows; if a directory entry lacks a canonical Device URA,
  matching `node_id`, or supported status, the CLI must not synthesize a
  routable-looking device row that later fails descriptor or namespace
  resolution.
- After the change, `codegraph query project_directory_entry` reports a
  fallible projection returning `anyhow::Result<Value>`, and
  `codegraph query directory_device_state` reports the explicit status
  transition helper. The projection requires canonical Device URA, matching
  `node_id`, and one of `active|stale|draining`.

## 2026-07-20 PrincipalLifecycle anonymous external key audit

- `codegraph query key_id` and `rg` found an incomplete-key projection path in
  `src/cli/commands/groups/principal.rs::resolve_principal_signing_key`.
- The active fallback accepted `--public-key-b64` without a key id by applying
  `source.key_id.unwrap_or_default()`. Downstream request builders then omitted
  `key_id` when the trimmed string was empty.
- The root abstraction problem was mixed custody semantics. Daemon-managed keys
  have daemon-issued key ids; explicit external public-key projections must
  carry the operator's key id as binding evidence. An anonymous public key is
  neither daemon-custodied nor fully explicit.
- After the change, `resolve_principal_signing_key` requires a non-empty key id
  whenever direct public-key material is supplied. `codegraph query
  resolve_principal_signing_key` reports the production resolver and the new
  fail-closed tests for anonymous/blank key ids.

## 2026-07-20 PrincipalLifecycle default proof reference audit

- `codegraph query default_proof_ref` and `rg` found the remaining
  PrincipalLifecycle proof-reference fallback in
  `src/cli/commands/groups/principal.rs`: `principal create` and
  `principal bootstrap` could synthesize `proof:<idempotency_key>` when the
  caller omitted `proof_ref`.
- The root abstraction problem was conflating replay identity with proof
  evidence. `idempotency_key` names a command replay cell; `proof.reference`
  names the proof material authorizing the lifecycle transition. Deriving the
  latter from the former creates a local CLI proof authority outside the
  PrincipalLifecycle model.
- After the change, `BootstrapArgs` and `CreateArgs` require `proof_ref`;
  `required_proof_ref` trims and rejects blank values; `default_proof_ref` and
  `bootstrap_proof_ref` are deleted. `codegraph query default_proof_ref`
  returns no results.

## 2026-07-20 Federation peer inspection empty-topology fallback audit

- `rg` found `read_federated_peers().unwrap_or_default()` and
  `read_trusted_hubs().unwrap_or_default()` in
  `src/cli/commands/federation_peers.rs::run`. Any read or parse failure in
  existing operator config files was projected as an empty peer/trust topology.
- `codegraph query read_federated_peers` and `read_trusted_hubs` showed the
  helper ownership is local to the `federation peers` command, so the migration
  could be made at the operator inspection boundary without changing daemon
  boot/reload cells.
- The root abstraction problem was conflating fresh missing files with invalid
  local authority inputs. Missing daemon-config/realm-trust files are valid
  fresh-install empty state; existing unreadable/malformed files are not proof
  of "no peers" or "no trusted hubs".
- After the change, `run` propagates read/parse errors, production and tests
  share the same parser helpers, malformed `[daemon.federated_peers]` entries
  fail closed, and hub-role trust entries require non-empty `agent_ura`.
- `codegraph query read_federated_peers().unwrap_or_default` returns no results
  after migration. `codegraph query read_federated_peers_from_path` and
  `malformed_daemon_config_fails_closed` identify the new path-level reader and
  fail-closed test.

## 2026-07-20 FFI caller signature key identity fallback audit

- `/Users/macbook.silan.tech/.local/bin/codegraph query caller_signature_key_id_hint`
  reports a single helper in `src/ffi/invocation/mod.rs`, shared by canonical
  invocation JSON and detached `SignatureMaterialJson` parsing.
- The deleted production path accepted missing `key_id_hint` by falling back to
  `signer_public_key_base64`, and then to an empty string. That made FFI a
  second key-identity projection authority instead of requiring the external
  signer boundary to provide explicit identity material.
- After the change, both parser paths call
  `required_string(..., "key_id_hint")`. The remaining
  `signer_public_key_base64` matches in `src/ffi/invocation/mod.rs` are
  negative test fixtures proving public-key material alone is rejected.

## 2026-07-20 Authority tuple empty-identity fallback audit

- `/Users/macbook.silan.tech/.local/bin/codegraph query verify_session_authority_bindings`
  and `verify_delegation_bindings` report the two authority proof comparison
  helpers in `src/daemon/invocation/admission/admission_facade.rs`.
- `rg` found that both helpers previously used `unwrap_or("")` for missing
  envelope caller/subject/callee. `verify_authority_proof_metadata` also used
  `unwrap_or_default()` before deriving proof verification context. Those paths
  did not authorize a missing tuple, but they reclassified malformed canonical
  input as authority mismatch/audience diagnostics.
- After the change, authority proof metadata, session binding, and delegation
  binding all use `caller_ura_required`, `callee_ura_required`, and
  `subject_ura_required`. Missing or blank tuple identities fail at the
  envelope-completeness boundary before authority facts are compared.

## 2026-07-20 Descriptor catalog matched-row schema fallback audit

- `/Users/macbook.silan.tech/.local/bin/codegraph query descriptor_catalog_resolution_from_entries`
  reports the Rust FFI catalog resolver now returning
  `anyhow::Result<Option<Value>>`, so matching rows can fail closed instead of
  being projected with empty descriptor facts.
- `/Users/macbook.silan.tech/.local/bin/codegraph query cabiRequiredCatalogString`
  reports the Go SDK CABI diagnostics row validator.
- `/Users/macbook.silan.tech/.local/bin/codegraph query _required_catalog_entry_string`
  reports the Python SDK CABI diagnostics row validator.
- The root abstraction problem was treating provider-owned descriptor catalog
  rows as loose maps. Once a row matches the requested selector, missing
  `descriptor_ref` or identity fields are provider schema defects, not a
  descriptor miss and not a reason to synthesize empty JSON fields.

## 2026-07-20 Runtime caller signer custody fallback audit

- `rg` found the remote invocation signer resolver in
  `src/daemon/identity/self_identity.rs::load_runtime_caller_signer` and its
  three production callers in
  `src/daemon/invocation/routing/remote_invoke.rs` for unary, stream, and bidi
  remote invocations.
- `codegraph query RuntimeCallerSignerResolver` now reports the single signer
  custody resolver object. It classifies caller owners before touching the
  key-service: User callers use managed subject-bound inventory, while Device,
  Authority, and Agent callers use runtime-owner custody.
- `rg` also found `register_paired_user_runtime_signer()` in
  `src/daemon/boot/invocation/mod.rs` silently returning `Ok(())` when
  `credentials.user_ura()` failed. That preserved a boot-time half-state where
  the daemon could publish Invocation readiness but user-as-caller descriptor
  resolution later failed with missing signer or authority-subject errors.
- After the change, non-principal or invalid caller URAs fail closed at signer
  classification, User callers cannot fall back to owner-key lookup, and
  Device/Both boot requires the paired User URA before registering the managed
  signing key into runtime trust.
