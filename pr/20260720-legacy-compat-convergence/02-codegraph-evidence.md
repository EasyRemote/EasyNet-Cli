# Codegraph evidence

Evidence will be appended after indexing and focused impact queries.

## 2026-07-22 Runtime-owner signer User custody audit

- `/Users/macbook.silan.tech/.local/bin/codegraph status` — PASS; index was
  up to date for the current checkout before this slice.
- `/Users/macbook.silan.tech/.local/bin/codegraph explore
  ensure_managed_signing_key ensure_paired_user_signing ensure_runtime_trust
  sync_paired_user_trust_prelude RuntimeSigningIdentity load_default
  KeyringClient inventory_list` identified `RuntimeSigningIdentity`,
  `RuntimeCallerSignerResolver`, and managed inventory lookup as the signer
  custody seam behind product-facing "requires a caller signer" failures.
- Targeted source inspection showed `start_daemon_invocation_transport` already
  calls `register_paired_user_runtime_signer` before Invocation transport
  assembly, and `ensure_user_runtime_signing_identity` validates purpose,
  `Active` status, exact `bound_subject`, non-empty key id, public key decode,
  and purpose-aware signer policy.
- The remaining root abstraction problem was that
  `RuntimeSigningIdentity::load_default/load` still accepted arbitrary URA
  strings. A misrouted User URA therefore reached the runtime-owner keyring
  lookup and surfaced as `keyring entry not found: <user>`, instead of failing
  in the canonical managed-user custody state.
- After the change, `RuntimeSigningIdentity::load` validates the owner URA as
  Agent, Device, or Authority before `provider.public_key(...)`. User URAs are
  rejected with an explicit managed-user custody error and never touch the
  runtime-owner key-service lookup.

## 2026-07-22 SDK history authority subject expansion audit

- `/Users/macbook.silan.tech/.local/bin/codegraph status` — PASS; index is up
  to date for 1,017 files, 35,242 nodes, and 135,288 edges.
- `/Users/macbook.silan.tech/.local/bin/codegraph explore
  validateSessionHistoryRequest validateSessionHistoryFilterBinding
  _validate_session_history_request _validate_session_history_filter_binding
  sessionAuthorityAdmitsSubject _session_authority_admits_subject` identified
  `sdk/go/authorized_runtime_session.go::validateSessionHistorySessionBinding`
  and
  `sdk/python/easynet_sdk/authorized_runtime_session.py::_validate_session_history_authority_binding`
  as history-specific validators reusing the ordinary owner-aware session
  subject admission helper.
- The same query confirmed the blast radius is limited to authorized runtime
  session history validation plus ordinary runtime ability validation. The
  ordinary runtime ability path still needs owner-aware admission for
  descriptor-bound user resources; the receipt history path needs exact tuple
  binding before provider dispatch.
- Source inspection found the Python authorized-session helper still used
  substring owner checks over `subject_ura`, while Node/Rust already parse
  owner shape for ordinary invocation. The root issue for history was not URA
  parsing alone; it was using an invocation admission predicate for an
  observation capability.
- After the change, Go and Python authorized runtime sessions route
  `invocation.history.list` session authority checks through dedicated
  exact-subject helpers and retain the broader owner-aware helper only for
  non-history runtime invocation/authorization.

## 2026-07-22 Banner runtime projection fallback audit

- `/Users/macbook.silan.tech/.local/bin/codegraph status` reports the checkout
  is not initialized for CodeGraph, so this iteration used targeted `rg` and
  source inspection instead of creating a new index inside the working tree.
- `rg -n
  "config::load\\(\\)\\.ok\\(\\)|render_top_level_banner|write_runtime_status|RuntimeLifecycleService|runtime projection"
  src/cli/presentation src/cli ...` identified
  `src/cli/presentation/banner.rs::write_runtime_status` as the last
  production CLI path directly defaulting runtime projection load failures.
- Source inspection found `let runtime_state = config::load().ok()` driving
  the top-level help banner's daemon row. A malformed `runtime.json` therefore
  rendered as the fresh stopped state instead of unavailable metadata.
- The root abstraction problem was product navigation owning a second runtime
  state machine. The banner cannot return `Result`, but it can still consume
  lifecycle status and render projection-load failure as explicit unavailable
  metadata.
- After the change, the banner maps `RuntimeLifecycleStatus` through
  `BannerDaemonObservation`, renders `metadata unavailable` for lifecycle
  observation errors, and no production `src/` path contains
  `config::load().ok()`.

## 2026-07-22 MCP status runtime projection fallback audit

- `/Users/macbook.silan.tech/.local/bin/codegraph status` reports the checkout
  is not initialized for CodeGraph, so this iteration used targeted `rg` and
  source inspection instead of creating a new index inside the working tree.
- `rg -n "mcp status|run_status\\(|config::load\\(\\)\\.ok\\(\\)|RuntimeLifecycleService"
  src/cli tests tools/scripts/check-architecture-convergence.sh ...`
  identified `src/cli/commands/groups/mcp.rs::run_status` as the remaining
  product diagnostics surface that interpreted `runtime.json` directly.
- Source inspection found `let state = config::load().ok()` feeding both
  reachable and unreachable MCP status rendering. A malformed runtime
  projection therefore became "runtime not running", hiding the lifecycle
  projection error from the operator.
- The root abstraction problem was duplicated diagnostics authority. MCP
  status should report whether the local MCP surface is usable, but lifecycle
  projection classification belongs to `RuntimeLifecycleService`.
- After the change, MCP status obtains `RuntimeLifecycleService::status()?`,
  renders details through `render_lifecycle_details`, and distinguishes
  "projection missing but daemon facts exist" from "runtime not running".

## 2026-07-22 Device reset runtime projection fallback audit

- `/Users/macbook.silan.tech/.local/bin/codegraph status` reports the checkout
  is not initialized for CodeGraph, so this iteration used targeted `rg` and
  source inspection instead of creating a new index inside the working tree.
- `rg -n "config::load\\(\\)\\.ok\\(\\)|RuntimeLifecycleService|runtime projection|runtime_state|runtime\\.json"
  src/cli/presentation src/cli/commands src/daemon/boot/lifecycle ...`
  identified `src/cli/commands/reset.rs` as a destructive product lifecycle
  boundary still reading `runtime.json` directly.
- Source inspection found `let runtime_state = config::load().ok()` feeding
  the active-runtime guard, best-effort revoke decision, and stale
  `runtime.json` cleanup. A malformed projection therefore became "no runtime
  state" and could allow credentials deletion.
- The root abstraction problem was duplicated runtime projection authority.
  `reset` is allowed to consume lifecycle status, but it must not own a
  second optional interpretation of `runtime.json` after the lifecycle module
  has modeled missing/corrupt projection as different states.
- After the change, `reset` consumes `RuntimeLifecycleService::status()?`,
  classifies active runtime states through `reset_runtime_is_active`, cleans
  only `ProjectionPresentProcessMissing` stale projections, and propagates
  cleanup errors.

## 2026-07-22 Runtime lifecycle projection load fallback audit

- `/Users/macbook.silan.tech/.local/bin/codegraph status` reports the checkout
  is not initialized for CodeGraph, so this iteration used targeted `rg` and
  source inspection instead of creating a new index inside the working tree.
- `rg -n
  "RuntimeProjectionStore|\\.load\\(\\).*projection|projection_store|load_current\\(\\)|RuntimeStatusReport::capture|from_parts\\("
  src/daemon/boot/lifecycle src/cli src/ffi tests` identified
  `RuntimeSessionProjection::load_current` and `RuntimeLifecycleService` as
  the lifecycle authority path for `runtime.json`.
- Source inspection found `config::load().ok().map(Self::from_state)`, which
  made missing `runtime.json`, malformed JSON, unreadable files, and any
  other projection load failure indistinguishable from no runtime projection.
- The root abstraction problem was an optional projection modeled as
  `Option<RuntimeSessionProjection>` before the read-model state had been
  classified. Missing projection is a valid lifecycle observation; corrupt or
  unreadable projection is unavailable lifecycle input and must block
  status/start/stop planning.
- After the change, `config::load_optional_runtime_state` owns the
  missing-versus-corrupt split, `RuntimeProjectionStore::load` returns
  `Result<Option<RuntimeSessionProjection>>`, and `RuntimeLifecycleService`
  propagates `ProjectionLoadFailed` through status, start preflight, and stop
  planning.

## 2026-07-22 Pages API body ingress fallback audit

- `/Users/macbook.silan.tech/.local/bin/codegraph status` reports the checkout
  is not initialized for CodeGraph, so this iteration used targeted `rg` and
  source inspection instead of creating a new index inside the working tree.
- `rg -n "unwrap_or\\(Value::Null\\)|unwrap_or_default\\(\\)|\\.ok\\(\\)\\?|\\.ok\\(\\)"
  src/daemon/resources/pages ...` identified
  `src/daemon/resources/pages/pages_listener.rs` as the product-facing HTTP
  ingress where `/api/<verb>` request bodies are converted into invocation
  args.
- Source inspection found `serde_json::from_slice(&body_bytes).unwrap_or(
  serde_json::Value::Null)`, which repaired malformed non-empty JSON into a
  valid `null` body before dispatching to the Pages API handler.
- The root abstraction problem was body-schema defaultization at product
  ingress. Empty body is a declared no-payload state, but malformed body is an
  invalid HTTP input fact and must not be projected as a canonical invocation
  argument.
- After the change, Pages API body parsing is centralized in
  `parse_pages_api_body`: empty bytes return `Value::Null`, non-empty bytes
  are parsed fallibly, and handler errors return HTTP 400 before API ability
  dispatch.

## 2026-07-21 Ability discovery candidate projection fallback audit

- `/Users/macbook.silan.tech/.local/bin/codegraph explore
  "discover skipped_unparseable filter_map ability URA parse dropping rows"`
  identified `Candidate::from_ladder_row`, `project_rows`, and
  `DiscoverExecutionState::extend_candidates` as the product-facing seam where
  runtime ladder rows become `easynet discover` output.
- Targeted source inspection found the compatibility path:
  `project_rows` used `filter_map`, incremented `skipped_unparseable`, and
  still returned a successful report after a minted row carried a malformed
  `qualified_name`. Missing `candidates` arrays also projected as an empty
  candidate list.
- The root abstraction problem was partial-success catalogue projection.
  Discovery ranking may drop valid zero-score rows, but it must not repair a
  corrupt runtime read model into a smaller successful product answer.
- After the change, `Candidate::from_ladder_row` returns a fallible projection
  error for non-canonical minted `qualified_name` and missing minted
  `qualified_name`; `project_rows` requires a `candidates[]` array and
  attaches row index context. `DiscoverReport` no longer exposes the
  `skipped_unparseable` compatibility field.

## 2026-07-21 Ability recording resource read-model projection audit

- `/Users/macbook.silan.tech/.local/bin/codegraph explore
  "ability_record meta.list_resources filter_map resource_ura resource list
  recording"` identified `ability_record::default_resource_ura` as the
  product-facing seam where `meta.list_resources` output becomes the
  invocation `subject_ura` for `mic.subscribe` and camera recording flows.
