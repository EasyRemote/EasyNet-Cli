# Decisions Log

- Keep `LocalAbilityTarget::daemon_system_subject_ura` crate-private because it
  is policy data consumed by the canonical target issuer.
- Remove only the duplicate local-invoke convenience path, not explicit
  subject-target invocation helpers needed by stream and ability-record flows.
- Add `LocalTargetRootInvocation` as the issuer-owned value object for
  target-derived daemon-system root tuple facts.
- Keep `LocalDaemonSystemAbilityIssuer` as transport only: it now consumes
  issued target-root facts and refuses non-RPC issued calls.
- Update both convergence gates so the removed derived-subject helper cannot
  return as a local invoke compatibility path.
