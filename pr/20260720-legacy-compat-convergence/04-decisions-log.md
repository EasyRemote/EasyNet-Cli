# Decisions log

Decisions will be appended as root-fork choices are made.

## 2026-07-20 ability target tuple ingress

- Treat `AbilityTargetRequest.subject_ura` as explicit invocation tuple input,
  not a value derived from ability URA or descriptor-ref selector projection.
  Ability projection may identify the route owner and descriptor binding, but
  it must not become the envelope subject.
- Treat blank target `call_mode` as invalid SDK/provider input. RPC remains a
  valid high-level operation choice only when the caller/facade selects it
  explicitly before crossing the provider seam.

## 2026-07-20

- Treat Go `RuntimeIdentityProjection.DeviceID` as a product-named canonical
  SDK defect. The canonical root now exposes `RuntimeInstanceID` only.
- Keep EasyNet daemon `device_id` and `node_id` support outside the SDK root in
  `sdk/go/provider/easynet`, matching the Python provider split.
- Update the frozen edge-adapter policy because the non-canonical Go public
  surface shrank by two members; this is recorded as compatibility-surface
  deletion, not as a new edge adapter.
- Treat LedgerSink `_system` receipt-owner derivation as an architectural
  defect, not a convenience fallback. A terminal ledger row must be owned by
  the descriptor/binding-derived subject, callee, or caller; if none can own
  the route/record, the sink adapter must fail fast and let Axon's bounded
  persistence path report the sink failure rather than writing a forged system
  owner.
- Authority-owned receipt routes must carry an owner-local namespaced ability
  such as `system.chat`; bare `chat` under `authority` is rejected instead of
  being rewritten to `_system/authority.system.chat`.
- Treat `AgentEntry.model` as stale registry state, not as a dispatch-time
  model authority. Mission dispatch model selection is now only:
  per-invocation override > `agent.toml::model`; if both are absent, the driver
  owns its default. This removes the registry row as a second runtime
  configuration source while preserving the public ability override path.
- Treat `AgentEntry.timeout_secs` as stale registry state at dispatch time.
  Migration owns translating customized v1 row timeouts into `agent.toml`;
  dispatch now uses `agent.toml::timeout_secs` or the canonical agent runtime
  default, not the registry row. R66 now covers both model and timeout entry
  fallback.
- Treat `RealmTrustAnchor::lookup()` as a singleton-principal lookup only.
  User-role trust is not singleton state: it is a descriptor/admission proof
  fact bound to `(user_ura, pubkey)`. Bare user URA resolution now fails
  closed; callers that need user material must present or enumerate the user
  key through explicit user-key APIs.
- Treat descriptor call mode as required tuple input at generic runtime
  resolver seams. RPC is a valid high-level convenience choice, but it must be
  made by the caller/facade before the provider boundary; RuntimeClient and C
  ABI descriptor providers must not infer it from absence.
- Treat `sdk/python/easynet_sdk/_key_service.py` as a retired product-specific
  compatibility facade, not as a canonical SDK abstraction. The canonical
  public capability remains `managed_signing`; EasyNet key-service transport
  details remain owned by `providers.easynet.key_service`, and SDK
  product-neutrality now rejects the private facade if it reappears.
- Treat `sdk/go/daemon_compat.go` as an obsolete released-edge adapter rather
  than a valid compatibility layer. The Go SDK root now exposes only the
  canonical `RuntimeHost` lifecycle model. EasyNet daemon process policy
  (`StartConfig`, `ModeDevice`, provider directory roots, and C ABI daemon
  aliases) belongs under `sdk/go/provider/easynet` or is removed from the root.
- Treat an empty released-edge-adapter policy as a valid cutover-complete
  state. The policy still freezes non-canonical public surface counts and
  shapes, but it no longer requires retaining at least one compatibility
  adapter just to make the policy meaningful.
- Treat target-owned daemon-system local invocation subject selection as an
  issuer input, not as a support transport fallback. `LocalDaemonSystemAbilityIssuer`
  now requires a concrete subject URA, so product commands must choose either
  an envelope subject or the target's descriptor-owned default before crossing
  the local daemon gRPC boundary.
