# System ability contract audit

## Intent

Audit the complete system ability inventory and plugin-contributed ability
surface. Replace manual spot checks with deterministic gates that prove every
published ability has one coherent descriptor, owner, call mode, execution
binding, authority/subject contract, schema, and terminal contract. Repair all
confirmed seams in their owning layer without touching unrelated parallel
RemoteApp/media work.

## Current inventory baseline

- 189 static system ability descriptor contracts.
- 178 `cutover_ready`, 8 `seam`, and 3 `unsupported` contracts.
- 172 RPC, 11 stream, and 6 bidi static contracts.
- Plugin abilities are additional runtime contributions and must satisfy the
  same descriptor/binding invariants when installed.

The historical count 162 is not used as a hard-coded target. The filesystem
contract inventory is the source set; conformance must fail on unreviewed
addition, removal, or state drift.

## Audit axes

For every contract:

1. Identity: unique canonical name, path/name agreement, version, schema hash.
2. Ontology: canonical SystemAgent owner and Device sponsorship remain distinct.
3. Surface: call mode, exposure, dedicated surface, bidi wire kind, visibility.
4. Input/output: valid JSON Schema, required fields, receipt schema and semantics.
5. Executability: `cutover_ready` means exactly one executable binding on every
   hosting authority set; `seam`/`unsupported` must not be published executable.
6. Invocation: complete caller/callee/ability/subject/nonce/causal-context/args.
7. Authority: descriptor owner, callee, subject contract, admission action and
   hosted authority agree.
8. Lifecycle: stream/bidi paths have bounded queues, one terminal outcome, and
   preserve the first typed Runtime failure.
9. Product projection: backend/facades resolve descriptors and preserve typed
   failure/receipt facts instead of rebuilding protocol semantics.

## Execution order

1. Inventory existing conformance gates and produce a machine-readable audit
   report for every static descriptor and installed plugin contribution.
2. Run descriptor/catalog/registration/authority/schema gates and classify
   every failure as source defect or parallel-work baseline failure.
3. Repair source defects in ownership order: descriptor -> Runtime registry ->
   SDK projection -> product consumer.
4. Add regression gates for every repaired class, not one test per symptom.
5. Run focused and repository-level verification; record exact blockers.

## Invariants

- No `cutover_ready` descriptor may lack its executable implementation.
- No `seam` or `unsupported` descriptor may appear as executable.
- No ability may be silently filtered from an authority set that owns it.
- No RPC/stream/bidi call-mode mismatch may survive catalogue assembly.
- No product caller may invent callee, subject authority, or descriptor refs.
- Terminal and transport-terminal are mutually unambiguous and consumed once.
- Audit tooling is deterministic, read-only by default, and count-independent.

## Dirty-worktree boundary

The existing Cargo/media/RemoteApp changes are parallel work. Do not modify
them unless a failing invariant is proven to be owned by the same edited lines
and repair cannot be isolated. Stage or commit nothing without explicit user
authorization.

## Verification ledger

- `python3 tools/scripts/check-system-ability-contract-inventory.py --self-test`
  - PASS.
- `python3 tools/scripts/check-system-ability-contract-inventory.py`
  - PASS: 189 system + 20 builtin-plugin contracts; 209 total.
  - Modes: 188 RPC, 13 stream, 8 bidi.
  - States: 178 cutover-ready, 20 provider-backed, 8 seam, 3 unsupported.
- `cargo test --lib local_session_dispatcher --no-fail-fast`
  - PASS: 26/26, including canonical terminal receipts, native PTY bytes,
    10 MiB lossless opening-ingress backpressure, carrier-scope isolation, and
    Remote Desktop bidi frame projection.
- Catalogue assembly executable test binary:
  - `every_published_ability_resolves_to_a_handler`: PASS.
  - `build_registry_satisfies_device_baseline_contract`: PASS.
  - `every_published_ability_has_a_toml_byte_for_byte_matching_the_renderer`:
    PASS after Pages manifests were given their canonical
    `operator/pages/dedicated-surface` frontend contract.
- `go test ./...` under `sdk/go`: PASS.
- Python SDK focused bidi/C-ABI/stream suite: PASS, 87 tests.
- Backend terminal bridge/service/session suites: PASS.
- Architecture, daemon Invocation migration, key-service custody, and full
  canonical-runtime convergence gates: PASS before the concurrent RemoteApp
  schema edits resumed.
- `cargo run --bin gen-ability-tomls -- --check`
  - PASS: 209 unchanged after repairing Pages runtime metadata and refreshing
    the four RemoteApp descriptors changed by the concurrent schema work:
    `end_session`, `focus_target`, `grant_consent`, and `report_client_state`.

## Repairs

- Added a count-independent full filesystem contract auditor and CI/convergence
  integration.
- Removed stale architecture/migration/key-custody guard assumptions exposed by
  the current owner and FFI shapes.
- Corrected `terminal.attach` descriptor semantics to native stdin/stdout bytes.
- Made handler input closure an explicit per-call delivery state, never a
  synthetic `BIDI_INPUT_CLOSED` terminal or a `SessionDispatchError`; the output
  lifecycle remains the only terminal authority.
- Corrected Pages runtime manifests so live descriptors and static TOMLs share
  one frontend contract.
