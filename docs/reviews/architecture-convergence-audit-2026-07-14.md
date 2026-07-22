# Architecture Convergence Audit - 2026-07-14

> **Historical frozen record:** This document preserves checkout observations
> captured from 2026-07-14 through 2026-07-17. Terms such as "current",
> "working tree", "Open", and "failed" describe those captured states, not the
> accepted 2026-07-18 checkout. Sections and addenda remain unchanged as audit
> evidence. Current Canonical Runtime Convergence V2 status is authoritative
> only in `docs/spec/canonical-runtime-convergence-v2.md` Section 12 and
> `docs/reviews/canonical-runtime-convergence-v2-closure-2026-07-18.md`.

## 1. Audit objective and scope

This audit asks whether EasyNet-Cli and EasyNet-Axon implement one canonical
runtime architecture, rather than whether individual features compile or have
tests. It covers production source under `src/`, `sdk/`, `plugins/`, Axon
`sdk/`, `core/runtime-rs/`, and `core/proto/`. Generated protobuf code and test
fixtures are inspected as evidence but are not counted as independent owners.

Baseline classification is against the merge bases with `main`:

- EasyNet-Cli: `bd84744c`
- EasyNet-Axon: `5303cbd`

The working tree is intentionally separate from the branch baseline. A defect
can therefore be main-existing, branch-new, or temporarily corrected only in
the uncommitted working tree.

Methods used:

1. CodeGraph symbol and caller analysis in both repositories.
2. `git diff main...HEAD`, merge-base archive scans, and current-tree scans.
3. Capability-matrix to public-API inventory comparison.
4. Owner truth-table, runtime route, lifecycle, receipt, SDK, and proto review.
5. Executable architecture gate and negative fixture suite.

The architecture gate found 34 violations in `main` and 38 in committed branch
HEAD. The four branch additions were `PagesResourceURA`, Python
`AbilityAddress`, and two semantic `ability_address` variables. The current
working tree clears these 38 mechanical findings, but that does not clear the
structural findings below.

## 2. Overall diagnosis

The current branch is not yet architecture-converged. The central fork is not
one broken dependency edge; it is disagreement about what owns the canonical
runtime model:

- Axon contains two independently implemented Rust invocation SDK cores.
- EasyNet-Cli introduces a new multi-language SDK whose package identity,
  daemon lifecycle, directory layout, and C ABI are still EasyNet-specific.
- The Go/Python matrix records matching labels for 26 capabilities while the
  actual public inventories contain 593 Go and 312 Python symbols.
- Product protocols remain inside Axon's SDK/protocol domain.
- Several lifecycle paths project terminality before cryptographic proof or
  before the canonical owner has finalized the state.

This means the repository currently has protocol convergence in selected call
paths, but not model convergence.

## 3. Complete architecture-break table