- Treat the retired `~/.easynet/workspaces` layout as unsupported state rather
  than startup compatibility input. New runtime state has a single per-agent
  directory authority, `agents_root()`, and registry load/write no longer owns a
  root-prefix migration path.
- Treat all-zero principal identifiers as invalid authority facts, not as
  values that can be carried until daemon admission rejects them. The canonical
  authority facade now fails fast across Go, Python, Node, Java, and Swift for
  all-zero session owners or owner-bearing URAs.
- Treat PrincipalLifecycle as the canonical same-realm User principal/key
  authority. `RealmTrustAnchor` remains a projection and federation trust set;
  it is no longer the only source consulted by LocalRuntime caller
  authentication or product admission role classification.
- Keep `FederatedKeyResolver` as the single shared key-resolution provider for
  LocalRuntime admission and `federation.resolve_key`. The late-bound
  PrincipalLifecycle reader exists only to close boot-order construction:
  LocalRuntime is built before transport boot derives the lifecycle store path,
  but both layers share the same resolver instance.
- Product admission caller-role classification now follows the same authority
  order as signature verification: trust-anchor projection, active same-realm
  PrincipalLifecycle User, then explicitly configured federated caller role.
  Unknown local callers fail closed with a canonical local principal/trust
  diagnostic instead of a trust-anchor-only message.
- Error-path `axon_rpc_*` op events are retained as production observability,
  not temporary debugging. They fire only when LocalRuntime request preparation,
  admission, or finalization fails before a complete receipt chain can be
  returned.
- Treat SDK conformance report execution as the owner of shared repository-gate
  toolchain dependencies. Language slices select which adapters run; they do
  not select which cross-language validators are available. The report gate now
  resolves a pytest-capable Python before source-snapshot execution so live
  result files are not dependent on ignored `.venv` state or the caller's
  ambient shell.
- Treat SDK conformance report execution and parity validation as one
  toolchain-attestation domain. The report runner must use the same
  `toolchain_path.sh` resolver as `check-sdk-parity-matrix.sh`; otherwise live
  records can pass selector execution while becoming unreplayable under the
  canonical matrix validator.
- Treat paired User runtime signing as daemon Invocation boot state, not as a
  CLI `start` post-ready compatibility repair. Device/Hub/_system identities
  still belong to process entry boot; paired User managed signing must be
  ensured and registered into runtime trust before the Invocation transport
  advertises readiness.
- Treat all-zero `credentials.user_id` as invalid persisted runtime state.
  Loading such credentials now fails with an explicit re-pair diagnostic
  instead of deriving `user/0000...` and later producing
  `AUTHORITY_SUBJECT_MISMATCH` under admission.
- Treat plugin realtime transport as a single declared carrier, not a
  primary/fallback pair. A plugin that wants WebRTC must satisfy WebRTC's
  activation roles; the daemon must report blocked when those roles are
  unavailable instead of selecting `invoke_bidi` or `invoke_stream` as an
  architectural fallback.
- Treat remote desktop InvokeBidi and preview-stream carriers as diagnostic
  product transports, not fallback transports. They may be exposed in the
  transport capability list, but their role, descriptions, backend labels, and
  internal projection methods must not imply a legacy or fallback architecture.
- Treat schema-less owner projection cursor files as retired local state, not
  migration input. Owner publication cursors are schema-v2 lifecycle facts; a
  missing `schema_version` must fail closed and ask the operator to delete and
  republish instead of synthesizing `generation = 1` / `active` lifecycle.
- Treat retired LocalDevice resource subjects as unsupported local state, not
  migration input. Device-local media resources are device-stream subjects from
  creation; if `resources.json` still contains generic/pre-join local-device
  rows, the daemon must fail closed and require local state republish rather
  than rewriting subject authority during upsert.
- Treat recorded artifact `content_type` as producer evidence, not a CLI
  inference. A recording manifest must preserve content type from the stream
  payload/frame; if the producer omits it, recording fails closed instead of
  writing a guessed media type based on ability kind.
