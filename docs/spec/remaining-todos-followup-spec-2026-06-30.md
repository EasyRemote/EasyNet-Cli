# Remaining Follow-up TODOs — Execution Spec (2026-06-30)

Execution tracker for the original five follow-up items after the wrapper→CLI
runtime-refactor push (see `easynet-wrapper-cli-runtime-refactor-spec-2026-06-28.md`)
and the canonical-runtime convergence work discovered while completing them.
Each item below is self-contained: background, current on-disk state, the exact
change, and an acceptance gate. Section 6 is a binding expansion of the target,
not an optional feature: it closes the standalone-Hub principal lifecycle that
must exist before the staged HTTP pairing path can be removed.

The thirteen completed items (conformance §7.3 model, the three latent bugs,
§9.1.A registry convergence, §9.2 backend→product-wrapper, CLI user
signing-key facade, stable install-id rejoin, the bug-3 owner-prefix grammar
fix, the stale-JWT existence gate) are NOT repeated here — they are done,
built, and tested.

Execution update: §1, §2, §3 Phase 1, §4, §5, and §6 have been landed and
verified. §4 now has executable cross-language SDK runners, so the protocol-pack
vectors are not dead fixtures. Section 6 is
governed normatively by `daemon-sdk-requirements-v1.md` section 14 and is
accepted by the standalone-Hub plus Backend-present gates described below.

Conventions: `realm` is the federation/DNS namespace; owner-prefixed URAs
(`<username>.<agent>`) carry the **username slug**, subject/trust URAs carry the
**user UUID** (§15.1-3 dual grammar). `--features axon-pb` is the production
build; the default proto-free build must also compile.

---

## §1. Wire the conformance model into production startup (spec §9.1 item 7)

**Status: DONE** — implemented in `src/bin/easynet-daemon.rs`.

### Background

The typed conformance model (`daemon::ability::conformance`:
`BaselineAbility`, `HubBaseline`, `DeviceBaseline`, `RegistryConformance`,
`RuntimeAdminConformance`, `BaselineConformanceReport`) and its integration
gate (`tests/conformance_baseline_gate.rs`) already exist and pass. They prove,
at test time, that the daemon registry built by
`runtime::agents::build_registry()` satisfies the Hub/Device baselines.

Spec §9.1 item 7 requires this to be a **boot-time** invariant, not only a
CI-time one: a daemon that boots missing a baseline ability must fail loudly at
startup rather than silently serve a degraded surface that only a later
conformance test would catch.

### Current state

- `tests/conformance_baseline_gate.rs` runs `RegistryConformance` +
  `RuntimeAdminConformance` against the real registry — at TEST time only.
- `src/bin/easynet-daemon.rs` boot path does **not** invoke the conformance
  check. There is no startup assertion.

### The change

1. In the daemon boot sequence (`src/bin/easynet-daemon.rs`, after
   `build_registry()` and after the daemon mode — Hub vs Device — is known),
   run the matching baseline:
   - Device mode → `RegistryConformance::new(&registry).check("device",
     &DeviceBaseline::required_abilities())`.
   - Hub mode → both the Hub `LocalRegistry` slice
     (`HubBaseline::required_abilities()`) and
     `RuntimeAdminConformance::from_daemon_surface().check("hub", …)`.
2. On a non-conformant report, **panic at boot** with
   `report.panic_message()` (the gate test already uses this exact message
   shape), so the operator sees precisely which baseline ability is missing —
   the same failure CI would have produced, now at the point of deployment.
3. Place the call behind the same `mode` switch `run_as_hub` / device boot
   already branch on; do not duplicate the registry build.

### Acceptance

- Daemon boot with a complete registry: starts normally (no behavior change).
- Daemon boot with a deliberately-removed baseline ability: panics at startup
  with the baseline-missing message, before binding any listener.
- `tests/conformance_baseline_gate.rs` continues to pass unchanged.
- Build + clippy clean on default and `--features axon-pb`.

### Execution note

The daemon now runs the selected baseline conformance report after registry
assembly and before listener binding. Non-conformant startup emits the boot
failure stage and panics with the same `panic_message()` shape used by CI.

---

## §2. Consolidate the DaemonInvocation route list (spec §7.3 item 5)

**Status: DONE** — implemented in
`src/services/invocation_transport/daemon_invocation_service.rs` and
`src/daemon/ability/conformance.rs`.

