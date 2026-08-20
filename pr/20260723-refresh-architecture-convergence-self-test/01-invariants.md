# Invariants

## Semantic

- A gate self-test canonical fixture must represent the current canonical
  architecture, not an older accepted architecture.
- Negative fixtures must continue to exercise forbidden legacy surfaces.

## Safety

- Updating the fixture must not weaken the production gate.
- The fixture must include explicit canonical states for identity, routeability,
  local loopback subject policy, and callee-only target extraction.

## Boundedness

- The fixture remains a small textual model; it should contain only the tokens
  needed to prove gate behavior.

## Recovery

- Future gate additions should fail this self-test only when the canonical
  fixture needs to model a new accepted boundary.
