# Ability conformance owner-binding plan

## Intent

Make the daemon ability conformance check prove the expected descriptor owner
and runtime binding state, not only the public ability name.

## Boundary

- Do not change public ability names, descriptor refs, or frontend contracts.
- Keep device-native baseline abilities owned by their device-sponsored
  SystemAgent.
- Keep hub introspection baseline abilities owned by realm authority.
- Do not reintroduce name-only fallback lookup for mixed authority roots.

## Invariants

- A baseline ability is supported only when the committed authority catalogue
  has the expected owner, call mode, and a bound runtime binding.
- Name collisions across authority roots must fail closed.
- Device is still execution substrate/sponsor, not the public ability callee.

## Verification plan

- Run the targeted conformance regression test.
- Run the broader ability conformance test set if the targeted test passes.
- Run `git diff --check`.
- Run CodeGraph status after the change.
