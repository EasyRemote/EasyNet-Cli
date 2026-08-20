# Decisions Log

- Decision: do not change `RegistryDaemonBuildConfig::new` production
  semantics.
  Rationale: production boot should fail closed when local Device authority is
  missing; the defect is tests using that constructor for explicit fixtures.
