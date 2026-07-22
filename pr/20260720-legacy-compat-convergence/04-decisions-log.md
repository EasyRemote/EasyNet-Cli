# Decisions log

Decisions will be appended as root-fork choices are made.

## 2026-07-22 runtime-owner signer User custody

- Treat runtime-owner signing and managed-user signing as separate explicit
  custody states. Agent, Device, and Authority URAs may use
  `RuntimeSigningIdentity`; User URAs must use the managed signing inventory
  through `load_runtime_caller_signer`.
- Reject User URAs before provider/key-service lookup inside
  `RuntimeSigningIdentity::load`. This turns a misrouted user caller into a
  deterministic local custody error rather than a legacy
  `keyring entry not found` runtime-owner lookup.
- Keep paired-user boot provisioning in `register_paired_user_runtime_signer`.
  That path ensures/validates the managed user key before Invocation transport
  assembly; the new guard prevents any remaining or future direct
  `RuntimeSigningIdentity` path from bypassing that model.
- Add architecture and SPEC v2 gates so the runtime-owner signer cannot regain
  arbitrary-URA behavior.

## 2026-07-22 SDK history authority subject binding

- Treat `invocation.history.list` as an authority-bearing observation
  capability, not as a normal owner-scoped invocation capability. Session
  authority for history must match the receipt query `subject_ura` exactly
  before the receipt provider is called.
- Preserve owner-aware session admission for ordinary descriptor-bound
  invocation. Removing that globally would narrow the canonical runtime model
  incorrectly; the defect was history reusing the wrong predicate.
- Add dedicated Go/Python exact-subject helpers for history instead of adding
  product-specific checks at CLI/UI call sites. This keeps the SDK as the
  canonical runtime model and prevents EasyNet/EasyRemote consumers from
  implementing their own history authority rules.
- Upgrade both convergence gates so a future reintroduction of
  owner-equivalent history subject expansion fails before product smoke tests.

## 2026-07-21 ability discovery candidate projection

- Treat minted discovery candidates as schema-bound runtime read-model rows.
  A minted row must carry a canonical Ability URA in `qualified_name`; malformed
  values fail the discovery command instead of being counted and dropped.
- Treat missing `candidates[]` as unavailable/corrupt discovery output, not as
  an empty search result. Empty arrays remain the valid no-candidate state.
- Preserve the separate unminted identity state. Rows with
  `identity_state != "minted"` and no `qualified_name` still project as
  explicit non-callable candidates with diagnostics, because that is a real
  lifecycle state rather than corrupt minted catalogue data.
- Remove the `skipped_unparseable` report field instead of keeping a
  compatibility placeholder that suggests partial success is acceptable.

## 2026-07-21 ability recording resource inventory projection

- Treat `meta.list_resources` as the resource inventory read-model authority
  for automatic `ability record` subject selection. Product recording code may
  select the first valid mic/camera resource, but it must not repair or skip
  malformed rows.
- Treat missing resource rows and malformed resource rows as different states.
  An empty resources array means no matching device inventory exists; a row
  missing canonical `resource_ura` means the daemon read model is corrupt or
  schema-incomplete and must fail closed.
- Keep URA validation delegated to Axon's canonical parser through
  `crate::core::ura::parse_ura`; the CLI projection only checks that the
  parsed kind is `Resource`.

## 2026-07-21 pairing auto-wire credential facts

- Treat blank `realm` / `node_id` in pairing credentials as malformed runtime
  state. These fields bind the local device identity, trust entry, and
  federated peer target; silently ignoring them turns a local credential defect
  into later `AUTHORITY_SUBJECT_MISMATCH`, missing signer, or descriptor
  visibility failures.
- Preserve only the explicit environment no-op: when no daemon-config exists,
  there is no local hub-mode federation table to write. That condition is
  separate from incomplete credential material.
- Treat `runtime start` realm-trust auto-wire as part of invocation-readiness,
  not a best-effort compatibility repair. Startup now fails with a local
  context-rich error if the required trust facts cannot be wired.
- Keep validated pairing state cohesive through `PairingTrustFacts`. The
  file-projection helper consumes this typed state so the implementation does
  not split into outer validation plus inner raw-string interpretation.

## 2026-07-21 user-device directory projection

- Treat `PresenceRegistry` as the local runtime presence authority, not as a
  product hint source. Same-realm rows that look like Device URAs but are not
  canonical Device URAs are corrupt runtime state and now fail
  `federation.list_user_devices` instead of producing empty `node_id` rows.
- Treat peer `ListUserDevicesResponse` as schema-bound input. The merge
  boundary validates canonical Device URA, non-empty matching `node_id`, and
  non-empty status before stamping `origin_realm`/`hub_endpoint`.
