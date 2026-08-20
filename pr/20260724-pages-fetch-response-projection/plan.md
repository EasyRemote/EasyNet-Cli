# Pages Fetch Response Projection

## Goal

Move `<user>.<project>.page.fetch` public response ownership out of the fetch handler and into typed daemon resource projections. Preserve the public wire shape while keeping sandboxed file reads and payload DTO construction separate.

## Invariants

1. `pages.fetch` handler does not assemble the public fetch payload with raw response JSON.
2. Fetch response carries `bytes_b64`, `content_type`, `size_bytes`, `force_attachment`, and `sha256`.
3. Sandboxed file opening, size validation, MIME selection, and hash computation remain in `pages/fetch.rs`.
4. Public response DTO ownership sits in `daemon::resources::projection`.
5. Unknown fields on fetch response DTOs fail closed.
6. Ability manifest schema generation remains out of scope for this iteration.
7. Only URA terminology is used.

## Boundary Proof

- `pages/fetch.rs` owns ability ingress, sandboxed read, deterministic bytes/hash facts, and MIME facts.
- `daemon::resources::projection` owns the public fetch response shape.
- `pages::sandbox` remains the kernel/path safety boundary.
- No product-specific lifecycle or SDK abstraction is introduced.

## Verification Plan

- Unit tests for typed `PagesFetchResponse` public wire shape.
- Strict unknown-field rejection test for the fetch response DTO.
- Focused handler test proving `handle_fetch` returns the same public shape after migration.
- SPEC v2 gate coverage preventing handler-owned Pages fetch response JSON assembly from returning.
- Existing SPEC v2, architecture gate, cargo check, rustfmt, diff check, and codegraph sync/status.

## Implementation Delta

- Added `PagesFetchResponse` to `daemon::resources::projection` with strict unknown-field rejection.
- Moved public `pages.fetch` payload construction out of `pages/fetch.rs` and into the projection DTO.
- Kept sandboxed file access, byte-size validation, MIME selection, Base64 encoding, and SHA-256 facts inside the fetch handler.
- Added SPEC v2 gate coverage and self-test fixture to reject handler-owned fetch response assembly.
- Added focused handler coverage proving the public wire shape is preserved without leaking local path state.

## Verification Results

- `cargo test -q --features axon-pb pages_fetch_response_preserves_public_shape --lib`
- `cargo test -q --features axon-pb pages_fetch_response_rejects_unknown_fields --lib`
- `cargo test -q --features axon-pb handle_fetch_returns_typed_payload_projection_shape --lib`
- `tools/scripts/check-canonical-runtime-convergence-v2.sh`
- `tools/scripts/check-canonical-runtime-convergence-v2.sh --self-test`
- `tools/scripts/check-architecture-convergence.sh`
- `cargo check -q --features axon-pb`
- `cargo fmt --check`
- `git diff --check`
- `/Users/macbook.silan.tech/.local/bin/codegraph sync .`
- `/Users/macbook.silan.tech/.local/bin/codegraph status .`
- `/Users/macbook.silan.tech/.local/bin/codegraph affected src/daemon/ability/builtins/resources/pages/fetch.rs src/daemon/resources/projection.rs tools/scripts/check-canonical-runtime-convergence-v2.sh`

## Follow-up Seam

`pages.publish` still owns public response assembly locally and should be migrated into `daemon::resources::projection` next. This should be done as a separate seam because publish owns a different lifecycle boundary than fetch: it creates project visibility state and registers the fetch route, while fetch only projects already-published bytes.
