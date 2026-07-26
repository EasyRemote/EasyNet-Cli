# Runtime-state read subject convergence

## Goal

Remove the FFI receipt-history descriptor resolver's private subject parsing rule and replace it with a shared canonical runtime-state read subject value object.

## Invariants

1. Runtime-state read subjects are generic runtime concepts, not EasyNet/EasyRemote product concepts.
2. A receipt-history descriptor request may only use a canonical `Resource` URA owned by `user.<id>` with path `runtime-state/read`.
3. All-zero principal placeholders fail before descriptor resolution or transport I/O.
4. Parsing failures must stay explicit; no empty-string/default projection may decide tuple authority.
5. Public behavior remains compatible for valid canonical subjects.

## Boundary proof

- Source of truth: `crate::core::ura`, which already wraps Axon-owned URA builders/parser for CLI-local projections.
- Consumers: FFI descriptor resolver uses the shared value object; it no longer owns a second receipt-history subject grammar.
- Non-goals: no compatibility path for retired `/session/invocation_history` subjects; no product-specific naming; no route/discovery behavior changes.

## Implementation checklist

1. Add `RuntimeStateReadSubject` to `src/core/ura/mod.rs`.
2. Replace FFI receipt-history descriptor subject validation with the shared parser.
3. Add tests for canonical acceptance, retired session rejection, non-user resource rejection, and all-zero rejection.
4. Run targeted Rust tests, fmt, diff check, and convergence gates.

## Decisions

- The value object returns the parsed `realm`, `user_id`, and original canonical subject string. This keeps future receipt/history/read-model callers from re-parsing URA strings.
- The parser intentionally rejects `agent.<user>.<agent>` resource owners for runtime-state read subjects. Agent-owned resources may be admitted by session authority elsewhere, but receipt-history read state is user-owned runtime state.

## Verification

- `cargo test runtime_state_read_subject --lib`
- `cargo test runtime_descriptor_resolver_rejects_receipt_provider_non_runtime_state_subjects --lib`
- `cargo test core::identity --lib`
- `cargo test ffi::invocation::tests::runtime_descriptor_resolver --lib`
- `cargo fmt --check`
- `git diff --check`
- `bash tools/scripts/check-architecture-convergence.sh`
- `bash tools/scripts/check-canonical-runtime-convergence-v2.sh`
- `bash tools/scripts/check-canonical-runtime-convergence-v2.sh --self-test`
