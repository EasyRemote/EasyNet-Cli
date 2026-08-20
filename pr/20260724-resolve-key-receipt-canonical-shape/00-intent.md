# Intent

## Goal

Converge the federation client `ResolveKeyReceipt` onto the canonical `federation.resolve_key` response shape. Remove unused legacy receipt fields and make parsing fail closed on unknown fields.

## Non-goals

- Do not change hub key resolution dispatch behavior.
- Do not change join transport or key-service custody.
- Do not introduce a compatibility adapter for older resolve_key responses.

## Acceptance criteria

- `ResolveKeyReceipt` exposes canonical resolve-key facts only.
- Retired fields such as `agent_ura`, `status`, `key_id`, and `rotation_epoch` are no longer accepted.
- Current canonical responses with `public_key_b64`, `public_key_hex`, `public_keys_b64`, and optional principal owner fields still parse.
- Join still consumes `public_key_hex` from the canonical receipt.