- Targeted source inspection found the compatibility path:
  `default_resource_ura` filtered rows by `type`, then used `filter_map` over
  `resource_ura`. A matching `mic` / `camera` row that omitted
  `resource_ura`, carried a blank value, or carried a non-Resource URA was
  silently skipped and projected as "no resource is registered".
- The root abstraction problem was treating resource inventory rows as
  optional product hints. `meta.list_resources` is a daemon read model; once a
  row is returned for a requested resource type, its `resource_ura` is the
  authority-bearing invocation subject and must be schema-bound.
- After the change, `select_default_resource_ura` validates every returned row
  through `resource_row_ura`: rows require object shape, matching `type`,
  non-empty `resource_ura`, and canonical `URAKind::Resource`. Empty arrays
  remain the only successful "no resource registered" projection.

## 2026-07-21 Pairing auto-wire credential fact audit

- `/Users/macbook.silan.tech/.local/bin/codegraph explore
  "auto_wire_self_realm_trust_from_credentials
  auto_wire_federated_peer_from_credentials required pairing realm node_id let
  _"` identified `federation_wire` and `runtime start` as the product-facing
  seam where complete pairing facts become local daemon trust and federation
  configuration.
- Targeted source inspection found two legacy compatibility paths:
  `auto_wire_federated_peer_from_credentials` returned `Ok(())` for blank
  `realm`, and `auto_wire_self_realm_trust_from_credentials` returned
  `Ok(())` for blank `realm` or `node_id`. `runtime start` also discarded the
  realm-trust auto-wire result with `let _ = ...`.
- The root abstraction problem was treating malformed pairing credentials as
  an environment no-op. SDK invocation can build canonical tuples, but it
  cannot repair a daemon that was allowed to boot without the local signer,
  subject, and trust facts needed for descriptor-bound admission.
- After the change, both auto-wire helpers require explicit non-empty pairing
  facts before doing any trust/federated peer work, while preserving the
  separate no-local-hub-config no-op when `daemon-config.toml` is absent.
  Runtime start now propagates realm-trust wiring errors before daemon boot.
  The realm-trust writer receives a `PairingTrustFacts` value so inner file
  projection uses the same validated state instead of re-reading raw credential
  strings.

## 2026-07-21 User-device directory projection fallback audit

- `codegraph explore "unwrap_or_default fallback empty route signer authority receipt history device list user devices parse_ura ok"`
  identified `federation_wrappers::handle_list_user_devices` as an active
  product-visible fallback. The handler filtered presence rows by string
  prefix, then projected parse failures or missing device ids into
  `DirectoryEntry { node_id: "", status: "active" }`.
- The same audit found `dispatch_federation_proxy_list_user_devices` accepting
  decoded peer `ListUserDevicesResponse` rows without validating
  `agent_ura`/`node_id` binding, and swallowing fanout errors into logged
  events before returning an empty or partial successful device list.
- The root abstraction problem was false directory authority. Presence rows
  and peer directory responses are runtime projections, not product hints that
  can be repaired locally. Same-realm malformed Device URAs, untrusted selected
  peers, malformed peer rows, and peer fanout failures must remain visible
  runtime facts.
- After the change, `/Users/macbook.silan.tech/.local/bin/codegraph explore
  "dispatch_federation_proxy_list_user_devices selected peer_hub_urls empty success fanout error validate_list_user_devices_response"`
  reports the proxy path using `sorted_non_empty_urls`, explicit selected-peer
  preconditions, `validate_list_user_devices_response`, and a fail-closed
  `peer fanout failed` status before any response merge.

## 2026-07-21 Namespace proxy resolve fallback audit

- `codegraph explore "namespace_proxy_resolve_merge_answer namespace_record_merge_key unwrap_or_default records fanout error empty answer"`
  identified the remaining namespace proxy merge fallback. The dispatcher
  returned an empty resolve answer when a selected peer scope lacked a
  federation client, skipped untrusted selected peers, logged fanout errors,
  and merged whatever peer answers remained.
- The same query showed `namespace_proxy_resolve_merge_answer` skipping peer
  answers with no `records` array and `namespace_record_merge_key` using
  `unwrap_or_default()` for `name`/`record_type`, while also accepting the
  retired camel-case `recordType` field.
- The root abstraction problem was duplicate resolver schema authority. Peer
  `namespace.resolve` output is a canonical resolver projection, not an
  optional product hint. The proxy may merge records, but it must not repair or
  reinterpret malformed peer answers.
- After the change, `/Users/macbook.silan.tech/.local/bin/codegraph explore
  "dispatch_namespace_proxy_resolve selected peer_hub_urls namespace_proxy_resolve_merge_answer namespace_record_merge_key recordType unwrap_or_default fanout_errors"`
  reports the flow `dispatch_namespace_proxy_resolve ->
  validate_namespace_proxy_resolve_peer_answer -> namespace_record_merge_key`.
  Selected-peer failure now aborts before merge, and record merge keys require
  canonical non-empty `name` plus canonical `record_type`.

## 2026-07-20 InvokeBidi receipt payload projection fallback audit

- `rg` found identical down-frame projection logic in
  `src/daemon/invocation/routing/remote_invoke.rs` and
  `src/support/platform/local_daemon_grpc.rs`: `DownPayload::Receipt` decoded
  `receipt.payload` as JSON but wrapped malformed bytes as `{"data_b64": ...}`.
- `codegraph impact LocalBidiFrame --depth 2` showed this projection feeds
  `ability bidi`, local daemon gRPC tuple drain, and remote target bidi drain.
  That made the fallback product-visible rather than an internal transport
  detail.
- The root abstraction problem was duplicated receipt projection authority:
  two transport adapters interpreted receipt payload schema locally and both
  hid malformed receipt facts behind opaque JSON. BinaryChunk payloads are
  allowed to be bytes; receipt payloads are receipt projection facts and must
  fail closed when non-empty bytes are not declared/parseable JSON.
- After the change, `codegraph query project_invoke_bidi_down_frame --limit 40`
  reports one helper in `src/support/platform/local_invoke.rs`, and
  `codegraph callers project_invoke_bidi_down_frame --limit 40` reports the two
  production drains plus focused tests.

## 2026-07-20 Cross-Hub peer envelope subject fallback audit

- `codegraph impact build_peer_envelope --depth 3` reported production impact
  through `PeerInvokeRequest::into_invoke_request`,
  `FederatedKeyResolver::resolve_federated`,
  `UnaryRouteDispatcher::dispatch_federation_proxy_list_user_devices`, and
  `UnaryRouteDispatcher::dispatch_namespace_proxy_resolve`.
- Targeted `rg` found `caller_envelope.cloned().unwrap_or_default()` and
  subject fallback to `target_ura.trim().to_string()` in
  `src/daemon/invocation/admission/peer_envelope_signer.rs`.
- The root abstraction problem was ambiguous peer invocation subject state:
  `None` meant both "fresh daemon-owned peer request" and "forwarded product
  invocation omitted caller envelope". That allowed missing tuple provenance to
  become a target-self subject and surface later as authority/descriptor/route
  failure.
- After the change, `codegraph query PeerInvocationSubject --limit 40`
  reports the explicit `ForwardedCaller` and `ExplicitSubject` variants, and
  `PeerInvokeRequest::new(...)` requires `PeerInvocationSubject<'a>` rather
  than `Option<&Envelope>`.

## 2026-07-20 Plugin wire profile core-only fallback audit

- `codegraph query PluginRuntimeManager --limit 50` identified the daemon
  plugin runtime manager as the owner of default plugin package/load state and
  the shared `AbilityWireRegistry` handle consumed by catalog/transport
  assembly.
- `codegraph query AbilityWireRegistry --limit 50` showed the registry feeds
  `DaemonInvocationService`, `LocalAxonSessionDispatcher`, and bidi dispatch
  route selection. A stale or core-only registry therefore becomes a
  product-visible "route not visible"/`ABILITY_NOT_FOUND` symptom.
- Targeted `rg` found two hidden defaultization points:
  `PluginRuntimeManager::new()` mapped default-state load failure to
  `AbilityWireRegistry::core()`, and invocation transport independently
  attempted `AbilityWireRegistry::load_default_profile()` before warning and
  continuing with core bidi profiles.
- The root abstraction problem was split projection authority. Plugin ability
  catalog rows and plugin bidi wire profiles are two projections of the same
  plugin runtime state. Booting one projection while defaulting the other to
  core-only hides package/sidecar/index failure behind later descriptor/route
  errors.

## 2026-07-20 Context clipboard history fallback audit

- `rg` found `fs::read_to_string(clipboard_log_path()).unwrap_or_default()` in
  `src/daemon/persistence/context_store.rs::remove_clip`.
