# Intent

## Goal

Remove stale-row preservation from join-time realm-trust auto-wiring by making `[[trusted_agent]]` upsert normalize the existing row for the target URA regardless of role.

## Non-goals

- Do not change the realm-trust file format.
- Do not remove real `agent_ura` fields from realm-trust, where that field is the current trust-anchor schema.
- Do not add migration fallback paths or alternate parsers.

## Acceptance criteria

- Existing device rows are updated to the canonical public key and role instead of silently preserved.
- Existing device rows do not retain hub-only fields after canonicalization.
- Existing hub rows are updated through the same row replacement path as device rows.
- Tests prove stale rows are normalized and unrelated rows are preserved.
