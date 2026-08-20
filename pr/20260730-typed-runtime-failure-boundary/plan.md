# Typed Runtime Failure Boundary Plan

## Goal

Close the remaining runtime failure classification drift where SDKs or daemon
boundaries infer canonical runtime state from diagnostic text.

## Invariants

- Canonical runtime state is carried by typed error/failure codes.
- Diagnostic detail may be preserved or redacted, but must not upgrade an
  untyped failure into a canonical state.
- Go and Python direct runtime providers follow the same descriptor-resolution
  contract.
- Gates must prevent reintroducing owner-offline classification from
  `ROUTE_NEGATIVE ... owner is not online` text.

## Work Items

1. Retire Python SDK owner-offline message classification.
2. Update Python direct gRPC error projection to accept only canonical code
   prefixes for typed owner-offline transport messages.
3. Update Python tests and SPEC gates so route diagnostic text remains
   descriptor-not-found / ability-not-found, not descriptor-owner-offline.
4. Tighten daemon `RuntimeFailureFacts` so semantic classification is derived
   from typed failure code, while canonical detail can still sanitize typed
   signer-custody messages.

## Verification

- Python SDK targeted tests for error JSON and direct runtime gRPC projection.
- Rust targeted tests for runtime failure and remote failure projection.
- `cargo fmt --check`
- `git diff --check`
- `tools/scripts/check-architecture-convergence.sh`
- `tools/scripts/check-canonical-runtime-convergence-v2.sh`
- `tools/scripts/check-sdk-canonical-public-api.sh`
