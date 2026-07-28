# Descriptor provider subject parity

Date: 2026-07-29

## Goal

Remove the remaining Java/Swift catalogue-provider legacy rule that requires
`ability_descriptor` descriptor resolution to use the callee realm Authority
subject. The canonical model uses runtime governance read subjects: either the
callee runtime-owner subject or a user-owned `runtime-state/read` subject.

## Invariants

1. `ability_descriptor` and `receipt_history` provider requests require
   explicit `caller_ura` and `subject_ura`.
2. Provider subject validation is generic runtime policy, not product policy.
3. Catalogue descriptor resolution must accept the exact callee runtime-owner
   subject so products can read a device/authority catalogue without inventing
   an authority-subject fallback.
4. Provider subject validation must reject unrelated Authority URAs and
   malformed/path-substring runtime-state subjects before transport.
5. Go, Python, Node, Java, and Swift must express the same subject contract.

## Boundary decision

The old callee-realm Authority-only rule is a compatibility artifact. It makes
Java/Swift diverge from the provider-backed runtime model and can cause product
catalogue reads such as `meta.list_resources` to fail before reaching the
canonical provider path. The fix belongs in SDK runtime primitives, not in
product adapters.

## Work plan

1. Add Java/Swift runtime-governance subject predicates near existing runtime
   subject helpers. Done.
2. Refactor Java/Swift `RuntimeDescriptorRefRequest` provider validation to use
   the shared predicate for both `ability_descriptor` and `receipt_history`.
   Done.
3. Add negative and positive tests for runtime-owner, user-owned
   `runtime-state/read`, unrelated Authority, and path-substring subjects.
   Done.
4. Strengthen SPEC v2 gate to reject the retired Authority-only rule and require
   Java/Swift parity tests. Done.
5. Run Java/Swift SDK tests, SPEC v2, self-test, architecture gate, and fmt.
   Done.

## Result

- Java and Swift no longer derive a callee-realm Authority subject for
  catalogue descriptor resolution.
- Java and Swift provider requests now validate against runtime governance read
  subject semantics, matching the canonical provider model.
- The obsolete `authorityURAForRealmOf` helper was deleted.
- SPEC v2 gate now rejects reintroducing the Authority-only provider subject
  rule.