- Treat product-selected `peer_hub_urls` as exact fanout scope. Empty scope is
  a valid empty answer; selected peers require a configured federation client,
  trusted hub entry, origin realm, successful dial, and valid response. Any
  selected-peer failure is returned as failed/unavailable state, not hidden as
  an empty or partial device list.

## 2026-07-21 namespace proxy resolve projection

- Treat `namespace.proxy_resolve` peer URLs as exact product-selected fanout
  scope. Empty scope remains a valid empty non-dispatchable resolve answer;
  selected peers require a configured federation client, trusted hub entry,
  successful peer invocation, and valid peer resolver output.
- Treat peer `namespace.resolve` output as a canonical resolver projection.
  The proxy may merge canonical records but must not skip missing `records`,
  synthesize empty merge keys, or accept retired camel-case `recordType` rows.
- Treat peer fanout errors as unavailable resolver state. Returning a partial
  resolve answer after one selected peer failed would make product route
  visibility depend on error timing instead of explicit runtime state.

## 2026-07-20 InvokeBidi receipt payload projection

- Treat `LocalBidiFrame` as the support-layer owner for projecting
  `InvokeBidiDown` frames into product-consumable JSON. Local daemon and remote
  target bidi drains now transport frames and delegate projection; they do not
  independently interpret receipt payload schema.
- Preserve lossless `data_b64` only for `BinaryChunk`, where bytes are the
  actual frame payload. Receipt payload bytes are receipt projection facts:
  non-empty payloads must declare a JSON content type and parse as JSON.
- Treat malformed/non-JSON receipt payloads as unavailable receipt projection
  state. Product callers should receive a direct projection error, not a
  synthetic JSON object that can be mistaken for verified receipt facts.

## 2026-07-20 cross-Hub peer invocation subject state

- Treat cross-Hub peer envelope subject selection as explicit state, not an
  optional caller-envelope fallback. `ForwardedCaller` preserves product
  invocation provenance; `ExplicitSubject` covers daemon-owned fresh peer
  requests such as federated key resolution.
- Treat missing forwarded caller URA as invalid tuple input. It must fail
  before peer signing/network dispatch instead of becoming a target-self
  subject and producing later descriptor or authority errors.
- Keep subject normalization in the signing step: Hub/User provenance may still
  normalize to a peer-owned descriptor-bound ability subject, but the input to
  that normalization must be explicit and parseable.

## 2026-07-20 plugin wire profile projection authority

- Treat plugin catalog registration and plugin bidi wire profile lookup as two
  projections of one daemon-owned `PluginRuntimeState`. The daemon default
  `PluginRuntimeManager` is now fallible; package-state load failure aborts
  catalog assembly instead of exposing a live manager with a core-only wire
  registry.
- Treat invocation transport as a consumer of the catalog-owned plugin runtime
  manager, not as a second plugin profile reader. If the manager is absent,
  transport assembly fails instead of independently reloading default plugin
  state and downgrading to core profiles.
- Remove process-global wire lookup helpers that swallow
  `load_default_profile()` errors. Runtime dispatch must use the explicit
  `AbilityWireRegistry` handle injected from boot so route visibility and bidi
  wire selection stay bound to the same plugin state that registered the
  ability.

## 2026-07-20 Context clipboard history read model

- Treat `context/clipboard.jsonl` as the clipboard history read-model
  authority. Missing file remains a valid empty history; unreadable files and
  malformed JSONL rows fail list/get/remove instead of being skipped or
  reclassified as "not found".
- Keep `context.clipboard.remove` byte-preserving for non-target rows: the
  mutation validates every row before rewriting, then drops only the selected
  row.

## 2026-07-20 API key credential store lifecycle

- Treat `api_keys.toml` as credential authority state, not a cache. Missing file
  remains a documented fresh-install empty state; malformed/unreadable existing
  files fail create, list, revoke, and bearer-token resolution.
- Keep operator convenience `api_keys.local.toml` separate from the credential
  authority. The local default token cache may remain optional because it is not
  the authority used to admit bearer tokens.

## 2026-07-20 EAL agent registry dispatch authority

- Treat the registered-Agent registry projection as the sole Agent lookup
  authority for EAL Agent-target dispatch. Dispatch construction reads exactly
  that projection and does not depend on hosted-Agent identity inventory.
- Treat malformed/unreadable registry state as unavailable Mission dispatch
  state. It is not equivalent to an empty first-run registry and must not be
  rewritten into `AgentRegistry::default()`.

## 2026-07-20 ability catalogue descriptor facts

- Treat `meta.list_abilities` as the sole descriptor read-model authority for
  product catalogue rows. The CLI may filter and render rows, but it must not
  rebuild missing descriptor refs from adjacent hash/version/action fields.
- Treat missing `abilities` arrays and missing/invalid row `descriptor_ref`
  fields as unavailable/corrupt catalogue state, not as an empty catalogue or
  repairable projection.

## 2026-07-20 local daemon loopback subject policy

