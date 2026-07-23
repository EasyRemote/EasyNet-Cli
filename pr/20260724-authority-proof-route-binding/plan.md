# AuthorityProof route binding convergence

## Goal

Move AuthorityProof route-selected binding checks into the AuthorityProof domain
model so child invocation assembly cannot keep a second authority-proof route
truth path.

## Root abstraction problem

`AuthorityProofVerifier` already owns proof admission semantics, but
`child_invocation_builder.rs` still compared proof `callee_ura`, `subject_ura`,
`ability_ura`, and `audience_ura` directly against the selected child route.
That duplicated one slice of AuthorityProof semantics outside the proof model
and made future carrier/composite route changes easy to implement in one path
while forgetting the other.

## Architectural decision

Introduce `AuthorityProofRouteBinding` as the typed route binding projection for
AuthorityProof:

- `callee_ura`
- `subject_ura`
- `ability_ura`
- `audience_ura`

`AuthorityProof::matches_route_binding` is the only production predicate that
compares those route facts against a proof. Full admission verification remains
responsible for principal, token, action, nonce, canonical hash, session owner
facts, expiry, issuer authorization, revocation, and signature verification.

This keeps child invocation construction narrow: it derives route facts from
the selected descriptor route, builds the typed binding, and delegates proof
matching to the AuthorityProof domain.

## Boundary invariants

1. Child invocation construction may derive selected route facts, but must not
   directly inspect AuthorityProof route fields for equality.
2. Admission verification and child invocation construction must use the same
   AuthorityProof route binding predicate.
3. Audience binding remains explicit: forwarded/hosted child invocations bind
   proof audience to the selected execution host when present, otherwise to the
   selected callee.
4. Action remains outside child route binding because child construction does
   not own action admission; full verifier context continues to check it.

## Refactoring completed

- Added `AuthorityProofRouteBinding` in
  `src/daemon/invocation/admission/authority_proof.rs`.
- Added `AuthorityProof::matches_route_binding`.
- Updated `verify_invocation_binding` to consume the route binding predicate
  instead of duplicating callee/subject/ability comparisons inline.
- Updated `validate_authority_proof_binding` in
  `child_invocation_builder.rs` to construct `AuthorityProofRouteBinding`
  instead of directly comparing proof route fields.
- Extended SPEC v2 gate to require domain ownership of route binding and forbid
  direct proof route comparisons in child builder production code.
- Added SPEC v2 self-test fixture proving legacy child-builder duplicate
  comparisons are rejected.

## Verification

Completed during implementation:

- `cargo check --features axon-pb`
- `tools/scripts/check-architecture-convergence.sh`
- `cargo fmt --check`
- `cargo test -q --features axon-pb authority_proof_child_binds_selected_route --lib`
- `cargo test -q --features axon-pb verifier_accepts_matching_signed_proof --lib`
- `tools/scripts/check-canonical-runtime-convergence-v2.sh`
- `tools/scripts/check-canonical-runtime-convergence-v2.sh --self-test`
- `git diff --check`
- `/Users/macbook.silan.tech/.local/bin/codegraph sync .`
- `/Users/macbook.silan.tech/.local/bin/codegraph status .`