| ID | Severity | Architecture break and root abstraction problem | Evidence | Baseline attribution | Current working-tree state | Required convergence |
|---|---|---|---|---|---|---|
| A01 | P0 | The new SDK is owned and named as an EasyNet daemon SDK, not as the canonical runtime model. Product and provider are fused into one public abstraction. | `sdk/go/go.mod`, `sdk/python/pyproject.toml`, `sdk/java/pom.xml`, `sdk/node/package.json`, `sdk/swift/Package.swift`; `main` contained only five SDK README files, while this branch adds 319 SDK files and about 85,865 lines. | **Branch-new** | Open | Extract/rename a product-neutral runtime SDK. Keep EasyNet daemon transport and product facades downstream as providers/consumers. |
| A02 | P0 | Axon has two same-language canonical invocation implementations, so canonical bytes, envelope validation, signing, and admission can drift inside one repository. | CodeGraph resolves `canonical_invocation_bytes` to both `sdk/rust/src/invocation/axiom.rs:913` and `core/runtime-rs/client-sdk/src/domain/admission.rs:392`. | **Main-existing** | Open | Select one canonical Rust domain crate; migrate callers; make the other package a thin export/transport adapter; delete the duplicate model and encoder. |
| A03 | P0 | Deprecated non-proof-bound invocation signing remains executable. This is a legacy architecture fallback, not an explicitly required protocol state. | `sdk/rust/src/invocation/axiom.rs:910,1234,1327`, `invocation/admission.rs:218,786,845`, and `src/bin/verify.rs:495`. | **Main-existing** | Open | Migrate every caller to descriptor-bound proof, delete plain sign/verify APIs and verifier branch, and remove deprecated re-exports. |
| A04 | P0 | Terminal state and receipts were minted/projected in multiple EasyNet-Cli geometry handlers instead of being finalized once by Axon runtime. | Main gate: two daemon ledger writers and four terminal receipt writers. Affected bidi, stream, local-session, and receipt projection paths. | **Main-existing** | Refactor in verification | One Axon finalized result must carry terminal state plus signed chain. All CLI geometries may only project it, never infer or mint it. |
| A05 | P0 | EAL/Mission child calls bypassed canonical invocation admission. The first repair replaced direct catalog calls with a daemon-socket self-loop, which fixes the gate but not the ownership model. | Main gate: eight R1 bypasses. Current `src/daemon/execution/mission/invocation_gateway.rs:66` re-enters `daemon.sock`; test gateway still calls catalog directly. | **Main-existing root; worktree transitional path** | Partially corrected | Use Axon `AbilityContext::invoke_child` with inherited causal context and signing-authority provider. Delete the daemon self-call gateway after migration. |
| A06 | P0 | Receipt acceptance proves shape and chain binding but explicitly does not verify signatures. A structurally valid anchor is therefore exposed as causal evidence without cryptographic trust. | `src/support/platform/local_daemon_grpc.rs:1281` emits `cryptographic_verification: not_performed`; validation checks sizes/bindings but has no key resolver. | **Branch-new** | Open | Put receipt verification behind a canonical verifier with a `KeyResolver`; expose a typed `Unverified` state or reject use as a trusted causal anchor. |
| A07 | P0 | The four-state capability model is not closed over the public API. Capabilities can be shipped without appearing as unsupported/seam/provider-backed/cutover-ready. | Matrix has 26 entries; inventory contains Go-only `FederationRevokePayload` and Python-only `AbilityInvocationClient`, `AbilityChildContext`, `ControlIpcClient`. | **Branch-new** | Open | Generate public inventories from the capability manifest or fail CI when an exported capability has no matrix owner/state/evidence. |
| A08 | P0 | Go and Python are independently shaped SDKs despite matching matrix labels. Equal status labels conceal different lifecycle ownership and type semantics. | Python `connection.py:61` adds control endpoint/version/flags absent from Go `connection.go:34`; Python `environment.py:160` owns IPC/gRPC/C ABI/state root while Go `environment.go:75` has a different boundary. Inventory size (593 vs 312) is only a warning signal. | **Branch-new** | Open | Define a language-neutral canonical model/schema first, generate or conformance-bind both SDK surfaces, and explicitly mark language omissions unsupported. |
| A09 | P0 | Product protocols remain part of Axon's canonical SDK/proto domain: voice, remote desktop, MCP/EasyNet semantic concepts. Deleting several adapters did not remove the product model. | `sdk/rust/src/lib.rs:30,36,50,57`; `core/runtime-rs/client-sdk/src/domain/mod.rs:48`; `core/proto/axon/v1/voice.proto`, `remote_desktop.proto`, and product types in `types.proto`. | **Main-existing** | Partially reduced | Move product protocols and generated clients downstream. Axon keeps only generic invocation/capability/control primitives. Preserve wire compatibility only through an explicitly versioned protocol package, not canonical SDK types. |
| A10 | P0 | Voice has contradictory owners. The normative truth table says agent-owned while profile/runtime comments and registration treat it as host/device-owned. This creates duplicate discoverability and inconsistent authorization identity. | `docs/spec/owner-truth-table/ability-owner-truth-table.tex:638-671` vs `src/daemon/ability/catalog/profiles/llm.rs:27-38` and device catalog registration. | **Main-existing; branch amplified before partial correction** | Open specification conflict | Choose one owner from the concrete microphone/speaker/signaling use case, update the truth table first, then migrate registration, URAs, authorization, and discovery atomically. |
| A11 | P0 | Agent destructive lifecycle was coupled to stop semantics and lacked a durable, serialized purge transaction. Filesystem, registry, authority grants, and publication could diverge after a crash. | `agent.stop` descriptor and `builtins/agents/lifecycle.rs`; current work adds a distinct `agent.purge`, persistence journal, and explicit purge stages. | **Main-existing lifecycle gap; branch implementation in progress** | Refactor in verification | Keep stop and purge as separate commands. Purge must be an explicit recoverable FSM with cross-process lock, quarantine identity revalidation, and idempotent recovery. |
| A12 | P0 | Python transport lifecycle allowed close/reopen and result-publication races. The races were corrected, but the API still calls a re-openable quiesce operation `close`, so lifecycle semantics remain unresolved. | `sdk/python/easynet_sdk/transport.py`; deterministic tests now cover generation barriers, delegated-close result sharing, active-use leases, and outcome-before-cleanup. | **Branch-new** | Race fixes verified; lifecycle contract open | Keep the corrected synchronization, then align the public terminal/quiesce vocabulary with the shared Go/Python state machine. |
| A13 | P1 | Product-specific directory and process lifecycle are exposed as canonical SDK concepts. | Go `control_discovery.go:11` and Python `control_ipc.py:298` hard-code `.easynet/control.json`; runtime environment derives credentials/device/node/Hub facts; public `DaemonMode` includes device/hub/both. | **Branch-new** | Open | Canonical SDK owns generic runtime environment/endpoints. EasyNet provider resolves `.easynet`, credentials, topology, daemon process policy, and node-id migration. |
| A14 | P1 | Daemon modules depended upward on CLI command modules, reversing the intended dependency direction. | Main gate: eight R5 daemon-to-CLI dependencies. | **Main-existing** | Corrected in worktree | Keep command parsing/rendering in CLI and reusable orchestration under daemon/application owners; enforce with the gate. |
| A15 | P1 | Multiple receipt/ledger projections duplicated Axon-owned facts in EasyNet-Cli. | Deleted `src/daemon/invocation/receipts/ledger_projection.rs` and `src/support/platform/invocation_receipt_projection.rs`. | **Main-existing** | Corrected in worktree | Retain one Axon receipt model and read-only product projection. No local receipt synthesis or shadow ledger. |
| A16 | P1 | “URA only” naming is satisfied in non-generated SDK production code; remaining locator types are HTTP transport locators. The SDK defect is semantic divergence in URA acceptance/defaulting, not spelling. | Go `addressing.go:124` defaults empty `owner_kind` to user; Python `axon_addressing.py:292` requires it and accepts a different set. Historical docs still contain pre-URA identity prose outside SDK. | **Branch-new SDK semantics; main-existing docs** | Open | Use shared vectors for accepted URA forms, normalization, defaults, and typed errors; retain locator naming only for actual HTTP/gRPC transport locators. |
| A17 | P1 | MCP stdio implementation is placed under generic `support/platform`, although it is a daemon execution/product protocol owner. Correct dependency direction alone does not establish semantic ownership. | `src/support/platform/mcp_stdio.rs` consumed by CLI and daemon; daemon already has `src/daemon/execution/mcp/`. | **Branch/worktree-new extraction** | Independent review in progress | Move implementation to daemon execution/application ownership; let CLI call downward through a narrow interface. Keep `support` limited to product-neutral OS primitives. |
| A18 | P1 | Core modules are procedural responsibility accumulators. This prevents owner boundaries from being represented by types and makes local fixes spread across unrelated behavior. | `ability/dispatch.rs` 8,175 lines; `ffi/invocation/mod.rs` 7,196; `admission_facade.rs` 4,146; `agent lifecycle.rs` 3,284; bidi 2,731; local session 2,703. | **Mostly main-existing; SDK/FFI additions branch-new** | Open | Split by lifecycle aggregate and owner: admission, descriptor binding, execution geometry, finalization, projection, persistence. Introduce explicit aggregate/state-machine objects before moving code. |
| A19 | P1 | The public C ABI is named and versioned as EasyNet (`easynet_*`, `libeasynet_cli`) and exposes daemon operations directly. The branch defect is not only the existing ABI name; it is that SDK/conformance surfaces treat the provider ABI as the canonical runtime model instead of recording it as a provider binding. | `include/easynet_cli.h`, `src/ffi/daemon/mod.rs`, `src/ffi/invocation/mod.rs`, `sdk/python/easynet_sdk/_cabi.py`, `sdk/go/cabi_runtime.go`, and `sdk/conformance/canonical-public-api.json`. | **Main-existing ABI debt; branch-new canonicalization/amplification** | Open | Separate neutral runtime concepts from EasyNet provider ABI. Keep `easynet_*` only as versioned provider binding until an ABI bump is explicitly specified; do not register provider names as canonical SDK concepts. |
| A20 | P1 | Agent registry persistence is a shared mutable data service used directly across many owners; lifecycle invariants cannot be enforced at one aggregate boundary. | CodeGraph resolves `AgentRegistry` through lifecycle, discovery, publication, catalog, and persistence; the direct type has dozens of call sites while the cache abstraction has only a few. | **Main-existing** | Partially addressed for purge only | Introduce an `AgentAggregateRepository`/transaction owner with typed operations. Migrate direct load/save callers and delete procedural persistence access after cutover. |
| A21 | P0 | Thirty-one exact daemon routes execute outside Axon `LocalRuntime`. A mutation can commit, then the strict client rejects the default receipt fields and retries the already-applied operation. | `daemon_invocation_service.rs:994` dispatches federation/identity/principal handlers directly; only the catch-all at `:1090` enters LocalRuntime. `LedgerSink` is attached only in `runtime_factory.rs:51`; strict receipt requirements are at `local_daemon_grpc.rs:1165,1602`. | **Main-existing route fork; exposed by worktree receipt hardening** | Open | Register and execute every exact route through LocalRuntime. Do not restore a second CLI receipt writer. Add a mutation test proving one commit and one finalized receipt chain. |
| A22 | P1 | MCP advertises streaming-only abilities as callable tools, but the production provider has no stream implementation and falls back to unary, which the daemon rejects. This is a false capability publication. | `profiles/mcp.rs:212` projects all descriptors; `consent.subscribe`/`voice.subscribe` are streaming-only; default `mcp_stdio.rs:46,331` falls back to unary; daemon rejection is at `daemon_invocation_service.rs:1073`. | **Main-existing capability-model gap; worktree extraction retains it** | Open | Either implement stream-aware MCP invocation over the canonical stream provider or exclude unsupported geometries from `tools/list`; record the state in the capability matrix. |
| A23 | P1 | Voice runtime is registered under Device and Hub while sharing state keyed only by `call_id` and handlers carry no authority context. The same aggregate can therefore be mutated through two receipt owners. | `builtins/resources/voice.rs:53,92,107`; owner truth table `:264` and `:638-671`. | **Main-existing owner ambiguity; branch/worktree amplified to dual registration** | Open | Select one canonical owner. If another plane needs access, model it as an explicit proxy and key/authorize state by canonical owner. |
| A24 | P2 | MCP's 4 MiB frame limit is checked after `read_line` has already allocated the whole newline-free input, so the declared bound does not bound memory. | `src/support/platform/mcp_stdio.rs:228-234`. | **Worktree-new extracted implementation of main behavior** | Open | Perform bounded incremental reads up to `MAX_LINE_LENGTH + 1` and reject before further allocation. |
| A25 | P2 | Normative Axon documents still assign deleted MCP/audio product APIs to Axon; ownership documentation and executable architecture disagree. | `document/rfcs/004-mcp-binding.md:228-259`, `sdk/SDK_PARITY.md:85` in EasyNet-Axon. | **Main-existing; made stale by worktree deletion** | Open | Update normative ownership and parity documents in the same migration that removes product SDK exports; add doc-path checks to the convergence gate. |
| A26 | P2 | The MCP recursion E2E is permanently ignored and invokes removed CLI option `--enable-agent-dispatch`, so a claimed architecture guard has no executable evidence. | `src/daemon/execution/mission/dispatch.rs:1235,1253`; current MCP CLI options at `src/cli/mcp/server.rs:22`. | **Main-existing; stale after CLI evolution** | Open | Replace it with an executable recursion test through canonical child invocation, then remove the ignored obsolete test. |
| A27 | P0 | Axon client SDK silently creates process-local signing identities when the host supplies no key. Identity authority therefore changes after restart and the SDK, rather than the host runtime, becomes a hidden identity provider. | `core/runtime-rs/client-sdk/src/domain/easynet/semantic.rs:191-203,373`; deprecated derivation APIs still delegate to this fallback. | **Main-existing** | Open | Require an explicit host-managed signing authority for authenticated calls. Model anonymous/unsupported explicitly; delete generated-key fallback and deprecated derivation APIs. |
| A28 | P1 | Axon wire conversion treats an empty URA profile as `StrictV2`. A missing protocol fact is silently promoted to current semantics, preserving a pre-RFC architecture inside the canonical parser. | `sdk/rust/src/invocation/wire.rs:31-39`. | **Main-existing** | Open | Reject absent profile at the canonical boundary unless an active SPEC names a negotiated legacy state. If compatibility is required, isolate it in a versioned transport adapter, not the domain parser. |
| A29 | P1 | Agent directory ownership is still dual-model: reads choose `agents/` or `workspaces/`, and a fresh install deliberately writes the legacy `workspaces/` path. The promised migration is not implemented and no referenced SPEC defines the window. | `src/daemon/persistence/config.rs:316-356`; registry migration depends on the same fallback at `agent_registry.rs:446-510`. | **Main-existing** | Open | Implement one atomic directory migration, switch all writers/readers to `agents/`, migrate registry roots, and delete the fallback and compatibility tests in the same change. |
| A30 | P1 | New access-control boundaries accept both canonical URAs and legacy scalar IDs. This keeps two identity models in mutation APIs and permits calls without URA when the legacy field is present. | `src/daemon/ability/builtins/governance/access_control.rs:413-480`; file is absent from main baseline. | **Branch-new** | Open | Make URA the only domain input, migrate every caller, and delete scalar-ID fallback fields from internal models. Preserve an external wire field only in a versioned adapter if the SPEC requires it. |
| A31 | P0 | The only capability labelled `cutover-ready`, `runtime_events`, hard-coded EasyNet product event categories and daemon ability names. A product-specific provider map was certified as the canonical runtime model. | Go/Python now route through explicit `RuntimeEventSubscriptionRoute(s)` catalogs, and the parity matrix includes `runtime-ability-lowering` evidence while downgrading `runtime_events` to `provider-backed`. Default EasyNet provider routes still map to `federation.subscribe_directory_v2`, `events.device.subscribe`, and `session.attach` for public compatibility. | **Branch-new** | Partially corrected; cutover claim removed | Move the default EasyNet route catalog fully downstream or into an explicitly versioned provider package, then reclaim cutover-ready only after downstream duplication is deleted. |
| A32 | P0 | Concrete provider routing is broadly embedded in the canonical SDK. Principal, access-control, directory, receipt, inventory, identity, runtime-admin, and event clients know EasyNet daemon ability literals, so downstream products are not consumers of an independent model. | Production SDK literals include `principal.lifecycle.*`, `policy.request.*`, `meta.list_abilities`, `invocation.history.*`, `inventory.*`, `federation.*`, `system.watch_boot`, and socket/file names. | **Branch-new** | Open | Split every capability into neutral domain interface/model and an EasyNet daemon provider package. Product ability names, sockets, files, and lowering policy belong only to the provider. |
| A33 | P0 | The first child API accepted arbitrary/no causal refs. The first repair binds a real parent admission anchor, but second review found it still did not require child caller to equal the parent admitted callee. | Axon `local_runtime/child.rs`, binding check in `descriptor_bound.rs:347`. | **Worktree-new attempted convergence defect** | Second refactor in progress | AbilityContext derives the anchor and child caller from parent runtime state; signature/admission must reject any caller other than the parent callee. |
| A34 | P0 | Child lifecycle is not yet one bounded state machine. Initial limits/deadlines were added, but admission occurs before atomic registration; public runtime access bypasses limits; deadline/cancel terminal ownership and retention still race. | Axon `task.rs:124,238,322,343`, `cancel.rs:62-83`, `launch.rs`, `handle.rs`. | **Main-existing lifecycle omissions exposed by worktree child API** | Second refactor in progress | Reserve/register before any lifecycle receipt; remove bypass APIs; use one terminal-owner transition; enforce deterministic deadline priority and generation-safe bounded retention. |
| A35 | P1 | Child signing authority is synchronous while production key custody is asynchronous UDS/HSM-style I/O. A direct adapter would block Tokio workers or require a nested runtime bridge. | Axon `invocation/signing.rs:27`; EasyNet-Cli `daemon/identity/self_identity.rs:127`. | **Worktree-new interface defect** | Refactor in progress | Make signing an async owner-bound capability and keep raw key material outside the runtime. Remove the public `hosted()` state that can only fail. |
| A36 | P0 | Direct child submission to Axon LocalRuntime performs descriptor admission but bypasses EasyNet-Cli's route/access policy gate. The generic API is being mistaken for a complete product dispatch contract. | Axon `local_runtime/child.rs:123`; EasyNet Mission still uses the daemon self-loop rather than a daemon-owned policy dispatcher over Axon's prepared child request. | **Worktree-new integration fork** | Not integrated | Axon should produce/accept a generic descriptor-bound child request; a daemon-owned dispatcher must apply product policy/routing before submission. Do not replace the current self-loop until this seam is complete. |
| A37 | P1 | Child signing was not wired into production despite tests presenting it as available. The implementation originally existed only in an alternate constructor/test path. | Production boot now builds `ProductionReceiptAuthorityConfig` and calls `build_production_local_runtime`, which installs owner-bound invocation and receipt signing providers. | **Worktree-new capability-state error, fixed in worktree by Section 24** | Fixed for signer wiring | Keep child invocation as a seam until A36 daemon policy integration is complete; do not mark the child feature provider-backed merely because production signing is now available. |
| A38 | P0 | Purge publication effects originally occurred before durable commit. The first repair moved them post-commit but advanced stages when publisher/credentials were unavailable, so Hub tombstone/revoke could still be permanently omitted. | Durable publication FSM and Hub revoke result store in `agent_lifecycle.rs` / `federation_revoke.rs`; normative contract in `agent-purge-publication-fsm-v1.md`. | **Worktree-new purge transaction defect** | Fixed in worktree | Local facts commit before outbox delivery; finite per-stage retry enters durable reconciliation, claims isolate concurrent drains, and purge transaction IDs deduplicate Hub revoke across restart. |
| A39 | P0 | Purge authorization and filesystem deletion were not commensurate with destructiveness. Manage mapping is now present, but recursive child deletion still had stat/open/unlink pathname swap windows and non-Unix purge regressed to unconditional refusal. | `admission_facade.rs` Manage mapping; descriptor-relative deletion around lifecycle `:1553-1580`; non-Unix branch near `:297`. | **Worktree-new purge security defect** | Second refactor in progress | Retain Manage/destructive consent; identity-check every descriptor-relative child operation; provide a safe cross-platform claim/delete strategy that preserves public purge behavior. |
| A40 | P0 | New receipt validation assumes admission is chain index 0 and terminal is index 1 directly after it. Axon emits Accepted, Admitted, Dispatched, Running, then terminal, so legal finalized results are rejected. | `local_daemon_grpc.rs:1192-1200` vs Axon `local_runtime/task.rs:102` and `handle.rs:731`. | **Worktree-new finalization integration defect** | Refactor in progress | Validate checkpoint state, invocation, bindings, and monotonic index. Full-chain claims require intermediate receipts or a chain proof; never infer adjacency from two checkpoints. |
| A41 | P0 | Remote unary discards runtime receipts and synthesizes `Completed`, so remote execution still has a separate terminal model despite local finalization work. | `local_session_dispatcher.rs:278`, `unary_dispatcher.rs:1592-1612`; `DispatchResult` originally carried one receipt. | **Main-existing** | Refactor in progress | Additive wire fields must carry exact admission and terminal checkpoints; Hub projects terminal state/failure/output only from the terminal receipt/finalized result. |
| A42 | P0 | Remote stream and bidi lack one finalized-result verifier. They check selected fields and forward payload/failure without proving output hash, authority/ability/nonce/causal bindings, or admission-once/order. | `stream_dispatcher.rs:723-754`, `bidi_dispatcher.rs:373`, `pending_dispatch.rs:441`. | **Main-existing; worktree partial hardening** | Refactor in progress | Introduce one `ForwardedFinalizedInvocation` state machine/verifier used by all remote geometries. |
| A43 | P1 | FFI stream/bidi drops receipt proof fields that Go/Python decoders expect, so canonical daemon facts do not survive the public SDK boundary. | `ffi/invocation/mod.rs:4961,5116`; Go `stream.go:457`; Python `stream.py:39`. | **Main-existing FFI; branch-new SDK contract exposes mismatch** | Refactor in progress | Serialize complete admission/terminal checkpoints and proof bindings from the shared finalized projection; generate both decoders from the same schema. |
| A44 | P0 | FFI cancellation locally marks a call Cancelled and returns an outcome without waiting for Axon's terminal receipt. Remote work may continue after the caller observes terminality. | `ffi/invocation/mod.rs:2215,2705,4735`. | **Main-existing cancellation root; worktree synthetic projection** | Refactor in progress | Cancel through Axon lifecycle and await the unique canonical terminal receipt; local cancellation is only a request/transport state, never terminal proof. |
| A45 | P0 | Local carrier error paths send `terminal=true` without a terminal receipt when admission/finalization fails or the stream ends early. This reintroduces terminal inference under error handling. | `local_session_dispatcher.rs:378,433,518`. | **Main-existing carrier behavior; worktree finalization gap** | Refactor in progress | Close as transport failure or request cancellation and await finalization. Never emit a terminal protocol frame without canonical terminal proof. |
| A46 | P0 | SDK lifecycle vocabulary contains impossible or misnamed states. Python `UnaryDispatchPool.close()` transitions to reusable `QUIESCENT`; Go/Python expose Degraded/Reconnecting/reconnect options that no transition reaches; daemon status can overwrite state without transition validation. | Python `transport.py:702,961`; Go `connection.go:13`, Python `connection.py:205`; Go `daemon.go:315`. | **Branch-new** | Open | Define one explicit cross-language FSM. Separate `quiesce` from terminal `close`, implement or remove unreachable states/options, and validate every transition. |
| A47 | P1 | Receipt history leaks an EasyNet local ledger filesystem path into the canonical receipt model, while directory resolution fixes namespace/federation product routing. Protocol facts and product read models are fused. | Go `receipt.go:206`, Python `receipt.py:69`; Go `directory.go:165`. | **Branch-new** | Open | Axon model owns verifiable receipt facts only. EasyNet provider owns ledger storage/history transport and namespace/federation read models. |
| A48 | P1 | SDK dynamic loading searches repository `target/debug` and `target/release`, making development-tree layout part of runtime discovery. | Go `cabi_dynamic.go:230`; Python `_cabi.py:1527`. | **Branch-new** | Open | Require an explicit installed provider/library locator. Keep development lookup in test/dev tooling, not production SDK fallback. |
| A49 | P1 | Untracked compatibility aliases preserve obsolete identities and wire shapes: `device_id <- node_id`, stream `kind <- event`, content-type aliases, `prepared_id <- request_id`, and an always-error legacy identity API. | Python `runtime_environment.py:82`, Go `stream.go:458`, Python `_cabi.py:1591`, Go `runtime_identity.go:86`. | **Branch-new** | Open | Migrate callers and delete aliases. If a released wire contract requires one, isolate it in a versioned provider adapter and represent its lifecycle in the matrix. |
| A50 | P1 | SDK parity and product-neutrality gates are self-referential: the matrix validator checks a fixed 26-item list and does not map exported API inventory back to capability ownership/state. | `tools/scripts/check-sdk-parity-matrix.sh:31`, `sdk-go-python-parity-matrix.yaml:22`. | **Branch-new** | Open | Generate the required set from the public capability manifest/inventory and fail on every unmapped export, product literal, fallback alias, or unreachable lifecycle state. |
| A51 | P0 | Owner-projection revision state is deleted on purge. Recreating the same agent URA restarts revision at 1, so Hub stale-revision fencing rejects the new projection: an ABA bug in durable identity generation. | `owner_projection.rs:289,346`; same-URA recreation in `profiles/bootstrap.rs:255`. | **Main-existing projection model exposed by worktree purge** | Refactor in progress | Preserve a retired high-water/generation record. A recreated URA must continue monotonically or use a new generation identity; tombstone retirement may not erase fencing history. |
| A52 | P1 | Owner-projection cursor updates are unlocked whole-file load/modify/save operations shared by session prelude and purge. Concurrent writers can lose, resurrect, or regress cursor state. | `owner_projection.rs:204`; session writer at `session_initiator/prelude.rs:601`. | **Main-existing; worktree adds concurrent purge writer** | Refactor in progress | Put all cursor operations behind one cross-process lock and atomic compare-and-write repository; test concurrent session/purge writers and revision monotonicity. |
| A53 | P0 | Ability handlers receive a public LocalRuntime and can invoke the lower-level start API directly, bypassing child signing, inherited deadline, depth/fan-out/active/pending limits, and cancellation propagation. | Axon `local_runtime/mod.rs:149`, `launch.rs:123`; existing tests use this route. | **Main-existing public runtime surface; worktree child API failed to close it** | Refactor in progress | Expose a capability-limited AbilityContext only. Make raw start internal/privileged and migrate all handlers/tests to the bounded child path. |
| A54 | P0 | Cancel/task terminal ownership is check-then-act rather than one atomic transition. Simultaneous cancels can disagree, and handler completion may race the external terminal owner. | Axon `cancel.rs:62-83`, `task.rs:343`. | **Main-existing; exposed by worktree child lifecycle** | Refactor in progress | Use one locked/CAS terminal state transition with idempotent cancel success and exactly one receipt finalizer. |
| A55 | P1 | Invocation ID and retention have an ABA path: 64-bit IDs can overwrite an existing map entry, then an old retention record deletes the new live invocation by ID. | Axon `handle.rs:1264`, `task.rs:272`, `launch.rs:87`. | **Main-existing; bounded retention makes it observable** | Refactor in progress | Reject/retry registration collisions and bind retention deletion to an entry generation/token, not ID alone. |
| A56 | P1 | Pending child budget is released when a wrapper converts to a descriptor request, so callers can cache unbounded prepared requests outside the queue limit; inherited deadline is not part of the transferable request. | Axon `local_runtime/child.rs:81`. | **Worktree-new** | Refactor in progress | Carry the budget lease and absolute deadline in the dispatch request until dispatch or drop. |
| A57 | P1 | The Python SDK has no effective static-analysis contract. Full Ruff analysis reports 326 findings, including executed-code defects rather than only export style. | `sdk/python/easynet_sdk/_cabi.py:1231-1232` calls undefined `_string_or_empty`; `__init__.py` relies on broad star exports; no Ruff configuration exists in `pyproject.toml`. | **Branch-new** | Open; behavior tests do not cover the failing branch | Add an explicit lint/type contract to CI, replace star-export assembly with generated explicit exports, fix runtime errors, and keep the public inventory generated from the same manifest. |
| A58 | P1 | Parent-bearing descriptor invocation remains a public route around the runtime-minted child capability. Limits are enforced only when a `ChildDispatchCapability` happens to be attached. | Axon `descriptor_bound.rs:148,416`, `invoke_api.rs:77`. | **Main-existing public API amplified by worktree child refactor** | Open after independent review | Require an unforgeable child capability whenever `parent_invocation_id` is set; separate non-child causal links into a different explicit API. |
| A59 | P1 | Runtime terminal authority remains publicly writable. A consumer can obtain `InvocationCore` and emit a terminal state before the supervisor, bypassing cleanup, child cancellation, retention, and terminal-input sealing. | Axon `handle.rs:696,804,1226`, export at `invocation/mod.rs:82`. | **Main-existing; worktree terminal-permit design did not close it** | Open after independent review | Expose read-only snapshots/progress; keep terminal transitions and mutable core crate-private and permit-bound. |
| A60 | P1 | Terminal ownership can be permanently abandoned after claim. Cleanup callback panic or cancellation-future abort occurs before terminal commit, while `TerminalPermit` has no unwind/drop recovery. | Axon `handle.rs:408,523`, `task.rs:457`, `cancel.rs:90`, `supervisor.rs:418`. | **Main-existing cleanup callbacks; worktree-new terminal claim** | Open after independent review | Make claim-to-commit an unwind- and cancellation-safe transaction; contain callback panic and deterministically finalize `Failed`, or release/recover an uncommitted claim. |
| A61 | P2 | Generation safety stops at the registry. Stale stream/bidi handles retain only the 64-bit invocation ID and can cancel or feed a later invocation after an ID reuse. | Axon `call_mode.rs:332,516`, ID generation at `handle.rs:1374`. | **Main-existing; worktree retention fix is incomplete** | Open | Carry `(invocation_id, generation)` in every public handle and require token equality for cancel/input/close. |
| A62 | P1 | Corrupt credentials can strand purge in `Quarantined` before local commit. Runtime unregister triggers credential-dependent re-enrollment, and compensation repeats the same failing dependency. | `hot_agent_registrar.rs:836`, `ability/dispatch.rs:1369`, `lifecycle.rs:1470,3703`; the dedicated test expected `Committed` but observed `Quarantined`. | **Worktree-new purge integration defect over main registrar coupling** | Open; failing test | Make local unregister/commit independent of federation credentials; publication uses the durable post-commit outbox and compensation must not call the failed external dependency. |
| A63 | P1 | `DaemonMode::Both` exposes device-owned agents but installs neither purge publisher nor device session supervisor, so a committed purge can remain journaled forever. | `boot/invocation/mod.rs:519,634`, `lifecycle.rs:1819`. | **Main-existing topology ambiguity exposed by worktree purge** | Open | Give every mode an explicit provider capability set; reject unsupported purge before mutation or install the same durable publication owner in every device-capable mode. |
| A64 | P1 | The owner-projection persistence schema changed without migration. Existing files lack required `generation` and `lifecycle` fields and fail to deserialize after upgrade. | `persistence/owner_projections.rs:29,94`. | **Worktree-new migration defect** | Open | Add a one-time explicit schema migration with deterministic generation/high-water derivation, then delete the old reader after migration completion; do not silently default protocol facts. |
| A65 | P1 | Non-Unix purge is both uncompilable and path-swap unsafe. A Unix-only identity method is called unconditionally; Windows fallback verifies a path, runs a hook, then recursively deletes by path before final identity recheck. | `agent_lifecycle.rs:117`, `lifecycle.rs:1291,1675-1685`. | **Worktree-new cross-platform defect** | Open | Implement descriptor/handle-relative deletion using Windows file identity and delete-by-handle semantics; add a real Windows target check and swap test. |
| A66 | P0 | The shared forwarded-finalization verifier rejects every legitimate LocalRuntime receipt because it requires `callee_signature`, while Axon currently emits `None`. This breaks remote unary, stream, and bidi cutover. | Axon `binding.rs:109`, `wire.rs:341`; CLI `forwarded_finalization.rs:232`; full suite fails `invoke_stream_dispatches_remote_selected_route_over_presence_session`. | **Worktree-new integration regression over main producer behavior** | Open; failing test | Align the canonical receipt contract first: either Axon signs the callee binding or the verifier proves the actual canonical unsigned form. Do not weaken to shape-only acceptance. |
| A67 | P1 | Forwarded receipt verification originally proved field shape, not authenticity. Arbitrary nonzero hashes and nonempty signature bytes could pass without recomputing `self_hash` or resolving/verifying the signer. | Historical CLI `forwarded_finalization.rs:226-236`; current verifier delegates to `receipts/finalization_projection.rs` and Axon `SignedInvocationReceipt::verify`/`FinalizationCheckpointVerifier`. | **Worktree-new attempted convergence defect, fixed in worktree by Section 23** | Fixed | Keep forwarded finalization on the Axon verifier path; do not reintroduce shape-only receipt acceptance. Production signer custody is Section 24; SDK/Dendrite receipt projection remains C07/A71. |
| A68 | P1 | Cancellation reuses the original invocation nonce/signature and appends unsigned control metadata. Original-first makes cancel a replay; cancel-first consumes the nonce and rejects the real invocation. | `dispatch/request.rs:937`, `daemon_invocation_service.rs:969`, `admission_facade.rs:829`. | **Worktree-new** | Open; unary cancel cannot work correctly | Define a canonical signed lifecycle-control request with its own nonce and binding to the target invocation, or a proof-preserving non-consuming control verification path specified by protocol. |
| A69 | P1 | Stream/bidi cancellation still removes only local FFI readers; Go/Python then synthesize terminal `Cancelled`. The canonical runtime may continue executing. | FFI `mod.rs:1399,2515`; `stream_dispatcher.rs:529`; `bidi_dispatcher.rs:787`; Go `cabi_runtime.go:998`; Python `_cabi.py:1106`. | **Main-existing FFI behavior; branch SDK exposes it as canonical** | Open | Send a lifecycle-control cancellation to Axon, retain the handle until its terminal receipt arrives, and expose transport cancellation separately from lifecycle terminality. |
| A70 | P1 | FFI transport failures are projected as `terminal=true` without terminal receipts, and Go/Python accept that flag. Connection loss is therefore confused with runtime finalization. | FFI status/backpressure projection now emits `terminal=false` plus `transport_terminal=true`; Go/Python stream/bidi/runtime tests pass. | **Main-existing FFI; branch-new SDK acceptance, fixed for FFI status/backpressure by Section 26** | Fixed for observed FFI transport-error projection | Extend the same rule to any remaining transport-only cancel/close surfaces; only a verified terminal receipt may set lifecycle terminal. |
| A71 | P1 | The SDK outcome projection collapses signed causal/authority bindings to kind strings, so distinct parents, delegation scopes, and session authorities cannot be reconstructed or verified. | CLI `ReceiptSummary`, FFI JSON, Go direct runtime, Python direct runtime and Go/Python `RuntimeReceipt` now expose structured `causal_binding` and `authority_binding` beside legacy kind summaries. | **Worktree-new projection defect in branch-new SDK contract, fixed for runtime receipt DTO by Section 26** | Fixed for runtime receipt DTO | Generate these projections from a schema and extend typed trust-state DTOs; keep legacy kind fields as compatibility summaries only. |
| A72 | P2 | Repeated idempotent cancellation corrupts bounded terminal retention because every terminal observation appends a duplicate key; eviction of an old duplicate deletes the current map entry. | `dispatch/cancellation.rs:90-97`. | **Worktree-new** | Open | Make retention insertion idempotent or generation-tagged; evict only when deque token matches the current map generation. |
| A73 | P1 | Voice ownership migration is internally inconsistent with executable catalog contracts. The worktree excludes eight voice abilities from Device registration while Device baseline and real-invoke tests still require them. | Full suite failures in `real_invoke_tests.rs`, `catalog/assembly_tests.rs`, and `conformance.rs`; eight `voice.*` abilities are missing. | **Main-existing owner contradiction; worktree-new incomplete cutover** | Open; eight behavior tests plus two catalog tests fail | Resolve the owner truth table first, migrate descriptor ownership, registration, conformance baselines and tests atomically, then remove the obsolete owner path. |
| A74 | P1 | New published abilities are not fully integrated into the catalog contract. `agent.ability.put` lacks descriptor path, metadata, layer classification and real-invoke coverage; `agent.purge` lacks real-invoke coverage. | Full suite failures in `real_invoke_tests.rs:2330`, `catalog/assembly_tests.rs:220,1109`, `descriptor_paths.rs:256`. | **Worktree-new** | Open | Complete one feature slice across descriptor, metadata, layer, authorization, registration, real invocation and docs, or remove the partially published feature until complete. |

## 4. Redundant and obsolete code inventory

