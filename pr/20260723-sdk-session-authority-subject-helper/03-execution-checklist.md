# Execution Checklist

- [x] Replace authorized-session wrapper calls with direct canonical helper calls.
- [x] Delete `_session_authority_admits_subject` from `authorized_runtime_session.py`.
- [x] Update v2 gate direct-call assertions and wrapper rejection.
- [x] Update the legacy architecture gate so it protects the same single-owner rule.
- [x] Run Python SDK targeted tests and SPEC gates.
- [x] Run codegraph and `rg` residual checks.