### Background

§7.3 item 5 forbids a hand-mirrored second copy of the daemon's invocation
route list. The single source of truth for which `federation.*` / `namespace.*`
/ `identity.*` unary/stream routes the daemon serves is the `match` arms in
`services::invocation_transport::daemon_invocation_service`. The conformance
model's `DaemonInvocationSurface` type was built to assert against that source,
but the assertion was deliberately deferred (`tests/conformance_baseline_gate.rs`
header documents this) because honestly checking it requires exporting a route
const co-located with those `match` arms — and that file carried unrelated
uncommitted work.

### Current state

- `DaemonInvocationSurface` (the typed model) exists in
  `daemon::ability::conformance`.
- `bidi_dispatcher.rs` already exposes `RUNTIME_ADMIN_BIDI_ROUTES`
  (the bidi carrier routes) as a const — the pattern to follow.
- The unary/stream `federation.*`/`namespace.*`/`identity.*` routes are still
  only the `match` arms in `daemon_invocation_service.rs`; no exported const.
- `conformance_baseline_gate.rs` explicitly skips the `DaemonInvocation`
  surface with a comment pointing here.

### The change

1. In `daemon_invocation_service.rs`, co-locate a
   `pub(crate) const DAEMON_INVOCATION_UNARY_ROUTES: &[&str]` (and a stream
   counterpart if the stream routes diverge) **next to** the `match` arms, so a
   contributor adding a route arm sees the const and updates it in the same
   edit. Mirror the `RUNTIME_ADMIN_BIDI_ROUTES` placement/shape.
2. Wire `DaemonInvocationSurface::from_daemon_routes()` (or equivalent
   constructor) to read that const.
3. In `conformance_baseline_gate.rs`, add the deferred test row asserting the
   Hub baseline's DaemonInvocation surface against the const — removing the
   "deliberately deferred" note.
4. Add an in-file guard test (or a clippy-style comment) reminding that the
   const and the `match` arms must stay in sync; ideally a test that fails if a
   route appears in the const but not the match (or vice-versa), so drift is
   caught mechanically rather than by review.

### Acceptance

- A new route added to the `match` arms but not the const (or vice-versa)
  fails a test — no silent drift.
- `conformance_baseline_gate.rs` asserts the DaemonInvocation surface with no
  hand-mirrored list elsewhere.
- Build + clippy clean both features; existing dispatcher tests green.

### Execution note

The daemon invocation surface is now encoded as typed route enums colocated with
dispatch. Conformance checks consume those route tables and an in-file guard
asserts dispatcher classification for every exported route.

---

## §3. `--hub` URA addressing — drop `http://`, dial by `easynet:///r/<realm>/authority`

**Status: DONE for Phase 1** — URA-shaped `--hub` now takes the Axon
`federation.join` path; HTTP URL join is retained for the staged account-binding
cutover.

