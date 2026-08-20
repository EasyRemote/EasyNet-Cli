# Architecture

Root abstraction problem:

`RegistryDaemonBuildConfig::new` combined two states:

- production daemon boot with local environment authority resolution;
- fixture-driven assembly with an already known authority context.

Callers attempted to correct this procedurally by assigning
`config.authority_context` after construction. That does not work because the
constructor already resolved local identity. The right boundary is a constructor
that takes authority as an input state.

Refactoring:

- Add `RegistryDaemonBuildConfig::new_with_authority_context`.
- Make `new` delegate to it after resolving production authority.
- Migrate test and real-invoke callers that immediately override authority to
  the explicit constructor.
