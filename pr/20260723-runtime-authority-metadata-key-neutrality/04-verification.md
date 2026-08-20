# Verification

Completed:

- `cargo test -q authority_metadata --lib`
- `go test ./...`
- `cd sdk/python && .venv/bin/python -m pytest -q tests/test_authority.py tests/test_authorized_runtime_session.py tests/test_runtime_ability.py`
- `cd sdk/node && npm test`
- `bash tools/scripts/check-java-sdk-seam.sh`
- `bash tools/scripts/check-swift-sdk-seam.sh`
- `cargo fmt --check`
- `bash tools/scripts/check-architecture-convergence.sh`
- `bash tools/scripts/check-canonical-runtime-convergence-v2.sh`
- `codegraph sync`
- `codegraph query x-easynet-delegation --limit 40`
- `codegraph query x-runtime-delegation --limit 40`
- `rg -n "x-easynet-delegation|x-easynet-session-authority" src sdk include ...`

Result:

- Rust daemon authority metadata tests passed.
- Go SDK full test suite passed.
- Python authority/session/runtime-ability tests passed.
- Node SDK full test suite passed.
- Java and Swift SDK seam checks passed.
- Architecture and canonical runtime convergence gates passed.
- codegraph reports no `x-easynet-delegation` symbol.
- Active `src/`, `sdk/`, and `include/` sources contain no old `x-easynet-*` canonical authority metadata key.