- The same file's clipboard list/read paths used `let Ok(content) = ... else
  { return Vec::new(); }` and `filter_map(|l| serde_json::from_str(l).ok())`,
  projecting unreadable logs and malformed rows into empty or partially
  repaired clipboard history.
- `codegraph impact clipboard_log_path --depth 2` reported the production
  impact as `append_clip`, `list_clips`, `list_clip_summaries`,
  `clip_image_abs_path`, and `remove_clip`, plus the public
  `context.clipboard.*` ability handlers.
- The root abstraction problem was treating an append-only read model as an
  optional cache. Missing `clipboard.jsonl` is fresh empty state; an existing
  unreadable or malformed log is unavailable/corrupt context state.

## 2026-07-20 API key store parse fallback audit

- `rg` found `toml::from_str(&text).unwrap_or_default()` in
  `src/daemon/ability/builtins/governance/api_key.rs::load_store`.
- `codegraph impact load_store --depth 2` reported the production impact as
  `<user>.api_key.{create,list,revoke}` plus `resolve_token`, which is consumed
  by the OpenAI compatibility bearer-auth path.
- The root abstraction problem was credential lifecycle defaultization:
  missing `api_keys.toml` is a fresh-install empty store, but a malformed
  existing store is unavailable credential authority. Projecting parse failure
  as `ApiKeyStore::default()` makes existing tokens appear unrecognized and can
  allow create to overwrite the corrupt store.

## 2026-07-20 EAL agent registry unavailable-state fallback audit

- `codegraph query load_registry_or_warn` reported the production EAL
  dispatcher helper in `src/eal/interpreter/dispatch.rs`.
- `rg` confirmed `AgentAwareDispatcher::new()` loaded the daemon
  `AgentAggregateRepository` snapshot and, on any load error, printed a warning
  before returning `AgentRegistry::default()`.
- The root abstraction problem was false empty-state projection. EAL dispatch
  needs only the registered-Agent registry projection, not hosted-Agent
  identity state. A missing registry file is a legitimate empty first-run
  registry, but a corrupt/unreadable registry is unavailable runtime state and
  must not become `agent not found` later in the child Invocation path.

## 2026-07-20 Ability catalogue descriptor-ref synthesis audit

- `codegraph query descriptor_ref` reported
  `src/cli/daemon_client/ability_catalog.rs::enrich_descriptor_ref` as a CLI
  facade descriptor-ref owner.
- `rg` confirmed the helper rebuilt `descriptor_ref` from `ability_ura`,
  `version`/`ability_version`, `descriptor_hash`, and `admission_action` when
  the daemon catalogue row omitted the canonical field.
- The root abstraction problem was duplicated descriptor authority:
  `meta.list_abilities` owns the descriptor read model, while the CLI was
  repairing incomplete rows into invokeable-looking rows. That can hide why a
  product route is not visible or why descriptor resolution later fails.
- After the change, `codegraph query enrich_descriptor_ref` reports no
  results. `AbilityCatalogueClient::abilities_from_value` delegates each row
  to `schema_bound_catalogue_entry`, which requires an object row and a
  canonical `descriptor_ref` supplied by the daemon read model.

## 2026-07-20 Local daemon loopback subject fallback audit

- `codegraph query invocation.history.list`, `codegraph query
  meta.list_abilities`, and `codegraph query browser.open_session` traced the
  user-visible failing surfaces back to local daemon Invocation ingress and
  device/receipt descriptor routes.
- `rg` on `invoke_local_ability_with_subject` and
  `LocalDaemonLoopbackSubjectPolicy` found the remaining root fallback inside
  `src/support/platform/local_daemon_grpc.rs`: `subject: Option<String>` was
  normalized by `explicit_or_target_self(None) -> TargetSelf`, and `TargetSelf`
  resolved to the selected callee URA.
- The root abstraction problem was ambiguous transport state. `None` meant
  "daemon-local root call" for simple CLI commands and also "caller forgot the
  subject" for product/public tuple paths. That allows product ingress defects
  to surface later as admission, descriptor, or timeout errors instead of as
  incomplete tuple construction.
- After the change, local loopback subject state is a closed enum with
  `LocalDaemonSelf` and `Explicit`. `invoke_local_ability(...)` selects the
  daemon-self root policy explicitly, while
  `invoke_local_ability_with_subject(...)` and timeout variants require a
  non-empty `subject_ura`.

## 2026-07-20 Ability target tuple default fallback audit

- `codegraph query AbilityTargetRequest` identified the Python public
  target-invocation facade as the owner of descriptor-ref and ability-URA
  selectors: `resolve_target()`, `build_target_invocation()`,
  `prepare_target()`, and target stream/bidi helpers all route through the same
  `AbilityTargetRequest` model.
- `codegraph query buildWithCallMode` identified the matching Go private
  builder seam in `sdk/go/runtime_ability.go`.
- Targeted `rg` found two compatibility defaults at the provider boundary:
  Python derived `subject_ura` from the selected ability projection when the
  request omitted it, while Go converted a blank target `call_mode` into
  `"rpc"`.
- The root abstraction problem was mixed selector authority: ability
  selector/projection facts were being reused as invocation-subject facts, and
  missing lifecycle state was being normalized at the provider seam instead of
  rejected as incomplete tuple input.
- After the change, Python requires explicit `AbilityTargetRequest.subject_ura`
  before resolving descriptor refs or building drafts, projects the selected
  ability once, and passes that explicit subject through descriptor resolution
  and draft construction. Go rejects blank `call_mode` before descriptor
  resolution rather than defaulting it to RPC.

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

## 2026-07-20 Authority metadata clock fallback audit

- `codegraph query AuthorityMetadata --limit 40` identified the typed SDK
  authority metadata surfaces and the daemon authority metadata core. Focused
  `rg` found
  `SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default()` inside
  `project_admitted_session_authority(...)`.
- The only production caller is `src/daemon/ability/dispatch.rs`, where
  admitted Axon request metadata is projected into `EnvelopeContext` for
  LocalRuntime handlers. That makes clock defaulting a post-admission runtime
  projection fallback, not a harmless formatting detail.
- The root abstraction problem was implicit lifecycle state: an unavailable
  authority clock was being converted into Unix epoch zero before expiry
  validation. Session authority projection now has a named
  `AUTHORITY_CLOCK_UNAVAILABLE` state and fails closed.

## 2026-07-20 Invocation signer custody fallback audit

- `codegraph query strict_identity --limit 40` and focused `rg` found
  `strict_identity(caller_ura).ok()` inside
  `KeyServiceReceiptAuthorityProvider::resolve(...)` for invocation signing.
- The receipt-signing provider already has two custody authorities:
  self-signed owner authority and hosted-agent signing leases. The fallback
  allowed a raw signer capability entry to become invocation authority by
  constructing an `AgentIdentity` directly from `caller_ura`.
- The root abstraction problem was conflating key custody with invocation
  authority. A signer key is not an ownership/admission fact by itself; the
  provider must project invocation signing only from self-signed ownership or a
  valid hosted-agent lease.
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

## 2026-07-20 Node authority binding preflight audit

- `codegraph query InvocationAuthorityBindingValidator --limit 30` reports the
  cohesive Node Runtime Core authority-binding validator object in
  `sdk/node/index.js`, with separate delegation/session validation methods.
- `codegraph query validateInvocationAuthorityBinding --limit 30` reports the
  thin `InvocationDraft` entrypoint into that validator.
- `codegraph query sessionAuthorityAdmitsSubject --limit 30` reports the Node
  session subject-admission helper mirroring daemon semantics: exact subject
  match, or resource ownership by `user.<session_owner>` /
  `agent.<session_owner>.<agent_id>`.
- `codegraph query abilityViewForInvocation --limit 30` reports the ability
  view used to match authority scopes against public name, canonical Ability
  URA, and descriptor wire form.
- The root abstraction problem was treating Node authority metadata as
  shape-only metadata. That let products build a descriptor-bound Device
  invocation carrying a User session authority, so deterministic
  `AUTHORITY_SUBJECT_MISMATCH` surfaced only after daemon admission.

## 2026-07-20 Node type-test product symbol fallback audit

- `codegraph query productSymbols --limit 30` reports product-neutrality
  assertion lists in `sdk/node/test/runtime-core.test.mjs` and
  `sdk/node/test/types.test.ts`; the type test now asserts absence from both
  runtime exports and `index.d.ts`.
- `codegraph query RuntimeTransport --limit 20` reports the generic
  `RuntimeTransport` interface in `sdk/node/index.d.ts`, which remains the type
  test's transport seam instead of any product client.
- `rg` for runtime imports/usages of removed product symbols and
  `opaque-authority` in `sdk/node/test/types.test.ts`, `sdk/node/index.js`,
  and `sdk/node/index.d.ts` returns no matches. The previous static
  `import { AdminClient }` failed during ESM loading before the neutrality
  assertion could run, and the opaque authority value depended on retired
  shape-only authority semantics.

## 2026-07-20 Authorized history subject-binding bypass audit

- `codegraph query invocation.history.list --limit 40` identified
  `SessionHistoryOperations` in the Go and Python SDKs as the product-facing
  high-level history entrypoint over the generic receipt provider.
- `rg` confirmed `SessionHistoryOperations.List/list` previously delegated a
  caller-supplied `ReceiptListRequest` directly to the receipt provider,
  leaving products free to supply a Device subject with a User session
  authority. That made deterministic `AUTHORITY_SUBJECT_MISMATCH` surface only
  at daemon admission or product UI error handling.
- The root abstraction problem was a state-machine bypass inside
  `AuthorizedRuntimeSession`: the `history` operation group existed, but it
  did not enforce the same caller/callee/subject/authority binding invariant
  that `invoke` enforces before dispatch.
- After the change, `codegraph query validateSessionHistoryRuntimeCall --limit
  20` reports the Go SDK preflight gate and `codegraph query
  _validate_session_history_call --limit 20` reports the matching Python SDK
  gate. Both validate complete `RuntimeCallContext` input, require typed or
  canonical metadata authority, and reject delegation/session subject mismatch
  before the receipt provider is called.

## 2026-07-20 Descriptor resolver signer-missing error downgrade audit

- `rg` found the C ABI descriptor resolver projecting
  `requires a caller signer` as canonical JSON code
  `CALLER_SIGNER_UNAVAILABLE` while still returning numeric `ERR_NOT_FOUND`.
  C callers or wrappers that fell back to numeric ABI classification could
  display the signer/key-service failure as `ABILITY_NOT_FOUND`.
- The root abstraction problem was mixing route absence and caller identity
  custody absence under one ABI miss code. A missing caller signer is an
  identity/authority precondition failure, not a descriptor or ability miss.
- After the change, the resolver returns `ERR_PERMISSION_DENIED` with the same
  precise canonical projection (`CALLER_SIGNER_UNAVAILABLE`,
  `stage=caller_identity`, `retry=never`), so older ABI classification no
  longer preserves the false ability-not-found story.

## 2026-07-20 Federation resolve-key empty-key fallback audit

- `codegraph callers resolve_key_response --limit 20` identified
  `handle_resolve_key` as the production builder owner for
  `federation.resolve_key` responses, with dispatcher fallback constructing
  the same wire response after federated key-provider lookup.
- `rg` found `BASE64_STANDARD.decode(public_key_b64.as_bytes()).map(hex::encode).unwrap_or_default()`
  in `src/daemon/invocation/dispatch/federation_wrappers.rs`, which projected
  invalid trust-anchor key material into `public_key_hex: ""`.
- The root abstraction problem was trust material defaultization: a missing
  trust-anchor entry is a route/trust-set miss, but an existing entry with
  malformed or non-Ed25519 key material is corrupt authority state. Returning
  an empty hex field preserves a valid JSON shape while deferring the real
  failure into later admission, descriptor, or product UI paths.
- After the change, `resolve_key_response(...)` validates base64 and requires
  exactly 32 decoded bytes before producing the canonical response. `None`
  remains reserved for “not in trust set”; invalid key material is a typed
  resolver error that the dispatcher maps to `FailedPrecondition`.

## 2026-07-20 Plugin sidecar stderr diagnostic fallback audit

- `codegraph query sidecar --limit 50` identified
  `src/daemon/plugins/sidecar/io.rs::collect_stderr` as the common diagnostic
  capture path for unary, stream, and bidi sidecar execution.
- `rg` found the active fallback in `spawn_stderr_reader` and
  `collect_stderr`: stderr was read through UTF-8-only `read_to_string`, the
  read result was ignored, and join failure was collapsed through
  `unwrap_or_default()`.
- The root abstraction problem was treating plugin stderr as best-effort debug
  output. In the product plugin loop, stderr is failure evidence for process
  exits, timeouts, and protocol failures. Binary stderr, read errors, and
  reader panics must remain operator-visible diagnostics instead of becoming an
  empty string.
- After the change, `capture_stderr_diagnostics(...)` captures bytes, preserves
  binary data with `String::from_utf8_lossy`, appends explicit read-failure
  diagnostics, and `collect_stderr(...)` reports reader panic explicitly. Rule
  `R72_PLUGIN_SIDECAR_STDERR_DIAGNOSTICS` now rejects the old empty diagnostic
  fallback.

## 2026-07-20 Trust key resolver corrupt user-key skip audit

- `codegraph callers decode_pubkey --limit 50` identified
  `RealmTrustAnchorKeyResolver::resolve_all` as the multi-key user bucket
  adapter feeding Axon's invocation `KeyResolver`.
- `rg` found the production fallback in
  `src/daemon/trust/key_resolver.rs`: user bucket rows were decoded through
  `.filter_map(|row| decode_pubkey(&row.public_key_b64, agent_ura).ok())`,
  which silently skipped corrupt key material when at least one valid user key
  remained.
- The root abstraction problem was partial authority projection. A DEC-EU user
  bucket is one trust snapshot for a principal; corrupt persisted key material
  is corrupt authority state, not an optional row. Skipping the row lets a
  damaged trust anchor continue admitting calls with the remaining keys and
  pushes operator-visible corruption into later, harder-to-diagnose failures.
- After the change, `resolve_all` propagates `decode_pubkey` errors with `?`.
  Any corrupt user key inside the verifier-bounded bucket fails the user
  principal closed. Rule `R67_TRUST_ANCHOR_USER_BUCKET_LOOKUP_FORK` now also
  rejects `filter_map(...decode_pubkey(...).ok())` in the key resolver.

## 2026-07-20 Device trust sync resolve-key schema fallback audit

- `codegraph query ResolvedCallerTrust --limit 50` identified
  `parse_resolved_caller_trust` as the parser for hub-attested
  `federation.resolve_key` responses used by device on-miss trust sync.
- `rg` found two active fallback paths in the parser: malformed
  `public_keys_b64` rows were skipped with `filter_map`, and missing or empty
  multi-key arrays were repaired from legacy `public_key_b64`.
- The root abstraction problem was response-shape compatibility inside an
  authority import path. Device trust sync imports hub-attested caller keys into
  the local trust anchor; it must consume schema-bound trust evidence, not
  rebuild missing multi-key evidence from older single-key fields or silently
  discard malformed rows.
- After the change, `public_keys_b64` is required and must be an array of
  non-empty strings. An empty array remains the explicit hub miss used by
  negative caching. Missing arrays, non-arrays, non-string rows, and empty rows
  fail parsing before any trust import. Rule
  `R73_DEVICE_TRUST_SYNC_RESOLVE_KEY_SCHEMA` rejects the old repair path.

## 2026-07-20 Pages serve fetch projection fallback audit

- `codegraph query pages_serve_ability --limit 50` identified
  `src/daemon/resources/pages/pages_serve_ability.rs::serve_bytes` as the
  HTTP Pages adapter over the `<user>.<project>.page.fetch` resource ability.
- `rg` found the active fallback in `bytes_from_value`: missing `bytes_b64`
  became `""`, invalid base64 decoded to `Vec::new()`, missing
  `content_type` became `application/octet-stream`, missing
  `force_attachment` became `false`, and missing `sha256` became `""`.
- The root abstraction problem was treating fetch output as best-effort HTTP
  framing. Pages fetch output is resource evidence: decoded bytes, content
  type, attachment disposition, and sha256 must be the exact projection from
  the resource ability. Returning HTTP 200 with empty/defaulted fields hides
  corrupted resource output and makes browser/product failures hard to
  diagnose.
- After the change, `bytes_from_value(...)` is a fallible schema-bound parser:
  it requires `bytes_b64`, `content_type`, `force_attachment`, and `sha256`,
  rejects invalid base64, and verifies the sha256 against decoded bytes. Bad
  fetch projections map to HTTP 502 instead of HTTP 200. Rule
  `R74_PAGES_SERVE_FETCH_PROJECTION_SCHEMA` rejects the old defaulting path.

## 2026-07-20 Ability catalogue authority-context fallback audit

- Targeted `rg` found
  `src/daemon/ability/catalog/build.rs::build_registry_with_services_result_inner`
  using `authority_context.unwrap_or_default()` after accepting
  `RegistryBuildConfig.authority_context: Option<AbilityAuthorityContext>`.
- `codegraph query RegistryBuildConfig --limit 30` and
  `codegraph query RegistryDaemonBuildConfig --limit 20` identified the two
  assembly config boundaries that feed deterministic snapshots, daemon boot,
  and assembly tests.
- The root abstraction problem was optional authority lifecycle state in the
  catalogue owner. A missing authority context should not be a valid assembly
  state because the default observes local environment and can publish Device
  or Hub owner planes the caller did not explicitly select.
- After the change, both build configs carry concrete `AbilityAuthorityContext`.
  Constructors still provide intentional defaults at the caller boundary, but
  the assembly core no longer repairs absent authority state. Daemon boot now
  constructs a concrete mode-specific context before loading agent registry or
  receipt authority.

## 2026-07-20 Ability publication projection silent-drop audit

- `codegraph query "owner_projection_values resolved_owner_projection_values
  fail closed ability publication" --limit 80` identified the two production
  projection seams: `LocalAbilityPublicationSnapshot::owner_projection_values`
  and `federation_wrappers::resolved_owner_projection_values`.