- Treat daemon-local root invocation subject as an explicit system policy, not
  as `Option::None` at the transport layer. `invoke_local_ability(...)` owns
  the daemon-self root policy.
- Treat `invoke_local_ability_with_subject(...)` as explicit tuple ingress.
  It now requires a concrete `subject_ura`; callers that have product/public
  context must choose that subject before entering local gRPC tuple planning.

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
  the current schema only; stale rows must be cleaned by the operator/rebuild
  path rather than rewritten by resource persistence.
- Treat authority clock availability as part of session authority projection
  state. A clock that cannot be represented as Unix epoch milliseconds is not a
  valid input to authority expiry validation and must surface as
  `AUTHORITY_CLOCK_UNAVAILABLE`, not epoch zero.
- Treat invocation signer custody as authority-backed, not key-backed. Raw
  key-service signer capabilities remain signing mechanisms, but descriptor-
  bound invocation authority is projected only from self-signed owner authority
  or a hosted-agent signing lease.
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
- Treat Node Runtime Core authority metadata as executable tuple authority,
  not opaque metadata. `InvocationDraft` now rejects delegation/session
  authority that does not admit the canonical caller, callee, subject, action,
  and ability before transport. This aligns Node with the Python/Go authorized
  runtime binding model and prevents product UI paths from discovering
  authority-subject mismatch only after daemon admission.
- Treat Node type tests as product-neutrality evidence, not a compatibility
  import harness. Removed product symbols such as `AdminClient` must be checked
  by inspecting runtime exports and declarations, never by importing them and
  relying on a type-error annotation. Generic runtime tests must use typed
  authority metadata so the test suite cannot preserve opaque authority
  placeholders after the SDK moves to tuple-bound preflight.
- Treat `AuthorizedRuntimeSession.history` as part of the same canonical
  invocation state machine as `invoke`, not as a raw receipt-provider bypass.
  A history list request must carry complete runtime tuple facts and an
  authority artifact that admits the caller, callee, subject, and
  `invocation.history.list` scope before any provider dispatch. This keeps
  product UI/device history paths from submitting a Device subject under a User
  session authority and discovering the mismatch only after daemon admission.
- Treat descriptor resolver caller-signer absence as an identity/authority
  failure, not a route miss. The C ABI numeric result now uses
  `ERR_PERMISSION_DENIED` while preserving canonical
  `CALLER_SIGNER_UNAVAILABLE` projection, so wrappers that only understand
  numeric ABI classes no longer convert key-service absence into
  `ABILITY_NOT_FOUND`.
- Treat `federation.resolve_key` trust material as authority state, not a
  best-effort read-model field. A resolver miss remains `None`/`NotFound`, but
  an existing entry with invalid base64 or non-Ed25519 key length is corrupt
  authority state and must surface as `FailedPrecondition`; it must never be
  projected as `public_key_hex: ""`.
- Treat plugin sidecar stderr as product failure evidence, not disposable debug
  text. The host may keep returning captured stderr as a diagnostic string, but
  binary data, read failures, and reader panics must remain visible; they must
  not be normalized to empty stderr on process failure, timeout, stream, or
  bidi terminal paths.
- Treat a DEC-EU user key bucket as one principal authority snapshot. Multi-key
  admission may verify against any returned key, but the resolver must first
  prove every bounded bucket row is valid key material. A corrupt row is not
  optional and must fail the principal closed instead of being skipped while
  other keys remain admissible.
- Treat hub `federation.resolve_key` responses consumed by device trust sync as
  schema-bound trust evidence. `public_keys_b64` is the canonical response
  field for both single-key and multi-key principals; legacy `public_key_b64`
  fallback and malformed-row skipping are not allowed in the authority import
  path. Empty `public_keys_b64` remains the explicit hub-miss state.
- Treat Pages fetch output as resource evidence, not optional HTTP framing.
  The serve adapter may map fetch/projection failures to HTTP status codes, but
  it must never synthesize response bytes, MIME type, attachment policy, or
  sha256. Malformed page.fetch output is an upstream projection failure and
  must not become HTTP 200 with empty/defaulted fields.
- Treat ability catalogue authority context as mandatory assembly input.
  `RegistryBuildConfig` and `RegistryDaemonBuildConfig` now carry concrete
  `AbilityAuthorityContext`; daemon boot and deterministic snapshot builders
  must select their authority profile before catalogue assembly, not rely on
  `Option::None` being repaired from local environment.
- Treat ability publication summaries as route/catalog evidence. Local
  publication, federation directory merge, route resolver, local discover, and
  `node.describe` must propagate corrupt projection state as unavailable or
  refused runtime state. They must not skip malformed summaries or synthesize
  empty ability arrays that make product callers believe a route simply is not
  visible.
