# Catalog exact authority/mode binding convergence

## Goal

Remove the mode-agnostic catalog authority-root inference used when deriving a
runtime execution key from `AxonAbilityCatalog`. Runtime binding must be proven
against exact `(authority_root, ability, call_mode)` control-plane records, not
against an ability-name-level owner collapse.

## Root abstraction problem

`static_control_plane_key` and `dynamic_control_plane_key` first read the
authority-scoped execution key and then call `control_plane_authority_root`.
That helper collapses all control-plane records for one ability name into a
single authority root. It is better than the old owner side table, but it still
preserves a compatibility-shaped query: "given only ability, guess its one
authority." The canonical state is mode-specific and authority-specific.

This matters for product-visible failures such as descriptor resolution and
`meta.list_abilities`: a same-name ability can legitimately have multiple
call-mode rows and hosted authorities. Runtime binding must validate the exact
row that corresponds to the execution key and handler mode.

## Invariants

1. The execution index remains an execution index only.
2. The control plane remains the only owner/authority/descriptor truth.
3. A runtime key is valid only if every installed handler mode has a matching
   control-plane row for the same authority root.
4. Missing or mismatched control-plane rows fail closed.
5. Public registration and invocation behavior remains source-compatible.

## Implementation order

1. Replace `control_plane_authority_root` with an exact execution-key verifier.
2. Migrate static and dynamic key derivation to the verifier.
3. Add negative coverage for same ability records under unrelated authorities.
4. Add convergence gates that reject mode-agnostic authority-root inference.
5. Verify targeted tests, format, architecture gates, and codegraph impact.

## Completed changes

- Removed `AbilityControlPlaneRegistry::authority_roots_for_ability`.
- Replaced `AxonAbilityCatalog::control_plane_authority_root` with
  `verify_execution_key_control_plane_modes`.
- Added `ExecutionIndex::handlers_for_key` so runtime key validation reads
  handler state by exact authority-scoped execution key rather than by merged
  ability name.
- Migrated static and dynamic runtime key derivation to validate every
  installed handler slot against
  `control_plane_record_for_authority_mode(authority_root, ability, mode)`.
- Added static and dynamic tests proving unrelated same-name authority records
  do not affect runtime key derivation.
- Added a negative test proving an unrelated authority record cannot rescue a
  handler whose own exact control-plane row was removed.
- Added architecture and SPEC v2 gates that reject ability-level authority-root
  collapse in runtime key derivation.

## Verification

- `cargo test -q exact_authority_mode_record --lib`
  - 2 passed.
- `cargo test -q unrelated_authority_record_as_rescue_path --lib`
  - 1 passed.
- `cargo test -q routeability_helpers --lib`
  - 2 passed.
- `cargo test -q control_plane_keeps_rpc_and_stream_records_for_same_ability --lib`
  - 1 passed.
- `cargo test -q dynamic_registration_rollback_restores_prior_snapshot --lib`
  - 1 passed.
- `cargo test -q runtime_registration_binds_control_plane_proof_facts --lib`
  - 1 passed.
- `cargo fmt --check`
  - passed.
- `git diff --check`
  - passed.
- `check-architecture-convergence.sh`
  - passed.
- `check-canonical-runtime-convergence-v2.sh`
  - passed.
- `codegraph callers verify_execution_key_control_plane_modes`
  - direct production callers are `static_control_plane_key` and
    `dynamic_control_plane_key`.
- `codegraph impact verify_execution_key_control_plane_modes`
  - impact is limited to catalog runtime-key lifecycle paths and tests.
