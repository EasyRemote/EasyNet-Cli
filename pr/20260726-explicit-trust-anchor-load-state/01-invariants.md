# Invariants

1. The trust-anchor model exposes explicit load state: loaded anchor or missing
   file.
2. Missing files are not collapsed into empty trust sets inside the storage
   loader.
3. Daemon boot may intentionally project missing trust-anchor state into an
   empty in-memory anchor while logging a first-run event.
4. SIGHUP reload must never replace a live trust anchor with an empty anchor
   because a file disappeared.
5. CLI read projections may render a missing trust-anchor file as an empty
   user-facing list, but that policy must be local to CLI projection code.
6. Receipt proof resolution must preserve missing, empty, loaded, and failed
   trust states as distinct states.
7. No production caller may invoke `load_or_empty`.