- Treat desktop companion status as a lifecycle observation. Runtime status may
  keep rendering `desktop_companions` for successful DTOs, but plugin default
  state failures and companion DTO projection failures must be emitted as
  `desktop_companion_errors` / `companion_error`. A broken companion projection
  is not equivalent to an absent companion package.
- Treat mission.think's curator owner catalog as authoring authority, not a
  best-effort prompt hint. Missing owner rows can remain a first-run empty
  catalog, but unreadable/corrupt Agent registry projection must stop the
  curator at `stage = "catalog"` instead of letting it author against false
  empty route visibility.
- Treat schedule due selection as a lifecycle observation, not a best-effort
  cache read. A poisoned schedule cache or corrupt enabled cron row means the
  tick state is unavailable for this iteration; it must be reported explicitly
  instead of becoming an empty due-fire list.
- Treat the schedule list read model as the same lifecycle observation plane as
  due selection. The public `schedule.list` ability, Kernel schedule snapshot,
  context loader, and tick runner must observe unreadable schedule state as
  failure, not as an empty catalog or `null` schedule row.
- Treat schedule context next-fire projection as snapshot-derived state.
  Context loaders that already hold a `ScheduleEntry` must compute from that
  entry through schedule core validation; they must not re-query by id and
  collapse next-fire errors into absent context.
- Treat live session index availability as runtime state, not a discovery
  cache detail. Unknown session may remain an empty attach snapshot, but a
  poisoned/unavailable session index must fail `session.list`, `session.attach`,
  and Kernel session snapshot paths explicitly.
- Treat discuss room registry availability as runtime state, not as a
  best-effort discovery detail. No rooms is a valid empty list only after the
  registry is readable; a poisoned/unavailable registry must fail Kernel room
  listing explicitly.
- Treat loop cache availability as lifecycle state. A stale loop id may remain
  a not-found result, but a poisoned/unavailable loop cache must fail
  `loop.status`, `loop.subscribe`, resume, Kernel loop status, and debug
  projection explicitly instead of becoming unknown-loop, empty-loop-list, or
  zero-loop state.
- Treat chat cross-agent ability discovery as route/context authority, not as
  an optional prompt enhancement. A registry with no other agents may produce
  no cross-agent hint, but an unreadable Agent aggregate projection must fail
  RPC and stream chat before dispatch; otherwise products receive a false
  "no peer abilities exist" prompt.
- Treat the permission pending queue as admission/operator state. A
  non-subscriber permission broker can legitimately expose no pending queue,
  but a subscriber broker whose pending queue cannot be read is unavailable
  admission state. `Kernel::pending_permission_requests`,
  `consent.list_pending`, and `consent.subscribe` must propagate that failure
  instead of reporting an empty queue or `null` request rows.
- Treat all-zero principal ids as invalid authority material at the daemon
  admission boundary. SDK language guards remain useful, but they are not the
  authority boundary because product ingress can submit raw metadata. Delegated
  and session authority validators must reject the all-zero placeholder before
  subject/audience matching so stale defaults do not become misleading
  `AUTHORITY_SUBJECT_MISMATCH` failures.
- Treat C ABI descriptor diagnostics fallback as descriptor resolution state,
  not generic ability discovery. A diagnostics catalog miss means the provider
  could not prove a descriptor ref for `(callee_ura, ability, call_mode)`;
  returning generic `NOT_FOUND` makes products collapse it with absent
  abilities and masks the signer/admission/route distinctions surfaced by the
  Rust FFI projection. Go and Python now converge on
  `DESCRIPTOR_NOT_FOUND`, and R91 keeps this boundary typed.
- Treat invocation attempt audit as required pre-runtime observability, not
  best-effort logging. Axon's receipt ledger remains canonical after runtime
  admission starts, but malformed targets, unwired routes, signer/admission
  rejection, and route-selection failures can happen before Axon creates an
  invocation id. The daemon must either record that attempt or fail closed; it
  must not continue with a disabled handle, skip corrupt rows, or report an
  empty product history when the attempt ledger is unavailable.
- Treat session prelude `federation.resolve_key` output as schema-bound trust
  evidence, not backward-compatible discovery output. Only canonical
  `public_keys_b64[]` is accepted; legacy `public_key_b64`, malformed JSON,
  non-string keys, and empty keys fail instead of being repaired or skipped.
  Paired-user trust sync must pin resolve with the locally published
  `presented_pubkey_b64` so key import is bound to the key material the local
  runtime actually holds.
- Treat FFI descriptor catalog rows as provider evidence, not optional route
  hints. `meta.list_abilities` and system descriptor catalogs feed SDK
  descriptor resolution; malformed rows must fail closed as provider payload
  errors instead of being skipped and reclassified as invisible routes,
  `descriptor_ref not found`, or `No browser.open_session route is visible`.
