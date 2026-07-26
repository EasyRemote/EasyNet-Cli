Decisions log
=============

2026-07-26
----------

- Treat `hub` in `AuthorityScope.owner_projection` as product vocabulary inside
  a core runtime authority fact.
- Do not preserve `hub` parsing as a compatibility alias; old data must fail
  closed and be regenerated through canonical runtime authority construction.
- Keep product-facing Hub CLI/deployment mode migration out of this slice so the
  commit remains bounded to the authority binding grammar.
