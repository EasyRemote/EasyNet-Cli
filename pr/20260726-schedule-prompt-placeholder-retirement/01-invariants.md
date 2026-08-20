Invariants
==========

- Every enabled schedule that reaches `due()` has enough input to construct the
  eventual Invocation payload without fallback text.
- Persistence is fail-closed for obsolete schedule rows that omit prompt or
  store it as null/blank.
- The schedule lifecycle remains explicit: create -> persist/cache -> due ->
  render prompt -> invoke.
- Public route names and response envelopes remain compatible; only invalid
  schedule creation/loading states are rejected.
