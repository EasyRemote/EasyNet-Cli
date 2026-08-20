Executed checks:
- `cargo test runtime_descriptor_resolver --lib` — passed, 12 tests.
- `cargo test descriptor_catalog --lib` — passed, 4 tests.
- `cargo test runtime_descriptor --lib` — passed, 13 tests.
- `bash tools/scripts/check-architecture-convergence.sh` — passed.
- `bash tools/scripts/check-canonical-runtime-convergence-v2.sh` — passed.
- `bash tools/scripts/check-canonical-runtime-convergence-v2.sh --self-test` — passed.
- `cargo fmt --check && git diff --check` — passed.
- `/Users/macbook.silan.tech/.local/bin/codegraph sync . && /Users/macbook.silan.tech/.local/bin/codegraph status .` — index up to date.
