# Invariants — RemoteApp Target Recovery UI Gate

- The product-flow gate may require frontend consumption of daemon projections;
  it must not define a second target lifecycle model.
- `latestTargetDiagnostic` is authoritative for target failure reason and
  `frontendAction`.
- `targetTracking` remains execution evidence for target binding/input state,
  not a product-complete claim.
- The readiness matrix must remain `product_complete=false` until every
  explicit product requirement has real current evidence.
