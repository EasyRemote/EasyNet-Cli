# Invariants

- Stream and bidi media operations must be observable through the provider's
  invocation ledger after remote and local CLI ingress.
- Each expected operation maps to one unique invocation record. Duplicate
  request ids or invocation URAs are product-level double-submit defects.
- Every recorded media invocation must preserve the expected ability URA and
  provider callee URA.
- Every recorded media invocation must expose a verified receipt chain.
- Every media receipt chain must have exactly one completed terminal receipt,
  and the chain head must be that terminal receipt.
- The E2E report must publish these facts as machine-readable evidence and fail
  if any fact is false.
