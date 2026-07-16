# Invariants

- Adapter report schema remains version 2.
- Evidence `ref_path` entries must remain inside the repository root.
- Only derived adapter-report digests may change.
- Runtime source, public SDK APIs, and public daemon behavior are unchanged.
- Stale source evidence must fail before refresh and pass after refresh.
