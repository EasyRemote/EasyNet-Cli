# Decisions Log

## 2026-07-17

- Treat the checked-in current worktree as the implementation baseline. Do not
  revert pre-existing edits.
- Follow SPEC delivery order: RF-5/RF-3 before daemon tuple and route work,
  then lifecycle, receipt, product/Mission extraction, and URA/schema gates.
- Use URA terminology even where older local skill text still says URI.
- Compatibility code is acceptable only as edge adaptation that constructs a
  complete descriptor-bound request. It is not allowed inside the canonical
  runtime domain.
- Keep the public matrix wire values (`unsupported`, `seam`,
  `provider-backed`, `cutover-ready`) for compatibility, but bind them to the
  SPEC names (`Unsupported`, `Seam`, `ProviderBacked`, `CutoverReady`) through
  `status_canonical_names`.
- Replace resolved `InvocationTarget` optional subject/causal fields with
  explicit policy values. Public callers may provide explicit tuple fields;
  daemon system calls must name their descriptor/root derivation policy before
  LocalRuntime dispatch.
- Treat plain `canonical_invocation_bytes`, `verify_signature`, and
  `run_admission` as quarantined public-surface defects in this repository's
  conformance evidence. The canonical runtime entry point is descriptor-bound
  proof.
