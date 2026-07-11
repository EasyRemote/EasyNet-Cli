# Remaining Follow-up TODOs — Execution Spec (2026-06-30)

Status tracker for the five work items left open after the wrapper→CLI
runtime-refactor push (see `easynet-wrapper-cli-runtime-refactor-spec-2026-06-28.md`)
and the device-identity / trust hardening that followed it. Each item below is
self-contained: background, current on-disk state, the exact change, and an
acceptance gate.

The thirteen completed items (conformance §7.3 model, the three latent bugs,
§9.1.A registry convergence, §9.2 backend→product-wrapper, CLI user
signing-key facade, stable install-id rejoin, the bug-3 owner-prefix grammar
fix, the stale-JWT existence gate) are NOT repeated here — they are done,
built, and tested.

Execution update: §1, §2, §3 Phase 1, and §5 have been landed. §4 remains
deferred because vectors without SDK runners would be dead fixtures.

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

## §3. `--hub` URA addressing — drop `http://`, dial by `easynet:///r/<realm>/hub`

**Status: DONE for Phase 1** — URA-shaped `--hub` now takes the Axon
`federation.join` path; HTTP URL join is retained for the staged account-binding
cutover.

CTO ruling: **URA-only, no backward-compat** — but staged so the irreversible
deletion of HTTP pairing happens last. Phase 1 (this spec's P0) is **reversible
and additive**: URA-shaped `--hub` takes the new flow, URL-shaped `--hub` keeps
the old HTTP flow, nothing is deleted yet.

### Background & the thesis

The hub is a network Agent with a URA (`easynet:///r/<realm>/hub`), not an HTTP
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

- `easynet device join easynet:///r/<realm>/hub [--hub-ca …]` joins a hub over
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

**Status: DEFERRED** (owner sign-off required; not a code bug).

### Background

§9.3 confirmed the canonical Axon proto contracts exist and are correct
(DirectoryEntry / ListUserDevices / DeviceJoinCredentialEnvelope / ResolveKey /
AbilityProjectionSummary / ResolveAnswer in `core/proto/axon/v1/`; the
`callable_summary` exclusion is pinned in `namespace.proto`). The remaining
§9.3 deliverable is cross-language conformance *vectors* for the
namespace/join/directory/resolve_key wire shapes, so Go / Rust / TS / Python
SDKs can prove byte-identical canonicalization against a shared fixture set.

### Current state

- `EasyNet-Axon/packaging/protocol-pack/conformance-vectors/` contains only
  `easynet-uri-v1.json` and `envelope-signing-v1.json`.
- **No runner consumes these vectors.** A grep for the vector files across the
  Go/Rust/TS SDK test suites returns zero hits. Adding more JSON files alone
  produces dead fixtures.

### Why deferred (the honest blocker)

The valuable artifact is not the vectors — it is a **cross-language runner** that
loads them and asserts each SDK's canonicalization matches. Building Go + Rust +
TS (+ Python) harnesses that consume a shared vector set is an independent,
RFC-scale effort touching multiple SDK test trees. Shipping vectors without a
runner is busywork that reads as "covered" while covering nothing.

### The change (only if owner authorizes the runner too)

1. Define the vector schema for each shape (namespace.resolve query/answer,
   federation.join args/receipt, directory entry, resolve_key request/response)
   — input + expected canonical bytes + expected error, mirroring the existing
   `easynet-uri-v1.json` shape (`{version, description, vectors:[{id, input,
   canonical, expect_error}]}`).
2. Author the fixtures under `conformance-vectors/<shape>/`.
3. Add a runner per SDK that loads the fixtures and asserts
   canonicalization/parse equality — Rust (`sdk/rust`), Go
   (`sdk/go/easynet/invocation`), TS, Python — each as a normal test target.
4. Wire the runners into each SDK's CI so a wire-spec drift in any language
   fails a test.

### Acceptance

- Each SDK has a test that loads the shared vectors and passes; a deliberate
  canonicalization change in one SDK fails its runner.
- No dead fixtures: every vector file is consumed by at least one runner.

### Decision needed

Authorize the cross-language runner work (then implement the full §4), or leave
§9.3 confirmed-but-vectorless. Do not ship vectors without runners.

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

### Current state

- `backend/internal/registry/ability_registry.go` + `ability_registry_test.go`
  still present.
- No production caller. The methods reference `ent.Ability`
  (the ent schema for an `abilities` table).

### Execution note

`internal/registry` had zero production consumers. `ent.Ability` and the
dependent `AbilityVersion` schema were also unconsumed outside their own ent
island, so the schema and generated packages were removed together and ent was
regenerated.

### The change (if owner authorizes)

1. Confirm zero remaining consumers (re-run the grep at delete time — the tree
   is shared and may have changed).
2. Delete `internal/registry/ability_registry.go` +
   `ability_registry_test.go`.
3. Determine `ent.Ability` schema fate: if `internal/registry` was its only
   consumer, remove `ent/schema/ability.go` and re-run `ent generate`; if other
   code reads the `abilities` table, leave the schema and only delete the dead
   wrapper.
4. `go build ./... && go vet ./... && go test ./...` green.

### Acceptance

- `internal/registry` gone; no dangling imports; full backend build + test
  green.
- `ent.Ability` schema either removed (with regen) or justified as
  still-consumed.

### Decision needed

Authorize deletion, or keep the dead package. It is not blocking anything.

---

## Summary table

| § | Item | Status | Gate to start |
|---|---|---|---|
| 1 | Conformance boot-time gate (§9.1-7) | DONE | Verified |
| 2 | DaemonInvocation route const (§7.3-5) | DONE | Verified |
| 3 | `--hub` URA addressing | DONE for Phase 1 | Verified |
| 4 | Axon §9.3 cross-language vectors | DEFERRED | owner authorizes the runner |
| 5 | Delete dead `internal/registry` | DONE | Verified |

§4 is the only intentionally open item. It should remain open until the
cross-language runner work is authorized; adding fixture JSON alone is not an
acceptable completion.
