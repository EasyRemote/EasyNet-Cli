# Invariants

- No new product-specific abstraction may enter the SDK canonical runtime
  surface.
- No second route, proof, admission, receipt, or descriptor authority may remain
  after callers have migrated.
- A removed compatibility layer must have its callers migrated in the same
  change.
- Edge compatibility may preserve public behavior only by constructing complete
  descriptor-bound runtime input and delegating to the canonical model.
- Verification must include the narrow tests for the removed path and at least
  one architecture/conformance gate that proves the old path cannot reappear.
- User-role trust is a composite `(user_ura, pubkey)` fact. Runtime code must
  never resolve a bare user URA into one selected signing key.
- Descriptor selection at a generic runtime/provider seam must carry an
  explicit `call_mode`; RPC defaults belong only in higher-level convenience
  methods before they construct complete tuple input.
- Session inventory and pointer state must come from the canonical session
  index only; transcript content files must not become a discovery or repair
  authority.
- Runtime lifecycle configuration must be a closed state machine: missing
  config may select a documented default, but malformed config must fail
  closed instead of being rewritten to a valid lifecycle state.
- Canonical resource write surfaces must not infer producer facts. If a blob
  needs a filename or content type in its public record, the producer must
  submit that fact explicitly before the runtime stores or projects it.
- Canonical resource read surfaces must expose one selector model per
  capability. Product-specific path selectors belong to product abilities such
  as Pages, not to the generic content-addressed Files resource surface.
- Product federation directory reads must be user-scoped by default. An
  unfiltered cross-realm directory read is an explicit operator/audit capability,
  not a product fallback and not a shared helper default.
- If a caller supplies `local_user_id`, daemon dispatch must either apply the
  federated user-binding filter or fail closed. Missing filter dependencies
  (`FederatedBindingsStore`, `session_realm`) are invalid lifecycle assembly,
  not permission to return the unfiltered directory.
- Durable agent specs must be self-describing. A missing `agent.toml`
  `schema_version` is retired pre-stamp local state and must fail closed;
  current writers and test fixtures must use the canonical `AgentSpec` writer
  instead of hand-written implicit-schema TOML.
- Global skill pool identity must be declared by package metadata. `SKILL.md`
  frontmatter `name` is the public semantic package identity; filesystem
  directory names are physical source subpaths only and must not be used as
  fallback skill names.
- Product fleet status must not project directory read failures as empty
  directory state. Signer, admission, namespace, or descriptor failures are
  runtime facts that must surface as unavailable/error, not as zero peers.
- Product device inspection must not synthesize remote targets from bare ids
  or repair missing ability facts from the local catalogue. Remote device
  inspection requires a canonical Device URA, and hosted abilities must come
  from the `node.describe` payload returned by the inspected device.
- Product device removal must not synthesize revocation targets from bare ids
  or current-pairing realm. Removing a remote substrate requires an explicit
  canonical Device URA; self-removal remains the separate local `device reset`
  lifecycle.
- Product plugin runtime/status surfaces must be daemon-authoritative. The CLI
  may mutate the installed package index while the daemon is offline, but
  `plugin list` and `plugin status` must not project local package plans,
  companion manager observations, or stale daemon/local disagreement into
  runtime status facts.
- Operator federation peer inspection may treat a missing config file as an
  empty local configuration, but an existing unreadable or malformed
  daemon-config/realm-trust file is unavailable state. It must not be projected
  as "no peers" or "no trusted hubs".
- Operator federation peer inspection must consume the daemon-owned
  `RealmTrustAnchor` aggregate for `realm-trust.toml` schema validation.
  Hub-role trust rows are selectable cross-hub peer evidence only when they
  carry complete dial-eligible schema-B facts: canonical peer hub URA,
  non-empty `origin_realm`, non-empty `hub_endpoint`, and non-empty
  `tls_ca_pem_path`.
- Product doctor agent inspection must distinguish an empty daemon-owned agent
  registry from an unavailable daemon-owned agent registry. `agent.list`
  failures and invalid daemon projection rows are runtime facts, not permission
  to synthesize default local CLI probe results.
- Local CLI probing is a product diagnostic over declared runtime kinds, not a
  fallback descriptor authority. `external` agents have no default local binary
  probe, and `codex-app-server` maps explicitly to the Codex CLI probe instead
  of falling through a "not claude means codex" branch.
