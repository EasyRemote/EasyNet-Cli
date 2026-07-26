# Docker media/bidi catalogue matching convergence

## Goal

Keep `tools/scripts/docker-media-bidi-e2e.sh` as a real product-level mutation test while removing script-local catalogue matching divergence.

## Invariants

- Ability URA and descriptor-ref extraction must use the same catalogue row identity model.
- The test must continue to prove descriptor-bound remote stream and bidi ingress.
- Failure diagnostics must report hub, provider, and caller logs without duplicated service dumps.
- No fallback route, legacy receipt path, or compatibility cleanup is introduced.

## Verification

- `bash tools/scripts/docker-media-bidi-e2e.sh --self-test`
- `bash -n tools/scripts/docker-media-bidi-e2e.sh`
- `cargo fmt --check`
- canonical convergence gates before commit
