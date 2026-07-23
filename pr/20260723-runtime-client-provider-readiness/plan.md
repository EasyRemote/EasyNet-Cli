# Runtime client provider readiness convergence

## Goal

Remove the Go SDK runtime-client provider nil-client panic seam without changing the public constructor shape.

The SDK defines the canonical runtime model. Provider-backed adapters must expose explicit provider readiness failures, not language-specific behavior where Python rejects a missing client while Go accepts it and panics later.

## Boundary proof

- This slice only changes SDK runtime-client provider readiness handling.
- Public Go constructor names and return shapes remain unchanged.
- Successful non-nil runtime-client behavior remains unchanged.
- Nil runtime clients fail closed as `PROVIDER_UNAVAILABLE` SDK errors before any method dereferences provider state.
- Python already rejects missing runtime clients at provider construction; the SPEC v2 gate will pin cross-language parity.

## Invariants

1. Go `RuntimeClientSessionRuntimeProvider` methods cannot dereference a nil `RuntimeClient`.
2. Go `RuntimeClientDescriptorProvider` cannot dereference a nil `RuntimeClient`.
3. Nil runtime-client providers surface canonical SDK errors, not panics.
4. Python provider constructors continue to reject missing runtime clients.
5. SPEC v2 rejects reintroduction of direct Go `p.client.*` dereferences in runtime-client providers.

## Verification plan

- Go authorized runtime session provider readiness tests.
- Python authorized runtime session provider readiness tests.
- SPEC v2 gate.
- SDK product-neutrality and public API gates.
- codegraph sync/status.

## Delta log

- Added Go runtime-client provider readiness guards for submit, prepare, await, cancel, events, diagnostics, stream, bidi, and descriptor resolution.
- Preserved public Go constructor signatures while converting nil runtime clients into canonical `PROVIDER_UNAVAILABLE` SDK errors.
- Added Go tests proving nil runtime-client providers fail before dereference.
- Added Python tests pinning constructor-time rejection for missing runtime clients.
- Added SPEC v2 structural and mutation coverage for runtime-client provider readiness parity.
- Verified focused Go/Python authorized runtime session tests, SPEC v2, SDK product-neutrality, SDK public API, cargo fmt, and codegraph.
