# Media/Bidi Product E2E Gate Plan

## Goal

Bind the Docker media/bidi product E2E script to the SPEC v2 source gate so the product-loop assertions cannot silently drift while the full Docker run remains an explicit integration verification.

## Invariants

1. The product path must use Hub/provider/caller daemon topology rather than a test-only daemon bypass.
2. Remote stream and bidi calls must be descriptor-bound and must preserve caller, callee, subject, ability, nonce and causal-root inputs.
3. Each product operation must produce one unique invocation record and one verified terminal receipt chain.
4. Plugin removal must unpublish ability rows and removed routes must reject invocation without harness timeout.
5. The gate must not make every SPEC v2 run depend on a Docker daemon or image build.

## Boundary Proof

- Full Docker execution remains in `tools/scripts/docker-media-bidi-e2e.sh`.
- SPEC v2 gate executes the script self-test only, proving the script still carries the required product assertions and helper-backed sidecar structure.
- No SDK/product behavior is re-routed to satisfy the gate; this change only strengthens source-level conformance.

## Verification Plan

1. Run `tools/scripts/docker-media-bidi-e2e.sh --self-test`.
2. Run a real Docker media/bidi E2E using the local runtime image.
3. Run `tools/scripts/check-canonical-runtime-convergence-v2.sh`.
4. Run `tools/scripts/check-canonical-runtime-convergence-v2.sh --self-test`.
5. Run `tools/scripts/check-architecture-convergence.sh`.
6. Run `cargo fmt --check`.