- Targeted `rg` found the old local publication path converting
  `summary_from_descriptor(...)` and `serde_json::to_value(...)` failures into
  skipped rows through `filter_map(... .ok())`.
- The same merge path inside `federation.resolve` parsed JSON summaries with an
  `Option` chain and returned from the push closure on invalid rows. That made
  corrupt local publication or corrupt ability summaries appear as normal empty
  route visibility to product callers.
- The root abstraction problem was treating route/catalog evidence as a lossy
  render cache. Ability summaries are descriptor/read-model facts used by
  namespace resolution, device inspection, and product route visibility. A
  corrupt row is unavailable catalogue state, not an absent ability.
- After the change, local publication projection returns
  `Result<Vec<Value>, String>`, `federation.resolve` returns
  `Result<ResolveResponse, String>`, and route resolver/discover/`node.describe`
  callers surface projection failure as unavailable/refused state instead of
  falling through to empty abilities.

## 2026-07-20 Desktop companion status projection silent-drop audit

- `codegraph query "desktop_companion_statuses status_json companion_error
  PluginSurfaceProjector" --limit 80` identified two production consumers of
  companion status projection:
  `daemon::boot::lifecycle::status::desktop_companion_statuses` and
  `PluginSurfaceProjector::project_packages_with_daemon`.
- Targeted `rg` found the runtime-status path returning `Vec::new()` when
  plugin default state could not be loaded, then using
  `filter_map(|package| manager.status_json(package).ok())` to skip broken
  companion DTO projection. The plugin package surface used the same
  `status_json(package).ok()` pattern when rendering package rows.
- The root abstraction problem was treating desktop companion status as an
  optional decoration. Runtime status and plugin package list are operator
  read-model surfaces: companion manager/index/projection failure is a runtime
  observation and must remain visible, not disappear as "no companion".
- After the change, runtime status uses
  `DesktopCompanionStatusObservation` to carry both companion DTOs and
  projection errors, `RuntimeStatusReport` exposes
  `desktop_companion_errors`, `PluginPackageSurfaceRecord` exposes
  `companion_error`, and `DesktopCompanionManager::status_json` preserves the
  concrete serialization/projection error source.

## 2026-07-20 Curator owner catalog registry projection fallback audit

- `codegraph query "collect_owner_catalog validate_curated_manifest
  CatalogEntry curator no catalog available" --limit 100` identified
  `src/daemon/ability/builtins/automation/think.rs::collect_owner_catalog`
  as the single owner-catalog acquisition path for the mission.think curator
  prompt and authored ability validation.
- Targeted `rg` found the active compatibility behavior documented in the
  function header: registry projection load failure returned `Vec::new()`, so
  unreadable/corrupt Agent registry state looked identical to a fresh owner
  with no published abilities.
- The root abstraction problem was treating the owner ability catalog as a
  best-effort prompt hint. For team-scope curation, this catalog is authoring
  authority: it constrains the curator's EAL references and validates the
  authored manifest before publish. A corrupt registry projection is
  unavailable catalog state, not an empty catalog.
- After the change, `collect_owner_catalog` returns
  `Result<Vec<CatalogEntry>, String>`. Missing owner rows still produce an
  empty first-run catalog, but registry projection failures become
  `owner ability catalog unavailable` and mission.think records
  `curator.stage = "catalog"` without asking the curator to author against a
  false empty catalog.

## 2026-07-20 Schedule due-selection silent empty fallback audit

- `codegraph query "ScheduleService due DueFire schedule tick cache lock
  poisoned" --limit 100` identified `src/daemon/execution/schedule/mod.rs`
  as the due-selection authority consumed by
  `src/bin/easynet-daemon.rs::spawn_schedule_tick`.
- `codegraph callers due --limit 100` showed the production tick runner plus
  focused schedule tests as the only callers, making the migration to a
  fallible due-selection surface bounded.
- Targeted source inspection found the compatibility seam:
  `ScheduleService::due` returned `Vec<DueFire>`, mapped cache lock poisoning
  to `Vec::new()`, and skipped enabled rows with invalid cron expressions.
- The root abstraction problem was treating schedule due selection as a
  best-effort cache read. For the tick lifecycle, a poisoned cache or corrupt
  enabled schedule is unavailable runtime state; it is not proof that no
  schedule is due.
- After the change, `ScheduleService::due` returns
  `anyhow::Result<Vec<DueFire>>`, preserves poisoned cache and invalid cron
  rows as explicit errors, and the tick runner logs `due selection failed`
  before skipping only the current tick.

## 2026-07-20 Schedule snapshot empty read-model fallback audit

- Follow-up source inspection on the same `ScheduleService` state machine found
  `ScheduleService::list` still returning `Vec<ScheduleEntry>` and mapping
  cache lock poisoning to `Vec::new()`.
- `codegraph query "ScheduleService list schedule.list handler context loader
  cache lock poisoned" --limit 100` identified the product-visible consumers:
  `Kernel::list_schedules`, the `schedule.list` ability handler, the daemon
  schedule tick runner, and the schedule context loader.
- The root abstraction problem was inconsistent snapshot ownership: due
  selection had become fail-closed, but the public schedule read-model still
  treated unreadable lifecycle state as an empty schedule catalog. That would
  make products believe no schedules exist even when the runtime cannot prove
  the schedule state.
- After the change, `ScheduleService::list` returns
  `anyhow::Result<Vec<ScheduleEntry>>`; Kernel, `schedule.list`, context
  loader, and tick runner propagate or log snapshot failure; and `schedule.list`
  no longer serializes failed rows as `null`.

## 2026-07-20 Schedule context next-fire fallback audit

- `codegraph query "ScheduleLoader next_fire_after Err continue schedule
  context corrupt cron" --limit 100` identified the remaining context-loader
  fallback: `ScheduleLoader::load` called `next_fire_after(&entry.id, now)` for
  entries already obtained from a coherent snapshot and collapsed
  `Ok(None) | Err(_)` into `continue`.
- Targeted source inspection showed this preserved a second interpretation path
  for schedule lifecycle state: `schedule.list` and `due` were fail-closed, but
  chat context could still hide corrupt cron/read-model errors as no upcoming
  schedule context.
- The root abstraction problem was re-querying mutable schedule state after
  obtaining a snapshot entry. That forced the loader to classify lookup races
  and corrupt cron through one `anyhow::Error` channel and encouraged an
  `Err(_) => continue` fallback.
- After the change, schedule core exposes
  `ScheduleService::next_fire_for_entry`, backed by shared `parse_entry_cron`.
  `ScheduleLoader` computes next fires from the snapshot entry and propagates
  corrupt cron errors instead of emitting empty context.

## 2026-07-20 Session index silent empty/unknown fallback audit

