## Invariants

- Product daemon state has one explicit home-rooted directory authority.
- Startup must not create daemon control, pid, log, or invocation paths under
  the caller working directory.
- A child launch environment override is authoritative when present and
  non-blank.
- A blank child `HOME` is invalid input, not permission to consult another
  fallback path.
- Existing daemon attach validation remains endpoint- and identity-based after
  path construction succeeds.