| Cluster | Attribution | Status | Convergence action |
|---|---|---|---|
| Two Rust canonical invocation/admission implementations in Axon | Main-existing | **Still present** | Merge into one domain crate; delete the second encoder/model. |
| Plain, non-proof-bound sign/verify pipeline and verifier branch | Main-existing | **Still present** | Migrate and delete; deprecation is not convergence. |
| Axon product SDK adapters (`ability`, `audio`, MCP, presets, product receipt, tool adapter, utilities) | Main-existing | Mostly deleted in worktree | Verify callers; delete remaining voice/remote-desktop product exports and downstream product schemas from canonical SDK. |
| EasyNet-Cli local ledger and invocation receipt projection | Main-existing | Deleted in worktree | Keep Axon finalized receipt as source of truth. |
| Daemon `runtime_dispatch` plus adapter | Main-existing | Deleted in worktree | Canonical invocation service owns dispatch. |
| Federation gateway/init/publish legacy paths | Main-existing | Deleted in worktree | Keep one federation application path; verify no fallback registration remains. |
| CLI-owned mission implementation duplicated application orchestration | Main-existing | Moved in worktree | Finish migration to child invocation and remove transitional daemon self-loop. |
| Four checked-in RFC `.bak` snapshots (about 4,248 lines) | Main-existing | Deleted in worktree | Git history is the archive; do not restore backups. |
| Repeated Go/Python C ABI handle, state, and JSON projection logic | Branch-new | Still present | Generate bindings/projections from one ABI schema and keep language-specific code to ownership/lifetime wrappers. |
| `DaemonControl`/`DaemonLifecycleFacade`/`DaemonHandleFacade` parallel public layers | Branch-new | Still present | Collapse to canonical runtime environment/handle model; keep product convenience facade downstream. |
| Direct `AgentRegistry` load/save logic across catalog, lifecycle, and discovery | Main-existing | Still present | Move mutations behind one transactional repository aggregate. |
| Thirty-one exact daemon route handlers outside LocalRuntime | Main-existing | Still present | Register them as runtime abilities and delete the parallel direct-dispatch table. |
| Ignored MCP recursion test using removed CLI flags | Main-existing | Still present but dead | Replace with live canonical-invocation E2E and delete obsolete test. |
| `agents/` plus `workspaces/` read/write fallback and migration tests | Main-existing | Still present | Execute one migration and delete the old directory model immediately. |
| Process-local generated signing identity plus deprecated derivation APIs | Main-existing | Still present | Require host signing authority and delete both fallback and deprecated entrypoints. |
| URA plus legacy scalar identity fields in access-control boundaries | Branch-new | Still present | Migrate callers to URA-only domain requests and delete scalar compatibility logic. |
| Product ability-name lowering duplicated in Go and Python canonical packages | Branch-new | Still present | Define one provider manifest/schema; move generated lowering into the EasyNet provider and keep it out of the canonical model. |
| Runtime parent tree plus caller-supplied causal receipt DAG | Worktree-new | Refactor in progress | Keep one runtime-minted causal relation and delete arbitrary child refs. |
| Alternate child-runtime constructor used only by tests | Worktree-new | Still present | Mark seam until production policy and key custody consume it; remove unusable public states. |
| Per-geometry remote finalization checks and receipt projection | Main-existing/worktree | Refactor in progress | Replace with one forwarded-finalization verifier and one wire projection. |
| FFI local cancel outcome beside Axon terminal receipt | Main-existing | Refactor in progress | Delete local terminal synthesis after canonical cancel/await cutover. |
| Go/Python handwritten provider ability tables and C ABI state projections | Branch-new | Still present | Generate provider lowering/bindings from one manifest and delete per-language copies. |
| Compatibility field aliases not represented in capability lifecycle | Branch-new | Still present | Migrate callers and delete; do not retain silent fallback. |

## 5. Features that do not belong in the canonical SDK

| Feature/surface | Why non-conforming | Correct owner |
|---|---|---|
| EasyNet package names and `easynet_*` C ABI | Product identity, not runtime ontology | EasyNet provider adapter/distribution |
| Daemon start/attach/discover/stop policy | Product process lifecycle | EasyNet-Cli provider |
| Device/Hub/Both modes | EasyNet deployment topology | EasyNet product layer |
| `~/.easynet/control.json` and EasyNet directory discovery | Product directory model | EasyNet transport/environment provider |
| EasyNet-specific receipt history/projection | Product read model | Downstream consumer over canonical Axon receipts |
| Voice call lifecycle and media negotiation | Product capability | Voice product/plugin repository |
| Remote desktop session/media backend contract | Product capability | Remote desktop plugin/repository |
| MCP and EasyNet hook typed product clients | Product interoperability adapter | Downstream integration package |
| Federation revoke payload exported by one SDK only | Unowned language-specific capability | Add to canonical manifest for both languages or move downstream |
| Python-only child/control clients | Language-specific architecture fork | Canonical manifest plus equivalent Go state, or explicit unsupported state |
| Runtime-event mapping to federation/device/session abilities | Product event taxonomy and daemon route names | EasyNet runtime-event provider |
| Principal/access/directory/receipt/inventory ability-name lowering | Product transport protocol, not domain model | EasyNet daemon provider generated from one manifest |
| `InvocationSigningAuthority::hosted()` that always errors | Unusable public compatibility state | Unsupported until hosted attestation has a real provider |
| Receipt `ledger_path` and repository `target/{debug,release}` lookup | Product storage/development environment | EasyNet provider and development tooling |

## 6. Capability-state truth

The matrix currently reports Go/Python parity because every listed row has the
same status. That is necessary but insufficient:

| Check | Result | Interpretation |
|---|---:|---|
| Matrix capabilities | 26 | Only declared rows are compared. |
| Go public symbols | 593 | Much larger actual architecture surface. |
| Python public symbols | 312 | Surface is independently shaped. |
| Matrix rows with unequal Go/Python status | 0 | Label parity, not model parity. |
| Public capabilities known to escape the matrix | At least 4 | The four-state invariant is not closed. |

`cutover-ready` must mean the downstream consumer uses the provider and the old
owner/duplicate implementation has been deleted. Passing provider tests alone
is only `provider-backed`.

## 7. Why a zero architecture-gate result is not convergence

The current gate is useful but intentionally syntactic. It catches direct
EAL/Mission dispatch, duplicate terminal writers, Axon Rust product adapter
modules, daemon-to-CLI imports, and selected retired names. It does not prove:

- one canonical implementation per language;
- one route through LocalRuntime for every mutation;
- cryptographic receipt verification;
- capability-manifest closure over exported APIs;
- product-neutral package, directory, lifecycle, and provider naming;
- Go/Python model equivalence;
- one owner for stateful abilities;
- absence of deprecated/fallback execution paths;
- callable geometry for advertised MCP tools; or
- consistency between normative ownership documents and code.

The gate should be expanded only as each root refactor lands. Adding regexes
that merely bless the current split would turn the gate into false assurance.

## 8. Required convergence order

1. Freeze new SDK surface additions and make the capability manifest closed.
2. Select the single canonical Axon Rust invocation/admission model.
3. Remove the plain-signature compatibility architecture.
4. Complete one Axon-owned finalized result for unary, stream, and bidi.
5. Migrate EAL/Mission/Think to signed child invocation; remove self-loop and
   direct catalog production paths.
6. Separate the neutral runtime SDK/ABI from the EasyNet daemon provider.
7. Move voice, remote desktop, MCP, and EasyNet semantics downstream.
8. Resolve the voice owner truth-table conflict and migrate registrations.
9. Finish lifecycle FSMs for agent purge and SDK connections.
10. Move all exact daemon routes into LocalRuntime and close the mutation/receipt split.
11. Make MCP capability publication geometry-aware and restore executable recursion evidence.
12. Remove hidden identity/profile/directory compatibility models after explicit migration.
13. Move every concrete daemon ability-name lowering out of the canonical SDK.
14. Complete child causality, policy dispatch, async signing, cancellation, deadline, and resource limits as one lifecycle.
15. Carry and verify one finalized result across remote unary/stream/bidi and FFI without geometry-specific inference.
16. Replace the self-referential SDK gate with manifest-to-export closure and one cross-language lifecycle FSM.
17. Split procedural modules only along the resulting owner/state boundaries,
    then delete obsolete implementations immediately.

## 9. Verification status

Final verification of the reviewed worktree produced mixed results:

- `cargo check --all-targets --all-features`: passed.
- `cargo fmt --all -- --check`: passed.
- `bash tools/scripts/check-architecture-convergence.sh`: passed.
- `bash tests/scripts/test_check_architecture_convergence.sh`: passed.
- `bash tools/scripts/check-sdk-product-neutrality.sh`: passed.
- `bash tools/scripts/check-sdk-parity-matrix.sh`: passed.
- `go test ./...` in `sdk/go`: passed.
- `PYTHONPATH=sdk/python sdk/python/.venv/bin/python -m pytest sdk/python/tests -q`:
  334 passed and 94 subtests passed.
- Axon `sdk/rust`: full all-feature/all-target test suite passed in the child
  implementation round (227 library tests plus integrations; two existing
  generator/vector tests ignored), and formatting/diff checks passed.
- `codegraph sync .`: completed in both EasyNet-Cli and EasyNet-Axon after the
  final implementation/review edits.

Repository-wide EasyNet-Cli tests do **not** pass: 3,866 passed, 18 failed and
5 were ignored. The failures prove unresolved integration defects rather than
test flakiness:

- purge fail-closed/credential state mismatch and old CLI purge expectation;
- incomplete `agent.purge` / `agent.ability.put` catalog and real-invoke slices;
- eight voice abilities removed from the Device owner without an atomic SPEC,
  baseline and test migration;
- forwarded-finalization rejects valid LocalRuntime receipts;
- carrier bidi admission frame no longer carries the expected receipt;
- FFI cancel state/terminal expectation mismatch.

Full Ruff analysis also fails with 326 findings. In addition to broad star
exports and unused imports, `_cabi.py:1231-1232` calls an undefined helper, so
the result cannot be dismissed as formatting debt.

The passing architecture and SDK gates are therefore necessary but not
sufficient. They prove selected forbidden patterns and fixed matrix rows only;
they do not prove owner uniqueness, cryptographic receipt verification, public
API closure, lifecycle reachability or cross-language semantic isomorphism.
This worktree is not stable and must not be committed.

## 10. CodeGraph re-audit addendum

This addendum records the second-pass SDK/conformance review requested on
2026-07-14. CodeGraph was synced before this pass and reported 896 indexed
files, 30,615 nodes, and 109,122 edges. `git diff main...HEAD` shows the
multi-language SDK and conformance inventory are branch additions; the C ABI
provider prefix existed on main and was expanded by this branch.

### 10.1 SDK lifecycle and naming fixed points

| Area | Evidence | Architecture break | Attribution | Required convergence |
|---|---|---|---|---|
| Go daemon lifecycle surface | `sdk/go/daemon.go` exports `DaemonMode`, `DaemonLifecycleState`, `DaemonStatus`, `DaemonTransport`, `DaemonControl`, `DaemonHandle`, `Start`, `Attach`, `Discover`, and `ConnectLocal`. CodeGraph impact for `DaemonHandle` reaches 114 affected symbols. | The root lifecycle model is named after a product/provider process, so the SDK cannot be the canonical runtime model. | **Branch-new** | Introduce neutral `RuntimeHostMode`, `RuntimeLifecycleState`, `RuntimeHostStatus`, `RuntimeLifecycleTransport`, `RuntimeLifecycle`, and `RuntimeHandle`; old `Daemon*` names may only remain as source-compatible aliases until a SPEC cutover. |
| Python daemon lifecycle surface | `sdk/python/easynet_sdk/daemon.py` exports `DaemonMode`, `DaemonControl`, `DaemonLifecycleFacade`, `DaemonHandleFacade`, `DaemonHandle`, `start_daemon`, `attach_runtime_host`, and `discover_runtime_host`. | Python repeats the Go product/process naming and adds facade layers around the same non-neutral model. | **Branch-new** | Make the canonical implementation neutral first; expose `start_runtime_host`, `attach_runtime_host`, `discover_runtime_host`, and neutral facade classes. |
| Runtime administration depends on daemon types | CodeGraph shows Go `NewRuntimeAdminClient(control *DaemonControl, ...)` and Python `RuntimeAdminClient.start(...) -> DaemonHandle`, `status(handle: DaemonHandle) -> DaemonStatus`. | A runtime-neutral admin API is internally and publicly parameterized by daemon lifecycle types. | **Branch-new** | `RuntimeAdminClient` must depend on neutral lifecycle/control interfaces. Daemon-specific provider objects stay below the provider boundary. |
| SDK environment root returns daemon control | Go `SdkEnvironment` and Python `SdkEnvironment` expose daemon control/discovery methods and use daemon process facts as the environment root. | The SDK environment confuses canonical runtime environment with EasyNet daemon installation/discovery policy. | **Branch-new** | Environment should expose neutral runtime lifecycle/provider locator facts. `.easynet`, control.json, credentials, and device/hub topology belong to the EasyNet provider. |
| C ABI provider wrapper is named as daemon transport | Go `CABIDaemonTransport`, `OpenCABIDaemonTransport`, `NewCABIDaemonControl`; Python `CABIDaemonTransport`. | Language facades leak provider process naming into SDK canonical provider names. | **Branch-new over main ABI** | Add neutral provider wrapper names such as `CABIRuntimeHostTransport` / `OpenCABIRuntimeHostTransport`; keep C symbols as provider ABI implementation details. |
| Conformance canonical public API registers daemon names | `sdk/conformance/canonical-public-api.json` lists `CABIDaemonTransport`, `DaemonHandle`, `DaemonControl`, `DaemonInvocationTransport`, `DirectDaemonRuntimeTransport`, and method members such as `DaemonControl.Start`. | The conformance suite turns provider/legacy names into canonical SDK truth, so future refactors will be judged against the wrong model. | **Branch-new** | Replace flat canonical-name inventory with concept records: canonical neutral names, capability state, legacy public aliases, and provider bindings. |
| Java public package and artifact | `sdk/java/pom.xml` uses `groupId=run.easynet`, `artifactId=easynet-daemon-sdk`; sources use `package run.easynet.daemon`. | Java package identity permanently encodes EasyNet + daemon even when classes are named `Runtime*`. | **Branch-new** | Create a runtime-neutral package/artifact target. Record `run.easynet.daemon` only as a migration/adapter package if public compatibility is required. |
| Swift public module | `sdk/swift/Package.swift` and source/test paths use `EasyNetDaemonSDK`. | Swift module import surface encodes product/provider naming. | **Branch-new** | Create a runtime-neutral Swift module; old module can only be a SPEC-bound compatibility product. |
| C ABI symbol prefix | `include/easynet_cli.h` and `src/ffi/**/*.rs` export `easynet_*` and `runtime_host_*` symbols. | The ABI is product/provider-branded, but it is already a stable transport ABI and cannot be renamed as a local cleanup. | **Main-existing; branch-expanded** | Treat as versioned provider binding. Do not expose it as canonical SDK model; rename only through an explicit ABI bump and downstream migration. |

### 10.2 Redundant code and compatibility inventory from the second pass

| Redundant or obsolete surface | Evidence | Attribution | Required action |
|---|---|---|---|
| Parallel `DaemonControl` / `RuntimeAdminClient` ownership | `RuntimeAdminClient` delegates lifecycle to `DaemonControl` while exposing runtime-neutral naming. | **Branch-new** | Collapse admin/lifecycle ownership into neutral runtime lifecycle control. |
| Go/Python duplicated C ABI JSON and handle projection | `sdk/go/cabi_runtime.go` and `sdk/python/easynet_sdk/_cabi.py` independently project daemon handles, status JSON, open-runtime behavior, and errors. | **Branch-new** | Generate provider bindings/projections from one ABI schema; keep only language lifetime wrappers handwritten. |
| Flat canonical public API inventory | `canonical-public-api.json` cannot distinguish canonical concepts, source-compatible aliases, and provider ABI names. | **Branch-new** | Move to a concept/capability schema with `canonical_names`, `legacy_public_names`, `provider_bindings`, and four-state capability status. |
| Direct daemon runtime names | Inventory contains `DirectDaemonRuntimeConnector`, `DirectDaemonRuntimeTransport`, and Python `DaemonInvocationTransport`. | **Branch-new** | Rename canonical surface to `DirectRuntimeConnector`, `DirectRuntimeTransport`, `RuntimeInvocationTransport`; keep daemon names only as temporary aliases. |
| Product capability IDs inside SDK matrix | Matrix previously included `daemon_lifecycle`; the working tree now uses `runtime_lifecycle` and `runtime/control_only`, while source-compatible `Daemon*` public aliases remain inventoried separately. | **Branch-new** | Continue separating canonical capability records from legacy public aliases and provider/downstream evidence. |
| Java/Swift package tests hard-code product module paths | Java test uses `run.easynet.daemon`; Swift test imports `EasyNetDaemonSDK`. | **Branch-new** | Add neutral package/module tests and demote product imports to adapter compatibility tests. |
| Pre-URA documentation and historical RFC prose | Current scan still finds identity-oriented pre-URA prose in historical docs/RFCs; SDK production code mostly avoids non-URA identity naming except transport locator types. | **Main-existing docs; branch-new SDK semantic drift** | Replace semantic identity prose with URA in normative docs; keep locator naming only for HTTP/gRPC transport locators. |

### 10.3 Non-conforming SDK features by owner

| Feature | Why it violates the SDK principles | Correct owner/state |
|---|---|---|
| `DaemonMode` with `device`, `hub`, and `both` | EasyNet deployment topology is not a canonical runtime concept. | EasyNet provider; SDK canonical state should be generic runtime host mode or unsupported. |
| `.easynet/control.json` discovery | Product directory and process-discovery policy. | EasyNet provider-backed runtime host locator. |
| `CABIDaemonTransport` as public SDK type | C ABI provider is exposed as the runtime model. | Provider binding under a neutral runtime lifecycle transport interface. |
| Java `run.easynet.daemon` and Swift `EasyNetDaemonSDK` | Product/provider naming is embedded in import paths. | Neutral package/module plus product adapter package only if SPEC requires compatibility. |
| `runtime_lifecycle` capability row | Capability row now uses neutral runtime lifecycle naming and classifies the EasyNet implementation as provider-backed. | Move remaining default provider lifecycle policy downstream or into explicit provider records before any cutover-ready claim. |
| Runtime events mapped to EasyNet daemon ability names | Product event taxonomy is treated as canonical SDK. | Generic runtime event cursor/stream model; EasyNet provider owns ability-name lowering. |
| Receipt history with local ledger path | Product storage/read model leaks into canonical receipt surface. | Axon receipt facts in SDK; EasyNet provider owns ledger storage/history transport. |
| Development-tree dynamic library lookup | Runtime discovery depends on repository layout (`target/debug`, `target/release`). | Dev/test helper only; production provider locator must be explicit or installed. |

## 11. Authoritative independent re-audit snapshot

Status meanings are strict: `fixed` means the named defect is removed in the
working tree, not that the surrounding capability is cutover-ready; `partial`
means one path or layer converged while another executable path remains;
`open` means the root abstraction or a production path is still defective.

This snapshot contains **90 findings: 40 open, 14 partial and 36 fixed in the
working tree**. Severity distribution is **34 P0, 50 P1 and 6 P2**. Every row
states whether the root existed on `main`, was introduced by this branch, or
was introduced by the uncommitted convergence work. Final CodeGraph indexes:
EasyNet-Cli **911 files / 31,023 nodes / 111,181 edges**; EasyNet-Axon **872
files / 20,789 nodes / 66,835 edges**.

### 11.1 Canonical SDK and ownership

| ID | Sev. | Status | Baseline | Current diagnosis |
|---|---|---|---|---|
| A01 | P0 | open | Branch-new | Public packages and artifacts are still EasyNet/daemon-owned rather than a product-neutral canonical runtime SDK. |
| A02 | P0 | fixed | Main-existing | Client SDK, runtime and Dendrite now delegate descriptor-bound canonicalization to `easynet-axon`; the duplicate client-sdk encoder was removed. |
| A03 | P0 | open | Main-existing | Plain canonical bytes, sign, verify and admission APIs remain executable and publicly re-exported in `sdk/rust/src/invocation`. |
| A04 | P0 | partial | Main-existing | Local runtime unary/stream/bidi wait for Axon finalization, but exact routes and selected carrier/FFI errors still synthesize terminality. |
| A05 | P0 | partial | Main-existing plus worktree transition | Mission/EAL no longer call the catalog directly in production, but the daemon-socket self-loop still bypasses the Axon child capability seam. |
| A06 | P0 | open | Branch/worktree-new | Receipt acceptance checks shape and binding but explicitly performs no cryptographic verification. |
| A07 | P0 | partial | Branch-new | Export inventory closure improved, but only two of 26 capability rows map canonical exports; public capabilities still escape the four-state model. |
| A08 | P0 | open | Branch-new | Go and Python remain independently shaped: 598 versus 314 canonical symbols and divergent endpoint/lifecycle models without explicit unsupported states. |
| A09 | P0 | partial | Main-existing | Voice/remote-desktop Rust/proto ownership moved downstream, but Axon Python and client-sdk still expose MCP/orchestrator/tool/audio/EasyNet semantics. |
| A10 | P0 | fixed | Main-existing, fixed in worktree | Voice descriptor ownership, truth table and catalog exclusion now agree on Hub ownership. |
| A11 | P0 | fixed | Main-existing, fixed in worktree | Stop and purge are separate; purge has explicit durable local and publication state machines, locking, and recovery. |
| A12 | P0 | fixed | Branch-new, fixed in worktree | Python `close()` is terminal and reusable draining is explicitly named `quiesce()`, with deterministic lifecycle tests. |
| A13 | P1 | open | Branch-new | Device/Hub/Both, daemon paths, Hub endpoint and `.easynet/control.json` remain canonical SDK concepts instead of provider policy. |
| A14 | P1 | fixed | Main-existing | Daemon-to-CLI upward imports were removed and the boundary is mechanically enforced. |
| A15 | P1 | fixed | Main-existing | Duplicate CLI receipt/ledger projections were deleted; product paths now consume the Axon receipt model. |
| A16 | P1 | open | Branch-new SDK; main docs | URA spelling is mostly corrected, but Go silently defaults missing Agent `owner_kind` while Python rejects it; normative pre-URA identity prose remains. |
| A17 | P1 | fixed | Worktree placement defect | MCP stdio implementation is owned by `daemon/execution/mcp`; the generic support owner was removed. |
| A18 | P1 | open | Mostly main-existing | `dispatch.rs`, FFI invocation, admission and lifecycle modules remain procedural responsibility accumulators spanning multiple aggregates. |
| A19 | P1 | partial | Main ABI plus branch amplification | Neutral facade names exist, but canonical packages load `libeasynet_cli`, call `runtime_host_*`, and expose product aliases. |
| A20 | P1 | partial | Main-existing | Purge has a transaction owner; catalog, MCP and dispatch still directly load/save the shared mutable `AgentRegistry`. |
| A21 | P0 | open | Main-existing | Thirty-one exact daemon routes execute outside `LocalRuntime`, return no canonical receipts and can mutate before a strict client rejects the response. |
| A22 | P1 | fixed | Main-existing | MCP publication is geometry-aware and excludes Stream/Bidi abilities from the unary tool provider. |
| A23 | P1 | fixed | Main-existing/amplified | Voice state keys include `(authority_ura, call_id)` and handlers enforce the Hub callee. Realm durability remains open under A86. |
| A24 | P2 | fixed | Worktree-new | MCP frame reads are incrementally bounded with `fill_buf`; a newline-free frame cannot allocate past the declared bound. |
| A25 | P2 | partial | Main-existing docs | RFC/parity ownership text changed, but Axon README/SDK skill still advertise removed MCP APIs and the product boundary gate misses those paths. |
| A26 | P2 | open | Main-existing | The ignored MCP recursion E2E still invokes removed `--enable-agent-dispatch` behavior. |
| A27 | P0 | fixed | Main-existing | Authenticated client/federation calls now require injected signing authority and fail closed before transport. Production receipt signer custody is closed by A76/Section 24. |
| A28 | P1 | fixed | Main-existing | Empty/unknown URA profiles are rejected by strict wire parsing and covered by tests. |
| A29 | P1 | open | Main-existing | `agents/` and `workspaces/` remain live production roots; mixed installations can resolve old rows into the wrong global root instead of migrating per agent. |
| A30 | P1 | open | Branch-new | Access-control mutation boundaries still accept URA and legacy scalar identities; scalar-only calls remain executable. |
| A31 | P0 | fixed (narrow) | Branch-new | Runtime-event routes now come from explicit EasyNet provider manifests and the false `cutover-ready` claim was removed. |
| A32 | P0 | open | Branch-new | Principal, access, directory, receipt, inventory, signing and admin modules still embed EasyNet daemon route literals in canonical SDK packages. |