- Treat remote descriptor probes as signed runtime invocations, not local
  descriptor conveniences. Once descriptor resolution has to call remote
  `meta.list_abilities`, `caller_ura` is authority material and must be
  supplied by the SDK tuple; the resolver must not synthesize it from
  `runtime_owner_ura` or any ambient local runtime state.
- Treat authorized receipt history filters as authority-bearing query scope,
  not passive read predicates. The session authority binds the runtime call
  tuple; SDK history filters may narrow that tuple but must not substitute a
  different caller, callee, or subject set after authority validation.
- Treat product device visibility as route visibility, not directory
  visibility. Realm directory rows remain useful discovery evidence, but a
  remote device is not selectable by product surfaces until the daemon proves a
  signed health route to that owner. Probe-failed rows are preserved under
  `unavailable_nodes` / network-health diagnostics and explicit
  `node.describe` fails closed instead of returning stale ability summaries.
- Treat `qtype` as the public namespace resolver state-machine selector, not
  a convenience hint. `namespace.resolve` and `namespace.proxy_resolve` must
  reject missing, empty, unspecified, numeric, or shorthand qtype values before
  route selection or peer fanout; products must submit the canonical
  `ResolveType` enum string that describes the operation they intend.
- Treat `realm-trust.toml` as daemon-owned trust aggregate state for operator
  federation peer inspection. The CLI no longer owns a loose `toml_edit`
  parser that defaults missing trust roles or accepts non-canonical hub
  identities; it consumes `RealmTrustAnchor` and projects only validated trust
  entries.
- Treat a hub-role trust row without complete schema-B dial facts as
  unavailable federation peer state, not as a trusted hub. `trusted_hubs`
  output now means the same thing the cross-hub dialer requires: canonical
  peer hub URA plus non-empty `origin_realm`, `hub_endpoint`, and
  `tls_ca_pem_path`.
- Treat remote-desktop causal-context receipts as proof input, not optional
  UI context. If a caller declares scalar/list causal context, every receipt
  row must carry non-empty `receipt_ura` and `receipt_hash`; malformed rows
  fail the session operation instead of being skipped.
- Preserve owner-self consent only for true no-receipt states. A malformed
  receipt-bearing causal context is not equivalent to no causal context and
  must not fall through to `owner_self_consent`.
- Treat invocation-history filters as schema-bound observation scope. A
  present but malformed filter is not an omitted filter: it must fail before
  reading/projecting canonical ledger rows or pre-runtime attempt diagnostics.
- Keep canonical ledger and attempt-audit filters on one parser. Attempt rows
  exist specifically to explain signer/admission/route failures before Axon
  minted an invocation id; they must not use looser scope semantics than the
  receipt ledger they augment.
- Treat remote-desktop create-session preferences as durable session facts.
  Defaults are selected only when fields are absent; present malformed fields
  fail before session id minting, consent capture, lease creation, or transport
  negotiation state.
- Keep remote-desktop parser and ability descriptor schema in one contract.
  Nested `video` and `input_policy` / `input` fields are advertised by dynamic
  registration and static plugin TOML instead of being hidden parser-only
  behavior.
- Treat remote-desktop direct input frames as schema-bound local control
  commands. Missing payload facts are invalid frames, not empty clipboard
  writes, empty file drops, or platform-level key injection failures.
- Split malformed control frames from unsupported frame types. Missing,
  non-string, or blank diagnostic Bidi `type` is `invalid_frame`; only
  schema-valid but unsupported frame names are `unknown_frame`.
- Treat remote-desktop ICE candidate rows as signaling facts. Malformed local
  candidate rows no longer mean end-of-candidates, and malformed remote
  candidates fail before session storage.
- Keep explicit end-of-candidates distinct from malformed candidate rows.
  `null` and empty-string `candidate` remain valid end markers; missing or
  non-string `candidate` is invalid signaling schema.
- Treat desktop companion desired-state rows as lifecycle authority. Missing
  file / absent row remains the fresh-install disabled state; malformed
  existing rows fail instead of being repaired into disabled.
- Reject duplicate companion desired-state rows. Reconciliation and status
  projection must not depend on first-match TOML order for the same
  `(id, version)` lifecycle identity.
- Treat installed plugin active state as package authority, not a loose cache.
  Missing state files remain valid fresh-install empty state, but an existing
  `plugins.toml` or `plugin-lock.toml` must be schema-complete and uniquely
  identify active packages.
- Keep active-state shape validation in `PluginStateToml`, not in
  `PluginPackageIndex`. The index consumes active package authority and loads
  packages; it must not define a second TOML interpretation path.
- Treat malformed active-state shape as lockfile-level unavailability in
  resilient daemon boot. Bad package directories can still produce row-level
  operator diagnostics, but blank identities, duplicate active package rows, or
  multiple active versions make the active set itself untrustworthy.