- `codegraph query "SessionService list lock poisoned Vec::new session.events
  loop instance discuss room" --limit 120` identified `SessionService` plus
  the product-facing `device.session.list` / `device.session.attach` handlers
  as live-session read-model consumers.
- Targeted source inspection found two compatibility projections:
  `SessionService::list_active` returned `Vec<Session>` and mapped lock poison
  to `Vec::new()`, while `SessionService::get` erased lock poison with
  `.read().ok()` and returned `None`.
- The root abstraction problem was conflating unknown session with unavailable
  session index state. Unknown session can remain an empty attach snapshot for
  stale IDs; poisoned session index must be explicit unavailable runtime state.
- After the change, `list_active` returns `anyhow::Result<Vec<Session>>`, `get`
  returns `anyhow::Result<Option<Session>>`, Kernel propagates both results,
  `device.session.list` preserves serialization failures, and
  `device.session.attach` only emits the empty snapshot after proving the index
  is readable.

## 2026-07-20 Discuss room registry silent empty fallback audit

- `codegraph query "DiscussService list lock poisoned Vec::new
  list_discuss_rooms discuss.list LoopService list lock poisoned" --limit 140`
  identified `src/daemon/execution/mission/discuss/mod.rs` as the room registry
  snapshot authority and `Kernel::list_discuss_rooms` as the product-facing
  boundary.
- Targeted source inspection found the compatibility seam:
  `DiscussService::list` returned `Vec<DiscussRoom>` and mapped a poisoned room
  registry read lock to `Vec::new()`.
- The root abstraction problem was treating room registry availability as
  best-effort room discovery. An empty room list is valid only after the runtime
  proves the registry is readable; an unreadable registry is unavailable runtime
  state.
- After the change, `DiscussService::list` returns
  `anyhow::Result<Vec<DiscussRoom>>`, Kernel returns that result directly, and a
  poisoned registry fails explicitly instead of projecting false empty room
  state.

## 2026-07-20 Loop cache silent empty/unknown fallback audit

- `codegraph query "LoopService list lock poisoned Vec::new loop_status
  list_loops Kernel loop list" --limit 140` identified
  `src/daemon/execution/loop_instance/mod.rs` as the loop lifecycle cache
  authority and `Kernel::loop_status` / `loop.status` / `loop.subscribe` as
  product-facing consumers.
- Targeted source inspection found two compatibility projections:
  `LoopService::status` erased cache lock poisoning through `.read().ok()` and
  returned `None`, while `LoopService::list` mapped lock poisoning to
  `Vec::new()`.
- The root abstraction problem was conflating unknown loop id with unavailable
  loop cache state. Unknown loop can remain a not-found error for stale ids;
  poisoned cache means the lifecycle state is unavailable and must not be
  classified as absent loop state.
- After the change, `LoopService::status` returns
  `anyhow::Result<Option<LoopInstance>>`, `LoopService::list` returns
  `anyhow::Result<Vec<LoopInstance>>`, resume/subscribe/status callers
  propagate cache failures, and Debug no longer projects unreadable cache state
  as zero loops.

## 2026-07-20 Chat cross-agent registry silent empty fallback audit

- `codegraph query "agents chat Err return Vec new registry projection
  candidates list agents chat fallback" --limit 120` identified
  `src/daemon/ability/builtins/agents/chat.rs::enumerate_other_agent_specs`
  as the cross-agent ability discovery authority used by both RPC chat and
  stream chat context construction.
- Targeted source inspection found the compatibility seam:
  `enumerate_other_agent_specs` called `AgentAggregateRepository::load_snapshot`
  and mapped every load failure to `Vec::new()`.
- The root abstraction problem was treating Agent aggregate projection
  availability as a best-effort prompt hint. A valid empty peer-ability list is
  only true after the registry projection is readable; an unreadable projection
  means chat cannot prove cross-agent route/context visibility.
- After the change, `enumerate_other_agent_specs` returns
  `anyhow::Result<Vec<AgentAbilitySpec>>`, classifies unreadable aggregate state
  as `load cross-agent ability registry projection`, and both RPC and stream
  chat propagate the failure before dispatching any LLM turn.

## 2026-07-21 Permission pending queue silent empty fallback audit

- `codegraph query "SubscriberBroker pending_snapshot pending queue lock
  poisoned consent list_pending subscribe PermissionService pending" --limit
  80` identified `SubscriberBroker::pending_snapshot`,
  `PermissionService::pending`, `consent.list_pending`, and
  `consent.subscribe` as one admission/operator read-model chain.
- Targeted source inspection found the compatibility seam:
  `SubscriberBroker::pending_snapshot` erased a poisoned pending queue through
  `.read().ok()` and `unwrap_or_default()`, and `PermissionService::pending`
  preserved that false-empty projection for Kernel and consent surfaces.
- The root abstraction problem was treating pending consent queue availability
  as an optional UI snapshot. An absent subscriber broker can mean the
  AllowAll policy has no queue; a present but unreadable pending queue is
  unavailable admission state and must not be projected as "no pending
  permissions."
- After the change, `SubscriberBroker::pending_snapshot` returns
  `anyhow::Result<Vec<PermissionRequest>>`, `PermissionService::pending`
  preserves the AllowAll empty state while propagating subscriber failures, and
  `Kernel::pending_permission_requests`, `consent.list_pending`, and
  `consent.subscribe` all fail closed on unreadable pending state.

## 2026-07-21 Authority metadata all-zero principal placeholder audit

- `codegraph query "00000000 session_owner_user_id invocation_history active
  profile subject authority subject RemoteSystemInvocationIssuer caller signer
  bootstrap self identity" --limit 120` identified daemon authority metadata,
  SDK authority builders, `RemoteSystemInvocationIssuer::root_plan`, and
  caller signer resolution as the chain behind product-visible
  `AUTHORITY_SUBJECT_MISMATCH` / missing signer failures.
- Targeted source inspection found the compatibility seam: SDKs already reject
  all-zero principal ids, and daemon credentials reject all-zero `user_id`, but
  raw daemon authority metadata validation only required
  `session_owner_user_id` to be non-empty. A product ingress that hand-built
  session authority metadata could therefore submit
  `00000000-0000-0000-0000-000000000000` and reach later admission as a
  misleading subject mismatch.
- The root abstraction problem was placing identity-placeholder rejection at
  language facade boundaries instead of the runtime admission boundary. SDK
  builders can protect SDK callers, but daemon admission owns canonical raw
  metadata validation for every product ingress.
- After the change, `validate_delegation_payload_shape` and
  `validate_session_authority_payload_shape` both call one centralized
  all-zero principal rejection helper before subject/audience matching. Raw
  all-zero authority metadata now fails as `AUTHORITY_FORMAT_INVALID` instead
  of reaching downstream admission as a false user/session authority.

## 2026-07-21 C ABI descriptor diagnostics generic not-found fallback audit

- `/Users/macbook.silan.tech/.local/bin/codegraph explore
  resolveDescriptorRefFromDiagnostics _resolve_descriptor_ref_from_diagnostics
  descriptor_resolution_error_projection` identified the descriptor resolution
  boundary across Rust FFI, Go C ABI provider, and Python C ABI provider.
- Rust FFI already projects descriptor resolution failures into typed runtime
  states: caller signer absence becomes `CALLER_SIGNER_UNAVAILABLE`, offline
  owners become `DESCRIPTOR_OWNER_OFFLINE`, and unresolved descriptor refs
  become `DESCRIPTOR_NOT_FOUND`.
- Targeted source inspection found the remaining compatibility seam in the Go
  and Python diagnostics fallback providers: when they could not match a
  descriptor catalog row, they returned generic `NOT_FOUND`. That erased
  descriptor-resolution semantics and let product callers conflate catalog
  misses with missing abilities or upstream signer/admission failures.
- The root abstraction problem was placing a descriptor lookup fallback inside
  generic ability-not-found semantics. Diagnostics fallback still implements
  descriptor resolution; therefore its terminal miss state must be
  descriptor-specific and consistent across SDK languages.
- After the change, Go and Python C ABI diagnostics fallback paths return
  `DESCRIPTOR_NOT_FOUND`, focused regressions cover owner-mismatch catalog
  misses, and architecture rule R91 prevents reintroducing generic
  `NOT_FOUND` in this boundary.

## 2026-07-21 Invocation attempt audit disabled compatibility audit

- `/Users/macbook.silan.tech/.local/bin/codegraph explore
  InvocationAttemptLedger InvocationAttemptHandle invocation.history.list
  missing_invocation_attempt_ledger invocation_attempt_audit_status` identified
  the pre-runtime failure path across `DaemonInvocationService`,
  `InvocationAttemptLedger`, and `invocation.history.list`.
- Targeted source inspection found three compatibility seams in the current
  WIP: the service could create a disabled attempt handle when no ledger was
  wired, boot could continue after attempt ledger open failure, and the JSONL
  ledger skipped corrupt rows / ignored append failures.
- The root abstraction problem was treating pre-runtime invocation attempts as
  optional logging. Product history uses this layer to explain failures that
  occur before Axon mints a canonical invocation id; an unavailable attempt
  ledger is therefore unavailable runtime observability, not a harmless missing
  diagnostic.
- After the change, production boot opens the attempt ledger as a required
  dependency, transport entry begins fail closed when audit is not wired or
  cannot append, and corrupt attempt ledger rows make the history read model
  fail instead of producing a false partial list.

## 2026-07-21 Session prelude resolve_key schema and paired user key pinning audit

- `/Users/macbook.silan.tech/.local/bin/codegraph explore
  sync_paired_user_trust_prelude paired_user_resolve_key_args
  publish_paired_user_keys_prelude resolved_public_keys federation.resolve_key
  presented_pubkey_b64` identified the session prelude trust-sync boundary.
  The concrete blast radius was intentionally small: `resolved_public_keys`
  feeds only the realm hub trust import and paired user trust import paths in
  `session_initiator/prelude.rs`.
- Targeted source inspection found the compatibility seam: the prelude parser
  still repaired legacy `public_key_b64`, skipped malformed rows with
  `filter_map`, collapsed malformed JSON through `.ok()` and
  `unwrap_or_default()`, and the paired-user resolve request could be sent with
  only `agent_ura` after publishing local key material.
- The root abstraction problem was treating `federation.resolve_key` as a
  tolerant discovery payload. In the session prelude it is trust evidence used
  before `session.open`; accepting legacy or malformed key payloads silently
  weakens signer custody and makes product-visible route/admission failures
  look like ordinary discovery misses.
- After the change, `resolved_public_keys` is a fallible schema-bound parser
  for canonical `public_keys_b64[]` only, rejects malformed JSON,
  non-array/non-string/empty key rows, and no longer repairs
  `public_key_b64`. Paired user trust sync now resolves with the locally
  published `presented_pubkey_b64`, propagates resolve schema failures as
  bootstrap failures, and R93 guards this boundary against legacy response
  repair.

## 2026-07-21 FFI descriptor catalog provider payload audit

