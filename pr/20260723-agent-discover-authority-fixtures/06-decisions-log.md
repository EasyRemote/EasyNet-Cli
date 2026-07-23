# Decisions Log

- Decision: keep the explicit Device authority root local to
  `agent.discover` tests.
  Rationale: this module exercises one Device-hosted discover surface; a
  module-local helper keeps the invariant visible without widening production
  API surface.