- Treat Context store projections as one read-model authority. `config.json`,
  `folders.json`, `favorites.json`, and `captures.jsonl` now share the same
  missing-file versus corrupt-existing-file semantics as clipboard history:
  missing means documented first-run empty/default, corrupt existing state
  means unavailable Context state.
- Keep display hints distinct from authority facts. Empty `preview` values
  remain valid because catalog rendering already has explicit display
  fallbacks; non-empty validation applies to identity, path, ability,
  reference, and timestamp facts that determine lookup and routing behavior.
- Treat the clipboard tracker as a consumer of Context config authority. If
  `config.json` is corrupt, the tracker does not capture and does not rewrite
  the file; it reports the unavailable config and retries later.
- Treat global skill pool roots and global skill packages as different states.
  A missing pool directory is an optional environment absence. A directory
  inside a pool that has skill package shape is package state and must declare
  semantic identity in `SKILL.md` frontmatter; if it is corrupt, inventory and
  publish lookup fail instead of projecting a smaller successful skill list.
- Keep global skill lookup and global skill listing on the same authority
  parser. `skill.list`, `skill.publish`, and exact `global:<pool>` lookup now
  use fallible global-pool APIs so a corrupt `SKILL.md` cannot be interpreted
  as "skill not found" on one path and an operator warning on another.
- Preserve non-skill user directories under global pool roots as ignorable
  environment noise. The fail-closed rule starts only after a directory has
  skill package shape (`SKILL.md` / `skill.json`).
- Treat Pages API ability discovery as route-registration authority, not a
  best-effort directory hint. Missing `api/` means a project has no dynamic
  backend routes; a corrupt existing `api` path means the project API surface
  is unavailable and registration must fail.
- Keep filename validation as the filter for user-authored non-route files.
  Invalid verb names are still skipped with an operator event because they are
  explicit non-ability artifacts; directory scan and file-type errors are not
  artifacts and must not hide routes.
- Treat Pages API request bodies as product ingress schema, not convenience
  payload hints. Empty body remains a valid no-payload state projected to JSON
  null; malformed non-empty JSON is invalid input and must return HTTP 400
  before ability dispatch.
- Keep Pages API body parsing centralized in the listener boundary. Downstream
  API handlers should receive already-decided invocation args and must not own
  a second "repair malformed body" interpretation path.
- Treat runtime projection loading as a three-state observation: absent,
  present, or unavailable. `Option<RuntimeSessionProjection>` is valid only
  after filesystem and JSON parsing have succeeded; it is not the right type
  for the loader boundary.
- Preserve corrupt `runtime.json` as operator evidence. Start preflight must
  not remove or overwrite a malformed projection as stale state, and stop
  planning must not decide cleanup from a projection read that failed.
- Treat device reset as a consumer of lifecycle status, not as a second
  runtime projection reader. Deleting credentials is destructive; if the
  runtime projection cannot be classified, reset must abort before mutating
  pairing state.
- Keep stale runtime projection cleanup explicit. `ProjectionPresentProcessMissing`
  is the only reset path that removes `runtime.json`, and cleanup errors
  return to the operator instead of being ignored.
- Treat MCP status as a consumer of lifecycle diagnostics, not as a second
  runtime projection reader. It may combine lifecycle status with the MCP
  health probe, but it must not decide whether malformed `runtime.json` means
  stopped, stale, or running.
- Preserve daemon facts when projection is missing. MCP status now reports
  lifecycle daemon evidence separately from `runtime.json`, so a live daemon
  with missing projection does not collapse into the fresh-install message.
- Treat the top-level help banner as a lifecycle report renderer, not a
  runtime projection reader. The banner's public API remains a pure `String`,
  but corrupt lifecycle metadata is still visible as `metadata unavailable`.
- Keep lifecycle-to-banner projection in one value object. `BannerDaemonObservation`
  owns the display mapping from lifecycle state-machine values so future
  banner copy changes do not reintroduce ad hoc runtime state checks.

## 2026-07-22 Auth agents canonical backend row projection

- Treat `/api/v1/agents` as a schema-bound backend read model for CLI table
  rendering. The CLI projects the canonical row fields directly instead of
  accepting retired `ura` / `name` aliases.
- Keep JSON output raw. Operators who request JSON still receive the daemon
  payload for diagnosis, while the human table refuses to turn legacy row
  shapes into canonical identity display facts.
- Add a SPEC v2 guard because this fallback is not observable through normal
  product smoke tests: a legacy backend row can make the UI look functional
  while preserving a second, undocumented agent inventory shape.

## 2026-07-22 Pages identity credential state classification

- Split credential loading into mandatory and optional states at the
  persistence boundary. `load_credentials_optional()` returns `None` only for
  the absent-file unpaired state; corrupt existing credentials remain errors.
- Make Pages identity resolution fallible. Device/Both daemon boot and the
  real-user smoke registry builder now call `PagesIdentity::try_from_env()`
  so a bad credentials file cannot silently remove user-rooted abilities or
  stamp them into a default realm.
