# Decisions Log

- 2026-08-23: Add per-platform input summaries to the verifier report rather than making product completion parse full raw OS effect evidence. This preserves verifier ownership while closing the weak aggregate seam.
- 2026-08-23: Preserve `applied_inputs` ordering from verifier evidence instead of sorting by input kind, because sequence monotonicity is part of the invocation/input lifecycle proof.
- 2026-08-23: Product-completion status remains unclaimed; this change only hardens input-injection aggregate evidence.
