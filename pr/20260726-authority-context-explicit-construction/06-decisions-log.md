Decisions log
=============

2026-07-26
----------

- Treat local-environment authority discovery as an explicit daemon boot choice,
  not as the default value of the authority context type.
- Remove the ambient metadata-only `AxonAbilityCatalog::new()` constructor;
  metadata-only catalog creation now requires an explicit
  `AbilityAuthorityContext`.
- Keep `AxonAbilityCatalog::new_with_runtime` because daemon boot remains a
  public construction path, but gate it so the non-test branch explicitly binds
  `AbilityAuthorityContext::from_local_environment()`.
- Upgrade assembly fixtures to canonical AgentRegistry keys and explicit
  hosted-agent authority inventory instead of tolerating shorthand keys or
  device-scoped replay fallback.
