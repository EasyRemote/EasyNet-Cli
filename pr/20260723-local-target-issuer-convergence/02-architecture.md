# Architecture

Before:

- MCP/A2A bridges used `SystemInvocationTargetIssuer::local_root_for_target`.
- Pages/Principal used
  `LocalDaemonSystemAbilityIssuer::invoke_target_root_derived_subject_timeout`.
- `local_invoke` therefore retained a second target-derived subject issuer.

After:

- Pages/Principal build an `InvocationTarget` through
  `SystemInvocationTargetIssuer::local_root_for_target`.
- All target-derived daemon-system subject policy flows through routing target
  issuance.
- `local_invoke` only transports explicit tuple facts or caller-selected target
  roots; it does not own target-derived subject policy.
