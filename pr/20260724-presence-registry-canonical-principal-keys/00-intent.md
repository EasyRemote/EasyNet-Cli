# Intent

## Goal

Move presence-key validation into `PresenceRegistry` so malformed or retired liveness identities cannot enter the canonical session liveness map and be repaired or silently ignored by downstream read models.

## Non-goals

- Do not remove support for canonical Device, User, or Agent presence principals.
- Do not change session.open wire shape.
- Do not add read-model fallback filtering for malformed keys.

## Acceptance criteria

- `PresenceRegistry::insert*` rejects non-canonical URAs at insertion.
- Allowed presence keys are canonical Device, User, or Agent URAs only.
- `federation.list_user_devices` no longer carries a test asserting legacy agent-shaped rows are silently ignored after insertion.
- SPEC v2 gate rejects reintroduction of unchecked presence insertions or wrapper-level legacy swallowing.
