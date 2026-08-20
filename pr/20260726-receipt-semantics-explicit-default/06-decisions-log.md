Decisions log
=============

2026-07-26
----------

- Treat operational receipt semantics as an explicit descriptor construction
  decision, not as the enum's default value.
- Keep public descriptor behavior stable by preserving the explicit
  `ReceiptSemantics::Operational` constructor assignment.
- Treat stale `realm Hub` voice descriptor descriptions as canonical metadata
  drift, not compatibility vocabulary. Runtime voice capabilities are described
  as realm Authority-owned/provider-backed seams.
- Add a SPEC v2 guard so implicit receipt semantics cannot return without
  failing the convergence gate.
