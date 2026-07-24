# Invariants

## Semantic invariants

- Heartbeat state is determined by `membership_status`, `realm_directory_size`, canonical `header`, `rejected_nodes`, and `hub_abilities_diff`.
- Top-level `status` and `permanent` are not canonical heartbeat facts.
- `hub_abilities_diff` remains mandatory so a client can distinguish "empty diff" from "old hub omitted the diff".

## Safety invariants

- Unknown heartbeat receipt fields fail closed.
- Client code must not perform JSON shape inspection to recover older hub wrappers.
- Retired aliases must not bypass canonical receipt validation.

## Boundedness invariants

- The heartbeat request/response remains one descriptor-bound invocation.
- No fallback network call or legacy parser branch is introduced.