- Treat bare remote node ids as directory selectors, not as enough material to
  construct a canonical target. CLI remote-device resolution must use an
  explicit canonical device URA or a `federation.discover` directory hit; a
  miss must stay unresolved instead of minting a local-realm device owner.
- Treat bare default-tenant EAL agent registry keys as retired local state, not
  dispatch aliases. EAL source shorthand may still parse `claude` into
  `default/claude`, but runtime registry lookup resolves only the canonical
  `AgentId::Display` key and requires stale registry state to be rebuilt.
- Treat agent run persistence as a child of the validated `AgentDirectory`
  root. `RunDir` must not derive `agents_root()/name`; dispatch already owns
  registry `root_path` validation, so persistence receives that root directly
  and the legacy workspace helper is removed.
- Treat control discovery as the only authority for the running daemon's local
  invocation URA. `local_daemon_ura()` now fails when the daemon has not
  published its identity instead of falling back through device credentials or
  the unpaired `default/local` device URA.
- Treat chat session `index.json` as the only session inventory and pointer
  authority. Transcript JSONL files are content records; they must not
  reconstruct missing/corrupt inventory state, and lifelong/latest pointers may
  only name sessions already present in the canonical index.
- Treat MCP reflection mode as a closed configuration state machine. Absence
  maps to the documented default `lazy`, but malformed operator input is not a
  lifecycle state and must abort daemon registry assembly instead of warning
  and running as `lazy`.
- Treat `files.put` metadata as producer evidence. The canonical files resource
  store must not derive `content_type` from `filename`, and must not synthesize
  a display filename when the producer omitted one. Product shims may translate
  their own external compatibility defaults before calling the resource
  surface, but the resource surface itself only accepts explicit runtime facts.
  Those facts are persisted as immutable per-sha metadata; the same bytes with
  different metadata is a conflict because the URA only names the sha.
- Treat `files.get` as a content-addressed resource read API, not a Pages path
  API. Generic files reads accept exactly one of `sha256` or canonical resource
  `ura`; Pages keeps its own `{path}` route where project file paths are the
  product model. `files.get` and `files.list` must read persisted metadata and
  fail closed on pre-metadata/orphan blobs instead of sniffing or hiding them.
- Treat unfiltered `federation.discover` as an operator/audit capability only.
  Product surfaces (`device list`, `runtime status`, `doctor`, remote-device
  bare-node resolution, and `agent.discover` federation expansion) must provide
  the paired user's `local_user_id` and use the user-binding filter. A caller
  that has no paired user context cannot safely read a cross-realm directory.
- Treat federated directory filter dependencies as route-registration-time
  lifecycle facts. Tests now assemble `FederatedBindingsStore` before registering
  exact LocalRuntime routes; a user-scoped request made against a route without
  those dependencies returns a canonical failure instead of falling through to
  unfiltered discovery.
- Treat `agent.toml` schema stamps as mandatory durable state. Missing
  `schema_version` is no longer the current implicit schema; it is retired
  pre-stamp local state. Current production writers and test fixtures must go
  through `AgentSpec::new(...).to_toml_string()` so the schema authority is one
  canonical writer rather than scattered handwritten TOML.
- Treat global skill package names as metadata facts, not directory-layout
  facts. A global skill directory without `SKILL.md` frontmatter `name` is
  unnamed retired local state and must not appear in list/tree/read routes.
  Directory names remain available as `SkillSource.subpath` provenance only.
- Treat product fleet directory failure as unavailable runtime state, not an
  empty directory. `easynet status` may render local lifecycle facts first, but
  it must not turn user-scoped `federation.discover` failures into "0 nodes";
  signer, admission, descriptor, and namespace failures need to remain visible
  to the operator.
- Treat `easynet device show` as a descriptor-bound inspection result, not a
  local repair composite. Remote inspection now requires a canonical Device
  URA, and the ability list must be supplied by `node.describe`; local
  `meta.list_abilities` is not a fallback for missing inspected-device facts.
