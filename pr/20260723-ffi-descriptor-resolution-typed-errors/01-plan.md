# FFI descriptor resolution typed errors

## Goal

Remove the descriptor resolver's message-string compatibility classifier and
make FFI error projection derive from explicit descriptor-resolution states.

## Root abstraction problem

`easynet_runtime_resolve_descriptor_ref` receives a broad `anyhow::Error`,
formats it as text, and then classifies canonical runtime codes by searching
message substrings. That couples public product diagnostics to incidental text
from signer, route, catalog, and transport layers. It also makes new failures
fall through to misleading descriptor-not-found semantics.

## Invariants

1. Invalid request shape maps to `INVALID_ARGUMENT` at `sdk`.
2. Invalid descriptor catalog payload maps to `INVALID_ARGUMENT` at
   `provider_payload`.
3. Missing runtime owner identity maps to `CALLER_IDENTITY_UNAVAILABLE`.
4. Missing caller signer maps to `CALLER_SIGNER_UNAVAILABLE`.
5. Remote owner offline / negative namespace route maps to
   `DESCRIPTOR_OWNER_OFFLINE` with retry `after_backoff`.
6. Catalog miss maps to `DESCRIPTOR_NOT_FOUND` without pretending the ability
   was locally synthesized.
7. FFI projection must not classify by scanning the final rendered error
   message.

## Implementation order

1. Add `DescriptorResolutionError` enum with canonical ABI projection.
2. Convert `runtime_resolve_descriptor_ref_json` and helper callsites to return
   typed errors.
3. Keep any unavoidable upstream-string interpretation local to the remote
   probe boundary, then project from the enum only.
4. Update tests and gates to reject the retired message classifier.

## Verification

- Targeted Rust descriptor resolver tests.
- `cargo fmt --check`
- `git diff --check`
- `check-canonical-runtime-convergence-v2.sh`
- `check-architecture-convergence.sh`
