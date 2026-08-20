# Pages API Response Projection

## Goal

Move the shared `pages.api` HTTP-like response envelope out of the dynamic API handler and into typed daemon resource projections. Preserve the public wire shape while keeping manifest parsing, echo merging, and ability dispatch inside `pages/api.rs`.

## Invariants

1. `pages.api` handler does not assemble the public `status/body/content_type` envelope with raw response JSON.
2. API response carries `status`, `body`, and `content_type`.
3. TOML manifest loading, kind selection, echo merge semantics, and ability dispatch remain in `pages/api.rs`.
4. Public response DTO ownership sits in `daemon::resources::projection`.
5. Unknown fields on the API response DTO fail closed.
6. The DTO is Pages API specific and does not enter SDK canonical abstractions.
7. Only URA terminology is used.

## Boundary Proof

- `pages/api.rs` owns project-authored manifest interpretation and runtime dispatch to an ability when `kind = "ability"`.
- `daemon::resources::projection` owns the public HTTP-like response envelope shape returned by all API kinds.
- The Pages sandbox remains the manifest read boundary.
- The Axon registry remains the ability dispatch authority; this change adds no fallback route or product-specific SDK lifecycle.

## Verification Plan

- Unit tests for typed `PagesApiResponse` public wire shape.
- Strict unknown-field rejection test for the API response DTO.
- Focused handler tests proving static JSON, echo, and ability-backed branches all return the typed public envelope.
- SPEC v2 gate coverage preventing handler-owned Pages API response envelope assembly from returning.
- Existing SPEC v2, architecture gate, cargo check, rustfmt, diff check, and codegraph sync/status.

## Implementation Delta

- Added `PagesApiResponse` to `daemon::resources::projection` with strict unknown-field rejection.
- Replaced the three duplicated `status/body/content_type` handler envelopes in `pages/api.rs` with one typed projection path.
- Kept TOML parsing, static JSON conversion, echo merge semantics, and ability-backed dispatch inside the API handler.
- Added SPEC v2 gate coverage and self-test fixture to reject handler-owned Pages API response envelope assembly.
- Added focused handler coverage for static JSON, echo, and ability-backed API responses.

## Verification Results

- `cargo test -q --features axon-pb pages_api_response_preserves_public_shape --lib`
- `cargo test -q --features axon-pb pages_api_response_rejects_unknown_fields --lib`
- `cargo test -q --features axon-pb static_json_manifest_returns_typed_payload_projection_shape --lib`
- `cargo test -q --features axon-pb echo_manifest_returns_typed_payload_projection_shape --lib`
- `cargo test -q --features axon-pb ability_manifest_invokes_registry_handler --lib`
- `tools/scripts/check-canonical-runtime-convergence-v2.sh`
- `tools/scripts/check-canonical-runtime-convergence-v2.sh --self-test`
- `tools/scripts/check-architecture-convergence.sh`
- `cargo fmt --check`
- `git diff --check`
- `cargo check -q --features axon-pb`
- `/Users/macbook.silan.tech/.local/bin/codegraph sync .`
- `/Users/macbook.silan.tech/.local/bin/codegraph status .`
- `/Users/macbook.silan.tech/.local/bin/codegraph affected src/daemon/ability/builtins/resources/pages/api.rs src/daemon/resources/projection.rs tools/scripts/check-canonical-runtime-convergence-v2.sh`

## Follow-up Seam

The next Pages-local response seam is no longer the `pages.publish/fetch/api` family. Continue with a fresh codegraph pass across remaining handler-owned `Ok(json!({ ... }))` responses and select the next seam by authority boundary, not by textual proximity.