- Preserve the explicit env override semantics. `EASYNET_PAGES_USER`,
  `EASYNET_PAGES_REALM`, and `EASYNET_PAGES_PORT` still drive dev rigs, but
  they no longer mask corrupt persisted pairing authority.

## 2026-07-22 Mission traditional agent target conflict naming

- Treat `call ... on "<registered-agent>"` as an EAL target-kind conflict,
  not as an implicit fallback path. The validator still rejects before run-dir
  persistence, but its type/function/test names now describe the active
  architecture rather than the retired behavior it prevents.
- Keep the agent registry lookup at the single MissionRunner entry point.
  The planner remains registry-free, and execution still receives already
  validated IR target kinds.
- Add a SPEC v2 guard so production Mission code cannot reintroduce
  `ImplicitAgentFallback` or `find_implicit_agent_fallback` terminology.

## 2026-07-22 Device settings fail-closed loader

- Treat `device_settings.json` as user configuration authority. Missing file
  remains first-run default, but existing unreadable, malformed, or
  unknown-field state is unavailable configuration, not permission to project
  defaults.
- Make settings loading fallible and migrate config display/mutation plus
  install-id generation to propagate loader errors before mutating state.
- Add a SPEC v2 guard because this fallback can silently disable
  `session_bridge_exec_enabled` or rotate stable install identity when a
  settings file is corrupt.

## 2026-07-22 Local API key default-token cache

- Treat `api_keys.local.toml` as local credential projection state rather than
  a best-effort convenience cache. Missing file remains the only valid
  no-default-token state; existing malformed, unreadable, unknown-field, or
  blank-token state now fails before `llm-api` constructs bearer auth.
- Keep the credential boundary cohesive by sharing one path helper between
  read and write. The reader no longer chains `.ok()?`, and the writer still
  owns creation and 0600 permission tightening.
- Add a SPEC v2 guard because product smoke tests can pass with explicit
  `--key` while the default-token path still preserves silent credential
  fallback.

## 2026-07-22 Runtime trust revocation credential projection

- Treat the local connection-state projector as part of the trust-revocation
  preflight, not a best-effort post-commit side effect. If existing local
  credentials are malformed, revoke fails before mutating the trust anchor.
- Preserve the explicit no-local-credentials state. A daemon without paired
  credentials can still perform trust revocation without recording local
  connection-state, but corrupt credentials are unavailable identity state.
- Keep the invalidator object pure over already-classified state. It receives
  `Option<RuntimeTrustConnectionStateProjector>` and never reads credentials
  during invalidation.

## 2026-07-22 Admission owner credential projection

- Treat local device owner facts as authority-bearing admission input, not a
  best-effort hint. Malformed local credentials now fail owner resolution
  instead of continuing policy evaluation with an unresolved owner.
- Preserve first-run/unpaired behavior. Missing credentials produce no local
  device owner fact, which keeps pre-join and hub-only paths explicit without
  hiding corrupt identity state.
- Make `resolve_owner` fallible so admission policy can distinguish malformed
  URA/credential state from ordinary no-match owner projection.

## 2026-07-22 Shared local device owner projection

- Move fail-closed credential classification into
  `owner_resolution::local_device_owner_fact` so bootstrap authority and device
  principal construction consume the same state model.
- Add an explicit bootstrap-authority unavailable state. A corrupt local
  identity source is no longer projected as `NotApplicable`, because that would
  continue admission as if no bootstrap authority existed.
- Make device principal construction fallible for malformed local credentials
  while preserving trust-anchor owner facts and missing-credentials first-run
  behavior.

## 2026-07-22 Node session authority subject binding

- Treat session authority subject binding as canonical SDK runtime validation,
  not product-specific defensive code. Node now rejects metadata/request payloads
  whose subject cannot be a daemon-admissible user subject or user-owned session
  resource.
- Preserve the public Node SDK API. The refactor adds internal semantic
  validation only; callers still construct `SessionAuthority` and
  `SessionAuthorityRequest` with the existing fields.
- Add a SPEC v2 guard and focused Node test so the SDK cannot regress to
  all-zero-only authority validation.

## 2026-07-22 Session prelude paired-user credentials

- Treat paired-user trust bootstrap as part of session readiness. A corrupt
  paired identity source now fails before `session.open`, rather than being
  projected as "not required" and leaving later remote invocation to discover
  missing user signer/trust state.
- Preserve the unpaired daemon case. `credentials.json` absence still means no
  paired-user trust sync is required; only existing invalid credentials become
  `CredentialsUnavailable`.
- Add SPEC v2 and architecture gates so prelude credential classification
  cannot regress to `let Ok(...) else NotRequired` fallback semantics.

## 2026-07-22 Descriptor-ref route selector failures

