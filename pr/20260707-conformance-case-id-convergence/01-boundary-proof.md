# Boundary Proof

## Ownership

Conformance case ids are SDK contract metadata. They must describe generic
Runtime Core behavior and must not encode product route names, transport
families, or language-specific implementation details.

## Case Identity

The SPEC is the canonical source for public case ids. A case id rename is not a
compatibility alias; adapter reports and tests must move to the latest id so
the suite has one shared runtime vocabulary.

## Stream And Bidi Separation

Stream ordered terminal behavior and bidi close-send behavior are separate
Runtime Core lifecycle obligations. Splitting the combined case keeps each
state machine independently auditable while reusing the same existing
language-level behavioral tests.

## Product Boundary

The change stays inside SDK conformance metadata and Runtime Core seam tests.
It does not add backend, EasyRemote, daemon product, or UI policy.

## URA Discipline

No address terminology or identity model changes are introduced.
