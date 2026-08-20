# Intent

## Goal

Remove the legacy top-level heartbeat receipt aliases `status` and `permanent` from the federation client contract. Heartbeat receipts must use the canonical Axon response `header` plus explicit `hub_abilities_diff`.

## Non-goals

- Do not alter heartbeat dispatch semantics or lease refresh behavior.
- Do not add a product-specific compatibility adapter.
- Do not preserve older hub wrapper JSON shapes as accepted client input.

## Acceptance criteria

- `HeartbeatReceipt` no longer contains top-level `status` or `permanent`.
- Receipt parsing rejects retired top-level aliases instead of ignoring them.
- Existing canonical heartbeat receipts still parse.
- Heartbeat session code continues to consume the canonical `header` projection.
