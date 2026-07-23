# Verification

Executed:

- `tools/scripts/check-canonical-runtime-convergence-v2.sh --self-test`
  - Result: passed.
- `tools/scripts/check-canonical-runtime-convergence-v2.sh`
  - Result: passed.
- `tools/scripts/check-java-sdk-seam.sh`
  - Result: passed.
- `tools/scripts/check-architecture-convergence.sh`
  - Result: passed.
- `cargo fmt --check`
  - Result: passed.
- `git diff --check`
  - Result: passed.
- `/Users/macbook.silan.tech/.local/bin/codegraph sync .`
  - Result: synced changed Java/gate graph.
- `/Users/macbook.silan.tech/.local/bin/codegraph status .`
  - Result: index is up to date.

Focused negative evidence:

- Java `InvocationBuilder.inspect()` now validates the constructed tuple with `InvocationAuthorityBindingValidator.validate(tuple)`.
- SPEC v2 self-test now includes a Java legacy fixture where the builder accepts shape-only authority metadata and confirms the gate fails.