### 11.2 Invocation, lifecycle and projection

| ID | Sev. | Status | Baseline | Current diagnosis |
|---|---|---|---|---|
| A33 | P0 | fixed | Worktree convergence defect | Child caller is derived from the executing parent callee and admission rejects a mismatched child caller. |
| A34 | P0 | fixed | Main omissions exposed in worktree | Child registration, quotas, terminal commit, retention and deadline ownership are generation-bound and serialized. |
| A35 | P1 | fixed | Worktree interface defect | Invocation and receipt signing authorities are asynchronous owner-bound capabilities. |
| A36 | P0 | open | Worktree-new integration fork | Axon exposes a policy-neutral prepared child request, but CLI production never consumes it and Mission still uses the daemon self-loop. |
| A37 | P1 | fixed | Worktree-new capability-state error | Production daemon now injects owner-bound invocation/receipt signing providers; child dispatch remains a policy-integration seam under A36. |
| A38 | P0 | fixed | Worktree-new purge transaction | Publication is a finite durable FSM; terminal failures retain the identity fence and require typed manual retry. Hub revoke is transaction-idempotent across restart. |
| A39 | P0 | partial | Worktree-new purge security | Manage authorization and Unix descriptor-relative deletion exist; destructive consent is advisory and non-Unix purge rejects the public operation. |
| A40 | P0 | fixed | Worktree-new integration defect | Receipt verification now requires monotonic terminal indices rather than assuming admission and terminal are adjacent. |
| A41 | P0 | partial | Main-existing | Presence unary verifies two receipts; escalation still synthesizes `Completed` and the peer path returns an unverified response. |
| A42 | P0 | fixed | Main gap, fixed in worktree | One `ForwardedFinalizationVerifier` now owns ordering/cardinality for remote stream and bidi. Trust-state projection remains C07/A71. |
| A43 | P1 | fixed | Main FFI gap | Stream and bidi FFI frames preserve admission and terminal receipt fields. |
| A44 | P0 | fixed (projection) | Main-existing | Unary FFI cancel is non-terminal `CancelRequested` until a canonical terminal receipt arrives; the cancel protocol itself remains broken under A68. |
| A45 | P0 | partial | Main-existing | Stream carrier failure is non-terminal and requests cancellation; bidi and pre-runtime unary errors still emit terminal frames without receipts. |
| A46 | P0 | partial | Branch-new | Go validates lifecycle transitions; Python still overwrites status directly and the cross-language FSM is not isomorphic. |
| A47 | P1 | open | Branch-new | Canonical receipt history exposes local `ledger_path` and EasyNet history routes, fusing protocol facts with product storage. |
| A48 | P1 | open | Branch-new | Production SDK loaders search repository `target/debug`, `target/release` and `deps`. |
| A49 | P1 | open | Branch-new | Executable aliases/fallbacks remain for `node_id`, stream event kind, content type, prepared request IDs and an always-error identity API. |
| A50 | P1 | open | Branch-new | The parity set is hard-coded and committed reports are treated as proof; capability state is not derived from the public export graph. |
| A51 | P0 | fixed | Main-existing, exposed by purge | Retired owner-projection cursors retain revision/generation high-water and recreation stays monotonic. |
| A52 | P1 | fixed | Main-existing plus purge writer | Cursor load/update uses process and file locks plus atomic writes; concurrent writers cannot regress the projection. |
| A53 | P0 | fixed | Main-existing | `AbilityContext` no longer exposes raw `LocalRuntime`; handlers receive capability-scoped child operations. |
| A54 | P0 | fixed (narrow) | Main-existing | Terminal commit is private and serialized by one emit gate. Concurrent cancel reason consistency remains A79. |
| A55 | P1 | fixed | Main-existing | Invocation ID collisions are rejected/retried and retention deletion is generation-bound. |
| A56 | P1 | fixed | Worktree-new | Transferable child requests carry their pending lease, capability and absolute deadline until dispatch/drop. |
| A57 | P1 | partial | Branch-new | CLI Python Ruff passes and exports are explicit, but no typing contract or CI enforcement exists. Axon Python debt is separate A85. |
| A58 | P1 | fixed | Main-existing | Parent-bearing calls require the runtime-private child capability; generic wrappers reject caller-supplied parent IDs. |
| A59 | P1 | fixed | Main-existing | Public progress emission rejects terminal states and mutable terminal commit is crate-private. |
| A60 | P1 | fixed | Main-existing/worktree claim | Supervisor-owned cleanup survives waiter cancellation and contains callback panics before finalization. |
| A61 | P2 | fixed | Main-existing | Unary/stream/bidi handles carry generations and controls verify `(invocation_id, generation)`. |
| A62 | P1 | fixed | Worktree-new | Local hot-agent unregister/recovery no longer depends on credentials; corrupt-credentials purge coverage exists. |
| A63 | P1 | fixed (fail-closed) | Main topology ambiguity | Device-capable `Both` mode is explicitly unsupported for this mutation and rejects before state changes. |
| A64 | P1 | fixed | Worktree-new | Owner-projection schema v2 explicitly migrates legacy cursors with deterministic generation/lifecycle facts. |
| A65 | P1 | open | Worktree-new | Non-Unix purge remains unavailable; delete-by-handle and Windows path-swap verification are absent. |
| A66 | P0 | fixed (signature presence) | Worktree regression | Axon now signs lifecycle receipts, so the earlier “all valid receipts lack signatures” integration break is removed. Production signer custody is closed by A76/Section 24. |

### 11.3 Remaining proof, control and conformance defects

| ID | Sev. | Status | Baseline | Current diagnosis |
|---|---|---|---|---|
| A67 | P1 | fixed | Worktree-new | Forwarded receipt verification now decodes wire receipts through Axon, resolves the signer key and verifies Ed25519/self-hash before projecting a trusted finalization checkpoint. |
| A68 | P1 | open | Worktree-new | Cancellation reuses the original signed invocation/nonce and appends unsigned metadata, so replay admission consumes or rejects the wrong operation. |
| A69 | P1 | open | Main FFI plus branch SDK | Stream/bidi cancel only closes local readers; Go/Python synthesize terminal `Cancelled` while canonical work may continue. |
| A70 | P1 | fixed by Section 26 for FFI status/backpressure | Main FFI plus branch SDK | Transport status and callback backpressure now project `terminal=false`, `transport_terminal=true`; canonical terminality remains receipt-backed. |
| A71 | P1 | fixed by Section 26 for runtime receipt DTO | Branch/worktree-new | Receipt DTOs now carry structured `causal_binding` and `authority_binding` objects while retaining legacy kind summaries. |
| A72 | P2 | fixed | Worktree-new | Terminal retention insertion is idempotent and repeated cancellation cannot evict the live entry through duplicate FIFO keys. |
| A73 | P1 | partial | Main owner contradiction plus worktree cutover | Catalog ownership is Hub-only, but nine voice real-invoke paths still fail after the owner migration. |
| A74 | P1 | fixed | Worktree-new | `agent.ability.put` and `agent.purge` now have descriptor paths, metadata/layer classification, catalog registration and focused real-invoke coverage. |
| A75 | P0 | open | Worktree-new | Remote bidi writes the admission receipt into `terminal_receipt`; Hub reads `admission_receipt`, sees data before admission and rejects the session. |
| A76 | P0 | fixed by Section 24 | Main lacked signed custody; worktree added wrong fallback | Production receipt signer custody now uses daemon key-service identities through `ProductionReceiptAuthorityConfig` and `build_production_local_runtime`; `LocalRuntime::new()` is fail-closed and local-fast signing is an explicit test/probe seam. |
| A77 | P1 | open | Main-existing | Public `new_receipt` and `receipt_to_wire` allow an unsigned `InvocationReceipt` to cross the signed receipt boundary. |
| A78 | P1 | partial | Main-existing plus branch SDK exposure | Language SDK public DTO/control seams now use observation-only JSON and runtime-bound control capabilities; C ABI/provider lifecycle controls remain ID/token-oriented and still need generation/runtime ownership closure. |
| A79 | P1 | open | Worktree-new | `CancelState` latches the first reason, but each concurrent canceller attempts terminal commit with its own local reason. |
| A80 | P0 | fixed by Sections 20-21 | Branch-new | SDK conformance evidence is now runner-owned and live: adapter reports carry evidence hashes, committed `status=passed` is rejected by schema/self-test, language records require execution proof, and the report wrapper rejects mixed nonce/tree attestations. |
| A81 | P1 | fixed by Section 22 | Branch-new | Product-neutrality now reads canonical roots from the public API manifest, includes the active `sdk/go/runtimeevents` provider-neutral core and verifies EasyNet provider delegation separately. |
| A82 | P1 | partial by Section 22 | Branch-new | The false-conformance path is fixed: daemon-named aliases, including snake_case Python functions, are quarantined from the canonical graph. SPEC-permitted REQ-LANG-5 source aliases remain as non-canonical cutover debt until a major-version removal. |
| A83 | P1 | fixed by Section 22 | Branch-new | The normative SDK spec now distinguishes the provider-neutral canonical runtime model from the EasyNet provider C ABI; daemon lifecycle symbols are explicitly scoped to the provider binding. |
| A84 | P1 | fixed by Section 22 | Branch-new | The repository workflow now installs pinned SDK toolchains and runs Go/Python/Node/Java/Swift SDK tests, Ruff, strict mypy, inventory/product-neutrality gates, live conformance/parity and exact C ABI export checks. |
| A85 | P1 | open | Main-existing | Axon Python still exports MCP/orchestrator/tool/audio product APIs, has no Ruff/type contract, retains star exports and an undefined `MessageInbox` reference. |
| A86 | P1 | open | Main-existing state model, exposed by Hub ownership | Voice is specified as realm-shared but stores calls in an in-process `Mutex<HashMap>`; restart loses state and multiple Hub replicas diverge. |
| A87 | P1 | fixed | Worktree-new | Bounded drains claim entries independently, attempt each transaction once per drain, and stop automatically retrying reconciliation-required entries. |
| A88 | P1 | open | Branch-new | Revoke audit falls back from missing `actor_ura` to scalar `owner_user_id` and persists it as if it were a validated URA. |
| A89 | P2 | open | Branch/worktree-new | Admission explain classifies every `voice.*` action as Stream, including RPC abilities such as create/list, making audit output semantically false. |
| A90 | P1 | open | Main lifecycle omission | Caller disconnect on local/remote stream and remote bidi does not propagate canonical cancellation; downstream work can outlive its consumer. |

#### A38/A87 round-3 repair note

The purge publication repair now treats the owner-cursor generation as the
Agent incarnation. Device outbox claims use persisted drain epochs and
monotonic delivery fences under a dedicated cross-process bounded-drain guard;
wall-clock lease expiry is no longer an ownership decision. Tombstones use a
projection-only publisher and the Hub persists their command/fence before
read-model mutation.

Hub hosted-Agent inventory is now durable and shares one locked repository
with the revoke FSM. Revoke persists `Prepared`, conditionally retires the
exact generation, persists `Applied` with an exact outcome, and only then
updates generation/session-bound read models. Full logical command digest
rebinding, old-generation ABA replay, and stale delivery fences fail closed.
Manual retry is exposed as the Manage-admitted envelope-aware
`agent.purge.reconcile` operation with command-ID deduplication and immutable
audit history.

### 11.4 Root-fork summary

The remaining open/partial findings collapse into eight architecture forks:

1. **Canonical SDK versus EasyNet provider:** A01, A07-A09, A13, A16,
   A19, A31-A32, A46-A50, A57, A82, A85.
2. **Axon proof model versus executable legacy paths:** A03, A06,
   A77-A79.
3. **LocalRuntime versus direct daemon execution:** A04-A05, A21, A36-A37.
4. **Canonical terminality versus transport projection:** A41, A45,
   A68-A71, A75, A90.
5. **Agent aggregate versus shared files/direct persistence:** A18, A20,
   A29-A30, A39, A65, A88.
6. **Hub-owned voice versus process-local feature state:** A73, A86, A89.
7. **Public compatibility versus completed migration:** A25-A26, A48-A49,
   A82-A83, A85.
8. **Syntactic gates versus executable proof:** A50, A57.

### 11.5 Redundant and obsolete executable surfaces

| Surface | State | Required deletion/convergence |
|---|---|---|
| Plain invocation sign/verify/admission APIs beside descriptor-bound APIs | Open | Migrate remaining callers, remove public re-exports and delete the plain implementation. |
| Thirty-one exact daemon dispatch routes beside `LocalRuntime` dispatch | Open | Register them as Axon abilities and delete direct mutation/response synthesis. |
| Mission daemon-socket self-loop beside Axon child capability | Transitional | Inject daemon policy dispatcher into the prepared child seam, migrate Mission/EAL, then delete the self-loop. |
| Canonical SDK product route tables beside provider manifests | Open | Move every EasyNet route literal into provider packages generated from one manifest. |
| `Daemon*` canonical names beside neutral `Runtime*` names | Open | Keep only SPEC-required source aliases in a downstream adapter; remove them from canonical concept inventories. |
| Go/Python handwritten C ABI JSON/status/receipt projections | Open | Generate projections from one schema and retain only language lifetime wrappers. |
| `agents/` and `workspaces/` production directory roots | Open | Perform one explicit per-agent migration and delete all legacy readers/writers/tests. |
| Direct `AgentRegistry` load/save across catalog/lifecycle/MCP | Open | Put mutation and snapshots behind one aggregate repository, migrate callers, delete procedural access. |
| Unsigned `InvocationReceipt` constructor/wire path | Open | Make unsigned receipt construction private and expose only signed/verifiable boundary types. |
| ID-only `LocalRuntime` controls beside generation handles | Open | Require generation/control capability everywhere and remove raw ID methods. |
| Process-local receipt signer beside daemon key custody | Fixed in production | Production boot injects persistent owner-bound signing and resolver-visible keys; local generated signing exists only behind explicit test/local-fast seams. |
| Voice process-local call store beside realm-shared Hub ownership | Open | Introduce one durable realm aggregate/state machine and remove per-process authoritative state. |
| Purge sorted-head outbox retry | Removed | Per-entry finite FSM, dead-letter state, epoch-fenced bounded drain, and authorized reconciliation replace the obsolete sorted-head retry path. |
| Axon Python MCP/orchestrator/audio/tool facades | Open | Move product APIs downstream, migrate consumers and delete the Axon exports. |

### 11.6 Final verification and commit decision

Passed in the final re-audit:

- Architecture convergence gate and its negative fixture suite.
- Canonical public API, parity-matrix and product-neutrality gates. Section 11's
  A80 caveat is historical after Sections 20-21; A81/A83/A84 are historical
  after Section 22. A82 remains only as SPEC-permitted non-canonical alias
  cutover debt.
- CLI Agent command module: **51 passed** after aligning its injected Device
  authority with the fixture's paired identity.
- Go SDK tests, Python SDK tests/Ruff and focused Axon/client/Dendrite/runtime
  suites reported by the independent reviewers.
- CodeGraph sync for both repositories and `git diff --check`.

Failed or incomplete:

- Voice real-invoke slice: **1 passed, 9 failed** because the tests/runtime
  path still build a Device-only catalog after the Hub-owner cutover (A73).
- Remote bidi carrier admission-frame test: **failed** because the admission
  receipt is encoded in the terminal field (A75).
- Repository-wide CLI suite was not re-declared green; independent full runs
  observed the failures above and additional integration fallout.
- The passing SDK gates remain structurally insufficient for branch-wide
  convergence due to remaining product/provider boundary leaks and
  major-version alias cutover debt. The self-attested evidence defect A80 is
  superseded by Sections 20-21; A81/A83/A84 are superseded by Section 22.

The working tree is therefore **not stable and no commit was created**. A
commit would incorrectly package known production-path regressions and would
violate the requested architecture-convergence policy.

## 12. Post-reaudit addendum: A75 narrow convergence slice

This addendum records the follow-up audit after applying the stricter
production-infrastructure standard. It does not supersede the broad findings
above; it narrows one previously failing transport-projection defect and keeps
the remaining architecture forks open.

### 12.1 Scope

Goal restatement: converge the signed bidi receipt projection path without
inventing an EasyNet-specific SDK behavior, preserving public wire behavior
while separating legacy transport projection from canonical verifier
projection.

The concrete slice was the Axon Dendrite signed bidi receive/decode path in:

- `/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Axon/core/runtime-rs/dendrite-bridge/src/invoke_signed_bidi.rs`
- `/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Cli/pr/20260714-bidi-receipt-projection/00-plan.md`
- `/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Cli/pr/20260714-bidi-receipt-projection/02-verification.md`

### 12.2 Updated findings

| ID | Finding | Branch-relative status | Current state | Evidence | Required convergence |
|---|---|---:|---|---|---|
| A75-N | Signed bidi receipt projection encoded admission receipts through the terminal receipt slot and mixed legacy transport JSON with verifier JSON. | New on this branch | Fixed for the narrow Dendrite decode/recv slice | `decode_down_payload` now projects legacy `receipt` separately from verifier-shaped `admission_receipt` and `terminal_receipt`; focused decode and recv-path tests pass. | Keep the split as the canonical projection rule and migrate any remaining downstream consumers to the verifier-specific fields. |
| A75-B | Terminality is still not globally canonical across all transport surfaces. | New on this branch | Partial | The narrow signed bidi projection is fixed, but broader stream/unary/SDK projections were not proven by this slice. | Define one terminality/projection contract and run it through all generated SDK conformance cases. |
| C07 split | Receipt trust-state projection was split between verified internal chain state and exported unverified Dendrite JSON. | Main-existing root, branch-new DTO exposure | Partially fixed by Section 25 under C07/A71, not A76 signer custody | Dendrite signed unary/common and bidi now export typed receipt trust-state objects while preserving legacy status strings. | Complete SDK DTO adoption and resolver-verified receipt projection so consumers can distinguish `resolver_unverified` from `resolver_verified` without reading ad hoc strings. |
| A77 | Public verifier/error conformance is still under-specified around missing or invalid proof material. | New on this branch | Partial/Open | A focused rejection behavior exists, but the expected public error contract and generated conformance matrix are not complete. | Specify verifier failure taxonomy once and generate language tests from it. |
| A78 | Runtime control was exposed as raw invocation IDs across public SDK surfaces. | New on this branch | Partial after Section 15 | Go/Python/Node/Java/Swift now use opaque control capabilities and observation-only public JSON; Rust/C ABI provider controls remain product/registry tokens. | Keep raw IDs private to transport/provider internals, then close C ABI/provider lifecycle with generation-bound runtime ownership. |
| A80-A84 | SDK convergence gates still prove syntax more than architecture. | New on this branch | Partial after Sections 20-22 | A80's self-attested evidence gap is fixed by live execution/hash/tree/nonce proof. A81's stale scan path, A83's generic/provider spec ambiguity and A84's CI omission are fixed in the current worktree. A82 remains as SPEC-permitted non-canonical alias debt until major-version cutover. | Keep executable public-surface and behavior checks across the same capability matrix, then remove REQ-LANG-5 aliases at the declared major cutover. |
| A86/A89 | Voice/call ownership still has product/process-local state incompatible with the shared runtime model. | New on this branch | Open | Earlier real-invoke slice still failed 9 voice tests after Hub-owner cutover. | Move voice/call lifecycle into one durable realm aggregate or downstream product repository; remove process-local authority. |

