# Decisions Log

## 2026-07-24

- Receipt history is treated as a canonical runtime receipt/session query, not as a daemon-system target-owned remote ability.
- The fix belongs at the facade boundary where target-owned subject policy is selected; daemon admission remains a verifier rather than a repair path.
- Subject derivation moved from CLI/federation callsites into `RemoteSystemInvocationIssuer::target_owned_root_plan` so the issuer owns nonce, causal root, and target-owned subject policy together.
- The old internal `root_plan(..., subject, ...)` API was removed instead of retained as a compatibility layer because passing a subject into a daemon-system issuer preserved the legacy policy leak.
