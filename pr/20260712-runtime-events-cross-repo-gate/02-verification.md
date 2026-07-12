Completed checks:

- `bash tools/scripts/runtime-events-cross-repo-e2e.sh --self-test`
- `bash tools/scripts/runtime-events-cross-repo-e2e.sh`
- `bash tools/scripts/check-sdk-cutover-readiness.sh --self-test`

The cross-repository gate requires Go and Python test tools on `PATH`; the
passing run used:

```bash
PATH="$HOME/go/bin:/opt/homebrew/bin:$PATH" \
PYTHON_BIN="/Users/macbook.silan.tech/Documents/GitHub/EasyRemote/.venv/bin/python" \
bash tools/scripts/runtime-events-cross-repo-e2e.sh
```

It passed:

- Go SDK Runtime Events tests;
- Python SDK Runtime Events tests;
- Backend `internal/sdkevents`, `internal/svc` and `internal/sdkboundary`
  event adapter tests;
- EasyRemote mission event consumer cursor/fail-closed tests.
