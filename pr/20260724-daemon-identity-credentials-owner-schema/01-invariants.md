# Invariants

1. `persistence::config::Credentials` is the single owner of `credentials.json` schema.
2. Daemon caller URA is derived only from canonical `realm` + `node_id`.
3. Missing credentials remains the only non-error absent identity state.
4. Existing but malformed, stale, retired, or unknown-field credentials fail daemon boot.
5. Signer custody is attempted only after the credentials schema has passed the owning validation path.
