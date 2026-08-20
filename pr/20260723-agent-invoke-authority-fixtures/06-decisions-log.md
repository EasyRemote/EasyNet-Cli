# Decisions Log

- Decision: add runtime-backed test constructor now.
  Rationale: `agent.invoke` has an immediate runtime-backed fixture caller, so
  this is convergence of existing test setup rather than unused API surface.
- Decision: reuse a single explicit Device authority root for `agent.invoke`
  runtime-backed and metadata-only fixtures.
  Rationale: the tests exercise one Device-owned ability surface; varying the
  owner root per fixture would obscure the authority invariant being verified.