### 12.3 Redundancy and obsolete-code delta

| Surface | Status after this iteration | Decision |
|---|---|---|
| Legacy bidi `receipt` JSON | Kept only as the old transport projection | Public wire compatibility is preserved, but verifier semantics moved to explicit receipt fields. |
| Verifier receipt projection | Now separated into admission/terminal fields in the signed bidi slice | This becomes the convergence target for other receipt surfaces. |
| Raw invocation control IDs | Still redundant with lifecycle-capability control | Next root refactor target; remove public numeric control as an architecture concept. |
| Product-specific SDK/provider naming | Still present | Not touched in this slice; remains an architecture defect, not a naming cleanup task. |

### 12.4 Verification

Passed:

- `cargo fmt --manifest-path /Users/macbook.silan.tech/Documents/GitHub/EasyNet-Axon/core/runtime-rs/dendrite-bridge/Cargo.toml -- /Users/macbook.silan.tech/Documents/GitHub/EasyNet-Axon/core/runtime-rs/dendrite-bridge/src/invoke_signed_bidi.rs`
- `cargo test --manifest-path /Users/macbook.silan.tech/Documents/GitHub/EasyNet-Axon/core/runtime-rs/dendrite-bridge/Cargo.toml decode_down_payload_projects_admission_receipt_field --lib`
- `cargo test --manifest-path /Users/macbook.silan.tech/Documents/GitHub/EasyNet-Axon/core/runtime-rs/dendrite-bridge/Cargo.toml decode_down_payload_projects_terminal_receipt_field --lib`
- `cargo test --manifest-path /Users/macbook.silan.tech/Documents/GitHub/EasyNet-Axon/core/runtime-rs/dendrite-bridge/Cargo.toml recv_signed_projects_receipt_fields_after_chain_verification --lib`
- `cargo test --manifest-path /Users/macbook.silan.tech/Documents/GitHub/EasyNet-Axon/core/runtime-rs/dendrite-bridge/Cargo.toml bidi_receipt_classifier_distinguishes_admission_terminal_and_invalid --lib`
- `git -C /Users/macbook.silan.tech/Documents/GitHub/EasyNet-Axon diff --check -- core/runtime-rs/dendrite-bridge/src/invoke_signed_bidi.rs`
- `git -C /Users/macbook.silan.tech/Documents/GitHub/EasyNet-Cli diff --check -- pr/20260714-bidi-receipt-projection/00-plan.md pr/20260714-bidi-receipt-projection/02-verification.md`

Not claimed:

- Repository-wide CLI green status.
- Repository-wide Axon green status.
- Full SDK matrix convergence.
- Complete receipt-trust-boundary closure.

### 12.5 Iteration report

1. Goal restatement: architecture convergence, with the first completed slice
   limited to signed bidi receipt projection correctness.
2. Remaining work: raw control capability, verified receipt export,
   SDK-product neutrality, single capability matrix execution and voice/call
   ownership remain open.
3. Architectural decisions made: legacy transport receipt JSON and verifier
   proof JSON are distinct projections; admission and terminal receipts are
   separate state-machine outputs.
4. Refactoring completed: signed bidi decode/recv projection now routes receipt
   kinds through explicit classifier output instead of terminal-field reuse.
5. Newly implemented capabilities: focused signed bidi projection tests,
   including a recv-path test that exercises raw stream receive, protobuf
   decode, structural validation, sequence/MAC verification and projection.
6. Technical debt removed: the admission-as-terminal projection bug is removed
   in the signed bidi Dendrite slice.
7. SPEC conformance: improved for URA/runtime-neutral receipt projection; not
   yet conformant for raw control IDs, product-neutral SDK shape or full
   capability matrix execution.
8. Self-evaluation: Architecture 7/10 for this narrow slice and 4/10 for the
   whole branch; Code Quality 7/10 for the slice and 5/10 branch-wide; Product
   Consistency 6/10 for the slice and 3/10 branch-wide; SPEC Conformance 6/10
   for the slice and 3/10 branch-wide.

The working tree is still not stable enough to commit. A commit is deferred
until the next root architecture repair has executable verification and does
not package known failing production paths.

## 13. Post-reaudit addendum: A78 invocation control capability fork

### 13.1 Root abstraction problem

The submitted invocation lifecycle has a real state machine:

`Submitted -> Running -> CancelRequested -> Completed | Cancelled | Failed`

but the canonical SDK surface still models control as a raw numeric
`handle_id`. That numeric value is a process-local registry index, not a
runtime capability. Exposing it through public SDK DTOs and transport seams
turns a local implementation detail into the architecture's control authority.

The correct abstraction is an opaque, generation-bound
`InvocationControlCapability` owned by the submitted invocation handle. The
runtime may internally store a numeric registry id, but SDK consumers and
language transports should operate on the capability object, not on the raw
integer.

### 13.2 Cross-language evidence

| Surface | Evidence | Architecture defect | Branch-relative status |
|---|---|---|---|
| Rust C ABI | `/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Cli/src/ffi/invocation/mod.rs` exports `pub type InvocationHandleId = u64` and public `runtime_invocation_handle_await/cancel/events/free(handle, invocation_handle_id, ...)`. | FFI treats process-local registry id as the public control credential. | New on this branch |
| Go SDK transport | `/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Cli/sdk/go/runtime.go` declares `AwaitHandle(ctx, handleID uint64)`, `CancelHandle(ctx, handleID uint64, ...)`, `HandleEvents(ctx, handleID uint64)`, `FreeHandle(ctx, handleID uint64)`. | The canonical transport seam is ID-oriented instead of capability-oriented. | New on this branch |
| Go SDK model | `/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Cli/sdk/go/runtime.go` exposes `InvocationHandle.HandleID() uint64` and `InvocationCancel.HandleID() uint64`. | Public SDK consumers can extract and replay raw control ids. | New on this branch |
| Python SDK transport | `/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Cli/sdk/python/easynet_sdk/runtime.py` defines `await_handle(handle_id: int)`, `cancel_handle(handle_id: int, ...)`, `handle_events(handle_id: int)`, `free_handle(handle_id: int)`. | Python repeats the Go raw-ID architecture rather than converging on a shared capability model. | New on this branch |
| Python SDK model | `/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Cli/sdk/python/easynet_sdk/runtime.py` stores `InvocationHandle.handle_id` and routes control through that field. | The supposedly opaque handle leaks the control primitive. | New on this branch |
| Node SDK transport | `/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Cli/sdk/node/index.d.ts` exposes `awaitHandle(handleId: number)`, `cancelHandle(handleId: number, ...)`, `handleEvents(handleId: number)`, `freeHandle(handleId: number)`. | TypeScript public API bakes in numeric control. | New on this branch |
| Node SDK model | `/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Cli/sdk/node/index.js` stores `InvocationHandle.handleId` and passes it to transport methods. | JS object is not an opaque lifecycle handle; it is a DTO wrapper around a raw id. | New on this branch |
| Java SDK model | `/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Cli/sdk/java/src/main/java/run/easynet/daemon/InvocationHandle.java` exposes `public long handleId()`. | Java lacks the same control behavior as Go/Python/Node but still leaks the raw control id. | New on this branch |
| Swift SDK model | `/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Cli/sdk/swift/Sources/EasyNetDaemonSDK/Invocation.swift` exposes `public let handleId: Int64`. | Swift lacks the same control behavior as Go/Python/Node and still exposes raw id as model state. | New on this branch |
| Capability matrix | Java/Swift only submit and expose handles, while Go/Python/Node implement await/cancel/events/free. | The seven SDKs are not converging as implementations of one runtime model. | New on this branch |

CodeGraph recheck: `codegraph explore RuntimeTransport --max-files 20`
confirmed the same structural split. It found Go `RuntimeTransport` with
`AwaitHandle/CancelHandle/HandleEvents/FreeHandle(handleID uint64)`, Python
`RuntimeTransport` with `handle_id: int`, Node `RuntimeTransport` with
`handleId: number`, Java `RuntimeTransport` without handle-control methods and
Swift `RuntimeTransport` without handle-control methods. The index reported
925 files / 32,021 nodes / 117,416 edges, with pending changes still present.

### 13.3 Required convergence

| Step | Required change | Why this is root-cause, not patching |
|---|---|---|
| 1 | Introduce a language-neutral `InvocationControlCapability` concept in the canonical SDK matrix. | The shared architecture needs a named lifecycle-control abstraction before language implementations can converge. |
| 2 | Change public SDK client methods to accept `InvocationHandle` or its private capability, never a numeric id. | Consumers should not be able to construct or replay control ids. |
| 3 | Move numeric `handle_id` into private transport adapters and C ABI binding internals. | The number remains an implementation detail for the current daemon registry but stops defining the SDK architecture. |
| 4 | Add generation/incarnation material to the capability, or require the daemon to return an opaque token with owner/runtime binding. | Prevents stale handle ABA and cross-runtime replay. |
| 5 | Implement await/cancel/events/free parity for Java and Swift or mark them `Unsupported` in the shared matrix. | Avoids silent language-specific architecture drift. |
| 6 | Generate conformance tests that attempt raw-id replay, cross-runtime replay and post-free replay. | The gate must prove capability semantics, not merely scan names. |
| 7 | Remove public `HandleID`/`handle_id`/`handleId` accessors unless a SPEC explicitly requires source compatibility. | Compatibility aliases would preserve the defective architecture unless SPEC-owned. |

### 13.4 Current decision

This audit does not apply the A78 refactor yet. The blast radius crosses Rust
FFI, Go, Python, Node, Java, Swift and the SDK conformance matrix. A local
change in only one language would make the architecture less convergent.

The next implementation iteration should be a grouped refactor with one
capability contract and generated fixtures, not isolated method renames.

### 13.5 Iteration report

1. Goal restatement: identify the full root fork behind raw invocation control
   ids and avoid treating it as a local SDK bug.
2. Remaining work: implement the cross-language capability model and update the
   conformance matrix; no A78 code repair has been claimed.
3. Architectural decisions made: raw `handle_id` is an implementation detail,
   not a canonical SDK concept.
4. Refactoring completed: none in this A78 iteration; only diagnosis was added.
5. Newly implemented capabilities: none.
6. Technical debt removed: none in code; diagnostic debt reduced by making the
   cross-language fork explicit.
7. SPEC conformance: currently non-conformant with explicit lifecycle state
   machine and shared runtime model requirements.
8. Self-evaluation: Architecture 8/10 for diagnosis, 3/10 current
   implementation; Code Quality 6/10 diagnosis, 4/10 implementation; Product
   Consistency 4/10; SPEC Conformance 3/10.

## 14. Post-reaudit addendum: A78 control capability implementation slice

### 14.1 Implemented convergence

| Language | Change | State |
|---|---|---|
| Go | `RuntimeTransport` await/cancel/events/free now accept `InvocationControlCapability`; C ABI/direct adapters extract the private registry id internally. | Seam |
| Python | `RuntimeTransport`, C ABI adapter, direct adapter, runtime event provider and retained-handle cleanup now use `InvocationControlCapability`. | Seam |
| Node | Runtime transport type declarations and runtime client use `InvocationControlCapability`; public handle objects no longer expose `handleId`; cancel projection accepts `request_accepted` and `deduplicated`. | Seam |
| Java | Still exposes `handleId()` and lacks await/cancel/events/free parity. | Unsupported/Partial |
| Swift | Still exposes `handleId` and lacks await/cancel/events/free parity. | Unsupported/Partial |
| C ABI | Still exposes numeric invocation handle ids as C opaque handles. | Adapter-private target not yet achieved |

### 14.2 Verification

Passed:

- `(cd /Users/macbook.silan.tech/Documents/GitHub/EasyNet-Cli/sdk/go && go test .)`
- `PYTHONPATH=sdk/python python -m pytest sdk/python/tests/test_runtime.py sdk/python/tests/test_runtime_events.py sdk/python/tests/test_ability_invocation.py -q`
- `(cd /Users/macbook.silan.tech/Documents/GitHub/EasyNet-Cli/sdk/node && npm test && npm run typecheck --if-present)`

### 14.3 Remaining rejection conditions

Queen must still reject full A78 completion while any of these remain:

- Java/Swift lack submitted invocation await/cancel/events/free parity.
- C ABI and JSON documentation still present numeric submitted handles as
  control authority instead of adapter-private handles.
- Submitted lifecycle remains owned by FFI registry rather than daemon/runtime
  lifecycle state.
- Product-neutrality defects D1-D10 remain in canonical SDK public API.
- Receipt trust-state vocabulary remains inconsistent across Dendrite, CLI and
  SDK DTOs.

### 14.4 Iteration report

1. Goal restatement: converge submitted-invocation control around explicit
   lifecycle capabilities.
2. Remaining work: Java/Swift/C ABI/runtime-core lifecycle, SDK product
   neutrality and receipt trust-state convergence.
3. Architectural decisions made: Go/Python/Node canonical SDK transports no
   longer accept naked raw ids.
4. Refactoring completed: Go/Python/Node control seam refactor and focused test
   migration.
5. Newly implemented capabilities: `InvocationControlCapability`.
6. Technical debt removed: raw numeric control is no longer the canonical
   transport seam in Go/Python/Node.
7. SPEC conformance: partial; the shared runtime model improved but the full
   seven-language matrix is not converged.
8. Self-evaluation: Architecture 6/10 for this slice, 4/10 branch-wide; Code
   Quality 7/10; Product Consistency 4/10; SPEC Conformance 4/10.

## 15. Post-reaudit addendum: CodeGraph-backed current matrix

This section supersedes the Java/Swift state in Section 14. It reflects the
working tree after the Java/Swift A78 repair, the public-DTO provenance repair
and the latest `codegraph sync .`.

CodeGraph status for EasyNet-Cli after sync:

- 927 indexed files.
- 32,061 symbols.
- 117,696 graph edges.
- `InvocationControlCapability` found in Go, Python, Node, Java and Swift.
- SDK product-symbol query still finds `Daemon`, `Easynet`, `EasyNet`,
  `Device`, and `Hub` symbols under `sdk/`.
- After regenerating the public API inventory,
  `sdk_concepts --validate-actual` now passes. The previous
  `canonical_inventory_product_leak:go:languages:ControlDiscovery` diagnosis
  is stale after the inventory/model repair.
- SDK non-URA locator query does not find a real hand-written identity architecture surface:
  hits are generated protobuf `SecurityClass` names and a `during` test-name
  substring. URA/locator naming is therefore not the current primary SDK hand-written
  defect.

### 15.1 Current architecture-break matrix

| ID | Root abstraction problem | CodeGraph/local evidence | Branch-relative status | Current state | Required convergence |
|---|---|---|---|---|---|
| C01 | Canonical SDK public identity is still EasyNet/daemon-shaped, not product-neutral runtime-shaped. | `codegraph query EasyNet` and SDK product-symbol SQL find `sdk/go/daemon.go`, `sdk/python/easynet_sdk/daemon.py`, Java package `run.easynet.daemon`, C ABI `Easynet*` aliases and provider packages. | Branch-new SDK surface over main-existing product ABI debt | Open | Split neutral runtime SDK from EasyNet provider packages. Product names remain only in downstream provider/distribution layers. |
| C02 | A78 raw submitted-control ids were promoted into the SDK runtime model. | Historical Section 13 evidence; current CodeGraph now finds `InvocationControlCapability` across five languages after repair. | Branch-new | Closed for Go/Python/Node/Java/Swift SDK seams; C ABI/provider lifecycle remains open | Keep numeric ids adapter-private; move C ABI/process-local lifecycle behind provider binding and add replay/generation tests. |
| C03 | C ABI still uses EasyNet product names and raw submitted handle ids as public provider tokens. | `include/easynet_cli.h` contains `RuntimeInvocationHandleId`; Rust FFI exports `runtime_invocation_handle_await/cancel/events/free`. | Main-existing ABI name, branch-new SDK canonicalization pressure | Open | Treat C ABI as EasyNet provider ABI, not canonical SDK model. A neutral ABI needs an explicit SPEC and version bump. |
| C04 | Submitted lifecycle ownership is still provider registry-local rather than daemon/runtime aggregate-owned. | SDK adapters extract adapter ids from capabilities; FFI registry remains the owner that awaits/cancels/events/frees. | Branch-new exposure over main-existing FFI behavior | Open | Runtime aggregate owns submitted lifecycle state; provider registry only stores transport resources. |
| C05 | SDK product-neutrality gates are too shallow. | Product-neutrality script passes, but CodeGraph still counts product symbols led by `sdk/go/daemon.go`, `_cabi.py`, `direct_runtime.go`, `ura.go`, Java package names and provider packages. | Branch-new | Open | Gate canonical SDK exports by owner class: generic runtime vs EasyNet provider vs generated Axon protobuf. |
| C06 | Go/Python/Node/Java/Swift A78 seam is now aligned, but the seven-language matrix still has provider/ABI and product-neutrality gaps. | CodeGraph finds five language implementations; regenerated public API inventory and `sdk_concepts --validate-actual` now pass for this slice; C ABI/Rust provider remains raw and product-named. | Branch-new | Improved but incomplete | Keep generated public inventory current and require every capability row to declare state, owner and provider evidence. |
| C07 | Receipt trust semantics remain split between frame authentication, structural receipt shape and resolver-verified receipt chain. | Dendrite signed receipt JSON now carries typed `receipt_trust_state` and top-level `*_receipt_trust_state`, but CLI still reports ledger/chain status separately from local verification and SDK DTOs remain mostly string/object projections. | Main-existing root, branch-new DTO exposure | Partial by Section 25 | Use one typed trust-state enum across Dendrite, CLI and generated SDK DTOs with explicit `unverified`, `frame_authenticated`, `ledger_reported`, `resolver_verified` states. |
| C08 | Exact daemon routes still bypass canonical LocalRuntime finalization in multiple mutation paths. | Section 3 A21 evidence remains the architecture owner issue; no full route cutover was completed in this A78 slice. | Main-existing route fork, worktree receipt hardening exposes it | Open | Move every mutating exact route through LocalRuntime and delete parallel direct-dispatch receipt/finalization paths. |
| C09 | Product-specific runtime events are certified as runtime-core concepts. | `sdk/go/provider/easynet/runtime_events.go` and Python provider imports exist; previous matrix labelled `runtime_events` too strongly before downgrade. | Branch-new | Partially corrected by provider split, still product-owned | Keep runtime event core generic; EasyNet event topics and ability names live in provider packages and cannot be `cutover-ready` canonical runtime. |
| C10 | Go/Python SDKs still contain EasyNet directory, device, hub and daemon lifecycle facades. | CodeGraph product-symbol counts: `sdk/go/daemon.go` 27, `sdk/python/easynet_sdk/daemon.py` 15, `sdk/go/ura.go` 12, `sdk/python/easynet_sdk/axon_addressing.py` 11. | Branch-new | Open | Move process lifecycle, `.easynet` paths, device/hub URA builders and receipt read models downstream. |
| C11 | Java/Swift package identity is product-specific despite A78 lifecycle parity. | Every Java source is under namespace `run.easynet.daemon`; Swift package remains `EasyNetDaemonSDK`. | Branch-new | Open | Rename/package under neutral runtime SDK or keep them as EasyNet provider SDKs while introducing neutral canonical packages. |
| C12 | Python exposes weaker encapsulation for capability construction by language nature. | `InvocationControlCapability` is no longer root-exported, but remains importable from `easynet_sdk.runtime` with dataclass construction. | Branch-new | Mitigated, not closed | Use module-private factory conventions plus negative tests; if stronger opacity is required, use a private class and protocol surface. |
| C13 | Node type surface hides construction in JS but still exposes an empty public interface. | `sdk/node/index.d.ts` exports `interface InvocationControlCapability {}` while implementation class is not exported. | Branch-new | Acceptable seam, needs gate | Keep only type-level exposure for transport implementers; add tests that JS consumers cannot construct/fromHandle raw ids. |
| C14 | Public API compatibility has been preserved for JSON wire `handle_id`, but internal model changed. | Go/Python/Node/Java/Swift still decode wire `handle_id` into capability. | Branch-new | Intentional adapter seam | Keep wire compatibility only as DTO parsing; do not reintroduce raw-id accessors. |
| C17 | Public DTO handle construction previously forged lifecycle authority from caller-supplied `handle_id`. | Go/Python/Node/Java/Swift public JSON decoders now produce observation snapshots only; trusted submit/internal paths bind runtime control; cancel/events reject mismatched returned handle ids. | Branch-new | Corrected for language SDKs | Keep public JSON compatibility as DTO observation only. Do not reintroduce public constructors or factories that can mint runtime-bound control. |
| C15 | Redundant hand-written provider lowering remains in multiple SDK languages. | Product-symbol query hits provider packages and tests; prior audit lists Go/Python handwritten provider ability tables. | Branch-new | Open | Generate provider lowering from one EasyNet provider manifest and remove duplicate tables. |
| C16 | URA naming constraint is mostly clean in SDK hand-written symbols, but generated/product terms still need owner classification. | CodeGraph non-URA locator query under `sdk/` returns generated `SecurityClass` and substring false positives, not hand-written identity abstractions. | Not a current SDK hand-written defect | No immediate rename required | Keep URA-only in domain names; allow locator naming only for real HTTP/tonic transport locators outside runtime identity. |

