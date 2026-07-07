# Verification

Status: Passed.

Commands run:

```sh
bash tools/scripts/check-sdk-conformance-reports.sh
cd sdk/go && go test ./...
PYTHONPATH=sdk/python uv run pytest -q sdk/python/tests/test_conformance.py sdk/python/tests/test_client.py
TMPDIR=/tmp bash tools/scripts/check-sdk-scaffold.sh
bash tools/scripts/check-sdk-completion-audit.sh
git diff --check
```

Notes:

- The shared `runtime_core/feature_discovery` case is now required for Rust,
  C ABI, Go, Python, Node, Java, and Swift action-adapter reports.
- Go and Python conformance tests decode the canonical
  `feature-discovery.v4.json` fixture and assert generic runtime profile and
  symbol facts without product feature catalog leakage.
