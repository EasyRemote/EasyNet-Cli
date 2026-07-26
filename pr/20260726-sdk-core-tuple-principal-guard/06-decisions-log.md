Decisions log
=============

2026-07-26
----------

- Treat all-zero principal rejection as SDK core tuple responsibility rather
  than a provider-specific admission concern.
- Preserve URI/URA shape validation boundaries; this task rejects only the
  sentinel identity value that is never semantically valid.
- Use product-neutral runtime-state/read all-zero fixtures in Go/Python core
  InvocationBuilder tests; receipt-history placeholder vectors remain confined
  to their existing negative admission/authority tests.
