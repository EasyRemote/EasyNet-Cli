Verification Plan
=================

Targeted tests:
- cargo test -q discover --lib
- cargo test -q abilities --lib
- cargo test -q agent_aggregate --lib

Convergence gates:
- tools/scripts/check-architecture-convergence.sh
- bash tests/scripts/test_check_architecture_convergence.sh

Formatting:
- rustfmt --edition 2021 --check touched Rust files
- scoped git diff --check
