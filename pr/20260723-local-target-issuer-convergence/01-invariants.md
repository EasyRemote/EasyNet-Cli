# Invariants

- Local target-derived daemon-system subject policy has one owner:
  `SystemInvocationTargetIssuer`.
- `LocalAbilityTarget::daemon_system_subject_ura` remains crate-private policy
  used by the routing target issuer.
- Local invoke transport accepts already-bound tuple facts; it does not derive
  subject policy for target-owned calls.
- Public behavior remains compatible: Pages and Principal still invoke the same
  abilities with the same arguments and timeout.
- Gates reject reintroduced derived-subject convenience entrypoints in
  `local_invoke`.
