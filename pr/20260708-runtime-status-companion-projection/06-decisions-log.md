# Decisions Log

- 2026-07-08: Keep the report constructor compatibility surface but make the
  production service use explicit observations. This avoids hidden I/O in JSON
  rendering while preserving existing tests and callers.