### 15.2 Redundant code and non-conforming features

| Cluster | Classification | Current status | Required action |
|---|---|---|---|
| `Daemon*` aliases and daemon lifecycle facades in SDK | Branch-new | Still present | Move to EasyNet provider or downstream facade; neutral SDK keeps runtime lifecycle only. |
| EasyNet C ABI names and `Easynet*` typedefs | Main-existing ABI debt, branch-new canonicalization pressure | Still present | Treat as provider ABI; do not list as canonical runtime API. |
| Device/Hub URA builders and runtime admin device revoke | Branch-new | Still present | Move to EasyNet topology/provider package. |
| Java `run.easynet.daemon` and Swift `EasyNetDaemonSDK` package identity | Branch-new | Still present | Rename or demote to provider SDK once neutral package exists. |
| Go/Python provider event lowering | Branch-new | Still present | Generate from a provider manifest and keep out of runtime core. |
| Raw submitted handle id in JSON/C ABI | Branch-new SDK wire detail over provider ABI | Language SDK JSON is observation-only; C ABI now mints opaque JSON-safe provider tokens instead of sequential ids | Keep JSON as DTO observation only; treat C ABI id as provider binding until a neutral ABI SPEC/version bump exists. Runtime-owned lifecycle still remains open. |
| Java optional cancel fields | Branch-new A78 implementation defect | Corrected | `request_accepted` and `deduplicated` are now required; missing-field negative test added. |
| Receipt trust projection strings/objects | Main-existing root, branch-new SDK DTO exposure | Still present | Replace with typed trust-state vocabulary and resolver-backed verification path. |
| Exact daemon route direct dispatch | Main-existing | Still present | Cut over to LocalRuntime, delete direct mutation/finalization fork. |

### 15.3 Current verification

Passed after this addendum:

- `(cd sdk/go && go test .)`
- `PYTHONPATH=sdk/python python -m pytest sdk/python/tests/test_runtime.py sdk/python/tests/test_runtime_events.py sdk/python/tests/test_ability_invocation.py -q`
- `(cd sdk/node && npm test && npm run typecheck --if-present)`
- `(cd sdk/java && mvn test && java -cp target/classes:target/test-classes run.easynet.daemon.RuntimeCoreSeamTest --list | while IFS= read -r s; do java -cp target/classes:target/test-classes run.easynet.daemon.RuntimeCoreSeamTest "$s"; done)`
- `(cd sdk/swift && swift test)`
- `git diff --check` for the A78 implementation files and documents.
- `python sdk/conformance/sdk_concepts.py --validate-schema`
- `python sdk/conformance/sdk_concepts.py --self-test --tmp /tmp/easynet_sdk_concepts_self_tmp`
- `python sdk/conformance/sdk_concepts.py --validate-actual`
- `bash tools/scripts/check-sdk-product-neutrality.sh`
- `codegraph sync .`
- `codegraph query InvocationControlCapability --path . -l 50`
- CodeGraph SDK product-symbol and non-URA locator SQL queries against
  `.codegraph/codegraph.db`.

Known failing verification:

- None in the focused A78 language-SDK slice. Branch-wide architecture
  convergence is still blocked by the open C ABI/provider lifecycle,
  product-specific SDK identity/provider concepts and receipt-trust findings
  listed above.

Queen follow-up:

- Accepted: the A78 forged public DTO/runtime-control authority blocker is
  closed for Go/Python/Node/Java/Swift.
- No public trusted factory leak was found in the reviewed public API surface.
- Remaining production blockers are outside this A78 language-SDK slice: C ABI
  raw handle authority/lifecycle, provider lifecycle ownership and
  non-verifying receipt DTO projections.

### 15.4 Iteration report

1. Goal restatement: re-run the architecture audit under the production-grade
   convergence standard and avoid reporting only the repaired A78 slice.
2. Remaining work: product-neutral SDK extraction, C ABI/provider boundary,
   submitted lifecycle ownership, receipt trust vocabulary, exact-route
   LocalRuntime cutover and capability-matrix closure.
3. Architectural decisions made: public JSON DTOs are observation snapshots,
   not lifecycle authority; runtime-bound control is produced only by trusted
   submit/internal paths.
4. Refactoring completed: Java/Swift A78 parity, Node cancel schema tightening,
   Java cancel schema tightening, Python root export narrowing, public DTO
   provenance repair across five SDKs and CodeGraph-backed diagnostic refresh.
5. Newly implemented capabilities: Java/Swift submitted invocation await,
   cancel, events and close-handle seams.
6. Technical debt removed: public raw submitted handle access and public DTO
   authority construction are no longer the SDK control model in
   Go/Python/Node/Java/Swift.
7. SPEC conformance: A78 is closed for the five language SDK control/DTO seam.
   SDK design principles still fail branch-wide while canonical SDK surfaces
   contain EasyNet/daemon/device/hub/provider concepts and C ABI provider
   lifecycle ownership remains product/process-local.
8. Self-evaluation: Architecture 8/10 for the A78 language-SDK slice, 5/10
   branch-wide; Code Quality 8/10 for focused changes; Product Consistency
   5/10; SPEC Conformance 6/10.

## 16. Post-reaudit addendum: C ABI provider-token hardening

This section supersedes the C ABI raw-id portion of Section 15. It does not
claim runtime-owned lifecycle completion.

### 16.1 Implemented convergence

| Area | Previous defect | Current state | Remaining gap |
|---|---|---|---|
| C ABI submitted handle allocation | `RuntimeInvocationHandleId` values were predictable monotonic registry ids. | Rust FFI now mints collision-checked OsRng provider tokens in `[2^52, 2^53 - 1]`, preserving `uint64_t` ABI and exact JSON numeric representation. | Token freshness remains provider-local; runtime-owned generation/session incarnation is still open. |
| Post-free lifecycle control | `free` accepted unknown handles as OK, preserving replay-compatible semantics. | Post-free `await`, `cancel`, `events` and repeated `free` now return `ERR_INVALID_HANDLE`. | Runtime aggregate still does not own submitted lifecycle truth. |
| Public-surface product quarantine | CamelCase `Device`/`Hub` and `CABI` replacement classification had false negatives/weak mapping. | Token-aware policy quarantines `ModeDevice`, `ModeHub`, `RuntimeModeHub`, `RuntimeAdminAbilityClient.RevokeDevice`; `ParsedURA.DeviceID` remains allowed URA grammar; C ABI maps to `native_runtime`. | Physical package/product naming extraction remains open. |

### 16.2 Verification

Passed:

- `cargo test --all-features invocation_handle --lib`
- `rustfmt --edition 2021 --check src/ffi/invocation/mod.rs`
- `python sdk/conformance/rebuild_public_api_model.py --write`
- `python sdk/conformance/sdk_concepts.py --validate-schema`
- `python sdk/conformance/sdk_concepts.py --self-test --tmp /tmp/easynet_sdk_concepts_self_tmp`
- `python sdk/conformance/sdk_concepts.py --validate-actual`
- `bash tools/scripts/check-sdk-product-neutrality.sh`
- `python -m py_compile sdk/conformance/sdk_public_surface_policy.py sdk/conformance/rebuild_public_api_model.py sdk/conformance/sdk_concepts.py`
- `git diff --check` over the touched FFI/conformance files.
- `npx -y @colbymchenry/codegraph sync .`

Known verification caveat:

- Full `cargo fmt --all -- --check` is currently blocked by unrelated
  formatting diffs in receipt finalization files. The touched FFI file passes
  direct `rustfmt --check`.

### 16.3 Queen result

Queen initially rejected high-bit `u64` provider tokens because `handle_id` is
also public JSON and would exceed Node exact-number safety and Java/Swift signed
integer ranges. After constraining the token range to `[2^52, 2^53 - 1]`, Queen
accepted the provider-token hardening slice.

Remaining production blockers:

- Runtime-owned invocation lifecycle and deterministic terminal ownership.
- Session/generation/incarnation binding beyond process-local probabilistic
  token freshness.
- Canonical receipt trust and terminal receipt verification as production
  authority.
- Broader runtime-owned cancellation/finalization semantics across unary,
  stream and bidi.

### 16.4 Iteration report

1. Goal restatement: continue architecture convergence by removing predictable
   C ABI submitted-handle authority without breaking stable ABI or JSON
   behavior.
2. Remaining work: runtime-owned lifecycle, session/generation incarnation,
   receipt trust, cross-geometry finalization and product-neutral SDK
   extraction.
3. Architectural decisions made: C ABI handle ids are provider tokens, not
   canonical runtime concepts; token values must be opaque and JSON-safe.
4. Refactoring completed: random JSON-safe provider token minting, strict
   post-free rejection and token-aware product quarantine.
5. Newly implemented capabilities: provider-token hardening and quarantine
   self-test coverage.
6. Technical debt removed: predictable submitted-handle ids, idempotent
   post-free replay and CamelCase Device/Hub canonical inventory leaks.
7. SPEC conformance: improved for provider boundary safety and SDK canonical
   inventory separation; not complete for runtime-owned lifecycle or receipt
   trust.
8. Self-evaluation: Architecture 8/10 for this slice, 5/10 branch-wide; Code
   Quality 8/10; Product Consistency 6/10; SPEC Conformance 6/10.

## 17. Post-reaudit addendum: FFI client-session binding

This section supersedes the session-incarnation remaining gap in Section 16 for
the EasyNet-Cli provider ABI layer only. It does not claim runtime-owned
invocation lifecycle completion.

### 17.1 Implemented convergence

| Area | Previous defect | Current state | Remaining gap |
|---|---|---|---|
| FFI client-session identity | Invocation resources were owned by naked `RuntimeHandle` numeric equality. | `ClientSession` now mints a private incarnation and exposes internal `ClientSessionBinding { handle, incarnation }`. | Binding is process-local provider identity, not runtime aggregate lifecycle identity. |
| Submitted invocation resource ownership | Submitted handles could only prove they belonged to a numeric handle value. | Submitted handle registry stores `ClientSessionBinding`; stale same-handle/different-incarnation access is rejected. | Await/events still read FFI projection state rather than a runtime-owned aggregate. |
| Stream/bidi resource ownership | Stream and bidi registries used a parallel naked-handle owner check. | Server-stream and bidi registries now share the same `ClientSessionBinding` owner model. | Stream/bidi terminal closure still needs the canonical receipt gate and runtime-owned finalization. |
| Shutdown cleanup | Shutdown cleanup matched resources by naked handle. | Cleanup first transitions the session `Active -> Closing`, drains resources by exact `ClientSessionBinding`, releases the handle and marks the session Released. | Crash/restart recovery and daemon-side lifecycle query remain outside this provider-local registry. |
| Open/shutdown race | A live check followed by resource insertion could race with shutdown. | Submit/open hold a `ClientSessionResourceGuard` until child registry insertion completes; shutdown uses the same lifecycle mutex before drain. | Long-term runtime lifecycle should make FFI handles observers of daemon-owned state. |
| v5 public JSON behavior | Public JSON behavior changes were implicit in current diffs. | REQ-ABI-5..9 now declare provider child-resource binding, post-free invalid handle behavior, cancel-request JSON, `terminal_receipt` and stream/bidi receipt-backed terminal authority. | Release-gate conformance still needs explicit cases for REQ-ABI-6/7. |

### 17.2 Verification

Passed:

- `cargo test --all-features invocation_handle --lib`
- `cargo test --all-features handle::tests --lib`
- `cargo test --all-features invocation_stream --lib`
- `cargo test --all-features invocation_bidi --lib`
- `cargo test --all-features cancel_invocations_for_handle --lib`
- `rustfmt --edition 2021 --check src/ffi/client/handle.rs src/ffi/invocation/mod.rs`
- `git diff --check -- src/ffi/client/handle.rs src/ffi/invocation/mod.rs src/ffi/mod.rs docs/spec/daemon-sdk-requirements-v1.md docs/reviews/architecture-convergence-audit-2026-07-14.md`
- `npx -y @colbymchenry/codegraph sync .`

Known verification caveat:

- `cargo test --all-features registry --lib` is too broad for this slice and
  currently runs unrelated daemon ability registry tests. The FFI registry tests
  in that run passed; unrelated failures remain in admission action, proof
  descriptor hash and catalog authority tests.

### 17.3 Queen result

Accepted after one rejection/fix cycle.

Initial rejection:

- Lifecycle validation and child-resource insertion were not atomic, leaving a
  shutdown/open zombie-resource race.
- Public JSON behavior changes needed an explicit v6 ABI decision rather than
  an implicit drift from earlier slices.

Corrected:

- `ClientSession` now owns an explicit `Active -> Closing -> Released`
  lifecycle state.
- Submit/open paths hold `ClientSessionResourceGuard` through child registry
  insertion; shutdown calls `begin_closing`, drains by exact binding, releases
  the handle and marks Released.
- REQ-ABI-5..9 now document provider child-resource binding and v5 public JSON
  semantics.

Remaining production blockers:

- Runtime-owned submitted lifecycle is still not canonical; the C ABI still
  owns local submitted-handle truth.
- Stream/bidi local cancel still needs lifecycle-control cancellation with
  receipt-backed terminal finalization.
- Release-gate conformance still needs explicit REQ-ABI-6/7 cases for stale
  same-handle replay, post-free replay, cross-session submitted-handle control
  and required cancel JSON fields.

### 17.4 Iteration report

1. Goal restatement: continue architecture convergence by binding FFI
   invocation resources to one live client-session incarnation without changing
   public C ABI.
2. Remaining work: runtime-owned lifecycle, canonical receipt verification
   gate, deterministic terminal ownership, cross-geometry finalization and
   product-neutral SDK extraction.
3. Architectural decisions made: `RuntimeHandle` remains public ABI; child
   resources store internal `ClientSessionBinding`; session incarnation is
   provider-local authority hardening, not canonical runtime lifecycle truth.
4. Refactoring completed: `ClientSession` incarnation minting, binding lookup,
   explicit lifecycle state, guarded child-resource registration,
   submitted/stream/bidi registry owner migration and shutdown cleanup by live
   binding after Closing.
5. Newly implemented capabilities: stale-session-incarnation rejection for FFI
   submitted handles and identity-aware owner checks across invocation resource
   registries.
6. Technical debt removed: provider invocation resource ownership no longer
   depends only on naked numeric handle equality.
7. SPEC conformance: improved for provider lifecycle replay isolation and
   boundary clarity; still incomplete for runtime-owned lifecycle and receipt
   trust.
8. Self-evaluation: Architecture 8/10 for this slice, 5/10 branch-wide; Code
   Quality 8/10; Product Consistency 6/10; SPEC Conformance 6/10.

## 18. Post-reaudit addendum: submitted-handle release gate

This section closes the Section 17 release-gate gap for REQ-ABI-6/7 only. It
does not claim runtime-owned invocation lifecycle completion or receipt-backed
terminal authority.

### 18.1 Implemented convergence

| Area | Previous defect | Current state | Remaining gap |
|---|---|---|---|
| REQ-ABI-6 conformance | Stale/cross-session/post-free submitted-handle authority was tested locally but not bound into release conformance. | `invocation/submitted_handle_authority` is a C ABI quality-gate case bound to `invocation_handle_provider_authority_conformance`. | Full live parity still needs other language result artifacts for the current branch state. |
| Cross-session submitted-handle control | The release artifact did not prove the whole public submitted-handle operation surface rejected a different session. | The selector asserts cross-session `await`, `cancel`, `events` and `free` all return `ERR_INVALID_HANDLE` and clear output pointers where applicable. | Runtime-owned lifecycle should ultimately make these provider handles observers of daemon aggregate state. |
| Post-free replay | Earlier coverage sampled post-free replay but did not cover every public submitted-handle operation. | The selector asserts post-free `await`, `cancel`, `events` and repeated `free` all return `ERR_INVALID_HANDLE`. | Crash/restart and daemon-side lifecycle query remain out of scope for this provider-local registry. |
| REQ-ABI-7 cancel projection | Cancel JSON shape was documented but did not have a dedicated release artifact. | The selector asserts `request_accepted`, `deduplicated`, `cancelled`, `state` and `terminal`, with `CancelRequested` non-terminal. | Receipt-backed terminal finalization remains a separate REQ-ABI-8/9 blocker. |
| Evidence chain | Tightened selector initially left stale adapter evidence. | `c-abi-action-adapter-report.json` now pins current `src/ffi/invocation/mod.rs` hash `0f7b0c075ca108aa48c47b2186518e8d5cfa367f8a8c32ad0a317b2d2a0ee10c`; C ABI runner passes. | Keep evidence hashes synchronized whenever selector source changes. |

### 18.2 Verification

Passed:

- `cargo test --features axon-pb --lib invocation_handle_provider_authority_conformance`
- `rustfmt --edition 2021 --check src/ffi/invocation/mod.rs src/ffi/client/handle.rs src/ffi/mod.rs`
- `git diff --check -- src/ffi/invocation/mod.rs src/ffi/client/handle.rs src/ffi/mod.rs sdk/conformance/cases/invocation-submitted-handle-authority.yaml sdk/conformance/canonical-public-api.json sdk/conformance/runner/execution-manifest.json sdk/conformance/runner/c-abi-action-adapter-report.json docs/spec/daemon-sdk-requirements-v1.md`
- `SDK_CONFORMANCE_LANGUAGES=c_abi bash tools/scripts/check-sdk-conformance-reports.sh`
- `bash tools/scripts/check-sdk-scaffold.sh`
- `python3 sdk/conformance/sdk_concepts.py --validate-schema`
- `npx -y @colbymchenry/codegraph sync .`

Known verification caveat:

- Full parity with `target/sdk-conformance-live-results` currently reports
  `missing_live_results:rust,go,python,node,java,swift`; this iteration
  intentionally generated only the C ABI live result.

### 18.3 Queen result

Accepted after two rejection/fix cycles.

Initial rejection:

- The first review observed stale C ABI adapter evidence after selector
  tightening.
- The second review observed that the default
  `target/sdk-conformance-live-results/c_abi.json` artifact was absent, so the
  current disk state did not prove the live conformance result.

Corrected:

- The adapter report now pins the current invocation FFI source hash.
- The C ABI conformance runner passed after the hash update and generated a
  live `invocation/submitted_handle_authority` result with selector
  `invocation_handle_provider_authority_conformance` collected exactly once.
- The default live-result artifact was regenerated and verified at
  `target/sdk-conformance-live-results/c_abi.json`.

### 18.4 Iteration report

1. Goal restatement: close the release-gate proof for submitted C ABI
   invocation-handle authority and cancel request lifecycle.
2. Remaining work: runtime-owned lifecycle, canonical receipt verification,
   deterministic terminal ownership, cross-geometry finalization and
   seven-language live parity.
3. Architectural decisions made: REQ-ABI-6/7 belong in one submitted-handle
   authority quality gate; the SDK conformance chain is the correct enforcement
   surface; provider C ABI naming is not canonical SDK model naming.
4. Refactoring completed: new conformance case, execution-manifest binding,
   adapter evidence record, quality-gate registration and expanded public C ABI
   selector coverage.
5. Newly implemented capabilities: release-conformance proof for stale
   incarnation rejection, cross-session invalid-handle rejection, post-free
   invalid-handle rejection and non-terminal `CancelRequested` JSON.
6. Technical debt removed: REQ-ABI-6/7 behavior is no longer dependent on
   unaffiliated local tests or prose-only spec statements.
