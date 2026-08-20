## Intent

Retire the Node SDK runtime-subject error fallback so runtime-state read subject construction stays on the canonical SDK error path.

## Boundary invariant

- Runtime-state read subjects are canonical invocation/admission preflight data, not UI convenience strings.
- Node must not keep a JavaScript-native `TypeError` fallback for subject construction while Python/Go expose SDK-owned errors.
- The public API shape remains `runtimeStateReadSubjectURA(realm, userID) -> string`; the internal helper requires explicit SDK error constructors.

## Decision

Remove optional fallback error factories from the internal Node helper and assert that callers provide SDK-owned error factories. The public wrapper remains the only exported SDK entry point and now has tests proving invalid runtime-state subjects surface as `SDKError`.

## Verification target

- Node runtime core tests for runtime-state subject construction and receipt history preflight.
- SDK public API/conformance gate if Node source attestation changes.
