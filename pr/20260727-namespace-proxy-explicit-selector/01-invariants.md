Ingress invariants:
- `query_name`, `qtype`, `caller_ura`, `subject_ura`, `realm_hint`, and `ability_name` are all explicit request facts.
- `peer_hub_urls` may remain default-empty because local empty fanout is an explicit proxy execution scope, not a route selector fact.
- Missing `ability_name` fails closed.
- `ability_name: null` is the only accepted representation of an explicit absent ability selector.

Routing invariants:
- Descriptor refs and Ability URAs remain resolved by `query_name` when no separate ability selector is present.
- Owner-local ability names remain resolved only when `ability_name` is a non-empty string.
- No proxy call should infer selector state from an absent JSON field.

Failure invariants:
- Missing selector state fails before peer fanout.
- Non-canonical caller/subject URAs still fail before peer fanout.
- Peer response validation remains fail-closed.