7. SPEC conformance: improved for REQ-ABI-6/7; incomplete for REQ-ABI-8/9 and
   runtime aggregate ownership.
8. Self-evaluation: Architecture 8/10 for this slice, 5/10 branch-wide; Code
   Quality 8/10; Product Consistency 7/10; SPEC Conformance 7/10.

## 19. Post-reaudit addendum: direct runtime provider naming

This section closes the narrow SDK/provider naming fork identified in the
DirectDaemon/LocalRuntimeEndpoint rows. It does not claim branch-wide SDK
product neutrality, because EasyNet package identity, provider ABI naming,
daemon lifecycle facades and product route catalogs remain open defects in
Sections 15-18.

### 19.1 Implemented convergence

| Area | Previous defect | Current state | Remaining gap |
|---|---|---|---|
| Direct runtime provider surface | Go/Python exposed `DirectDaemon*` names in the SDK provider seam. | Go/Python now expose `DirectRuntimeConnector`, `DirectRuntimeTransport`, direct stream/bidi transports and `direct_runtime_provider` conformance identity. | The surrounding SDK packages are still EasyNet-branded provider/facade packages rather than a neutral canonical SDK distribution. |
| Runtime endpoint path resolver | Go exposed `LocalRuntimeEndpoint*` naming for a generic runtime UDS path resolver. | Public resolver is now `RuntimeEndpointPathOptions` and `ResolveRuntimeEndpointPath`. | Product defaults and daemon lifecycle discovery still belong in the provider layer. |
| Bidi receipt projection | Direct runtime bidi JSON could write a receipt into the legacy `receipt` slot instead of verifier-shaped admission/terminal fields. | Go direct bidi projection now emits `admission_receipt` for non-terminal receipts and `terminal_receipt` for terminal receipts. | Broader terminality and receipt trust-state projection are still open under C07/A70/A71. |
| Public-surface policy residue | The product-neutrality quarantine rule still carried the removed `DirectDaemon` spelling. | The rule now matches generic `Daemon` product ownership without retaining the migrated symbol. | Package and ABI product names still require owner-classification gates. |

### 19.2 CodeGraph and local evidence

- `codegraph sync .` completed with 930 indexed files, 32,249 nodes and
  121,709 edges. CodeGraph still reports one removed-file residual pending
  after sync.
- `codegraph query DirectRuntime` resolves the current Go and Python direct
  runtime provider symbols; `codegraph query RuntimeEndpointPath` resolves the
  Go endpoint path resolver.
- `rg DirectDaemon|directDaemon|LocalRuntimeEndpoint|ResolveLocalRuntimeEndpointPath|easynet_direct_daemon`
  over `sdk/go`, `sdk/python` and `sdk/conformance` has no matches.

### 19.3 Verification

Passed:

- `bash tools/scripts/check-sdk-canonical-public-api.sh`
- `bash tools/scripts/check-sdk-product-neutrality.sh`
- `bash tools/scripts/check-sdk-parity-matrix.sh --self-test`
- `bash tools/scripts/check-architecture-convergence.sh`
- `python3 sdk/conformance/sdk_concepts.py --validate-schema`
- `python3 sdk/conformance/sdk_concepts.py --self-test --tmp target/sdk-concepts-self-test`
- `PYTHONPATH=sdk/python python3 -m pytest sdk/python/tests/test_direct_runtime.py sdk/python/tests/test_import_boundary.py -q`
- `(cd sdk/go && go test -tags easynet_direct_runtime ./...)`
- `bash tools/scripts/check-sdk-conformance-reports.sh`

Known verification caveat:

- Full live parity was rerun with all seven language result artifacts present,
  but failed with `sdk_parity_matrix: replayed_tree_attestation:rust:ability/descriptor_projection`
  because other working-tree edits landed during report generation. Recent
  writes were observed under `src/daemon/ability/...`, outside this SDK naming
  slice. No commit was created.

### 19.4 Iteration report

1. Goal restatement: use CodeGraph-backed diagnosis to continue architecture
   convergence, remove branch-new product naming from the direct runtime SDK
   provider seam, and verify without treating provider names as canonical
   runtime concepts.
2. Remaining work: branch-wide SDK product identity, EasyNet C ABI/provider
   naming, runtime-owned submitted lifecycle, receipt trust verification,
   exact-route LocalRuntime cutover, and stable seven-language live parity.
3. Architectural decisions made: the direct gRPC/UDS provider is a generic
   runtime provider seam, not a daemon-owned canonical abstraction; endpoint
   path resolution is generic runtime configuration, not product lifecycle.
4. Refactoring completed: DirectDaemon names were replaced with DirectRuntime
   names; LocalRuntimeEndpoint names were replaced with RuntimeEndpointPath
   names; conformance provider identity moved to `direct_runtime_provider`;
   bidi receipt projection now uses admission/terminal receipt slots.
5. Newly implemented capabilities: no new product capability was added; the
   implemented change is public-surface convergence and receipt projection
   correctness for the direct runtime provider.
6. Technical debt removed: removed stale DirectDaemon naming from SDK code,
   tests, conformance inventory and policy; removed a legacy receipt-slot
   projection path in direct bidi JSON.
7. SPEC conformance: improved for SDK product-neutrality and URA-only naming;
   incomplete for the broader canonical SDK/product-provider split.
8. Self-evaluation: Architecture 8/10 for this slice, 5/10 branch-wide; Code
   Quality 8/10; Product Consistency 7/10; SPEC Conformance 7/10.

## 20. Post-reaudit addendum: deterministic seven-language live parity

This section closes the live parity evidence fork observed after Section 19.
The defect was not a missing SDK capability; it was that seven-language
conformance evidence could be generated from different effective source trees
when language build sidecars or nested report runs changed the working tree
during collection.

### 20.1 Implemented convergence

| Area | Previous defect | Current state | Remaining gap |
|---|---|---|---|
| Live source attestation | A single run could contain multiple `tree_sha256` values, causing `replayed_tree_attestation` and making the matrix unsuitable as release evidence. | The report script checks that generated language results share one tree attestation before accepting the run. | Release scripts should consume an explicit results directory generated after source edits settle. |
| Run identity | Nested conformance execution could emit result files with independent side effects while the parent run was collecting reports. | The report script issues one run nonce before language iteration and checks that generated results share it. | CI should eventually publish the nonce and source tree as build metadata outside the attested source files. |
| Language sidecars | Local cache/build directories from Python, Java and Swift changed tree attestation even though they are not source architecture. | SDK conformance sidecars are ignored as local execution residue. | A future source-attestation helper should centralize the ignored-artifact policy instead of duplicating it across scripts. |
| Go adapter evidence | Go report evidence pointed at a stale `sdk/go/runtime_test.go` hash after concurrent branch edits. | Go adapter evidence now pins the current source hash and passes report validation. | Other adapters still need owner-classification review as product/provider boundaries are cut over. |

### 20.2 Live evidence policy

- Exact live-result directories, source-tree attestations and run nonces are
  build artifacts, not source records. Writing them into this source-covered
  audit file would change the attested tree and invalidate the proof.
- Final verification must therefore run after source edits settle and consume
  an explicit `target/sdk-conformance-live-results.*` directory outside the
  source tree.
- The expected passed-record distribution is C ABI 18, Go 38, Python 36, Node
  17, Rust 17, Java 14 and Swift 14.

### 20.3 Verification

Passed:

- `bash -n tools/scripts/check-sdk-conformance-reports.sh`
- `python3 -m json.tool sdk/conformance/runner/go-action-adapter-report.json`
- `git diff --check -- .gitignore tools/scripts/check-sdk-conformance-reports.sh sdk/conformance/runner/go-action-adapter-report.json`
- `SDK_CONFORMANCE_LANGUAGES=node bash tools/scripts/check-sdk-conformance-reports.sh`
- `EASYNET_SDK_PARITY_RESULTS_DIR="$PWD/target/sdk-conformance-live-results.*" bash tools/scripts/check-sdk-parity-matrix.sh`

Known verification caveat:

- `target/sdk-conformance-live-results` may contain older local artifacts. The
  accepted evidence for this slice must be an isolated live directory generated
  after source edits settle.

### 20.4 Iteration report

1. Goal restatement: make live seven-language SDK parity deterministic enough
   to support architecture convergence decisions.
2. Remaining work: SDK product identity, provider ABI naming, runtime-owned
   submitted lifecycle, receipt trust verification, exact-route LocalRuntime
   cutover and owner-classification gates.
3. Architectural decisions made: a release-quality matrix must have one source
   tree and one run nonce; language build residue is not an architecture input;
   nested checks must write to isolated live-result directories.
4. Refactoring completed: conformance sidecar ignores, single-nonce report
   flow, explicit language report iteration, bounded per-language failure
   handling, mixed-nonce/mixed-tree guards and refreshed Go evidence hash.
5. Newly implemented capabilities: seven-language live parity can now be
   accepted as a single runtime capability matrix for the attested source tree.
6. Technical debt removed: removed nondeterministic replay failures caused by
   unignored cache/build residue and nested runner writes.
7. SPEC conformance: improved for single shared runtime model and capability
   matrix discipline; still incomplete for product/provider separation.
8. Self-evaluation: Architecture 8/10 for this slice, 5/10 branch-wide; Code
   Quality 8/10; Product Consistency 7/10; SPEC Conformance 7/10.

## 21. Post-reaudit addendum: seven-language report proof closure

This section supersedes only the Section 19 verification caveat. It does not
claim branch-wide architecture convergence. The branch still has product/provider
and lifecycle defects recorded in Sections 15-19.

### 21.1 Implemented convergence

| Area | Previous defect | Current state | Remaining gap |
|---|---|---|---|
| Live report isolation | Report wrapper self-tests and nested runner probes could write into the caller's live-result directory, producing mixed run nonces. | Nested report executions now write to a temporary `nested-live-results` directory; the caller result set is checked for single-nonce and single-tree evidence. | Long-running report generation still depends on a quiet source tree for replayed tree attestation. |
| Matrix evidence closure | Passed manifest cases with evidence but no language public-surface item were modeled as unsupported, causing `unmodeled_passed_case` for ABI version discovery. | `sdk_matrix.py` treats evidence-backed cases as `seam` even without a public inventory item, preserving unsupported only for no-surface/no-evidence cases. | The matrix still reflects current branch capabilities, not final SDK convergence. |
| Runner attestation | A single failed diagnostic record prevented run context binding for all records in the language report. | The runner binds run context to non-failed emitted records before output; failed records remain diagnostics. | Runtime aggregate ownership and terminal receipt trust remain separate architectural blockers. |
| Go report evidence | `sdk/go/runtime_test.go` changed while the Go adapter report kept old hashes. | `go-action-adapter-report.json` now pins the current runtime test hash. | Evidence hashes must remain synchronized with future selector/source changes. |
| Canonical public API model | Current dirty Go lifecycle surface added `StartConfig.WorkingDir`, leaving generated public API inventory stale. | Canonical public API and parity matrix were regenerated against the current working tree; `StartConfig.WorkingDir` is modeled under `runtime_lifecycle`. | This records current API shape; it does not endorse daemon/product lifecycle as final canonical SDK design. |

### 21.2 CodeGraph and live evidence

- `codegraph sync . && codegraph status` completed with 930 indexed files,
  32,255 nodes and 121,605 edges. CodeGraph still reports one removed-file
  residual pending after sync.
- Seven-language live report proof must use a unique result directory under
  `target/sdk-conformance-live-results.*` generated after source edits settle.
- Parity must pass using that same result directory:
  `sdk parity matrix ok: sdk/conformance/sdk-parity-matrix.json`.

### 21.3 Verification

Passed:

- `bash tools/scripts/check-sdk-canonical-public-api.sh`
- `bash tools/scripts/check-sdk-product-neutrality.sh`
- `bash tools/scripts/check-sdk-parity-matrix.sh --self-test`
- `bash tools/scripts/check-architecture-convergence.sh`
- `python3 sdk/conformance/sdk_concepts.py --validate-schema`
- `python3 sdk/conformance/sdk_concepts.py --self-test --tmp target/sdk-concepts-self-test`
- `python3 -m py_compile sdk/conformance/sdk_matrix.py sdk/conformance/rebuild_public_api_model.py sdk/conformance/sdk_concepts.py sdk/conformance/sdk_public_surface_policy.py`
- `bash tools/scripts/check-sdk-conformance-reports.sh --self-test`
- `env CARGO_TARGET_DIR=target/sdk-conformance-runner-test cargo test -p sdk-conformance-runner`
- `PYTHONPATH=sdk/python python3 -m pytest sdk/python/tests/test_direct_runtime.py sdk/python/tests/test_import_boundary.py -q`
- `(cd sdk/go && go test -tags easynet_direct_runtime ./...)`
- `bash tools/scripts/check-sdk-conformance-reports.sh`
- `bash tools/scripts/check-sdk-parity-matrix.sh` with the matching live-result directory

Commit status:

- No commit was created. The worktree contains a large concurrent dirty set
  spanning daemon lifecycle, C ABI, provider routing, docs, descriptors and
  generated SDK inventory. A clean, attributable commit slice cannot be
  established without staging unrelated changes or splitting the branch first.

### 21.4 Iteration report

1. Goal restatement: finish the CodeGraph-backed architecture re-audit by
   closing direct runtime provider naming, repairing the conformance proof
   chain and producing a seven-language live parity result for the current tree.
2. Remaining work: branch-wide product/provider split, SDK package identity,
   runtime-owned lifecycle aggregate, receipt trust/finalization, LocalRuntime
   exact-route cutover and stale compatibility aliases.
3. Architectural decisions made: live conformance results are proofs over a
   tree/nonce/toolchain context; nested probes must be isolated; evidence-backed
   manifest cases are seams even without public API inventory; failed runner
   records are diagnostics, not live proof records.
4. Refactoring completed: report wrapper isolation, matrix evidence-state
   closure, runner run-context binding, direct runtime provider naming and Go
   report evidence refresh.
5. Newly implemented capabilities: no product capability was added; the new
   capability is reliable seven-language conformance proof replay for the
   current SDK matrix.
6. Technical debt removed: mixed-nonce result contamination, unsupported
   modeling of passed evidence-backed cases, stale Go report hash and failed
   record proof stripping.
7. SPEC conformance: improved for SDK canonical-runtime modeling and proof
   replay; incomplete for branch-wide product-neutral SDK architecture.
8. Self-evaluation: Architecture 8/10 for this slice, 5/10 branch-wide; Code
   Quality 8/10; Product Consistency 7/10; SPEC Conformance 7/10.

## 22. Post-reaudit addendum: SDK gate closure A81-A84

This section updates the SDK gate findings after the live parity proof work. It
does not claim branch-wide SDK/product convergence: provider ABI naming,
runtime-owned lifecycle and receipt trust remain separate architecture forks.

### 22.1 Implemented convergence

| Finding | Previous defect | Current state | Remaining gap |
|---|---|---|---|
| A81 | Product-neutrality scanned stale or empty Go roots and missed the active runtime-events core. | `canonical-public-api.json` declares `sdk/go/runtimeevents` as provider-neutral core; product-neutrality reads roots from the manifest, validates them and scans them recursively. | None for A81. |
| A82 | Conformance could claim product aliases were rejected while daemon-named aliases remained public. | Canonical graph now rejects daemon-named exports, including Python snake_case `start_daemon`/`attach_daemon`/`discover_daemon`. Existing REQ-LANG-5 source aliases are tracked only under `non_canonical` with cutover metadata. | SPEC-permitted aliases remain until an explicit major-version cutover removes them. |
| A83 | The SDK spec called the ABI generic while specifying EasyNet daemon lifecycle. | The spec now distinguishes provider-neutral canonical runtime concepts from the EasyNet provider C ABI; "generic" at the provider boundary means operation-family generic, not provider-neutral. | None for A83. |
| A84 | SDK tests, Ruff/typing and conformance gates were absent from CI. | `.github/workflows/tests.yml` installs pinned language toolchains and runs SDK tests, Ruff, strict mypy, public API/product-neutrality gates, live conformance/parity and exact C ABI export checks. | This proves workflow coverage shape, not that every integration body is always runnable in CI. |

### 22.2 Verification

Passed:

- `bash tools/scripts/check-sdk-product-neutrality.sh`
- `bash tools/scripts/check-sdk-product-neutrality.sh --self-test`
- `python sdk/conformance/rebuild_public_api_model.py --write`
- `bash tools/scripts/check-sdk-canonical-public-api.sh`
- `python3 sdk/conformance/sdk_concepts.py --validate-schema`
- `bash tools/scripts/check-sdk-parity-matrix.sh --self-test`
- `bash tools/scripts/check-architecture-convergence.sh`

Manual inventory check:

- No canonical `languages` or `members` entry contains `Daemon` or a
  daemon-named symbol after regeneration.
- Python daemon-named compatibility functions are now present only in
  `non_canonical.languages.python`.

### 22.3 Iteration report

1. Goal restatement: close the stale SDK gate findings without deleting
   SPEC-required source compatibility or treating provider ABI names as
   canonical runtime concepts.
2. Remaining work: REQ-LANG-5 alias removal at the declared major-version
   cutover, runtime-owned lifecycle, receipt trust verification and broader
   product/provider extraction.
3. Architectural decisions made: A82 is not an immediate public API deletion
   while REQ-LANG-5/REQ-PROD-5 require compatibility; it is a non-canonical
   quarantine/cutover contract. Snake_case daemon names must be treated the same
   as CamelCase `Daemon*` names.
4. Refactoring completed: `canonical_quarantine_reason` now detects daemon as a
   semantic token, regenerated public API inventory/parity data and updated the
   audit status for A81-A84.
5. Newly implemented capabilities: the public API gate now fails if a
   daemon-named symbol re-enters the canonical graph, including snake_case
   Python aliases.
6. Technical debt removed: removed the false canonical classification for
   `start_daemon`, `attach_daemon` and `discover_daemon`.
7. SPEC conformance: improved for REQ-OBJ-5, REQ-LANG-5, REQ-PROD-5 and
   URA-only/product-neutral SDK naming discipline.
8. Self-evaluation: Architecture 8/10 for this gate slice, 5/10 branch-wide;
   Code Quality 8/10; Product Consistency 7/10; SPEC Conformance 8/10.

## 23. Post-reaudit addendum: forwarded receipt authenticity A67

This section updates A67 after re-reading the current forwarded finalization
implementation and running the targeted receipt tests. The old diagnosis was
accurate for the shape-only verifier path, but stale for the current worktree:
forwarded finalization now delegates receipt canonicalization and cryptographic
verification to Axon.

### 23.1 Implemented convergence

| Area | Previous defect | Current state | Remaining gap |
|---|---|---|---|
| Wire receipt authenticity | Forwarded receipt verification checked field shape/nonempty bytes instead of recomputing `self_hash`, resolving the signer and verifying Ed25519. | `finalization_projection::verify_wire_checkpoint` decodes wire receipts with Axon, then calls `SignedInvocationReceipt::verify(resolver)`. `verify_wire_finalization_checkpoints` verifies admission and terminal checkpoints through Axon `FinalizationCheckpointVerifier`. | SDK/Dendrite trust-state projection is still C07/A71; this slice proves verifier usage when a resolver is available. |
| Forwarded finalization trust boundary | Remote stream/bidi/unary finalization could project receipts before a canonical proof boundary. | `ForwardedFinalizationVerifier` consumes untrusted wire checkpoints and exposes trusted projections only after Axon verification succeeds. Invalid cryptographic proof fails closed before projection and is surfaced as forwarded-finalization precondition failure. | SDK receipt DTOs still need structured proof preservation under A71. |
| Causal anchor projection test | The test assumed terminal receipt index `1`, implicitly restoring the invalid adjacent-checkpoint model from A40. | The test now derives `anchor_count` and receipt URA suffix from the actual terminal receipt index, while still requiring the verified projection as the only causal anchor source. | Full-chain proof is still not inferred from two checkpoints; that remains the intended boundary. |

### 23.2 Verification

Passed:

- `cargo test --features axon-pb --lib verified_finalization_projection_is_the_only_causal_anchor_source -- --nocapture`
- `cargo test --features axon-pb --lib forwarded_finalization -- --nocapture`
- `cargo test --features axon-pb --lib finalization_projection -- --nocapture`

Covered behaviors:

- Unsigned or invalid wire failure duplicates cannot enter the trusted domain.
- Public forwarded verifier rejects receipts without valid cryptographic proof.
- Non-adjacent admission/terminal checkpoints remain valid when Axon verifies
  the finalization pair.
- The local daemon projection records a causal anchor only from a verified
  finalization projection.

### 23.3 Iteration report

1. Goal restatement: close the stale A67 receipt-authenticity diagnosis without
   overclaiming broader receipt lifecycle convergence.
