# Retire SDK invocation-history subject compatibility

## Goal

Remove the SDK-level compatibility predicate for the retired
`session/invocation_history` subject path.

The canonical runtime SDK should not know product-era receipt-history directory
names. Runtime-state and governance reads already have canonical subject
predicates:

- user-owned `runtime-state/read`
- callee runtime-owner subject for device/authority governance reads

Retired history subjects should fail through those canonical predicates instead
of through a dedicated compatibility classifier.

## Root abstraction problem

The SDK currently exposes and consumes `isRetiredInvocationHistorySubjectURA`
variants in Go, Python, and Node. Even though the predicate rejects the old
shape, it keeps the old product directory model as first-class SDK knowledge.

That violates the SDK boundary: the SDK defines the canonical runtime model, not
EasyNet's old invocation-history receipt directory model.

## Boundary invariants

1. No SDK production module exports or imports a retired invocation-history
   subject predicate.
2. Session authority subject admission remains canonical:
   - exact authority subject matches are admitted;
   - user-owned resource subjects are admitted by owner identity;
   - malformed, all-zero, or unrelated subjects are rejected by canonical URA
     ownership parsing.
3. Receipt-history provider admission remains stricter than generic session
   authority:
   - only runtime governance read subjects are accepted;
   - old `session/invocation_history` subjects are rejected because they are not
     canonical governance read subjects.
4. Go, Python, and Node stay aligned; deleting the legacy classifier in one SDK
   requires deleting it in the others.
5. Tests keep negative vectors for the retired path, but the assertion is that
   canonical predicates reject it, not that a compatibility helper recognizes
   it.

## Implementation plan

1. Delete retired invocation-history subject constants and predicates from Go,
   Python, and Node SDK runtime-subject helpers.
2. Remove imports/calls from session authority and authority subject validators.
3. Update tests to assert canonical rejection through current validation paths.
4. Update convergence gates so they reject reintroduction of the retired helper
   instead of requiring it.
5. Run SDK targeted tests, SPEC v2 gate, and formatting/static checks.
