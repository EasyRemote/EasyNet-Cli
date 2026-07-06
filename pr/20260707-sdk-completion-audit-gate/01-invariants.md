# Invariants

1. A profile state is language-facade evidence; a cutover-ready claim is an
   aggregate consumer gate.
2. Go and Python P0 capability rows must not be weaker than
   `provider-backed`.
3. EasyRemote and backend cutover evidence must remain product-boundary rules,
   not SDK profile rows.
4. The completion audit must fail before a human can claim completion if
   readiness, conformance reports, parity matrix, product boundaries, or live
   smokes drift.
5. P1 language gaps are tracked as future binding work and do not weaken the
   P0 daemon SDK completion claim.
