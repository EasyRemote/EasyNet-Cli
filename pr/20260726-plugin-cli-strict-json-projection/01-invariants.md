# Invariants

- JSON output remains pass-through from the daemon plugin control ability.
- Table output is a CLI-owned projection over daemon JSON, not daemon internal DTO deserialization.
- Required table fields must be present with the expected JSON type.
- Optional cosmetic fields may be absent only where the UI contract explicitly renders `-`.
- Malformed daemon response shape must fail before rendering.

