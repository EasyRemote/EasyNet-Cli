Root abstraction:
- The proxy request has two separate state machines:
  1. fanout scope selection (`peer_hub_urls`)
  2. resolver selector selection (`ability_name`)
- The old shape conflated "field omitted" with "no ability selector".

Refactor:
- Introduce a small explicit nullable string type for selector fields that must be present.
- Store `ability_name` as explicit nullable selector state rather than a defaulted string.
- Convert the selector to the peer `namespace.resolve` JSON only at the peer argument boundary.

Result:
- The daemon can still express directory/listing queries.
- Callers must state whether an ability selector exists.
- There is no hidden fallback from missing JSON field to query-only resolver mode.