- Treat descriptor-ref parsing as a route selector state machine, not an
  optional public-name lookup. Canonicalization, descriptor ability extraction,
  and ability selector parsing now either produce a selector or a typed
  `ResolveRouteFailure`.
- Refuse descriptor owner mismatch before catalog/remote lookup. This prevents
  a descriptor-bound request for one owner from being reinterpreted as a local
  ability-name query against another owner.
- Extend architecture and SPEC v2 gates with negative fixtures for the retired
  `Option`/`.ok()?` descriptor selector path so future route work cannot
  reintroduce silent fallback semantics.

## 2026-07-22 FFI descriptor runtime owner precondition

- Treat native runtime owner identity as a descriptor-resolution precondition,
  not a cache optimization. `runtime_resolve_descriptor_ref_json` now fails
  with `CALLER_IDENTITY_UNAVAILABLE` when the session's control discovery
  cannot produce a daemon owner URA.
- Preserve explicit tuple semantics. Remote descriptor probes still require
  `caller_ura` from the SDK request; this change only removes the independent
  fallback that let missing local runtime identity be discovered later as
  signer/route/timeout noise.
- Add SPEC v2 and architecture guards against
  `runtime_owner_ura_from_session(session).ok()` inside the FFI descriptor
  resolver.

## 2026-07-22 Invocation history ledger URA projection

- Treat `ledger_ura` as receipt evidence ownership state rather than nullable
  UI metadata. Missing hosted identity remains a valid unjoined state; malformed
  or unavailable hosted identity fails closed.
- Split the projection into a fallible aggregate loader and a pure
  `host_device_agent_ura` parser so tests cover owner-shape semantics without
  depending on the operator's local state directory.
- Extend SPEC v2 and architecture gates to reject
  `ledger_resource_ura() -> Option<String>` and
  `load_hosted_identity_status().ok()?` fallback.

## 2026-07-22 Canonical ability catalog projection

- Treat hub-published ability state as a federation wire input, not a second
  runtime catalog. `HubPublishedAbilityStore` now stores validated
  `AbilityDescriptor` values rather than opaque `HubAbilityEntry` JSON.
- Reject malformed hub catalog snapshots and diffs before mutation. Diff
  application validates all added rows first, so a bad row cannot partially
  remove or replace good projection state.
- Keep `meta.list_abilities(scope=realm)` on the same public descriptor wire as
  local registry rows by serializing canonical descriptors and only stamping an
  empty `source` field as `hub:broadcast`.
- Treat FFI descriptor catalog dedupe as a schema boundary. Missing
  `descriptor_ref` or other identity fields now returns a catalog integrity
  error instead of silently filtering the row and manufacturing later
  descriptor-not-found diagnostics.

## 2026-07-22 Federation prelude receipt state machine

- Treat `federation.join` and `federation.heartbeat` response bodies as typed
  receipt inputs to the session state machine. They are no longer best-effort
  JSON decorations that can be skipped after decode failure.
- Reuse `ability_contract::parse_receipt` for join and heartbeat rather than
  hand-rolled `serde_json::from_slice`, removing duplicated weaker parsing
  semantics from the session initiator.
- Commit heartbeat revision-only diffs. An empty `added`/`removed` set is still
  a valid cursor transition when the hub advances `hub_abilities_diff.revision`.

## 2026-07-22 Heartbeat owner-projection refresh input

- Treat the owner-projection cursor file as canonical heartbeat input. The
  heartbeat builder must not submit a liveness update with an empty
  `refresh_owner_uras` set when local cursor state exists but cannot be parsed
  or validated.
- Preserve the legitimate first-boot case: `owner_projections::load()` already
  maps a missing file to an empty cursor file, so no separate compatibility
  fallback is needed at the heartbeat layer.
- Keep filtering by caller owner at the request edge. A heartbeat only refreshes
  the caller's owner projection lease, while the cursor loader remains the
  shared read-model source of all active owner cursors.

## 2026-07-22 Python SDK session-authority subject projection

- Preserve the canonical session-authority capability: a user session authority
  may admit exact user/session subjects and user-owned resources when ownership
  is proven by URA projection facts.
- Remove Python's duplicated substring matcher from
  `authorized_runtime_session.py`. Path text is not an ownership proof and must
  not make SDK-side authorization diverge from Go.
- Add a shared internal Python helper so runtime ability calls and authorized
  runtime sessions use the same subject-admission rule.

## 2026-07-22 Daemon exact-route descriptor-ref projection

- Treat descriptor-like route tokens as typed descriptor refs. They must parse
  into an `AbilitySelector` and match the envelope callee before exact route
  lookup.
- Preserve public function-name behavior for non-descriptor route tokens.
  Existing plain ability names still route by exact public name.
- Move descriptor-ref selector projection into
  `daemon::axon_bridge::descriptor_ref` so daemon exact routes and the main
  route resolver cannot drift into separate selector semantics.
