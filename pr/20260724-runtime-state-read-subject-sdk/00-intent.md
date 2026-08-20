# Runtime-state read subject SDK convergence

## Goal

Move runtime-state read subject construction out of product/CLI-only rules and into the canonical SDK runtime model so product consumers do not hand-roll `subject_ura` values for read-only runtime projections such as invocation history, ability catalogue, and health probes.

## Non-goals

- Do not add an EasyNet- or EasyRemote-specific receipt/history abstraction to the SDK.
- Do not weaken authority admission to accept device subjects for user-session history.
- Do not add fallback paths that turn missing user custody into daemon/device subjects.
- Do not change public invocation tuple fields.

## Acceptance criteria

- The SDK exposes a product-neutral runtime-state read subject constructor.
- Go, Python, and Node use the same semantics and tests for this constructor.
- Session history tests use the canonical constructor for accepted reads and keep device-subject mismatch as an explicit rejection case.
- CLI-local `LocalRuntimeStateReadSubject` remains semantically aligned with the SDK constructor until Rust SDK ownership is available in this repository.
- Convergence gates reject reintroduction of ad hoc history/session placeholder or callee-as-subject examples in SDK history tests.