- Treat `easynet device remove` as a canonical owner mutation, not a same-realm
  convenience selector. The remote substrate to revoke must be named by a
  canonical Device URA; current pairing credentials may identify the caller and
  protect self-removal, but they must not synthesize the target owner.
- Treat `easynet plugin list/status` as daemon-authoritative product status
  queries. Local package indexes and desktop companion process observations are
  configuration/manager inputs, not the runtime status authority exposed to
  product callers. When the daemon control ability is unavailable, the command
  must fail with a daemon-required diagnostic instead of showing an offline
  projection.
- Treat `easynet federation peers` as an operator configuration inspection
  surface with fail-closed parse semantics. Missing config files remain a fresh
  install empty state, but existing unreadable/malformed files and malformed
  peer/hub entries are not empty topology; they are invalid local authority
  inputs that must be surfaced.
- Treat `easynet doctor` agent inspection as a daemon-projection diagnostic.
  If `agent.list` is unavailable, doctor reports that unavailable state and
  stops the agent section instead of probing default local CLIs. Invalid daemon
  runtime rows fail the section, while local CLI probing is selected by an
  explicit runtime-kind mapping owned by the CLI command layer.
- Treat `easynet device show` state rendering as owned by the `node.describe`
  schema. The command no longer maps old numeric Axon SDK enum states or
  missing fields into `UNKNOWN`; non-string state is invalid describe output
  and must fail closed before rendering substrate status.
- Treat `easynet ability list` KIND as an owner-URA projection. The command no
  longer lets the legacy `fulfilled_by` handler tag override device/hub/agent/user
  classification, because handler implementation metadata is not a route or
  catalogue ownership authority.
- Treat `easynet device list` as a strict projection of user-scoped
  `federation.discover`. Device rows are no longer repaired from incomplete
  directory entries: missing `node_id`, missing/unknown `status`, and
  `agent_ura`/`node_id` mismatches are invalid read-model state that aborts the
  listing rather than producing non-routeable device rows.
- Treat PrincipalLifecycle explicit public-key input as complete key-binding
  evidence. The CLI no longer converts a missing `--key-id` /
  `--replacement-key-id` into an empty key id and omitted request field; users
  either use daemon-managed custody or provide both public key and key id.
- Treat PrincipalLifecycle `proof_ref` as mandatory caller proof material. The
  CLI no longer derives `proof:<idempotency_key>` for `principal create` or
  `principal bootstrap`; operators must provide the proof reference explicitly,
  matching every other PrincipalLifecycle mutation command.
- Treat FFI caller signature identity as explicit proof input. The FFI
  invocation parser no longer converts `signer_public_key_base64` into
  `key_id_hint` and no longer accepts a missing hint as an empty string. Public
  key bytes are verification material; key identity is a separate authority
  fact that must be supplied by the signer/caller boundary.
- Treat authority binding verification as tuple-completeness validation before
  proof comparison. Delegation/session metadata may only be compared against
  explicit caller, callee, and subject URAs. The admission facade no longer
  turns missing envelope identities into empty strings and then reports
  authority mismatch diagnostics.
- Treat descriptor catalog rows as schema-bound provider output once they
  match the requested selector. Rust FFI, Go SDK, and Python SDK must all reject
  incomplete matching rows instead of defaulting missing fields to empty
  strings, skipping the row, and surfacing a false `NOT_FOUND`.
- Treat runtime caller signer custody as an explicit identity state machine.
  User callers are managed, subject-bound signing identities; Device,
  Authority, and Agent callers are runtime-owner identities. Invalid or
  non-principal caller URAs are rejected before key-service lookup, so a User
  caller can no longer degrade into `keyring entry not found: <user_ura>` from
  the runtime-owner path.
- Treat paired User signing identity as required Device/Both daemon boot state.
  `register_paired_user_runtime_signer()` no longer skips missing canonical
  user credentials; the daemon must fail before Invocation readiness instead of
  advertising a runtime that later cannot sign user-as-caller descriptor
  invocations.