- `/Users/macbook.silan.tech/.local/bin/codegraph explore
  "descriptor_ref not found owner is not online browser.open_session
  invocation.history.list AUTHORITY_SUBJECT_MISMATCH
  target_owned_descriptor_catalog_subject_ura"` identified the C ABI
  descriptor resolver as a product-facing route-visibility boundary. The
  relevant source path was `runtime_resolve_descriptor_ref_json` through
  `runtime_system_descriptor_catalog_entries`,
  `runtime_meta_descriptor_catalog_entries`, and
  `descriptor_catalog_entry_from_value`.
- Targeted source inspection found the compatibility seam: provider catalog
  rows from `meta.list_abilities` were ingested with
  `filter_map(descriptor_catalog_entry_from_value)`, and system catalog rows
  used `.ok().and_then(...)`. A malformed descriptor row could therefore be
  silently dropped and reported to products as a descriptor miss or invisible
  route.
- The root abstraction problem was treating descriptor catalog rows as
  optional discovery hints. For SDK descriptor resolution they are provider
  evidence for `(callee_ura, ability, call_mode)`; malformed rows mean the
  provider payload is unavailable/corrupt, not that the ability is absent.
- After the change, descriptor catalog ingestion uses fallible row parsers for
  both provider and system rows, preserves explicit error detail for missing
  fields, non-canonical descriptor hashes, and invalid descriptor refs, and
  R94 prevents reintroducing skip-on-error catalog ingestion.

## 2026-07-21 FFI descriptor remote probe caller tuple audit

- `/Users/macbook.silan.tech/.local/bin/codegraph explore
  "runtime_resolve_descriptor_ref_json caller_ura subject_ura unwrap_or_else
  runtime_owner_ura invocation.history.list authorized runtime session
  descriptor resolve tuple default"` identified
  `runtime_resolve_descriptor_ref_json` as the descriptor-resolution ingress
  where SDK tuple fields are converted into a remote `meta.list_abilities`
  invocation.
- Targeted source inspection found the compatibility seam: after local/system
  catalog checks missed, the remote descriptor probe read optional
  `caller_ura` and fell back to `runtime_owner_ura`. An incomplete SDK tuple
  could therefore become a signed remote probe under the local runtime owner
  instead of failing as malformed input.
- The root abstraction problem was treating descriptor resolution as a local
  convenience lookup after it had crossed into remote invocation. Remote
  `meta.list_abilities` is a signed runtime call; its caller is authority
  material and must come from the canonical tuple, not from ambient runtime
  state.
- After the change, remote descriptor probes require explicit `caller_ura`
  before signer lookup or daemon IO, while local owner catalog and system
  descriptor catalog resolution remain available without a remote probe. R95
  prevents reintroducing runtime-owner caller synthesis.

## 2026-07-21 SDK authorized history filter tuple audit

- `/Users/macbook.silan.tech/.local/bin/codegraph explore
  "invocation.history.list subject_ura authority subject
  session_authority_admits_subject AUTHORITY_SUBJECT_MISMATCH
  RuntimeReceiptQuery authorized_runtime_session history caller_ura
  callee_ura"` identified the authorized session history facade as the
  product-facing receipt query boundary. The relevant cross-language nodes
  were Go `SessionHistoryOperations.List`, Python
  `SessionHistoryOperations.list`, and their receipt providers.
- Targeted source inspection found the compatibility seam: both SDKs validated
  the runtime call authority against `request.Call`, then passed
  `request.Filter` through unchanged to the receipt provider. A product could
  authorize a query envelope for one caller/callee/subject tuple while asking
  the filter to read another caller, callee, or subject scope.
- The root abstraction problem was treating receipt filters as passive read
  predicates after authority validation. In an authorized runtime session,
  history filters are part of the query authority surface because they define
  which receipt rows the product is attempting to observe.
- After the change, Go and Python authorized session history list paths both
  validate the complete request. Optional filter `caller_ura`, `callee_ura`,
  and `subject_uras` must be bound to the same call tuple already admitted by
  the runtime authority; mismatches fail before the receipt provider is called.
  R96 guards this cross-SDK parity.

## 2026-07-21 Device visibility route-gate audit

- `/Users/macbook.silan.tech/.local/bin/codegraph explore
  "meta.list_abilities browser.open_session route visible owner is not online
  descriptor_ref stale device"` identified `federation_probe::collect_device_view`
  and `federation_probe::resolve_device_record` as the product-facing device
  visibility boundary feeding `node.list`, `node.describe`, and
  `observe.network_health`.
- Targeted source inspection found the compatibility seam: realm directory
  rows that looked like device profiles were always pushed into `nodes`, even
  when the signed health probe failed with route/owner-offline evidence. The
  explicit `node.describe` path also returned directory ability summaries for
  a probe-failed remote device.
- The root abstraction problem was treating directory visibility as route
  visibility. A directory row can say a device profile was advertised; it does
  not prove that the owner is online, has a route-visible descriptor catalog,
  or can execute `browser.open_session` / `meta.list_abilities` now.
- After the change, `DeviceNetworkView` separates route-visible `nodes` from
  `unavailable_nodes`. `collect_device_view` only exposes remote peers after a
  successful signed probe, while preserving failed probe evidence for
  diagnostics. `resolve_device_record` rejects unrouteable remote devices
  before returning stale `ability_summaries`.

## 2026-07-21 Namespace resolver qtype ingress audit

- `/Users/macbook.silan.tech/.local/bin/codegraph explore
  "namespace.resolve qtype query_name ability_name default infer empty string
  route directory fallback"` identified the public resolver boundary across
  `handle_namespace_resolve_at`, `DaemonRouteResolver::resolve_query_json`,
  `NamespaceProxyResolveRequest`, and `namespace_proxy_resolve_peer_arguments`.
- Targeted source inspection found two compatibility seams. Local
  `namespace.resolve` inferred the resolve kind when `qtype` was missing, and
  `namespace.proxy_resolve` defaulted missing `qtype` to
  `RESOLVE_TYPE_DIRECTORY_LISTING` before forwarding to peers.
- The root abstraction problem was treating resolver input shape as a
  convenience envelope. `qtype` selects the resolver state machine; deriving it
  from `query_name` / `ability_name` allows incomplete product tuples to
  continue into route selection and makes bad product ingress look like
  ordinary route invisibility.
- After the change, public resolver ingress validates canonical `ResolveType`
  enum strings before dispatch or peer fanout. Missing, empty, unspecified, or
  shorthand qtype values fail closed with typed refused errors; the lower
  resolver object remains a pure route/directory engine once it receives
  schema-bound input.

## 2026-07-21 Federation peer trust projection audit

- `/Users/macbook.silan.tech/.local/bin/codegraph explore
  "federation_peers realm trust trusted_agent role unwrap_or schema-B
  trusted_hubs"` plus targeted `rg` identified
  `src/cli/commands/federation_peers.rs` as the operator-facing seam where
  `realm-trust.toml` becomes the `trusted_hubs` read model shown to products
  and operators.
- Targeted source inspection found a duplicated loose trust parser in the CLI:
  `parse_trusted_hubs_from` read TOML with `toml_edit`, defaulted missing or
  non-string `role` to `""`, skipped non-hub rows, and accepted hub
  `agent_ura` as any non-empty string. Schema-incomplete hub rows were listed
  as trusted hubs even though the cross-hub dialer would not treat them as
  dial-eligible federation peers.
- The root abstraction problem was a second trust read-model authority. The
  daemon already owns `RealmTrustAnchor`, canonical role validation, URA-role
  validation, duplicate-principal checks, and unknown-field rejection. The CLI
  operator view must consume that aggregate instead of reinterpreting the
  security-sensitive trust file.
- After the change, `easynet federation peers` loads `realm-trust.toml`
  through `RealmTrustAnchor::load_or_empty`, preserving only the documented
  missing-file empty state. Hub rows are projected only after daemon trust
  schema validation and after explicit schema-B completeness checks for
  `origin_realm`, `hub_endpoint`, and `tls_ca_pem_path`.

## 2026-07-21 Remote desktop consent receipt projection audit

- `/Users/macbook.silan.tech/.local/bin/codegraph explore
  "mission session filter_map serde_json from_value ok corrupt rows skipped
  session state"` surfaced `plugins/remote-desktop/src/session_consent.rs` as
  the product-facing receipt projection seam where Axon causal-context receipt
  facts become remote-desktop consent grants and later session access checks.
- Targeted source inspection found the compatibility path:
  `RemoteDesktopConsentReceipt::from_value` returned `Option`,
  `receipts_from_causal_context` used `filter_map`, and missing `receipts[]`
  defaulted to an empty vector. A malformed scalar/list causal context could
  therefore be treated as no receipt, and owner callers could fall back to
  `owner_self_consent` even though they supplied corrupt receipt proof input.
- The root abstraction problem was treating causal context as optional UI
  decoration after Axon admission. For remote desktop, a stored approval
  receipt is the session access proof fact. Once a caller declares scalar/list
  causal context, every receipt row is proof input and must be schema-bound
  before any consent policy fallback or session-token control-plane check.
- After the change, receipt projection is fallible and shared by creation and
  later access checks. `kind = "none"` / `kind = "merkle"` remain no-receipt
  states for this plugin, while `scalar` and `list` require non-empty
  `receipt_ura` and `receipt_hash` on every row. Malformed causal context
  fails as invalid argument instead of becoming self-consent or a partial
  receipt match.

## 2026-07-21 Invocation history filter schema audit

- `/Users/macbook.silan.tech/.local/bin/codegraph explore
  "invocation_history filter subject_uras caller_ura callee_ura
  unwrap_or_default filter_map invalid scope history list"` identified
  `src/daemon/ability/builtins/governance/invocation_history.rs` as the
  product-facing seam where UI/SDK history filters become canonical ledger
  queries and pre-runtime attempt diagnostic projections.
- Targeted source inspection found duplicated loose parsing:
  `subject_filter_values` accepted `filter.subject_uras` as an array but used
  `filter_map(non_empty_str)`, `value_string_set` ignored non-array or
  malformed `ability_uras`, and attempt filtering re-read
  `caller_ura`/`callee_ura`/`subject_ura`/`ability_ura`/`state`/`trace_id`
  with `and_then(non_empty_str)`. Malformed filter scope could therefore widen
  to all rows, become an empty ability set, or become a no-match diagnostic
  result instead of being rejected as invalid observation scope.
- The root abstraction problem was treating receipt-history filters as
  best-effort UI predicates after the history request had crossed the daemon
  boundary. These filters define which caller/callee/subject/ability receipt
  rows a product is trying to observe; they are authority-bearing read scope
  and must be schema-bound once provided.
- After the change, `optional_filter_string`, `value_string_set`, and
  `subject_filter_values` form a shared fail-closed parser for both Axon ledger
  records and attempt audit records. Missing optional fields remain no filter;
  present malformed strings, malformed arrays, empty scoped arrays, and
  invalid subject array members fail before any history rows are projected.

## 2026-07-21 Remote desktop create-session ingress schema audit

- `/Users/macbook.silan.tech/.local/bin/codegraph explore
  "remote_desktop parse_video_constraints codec_preferences scale_mode
  hardware_acceleration_required input_policy video schema invalid default"`
  identified `plugins/remote-desktop/src/request.rs` as the product ingress
  seam where `remote_desktop.create_session` arguments become immutable
  session profile, lease, media policy, and input policy state.
