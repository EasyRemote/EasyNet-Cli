# Invariants

- Backend event consumers may call the SDK event client builder but must not
  construct `eventcore` route catalogs from SDK canonical roots.
- Backend product event routing is local adapter policy and must stay behind
  `newRuntimeEventRouteCatalog`.
- The downstream gate rejects direct use of provider route exports such as
  `easynetprovider.RuntimeEventRoutes`.
- The gate self-test must include a negative fixture proving `eventcore`
  route-catalog imports fail the audit.
- This is a cutover guard only; it does not move product event taxonomies into
  the canonical SDK.