- Product device inspection must treat `node.describe` as a schema-bound
  projection. Device show may render unknown string states, but it must not
  translate missing or numeric legacy SDK enum state into display facts.
- Product ability catalogue grouping/classification must be owner-authoritative.
  `owner_ura` is the section and KIND classifier; legacy handler hints such as
  `fulfilled_by` may not override the canonical owner kind.
- Product device list must treat federated `DirectoryEntry` rows as
  schema-bound input. A projected device row requires a canonical Device URA,
  explicit matching `node_id`, and explicit supported status; missing fields,
  mismatched ids, or unknown status must fail closed instead of rendering empty
  ids, active state, or `UNKNOWN`.
- PrincipalLifecycle CLI key binding must not accept anonymous external public
  key projections. When the operator supplies public key material directly
  instead of using daemon-managed custody, the binding request must include an
  explicit non-empty key id.
- PrincipalLifecycle CLI commands must not synthesize proof references from
  idempotency keys. `proof.reference` is proof evidence supplied by the caller,
  not a receipt/idempotency namespace repair.
- FFI externally supplied caller signatures must carry explicit non-empty
  `key_id_hint` identity material. `signer_public_key_base64` is key material,
  not a key identity, and the FFI boundary must not project it into
  `key_id_hint` or default missing identity to an empty string.
- Authority admission must verify complete canonical tuples before comparing
  authority facts. Missing or blank caller/callee/subject URAs are
  `ENVELOPE_INCOMPLETE` input defects, not empty-string identities that can be
  reclassified as caller, subject, or audience mismatches.
- Descriptor catalog resolution must fail closed on schema-incomplete matching
  rows. A row that matches the requested ability/call mode but lacks
  descriptor_ref, owner_ura, ability_ura, or public name is invalid provider
  output, not a "descriptor not found" miss and not a row that may be projected
  with empty strings.
- Runtime caller signer custody must be classified explicitly before any
  key-service lookup. User callers use the managed, subject-bound signing
  inventory; Device, Authority, and Agent callers use runtime-owner custody;
  malformed or non-principal URAs fail closed instead of being tried as owner
  key names.
- Device/Both daemon boot must not publish Invocation readiness unless the
  paired User URA is present, has a managed runtime signing key, and that key's
  public projection has been registered into runtime trust. Missing paired User
  identity is invalid boot state, not permission to defer repair until a
  remote descriptor invocation fails.
- Ability-target ingress must carry a complete invocation tuple before it
  reaches the generic runtime/provider seam. `subject_ura` is caller intent,
  not a fact that may be derived from an ability selector or descriptor
  projection, and `call_mode` absence is invalid provider input rather than
  permission to infer RPC.
- Local daemon loopback transport must not interpret a missing subject as
  target-self. Daemon-self root calls and explicit-subject calls are separate
  named policies; targeted/product/public ingress must supply a concrete
  subject before the gRPC tuple plan is built.
- Product ability catalogue readers must not synthesize descriptor facts.
  `meta.list_abilities` is the descriptor read-model authority; CLI facades
  must fail closed on missing or invalid `descriptor_ref` instead of rebuilding
  it from hash/version/action fields.
- EAL/Mission dispatch must not project unavailable or corrupt agent registry
  state as an empty registry. A missing registry file is a valid first-run
  empty state; an unreadable or malformed registry is unavailable dispatch
  state and must fail before child Invocation planning.
- Bearer API key credential state must be fail-closed. A missing
  `api_keys.toml` is a valid fresh-install empty store; an existing unreadable
  or malformed store is unavailable credential authority and must not be
  projected as "no keys" or overwritten by create/revoke/list flows.
- Context clipboard history is an append-only read model. A missing
  `clipboard.jsonl` is a valid empty history; an existing unreadable file or
  malformed row is unavailable/corrupt context state and must not be projected
  as an empty history, skipped row, or "clip not found".
- Node Runtime Core must validate typed authority metadata against the
  descriptor-bound invocation tuple before transport. Delegation/session
  metadata is not just shape-valid JSON; it must admit the draft caller,
  callee, subject, audience, action, and ability scope locally so predictable
  authority-subject errors do not leak into daemon admission as product-facing
  runtime failures.
- Node SDK type/runtime tests are part of the product-neutrality boundary.
  They must prove product clients are absent from runtime exports and
  `index.d.ts` without importing removed product symbols, and they must build
  generic runtime drafts with typed authority metadata rather than opaque
  compatibility placeholders.
