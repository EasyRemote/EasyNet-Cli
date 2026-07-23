# API Contract

No public CLI, SDK, or daemon runtime API changes.

Internal crate contract:

- `RegistryDaemonBuildConfig::new` means production local-environment authority.
- `RegistryDaemonBuildConfig::new_with_authority_context` means caller-owned
  explicit authority snapshot.
- Tests must not use `new` when they already know the authority context.
