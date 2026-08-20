# Verification

Executed checks:

1. `bash -n tools/scripts/check-architecture-convergence.sh && bash -n tests/scripts/test_check_architecture_convergence.sh` - passed.
2. `bash tools/scripts/check-architecture-convergence.sh` - passed.
3. `bash tests/scripts/test_check_architecture_convergence.sh` - passed.
4. `cargo test -q agent_purge_descriptor_is_destructive_but_agent_stop_is_not` - passed.
5. `cargo test -q daemon::ability::builtins::agents::lifecycle::tests::purge_agent_deletes_only_the_registered_agent_root` - passed.
6. `cargo test -q agent_stop --lib` - passed, 1 test.
7. `cargo test -q purge_agent --lib` - passed, 3 tests.
8. `cargo test -q stop_agent --lib` - passed, 9 tests.
9. `cargo test -q agent_purge --lib` - passed, 3 tests.
10. `git diff --check -- tools/scripts/check-architecture-convergence.sh tests/scripts/test_check_architecture_convergence.sh pr/20260716-agent-purge-public-boundary-gate` - passed.

The first R32 draft used unescaped JSON-hint tokens for TOML descriptors and
failed against the real repository. The gate now checks the escaped TOML
descriptor facts and passes against production plus the self-test fixture.
