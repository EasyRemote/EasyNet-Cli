# Protocol Contract Boundary Plan

## Goal

Move SDK profile carrier and JSON DTO contract modules into the SPEC-owned
`src/protocol/` boundary so daemon process modules stop owning language-neutral
SDK protocol projections.

## Boundary Proof

- `src/protocol/` owns Axon-derived JSON/schema projection helpers and typed
  daemon protocol DTO contracts.
- `src/daemon/` owns daemon process lifecycle, execution, local runtime
  orchestration, resources, plugins, and daemon policy.
- `src/ffi/` owns C ABI projection and may call protocol helpers, but it must
  not define profile DTO semantics itself.

## Invariants

1. The SPEC remains unchanged.
2. JSON shapes, fixtures, ABI symbols, and language facade public behavior do
   not change in this structural slice.
3. Profile contract modules must import shared carrier helpers from
   `crate::protocol::sdk_contract`, not `crate::daemon::sdk_contract`.
4. FFI modules must depend on `crate::protocol::*_contract` for DTO projection
   rather than treating daemon process modules as the protocol source.
5. No compatibility re-export is kept under `daemon`; callers are migrated to
   the single protocol boundary.

## Implementation Steps

1. Add `src/protocol/mod.rs`.
2. Move shared and profile-specific contract modules with `git mv`.
3. Update imports, module declarations, and file headers.
4. Run formatting and Rust/SDK gates that cover moved modules.
5. Commit as a structural convergence slice.

## Verification

- `cargo fmt`
- `cargo check --lib --features axon-pb`
- `cargo test --bin sdk-conformance-runner`
- `cargo test --lib sdk_contract`
- `cargo test --lib host_stream_contract`
- `bash tools/scripts/check-sdk-scaffold.sh`
- URA terminology scan for touched Rust files and plan.
