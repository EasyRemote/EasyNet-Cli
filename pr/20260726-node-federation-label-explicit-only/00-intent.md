# Node federation label explicit-only cutover

## Goal

Remove the CLI/node read-model fallback that displays
`axon.federation.runtime_id` when `axon.federation.runtime_label` is absent.
The runtime id is a stable internal identifier, not a human-readable product
label. Product-visible "via" labels must come only from explicit runtime label
facts.

## Non-goals

- Do not change node online/offline state projection.
- Do not remove `runtime_id` from upstream labels or directory payloads.
- Do not add another display label heuristic.

## Acceptance criteria

1. `federation_label` returns `runtime_label` when non-empty.
2. `federation_label` returns `None` when only `runtime_id` is present.
3. Empty `runtime_label` is absent, not a trigger for another fallback.
4. SPEC v2 rejects future `runtime_id` display fallback.
