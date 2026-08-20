# Runtime ability projection state cutover

## Goal

Remove SDK runtime ability projection's path-marker split helpers and model
descriptor ability identity as explicit projection state across Node, Java, and
Swift. This keeps the language SDKs aligned with the canonical runtime direction:
descriptor identity is projected once, then authority/governance checks consume
typed facts.

## Root abstraction problem

`RuntimeAbilityProjection` exposed language-local marker constants and
`descriptorWireAbility` helpers. Callers used that extracted fragment as a
fallback scope candidate. That preserved a compatibility-style path substring
model instead of a shared explicit projection state.

## Invariants

1. Public invocation builder still rejects governance read descriptors before
   dispatch.
2. Authority scope admission still accepts the canonical public ability name when
   the descriptor owner matches the callee.
3. Explicit Ability URA scopes remain valid.
4. Descriptor owner mismatch must not synthesize a callee-local public name.
5. SDKs must not expose a runtime ability `wire` helper or path marker.

## Boundary proof

The Node, Java, and Swift SDKs do not currently have the same provider-backed
Addressing surface as Go/Python, so this isolated change keeps them in a seam
state instead of inventing language-local providers. The convergence step is to
keep descriptor parsing in one projection object per SDK and remove downstream
fallback fragments. Callers consume `publicName`, `abilityURA`, and
`intrinsicName`; they no longer split URA strings themselves.

## Verification plan

- Node runtime core tests.
- Java runtime core seam tests.
- Swift runtime core seam tests.
- canonical runtime convergence v2 gate.
- v2 self-test.
- SDK canonical public API attestation if source hashes change.
- `git diff --check`.
