# Verification

## Planned checks

- Targeted Rust tests for the touched runtime/CLI boundary.
- `cargo fmt --check`.
- `tools/scripts/check-canonical-runtime-convergence-v2.sh` when scope and time permit.
- Existing targeted architecture script covering the modified boundary.

## Evidence log

- `mvn -q -f sdk/java/pom.xml test-compile` — passed.
- `java -cp sdk/java/target/classes:sdk/java/target/test-classes run.runtime.sdk.RuntimeCoreSeamTest runtimeReceiptProofFactsAreMandatory` — passed.
- `java -cp sdk/java/target/classes:sdk/java/target/test-classes run.runtime.sdk.RuntimeCoreSeamTest` — passed.
- `tools/scripts/check-canonical-runtime-convergence-v2.sh` — passed.
- `tools/scripts/check-architecture-convergence.sh` — passed.
- `cargo fmt --check` — passed.
- `git diff --check` — passed.
