# Verification

Executed checks:

- `go test ./...` inside `sdk/go` — passed.
- `go test -tags runtime_direct ./...` inside `sdk/go` — passed.
- `PYTHONPATH="$PWD/sdk/python${PYTHONPATH:+:$PYTHONPATH}" "$SDK_CONFORMANCE_PYTHON" -m pytest sdk/python/tests/test_runtime.py sdk/python/tests/test_direct_runtime.py` — passed, 93 tests.
- `npm test --prefix sdk/node` — passed, 45 tests.
- `bash tools/scripts/check-java-sdk-seam.sh` — passed.
- `bash tools/scripts/check-swift-sdk-seam.sh` — passed.
- `bash tools/scripts/check-node-sdk-seam.sh` — passed.
- `bash tools/scripts/check-architecture-convergence.sh` — passed.
- `bash tools/scripts/check-canonical-runtime-convergence-v2.sh` — passed.
- `bash tools/scripts/check-sdk-canonical-public-api.sh` — passed.
- `cargo fmt --check` — passed.
- `git diff --check` — passed.
- `/Users/macbook.silan.tech/.local/bin/codegraph query backend_ura` — only remaining CodeGraph match is the Rust admission test `cross_realm_backend_ura_rejected_with_permission_denied`, not an SDK facade symbol.
- Targeted `rg` over SDK receipt facade validators found no retired `backend_ura/user_ura` field readers.