- Targeted source inspection found best-effort schema repair:
  non-object `video` was projected as an empty object, malformed numeric and
  boolean fields fell back to defaults, `codec_preferences` used
  `filter_map` and silently dropped bad rows or unsupported codecs,
  non-object `input_policy`/`input` became no input, malformed booleans became
  `false`, non-string `mode` became `view_only`, bad TTL fields fell back to
  defaults, and malformed `session_id` was replaced by a freshly minted id.
- The root abstraction problem was split ownership between a permissive
  product parser and a vague descriptor schema. Session creation is the point
  where consent, subject resource, transport preference, media policy, and
  input policy become durable session facts; once a caller supplies a field,
  it is descriptor-bound input and must be validated before any state is
  minted.
- After the change, absent optional fields still select documented defaults,
  but present malformed fields fail closed. The dynamic registration schema
  and static ability TOML now advertise the same nested video/input policy
  fields that the parser accepts, so descriptor evidence and runtime ingress
  cannot drift into two contracts.

## 2026-07-21 Remote desktop interactive input frame audit

- `/Users/macbook.silan.tech/.local/bin/codegraph explore
  "remote_desktop invoke_bidi frame type unwrap_or empty malformed frame
  ignored input media terminal session"` and
  `/Users/macbook.silan.tech/.local/bin/codegraph explore
  "remote_desktop input frame serde default deny_unknown_fields
  bidi_control_frame_type invalid_frame key code clipboard file_drop"`
  identified `plugins/remote-desktop/src/input.rs` and
  `plugins/remote-desktop/src/invoke_bidi.rs` as the direct-control seam where
  WebRTC data-channel / diagnostic InvokeBidi frames become local OS input
  actions or session diagnostics.
- Targeted source inspection found input-frame compatibility repair:
  `ClipboardInputFrame.text` and `FileDropInputFrame.files` defaulted to empty
  values, key frames could omit both `key` and `code`, unknown payload fields
  were accepted by Serde, and the diagnostic Bidi control loop classified
  missing or malformed `type` as `unknown_frame` through `unwrap_or("")`.
- The root abstraction problem was treating malformed input frames as regular
  product diagnostics after routing had already accepted the frame. Input
  frames are device-local control-plane commands; bad shape must fail before
  session policy or platform injection so products can distinguish corrupt
  control input from a supported-but-disabled action.
- After the change, input payload structs deny unknown fields, clipboard and
  file-drop payload facts are mandatory, key frames require at least one
  non-empty key identity (`key` or `code`), file-drop paths must be non-empty,
  and diagnostic Bidi control frame type parsing returns `invalid_frame` for
  missing/non-string/blank type. `unknown_frame` is now reserved for
  schema-valid but unsupported frame types.

## 2026-07-21 Remote desktop ICE candidate signaling audit

- `/Users/macbook.silan.tech/.local/bin/codegraph explore
  "remote_desktop session_store unwrap_or_default malformed store json default
  empty corrupt session state"` identified `RemoteDesktopSessionStore` as the
  store-level projection boundary for device-local WebRTC candidate facts.
- Targeted source inspection found that local WebRTC candidate projection used
  `candidate.get("candidate").and_then(Value::as_str).unwrap_or_default()`.
  Non-object candidate rows, missing `candidate`, and non-string `candidate`
  therefore collapsed into an empty string and were silently treated as
  end-of-candidates. The public `add_ice_candidate` handler also stored the
  remote candidate before the transport parser proved the candidate shape.
- The root abstraction problem was treating ICE candidates as optional display
  hints. They are signaling facts in the session read model: remote candidates
  are product-submitted control input and local candidates are device-generated
  transport evidence. Bad shape must fail before storage or be recorded as a
  transport diagnostic, not disappear as "no candidate".
- After the change, `sdp::ice_candidate_text` is the shared candidate fact
  parser. Remote add-ICE validates through `remote_ice_candidate_inits` before
  appending to the session. Local candidates validate through
  `ice_candidate_text` before session projection; serialization/projection
  failures become `ICE_CANDIDATE_SCHEMA_INVALID` diagnostics.

## 2026-07-21 Desktop companion desired-state store audit

- `/Users/macbook.silan.tech/.local/bin/codegraph explore
  "companion state_store unwrap_or_default malformed toml json default empty
  corrupt plugin desired state"` identified
  `src/daemon/plugins/companion/state_store.rs` as the durable lifecycle
  memory for desktop companion packages.
- Targeted source inspection found compatibility repair on persisted lifecycle
  rows: `CompanionStateRecord.desired_state` used `#[serde(default)]`, so an
  existing row missing its desired lifecycle state became
  `CompanionDesiredState::Disabled`. The TOML parser also accepted unknown row
  fields, blank identities, and duplicate `(id, version)` rows while
  `record(...)` selected the first match.
- The root abstraction problem was conflating missing package state with
  corrupt package state. Missing file / absent row is a valid fresh-install
  empty state. An existing row is lifecycle authority consumed by daemon-ready
  reconciliation, status projection, and cleanup; it must not be repaired into
  disabled or first-match behavior.
- After the change, companion state records deny unknown fields, require
  explicit `desired_state`, and validate non-empty identity plus unique
  `(id, version)` keys after parsing. Optional action/error telemetry remains
  optional because it is not the lifecycle decision fact.

## 2026-07-21 Installed plugin active-state store audit

- `/Users/macbook.silan.tech/.cache/codex-runtimes/codex-primary-runtime/dependencies/bin/override/codegraph`
  was not present in the current execution environment, and
  `/Users/macbook.silan.tech/.local/bin/codegraph explore
  "plugin install state PluginStateToml serde default plugins malformed blank
  duplicate lockfile installed package state"` reported that this checkout has
  no `.codegraph/` index. Per the tool guidance, no agent-owned `codegraph
  init` was run.
- Targeted source inspection with `rg` found two active-state parser owners:
  `PluginStateStore::read` parsed `plugins.toml` directly, and
  `PluginPackageIndex::{installed,installed_resilient}` parsed
  `plugin-lock.toml` directly. `PluginStateToml.plugins` used
  `#[serde(default)]`, so an existing empty or schema-incomplete file could be
  repaired into an empty active package set.
- The root abstraction problem was treating active installed package state as a
  loose DTO instead of durable package authority. Package directories are not
  live unless the active state names them, so the state parser must be the
  single owner of row shape, identity completeness, and active-version
  uniqueness.
- After the change, `PluginStateToml::parse_active_projection` is the shared
  parser for both `plugins.toml` and `plugin-lock.toml`. Existing state files
  deny unknown fields, require explicit `plugins`, require non-empty
  `id`/`version`/`hash`, and reject duplicate `id@version` rows or multiple
  active versions for one package id. Missing state files remain the only valid
  fresh empty install state.

## 2026-07-21 Context store read-model authority audit

- `.codegraph/` remains absent in the checkout; codegraph 1.4.1 is installed
  at `/Users/macbook.silan.tech/.local/bin/codegraph`, but no agent-owned
  indexing was performed. Targeted `rg` searches were used for this seam.
- `rg -n "read_to_string|serde_json::from_str|unwrap_or_default|filter_map"
  src/daemon/persistence/context_store.rs` identified remaining compatibility
  projections after the earlier clipboard-history fix: `load_config()` parsed
  `config.json` with `.ok().and_then(...).unwrap_or_default()`,
  `list_folders()` and `list_favorites()` parsed their JSON files with the
  same repair-to-empty pattern, and `list_captures()` /
  `list_capture_abilities()` used JSONL `filter_map` to skip malformed capture
  rows.
- The root abstraction problem was split state semantics inside one Context
  read model. Clipboard history had already become fail-closed, but tracking
  config, folder mappings, favorites, and media captures still treated corrupt
  existing files as first-run empty/default state.
- After the change, Context state has shared read/parse helpers and
  per-projection validators. Existing config/folder/favorite files deny
  unknown fields and fail on malformed JSON; folders/favorites require
  non-empty identity/path/reference facts; capture JSONL parsing fails on the
  first malformed row and validates non-empty id/timestamp/device/ability/file
  facts plus safe capture path segments. CLI, ability handlers, and capture
  lookup now propagate store errors instead of projecting empty product state.

## 2026-07-21 Global skill pool package inventory audit

- `.codegraph/` remains absent in the checkout; codegraph 1.4.1 is installed
  but unavailable for indexed queries. Targeted source search identified this
  seam.
- `rg -n "scan_global_pool_into|global_skill_record_from_dir|skill_dir_in_global_pool|filter_map|flatten|\\.ok\\(\\)|unwrap_or_default"
  src/daemon/resources/skills/store.rs
  src/daemon/ability/builtins/resources/skills/list.rs
  src/daemon/ability/builtins/resources/skills/publish.rs -S` identified the
  global skill inventory path: `skill.list` used `scan_global_pool_into`, which
  walked pool entries with `flatten`; `global_skill_record_from_dir` returned
  `Option`; `skill_dir_in_global_pool` returned `Option`; missing
  `SKILL.md` frontmatter name therefore looked the same as "no such skill".
- The root abstraction problem was conflating optional environment roots with
  corrupt package state. A missing global pool is a valid empty environment,
  but a directory with skill package shape is package authority and must not be
  silently removed from product inventory.
- After the change, global pool scan and lookup are fallible. Directory entry
  errors, unreadable pools, unreadable `SKILL.md`, missing frontmatter `name`,
  and metadata read failures surface as inventory errors. `skill.list` and
  `skill.publish` now propagate those errors, while non-skill user directories
  under the global pool remain ignored.

## 2026-07-21 Pages dynamic API ability discovery audit

- `.codegraph/` remains absent in the checkout; codegraph 1.4.1 is installed
  but unavailable for indexed queries. Targeted source search identified this
  seam.
- `rg -n "api_ability_names_for_project|read_dir\\(&api_dir\\)|read_dir\\.flatten|return Vec::new\\(\\)"
  src/daemon/ability/builtins/resources/pages/api.rs -S` identified dynamic
  API ability discovery as a product route-visibility seam. The old
  implementation returned an empty list when `api/` was unreadable and used
  `read_dir.flatten()` plus `file_type` skip behavior, so corrupt API surface
  state made dynamic routes disappear from the catalog.
- The root abstraction problem was conflating "project has no API directory"
  with "project API directory exists but is corrupt/unreadable". Missing
  `api/` is a valid no-API project state; existing corrupt API surface is
  unavailable route-registration state.
- After the change, `api_ability_names_for_project` returns
  `anyhow::Result<Vec<String>>`. Missing project or missing `api/` returns an
  empty list; unreadable/non-directory API path, directory entry errors, and
  file-type errors fail registration and propagate through
  `register_api_abilities_for_project`.

## 2026-07-21 Device product local identity fallback audit

- `/Users/macbook.silan.tech/.local/bin/codegraph status
  /Users/macbook.silan.tech/Documents/GitHub/EasyNet-Cli` reported the
  checkout as not initialized, and the user-provided override path was absent.
  No new `.codegraph/` index was created; targeted source search was used for
  this seam.
