# Decisions Log

## 2026-07-26

- Treat mission ability argument parsing as the canonical ingress boundary for mission runtime calls.
- Reject unknown fields instead of preserving a forward-compat or legacy-carrier window.
- Keep `label` optional but type-strict; a non-string label is not equivalent to absence.
- Keep the default label for an absent label because it is part of the documented public API; only type-mismatched labels are rejected.
- Use SPEC v2 to guard the parser shape because product smoke tests can prove `mission.run` works but cannot prove retired carrier fields are rejected.
