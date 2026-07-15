# Invariants

- Stream and bidi frame domain decoders require `kind`.
- C ABI callback projection may normalize ordering, payload naming, state, and
  error objects, but it does not synthesize `kind` from legacy `event`.
- Existing canonical `kind` behavior remains unchanged for stream and bidi
  sessions.
- No public SDK capability is reclassified in this slice.
- No alternate address terminology is introduced.
