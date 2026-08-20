# Authority Proof Session Fact Cutover

## Goal

Remove the verifier compatibility path that allowed session-scoped `AuthorityProof` values to omit `session_owner_user_id`. Session proofs must carry complete session binding facts before they can authorize a descriptor-bound invocation.

## Invariants

- Public wire shape remains source-compatible: `AuthorityProof` can still deserialize existing JSON fields.
- Domain verification fails closed for incomplete session proofs.
- A proof with `session_id` must also carry a non-empty `session_owner_user_id`.
- The proof `session_owner_user_id` must match the verification context.
- Request-scoped one-time proofs without `session_id` retain their existing nonce/canonical-hash binding rule.
- No broad compatibility fallback may synthesize the missing session owner from `owner_user_id`, `subject_ura`, or context.

## Boundary Proof

The compatibility defect is in verification semantics, not JSON decoding. Keeping the field optional at the serde boundary allows typed mismatch errors instead of parse failures, while the verifier becomes the single authority for domain completeness.

## Verification Plan

- Add a regression test for a session proof missing `session_owner_user_id`.
- Run focused `AuthorityProofVerifier` tests.
- Extend SPEC v2 gate to require the explicit session binding verifier and missing-owner regression test.
- Run SPEC v2, self-test, architecture gate, fmt, diff check, cargo check, and codegraph sync/status.

## Implementation Delta

- Added `verify_session_binding_facts` to centralize session proof completeness checks.
- Changed `verify_invocation_binding` so session proof ownership is no longer an inline optional compatibility check.
- Required any proof with `session_id` to carry a non-empty `session_owner_user_id` and match the verification context.
- Preserved non-session proof behavior: `session_owner_user_id` remains optional unless explicitly supplied.
- Added a regression test for a session proof with `session_owner_user_id = None`.
- Added SPEC v2 coverage and a self-test fixture for the retired optional session-owner verifier.

## Verification Results

- `cargo check -q --features axon-pb`
- `cargo test -q --features axon-pb verifier_rejects_session_proof_without_session_owner_fact --lib`
- `cargo test -q --features axon-pb verifier_accepts_matching_signed_proof --lib`
- `cargo test -q --features axon-pb verifier_rejects_session_proof_without_followup_set --lib`
- `cargo test -q --features axon-pb verifier_rejects_unbound_request_scoped_one_time_proof --lib`
- `tools/scripts/check-canonical-runtime-convergence-v2.sh`
- `tools/scripts/check-canonical-runtime-convergence-v2.sh --self-test`
- `tools/scripts/check-architecture-convergence.sh`
- `cargo fmt --check`
- `git diff --check`
- `/Users/macbook.silan.tech/.local/bin/codegraph sync .`
- `/Users/macbook.silan.tech/.local/bin/codegraph affected src/daemon/invocation/admission/authority_proof.rs tools/scripts/check-canonical-runtime-convergence-v2.sh`
- `/Users/macbook.silan.tech/.local/bin/codegraph status .`

## Follow-up Seam

`AuthorityProof` still keeps optional serde fields for non-session and request-scoped proof forms. Future work should introduce typed proof variants so session, grant, and one-time proof facts are represented by distinct domain structs instead of optional-field combinations.
