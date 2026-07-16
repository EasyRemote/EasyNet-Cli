# Verification

Passed:

```text
cargo test -p easynet real_ability_publish_writes_manifest_and_returns_envelope
bash tools/scripts/check-architecture-convergence.sh
bash tests/scripts/test_check_architecture_convergence.sh
git diff --check -- src/daemon/ability/builtins/real_invoke_tests.rs pr/20260716-ability-publish-real-runtime-proof
```