2. Remaining work: A71 structured SDK receipt projection, C07 typed
   trust-state projection, A70 transport-terminal separation and broader
   runtime-owned finalization remain open.
3. Architectural decisions made: daemon forwarded finalization is an adapter
   around Axon proof semantics; it must not own field-shape receipt trust rules.
   Terminal checkpoint adjacency is not a protocol invariant.
4. Refactoring completed: removed the stale test assumption that terminal
   receipts must be `/receipt/1`; updated audit status so A67 is fixed and the
   remaining receipt defects stay assigned to their real owner findings.
5. Newly implemented capabilities: no product capability was added. The
   executable evidence now enforces resolver-backed forwarded receipt proof in
   the targeted test suite.
6. Technical debt removed: stale A67 report contradiction and obsolete
   adjacent-checkpoint assertion in the local daemon projection test.
7. SPEC conformance: improved for canonical Axon proof ownership, URA-only
   receipt anchor naming and the single runtime finalization model. Incomplete
   for SDK/Dendrite trust-state projection fidelity.
8. Self-evaluation: Architecture 8/10 for A67 closure, 5/10 branch-wide; Code
   Quality 8/10; Product Consistency 7/10; SPEC Conformance 7/10.

## 24. Post-reaudit addendum: production receipt signer custody A76

This section updates A76 after re-reading production daemon boot, runtime
assembly and receipt-signing provider code. The old diagnosis that production
`LocalRuntime::new()` generated process-local per-callee receipt keys is stale.
Production boot now installs owner-bound signing providers backed by the daemon
key-service capability, and the resolver overlay exposes the same runtime's
receipt signer public keys for verification.

### 24.1 Implemented convergence

| Area | Previous defect | Current state | Remaining gap |
|---|---|---|---|
| Production runtime construction | Production daemon boot used `LocalRuntime::new()` and therefore had no persistent receipt signer custody. | `easynet-daemon` builds `ProductionReceiptAuthorityConfig` and calls `build_production_local_runtime`, which uses `LocalRuntime::new_with_signing_authority_providers`. | Child dispatch policy integration remains A36; signer wiring itself is closed. |
| Receipt signer custody | Receipt keys were process-local/generated and disappeared after restart. | `KeyServiceReceiptAuthorityProvider` loads owner-bound `RuntimeSigningIdentity` signers; no private key is copied into the runtime. Existing tests verify old receipt signatures remain valid after key-service restart. | Key rotation/revocation policy is separate from this custody closure. |
| Resolver-visible signer keys | Signed receipts could not be verified after restart because the verifier had no durable public key path. | `configure_local_runtime` installs `LocalSystemKeyResolver` with a weak reference to the same runtime. The resolver asks `runtime.resolve_receipt_signer_key` before falling back to the upstream trust anchor. | Dendrite/SDK exported trust-state still needs typed projection under C07/A71. |
| Explicit local-fast seam | The process-local signer was indistinguishable from production construction. | Axon `LocalRuntime::new()` is fail-closed for receipt signing; generated local signing is exposed only through `new_local_fast` or CLI `build_local_runtime` test/probe seams. | Keep all production boot paths on `build_production_local_runtime`. |

### 24.2 Verification

Passed:

- `bash tests/scripts/test_check_daemon_key_service_boundary.sh`
- `cargo test --features axon-pb --lib receipt_signing -- --nocapture`
- `cargo test --features axon-pb --lib runtime_factory -- --nocapture`

Subagent re-review:

- A76 production signer custody/publication should be closed.
- The residual `verification_status = "unverified_resolver_required"` claim is
  real in the Dendrite bridge, but belongs under C07/A71 trust-state
  projection, not A76 signer custody.

### 24.3 Iteration report

1. Goal restatement: correct the stale A76 diagnosis and separate production
   signer custody from receipt trust-state projection.
2. Remaining work: C07 typed trust-state projection, A71 structured SDK receipt
   projection, A70 transport-terminal separation and A36 child policy
   integration remain open.
3. Architectural decisions made: production runtime construction must be
   owner-bound and fail-closed; local-fast generated signing is only a named
   test/probe seam. Dendrite resolver-unverified JSON is a projection-state
   issue, not signer custody.
4. Refactoring completed: audit rows now close A76, update A37 signer wiring
   status and move residual Dendrite trust-state debt to C07/A71.
5. Newly implemented capabilities: no product capability was added. The report
   now reflects existing executable evidence for owner-bound receipt signer
   custody.
6. Technical debt removed: stale `LocalRuntime::new()` production signer
   diagnosis and duplicated A76/C07 ownership.
7. SPEC conformance: improved for explicit production signer ownership,
   fail-closed runtime construction and single-runtime verification wiring.
   Incomplete for exported receipt trust-state fidelity.
8. Self-evaluation: Architecture 8/10 for A76 closure, 5/10 branch-wide; Code
   Quality 8/10; Product Consistency 7/10; SPEC Conformance 7/10.

## 25. Post-reaudit addendum: Dendrite receipt trust-state C07

This section updates C07 after the Dendrite bridge projection repair in
`EasyNet-Axon`. The prior defect was not that Dendrite could produce a
resolver-verified receipt without a resolver; it was that exported JSON exposed
receipt objects and legacy string statuses without a typed trust-state object.
Consumers could therefore treat "receipt JSON present" as stronger proof than
the bridge actually had.

### 25.1 Implemented convergence

| Area | Previous defect | Current state | Remaining gap |
|---|---|---|---|
| Shared verifier receipt JSON | Each Dendrite path could hand-add status metadata around verifier JSON. | `receipt_to_verifier_json` now emits `receipt_trust_state` with `state = resolver_unverified`, `verified = false`, `receipt_present = true`, `required_verifier = key_resolver`, plus the legacy `verification_status` string for public compatibility. | Add resolver-backed `resolver_verified` projection once the bridge receives a verifier/key resolver. |
| Signed unary/common response shape | Top-level `admission_receipt_status` and `terminal_receipt_status` were string-only lifecycle facts. | The signed common path now also emits `admission_receipt_trust_state` and `terminal_receipt_trust_state` objects, and reserves those names so domain payloads cannot shadow them. | Generated SDK DTOs still need typed fields instead of loose maps/strings. |
| Signed bidi projection | Bidi manually set `verification_status = unverified_resolver_required` after calling the shared encoder. | Bidi now reuses the shared verifier JSON directly, so admission and terminal receipt chunks have the same typed receipt trust-state as unary/common. | Full stream/bidi terminality and SDK DTO adoption remain C07/A70/A71 work. |

### 25.2 Verification

Passed in `EasyNet-Axon`:

- `cargo test --manifest-path core/runtime-rs/client-sdk/Cargo.toml receipt_verifier_json -- --nocapture`
- `cargo test --manifest-path core/runtime-rs/dendrite-bridge/Cargo.toml signed_receipt_lifecycle -- --nocapture`
- `cargo test --manifest-path core/runtime-rs/dendrite-bridge/Cargo.toml verifier_json -- --nocapture`
- `cargo test --manifest-path core/runtime-rs/dendrite-bridge/Cargo.toml signed_response_reserved_fields -- --nocapture`
- `cargo test --manifest-path core/runtime-rs/dendrite-bridge/Cargo.toml decode_down_payload_projects -- --nocapture`
- `cargo test --manifest-path core/runtime-rs/dendrite-bridge/Cargo.toml recv_signed_projects_receipt_fields_after_chain_verification -- --nocapture`
- `cargo test --manifest-path core/runtime-rs/dendrite-bridge/Cargo.toml bidi_receipt_classifier -- --nocapture`

Known unrelated failure:

- Broad `cargo test --manifest-path core/runtime-rs/dendrite-bridge/Cargo.toml bidi -- --nocapture`
  still fails in `raw_transport::tests::bidi_stream_send_times_out_when_request_buffer_stays_full`
  because the stream handle is already removed when the test expects it to
  remain open. The signed receipt projection tests in the same run passed.

### 25.3 Iteration report

1. Goal restatement: make Dendrite exported receipt trust explicit and typed
   without claiming resolver-backed verification that the bridge cannot perform.
2. Remaining work: generated SDK DTO trust-state adoption, CLI ledger/local
   verification state unification, resolver-backed `resolver_verified`
   projection and A70 transport-terminal separation.
3. Architectural decisions made: receipt JSON presence is not proof. The bridge
   must expose a typed trust-state object; legacy string status is compatibility
   metadata, not the canonical consumer contract.
4. Refactoring completed: moved Dendrite receipt trust metadata into the shared
   verifier JSON encoder, added top-level signed response trust-state objects,
   updated FFI introspection and Go binding comments.
5. Newly implemented capabilities: Dendrite signed unary/common and bidi receipt
   projections now expose typed resolver-unverified trust state.
6. Technical debt removed: per-call-site `verification_status` mutation in the
   bidi path and string-only top-level receipt lifecycle status.
7. SPEC conformance: improved for explicit receipt proof semantics and shared
   runtime model. Incomplete for full cross-language DTO convergence.
8. Self-evaluation: Architecture 8/10 for this C07 slice, 5/10 branch-wide;
   Code Quality 8/10; Product Consistency 7/10; SPEC Conformance 7/10.

## 26. Post-reaudit addendum: transport terminality and structured receipt DTOs A70/A71

This section updates A70 and A71 after the FFI/SDK runtime receipt projection
repair. The architectural goal was convergence, not feature addition: transport
closure and runtime terminality are separate lifecycle states, and receipt DTOs
must preserve signed causal/authority facts instead of reducing them to kind
strings.

### 26.1 Implemented convergence

| Area | Previous defect | Current state | Remaining gap |
|---|---|---|---|
| FFI stream/bidi status errors | gRPC status errors were emitted as stream events with `terminal=true` but no terminal receipt. | `stream_status_error_json` now emits `terminal=false` and `transport_terminal=true`; lifecycle terminality remains receipt-backed. | Audit any remaining transport-only cancel/close projections outside this status/backpressure slice. |
| Callback backpressure | Callback queue overflow projected a terminal runtime error without a terminal receipt. | Stream and bidi backpressure projections now use `terminal=false`, `transport_terminal=true`. | Generated SDK schemas should explicitly name transport terminality. |
| Runtime receipt DTO | `causal_binding_kind` and `authority_binding_kind` were the only typed SDK fields, losing parent refs, delegation scopes and session authority facts. | CLI `ReceiptSummary`, FFI JSON, Go direct runtime, Python direct runtime and Go/Python `RuntimeReceipt` expose structured `causal_binding` and `authority_binding` objects beside legacy kind summaries. | Replace handwritten language projections with schema-generated DTOs. |
| Descriptor-ref matrix fixture | The bridge matrix still tested an obsolete descriptor-ref shape. | `signed_verification_matrix` now uses `ability_ura@version#descriptor_hash!invoke`, matching the canonical descriptor-bound runtime model. | Broader stale descriptor-ref examples remain audit debt outside this slice. |

### 26.2 Verification

Passed:

- `cargo test --features axon-pb --lib stream_status_error_is_transport_terminal_not_runtime_terminal -- --nocapture`
- `cargo test --features axon-pb --lib bounded_callback_enqueue_reports_transport_terminal_backpressure_when_full -- --nocapture`
- `cargo test --features axon-pb --lib stream_backpressure_event_is_transport_terminal_not_runtime_terminal -- --nocapture`
- `cargo test --features axon-pb --lib bidi_backpressure_frame_is_transport_terminal_not_runtime_terminal -- --nocapture`
- `cargo test --features axon-pb --lib unary_result_projects_terminal_receipt_without_losing_admission_checkpoint -- --nocapture`
- `go test . -run 'TestStreamEvent|TestStreamNext|TestRuntimeReceipt|TestInvocationResult|TestDirectRuntime|TestBidiFrame' -count=1`
- `PYTHONPATH=sdk/python python -m pytest sdk/python/tests/test_stream.py sdk/python/tests/test_bidi.py sdk/python/tests/test_runtime.py sdk/python/tests/test_direct_runtime.py -q`
- `cargo test --features axon-pb --lib verified_finalization_projection_is_the_only_causal_anchor_source -- --nocapture`
- `cargo test --manifest-path core/runtime-rs/client-sdk/Cargo.toml receipt_verifier_json -- --nocapture`
- `cargo test --manifest-path core/runtime-rs/client-sdk/Cargo.toml wrapper_returns_the_canonical_domain_bytes -- --nocapture`
- `cargo test --manifest-path core/runtime-rs/client-sdk/Cargo.toml signing_delegates_to_descriptor_bound_domain -- --nocapture`
- `cargo test --manifest-path core/runtime-rs/dendrite-bridge/Cargo.toml --test signed_verification_matrix -- --nocapture`

Known unrelated failure remains:

- Broad `cargo test --manifest-path core/runtime-rs/dendrite-bridge/Cargo.toml bidi -- --nocapture`
  still fails in `raw_transport::tests::bidi_stream_send_times_out_when_request_buffer_stays_full`.

### 26.3 Iteration report

1. Goal restatement: converge receipt/lifecycle projection so transport
   terminality cannot masquerade as runtime finalization, and signed
   causal/authority bindings survive SDK DTO projection.
2. Remaining work: schema-generate SDK receipt DTOs, extend typed trust-state
   DTO adoption, audit remaining cancel/close transport-only surfaces and
   resolver-backed `resolver_verified` projection.
3. Architectural decisions made: only receipt-backed finalization sets
   lifecycle `terminal=true`; transport end is `transport_terminal`. Kind fields
   are compatibility summaries, not canonical proof objects.
4. Refactoring completed: FFI status/backpressure projection now separates
   transport from lifecycle, and Rust/Go/Python runtime receipt projections
   expose structured causal and authority bindings.
5. Newly implemented capabilities: no product feature was added; the runtime
   DTO now carries canonical proof structure for SDK consumers.
6. Technical debt removed: terminal-without-receipt projection in the repaired
   FFI surfaces, and kind-only causal/authority receipt DTO projection.
7. SPEC conformance: improved for URA-only canonical runtime receipt DTOs and
   shared Go/Python projection shape. Incomplete until DTO generation replaces
   handwritten language projections.
8. Self-evaluation: Architecture 8/10 for A70/A71 slice, 6/10 branch-wide;
   Code Quality 8/10; Product Consistency 8/10; SPEC Conformance 8/10.

## 27. Post-Queen addendum: A70 SDK FSM and descriptor-ref proof closure

This section records the follow-up after Queen rejected Section 26 as too
narrow. The rejection was correct: the FFI producer emitted
`transport_terminal=true`, but Go/Python stream and bidi state machines still
treated transport-ending frames as ordinary nonterminal errors. Descriptor-ref
repair was also too narrow: matrix fixtures were updated while axiom vectors,
interop fixtures, public Dendrite docs and Rust reference pins still advertised
or consumed the obsolete short `ability_ura@version` shape.

### 27.1 Implemented convergence

| Area | Previous defect | Current state | Remaining gap |
|---|---|---|---|
| Go stream/bidi FSM | DTOs did not carry `transport_terminal`; a transport-ending frame could remain nonterminal to the SDK handle/session. | Go `StreamEvent` and `BidiFrame` decode `transport_terminal`; stream and bidi state machines move to failed transport state without setting runtime terminality. Legacy bidi `event` fallback was restored. | Apply the same transport-vs-runtime terminality audit to remaining cancel/close surfaces. |
| Python stream/bidi FSM | Python decoded only lifecycle terminality and accepted the same conflation. | Python stream and bidi DTOs decode/project `transport_terminal`; state machines fail the transport without synthesizing runtime terminal. Legacy bidi `event` fallback was restored. | Generate the DTO/FSM contract instead of maintaining handwritten parity. |
| Descriptor-bound SDK facade | `call_ability_ura_json_with_context` parsed a full descriptor ref, then rebuilt `canonical_ura@version`, dropping descriptor hash and action. | The facade canonicalizes only the ability URA portion and preserves descriptor version/hash/action in the signed target and signature bytes. | Federation convenience helpers that lack provider-backed descriptor facts are still a seam and must not invent synthetic hashes. |
| Axiom and interop fixtures | Several vectors and fixtures still used `ability_ura@version`, so conformance allowed a shape the parser rejects. | Axiom vectors, `invocation_envelope_interop`, Rust worked-example generator, Rust unit pins and Dendrite signed matrix now use `ability_ura@version#descriptor_hash!action`. Pinned hashes/signatures were regenerated from the Rust reference. | Downstream non-Rust SDK worked-example tests must consume the updated JSON vector. |
| Public Dendrite docs/errors | Signed Dendrite APIs and FFI metadata described short descriptor refs. | Error strings, FFI metadata and `AUTHENTICATED_INVOCATION.md` now require full descriptor refs and updated frozen values. | Complete provider-backed descriptor discovery for helpers instead of caller-supplied examples. |
| URA naming in axiom docs | Conformance docs used `caller.uri`, `subject.uri` and `len(uri)` wording. | Axiom README and documentation-only vectors now use `caller.ura`, `subject.ura` and `len(ura)` wording. | Continue repository-wide URA cleanup outside the touched conformance boundary. |

### 27.2 Verification

Passed:

- `cargo test --manifest-path core/runtime-rs/client-sdk/Cargo.toml --test invocation_envelope_interop -- --nocapture`
- `cargo test --manifest-path core/runtime-rs/client-sdk/Cargo.toml authenticated_call_without_host_authority_fails_closed -- --nocapture`
- `cargo test --manifest-path core/runtime-rs/client-sdk/Cargo.toml receipt_verifier_json -- --nocapture`
- `cargo test --manifest-path sdk/rust/Cargo.toml --test axiom_vectors -- --nocapture`
- `cargo test --manifest-path sdk/rust/Cargo.toml --lib ability_descriptor_ref_canonicalizes_components -- --nocapture`
- `cargo test --manifest-path sdk/rust/Cargo.toml --lib worked_example -- --nocapture`
- `cargo test --manifest-path core/runtime-rs/dendrite-bridge/Cargo.toml signed_receipt_lifecycle -- --nocapture`
- `cargo test --manifest-path core/runtime-rs/dendrite-bridge/Cargo.toml verifier_json -- --nocapture`
- `cargo test --manifest-path core/runtime-rs/dendrite-bridge/Cargo.toml signed_response_reserved_fields -- --nocapture`
- `cargo test --manifest-path core/runtime-rs/dendrite-bridge/Cargo.toml --test signed_verification_matrix -- --nocapture`
- `go test . -run 'TestStreamTransportTerminal|TestBidiTransportTerminal|TestBidiFrameAcceptsLegacyEventAlias|TestStreamEvent|TestBidiFrame|TestRuntimeReceipt|TestDirectRuntime' -count=1`
- `PYTHONPATH=sdk/python python -m pytest sdk/python/tests/test_stream.py sdk/python/tests/test_bidi.py sdk/python/tests/test_runtime.py sdk/python/tests/test_direct_runtime.py -q`
- Descriptor-ref short-shape scan and URI/uri scan over the touched Axon conformance/public-boundary files returned no matches.

Known caveats:

- Broad `cargo test --manifest-path sdk/rust/Cargo.toml --lib ...` previously
  exposed unrelated integration-test compile debt around `new_local_fast` and
  proof-binding test signatures when run without `--lib`; targeted canonical
  proof tests passed.
- Federation helper descriptor construction is intentionally fail-closed until
  provider-backed descriptor hash/action facts exist; no synthetic descriptor
  hash was introduced.

### 27.3 Iteration report

1. Goal restatement: close the Queen rejection by proving both producer and
   consumer transport-terminal semantics, and make descriptor refs full
   proof-bound facts across fixtures, docs and reference pins.
2. Remaining work: cancel/close transport-only surfaces, generated DTO/FSM
   parity, provider-backed descriptor discovery for federation helpers,
   resolver-verified receipt trust-state projection and branch-wide
   product-neutral SDK extraction.
3. Architectural decisions made: short descriptor refs are invalid at canonical
   boundaries; transport termination is a transport FSM state, not runtime
   terminality; helpers without descriptor proof facts must fail closed.
4. Refactoring completed: Go/Python stream+bidi FSMs, Axon SDK descriptor-ref
   preservation, axiom vectors, Dendrite public docs/errors and Rust reference
   pins.
5. Newly implemented capabilities: no product capability added; canonical
   proof and lifecycle semantics are now more explicit to SDK consumers.
6. Technical debt removed: short descriptor-ref conformance fixtures,
   string-only transport terminality in SDK FSMs and URI terminology in touched
   axiom docs.
7. SPEC conformance: improved for URA-only naming, descriptor-bound invocation
   and shared Go/Python transport FSM semantics. Incomplete for generated
   cross-language DTOs and full provider-backed descriptor discovery.
8. Self-evaluation: Architecture 8/10 for this closure, 6/10 branch-wide; Code
   Quality 8/10; Product Consistency 8/10; SPEC Conformance 8/10.