- Authority metadata projection must not synthesize time. If the runtime clock
  cannot produce a Unix epoch millisecond value, session-authority projection is
  unavailable and must fail with an explicit authority error instead of
  defaulting to epoch zero.
- Invocation signing custody must be owner-authority backed. A raw key-service
  signer capability is not sufficient to issue descriptor-bound invocation
  signatures unless it is attached to a self-signed owner authority or a valid
  hosted-agent signing lease.
- Ability catalogue assembly must be authority-context complete. Registry
  build configs carry one concrete `AbilityAuthorityContext`; daemon boot,
  deterministic snapshots, and tests may choose different profiles, but the
  assembly core must never interpret `None` as local-environment authority.
- Product device visibility must be route-visible, not directory-visible. A
  remote device profile row from the realm directory is discovery evidence
  only until the daemon proves the device with a signed health probe; failed
  probes may be reported as unavailable evidence but must not enter selectable
  `nodes` or return stale ability summaries.
- Namespace resolver ingress must be schema-bound before route selection. The
  public `namespace.resolve` and `namespace.proxy_resolve` daemon abilities
  require an explicit canonical `ResolveType` enum string; missing, empty,
  unspecified, numeric, or shorthand qtype values must fail closed instead of
  being inferred from `query_name` / `ability_name`.
- Product user-device directory projection must be schema-bound at both local
  presence and peer merge boundaries. A selected peer scope is not optional
  discovery: missing federation transport, untrusted peers, malformed peer
  rows, and fanout failures are runtime unavailable/configuration facts and
  must not become an empty or partial successful device list.
- Product namespace proxy resolution must treat `peer_hub_urls` as an exact
  selected scope. Missing federation transport, untrusted peers, malformed
  peer resolve answers, and fanout failures are unavailable/configuration
  facts; they must not be merged into empty or partial
  `RESOLVE_ANSWER_KIND_NON_DISPATCHABLE` success. Peer records must use the
  canonical `record_type` field and canonical resolver enum strings.
- Pairing auto-wire requires complete credential facts. Missing or blank
  pairing `realm` and `node_id` are invalid runtime state, not successful
  no-ops; join/start must surface them at the local ingress stage instead of
  letting SDK calls fail later as descriptor, signer, route, or admission
  errors.
- Runtime start must not swallow local realm-trust auto-wire failures. If the
  device cannot write or prove the trust facts required by local
  self-admission, daemon boot is not invocation-ready and must stop before
  advertising route visibility.
- Product media recording resource selection must treat `meta.list_resources`
  output as schema-bound read-model state. A matching resource row with
  missing, blank, wrong-kind, or non-canonical `resource_ura` is corrupt
  resource inventory, not proof that no mic/camera resource exists.
- Product ability discovery must not report partial success after receiving
  corrupt minted candidate rows. Zero-score rows may be ranking misses and
  unminted identity rows may project as explicit non-callable candidates, but
  missing `candidates[]` or non-canonical minted `qualified_name` is corrupt
  discovery read-model state and must fail closed.
- Product consent/receipt-bound session access must treat causal-context
  receipt facts as schema-bound proof input. Missing causal context can remain
  a valid no-receipt state for explicitly modeled owner-self consent, but a
  declared scalar/list causal context with missing, blank, or malformed receipt
  fields is invalid proof input and must not be skipped or downgraded into
  self-consent or a generic receipt mismatch.
- Invocation history filters are authority-bearing observation scope. Optional
  filter fields may be absent, but when present `caller_ura`, `callee_ura`,
  `agent_ura`, `subject_ura`, `subject_uras`, `ability_ura`,
  `ability_uras`, `state`, and `trace_id` must satisfy the published schema
  before either canonical ledger records or pre-runtime attempt records are
  read/projected. Malformed filter scope must not widen to "all history" or
  collapse into an empty/no-match result.
- Remote-desktop session creation arguments are descriptor-bound product
  ingress, not best-effort UI preferences. Absent optional fields may select
  documented defaults, but present `mode`, TTL, `session_id`, `video`, and
  `input_policy` / `input` fields must satisfy the same schema advertised by
  the ability descriptor before a session id, consent grant, lease, media
  policy, or input policy is minted. Malformed preference fields must not be
  repaired into default media/input policy.
