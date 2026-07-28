# Cutover Product E2E Source Gate Plan

## Goal

Keep product cutover readiness aligned with the full product E2E surface by making its source-level self-tests cover both EasyRemote two-node and media/bidi Docker E2E scripts without requiring external checkouts for script-contract validation.

## Invariants

1. Source-level `--self-test` paths must not require Docker, built images, sibling EasyRemote, sibling EasyNet backend, or sibling EasyNet-Axon checkouts.
2. Full product E2E scripts retain their runtime path validation outside `--self-test`.
3. Cutover readiness must explicitly name the media/bidi product E2E contract instead of relying on indirect coverage.
4. The change must not weaken the actual Docker E2E runtime preflight.

## Boundary Proof

- `docker-two-node-easyremote-cli-e2e.sh --self-test` validates script structure only.
- Runtime execution of `docker-two-node-easyremote-cli-e2e.sh` still calls `require_paths` before Docker work.
- `check-sdk-cutover-readiness.sh` gains direct EasyRemote and media/bidi source-contract gates so product cutover reports both Docker product closures explicitly.

## Verification Plan

1. Run `tools/scripts/docker-two-node-easyremote-cli-e2e.sh --self-test`.
2. Run `tools/scripts/docker-media-bidi-e2e.sh --self-test`.
3. Run `tools/scripts/check-sdk-cutover-readiness.sh --self-test`.
4. Run `tools/scripts/check-canonical-runtime-convergence-v2.sh`.
5. Run `tools/scripts/check-canonical-runtime-convergence-v2.sh --self-test`.
6. Run `tools/scripts/check-architecture-convergence.sh`.
7. Run `cargo fmt --check`.
