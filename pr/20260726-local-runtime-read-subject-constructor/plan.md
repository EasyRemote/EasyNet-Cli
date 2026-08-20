# Local runtime read subject constructor convergence

## Goal

Remove the local invoke layer's private runtime-state read subject value object and make it consume the canonical core `RuntimeStateReadSubject` constructor/parser.

## Invariants

1. Core identity owns the runtime-state read subject grammar and canonical construction.
2. Local invoke owns only runtime attachment validation and signer-custody proof.
3. No local invoke code may duplicate the `runtime-state/read` path or synthesize `user.<id>` resource owners directly.
4. All-zero/missing user identity still fails before any device/daemon subject fallback.
5. Valid public behavior is unchanged: local runtime-state reads still use a user-owned Resource URA.

## Boundary proof

- Lower layer: `src/core/identity/mod.rs` exposes `RuntimeStateReadSubject::new` and `RuntimeStateReadSubject::parse`.
- Upper layer: `src/support/platform/local_invoke.rs` derives trusted realm/user facts from paired credentials + Ready discovery + signer custody, then delegates subject construction to core.
- Removed boundary: `LocalRuntimeStateReadSubject` no longer exists as a second subject grammar owner.

## Implementation checklist

1. Add canonical `RuntimeStateReadSubject::new(realm, user_id)` to core identity.
2. Replace `LocalRuntimeStateReadSubject` with a local attachment object that returns a core `RuntimeStateReadSubject`.
3. Update runtime-state read subject gates to require the core constructor and reject the retired local subject type/path duplication.
4. Run targeted tests, fmt, diff check, and convergence gates.

## Decisions

- The local attachment object keeps explicit lifecycle/readiness checks because those are not subject grammar. They belong to local invoke.
- The subject object stays in `core::identity`, not `core::ura`, because it is a runtime identity admission fact layered above generic URA syntax.

## Verification

- `cargo test runtime_state_read_subject --lib`
- `cargo test support::platform::local_invoke::tests::runtime_state_read_subject --lib`
- `cargo test ffi::invocation::tests::runtime_descriptor_resolver_rejects_receipt_provider_non_runtime_state_subjects --lib`
- `bash tools/scripts/check-runtime-state-read-subject-boundary.sh`
- `bash tools/scripts/check-canonical-runtime-convergence-v2.sh`
- `cargo fmt --check`
- `git diff --check`
- `bash tools/scripts/check-architecture-convergence.sh`
- `bash tools/scripts/check-canonical-runtime-convergence-v2.sh --self-test`
