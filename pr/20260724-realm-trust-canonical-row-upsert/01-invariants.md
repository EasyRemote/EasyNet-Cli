# Invariants

## Semantic invariants

- `agent_ura` is the unique key for a `[[trusted_agent]]` row in the current realm-trust schema.
- Upsert means canonical replacement of the target row, not append-only preservation of prior contents.
- Device rows and hub rows share one row-materialization path.

## Safety invariants

- A stale trust key must not survive a successful auto-wire for the same URA.
- Hub-only routing/TLS fields must not survive on a canonical device row.
- Unrelated trust rows must be preserved byte-semantically as much as `toml_edit` allows.

## Boundedness invariants

- The upsert scans the bounded in-memory TOML table once.
- No fallback file path or second trust-anchor source is introduced.
