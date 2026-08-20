Implementation plan:

1. Add a small runtime-ready capability set abstraction near `ready_runtime_discovery`.
2. Require paired User runtime signer capability for Device/Both ready discovery.
3. Replace the old permissive test with a fail-closed regression test.
4. Keep Hub ready discovery free of paired User signer requirements.
5. Update architecture/SPEC gates if they currently encode the permissive behavior.
6. Verify with focused bin tests, fmt, architecture gate, SPEC v2 gate, and diff check.
