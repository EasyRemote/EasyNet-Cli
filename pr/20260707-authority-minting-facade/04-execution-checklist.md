# Execution Checklist

- [x] Recheck SDK cutover gates and identify backend import-ban as the active
  failure.
- [x] Confirm existing SDK only projects authority metadata.
- [x] Confirm Axon remains the canonical authority owner.
- [x] Add Go AuthorityClient facade and tests.
- [x] Add Python AuthorityClient facade and tests.
- [x] Update parity documentation for the new facade capability.
- [x] Run targeted Go/Python tests and cutover gate.
- [ ] Commit the semantic slice with canonical author identity.
