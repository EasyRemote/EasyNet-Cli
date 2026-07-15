# Invariants

- Every symbol or member listed under `non_canonical` must have matching
  `legacy_quarantine` metadata.
- Every `legacy_quarantine` item must be approved by
  `canonical_quarantine_reason`.
- The metadata reason must equal the canonical policy reason; hand-written
  alternate explanations are not allowed.
- Replacement targets and cutover references remain validated separately.
- The self-test must prove both unapproved quarantine and mismatched-reason
  failures.
