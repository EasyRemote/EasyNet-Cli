# Verification

All planned checks passed on 2026-07-23.

- `cargo test -q callee_ura_from_envelope --lib`
  - Result: passed, 2 tests.
- `cargo test -q invoke_rejects_namespace_proxy_resolve_legacy_camel_case_input_aliases --lib`
  - Result: passed, 1 test.
- `cargo fmt --check`
  - Result: passed.
- `git diff --check`
  - Result: passed.
- `bash tools/scripts/check-architecture-convergence.sh`
  - Result: `architecture-convergence: OK`.
- `bash tools/scripts/check-canonical-runtime-convergence-v2.sh`
  - Result: `canonical-runtime-convergence-v2: OK`.
- `/Users/macbook.silan.tech/.local/bin/codegraph sync .`
  - Result: synced changed dispatch sources and gates.
- `/Users/macbook.silan.tech/.local/bin/codegraph query target_ura_from_envelope`
  - Result: no results found.
- `/Users/macbook.silan.tech/.local/bin/codegraph callers callee_ura_from_envelope`
  - Result: production callers are unary, stream, bidi, and carrier-v1 dispatch paths.
