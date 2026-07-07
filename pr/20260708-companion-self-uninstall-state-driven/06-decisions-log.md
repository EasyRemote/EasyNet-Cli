# Decisions Log

- 2026-07-08: Do not make `selfcmd` inspect companion state-store internals.
  Manager owns that state and already owns platform cleanup semantics.
- 2026-07-08: Orphan desired-state rows are removed with their status directory
  and returned as warnings because no package manifest is available for
  platform supervisor cleanup.