CTO ruling: **URA-only, no backward-compat** — but staged so the irreversible
deletion of HTTP pairing happens last. Phase 1 (this spec's P0) is **reversible
and additive**: URA-shaped `--hub` takes the new flow, URL-shaped `--hub` keeps
the old HTTP flow, nothing is deleted yet.

### Background & the thesis

The hub is a network Agent with a URA (`easynet:///r/<realm>/authority`), not an HTTP
endpoint. Today `device join` addresses it as `--hub https://easynet.run`
(an HTTP URL), then runs an HTTP `preflight`/`validate` handshake against the
backend's `/api/v1/devices/pairing/...` REST surface. The wrapper→CLI thesis
says the hub should be addressed by URA and joined over Axon invocation
(`federation.join`), with the backend's HTTP pairing reduced to an *optional*
product facade.

Two facts make this feasible with a **CLI-only** change:

- `realm` is a CAIA DNS domain. So `realm` → dial address is a local rule:
  `localhost`/`127.*`/`::1` → `127.0.0.1`, else the realm string itself, on the
  canonical hub TLS port `50443` (`config.rs::DAEMON_TLS_PORTS = ["50443",
  "50543"]`). The URA self-contains its bootstrap address; zero `http://`.
- Axon natively admits a brand-new device's first call: `federation.join` is
  the §A6 pre-membership ability; the membership gate
  (`EasyNet-Axon/core/runtime-rs/src/services/invocation/membership_gate.rs`)
  skips the membership check for it, accepting the §A2 provisional URA
  (`provisional:<sha256(pubkey)>`, `provisional_ura.rs`) as caller. No
  HTTP-issued credential is required for the runtime join.

### The bootstrap-trust answer (the crux)

Two trust anchors ride the HTTP preflight today and must be re-sourced:

- **Hub TLS CA** — pins the gRPC dial channel. For a public-domain hub
  (`easynet.run:50443`) the dial passes `ca_path = None` and tonic uses WebPKI
  `with_native_roots()` — nothing pinned, nothing fetched, the same trust an
  HTTPS GET already relies on. A self-hosted private-cert hub supplies the CA
  out-of-band once via `--hub-ca <path>` (writes `tls_ca_pem_path` into the
  realm-trust hub row, exactly as `persist_hub_tls_ca_pem_for_join` does today).
- **Hub Agent Ed25519 pubkey** (the `role=hub` realm-trust row; without it
  every backend→daemon dispatch 401s). After the TLS session is up and
  `federation.join` returns, the device escalates
  `federation.resolve_key { agent_ura: <hub_agent_ura> }` over the *same* live
  session and imports the key (reuse `device_trust_sync::resolve_from_hub`,
  currently pointed at device URAs — point one call at the hub's own URA).
  Security reduces to "do you trust the TLS channel," identical to today's HTTP
  preflight. **Zero Axon change** (vs. carrying the key in the join receipt,
  which would need an Axon edit) — this is what keeps Phase 1 a pure CLI diff.

**Residual safety ruling (REQUIRED):** a self-hosted hub on a private cert with
NO `--hub-ca` and NOT a public domain → the in-band key fetch rides an
unauthenticated TLS channel (anchor-less TOFU). **Hard-fail that
configuration**: refuse the join with "supply `--hub-ca` or use a public-domain
hub." A public-WebPKI hub or a `--hub-ca`-pinned hub is safe; this one case is
not.

### The new join flow (Phase 1, additive — URA path alongside the URL path)

Route on the shape of `--hub`: a `easynet:///` value → new flow; an `https://`
value → existing HTTP flow (kept until Phase 3).

| OLD stage (`join.rs`) | NEW (URA) replacement |
|---|---|
| `preflight` HTTP GET → `PairingPreflight{realm, node_id, hub_public_key_b64, hub_tls_ca_pem_b64}` | Parse URA → extract `realm`; self-mint `device/<uuid>`; CA from `--hub-ca`/WebPKI; hub pubkey deferred to the realm-trust step |
| `validate` HTTP POST → `DeviceJoinCredentialEnvelope` | `federation.join` single-shot daemon invoke, args `{realm, membership_ura, public_key_hex}` → `JoinReceipt` |
| `creds.hub_api_base` set | not set on the URA path |
| `save-credentials` | same; fields self-sourced |
| `daemon-config` | same; `hub_endpoint` derived `<realm>→host:50443` |
| `realm-trust` hub row from `hub_pubkey_b64` | `federation.resolve_key` in-band → `upsert_hub_trusted_agent` (existing writer) |
| `refresh-runtime` | same (already daemon-invoke + advertise) |

### Exact edits (Phase 1)

- **`src/daemon/federation/client/ability_contract.rs`**: `JoinArgs` already gained
  `membership_ura` (done). Keep `realm`, `public_key_hex`, `pairing_secret`
  (hub discards the latter, harmless).
- **`src/cli/join.rs`**:
  - `parse_hub_ura(&str) -> Result<(realm, port: Option<u16>)>` wrapping
    `crate::ura::parse_ura`, asserting `kind == Hub`.
  - `hub_endpoint_for_realm(realm, port)` → `localhost`-family special-case +
    `<realm>:50443` (or shared-host template — confirm the live deploy convention;
    production `PublicEndpoint` is `https://easynet.run:50443`).
  - `do_federation_join(realm, membership_ura, public_key_hex, hub_endpoint,
    ca_path)` — single-shot dispatch cloned from
    `federation_invoke::invoke_federation_revoke` (`:943`), NOT the warm-session
    prelude.
  - `fetch_hub_pubkey_in_band(...)` via `device_trust_sync::resolve_from_hub`.
  - `--hub` now accepts a URA; add `--hub-ca <PathBuf>` and (lower-risk than a
    URA-grammar port) `--hub-port <u16>` for non-standard self-hosted ports.
    Route on URA-vs-URL shape.
  - Implement the hard-fail safety ruling above.
- **`src/persistence/config.rs`**: add `Credentials.join_receipt_hash:
  Option<String>` (membership lineage). Do NOT remove
  `credential_token`/`hub_api_base`/`deploy_signature` in Phase 1 — removal
  breaks `#[serde(deny_unknown_fields)]` read-back of existing
  `credentials.json` and is reserved for Phase 3.

### Phasing (binding — irreversible work is last)

- **Phase 1 (P0, this branch, reversible):** the above. URA flow added; HTTP
  flow untouched; nothing deleted. Verify a self-hosted URA join end-to-end
  with `federation gen-cert` + `--hub-ca`.
- **Phase 2 (Axon-coordinated):** implement the product-neutral
  `PrincipalLifecycle` from `daemon-sdk-requirements-v1.md` and allow
  `federation.join` to carry an admitted principal-enrollment proof. Do not put
  product `username`/`user_id` fields into the canonical join contract. Rewrite
  backend HTTP pairing as a facade that maps the authenticated account to a
  Principal URA through the Go SDK and then invokes the same lifecycle/join
  transitions used by a backend-free Hub.
- **Phase 3 (IRREVERSIBLE):** delete `preflight`/`validate`/`pick_validate_base`,
  `credential_token`, `hub_api_base`, the verify-credential calls. Only after
  Phase 2 multi-user parity and the standalone-Hub acceptance in
  `daemon-sdk-requirements-v1.md` section 14.3 are proven.

### Acceptance (Phase 1)

- `easynet device join easynet:///r/<realm>/authority [--hub-ca …]` joins a hub over
  TLS + `federation.join` with no HTTP call; device becomes a member, advertises,
  and inbound dispatch works with an empty `credential_token`.
- `easynet device join https://…` still works (HTTP path untouched).
- Self-hosted private-cert hub without `--hub-ca` and non-public domain →
  hard-fails with the documented message.
- Build + clippy clean both features (the `do_federation_join` dispatch is
  `axon-pb`-gated — the non-axon-pb stub must compile).

### Honest residual

The cold-start trust bootstrap is fully solved for public-WebPKI hubs and for
self-hosted hubs that pin `--hub-ca`. The one unsafe configuration (private
cert, no `--hub-ca`, non-public domain) is closed by the hard-fail ruling, not
by making it work. Multi-user enrollment genuinely breaks if HTTP is removed
before the canonical PrincipalLifecycle and admitted enrollment proof exist —
which is why Phase 1 keeps both paths.

---

## §4. Axon §9.3 — cross-language conformance test vectors

**Status: DONE** — owner authorized the runner work and the shared vectors are
now consumed by Go / Rust / Node / Python SDK test targets.

### Background

§9.3 confirmed the canonical Axon proto contracts exist and are correct
(DirectoryEntry / ListUserDevices / DeviceJoinCredentialEnvelope / ResolveKey /
AbilityProjectionSummary / ResolveAnswer in `core/proto/axon/v1/`; the
`callable_summary` exclusion is pinned in `namespace.proto`). The remaining
§9.3 deliverable is cross-language conformance *vectors* for the
namespace/join/directory/resolve_key wire shapes, so Go / Rust / TS / Python
SDKs can prove byte-identical canonicalization against a shared fixture set.

### Implemented state

- `EasyNet-Axon/packaging/protocol-pack/conformance-vectors/federation-directory-v1.json`
  is consumed by Go, Rust, Python and Node. Node currently validates this as an
  explicit seam-level projection because the Node SDK does not yet expose a
  typed directory model.
- `EasyNet-Axon/packaging/protocol-pack/conformance-vectors/federation-wire-v1.json`
  covers `federation.resolve`, `federation.join`, `resolve_key.request` and
  `resolve_key.response`.
- Go, Rust, Python and Node all load the same `federation-wire-v1.json` fixture.
  The federation resolve/join cases call product-neutral SDK payload builders
  rather than copying payload lowering into tests.
- `scripts/checks/protocol_pack_conformance_consumers.sh` prevents the
  federation protocol-pack vectors from becoming dead fixtures.

### Architectural decision

The valuable artifact is not fixture JSON by itself; it is the executable
cross-language runner. The implementation therefore added the missing SDK
runners and a consumer guard in the same change as the new federation wire
fixture.

One real drift was found and fixed during implementation: Rust
`ResolveKeyRequest` serialized absent optional `presented_pubkey_*` fields as
`null`. The model now skips absent optional key fields, converging to the shared
canonical wire projection instead of changing the vector to match a divergent
implementation.

The new SDK helpers are generic federation wire payload builders. They do not
start a daemon, manage product profiles, or introduce EasyNet/EasyRemote
product lifecycle into Axon.

### Verification

- `bash scripts/checks/protocol_pack_conformance_consumers.sh`
- `go test ./easynet -run 'ProtocolPack|FederationDirectory'`
- `uv run python -m pytest tests/test_protocol_pack_vectors.py tests/test_federation_directory.py`
- `cargo test --test protocol_pack_vectors --test federation_directory_vectors`
- bundled Node/tsc equivalent of `protocol-pack:vectors`:
  `node ./scripts/clean-generated.mjs && tsc -p tsconfig.json && node ./scripts/run-protocol-pack-vectors.mjs`

---

## §5. Remove the dead `internal/registry` package (EasyNet backend)

**Status: DONE** — deletion authorized by the implementation goal and verified
against current backend consumers.

### Background

The §9.4.2 audit found `internal/registry` (`ability_registry.go`:
`AbilityRegistry`, ent-backed `RegisterAbility` / `GetByURA` / `ListByOwner` /
`ListByName`) has **zero non-test callers** anywhere in the backend — not in
`svcCtx`, not in any handler or logic. It is an entirely dead, ent-DB-backed
ability registry. It is spec-compliant *if used* (a product read-model, not an
authoritative resolver), but nothing uses it.

### Current verified state

- `backend/internal/registry/ability_registry.go` and
  `ability_registry_test.go` are gone.
- `ent/schema/ability.go`, the dependent `AbilityVersion` schema and generated
  `ent/ability` package are gone.
- No dangling production caller remains.

### Execution note

`internal/registry` had zero production consumers. `ent.Ability` and the
dependent `AbilityVersion` schema were also unconsumed outside their own ent
island, so the schema and generated packages were removed together and ent was
regenerated.

### The completed change

1. Confirmed zero remaining consumers.
2. Deleted `internal/registry/ability_registry.go` and
   `ability_registry_test.go`.
3. Removed the unconsumed `ent.Ability` / `AbilityVersion` schema island and
   regenerated ent.
4. Verified backend build/test gates through the aggregate SDK completion audit.

### Acceptance

- `internal/registry` gone; no dangling imports; full backend build + test
  green.
- `ent.Ability` schema either removed (with regen) or justified as
  still-consumed.

---

## §6. Complete the standalone-Hub PrincipalLifecycle

**Status: DONE** — the multi-key substrate, backend-free lifecycle and
Backend-present account mapping are implemented against the same
PrincipalLifecycle, key-service, RuntimeTrust and admission roots.

This section records a binding delivery target. The detailed runtime contract,
transition invariants and cross-repository ownership rules are normative in
`daemon-sdk-requirements-v1.md` section 14.

### Current capability baseline

The correct conclusion is: the multi-key substrate is present, and the
backend-free plus Backend-present PrincipalLifecycle acceptance gates now prove
one shared runtime lifecycle rather than a second authentication system.

| Capability | Current state |
|---|---|
| One Hub manages multiple runtime owners | implemented |
| One User URA binds multiple public keys | implemented |
| Key create, query, rotate, revoke and expiry | implemented |
| Private keys are held only by the daemon key-service | implemented |
| Multi-user signature verification and admission | implemented |
| Create the first user without Backend | provider, CLI bootstrap facade and standalone-Hub TCP+TLS E2E implemented |
| User login, authentication and recovery without Backend | recovery policy proof, replay protection, CLI recovery facade and live recovery-edge E2E implemented; broader product UX packaging remains downstream |
| A user adds a second device/key without Backend | add/rotate/revoke, device enrollment proof binding and live E2E coverage implemented |
| Multi-user administration and permission governance without Backend | provider, live wrong-action grant denial and delete-grant terminality implemented; broader governance UX packaging remains downstream |

The implementation already establishes these lower-level facts:

- `RealmTrustAnchor` maps one User URA to multiple public keys, allowing one
  principal to use distinct device keys;
- `identity.register_pubkey`, `identity.list_user_pubkeys` and
  `identity.revoke_user_pubkey` exist;
- the daemon key-service supports create, list, public projection, rotate,
  revoke, expiry and subject binding;
- a managed key may bind to a User URA, while private material never enters
  Backend, an SDK consumer or EasyRemote; and
- cross-realm user-binding tokens and replay protection exist; and
- `principal.lifecycle.*` now has an initial daemon-owned durable provider
  that records principal state, key bindings, recovery policy and grants while
  projecting active/revoked public-key facts through the existing
  `RuntimeTrust` aggregate; and
- provider-side PrincipalLifecycle proof enforcement now validates
  active-key references against active key bindings, grant references against
  durable authorization grants, recovery references against the configured
  recovery policy, and bind-first-key continuity against the create-time
  bootstrap/enrollment proof; and
- durable PrincipalLifecycle enrollment authority now issues, revokes and
  consumes one-time `EnrollmentCapability` records inside the same aggregate.
  Additional principal creation no longer accepts a bare `proof.kind =
  enrollment`; it must reference an active, unexpired, unrevoked and
  unconsumed capability scoped to the target Principal URA.

These facts now compose into the product-neutral user lifecycle. Pure-URA
`federation.join` still establishes Device membership and never implicitly
creates a User; a principal binding is admitted only when the join carries a
valid PrincipalLifecycle proof. CLI facades now cover bootstrap, invitation
enrollment, additional keys, rotation, revocation, recovery, suspension,
reactivation, grants, deletion and inspection without Backend account state.
Product login screens and governance UX packaging remain downstream product
work, not missing canonical runtime state.

### Required canonical state machine

The implemented capability is one product-neutral lifecycle shared by
standalone and Backend-present deployments:

```text
CreateUser
  -> BindFirstKey
  -> Active
  -> AddKey
  -> RotateKey | RevokeKey
  -> Recover
  -> Suspend
  -> Active | Delete
```

The aggregate is generic runtime state, never a Backend account model:

```text
Principal URA
  -> enrollment authority
  -> public-key bindings
  -> key rotation/revocation state
  -> recovery policy
  -> authorization grants
```

Every transition must be an admitted Invocation with a replay-protected proof,
atomic durable mutation and verifiable receipt. A failed transition leaves the
previous principal and key-binding state unchanged.

### Cross-repository actions

1. **EasyNet-Cli daemon:** implement the durable PrincipalLifecycle provider,
   explicit first-principal bootstrap, invitation/enrollment proofs,
   additional-key authorization, recovery, suspension/reactivation, deletion
   and grant enforcement on the existing admission and key-service roots.
2. **Go and Python SDKs:** expose the same typed lifecycle commands,
   projections and errors; providers lower those operations once. SDK
   consumers never receive private keys or construct lifecycle wire payloads
   by hand.
3. **EasyNet Backend:** map PostgreSQL/OAuth/Passkey account results to the same
   Principal URA and lifecycle through the Go SDK. Backend remains an optional
   account/HTTP adapter and must not own another trust store, key inventory,
   recovery truth, daemon process tree or runtime authentication path.
4. **EasyRemote:** consume public principal, identity and signing projections
   through the Python SDK. It keeps only Remote product workflow and
   presentation state.
5. **URA join:** `federation.join` continues to create Device membership and
   may bind a principal only when it carries an admitted enrollment proof. It
   must never infer a User URA from HTTP/product account fields.

Standalone administration may use a local administrator capability, an
invitation capability or a signed enrollment/recovery proof. Backend-present
and Backend-free deployments must converge on the same User URA, key-service,
admission, grants, replay state and receipts. A second authentication system is
prohibited.

### Acceptance

A backend-free end-to-end gate must:

1. start one Hub and its single daemon key-service;
2. bootstrap the first administrator through a one-time authority;
3. enroll at least two User URAs and at least two keys per user;
4. admit every active key, revoke one key and prove its sibling remains valid;
5. exercise rotation, recovery, suspension/reactivation and delete
   terminality;
6. restart the Hub and prove lifecycle, grants, revocation and receipt state
   persist; and
7. join a Device through a Hub URA without a Backend HTTP dependency.

A second gate must attach EasyNet Backend to that same runtime and prove an
account flow maps into the same principal/admission truth without spawning a
second daemon/key-service or writing a parallel trust source.

### Recovery-state audit note

The interrupted restoration that preceded this convergence effort temporarily
left the worktree in an uncommitted state with Go SDK type conflicts. That
snapshot was not deliverable. Before the interruption, key-service and SDK
boundary tests were green, but the backend-free multi-user lifecycle was still
only partially complete as described above.

The compilation conflict is historical, not a current delivery blocker: the
baseline was restored and re-audited on 2026-07-11. Current green compilation
and the initial daemon `principal.lifecycle.*` provider must not be mistaken
for section 6 completion. Active-key, grant, recovery and admission-state
plus enrollment-capability enforcement have landed in the provider. The
product-neutral CLI facade now covers the provider-backed lifecycle transition
surface through the same daemon abilities, including create, bind-first-key,
add-key, rotate-key, revoke-key, configure-recovery, recover, suspend,
reactivate, delete, issue/revoke enrollment, issue/revoke grant and get. A
provider-level backend-free scenario gate proves multi-user, multi-key
enrollment, rotation, revocation, recovery, lifecycle state changes and
persisted trust/lifecycle reload at the daemon aggregate boundary. A real
daemon gRPC descriptor-ref E2E now drives the same `principal.lifecycle.*`
surface through `DaemonInvocationService`, restarts the daemon, and verifies
persisted PrincipalLifecycle plus trust-anchor public-key state without
Backend, HTTP account state or a second auth store. The `federation.join`
contract now has a product-neutral optional PrincipalLifecycle proof seam, and
the Hub daemon validates that proof before atomically binding the joined Device
URA to the User Principal in RuntimeTrust; a real daemon UDS E2E now proves that
binding persists through the same Backend-free PrincipalLifecycle test fixture.
The recovery edge cases and both standalone/backend-present end-to-end gates
are now covered by the acceptance scripts. The CLI
`principal bootstrap` facade now composes `principal.lifecycle.create` and
`principal.lifecycle.bind_first_key` with one bootstrap proof reference,
separate idempotency keys, fixed bind expected-version `1`, and daemon
key-service public-key projection only. The CLI `principal enroll` facade now
consumes an issued enrollment capability by composing the same create and
bind-first-key transition pair with `proof.kind = enrollment`, the shared
enrollment id, separate idempotency keys and fixed bind expected-version `1`.
These close the first-principal and enrollment-consume CLI entrypoints. The
Hub URA join CLI now also accepts `--principal-enrollment-id` as a
product-neutral shorthand for `proof.kind = enrollment`, reducing device join
proof assembly to `--principal-ura` plus the issued enrollment id. This closes
the CLI proof-lowering UX for device enrollment. A real CLI binary E2E now
starts a hub-mode `easynet-daemon` with a TCP+TLS Invocation listener,
bootstraps a Hub-side administrator, issues a product-neutral enrollment
capability, joins a Device by Hub URA with `--hub-ca`, `--principal-ura` and
`--principal-enrollment-id`, and verifies backend-free `federation.join`, empty
HTTP credential token, persisted `join_receipt_hash`, pinned CA persistence,
in-band Hub key import through `federation.resolve_key`, and the Hub
RuntimeTrust owner binding from Device URA to User Principal URA.
Recovery edge cases are now covered by the live Hub TCP+TLS E2E. Backend-present
E2E is covered by the live daemon-backed account-flow gate. The downstream SDK consumer cutover
and product private-key custody gates now cover
Backend/EasyRemote Receipt/Directory/runtime consumer usage and reject product
private-key custody, raw daemon process spawning and raw FFI escape paths. The
Go and Python SDKs now expose a product-neutral runtime environment projection
for local state root, credentials path and paired runtime identity. EasyRemote
`LocalIdentity` and compatibility `read_credentials()` consume that SDK
projection instead of interpreting daemon credentials as a separate product
identity model. The complete canonical public API inventory now tracks those
projection symbols and the corresponding `SdkEnvironment` members so the
runtime model cannot regress through an unreviewed public-surface deletion. A real CLI
binary E2E
now executes
`principal bootstrap`, `principal issue-enrollment`, `principal enroll`,
`principal add-key`, `principal rotate-key`, `principal revoke-key`,
`principal configure-recovery`, `principal recover`, `principal suspend`,
`principal reactivate`, `principal issue-grant`, `principal delete` and
`principal get` against the in-process daemon UDS fixture plus daemon
key-service, proving the user lifecycle CLI facades lower to the daemon-owned
aggregate and RuntimeTrust projection without Backend account state.
A real two-HOME CLI binary E2E now extends the TCP+TLS Hub daemon path to the
multi-user lifecycle scenario: it starts one Hub-mode `easynet-daemon`, joins a
Device by Hub URA with no Backend HTTP credential, consumes a product-neutral
enrollment into Alice's first key, enrolls Bob, binds at least two keys for both
User URAs, rotates Alice's first key, revokes Alice's sibling key while proving
the rotated sibling remains active, configures recovery, recovers, suspends and
reactivates Alice, deletes Bob through an admin grant, restarts the Hub daemon,
and verifies persisted PrincipalLifecycle state, grant state, RuntimeTrust
revocation projection and Device→Principal owner binding. The same live Hub
TCP+TLS E2E now rejects replayed recovery proofs and deleted-principal recovery
attempts, then verifies those failed replacement keys are not projected into
RuntimeTrust. It also rejects a wrong-action administrator grant before
deleting Bob and verifies Bob remains active until a grant for
`principal.lifecycle.delete` is supplied. Backend-present
mapping has now crossed the live runtime boundary: the Backend ServiceContext
has a tested single SDK profile graph proving PrincipalLifecycle, Receipt,
Directory, Events, Admin and AccessControl clients derive from the same Go SDK
native runtime provider rather than a second daemon/key-service/trust-store
construction. Backend account signing-key logic is tested through the real Go
SDK PrincipalLifecycle adapter, proving product account input lowers as
`principal.lifecycle.get`, `principal.lifecycle.create` and
`principal.lifecycle.bind_first_key` without legacy identity mutation. The
process-level Backend HTTP E2E drives `POST /api/v1/user/me/signing-keys`
before signed invocation admission, and the test principal runtime sits below
`principalprofile.NewClient` rather than beside it, proving the browser-facing
product route consumes the same SDK PrincipalLifecycle projection. A live
daemon-backed Backend-present account-flow E2E now starts a Hub-mode
`easynet-daemon` through the Go SDK C ABI daemon lifecycle, attaches Backend
account registration to the daemon-backed PrincipalLifecycle provider, and
verifies the resulting active Principal URA and public key binding through the
same SDK projection. Recovery UX edge-case closure is now covered by the live
Hub TCP+TLS recovery replay/deleted-principal rejection checks.
Go and Python PrincipalLifecycle projection
decoders now reject forbidden custody fields recursively, matching the
managed-signing public-projection guard, and the real CLI TLS lifecycle E2E
scans PrincipalLifecycle JSON output for private-key custody fields.
Runtime Events now have an explicit cross-repository adapter gate covering the
Go/Python SDK runtime-event facades, Backend SDK event subscription/open-stream
adapters and EasyRemote product event consumer behavior. Runtime Events now
also have a live daemon cutover gate:
`tools/scripts/runtime-events-live-daemon-e2e.sh` composes the cross-repository
adapter gate with Go and Python SDK live smokes that read bounded
`RuntimeEventClient` pages from real `easynet-daemon` handle events over the C
ABI. This promotes the runtime-events SDK capability itself to cutover-ready;
product event taxonomies remain downstream.

---

## Summary table

| § | Item | Status | Gate to start |
|---|---|---|---|
| 1 | Conformance boot-time gate (§9.1-7) | DONE | Verified |
| 2 | DaemonInvocation route const (§7.3-5) | DONE | Verified |
| 3 | `--hub` URA addressing | DONE for Phase 1 | Verified |
| 4 | Axon §9.3 cross-language vectors | DONE | Verified |
| 5 | Delete dead `internal/registry` | DONE | Verified |
| 6 | Standalone-Hub PrincipalLifecycle | DONE | Verified by canonical provider, SDK parity and `standalone-hub-principal-lifecycle-e2e.sh` section 14.3 gate |

Section 6 is accepted by `tools/scripts/standalone-hub-principal-lifecycle-e2e.sh`, which
composes the backend-free and Backend-present section 14.3 E2E shapes, and by
the aggregate SDK completion audit. Broader standalone recovery/governance UX
packaging remains downstream product work, not a missing canonical runtime
model. The irreversible Phase 3 removal of the staged HTTP pairing path remains
a separate cutover decision and must not be done merely because the section 6
runtime acceptance gates are green.
