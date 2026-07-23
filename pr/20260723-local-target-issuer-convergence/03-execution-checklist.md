# Execution Checklist

- [x] Add a local daemon system issuer entrypoint that returns issued
      target-root tuple facts.
- [x] Migrate Pages invocation to `SystemInvocationTargetIssuer`.
- [x] Migrate Principal invocation to `SystemInvocationTargetIssuer`.
- [x] Delete `invoke_target_root_derived_subject_timeout`.
- [x] Update v2 and legacy architecture gates.
- [x] Run targeted tests, gates, codegraph, and residual grep.