- `rg -n "load_credentials\\(\\)\\.ok\\(\\)|unwrap_or_default\\(\\).*local_node|local_tenant|String::new\\(\\).*local_ura|classify_device_show_target\\("
  src/cli/commands/groups/device.rs -S` identified the remaining product
  ingress fallback in `device show` / `device remove`: credential load errors
  were erased with `.ok()`, local node/realm became empty strings, and remote
  revocation proceeded with an empty `local_ura`.
- The root abstraction problem was treating local device identity as optional
  display context while using it to build self-target guards and remote
  operation authority context. Non-local device inspection/removal is a
  product runtime operation; missing or malformed credentials are unavailable
  local identity state, not permission to synthesize empty route/caller facts.
- After the change, `DeviceLocalIdentity` owns the explicit `(realm, node_id)`
  state, `load_local_device_identity` is the only product ingress loader, and
  `device show` / `device remove` fail closed before remote dispatch when
  local credentials are unavailable. `device show local` remains local-only,
  and canonical self Device URA is classified as local instead of routed
  remotely.
## 2026-07-22 Local API key default-token cache audit

- `/Users/macbook.silan.tech/.local/bin/codegraph query -p /Users/macbook.silan.tech/Documents/GitHub/EasyNet-Cli
  "legacy fallback compatibility unwrap_or_default production route descriptor authority signer"
  --limit 40` surfaced credential and compatibility fallback candidates after
  the MCP HTTP parse fallback proved test-only.
- `rg -n "read_local_default_token\\(|api_keys\\.local\\.toml|default_token"
  src/daemon src/cli tests tools -S` showed the local bearer cache has one
  reader in `governance/api_key.rs` and one product consumer in `llm_api.rs`.
- Root abstraction problem: `api_keys.local.toml` was treated as a best-effort
  cache (`HOME`, file read, TOML parse, and missing field all collapsed through
  `.ok()?`) even though it carries bearer credential projection state.
- Boundary decision: only absent cache file means "no default token"; existing
  corrupt/unreadable/schema-invalid/blank-token cache is unavailable credential
  state and must stop bearer request construction.

## 2026-07-22 Runtime trust revoke credential projection audit

- `/Users/macbook.silan.tech/.local/bin/codegraph query -p /Users/macbook.silan.tech/Documents/GitHub/EasyNet-Cli
  "caller signer keyring identity fallback ok unwrap_or default credential route descriptor"
  --limit 50` surfaced identity/credential projection candidates in admission
  and runtime trust code.
- `rg -n "load_credentials\\(\\)\\.ok\\(\\)|credentials\\(\\)\\.ok\\(\\)|user_ura\\(\\)\\.ok\\(\\)"
  src/daemon src/cli sdk/go sdk/python sdk/node -S --glob '!**/*test*'
  --glob '!**/tests/**'` showed
  `RuntimeTrustConnectionStateProjector::from_local_credentials` collapsed
  credential load failure and missing credentials into the same `None`.
- Root abstraction problem: trust revoke had a canonical trust-anchor mutation
  followed by best-effort runtime projection. If local credentials were corrupt,
  the trust mutation could commit while connection-state projection silently
  disappeared.
- Boundary decision: projector construction is now a preflight that returns
  `Result<Option<_>>`; missing credentials means no local projection, corrupt
  credentials abort before trust mutation.

## 2026-07-22 Admission owner credential projection audit

- `/Users/macbook.silan.tech/.local/bin/codegraph query -p /Users/macbook.silan.tech/Documents/GitHub/EasyNet-Cli
  "owner_fact_from_local_device load_credentials ok admission policy owner resolution"
  --limit 50` surfaced `policy_gate::owner_fact_from_local_device` as the
  admission owner-resolution fallback point.
- `rg -n "load_credentials\\(\\)\\.ok\\(\\)|user_ura\\(\\)\\.ok\\(\\)|parse_ura\\([^\\n]+\\)\\.ok\\(\\)"
  src/daemon/invocation/admission src/daemon/invocation/dispatch -S
  --glob '!**/*test*'` showed the local owner fact path collapsed malformed
  credentials into `None`, allowing policy evaluation to continue with an
  unresolved owner.
- Root abstraction problem: owner resolution mixed "not this local identity"
  with "local identity authority is corrupt" in an `Option<OwnerFact>`.
- Boundary decision: `resolve_owner` is now fallible and local device owner
  projection uses `load_credentials_optional()` so missing credentials and
  corrupt credentials remain distinct states.

## 2026-07-22 Shared local device owner projection audit

- `rg -n "local_device_owner_fact\\(" src/daemon/invocation/admission
  src/daemon -S` showed only two production consumers of the shared local
  owner projection: bootstrap authority and device principal construction.
- The root abstraction problem was that the shared projector returned
  `Option<OwnerFact>`, forcing both consumers to treat corrupt credentials and
  "no local owner fact" as the same state.
- Boundary decision: `local_device_owner_fact` is now the single fail-closed
  credential classifier for device-owner projection; bootstrap authority has
  an explicit `Unavailable` state and policy principal construction returns
  `Result<PrincipalProjection, Status>`.

## 2026-07-22 Node session authority subject-binding audit

- `/Users/macbook.silan.tech/.local/bin/codegraph callers mintSessionAuthority`
  showed the session-authority minting seam lives in SDK authority clients and
  tests, while daemon admission only verifies the resulting metadata.
- `/Users/macbook.silan.tech/.local/bin/codegraph query SessionAuthorityPayload`
  showed Rust admission owns strict subject-shape validation for
  `SessionAuthorityPayload`.
- `rg -n "sessionOwnerUserID|session_owner_user_id|subject_ura|SessionAuthority"
  sdk/node sdk/go sdk/python src -S --glob '!sdk/node/node_modules/**'`
  showed Go/Python already normalize session-owner user identity against
  canonical user subjects, while Node still stopped at all-zero/string checks.
- Root abstraction problem: Node treated typed session authority as shape-valid
  metadata even when its subject could never pass daemon admission, pushing
  deterministic authority failures into product runtime calls.
- Boundary decision: Node now classifies session authority subject semantics at
  the SDK facade boundary using the same canonical subject cases as daemon
  admission: user subject or user-owned session resource.

## 2026-07-22 Session prelude paired-user credential audit

- `/Users/macbook.silan.tech/.local/bin/codegraph explore load_runtime_caller_signer
  key service credentials user_ura caller signer` tied the reported
  caller-signer failures to daemon self-identity/key-service state and session
  prelude identity setup.
- `rg -n "let Ok\\([^)]*user_ura|user_ura\\(\\)\\.ok\\(\\)|load_credentials\\(\\)\\.ok\\(\\)"
  src/daemon src/cli src/bin sdk -S --glob '!**/*test*' --glob '!**/tests/**'
  --glob '!sdk/node/node_modules/**'` surfaced
  `sync_paired_user_trust_prelude` swallowing credential load and user URA
  projection failures into `NotRequired`.
- Root abstraction problem: paired-user trust bootstrap represented both
  "daemon is unpaired" and "paired identity is corrupt" as the same
  `NotRequired` outcome, allowing session open/ready state to proceed before
  canonical remote invocation failed on missing caller signer or authority
  state.
- Boundary decision: session prelude now uses the same optional-credential
  classifier pattern as admission: absent file is optional absence; existing
  invalid paired identity is a prelude failure.

## 2026-07-22 Descriptor-ref route selector fallback audit

- `/Users/macbook.silan.tech/.local/bin/codegraph status` reported the index
  for `/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Cli` at version
  `1.4.1` with pending Rust changes in the route resolver; the documented
  override path was absent in this shell, so the existing
  `/Users/macbook.silan.tech/.local/bin/codegraph` installation was used.
- `rg -n "descriptor_ref not found|meta.list_abilities|invocation.history.list|
  remote invocation requires a caller signer" src sdk tools tests -S` tied the
  reported product failures to the descriptor resolution and route selector
  boundary rather than the browser UI.
- `git diff -- src/daemon/invocation/routing/route_resolver.rs` showed the
  previous selector helper collapsed
  `canonical_ability_descriptor_ref(...).ok()?`,
  `ability_ura_from_descriptor_ref(...).ok()?`, and
  `AbilitySelector::parse(...).ok()` into `None`.
- Root abstraction problem: descriptor-ref parsing was represented as an
  optional route-shape match, so malformed descriptor refs and owner mismatches
  could degrade into generic route query failures or later catalog/remote
  discovery errors.
- Boundary decision: route selector construction is now fallible. Descriptor
  canonicalization, ability extraction, and selector parsing failures remain
  typed `ResolveRouteFailure` states; descriptor owner mismatch is refused
  before any route lookup.

## 2026-07-22 FFI descriptor runtime-owner fallback audit

- `/Users/macbook.silan.tech/.local/bin/codegraph explore
  runtime_owner_ura_from_session easynet_runtime_resolve_descriptor_ref caller
  signer descriptor_ref` produced the call flow
  `easynet_runtime_resolve_descriptor_ref ->
  runtime_resolve_descriptor_ref_json -> runtime_owner_ura_from_session`.
- The same codegraph output reported `runtime_owner_ura_from_session` has three
  callers in `/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Cli/src/ffi/invocation/mod.rs`
  and no direct covering tests, making the FFI descriptor resolver a high-risk
  compatibility seam.
- `rg -n "runtime_owner_ura_from_session\\(session\\)\\.ok\\(\\)|requires a
  caller signer|descriptor_ref not found|meta.list_abilities" src/ffi
  src/daemon sdk -S` showed `runtime_resolve_descriptor_ref_json` was the only
  descriptor-resolution path collapsing native runtime owner resolution into an
  optional value before remote probing.
- Root abstraction problem: FFI descriptor resolution treated native runtime
  owner identity as a best-effort catalog optimization. If `control.json` or
  daemon identity was unavailable, the resolver continued into remote
  discovery and could surface unrelated signer, owner-offline, or timeout
  failures.
- Boundary decision: runtime owner identity is a precondition for FFI
  descriptor resolution. Missing/invalid control discovery now returns a
  caller-identity error before local catalog lookup or remote
  `meta.list_abilities` probing.

## 2026-07-22 Invocation history ledger URA projection audit

- `/Users/macbook.silan.tech/.local/bin/codegraph explore
  invocation.history.list subject_filter ledger_resource_ura
  session_owner_user_id AUTHORITY_SUBJECT_MISMATCH` surfaced
  `ledger_resource_ura` as the history response owner-projection seam.
- The codegraph source showed `ledger_resource_ura()` returning `Option<String>`
  and using both `AgentAggregateRepository::load_hosted_identity_status().ok()?`
  and `parse_ura(...).ok()?`, so aggregate load failures and malformed hosted
  identity were indistinguishable from an unjoined daemon.
- `rg -n "ledger_resource_ura\\(|load_hosted_identity_status\\(\\)\\.ok\\(\\)?"
  src/daemon/ability/builtins/governance/invocation_history.rs
  tests/scripts/test_check_architecture_convergence.sh` confirmed the same
  compatibility pattern existed in the architecture good fixture.
- Root abstraction problem: invocation history treated the ledger URA as
  optional presentation metadata, even though it is the resource owner identity
  for returned receipt evidence.
- Boundary decision: unjoined state remains `Ok(None)`, but unreadable aggregate
  projection and malformed `host_device_agent_ura` are now typed history
  response failures.
