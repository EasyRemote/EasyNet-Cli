# Authority Proof Scope Classifier Ownership

## Goal

Move request-scoped one-time AuthorityProof classification into the AuthorityProof domain and remove the duplicate predicate from `admission_facade.rs`.

## Invariants

- Public behavior remains unchanged.
- `AuthorityProofVerifier` and admission consumption use the same request-scoped one-time classification.
- `admission_facade.rs` must not inspect `permission_request_id`, `grant_id`, and `session_id` to reclassify AuthorityProof lifecycle.
- The one-time proof consumption decision remains fail-closed and tied to the canonical domain predicate.

## Boundary Proof

AuthorityProof lifecycle classification is part of the AuthorityProof domain. The admission facade verifies and consumes proofs, but it must not own a second classifier with parallel semantics. A single exported domain predicate keeps verifier and consumption semantics coupled to one source of truth.

## Verification Plan

- Focused AuthorityProof verifier tests.
- SPEC v2 gate/self-test to reject duplicate facade classifiers.
- Architecture gate, cargo check, fmt, diff check, codegraph sync/status.

## Implementation Delta

- Renamed and exported the AuthorityProof domain predicate as `request_scoped_one_time_authority_proof`.
- Updated `AuthorityProofVerifier` to use the exported domain predicate.
- Updated `admission_facade.rs` to import and consume the domain predicate when deciding whether to consume one-time proofs.
- Deleted the duplicate `request_scoped_one_time_authority_proof` implementation from `admission_facade.rs`.
- Extended SPEC v2 coverage so `admission_facade.rs` cannot reintroduce a local request-scoped one-time proof predicate or duplicate the raw condition.
- Added a SPEC v2 self-test fixture where the domain predicate exists but the facade duplicates it, and verified the gate fails.

## Verification Results

- `cargo check -q --features axon-pb`
- `cargo test -q --features axon-pb verifier_rejects_unbound_request_scoped_one_time_proof --lib`
- `cargo test -q --features axon-pb verifier_accepts_matching_signed_proof --lib`
- `tools/scripts/check-canonical-runtime-convergence-v2.sh`
- `tools/scripts/check-canonical-runtime-convergence-v2.sh --self-test`
- `tools/scripts/check-architecture-convergence.sh`
- `cargo fmt --check`
- `git diff --check`
- `/Users/macbook.silan.tech/.local/bin/codegraph sync .`
- `/Users/macbook.silan.tech/.local/bin/codegraph affected src/daemon/invocation/admission/authority_proof.rs src/daemon/invocation/admission/admission_facade.rs tools/scripts/check-canonical-runtime-convergence-v2.sh`
- `/Users/macbook.silan.tech/.local/bin/codegraph status .`

## Follow-up Seam

AuthorityProof still encodes lifecycle forms through optional fields. The next deeper convergence step is a typed AuthorityProof lifecycle enum or state machine so grant-backed, session-scoped, and request-scoped one-time proofs cannot be assembled as invalid optional-field combinations.
