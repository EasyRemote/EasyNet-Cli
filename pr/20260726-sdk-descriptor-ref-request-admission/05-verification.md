# Verification

Completed:

- `(cd sdk/go && go test .)`
- `(cd sdk/python && uv run python -m pytest tests/test_runtime.py tests/test_runtime_ability.py tests/test_receipt.py tests/test_authorized_runtime_session.py)`
- `tools/scripts/check-sdk-canonical-public-api.sh`
- `cargo fmt --check`
- `git diff --check`
- `tools/scripts/check-canonical-runtime-convergence-v2.sh`
- `tools/scripts/check-architecture-convergence.sh`
- `/Users/macbook.silan.tech/.local/bin/codegraph sync . --quiet`
- `/Users/macbook.silan.tech/.local/bin/codegraph status .`
- `/Users/macbook.silan.tech/.local/bin/codegraph explore descriptor ref provider request admission`
- `cargo build --bin easynet`
- `target/debug/easynet start` after moving `~/.easynet` aside: failed closed with missing credentials, proving old device state was not reused.
- `target/debug/easynet runtime start --as-hub` after moving `~/.easynet` aside: failed closed with missing TLS daemon config, proving old hub config was not reused.
